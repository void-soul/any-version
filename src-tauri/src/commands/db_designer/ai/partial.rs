use super::*;

/// 从**可能还没写完**的 JSON 文本里，抠出已经写完整的数组元素。
///
/// 逐字符扫 + 括号配对（**字符串内部的括号不参与配对**），所以注释 / 默认值里的
/// 花括号不会打架；`]` 出现在数组层（depth==0）就说明这个数组结束了。
/// 这是「边收边落图」的关键：AI 每写完一张表，我们就能把它画上画布，
/// 不必等整个文档到齐（100 张表要等几十秒，全程空白很难受）。
pub(super) fn complete_elements(text: &str, key: &str) -> Vec<serde_json::Value> {
    let needle = format!("\"{}\"", key);
    let Some(k) = text.find(&needle) else {
        return Vec::new();
    };
    let Some(start) = text[k..].find('[').map(|i| k + i) else {
        return Vec::new();
    };
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let mut obj_start: Option<usize> = None;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else {
            match b {
                b'"' => in_str = true,
                b'{' => {
                    if depth == 0 {
                        obj_start = Some(i);
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth <= 0 {
                        // 一个元素收尾了。解析失败就丢掉（多半是还没写完）
                        if let Some(s) = obj_start {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[s..=i]) {
                                out.push(v);
                            }
                        }
                        obj_start = None;
                        depth = 0;
                    }
                }
                b']' if depth == 0 => break,
                _ => {}
            }
        }
        i += 1;
    }
    out
}

/// 抓顶层 `"key": "value"` 里的 value（只找第一次出现，够用即可）。
pub(super) fn grab_string(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let k = text.find(&needle)?;
    let rest = &text[k + needle.len()..];
    let a = rest.find('"')?;
    let b = rest[a + 1..].find('"').map(|i| a + 1 + i)?;
    Some(rest[a + 1..b].to_string())
}

/// 把「当前累积到的文本」尽力解析成一份**部分文档**（只含已经写完整的节点 / 关系）。
/// 解析不了的就当还没有 —— 下一 tick 内容更多时会再试。
pub(super) fn extract_partial(text: &str) -> DbDesignDocument {
    let mut doc = new_document("", "mysql");
    for v in complete_elements(text, "nodes") {
        if let Ok(n) = serde_json::from_value::<DbDesignNode>(v) {
            doc.nodes.push(n);
        }
    }
    for v in complete_elements(text, "relations") {
        if let Ok(r) = serde_json::from_value::<DbDesignRelation>(v) {
            doc.relations.push(r);
        }
    }
    // 补 id：AI 漏写 id 时节点现在能解析通过了（`id` 改成缺了当空），但空 id 会让
    // 前端 React key 撞车、关联也引用不到 —— 所以这里按 t1/t2、r1/r2 补齐。
    // 与 parse_document 用同一套编号，流式落图和最终结果对得上。
    let mut node_ids: Vec<String> = doc.nodes.iter().map(|n| n.id.clone()).collect();
    fill_missing_ids(&mut node_ids, "t");
    for (n, id) in doc.nodes.iter_mut().zip(node_ids) {
        n.id = id;
    }
    let mut rel_ids: Vec<String> = doc.relations.iter().map(|r| r.id.clone()).collect();
    fill_missing_ids(&mut rel_ids, "r");
    for (r, id) in doc.relations.iter_mut().zip(rel_ids) {
        r.id = id;
    }
    if let Some(d) = grab_string(text, "dialect") {
        if crate::commands::db_designer::models::DIALECTS.contains(&d.as_str()) {
            doc.dialect = d;
        }
    }
    doc
}

/// 推送一份「到目前为止画得出来的图」（`dbd-ai-partial`）。
/// 前端拿到就立刻落画布，所以这一路是**流式构图**，与思维导图观感一致。
pub(super) fn emit_partial(app: &Option<tauri::AppHandle>, run_id: &str, doc: &DbDesignDocument) {
    use tauri::Emitter as _;
    if let Some(handle) = app {
        let _ = handle.emit(
            "dbd-ai-partial",
            serde_json::json!({ "runId": run_id, "doc": doc }),
        );
    }
}

/// 给一批元素补 id：缺的按 `t1` / `r1` 这样的序号补，并且**不和 AI 自己写的 id 撞车**
/// （AI 可能只给一部分元素写 id，编号撞上去会覆盖它的引用关系）。
pub(super) fn fill_missing_ids(ids: &mut Vec<String>, prefix: &str) {
    let used: std::collections::HashSet<String> = ids
        .iter()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .collect();
    let mut seq = 1usize;
    for id in ids.iter_mut() {
        if !id.trim().is_empty() {
            continue;
        }
        loop {
            let cand = format!("{}{}", prefix, seq);
            seq += 1;
            if !used.contains(&cand) {
                *id = cand;
                break;
            }
        }
    }
}

/// 本地 id：AI 常常不写 `id`，而关联要按 id 引用节点，不补就全乱了。
pub(super) fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}", prefix, seq)
}
