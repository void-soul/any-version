// ─── 正则表达式工具模块 ───
// 基于 Rust `regex` crate 提供正则测试能力，语法与 JS 略有差异
// （如不支持反向引用），但与 ripgrep/grep -P 一脉相承。

use regex::Regex;
use serde::Serialize;

#[derive(Serialize)]
pub struct RxGroup {
    pub name: Option<String>,
    pub index: usize,
    pub value: String,
}

#[derive(Serialize)]
pub struct RxMatch {
    /// 完整匹配文本
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub groups: Vec<RxGroup>,
}

#[derive(Serialize)]
pub struct RxTestResult {
    pub ok: bool,
    pub error: Option<String>,
    pub matches: Vec<RxMatch>,
    pub total: usize,
    /// 耗时（微秒）
    pub elapsed_us: u128,
    pub replaced: Option<String>,
    pub split_count: usize,
}

fn build_regex(pattern: &str, case_insensitive: bool, multiline: bool, dot_matches_newline: bool) -> Result<Regex, String> {
    let mut cfg = regex::RegexBuilder::new(pattern);
    cfg.case_insensitive(case_insensitive);
    cfg.multi_line(multiline);
    cfg.dot_matches_new_line(dot_matches_newline);
    // 大文本安全上限
    cfg.size_limit(10 * 1024 * 1024);
    cfg.build()
        .map_err(|e| format!("正则编译失败: {}", e))
}

/// 测试正则：返回所有匹配（含分组）、可选替换预览、split 段数。
#[tauri::command]
pub fn rx_test(
    pattern: String,
    text: String,
    replace: Option<String>,
    case_insensitive: bool,
    multiline: bool,
    dot_matches_newline: bool,
) -> Result<RxTestResult, String> {
    let re = build_regex(&pattern, case_insensitive, multiline, dot_matches_newline)?;
    let start = std::time::Instant::now();

    let mut matches = Vec::new();
    for cap in re.captures_iter(&text).take(1000) {
        let whole = cap.get(0).ok_or("无效匹配")?;
        let mut groups = Vec::new();
        for i in 1..cap.len() {
            if let Some(m) = cap.get(i) {
                groups.push(RxGroup {
                    name: re.capture_names().nth(i).flatten().map(|s| s.to_string()),
                    index: i,
                    value: m.as_str().to_string(),
                });
            }
        }
        matches.push(RxMatch {
            text: whole.as_str().to_string(),
            start: whole.start(),
            end: whole.end(),
            groups,
        });
        if matches.len() >= 1000 {
            break;
        }
    }

    let replaced = replace.map(|r| re.replace_all(&text, r.as_str()).to_string());
    let split_count = re.split(&text).count();

    Ok(RxTestResult {
        ok: true,
        error: None,
        total: matches.len(),
        elapsed_us: start.elapsed().as_micros(),
        matches,
        replaced,
        split_count,
    })
}
