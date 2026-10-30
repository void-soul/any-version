use super::*;

/**
 * 从可能夹带前后缀、也可能多吐字符的文本里，**截出第一个完整的 JSON 对象**。
 *
 * 为什么不能只用 `ai::channel::parse_json`：模型偶尔会在正确的 JSON 后面多带一个括号
 * 或一句解释。实测回归：`...remark 为通用裸名按规范加 order_ 前缀"]}}` —— 结尾多了一个 `}`，
 * 严格解析报 `trailing characters` 把**整份设计**作废，而那一份内容完全可用。
 * 与 Q-0404 同源：格式噪声不应毁掉可用产出。
 *
 * 扫描方法同 `complete_elements`：括号配对，字符串内部的括号不参与（注释里的 `{"a":1}`
 * 很常见）。
 */
pub(super) fn extract_first_json_object(text: &str) -> Option<serde_json::Value> {
    let bytes = text.as_bytes();
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for i in start..bytes.len() {
        let b = bytes[i];
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// 只要「设计或追问」的场合用这个（增量分支会被当成失败 —— 那些场合本来就不该有增量）。
pub(super) fn parse_outcome(json: serde_json::Value) -> Result<AiOutcome, String> {
    match parse_shape(json)? {
        ParsedOutcome::Design {
            document,
            decisions,
        } => Ok(AiOutcome::Design {
            document,
            decisions,
            repair: RepairReport::default(),
            truncated: false,
        }),
        ParsedOutcome::Clarify { question, options } => {
            Ok(AiOutcome::Clarify { question, options })
        }
        ParsedOutcome::Patch { .. } => Err("这个调用点不接增量补丁".into()),
    }
}

pub(super) fn parse_shape(json: serde_json::Value) -> Result<ParsedOutcome, String> {
    // 问题分支：先看有没有 clarify（不能直接把它当文档解析，否则会报错误的句子）
    if let Some(cl) = json.get("clarify").and_then(|c| c.as_object()) {
        let question = cl
            .get("question")
            .and_then(|q| q.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if question.is_empty() {
            return Err("AI 说要问问题却没写问问内容".into());
        }
        let options = cl
            .get("options")
            .and_then(|o| o.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        return Ok(ParsedOutcome::Clarify { question, options });
    }
    // 增量分支：模型只说「哪几张表改了」。必须在 document 之前判断。
    if let Some(p) = json.get("patch") {
        let patch: crate::commands::db_designer::patch::DesignPatch =
            serde_json::from_value(p.clone()).map_err(|e| format!("patch 解析失败：{}", e))?;
        return Ok(ParsedOutcome::Patch {
            patch,
            decisions: read_decisions(&json),
        });
    }
    // 文档在包装层的 document 里，或者根本就是纯文档（旧形状）
    let doc_value = match json.get("document") {
        Some(d) => d.clone(),
        None => json.clone(),
    };
    let doc = parse_document(doc_value)?;
    Ok(ParsedOutcome::Design {
        document: doc,
        decisions: read_decisions(&json),
    })
}

/// 读 decisions 数组：空白项过滤掉（模型偶尔会写空字符串）。
pub(super) fn read_decisions(json: &serde_json::Value) -> Vec<String> {
    json.get("decisions")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// 把 AI 返回的 JSON 收成 DbDesignDocument：缺字段就补默认，结构不对就报清楚。
pub(super) fn parse_document(json: serde_json::Value) -> Result<DbDesignDocument, String> {
    let mut doc: DbDesignDocument = match serde_json::from_value(json.clone()) {
        Ok(d) => d,
        Err(e) => {
            // 常见的容错：AI 把设计包在 {"design": ...} 或 {"document": ...} 里
            let nested = json
                .get("design")
                .or_else(|| json.get("document"))
                .or_else(|| json.get("dbdesign"))
                .cloned();
            match nested.and_then(|v| serde_json::from_value::<DbDesignDocument>(v).ok()) {
                Some(d) => d,
                None => {
                    // 解析失败时把 AI 原始输出落滚动日志（同思维导图的做法）：
                    // 只给用户看 "missing field `id`" 没法定位，得看它到底写了什么。
                    let head: String = json.to_string().chars().take(2000).collect();
                    tracing::error!(
                        "[dbd] AI 产出无法解析为设计文件（{}）；AI 原始输出开头：{}",
                        e,
                        head
                    );
                    return Err(format!("AI 返回的不是合法设计文件：{}", e));
                }
            }
        }
    };
    if doc.id.trim().is_empty() {
        doc.id = new_id("dbd");
    }
    if doc.name.trim().is_empty() {
        doc.name = "未命名设计".to_string();
    }
    if !crate::commands::db_designer::models::DIALECTS.contains(&doc.dialect.as_str()) {
        doc.dialect = "mysql".to_string();
    }
    if doc.nodes.is_empty() {
        return Err("AI 没有产出任何表。换个说法再试一次，或自己加表".to_string());
    }
    // 节点 id 兜底：AI 常漏 id（这时 `id` 反序列化为空串），坐标更是不会给 ——
    // 位置交给前端「自动布局」。编号要避开 AI 自己写过的 id，否则会覆盖它的引用。
    let mut node_ids: Vec<String> = doc.nodes.iter().map(|n| n.id.clone()).collect();
    fill_missing_ids(&mut node_ids, "t");
    for (n, id) in doc.nodes.iter_mut().zip(node_ids) {
        n.id = id;
    }
    let mut relation_ids: Vec<String> = doc.relations.iter().map(|r| r.id.clone()).collect();
    fill_missing_ids(&mut relation_ids, "r");
    for (r, id) in doc.relations.iter_mut().zip(relation_ids) {
        r.id = id;
    }

    // 关联端点写的是**表名**而不是 id 时（AI 漏写 id 的情况下很常见），按表名回认一次。
    // 只在端点指向不存在的节点时才改 —— 正常的 id 引用一律不动。
    let id_by_name: std::collections::HashMap<String, String> = doc
        .nodes
        .iter()
        .filter(|n| !n.name.trim().is_empty())
        .map(|n| (n.name.trim().to_lowercase(), n.id.clone()))
        .collect();
    for r in doc.relations.iter_mut() {
        for end in [&mut r.from, &mut r.to] {
            let key = end.node.trim().to_lowercase();
            if key.is_empty() || doc.nodes.iter().any(|n| n.id == end.node) {
                continue;
            }
            if let Some(real) = id_by_name.get(&key) {
                end.node = real.clone();
            }
        }
    }
    Ok(doc)
}
