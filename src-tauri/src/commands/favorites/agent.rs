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
  { "type": "function", "function": { "name": "search_favorites", "description": "在本地收藏库里按关键词检索（匹配标题与简介）。可以多次调用、换不同关键词，直到找够相关条目。", "parameters": { "type": "object", "properties": { "keyword": { "type": "string", "description": "关键词（会做模糊匹配；想不出关键词时用需求里的核心名词）" }, "tag": { "type": "string", "description": "只在该分类下找（可选，先用 list_tags 看有哪些分类）" }, "source": { "type": "string", "description": "只在该来源找：github / bilibili / zhihu（可选）" }, "limit": { "type": "integer", "description": "最多返回几条，默认 20" } }, "required": ["keyword"] } } },
  { "type": "function", "function": { "name": "get_favorite_content", "description": "读取某条收藏缓存下来的正文（README 等），用于确认它到底讲什么、值不值得推荐。正文可能很长，只返回前若干字符。", "parameters": { "type": "object", "properties": { "id": { "type": "integer", "description": "收藏条目的 id（来自 search_favorites 的结果）" } }, "required": ["id"] } } },
  { "type": "function", "function": { "name": "list_favorite_tags", "description": "列出收藏库里的全部分类及各分类条数，用于判断从哪个分类入手。", "parameters": { "type": "object", "properties": {} , "required": [] } } }
]"##;

/// 事件名：前端订阅后渲染「检索到 N 条 / 读了某条正文 / 完成」。
pub const FAV_AGENT_EVENT: &str = "favorites-agent-progress";

/// 单轮最多跑几轮工具循环。
const MAX_ROUNDS: usize = 8;
/// 单条正文最多给模型看多少字符（README 动辄几万字符，会把上下文撑爆）。
const CONTENT_CHAR_LIMIT: usize = 1500;

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

fn system_prompt() -> String {
    "你是 Kira 的「收藏检索助手」。用户会用自然语言说出他想要找的东西，你在他本机收藏库\
     （GitHub star / B站收藏 / 知乎收藏）里检索，并把结果整理成整齐的清单。\n\n\
     【工作方式】\n\
     1. 先用 list_favorite_tags 看有哪些分类，判断从哪入手；\n\
     2. 用 search_favorites 检索（关键词要短、抓核心名词；可以换几个词多试几次）；\n\
     3. 对拿不准的条目，用 get_favorite_content 看一眼正文再决定要不要推荐；\n\
     4. 最后给出整理好的结果。\n\n\
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

    let mut messages: Vec<serde_json::Value> = vec![
        json!({ "role": "system", "content": system_prompt() }),
        json!({ "role": "user", "content": text }),
    ];
    let mut steps: Vec<FavAgentStep> = Vec::new();

    for _round in 0..MAX_ROUNDS {
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

            let result = run_tool(&name, &args);
            let result_step = FavAgentStep {
                step: "toolResult".to_string(),
                text: result.clone(),
                tool: Some(name.clone()),
            };
            steps.push(result_step.clone());
            emit(&app, &result_step);

            messages.push(json!({ "role": "tool", "tool_call_id": id, "content": result }));
        }
    }

    messages.push(json!({ "role": "user", "content": "请直接给出整理好的结果（不要再检索）。" }));
    let final_outcome = complete_chat_messages(&NoHooks, &provider, &model, &messages, 0.3, None).await?;
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

/// 执行一个检索动作（全部只读）。
fn run_tool(name: &str, args: &serde_json::Value) -> String {
    match name {
        "search_favorites" => {
            let Some(keyword) = args.get("keyword").and_then(|v| v.as_str()) else {
                return "缺少 keyword".to_string();
            };
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20)
                .min(50) as usize;
            // 分类改成树之后按 id 筛：名字先解析成 id，
            // 但**只查不建** —— 模型瞎猜的名字不该在检索时变成一个新分类。
            let category_id = args
                .get("tag")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .and_then(|name| {
                    db::with_conn(|conn| db::find_category_by_name(conn, name))
                        .ok()
                        .flatten()
                });
            let filter = db::ListFilter {
                keyword: Some(keyword.to_string()),
                category_id,
                source: args.get("source").and_then(|v| v.as_str()).map(|s| s.to_string()),
                limit,
                ..Default::default()
            };
            match db::with_conn(|conn| db::list(conn, &filter)) {
                Ok(rows) => {
                    if rows.is_empty() {
                        return format!("关键词「{}」没有命中任何收藏", keyword);
                    }
                    rows.iter()
                        .map(|r| {
                            let desc = r
                                .description
                                .as_deref()
                                .unwrap_or("")
                                .chars()
                                .take(120)
                                .collect::<String>();
                            format!(
                                "id={} | 标题=《{}》 | 来源={} | 分类={} | 收藏于={} | 链接={} | 简介={}",
                                r.id,
                                r.title,
                                r.source,
                                if r.tags.is_empty() { "未分类".to_string() } else { r.tags.join("/") },
                                r.favorited_at.as_deref().unwrap_or(&r.created_at),
                                r.url,
                                desc
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                Err(e) => format!("检索失败：{}", e),
            }
        }
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
                    lines.join("\n")
                }
            }
            Err(e) => format!("读取分类失败：{}", e),
        },
        other => format!("未知工具 {}（只能用 search_favorites / get_favorite_content / list_favorite_tags）", other),
    }
}
