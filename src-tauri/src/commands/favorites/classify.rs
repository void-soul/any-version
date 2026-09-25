//! AI 归类：构造 prompt + 解析模型返回的分组。
//!
//! 关键取舍：**整批解析失败就不落库**。宁可下次重跑，也不要把半截错误分类写进去
//! （写错了用户还得一条条改，而 `ai_locked` 会让后续归类跳过它）。

use serde_json::Value;

use super::db::ClassifyItem;

/// 每批送多少条。
///
/// 40 条 × 每行约 100 字符 ≈ 4k 字符，模型输出也不过几百 token；
/// 再大就会开始出现漏归类和 idx 串行。
pub const BATCH_SIZE: usize = 40;

/// 单条送入的字符上限（描述很长的仓库会挤掉别的条目）。
const MAX_TITLE_CHARS: usize = 120;
const MAX_DESC_CHARS: usize = 200;
/// 链接留够长度（带查询串的文档站地址可能很长），但别让它把一行的预算吃光
const MAX_URL_CHARS: usize = 120;

pub const SYSTEM_PROMPT: &str = "你是一个信息整理助手。用户会给你一批收藏条目，每行格式：\n\
`序号|名称|简介|语言|标签|热度|链接`\n\
字段说明：\n\
- 条目可能来自不同平台：GitHub 仓库名形如 `owner/repo`，「语言」是主要编程语言、「标签」是官方 topics、\n\
  「热度」是 star 数（视频/文章条目的语言、标签、热度可能为空，此时靠名称与简介判断）；\n\
- 「链接」是原始地址。浏览器书签常常只有名称和链接（没有简介/语言/标签），此时域名与路径是主要线索；\n\
- 热度只代表流行程度，不是分类依据，仅作参考。\n\
请按**主题**把它们分成若干组，输出**严格 JSON**，不要任何解释文字：\n\
{\"groups\":[{\"name\":\"分类名\",\"items\":[0,1,5]}]}\n\
规则：\n\
1. 分类名用中文，简短（每级不超过 8 个字），组总数不超过 12 个；\n\
2. 分类名可以是多级，用 `/` 分隔层级（如 \"编程语言/Rust\"、\"前端/框架/React\"），层级不超过 3 级；\n\
   没有合适子类时用单级即可，不要为了分层硬凑；\n\
3. 一个序号可以出现在多个组里（多标签），但每个组内的序号不要重复；\n\
4. `items` 里的序号必须是输入里出现过的序号，不要编造；也不要臆造输入里没有的主题；\n\
5. 无法确定归属的放进「其他」；\n\
6. 只输出 JSON。";

/// 把一批条目压成 prompt：每条一行，`序号|名称|简介|语言|标签`。
///
/// 只喂元数据（不抓 README）：上千条逐一抓正文的成本不可接受，
/// 而「名称 + 简介 + 语言 + topics」对归类的判断已经足够。
pub fn build_prompt(items: &[ClassifyItem]) -> String {
    let mut lines = Vec::with_capacity(items.len() + 2);
    lines.push(format!("共 {} 条，请归类：", items.len()));
    for (index, item) in items.iter().enumerate() {
        // 热度（star 数）只在有值时追加：视频/文章条目没有这个字段，
        // 凑一个空段只会让模型困惑该列是什么
        let stars = match item.stars {
            Some(count) if count > 0 => format!("|{}", count),
            _ => String::new(),
        };
        lines.push(format!(
            "{}|{}|{}|{}|{}{}|{}",
            index,
            clamp(&item.title, MAX_TITLE_CHARS),
            clamp(&item.description, MAX_DESC_CHARS),
            clamp(&item.language, 20),
            clamp(&item.topics.join(","), 60),
            stars,
            clamp(&item.url, MAX_URL_CHARS),
        ));
    }
    lines.push("只输出 JSON：".to_string());
    lines.join("\n")
}

fn clamp(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

/// 解析模型输出 → `Vec<(分类路径段, 条目下标列表)>`。
///
/// 分类名允许用 `/` 分隔多级（如「编程语言/Rust」），这里拆成路径段。
/// 容错链：剥代码围栏 → 取首个 `{` 到末个 `}` → 解析 → 校验序号在范围内 → 组内去重。
/// 任一步失败都返回 Err，由调用方决定整批放弃。
pub fn parse_groups(raw: &str, len: usize) -> Result<Vec<(Vec<String>, Vec<usize>)>, String> {
    let json_text = strip_fence(raw).ok_or_else(|| "模型输出里没有找到 JSON".to_string())?;
    let value: Value = serde_json::from_str(&json_text)
        .map_err(|e| format!("模型输出的 JSON 无法解析: {}", e))?;
    let groups = value
        .get("groups")
        .and_then(|g| g.as_array())
        .ok_or_else(|| "模型输出缺少 groups 数组".to_string())?;

    let mut parsed: Vec<(Vec<String>, Vec<usize>)> = Vec::with_capacity(groups.len());
    for group in groups {
        let name = group
            .get("name")
            .and_then(|n| n.as_str())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .ok_or_else(|| "分组缺少 name".to_string())?;
        // 多级分类名按 `/` 拆层级，逐段 trim 并丢弃空段
        let path: Vec<String> = name
            .split('/')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if path.is_empty() {
            return Err(format!("分组「{}」的分类名不能为空", name));
        }
        // items 缺失视为空组（模型可能会对空分类省略 items），不算错误
        let mut indices: Vec<usize> = Vec::new();
        if let Some(items) = group.get("items").and_then(|i| i.as_array()) {
            for item in items {
                let index = item
                    .as_u64()
                    .ok_or_else(|| format!("分组「{}」里有非数字序号", name))?
                    as usize;
                if index >= len {
                    return Err(format!(
                        "模型给出了越界序号 {}（本批只有 {} 条）",
                        index, len
                    ));
                }
                if !indices.contains(&index) {
                    indices.push(index);
                }
            }
        }
        parsed.push((path, indices));
    }
    if parsed.is_empty() {
        return Err("模型没有给出任何分组".to_string());
    }
    Ok(parsed)
}

/// 去掉 ```json 围栏与前后废话，只留最外层花括号包裹的部分。
fn strip_fence(raw: &str) -> Option<String> {
    let text = raw.trim();
    let without_fence = text.strip_prefix("```json").unwrap_or(text);
    let without_fence = without_fence
        .strip_prefix("```")
        .unwrap_or(without_fence);
    let start = without_fence.find('{')?;
    let end = without_fence.rfind('}')?;
    if end < start {
        return None;
    }
    Some(without_fence[start..=end].to_string())
}

#[cfg(test)]
mod tests {
    use super::{build_prompt, parse_groups, BATCH_SIZE};
    use crate::commands::favorites::db::ClassifyItem;

    fn item(title: &str, desc: &str, lang: &str, topics: &[&str]) -> ClassifyItem {
        ClassifyItem {
            id: 1,
            title: title.to_string(),
            description: desc.to_string(),
            language: lang.to_string(),
            topics: topics.iter().map(|t| t.to_string()).collect(),
            stars: None,
            url: String::new(),
        }
    }

    #[test]
    fn prompt_is_one_line_per_item() {
        let items = vec![item("o/r", "a cli", "Rust", &["cli", "rust"])];
        let prompt = build_prompt(&items);
        assert!(prompt.contains("0|o/r|a cli|Rust|cli,rust"), "实际: {}", prompt);
        assert!(prompt.contains("共 1 条"));
    }

    /// star 数作为热度参考附加在行尾；没有热度的条目（视频/文章）不能出现空段。
    #[test]
    fn prompt_appends_stars_only_when_present() {
        let mut starred = item("o/popular", "well known", "Rust", &[]);
        starred.stars = Some(123456);
        let plain = item("BV1xx", "一个视频", "", &[]);

        let prompt = build_prompt(&[starred, plain]);
        assert!(prompt.contains("0|o/popular|well known|Rust||123456"), "实际: {}", prompt);
        assert!(
            prompt.contains("1|BV1xx|一个视频||"),
            "无热度条目不应出现空的星号段: {}",
            prompt
        );
        assert!(!prompt.contains("||||"), "空热度不应留下空段: {}", prompt);
    }

    /// 浏览器书签只有名称和链接：链接必须进 prompt，否则模型没有任何线索。
    #[test]
    fn prompt_includes_url_for_bookmarks() {
        let mut bm = item("Python 官方文档", "", "", &[]);
        bm.url = "https://docs.python.org/3/".to_string();
        let prompt = build_prompt(&[bm]);
        assert!(
            prompt.contains("https://docs.python.org/3/"),
            "书签链接必须喂给模型: {}",
            prompt
        );
    }

    #[test]
    fn prompt_truncates_very_long_descriptions() {
        let long = "描".repeat(500);
        let prompt = build_prompt(&[item("o/r", &long, "Rust", &[])]);
        let line = prompt.lines().nth(1).unwrap();
        assert!(line.chars().count() < 400, "单行过长: {}", line.chars().count());
    }

    #[test]
    fn parses_plain_json() {
        let groups = parse_groups(r#"{"groups":[{"name":"CLI","items":[0,1]}]}"#, 5).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, vec!["CLI".to_string()]);
        assert_eq!(groups[0].1, vec![0, 1]);
    }

    /// 模型最爱包一层 ```json 围栏，也可能在前面说一句"好的"。
    #[test]
    fn parses_fenced_and_chatty_output() {
        let raw = "好的，分类如下：\n```json\n{\"groups\":[{\"name\":\"CLI\",\"items\":[0]}]}\n```";
        assert_eq!(parse_groups(raw, 3).unwrap()[0].0, vec!["CLI".to_string()]);
        assert_eq!(
            parse_groups("```\n{\"groups\":[{\"name\":\"X\",\"items\":[0]}]}\n```", 3).unwrap()[0].0,
            vec!["X".to_string()]
        );
    }

    /// 多级分类名用 `/` 分隔，解析时要拆成路径段。
    #[test]
    fn parses_multilevel_path() {
        let raw = r#"{"groups":[{"name":"编程语言/Rust","items":[0]},{"name":"前端/框架/React","items":[1]}]}"#;
        let groups = parse_groups(raw, 5).unwrap();
        assert_eq!(groups[0].0, vec!["编程语言".to_string(), "Rust".to_string()]);
        assert_eq!(groups[1].0, vec!["前端".to_string(), "框架".to_string(), "React".to_string()]);
        // 多级里也能多标签
        assert_eq!(groups[1].1, vec![1]);
    }

    /// 多标签：同一个序号出现在两个组里都要保留。
    #[test]
    fn multi_label_keeps_item_in_every_group() {
        let raw = r#"{"groups":[{"name":"LLM","items":[0]},{"name":"运维","items":[0,1]}]}"#;
        let groups = parse_groups(raw, 5).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1, vec![0]);
        assert_eq!(groups[1].1, vec![0, 1]);
    }

    #[test]
    fn dedupes_repeated_index_within_one_group() {
        let raw = r#"{"groups":[{"name":"X","items":[1,1,2]}]}"#;
        assert_eq!(parse_groups(raw, 5).unwrap()[0].1, vec![1, 2]);
    }

    /// 越界序号必须报错：写进去就是给不存在的条目打标签。
    #[test]
    fn rejects_out_of_range_index() {
        assert!(parse_groups(r#"{"groups":[{"name":"x","items":[99]}]}"#, 2).is_err());
    }

    /// 垃圾输出（模型拒答 / 返回散文）要报错，由上层整批放弃而不是写半截。
    #[test]
    fn rejects_garbage_instead_of_partial_write() {
        assert!(parse_groups("抱歉，我无法完成这个请求", 5).is_err());
        assert!(parse_groups("", 5).is_err());
        assert!(parse_groups("{\"nope\":1}", 5).is_err());
        assert!(parse_groups(r#"{"groups":[]}"#, 5).is_err());
    }

    /// 缺 items 视为空组（不炸），但缺 name 不行。
    #[test]
    fn missing_items_is_empty_group_but_missing_name_is_error() {
        assert_eq!(parse_groups(r#"{"groups":[{"name":"x"}]}"#, 5).unwrap()[0].1, Vec::<usize>::new());
        assert!(parse_groups(r#"{"groups":[{"items":[0]}]}"#, 5).is_err());
    }

    #[test]
    fn batch_size_is_sane() {
        assert!((20..=60).contains(&BATCH_SIZE));
    }
}
