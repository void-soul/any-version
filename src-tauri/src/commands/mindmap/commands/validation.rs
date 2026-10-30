use super::*;

/// 通用 JSON 对象请求（流式，文本协议）。

pub(super) fn json_to_mindmap_nodes(
    json: &serde_json::Value,
    document_id: &str,
    id_prefix: &str,
) -> Vec<MindmapNode> {
    let arr = match json.get("nodes").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return vec![],
    };
    let colors = [
        "#22d3ee", "#34d399", "#fbbf24", "#60a5fa", "#fb7185", "#a78bfa", "#f97316", "#f59e0b",
        "#f8fafc", "#94a3b8",
    ];
    let ids: Vec<String> = arr
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let raw = v
                .get("id")
                .and_then(|x| x.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("");
            if raw.is_empty() {
                format!("{}n{}", id_prefix, i + 1)
            } else {
                format!("{}{}", id_prefix, raw)
            }
        })
        .collect();
    let raw_to_id: std::collections::HashMap<&str, String> = arr
        .iter()
        .enumerate()
        .filter_map(|(i, v)| {
            v.get("id")
                .and_then(|x| x.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|raw| (raw, ids[i].clone()))
        })
        .collect();
    arr.iter()
        .enumerate()
        .map(|(i, v)| {
            let c = colors[i % colors.len()];
            let parent_raw = v
                .get("parent_id")
                .or_else(|| v.get("parentId"))
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty() && *s != "null");
            let parent_id = parent_raw.and_then(|raw| raw_to_id.get(raw).cloned());
            let is_root = parent_id.is_none();
            MindmapNode {
                id: ids[i].clone(),
                document_id: document_id.to_string(),
                // 只引用同一批导入节点，未知父级自动成为新的根节点，避免挂到旧树或丢失。
                parent_id,
                name: v
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("未命名")
                    .to_string(),
                detail: {
                    let d = v
                        .get("detail")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if d.is_empty() {
                        // 详情留空时用旧的 description 字段补全（兼容旧数据/AI 仍输出的说明），保证每个节点的详情里都有模块说明
                        let desc = v
                            .get("description")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .trim();
                        if desc.is_empty() {
                            String::new()
                        } else {
                            desc.to_string()
                        }
                    } else {
                        d
                    }
                },
                kind: if is_root {
                    "root".to_string()
                } else {
                    v.get("kind")
                        .and_then(|x| x.as_str())
                        .unwrap_or("other")
                        .to_string()
                },
                color: v
                    .get("color")
                    .and_then(|x| x.as_str())
                    .unwrap_or(c)
                    .to_string(),
                // 证据锚定：sources 数组（项目相对路径），去重、去空、限 6 个
                sources: {
                    let mut out: Vec<String> = Vec::new();
                    if let Some(arr) = v.get("sources").and_then(|x| x.as_array()) {
                        for s in arr.iter().filter_map(|x| x.as_str()) {
                            let t = s.trim().trim_start_matches("./").to_string();
                            if !t.is_empty() && !out.contains(&t) {
                                out.push(t);
                            }
                            if out.len() >= 6 {
                                break;
                            }
                        }
                    }
                    out
                },
                position_x: 0.0,
                position_y: 0.0,
            }
        })
        .collect()
}

pub(super) fn ensure_import_root(
    nodes: &mut Vec<MindmapNode>,
    document_id: &str,
    id_prefix: &str,
    name: &str,
    summary: &str,
) {
    if nodes.iter().any(|n| n.parent_id.is_none()) {
        return;
    }
    nodes.insert(
        0,
        MindmapNode {
            id: format!("{}root", id_prefix),
            document_id: document_id.to_string(),
            parent_id: None,
            name: name.to_string(),
            detail: if summary.is_empty() {
                "AI 导入根节点".to_string()
            } else {
                summary.to_string()
            },
            kind: "root".to_string(),
            color: "#f8fafc".to_string(),
            sources: Vec::new(),
            position_x: 0.0,
            position_y: 0.0,
        },
    );
}

/// 从节点名称/描述提取内容关键词（中英混合）：名称整体 + 英文词 + 中文片段及其 2-gram。
pub(super) fn node_keywords(name: &str, description: &str) -> Vec<String> {
    const STOP_EN: &[&str] = &[
        "the",
        "and",
        "for",
        "with",
        "this",
        "that",
        "from",
        "are",
        "was",
        "has",
        "have",
        "not",
        "its",
        "all",
        "will",
        "can",
        "use",
        "using",
        "used",
        "module",
        "service",
        "component",
        "file",
        "node",
        "config",
        "data",
        "info",
        "main",
        "api",
        "user",
        "order",
        "list",
        "view",
        "page",
        "src",
        "new",
        "out",
        "set",
        "get",
        "add",
        "del",
        "一个",
        "这个",
        "以及",
        "相关",
        "进行",
        "提供",
        "支持",
        "处理",
        "管理",
        "结构",
        "内部",
        "主要",
        "描述",
        "用于",
        "负责",
        "实现",
        "功能",
        "模块",
        "服务",
        "组件",
        "节点",
        "配置",
        "文件",
        "系统",
    ];
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String, is_name: bool| {
        let t = s.trim();
        if t.len() >= 2 && !out.iter().any(|x| x == t) && (is_name || !STOP_EN.contains(&t)) {
            out.push(t.to_string());
        }
    };
    // 名称整体始终保留（最强信号）
    let name_t = name.trim();
    if name_t.len() >= 2 {
        push(name_t.to_string(), true);
    }
    // 名称+描述：英文词 / 中文连续片段
    let raw = format!("{} {}", name, description);
    let mut buf = String::new();
    let mut mode = 0u8; // 0=间隔 1=ascii 2=cjk
    for ch in raw.chars() {
        let seg = if ch.is_ascii_alphanumeric() || ch == '_' {
            1
        } else if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            2
        } else {
            0
        };
        if seg != mode {
            if mode != 0 {
                push(std::mem::take(&mut buf), false);
            }
            mode = seg;
        }
        if seg != 0 {
            buf.push(ch);
        }
    }
    if mode != 0 {
        push(buf, false);
    }
    // 中文长片段补充 2-gram（滑动窗口），提高与代码注释的命中
    let cjk_runs: Vec<String> = out
        .iter()
        .filter(|k| k.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) && k.len() >= 3)
        .cloned()
        .collect();
    for run in cjk_runs {
        let chars: Vec<char> = run.chars().collect();
        for w in chars.windows(2) {
            let bigram: String = w.iter().collect();
            if !out.iter().any(|x| x == &bigram) {
                out.push(bigram);
            }
        }
    }
    out.truncate(12);
    out
}

/// 校验 AI 返回的 nodes JSON 结构，返回错误列表（空 = 通过）。
/// 借鉴 Archify 的「校验驱动」：结构/语义检查，为修复循环提供可反馈的诊断。
/// project 非空时额外做证据校验：sources 必须真实存在于扫描结果中，
/// 且文件内容/路径须与节点说明相关（防标注错误文件）。
pub(crate) fn validate_ai_nodes_json(
    json: &serde_json::Value,
    project: Option<&crate::commands::mindmap::scan::ProjectFiles>,
) -> Vec<String> {
    let mut errs: Vec<String> = Vec::new();
    let Some(arr) = json.get("nodes").and_then(|v| v.as_array()) else {
        return vec!["缺少 nodes 数组".into()];
    };
    if arr.is_empty() {
        return vec!["nodes 为空".into()];
    }
    if arr.len() > 80 {
        errs.push(format!("节点数量 {} 超过上限 80", arr.len()));
    }
    let mut ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut roots = 0usize;
    for (i, n) in arr.iter().enumerate() {
        let tag = format!(
            "节点 #{}（{}）",
            i + 1,
            n.get("name").and_then(|x| x.as_str()).unwrap_or("?")
        );
        let name = n.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if name.trim().is_empty() {
            errs.push(format!("{}：name 为空", tag));
        }
        if let Some(k) = n.get("kind").and_then(|x| x.as_str()) {
            if !NODE_KINDS.contains(&k) {
                errs.push(format!("{}：kind '{}' 不在允许列表", tag, k));
            }
        }
        if let Some(c) = n.get("color").and_then(|x| x.as_str()) {
            let ok = c.is_empty()
                || (c.len() == 7
                    && c.starts_with('#')
                    && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()));
            if !ok {
                errs.push(format!("{}：color '{}' 需为 #RRGGBB", tag, c));
            }
        }
        let raw_id = n.get("id").and_then(|x| x.as_str()).unwrap_or("");
        if raw_id.trim().is_empty() {
            errs.push(format!("{}：id 为空", tag));
        } else if !ids.insert(raw_id) {
            errs.push(format!("{}：id '{}' 重复", tag, raw_id));
        }
        let pid = n
            .get("parent_id")
            .or_else(|| n.get("parentId"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty() && *s != "null");
        match pid {
            None => roots += 1,
            Some(p)
                if !arr
                    .iter()
                    .any(|m| m.get("id").and_then(|x| x.as_str()) == Some(p)) =>
            {
                errs.push(format!("{}：parent_id '{}' 未在本批节点中定义", tag, p));
            }
            _ => {}
        }
    }
    if roots == 0 {
        errs.push("缺少根节点（应至少一个节点 parent_id 为 null）".into());
    } else if roots > 5 {
        errs.push(format!("根节点数量 {} 过多", roots));
    }
    // 证据锚定校验：sources 必须真实存在于扫描结果，且路径/内容与节点说明相关（防标注错误文件）
    if let Some(project) = project {
        let files = &project.files;
        // 内容读取缓存（同一文件被多节点引用时只读一次），仅读 ≤256KB、前 16K 字符
        let mut cache: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        let mut read_capped = |rel: &str| -> Option<String> {
            if let Some(v) = cache.get(rel) {
                return v.clone();
            }
            let path = project.root.join(rel);
            let v = (|| {
                let meta = std::fs::metadata(&path).ok()?;
                if meta.len() > 256 * 1024 {
                    return None;
                }
                let data = std::fs::read_to_string(&path).ok()?;
                Some(
                    data.chars()
                        .take(16 * 1024)
                        .collect::<String>()
                        .to_lowercase(),
                )
            })();
            cache.insert(rel.to_string(), v.clone());
            v
        };
        for (i, n) in arr.iter().enumerate() {
            let Some(srcs) = n.get("sources").and_then(|x| x.as_array()) else {
                continue;
            };
            let name = n.get("name").and_then(|x| x.as_str()).unwrap_or("");
            // 旧数据/AI 兼容：description 已废弃，内容并入 detail 后用于关键词
            let desc = n.get("description").and_then(|x| x.as_str()).unwrap_or("");
            let detail = n.get("detail").and_then(|x| x.as_str()).unwrap_or("");
            let kws: Vec<String> = node_keywords(name, &format!("{desc} {detail}"))
                .into_iter()
                .map(|k| k.to_lowercase())
                .collect();
            let tag = format!("节点 #{}（{}）", i + 1, name);
            for s in srcs.iter().filter_map(|x| x.as_str()) {
                let p = s.trim().trim_start_matches("./").to_string();
                if p.is_empty() {
                    continue;
                }
                let exists =
                    files.contains(&p) || files.iter().any(|f| f.starts_with(&format!("{}/", p)));
                if !exists {
                    errs.push(format!(
                        "{}：sources '{}' 不在扫描结果中（请只引用目录结构中真实存在的文件）",
                        tag, s
                    ));
                    continue;
                }
                // 路径自证：相对路径含关键词即视为相关（如节点名为 tsconfig.json、文件同名）
                let p_lower = p.to_lowercase();
                if !kws.is_empty() && kws.iter().any(|k| p_lower.contains(k.as_str())) {
                    continue;
                }
                // 内容相关度：读取文件内容，需命中至少一个关键词；无法读取（二进制/超大）则跳过
                if kws.is_empty() {
                    continue;
                }
                if let Some(content) = read_capped(&p) {
                    let hit = kws.iter().any(|k| content.contains(k.as_str()));
                    if !hit {
                        errs.push(format!(
                            "{}：sources '{}' 内容与节点说明不相关（未找到关键词『{}』，请核实标注的文件确实实现该模块，或移除 sources）",
                            tag,
                            s,
                            kws.iter().take(3).map(|k| k.as_str()).collect::<Vec<_>>().join("、")
                        ));
                    }
                }
            }
        }
    }
    errs
}
