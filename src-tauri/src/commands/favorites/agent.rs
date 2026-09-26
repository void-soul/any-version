//! 收藏搜索 Agent：用户用自然语言提需求，Agent 在本地收藏库里检索并整理成清单。
//!
//! 与安装助手同一套写法（抄 EchoBird 的 agent 形态、收敛到 Kira 已有的能力）：
//! - 只给三个只读动作（检索 / 看正文 / 看统计），**绝不写库**——收藏是用户的心血，
//!   agent 手滑一下就找不回来了；
//! - 模型可自由选择（前端选供应商 + 模型，后端只做回退）；
//! - 输出要求整理成整齐的清单（分组 + 一句话推荐理由 + 链接），而不是把原始记录倒出来。

use serde_json::json;
use tauri::Emitter;

use crate::commands::ai::channel::{complete_chat_messages, CompleteOutcome, NoHooks};
use crate::commands::ai::config::load_ai_config;
use crate::commands::ai::models::AiProvider;
use super::db;

/// Agent 可用的工具（OpenAI function calling 格式）。
pub const FAV_AGENT_TOOLS: &str = r##"[
  { "type": "function", "function": { "name": "search_favorites", "description": "在本地收藏库里检索（匹配标题与简介）。关键词只给**核心名词**，多词会自动按空格/顿号拆开分别匹配并按命中数排序。关键词留空或不传 = 不按词过滤，按最近更新浏览一批（用于「我最近收藏了什么」这类需求，或关键词全落空时的兜底）。", "parameters": { "type": "object", "properties": { "keyword": { "type": "string", "description": "核心名词（可选；留空 = 浏览最近更新）" }, "tag": { "type": "string", "description": "只在该分类下找（可选，分类名或「父/子」路径；先用 list_favorite_tags 看有哪些）" }, "source": { "type": "string", "description": "只在该来源找：github / bilibili / zhihu（可选）" }, "limit": { "type": "integer", "description": "最多返回几条，默认 10，最多 20" } } } } },
  { "type": "function", "function": { "name": "get_favorite_content", "description": "读取某条收藏缓存下来的正文（README 等），用于确认它到底讲什么、值不值得推荐。正文可能很长，只返回前若干字符。", "parameters": { "type": "object", "properties": { "id": { "type": "integer", "description": "收藏条目的 id（来自 search_favorites 的结果）" } }, "required": ["id"] } } },
  { "type": "function", "function": { "name": "list_favorite_tags", "description": "列出收藏库里的分类及各分类条数，用于判断从哪个分类入手。", "parameters": { "type": "object", "properties": {} , "required": [] } } }
]"##;

/// 事件名：前端订阅后渲染「检索到 N 条 / 读了某条正文 / 完成」。
pub const FAV_AGENT_EVENT: &str = "favorites-agent-progress";

/// 单条正文最多给模型看多少字符（README 动辄几万字符，会把上下文撑爆）。
const CONTENT_CHAR_LIMIT: usize = 1500;
/// 单次检索结果最多给模型看多少字符（超了就截断）。
const TOOL_RESULT_CHAR_LIMIT: usize = 3000;
/// 整个会话累计的检索结果字符预算：超了就把**最早**的检索结果压成「id + 标题」，
/// 否则第 6 轮还在重发第 1 轮的 10 条完整记录（每轮都重发，token 是二次增长的）。
const CONTEXT_CHAR_BUDGET: usize = 12_000;
/// 检索默认 / 上限条数（20 条 × 完整 URL + 简介太重，10 条足够整理成清单）。
const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 20;
/// 简介最长字符（原 120 字是主要体积来源）。
const DESC_CHAR_LIMIT: usize = 60;
/// 分类列表最多给多少行（分类多的库整棵树倒出来又是一大坨）。
const MAX_TAG_LINES: usize = 40;

/// 一次检索会话的记账状态（跨轮次）。
#[derive(Default)]
struct FavAgentState {
    /// 已经给模型看过的条目 id：跨轮去重，避免同一批结果被反复塞进上下文
    seen: std::collections::HashSet<i64>,
    /// 每条 tool 结果在 `messages` 里的下标与字符数，用于超预算时折叠最旧的
    results: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavAgentStep {
    /// "thinking" | "toolCall" | "toolResult" | "done"
    pub step: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavAgentReply {
    /// 整理好的结果（Markdown）
    pub text: String,
    pub steps: Vec<FavAgentStep>,
}

fn emit(app: &tauri::AppHandle, step: &FavAgentStep) {
    let _ = app.emit(FAV_AGENT_EVENT, step.clone());
}

/// 上下文超预算时，把**最早**的 tool 结果压成一行（保留 id + 标题即可）。
///
/// 每轮都会重发完整历史，越到后面越贵；折叠最旧的那些（模型通常已经消化过）
/// 能砍掉大部分重复体积，又不丢已检索到的候选。
fn compact_context(messages: &mut Vec<serde_json::Value>, state: &mut FavAgentState) {
    let total: usize = state.results.iter().map(|(_, len)| *len).sum();
    let mut budget = total;
    if budget <= CONTEXT_CHAR_BUDGET {
        return;
    }
    for (idx, len) in state.results.iter_mut() {
        if budget <= CONTEXT_CHAR_BUDGET {
            break;
        }
        let Some(msg) = messages.get_mut(*idx) else { continue; };
        let Some(content) = msg.get("content").and_then(|v| v.as_str()) else { continue; };
        // 折叠：只留前 200 字符（够看到「命中 N 条」和前几条标题）
        let head: String = content.chars().take(200).collect();
        let condensed = format!("{head}…（较早的结果已折叠，需要明细请用原关键词重新检索）");
        *msg = json!({ "role": "tool", "tool_call_id": msg.get("tool_call_id").cloned().unwrap_or(serde_json::Value::Null), "content": condensed });
        let new_len = condensed.chars().count();
        budget = budget.saturating_sub(*len) + new_len;
        *len = new_len;
    }
}

fn system_prompt() -> String {
    "你是 Kira 的「收藏检索助手」。用户会用自然语言说出他想要找的东西，你在他本机收藏库\
     （GitHub star / B站收藏 / 知乎收藏）里检索，并把结果整理成整齐的清单。\n\n\
     【工作方式】\n\
     1. 直接用 search_favorites 检索：关键词只给**核心名词**（会自动拆成多词分别匹配）；\n\
        拿不准有哪些分类、或想按分类收敛时，再调 list_favorite_tags；\n\
     2. 换 1~2 个更短的词再试一次即可；**关键词都落空就把 keyword 留空**（按最近更新浏览）；\n\
     3. 对拿不准的条目，用 get_favorite_content 看一眼正文再决定要不要推荐（最多看 2~3 条，省 token）；\n\
     4. 马上给出整理好的结果，别为了凑条目反复检索。\n\n\
     【输出格式（必须）】\n\
     用 Markdown：\n\
     - 开头一句「共找到 N 条相关收藏」；\n\
     - 按主题分 2~4 组，每组一个三级标题；\n\
     - 每条一行：`- [标题](链接) —— 一句话说明为什么符合需求`；\n\
     - 末尾用一行「其中《X》最贴合你的需求，因为……」给出首选建议。\n\n\
     【硬规则】\n\
     - 只推荐**检索结果里真实存在**的条目：严禁编造标题或链接。\n\
     - 检索不到就直说没找到，并说明试过哪些关键词，不要拿不相干的条目凑数。\n\
     - 全程用中文，简洁，不写客套话。\n"
        .to_string()
}

fn pick_provider_model(
    provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<(AiProvider, String), String> {
    let config = load_ai_config();
    let provider = provider_id
        .and_then(|pid| config.providers.iter().find(|p| p.id == pid))
        .or_else(|| config.providers.iter().find(|p| !p.api_key.trim().is_empty()))
        .ok_or_else(|| "还没有配置可用的供应商（请先在 AI 模块「模型」页添加一个）".to_string())?
        .clone();
    let model = model_id
        .filter(|m| !m.trim().is_empty())
        .map(|m| m.to_string())
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| format!("供应商「{}」下没有可用模型", provider.name))?;
    Ok((provider, model))
}

/// 收藏检索 Agent 入口：跑一轮「思考 + 检索」循环，返回整理好的结果。
#[tauri::command]
pub async fn fav_agent_search(
    app: tauri::AppHandle,
    provider_id: Option<String>,
    model_id: Option<String>,
    prompt: String,
) -> Result<FavAgentReply, String> {
    let text = prompt.trim();
    if text.is_empty() {
        return Err("请先说出你想找什么".to_string());
    }
    let (provider, model) = pick_provider_model(provider_id.as_deref(), model_id.as_deref())?;
    // 轮数来自模块设置（与思维导图「Agent 轮数」同一口径），不再硬编码
    let max_rounds = super::settings::load_settings().agent_rounds;

    let mut messages: Vec<serde_json::Value> = vec![
        json!({ "role": "system", "content": system_prompt() }),
        json!({ "role": "user", "content": text }),
    ];
    let mut steps: Vec<FavAgentStep> = Vec::new();
    let mut state = FavAgentState::default();

    for _round in 0..max_rounds {
        let thinking = FavAgentStep { step: "thinking".to_string(), text: "检索中…".to_string(), tool: None };
        steps.push(thinking.clone());
        emit(&app, &thinking);

        let outcome: CompleteOutcome = complete_chat_messages(
            &NoHooks,
            &provider,
            &model,
            &messages,
            0.3,
            Some(FAV_AGENT_TOOLS),
            // 记账下沉到通道：tool_id 必传，收藏检索的消耗自动进 AI 模块用量面板
            crate::commands::ai::usage::tool_ids::FAVORITES,
        )
        .await?;

        let tool_calls = outcome
            .message
            .as_ref()
            .and_then(|m| m.get("tool_calls"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if tool_calls.is_empty() {
            let answer = if outcome.text.trim().is_empty() {
                "没能整理出结果，请换个说法再试一次。".to_string()
            } else {
                outcome.text.clone()
            };
            let done = FavAgentStep { step: "done".to_string(), text: answer.clone(), tool: None };
            steps.push(done.clone());
            emit(&app, &done);
            return Ok(FavAgentReply { text: answer, steps });
        }

        if let Some(message) = outcome.message.clone() {
            messages.push(message);
        }
        for call in tool_calls {
            let id = call.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let name = call
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let args_raw = call
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let args: serde_json::Value = serde_json::from_str(args_raw).unwrap_or(json!({}));

            let call_step = FavAgentStep {
                step: "toolCall".to_string(),
                text: name.clone(),
                tool: Some(name.clone()),
            };
            steps.push(call_step.clone());
            emit(&app, &call_step);

            let result = run_tool(&mut state, &name, &args);
            let result_step = FavAgentStep {
                step: "toolResult".to_string(),
                text: result.clone(),
                tool: Some(name.clone()),
            };
            steps.push(result_step.clone());
            emit(&app, &result_step);

            state.results.push((messages.len(), result.chars().count()));
            messages.push(json!({ "role": "tool", "tool_call_id": id, "content": result }));
        }
        // 每轮结束压一次上下文：超预算就把最早的检索结果折叠成一行，
        // 否则后面每一轮都要重发前面所有完整结果（token 二次增长）。
        compact_context(&mut messages, &mut state);
    }

    messages.push(json!({ "role": "user", "content": "请直接给出整理好的结果（不要再检索）。" }));
    let final_outcome = complete_chat_messages(
        &NoHooks,
        &provider,
        &model,
        &messages,
        0.3,
        None,
        crate::commands::ai::usage::tool_ids::FAVORITES,
    )
    .await?;
    let answer = if final_outcome.text.trim().is_empty() {
        "检索步数用完了还没整理好，请缩小需求范围再试。".to_string()
    } else {
        final_outcome.text.clone()
    };
    let done = FavAgentStep { step: "done".to_string(), text: answer.clone(), tool: None };
    steps.push(done.clone());
    emit(&app, &done);
    Ok(FavAgentReply { text: answer, steps })
}

/// 把查询拆成多个检索词：「Rust 爬虫」这种传单个子串必然 0 命中（SQL 是 `LIKE '%词%'`）。
/// 分隔符覆盖空格、中英文标点。
pub fn split_terms(keyword: &str) -> Vec<String> {
    keyword
        .split(|c: char| {
            c.is_whitespace()
                || matches!(c, '、' | '，' | ',' | '。' | '.' | '；' | ';' | '／' | '/' | '｜' | '|' | '+' | '＋')
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .take(4) // 再多就是噪音，也浪费往返
        .collect()
}

/// 分类名 → id。工具给模型看的是「父/子」路径，而库里只存叶子名字，
/// 所以先按全名试、再按最后一段试——否则 tag 过滤会静默退化成「不过滤」。
fn resolve_category(tag: &str) -> Option<i64> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    let direct = db::with_conn(|conn| db::find_category_by_name(conn, tag))
        .ok()
        .flatten();
    if direct.is_some() {
        return direct;
    }
    // 「前端/构建」→ 拿「构建」
    tag.rsplit('/')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|leaf| db::with_conn(|conn| db::find_category_by_name(conn, leaf)).ok().flatten())
}

/// 一次检索：多词分别查再合并打分（标题命中权重高于简介命中），最后去重。
fn tool_search_favorites(state: &mut FavAgentState, args: &serde_json::Value) -> String {
    let keyword = args.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_SEARCH_LIMIT as u64)
        .clamp(1, MAX_SEARCH_LIMIT as u64) as usize;
    // 分类：只查不建 —— 模型瞎猜的名字不该在检索时变成一个新分类
    let category_id = args
        .get("tag")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(resolve_category);
    let source = args.get("source").and_then(|v| v.as_str()).map(|s| s.to_string());

    let terms = split_terms(keyword);
    // 关键词为空 → 浏览模式：不带关键词，按最近更新取一批
    let queries: Vec<String> = if terms.is_empty() {
        vec![String::new()]
    } else {
        terms.clone()
    };

    // id → (分数, 行)：多词命中累加，标题命中权重更高
    let mut scored: std::collections::HashMap<i64, (i32, db::FavoriteRow)> =
        std::collections::HashMap::new();
    for q in &queries {
        let filter = db::ListFilter {
            keyword: Some(q.clone()),
            category_id,
            source: source.clone(),
            // 每个词多取一些，合并后才有的选
            limit: (limit * 3).min(MAX_SEARCH_LIMIT * 3),
            ..Default::default()
        };
        let Ok(rows) = db::with_conn(|conn| db::list(conn, &filter)) else {
            continue;
        };
        let needle = q.to_lowercase();
        for r in rows {
            let score = if needle.is_empty() {
                1
            } else {
                let title_hit = r.title.to_lowercase().contains(&needle);
                let desc_hit = r
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&needle);
                if title_hit {
                    2
                } else if desc_hit {
                    1
                } else {
                    1
                }
            };
            scored
                .entry(r.id)
                .and_modify(|(s, _)| *s += score)
                .or_insert((score, r));
        }
    }

    let mut hits: Vec<(i32, db::FavoriteRow)> = scored.into_values().collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0));

    // 去重：之前几轮已经看过的条目不再整条重复列出（省 token，也避免模型重复推荐）
    let fresh: Vec<(i32, db::FavoriteRow)> = hits
        .iter()
        .filter(|(_, r)| !state.seen.contains(&r.id))
        .take(limit)
        .map(|(s, r)| (*s, r.clone()))
        .collect();
    if fresh.is_empty() {
        return if hits.is_empty() {
            if keyword.trim().is_empty() {
                "收藏库里没有匹配的条目".to_string()
            } else {
                format!("关键词「{}」没有命中任何收藏（可换个更短的核心名词，或留空按最近更新浏览）", keyword)
            }
        } else {
            "本次命中的条目都已在前面的结果里出现过（不重复列出），请换关键词或换分类".to_string()
        };
    }

    let total = fresh.len();
    let body = fresh
        .iter()
        .map(|(_, r)| {
            state.seen.insert(r.id);
            let desc: String = r
                .description
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(DESC_CHAR_LIMIT)
                .collect();
            // 只给模型真正用得上的字段：id（读正文）/ 标题 / 链接（输出清单）/ 简介（判断相关性）
            // 去掉了「收藏于」——几乎不影响推荐，却是每条的固定体积。
            format!(
                "id={} | 《{}》 | 来源={} | 分类={} | 链接={} | 简介={}",
                r.id,
                r.title,
                r.source,
                if r.tags.is_empty() { "未分类".to_string() } else { r.tags.join("/") },
                r.url,
                desc
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    truncate_chars(&format!("命中 {} 条：\n{}", total, body), TOOL_RESULT_CHAR_LIMIT)
}

/// 截断到指定字符数（中文按字符切，避免切出半个 UTF-8）。
fn truncate_chars(text: &str, limit: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        return text.to_string();
    }
    let mut out: String = chars[..limit].iter().collect();
    out.push_str("…（已截断）");
    out
}

/// 执行一个检索动作（全部只读）。
fn run_tool(state: &mut FavAgentState, name: &str, args: &serde_json::Value) -> String {
    match name {
        "search_favorites" => tool_search_favorites(state, args),
        "get_favorite_content" => {
            let Some(id) = args.get("id").and_then(|v| v.as_i64()) else {
                return "缺少 id".to_string();
            };
            match db::with_conn(|conn| db::get_content(conn, id)) {
                Ok(Some(content)) => {
                    let body: String = content.text.chars().take(CONTENT_CHAR_LIMIT).collect();
                    if body.is_empty() {
                        "这条没有缓存正文（可先在列表里打开一次让它缓存）".to_string()
                    } else {
                        format!("正文（前 {} 字）：{}", CONTENT_CHAR_LIMIT, body)
                    }
                }
                Ok(None) => "这条没有缓存正文（可先在列表里打开一次让它缓存）".to_string(),
                Err(e) => format!("读取失败：{}", e),
            }
        }
        "list_favorite_tags" => match db::with_conn(|conn| db::stats(conn)) {
            Ok(stats) => {
                if stats.categories.is_empty() {
                    "收藏库里还没有任何分类".to_string()
                } else {
                    // 分类是多级的：给模型看「父/子」路径，它才知道该拿哪一层去筛
                    fn walk(nodes: &[db::CategoryNode], prefix: &str, out: &mut Vec<String>) {
                        for n in nodes {
                            let path = if prefix.is_empty() {
                                n.name.clone()
                            } else {
                                format!("{}/{}", prefix, n.name)
                            };
                            out.push(format!("{}（本层 {} 条 / 含子级 {} 条）", path, n.count, n.total));
                            walk(&n.children, &path, out);
                        }
                    }
                    let mut lines = Vec::new();
                    walk(&stats.categories, "", &mut lines);
                    // 分类多的库整棵树倒出来很占上下文：只给前 N 行
                    let omitted = lines.len().saturating_sub(MAX_TAG_LINES);
                    let mut out = lines
                        .into_iter()
                        .take(MAX_TAG_LINES)
                        .collect::<Vec<_>>()
                        .join("\n");
                    if omitted > 0 {
                        out.push_str(&format!("\n（还有 {} 个分类未列出）", omitted));
                    }
                    truncate_chars(&out, TOOL_RESULT_CHAR_LIMIT)
                }
            }
            Err(e) => format!("读取分类失败：{}", e),
        },
        other => format!("未知工具 {}（只能用 search_favorites / get_favorite_content / list_favorite_tags）", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 多词查询必须能拆开：SQL 是 `LIKE '%词%'`，「Rust 爬虫」整串传进去必然 0 命中。
    #[test]
    fn split_terms_breaks_multi_word_queries() {
        assert_eq!(split_terms("Rust 爬虫"), vec!["Rust", "爬虫"]);
        assert_eq!(split_terms("爬虫、反爬"), vec!["爬虫", "反爬"]);
        assert_eq!(split_terms("  "), Vec::<String>::new());
        // 词太多只取前 4 个（再多就是噪音 + 多一次往返）
        assert_eq!(split_terms("a b c d e f").len(), 4);
    }

    /// 截断按字符切，中文不能切出半个 UTF-8（否则序列化到 JSON 会变成非法字符）。
    #[test]
    fn truncate_chars_respects_char_boundaries() {
        assert_eq!(truncate_chars("短", 10), "短");
        let long: String = "中文".repeat(100);
        let cut = truncate_chars(&long, 10);
        assert!(cut.starts_with("中文中文中文中文中文"));
        assert!(cut.ends_with("…（已截断）"));
        // 必须是合法 UTF-8（char 切分保证）
        assert_eq!(cut.chars().count(), 10 + 6);
    }

    /// 上下文超预算时折叠最旧的结果，且**保留 tool_call_id**
    /// （丢了它 OpenAI 协议会直接报「tool_call_id 对应不上」）。
    #[test]
    fn compact_context_folds_oldest_results_and_keeps_tool_call_id() {
        let mut state = FavAgentState::default();
        let mut messages: Vec<serde_json::Value> = vec![];
        // 5 条 4000 字符的结果 → 远超 12000 预算
        for i in 0..5 {
            let big = "x".repeat(4000);
            state.results.push((messages.len(), big.chars().count()));
            messages.push(json!({ "role": "tool", "tool_call_id": format!("c{i}"), "content": big }));
        }
        compact_context(&mut messages, &mut state);

        let total: usize = state.results.iter().map(|(_, l)| *l).sum();
        assert!(total <= CONTEXT_CHAR_BUDGET, "折叠后应回到预算内: {total}");
        // 折叠过的消息仍带 tool_call_id 与 role
        for (idx, _) in &state.results {
            assert_eq!(messages[*idx]["role"], "tool");
            assert!(
                messages[*idx]["tool_call_id"].as_str().is_some(),
                "tool_call_id 不能丢: {}",
                messages[*idx]
            );
        }
        // 最新的那条不该被折叠（模型还要用它整理结果）
        let last = state.results.last().unwrap().0;
        assert!(messages[last]["content"].as_str().unwrap().contains("xxxxxx"));
    }

    /// 预算内时什么都不动（幂等，避免每轮无谓地重写消息）。
    #[test]
    fn compact_context_is_noop_within_budget() {
        let mut state = FavAgentState::default();
        let mut messages: Vec<serde_json::Value> =
            vec![json!({ "role": "tool", "tool_call_id": "c0", "content": "很短" })];
        state.results.push((0, 3));
        compact_context(&mut messages, &mut state);
        assert_eq!(messages[0]["content"], "很短");
    }

    /// Agent 轮数来自设置且被钳制（与思维导图「Agent 轮数」同一口径，不再硬编码 8）。
    #[test]
    fn agent_rounds_setting_defaults_and_clamps() {
        use super::super::settings::{
            clamp_agent_rounds, DEFAULT_AGENT_ROUNDS, MAX_AGENT_ROUNDS, MIN_AGENT_ROUNDS,
        };
        assert_eq!(DEFAULT_AGENT_ROUNDS, 6);
        assert_eq!(clamp_agent_rounds(0), MIN_AGENT_ROUNDS);
        assert_eq!(clamp_agent_rounds(999), MAX_AGENT_ROUNDS);
        assert_eq!(clamp_agent_rounds(4), 4);
        // 旧设置文件没有该字段时回默认（serde default）
        let parsed: super::super::settings::FavoriteSettings =
            serde_json::from_str(r#"{"leftWidth":300}"#).unwrap();
        assert_eq!(parsed.agent_rounds, DEFAULT_AGENT_ROUNDS);
    }

    /// 工具集里 keyword 不再是必填（留空 = 浏览兜底），且仍是合法 JSON。
    #[test]
    fn tools_spec_makes_keyword_optional_and_stays_valid_json() {
        let spec: serde_json::Value = serde_json::from_str(FAV_AGENT_TOOLS).expect("工具集必须是合法 JSON");
        let arr = spec.as_array().unwrap();
        let search = arr
            .iter()
            .find(|t| t["function"]["name"] == "search_favorites")
            .expect("应有 search_favorites");
        let required = search["function"]["parameters"]["required"].as_array();
        assert!(
            required.map(|r| r.is_empty()).unwrap_or(true),
            "keyword 不该是必填（留空才能浏览兜底）"
        );
    }
}
