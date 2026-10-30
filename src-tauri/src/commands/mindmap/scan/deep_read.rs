use super::*;

/// 从 AI 请求 JSON 提取合法相对路径：
/// - paths: 数组字符串（去 ./ 前缀、反斜杠→正斜杠、去重）
/// - dirs: 目录路径 → 展开/标记为「目录请求」（不读内容，只确认存在）
/// 返回 (files, dirs, rejected)：前两者均过滤为存在于文件集内的路径；
/// rejected 是模型点了但文件集里不存在/格式无效的路径（回执给模型改点）。
pub fn parse_ai_file_request(
    json: &serde_json::Value,
    project: &ProjectFiles,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    let normalize = |s: &str| -> String {
        s.trim()
            .trim_start_matches("./")
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string()
    };
    if let Some(arr) = json.get("paths").and_then(|v| v.as_array()) {
        for p in arr.iter().filter_map(|x| x.as_str()) {
            let rel = normalize(p);
            if rel.is_empty() || files.contains(&rel) {
                continue;
            }
            if project.files.contains(&rel) {
                files.push(rel);
            } else {
                rejected.push(rel);
            }
        }
    }
    if let Some(arr) = json.get("dirs").and_then(|v| v.as_array()) {
        for d in arr.iter().filter_map(|x| x.as_str()) {
            let rel = normalize(d);
            if rel.is_empty() || dirs.contains(&rel) {
                continue;
            }
            // 目录请求：确认真实存在（有文件以它为前缀）
            if project
                .files
                .iter()
                .any(|f| f.starts_with(&format!("{}/", rel)))
            {
                dirs.push(rel);
            } else {
                rejected.push(rel);
            }
        }
    }
    (files, dirs, rejected)
}

/// 读取单个文件并压缩为上下文文本（去尾部空白、压缩连续空行、截断）。
/// 返回 (文本, 是否被截断)。不可读（二进制/超大）返回 (None, false)。
pub fn read_file_compressed(root: &Path, rel: &str, max_chars: usize) -> (Option<String>, bool) {
    let path = root.join(rel);
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(_) => return (None, false),
    };
    if meta.len() > 256 * 1024 {
        return (None, false); // 二进制/超大：不可读
    }
    let Ok(data) = std::fs::read_to_string(&path) else {
        return (None, false); // 二进制/非 UTF-8
    };
    // 压缩：去行尾空白、折叠 2+ 连续空行为 1、去行首空白
    let mut compressed = String::with_capacity(data.len());
    let mut blank_run = 0usize;
    for line in data.lines() {
        let t = line.trim_end();
        if t.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            compressed.push('\n');
        } else {
            blank_run = 0;
            compressed.push_str(t);
            compressed.push('\n');
        }
    }
    let truncated = compressed.chars().count() > max_chars;
    let text: String = compressed.chars().take(max_chars).collect();
    (Some(text), truncated)
}

/// 把一批文件读取结果压缩为 AI 上下文文本块。
/// files: (rel, content, truncated)；dirs: 目录确认列表。
/// 请求回执段：AI 点单中「在文件集里找不到/格式无效」的路径，反馈给模型让它
/// 下一轮改点真实存在的文件（否则模型可能反复点同一批不存在的路径，白白烧轮次）。
pub(super) fn format_request_rejected(rejected: &[String]) -> String {
    if rejected.is_empty() {
        return String::new();
    }
    format!(
        "## 请求回执（以下路径不在目录结构中或格式无效，未被读取；请改点『目录结构』中真实存在的文件）\n{}\n\n",
        rejected.join("\n")
    )
}

pub fn format_file_batch(
    files: &[(String, Option<String>, bool)],
    dirs: &[String],
    rejected: &[String],
) -> String {
    let mut out = String::new();
    out.push_str(&format_request_rejected(rejected));
    if !dirs.is_empty() {
        out.push_str(&format!("## 目录确认\n{}\n\n", dirs.join("\n")));
    }
    if files.is_empty() {
        out.push_str("## 文件内容\n（本次请求没有可读文件）\n");
        return out;
    }
    out.push_str("## 文件内容\n");
    for (rel, content, truncated) in files {
        match content {
            Some(text) => {
                out.push_str(&format!(
                    "\n=== {} ==={}\n{}\n",
                    rel,
                    if *truncated { "（已截断）" } else { "" },
                    text
                ));
            }
            None => {
                out.push_str(&format!(
                    "\n=== {} ===\n（不可读：二进制或超出大小限制）\n",
                    rel
                ));
            }
        }
    }
    out
}
