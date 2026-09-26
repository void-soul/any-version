//! 安装助手 Agent：用大模型帮用户把 AI 工具装上。
//!
//! 抄作业自 EchoBird 的 Mother Agent（`assets/mother/system_prompt.md` +
//! `agent_loop.rs`），但按 Kira 的能力收敛：
//! - **不给 agent 任意 shell**：只暴露 Kira 自己就有的三个动作（列工具 / 查工具 / 安装），
//!   安装走既有的 `install_ai_tool`（带备份、进度事件、包管理器回退），不另起炉灶；
//! - 工具清单来自 Kira 的注册表，agent 只能在这份清单里选，**不能编造工具**；
//! - 循环上限很小（默认 6 轮），避免「装个工具跑 150 轮」。

use serde_json::json;
use tauri::Emitter;

use super::channel::{complete_chat_messages, CompleteOutcome, NoHooks};
use super::config::load_ai_config;
use super::models::AiProvider;
use crate::commands::ai_registry::registry;

/// Agent 可用的工具（OpenAI function calling 格式）。
pub const INSTALL_AGENT_TOOLS: &str = r##"[
  { "type": "function", "function": { "name": "list_tools", "description": "列出 Kira 支持的全部 AI 工具：id、名称、分类、是否已安装、是否支持配置模型。回答「能装什么」「某某有没有」时先调它。", "parameters": { "type": "object", "properties": { "only_installed": { "type": "boolean", "description": "true = 只返回本机已安装的" } }, "required": [] } } },
  { "type": "function", "function": { "name": "get_tool_info", "description": "查看单个工具的详情：安装命令、官方网站、检测路径、是否已安装、版本。决定安装前用它确认 id 与安装方式。", "parameters": { "type": "object", "properties": { "tool_id": { "type": "string", "description": "工具 id（必须先从 list_tools 得到，不要自己编）" } }, "required": ["tool_id"] } } },
  { "type": "function", "function": { "name": "install_tool", "description": "安装指定工具。会真实执行安装命令（npm/pip/官方脚本），可能耗时几十秒。同一轮只装一个；装完如实转述命令输出里的结论，成功或失败都不要粉饰。", "parameters": { "type": "object", "properties": { "tool_id": { "type": "string", "description": "工具 id" } }, "required": ["tool_id"] } } }
]"##;

/// 事件名：前端订阅后逐条渲染「思考 / 调工具 / 工具结果 / 完成」。
pub const INSTALL_AGENT_EVENT: &str = "install-agent-progress";

/// 单轮对话最多跑几轮工具循环（装工具不是写项目，够用即可）。
const MAX_ROUNDS: usize = 6;

/// 系统提示词（抄 EchoBird 的「先确认再动手」主张，换成 Kira 的能力边界）。
fn install_agent_prompt() -> String {
    let tools = registry()
        .tool_ids()
        .iter()
        .filter_map(|id| {
            let (config, paths) = registry().get_tool(id)?;
            Some(format!(
                "- id=`{}` 名称=「{}」 分类={} 安装命令=`{}` 官网={}",
                config.id,
                config.display_name,
                config.category,
                paths.install_cmd,
                config.website
            ))
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "你是 Kira 的「安装助手」，负责帮用户把 AI 编程工具装到本机。\n\
         全程用中文，简洁、直接，不堆砌废话。\n\n\
         【你能做的事】\n\
         1. list_tools：列出 Kira 支持的全部工具（含是否已安装）\n\
         2. get_tool_info：查某个工具的安装方式、官网、检测路径\n\
         3. install_tool：真实执行安装（由 Kira 负责，带备份与进度）\n\n\
         【硬规则】\n\
         - 工具 id 只能来自 list_tools 的返回，严禁编造不存在的工具。\n\
         - 用户说的是别名（如「claude」「通义灵码」「codex」）时，先用 list_tools 找到对应 id 再行动。\n\
         - 该工具本机已安装时，**不要重复安装**：直接告诉用户已安装及版本，并说明无需操作。\n\
         - 安装前用一句话说明「即将安装 X（命令：...）」，不要长篇大论。\n\
         - 安装失败时，如实转述错误，并给出可操作建议（检查网络 / 是否需要管理员 / 改用官网安装包），不要假装成功。\n\
         - 不要建议用户执行任何你没通过工具验证过的命令。\n\n\
         【当前 Kira 注册表中的工具】\n{}\n",
        tools
    )
}

/// 前端渲染用的一条进度/结果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallAgentStep {
    /// "thinking" | "toolCall" | "toolResult" | "done" | "error"
    pub step: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallAgentReply {
    pub text: String,
    pub steps: Vec<InstallAgentStep>,
}

fn emit(app: &tauri::AppHandle, step: &InstallAgentStep) {
    let _ = app.emit(INSTALL_AGENT_EVENT, step.clone());
}

/// 选供应商与模型：指定 > 默认可用（与思维导图 AI 助手同一回退链）。
fn pick_provider_model(
    provider_id: Option<&str>,
    model_id: Option<&str>,
) -> Result<(AiProvider, String), String> {
    let config = load_ai_config();
    let provider = provider_id
        .and_then(|pid| config.providers.iter().find(|p| p.id == pid))
        .or_else(|| config.providers.iter().find(|p| !p.api_key.trim().is_empty()))
        .ok_or_else(|| "还没有配置可用的供应商（请先到「模型」页添加一个并填 API Key）".to_string())?
        .clone();
    let model = model_id
        .filter(|m| !m.trim().is_empty())
        .map(|m| m.to_string())
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| format!("供应商「{}」下没有可用模型", provider.name))?;
    Ok((provider, model))
}

/// 与前端交互的安装助手入口：跑一轮「思考 + 调工具」循环，返回结果与过程。
#[tauri::command]
pub async fn install_agent_chat(
    app: tauri::AppHandle,
    provider_id: Option<String>,
    model_id: Option<String>,
    prompt: String,
) -> Result<InstallAgentReply, String> {
    let text = prompt.trim();
    if text.is_empty() {
        return Err("请先说出你想装什么工具".to_string());
    }
    let (provider, model) = pick_provider_model(provider_id.as_deref(), model_id.as_deref())?;

    let mut messages: Vec<serde_json::Value> = vec![
        json!({ "role": "system", "content": install_agent_prompt() }),
        json!({ "role": "user", "content": text }),
    ];
    let mut steps: Vec<InstallAgentStep> = Vec::new();

    for _round in 0..MAX_ROUNDS {
        let step = InstallAgentStep {
            step: "thinking".to_string(),
            text: "思考中…".to_string(),
            tool: None,
            ok: None,
        };
        steps.push(step.clone());
        emit(&app, &step);

        let outcome: CompleteOutcome = complete_chat_messages(
            &NoHooks,
            &provider,
            &model,
            &messages,
            0.2,
            Some(INSTALL_AGENT_TOOLS),
            crate::commands::ai::usage::tool_ids::INSTALL_AGENT,
        )
        .await?;

        // 网关不支持 tools 或模型选择直接回答 → 输出即最终答复
        let tool_calls = outcome
            .message
            .as_ref()
            .and_then(|m| m.get("tool_calls"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if tool_calls.is_empty() {
            let answer = if outcome.text.trim().is_empty() {
                "模型没有给出可执行的结论，请换个说法再试一次。".to_string()
            } else {
                outcome.text.clone()
            };
            let done = InstallAgentStep {
                step: "done".to_string(),
                text: answer.clone(),
                tool: None,
                ok: Some(true),
            };
            steps.push(done.clone());
            emit(&app, &done);
            return Ok(InstallAgentReply { text: answer, steps });
        }

        // 把 assistant(tool_calls) 原样回传，再逐条执行工具
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

            let call_step = InstallAgentStep {
                step: "toolCall".to_string(),
                text: name.clone(),
                tool: Some(name.clone()),
                ok: None,
            };
            steps.push(call_step.clone());
            emit(&app, &call_step);

            let result = run_tool(&app, &name, &args).await;
            let ok = !result.starts_with("错误") && !result.starts_with("失败");
            let result_step = InstallAgentStep {
                step: "toolResult".to_string(),
                text: result.clone(),
                tool: Some(name.clone()),
                ok: Some(ok),
            };
            steps.push(result_step.clone());
            emit(&app, &result_step);

            messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": result,
            }));
        }
    }

    // 循环用完：让模型把手上的信息收个尾
    messages.push(json!({ "role": "user", "content": "请直接给出结论（不要再调用工具）。" }));
    let final_outcome = complete_chat_messages(
        &NoHooks,
        &provider,
        &model,
        &messages,
        0.2,
        None,
        crate::commands::ai::usage::tool_ids::INSTALL_AGENT,
    )
    .await?;
    let answer = if final_outcome.text.trim().is_empty() {
        "已经尽力了，但没能在限定步数内完成；请到「工具」页手动安装，或换个说法再试。".to_string()
    } else {
        final_outcome.text.clone()
    };
    let done = InstallAgentStep {
        step: "done".to_string(),
        text: answer.clone(),
        tool: None,
        ok: Some(true),
    };
    steps.push(done.clone());
    emit(&app, &done);
    Ok(InstallAgentReply { text: answer, steps })
}

/// 执行 agent 请求的一个工具（只暴露 Kira 已有的安全动作）。
async fn run_tool(app: &tauri::AppHandle, name: &str, args: &serde_json::Value) -> String {
    match name {
        "list_tools" => {
            let only_installed = args
                .get("only_installed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut lines = Vec::new();
            for id in registry().tool_ids() {
                let Some((config, paths)) = registry().get_tool(&id) else {
                    continue;
                };
                let installed = crate::commands::ai::detect::detect_ai_tools()
                    .await
                    .ok()
                    .and_then(|list| list.into_iter().find(|t| t.id == id.as_str()))
                    .map(|t| t.installed)
                    .unwrap_or(false);
                if only_installed && !installed {
                    continue;
                }
                lines.push(format!(
                    "id=`{}` 名称=「{}」 分类={} 已安装={} 安装命令=`{}`{}",
                    config.id,
                    config.display_name,
                    config.category,
                    if installed { "是" } else { "否" },
                    paths.install_cmd,
                    if config.config_file.is_some() { " 支持配置模型" } else { "" }
                ));
            }
            if lines.is_empty() {
                "没有符合条件的工具".to_string()
            } else {
                lines.join("\n")
            }
        }
        "get_tool_info" => {
            let Some(tool_id) = args.get("tool_id").and_then(|v| v.as_str()) else {
                return "缺少 tool_id".to_string();
            };
            let Some((config, paths)) = registry().get_tool(tool_id) else {
                return format!("没有 id 为 {} 的工具（请用 list_tools 确认）", tool_id);
            };
            let installed = crate::commands::ai::detect::detect_ai_tools()
                .await
                .ok()
                .and_then(|list| list.into_iter().find(|t| t.id == tool_id))
                .map(|t| format!("已安装，版本 {:?}", t.version))
                .unwrap_or_else(|| "未安装".to_string());
            format!(
                "id=`{}`\n名称=「{}」\n分类={}\n官网={}\n安装命令=`{}`\n检测命令=`{}`\n状态={}\n支持配置模型={}",
                config.id,
                config.display_name,
                config.category,
                config.website,
                paths.install_cmd,
                paths.detect_cmd,
                installed,
                config.config_file.is_some()
            )
        }
        "install_tool" => {
            let Some(tool_id) = args.get("tool_id").and_then(|v| v.as_str()) else {
                return "缺少 tool_id".to_string();
            };
            if registry().get_tool(tool_id).is_none() {
                return format!("没有 id 为 {} 的工具（请用 list_tools 确认）", tool_id);
            }
            // 复用既有的安装命令：备份、进度事件、包管理器回退都在里面
            match super::tools::install_ai_tool(app.clone(), tool_id.to_string()).await {
                Ok(result) => format!("安装结果：ok={} {}", result.ok, result.message),
                Err(e) => format!("错误：安装失败：{}", e),
            }
        }
        other => format!("错误：未知工具 {}（只能用 list_tools / get_tool_info / install_tool）", other),
    }
}
