use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::commands::ai_registry::{registry, ToolConfig};
use crate::commands::hidden_cmd;
use crate::proxy::types::ModelRoute;
use super::models::*;

use super::config::{load_ai_config, load_last_launch_configs, save_last_launch_configs, load_sessions, save_sessions_to_file};
use super::terminal::{get_terminal_exe_cfg, is_ext_terminal};

/// 选择出站协议：若供应商支持工具「原生协议」，则同协议直连（不转换）；
/// 否则取供应商首个支持的协议（由代理做协议转换）。
/// 供应商未配置任何协议端点 URL 时返回 None。
fn pick_outbound_protocol(native: &str, provider: &AiProvider) -> Option<String> {
    if provider.supported_protocols().contains(&native.to_string()) {
        return Some(native.to_string());
    }
    if !provider.openai_url.is_empty() { return Some("openai".to_string()); }
    if !provider.anthropic_url.is_empty() { return Some("anthropic".to_string()); }
    if !provider.google_url.is_empty() { return Some("google".to_string()); }
    None
}

/// 为指定工具启动一个本地代理（按需、空闲端口、后台 spawn）。
/// CLI 工具与 GUI/桌面应用共用：返回监听端口；若未配置 Provider/Key、
/// 无可用出站协议或绑定失败，则返回 0（调用方应回退普通逻辑）。
pub(crate) async fn start_tool_proxy(
    tool_config: &ToolConfig,
    provider: Option<&AiProvider>,
    config: &AiConfig,
    req: &LaunchAiToolRequest,
) -> (u16, Option<tokio::task::AbortHandle>) {
    let inbound_protocols = tool_config.inbound_protocols();
    let primary_inbound = tool_config.native_protocol();

    eprintln!("\n[proxy] inbound_protocols={:?}, primary={}", inbound_protocols, primary_inbound);

    // 根据供应商已配置的协议 URL 选择出站协议：优先工具原生协议（同协议直连），
    // 否则取供应商首个支持的协议（由代理做协议转换）。
    let chosen_outbound = provider
        .as_ref()
        .and_then(|p| pick_outbound_protocol(&primary_inbound, p))
        .unwrap_or_default();

    let mut proxy_port: u16 = 0;
    let mut abort_handle: Option<tokio::task::AbortHandle> = None;
    if let Some(p) = provider {
        if !p.api_key.is_empty() && !chosen_outbound.is_empty() {
            let outbound_protocol = chosen_outbound.clone();
            let upstream_base_url = p.url_for(&outbound_protocol);
            let proxy_settings = &registry().terminals().proxy_settings;
            let timeout = proxy_settings.timeout_seconds as u64;

            let conversion_mode = crate::proxy::types::derive_conversion_mode(&primary_inbound, &outbound_protocol);

            // 模型伪装：声明名 C → 实际模型 B；masquerade_model 为空表示不伪装。
            let target_model = req.model_id.clone().unwrap_or_default();
            let mut model_aliases: HashMap<String, String> = HashMap::new();
            if let Some(ref c) = req.masquerade_model {
                let c_norm = c.replace("[1m]", "").replace("[1M]", "").trim().to_string();
                if !c_norm.is_empty() && c_norm != target_model {
                    model_aliases.insert(c_norm, target_model.clone());
                }
            }

            // fallback/小模型伪装映射：声明名 C_small → 实际模型 B_small。
            if let Some(ref fb) = req.fallback_model_id {
                if !fb.is_empty() {
                    let claimed_requested = match &req.fallback_masquerade_model {
                        Some(c) if !c.is_empty() => c.clone(),
                        _ => format_model_name(fb, &tool_config),
                    };
                    let claimed_norm = claimed_requested.replace("[1m]", "").replace("[1M]", "").trim().to_string();
                    if !claimed_norm.is_empty() {
                        model_aliases.insert(claimed_norm, fb.clone());
                    }
                }
            }

            // 跨供应商路由：按实际模型名 → 其所属供应商的端点+key。
            let mut model_routes: HashMap<String, ModelRoute> = HashMap::new();
            if let Some(ref mid) = req.model_id {
                if !mid.is_empty() {
                    model_routes.insert(mid.clone(), ModelRoute {
                        base_url: p.url_for(&chosen_outbound),
                        api_key: p.api_key.clone(),
                    });
                }
            }
            if let Some(ref fb) = req.fallback_model_id {
                if !fb.is_empty() {
                    if let Some(fp) = req.fallback_provider_id.as_ref()
                        .and_then(|pid| config.providers.iter().find(|pr| &pr.id == pid)) {
                        model_routes.insert(fb.clone(), ModelRoute {
                            base_url: fp.url_for(&chosen_outbound),
                            api_key: fp.api_key.clone(),
                        });
                    }
                }
            }

            // 优化器 / 整流器：工具支持时可由启动请求开关覆盖，否则继承全局配置
            let optimizer_on = tool_config.supports_optimizer
                && req.optimizer_enabled.unwrap_or(true)
                && config.optimizer.enabled;
            let rectifier_on = tool_config.supports_rectifier
                && req.rectifier_enabled.unwrap_or(true)
                && config.rectifier.enabled;

            // 绑定空闲端口（OS 分配，避免冲突）
            match crate::proxy::server::bind_free_port(&proxy_settings.listen_address) {
                Ok((port, listener)) => {
                    proxy_port = port;
                    let listen_addr = proxy_settings.listen_address.clone();
                    let proxy_config = crate::proxy::types::ProxyConfig {
                        listen_address: listen_addr,
                        listen_port: port,
                        inbound_protocols: inbound_protocols.clone(),
                        outbound_protocol: outbound_protocol.clone(),
                        conversion_mode,
                        upstream_api_key: p.api_key.clone(),
                        upstream_base_url: upstream_base_url.clone(),
                        model_routes,
                        target_model,
                        timeout_secs: timeout,
                        model_aliases,
                        default_model: req.model_id.clone(),
                        tool_id: req.tool_id.clone(),
                        provider_id: p.id.clone(),
                        rectifier_enabled: rectifier_on,
                        rectifier_thinking_signature: req.rectifier_thinking_signature.unwrap_or(config.rectifier.thinking_signature),
                        rectifier_thinking_budget: req.rectifier_thinking_budget.unwrap_or(config.rectifier.thinking_budget),
                        rectifier_media_fallback: req.rectifier_media_fallback.unwrap_or(config.rectifier.media_fallback),
                        rectifier_protocol_mismatch: req.rectifier_protocol_mismatch.unwrap_or(config.rectifier.protocol_mismatch),
                        optimizer_enabled: optimizer_on,
                        optimizer_cache_injection: req.optimizer_cache_injection.unwrap_or(config.optimizer.cache_injection),
                        optimizer_thinking: req.optimizer_thinking.unwrap_or(config.optimizer.thinking_optimizer),
                        optimizer_deepseek: req.optimizer_deepseek.unwrap_or(config.optimizer.deepseek_normalize),
                    };
                    eprintln!("[proxy] ✓ 启动代理 -> 127.0.0.1:{}  ({} -> {})", port, primary_inbound, outbound_protocol);
                    let handle = tokio::spawn(async move {
                        if let Err(e) = crate::proxy::server::serve_proxy(proxy_config, listener).await {
                            eprintln!("[proxy] 代理错误: {}", e);
                        }
                    });
                    abort_handle = Some(handle.abort_handle());
                    wait_for_proxy_ready(&proxy_settings.listen_address, port).await;
                }
                Err(e) => {
                    eprintln!("[proxy] ✗ 绑定空闲端口失败: {}", e);
                }
            }
        }
    }
    (proxy_port, abort_handle)
}


// ─── 启动 AI 工具 ───

#[tauri::command]
pub async fn launch_ai_tool(req: LaunchAiToolRequest) -> Result<serde_json::Value, String> {
    eprintln!("══════════════════════════════════════════════════════════════");
    eprintln!("                    启动 AI 工具");
    eprintln!("══════════════════════════════════════════════════════════════");

    let config = load_ai_config();
    let tool_config = registry().get_tool_config(&req.tool_id).ok_or("未知工具")?.clone();
    let tool_paths = registry().get_path_config(&req.tool_id).ok_or("未知工具")?.clone();
    let provider = req.provider_id.as_ref().and_then(|pid| config.providers.iter().find(|p| &p.id == pid));

    eprintln!("\n[request] ▼ LaunchAiToolRequest 入参");
    eprintln!("  tool_id:          {:?}", req.tool_id);
    eprintln!("  project_path:     {:?}", req.project_path);
    eprintln!("  provider_id:      {:?}", req.provider_id);
    eprintln!("  model_id:         {:?}", req.model_id);
    eprintln!("  fallback_model_id:{:?}", req.fallback_model_id);
    eprintln!("  session_mode:     {:?}", req.session_mode);
    eprintln!("  session_id:       {:?}", req.session_id);
    eprintln!("  terminal_id:      {:?}", req.terminal_id);
    eprintln!("  one_m_context:    {:?}", req.one_m_context);

    eprintln!("\n[provider] provider_id={:?}", req.provider_id);
    match provider {
        Some(p) => eprintln!("  ✓ 找到: name={}", p.name),
        None => eprintln!("  ✗ 未找到，将使用官方默认模型"),
    }

    // ─── Step 1: 启动代理（强制开启，每工具独立实例 + 自由端口）───
    // 抽成 start_tool_proxy 复用：CLI 工具与 GUI/桌面应用共用同一套按需代理。
    let (proxy_port, _proxy_abort) = start_tool_proxy(&tool_config, provider, &config, &req).await;

    // 出站协议（供 Step 2 写配置文件使用；与 start_tool_proxy 内部推导一致）
    let chosen_outbound = provider
        .as_ref()
        .and_then(|p| pick_outbound_protocol(&tool_config.native_protocol(), p))
        .unwrap_or_default();

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 2: 写入工具配置文件（含 env.* 前缀的环境变量注入）");
    eprintln!("──────────────────────────────────────────────────────────────");

    // 写入工具的配置文件（由 config.json 的 configFile 字段驱动）
    // 代理必开：baseUrl 始终指向本地代理端口，由代理负责转发到真实上游。
    if tool_config.config_file.is_some() {
        if let Some(ref p) = provider {
            if !p.api_key.is_empty() {
                // 上游 URL（fallback 用）：取供应商当前出站协议对应的端点 URL。
                let upstream_url = p.url_for(&chosen_outbound);

                // baseUrl 始终指向本次启动的本地代理端口（所有协议统一指向代理）。
                // 未启动代理（无 Provider/Key）时回退到供应商 base_url。
                let effective_base_url: String = if proxy_port != 0 {
                    format!("http://127.0.0.1:{}", proxy_port)
                } else {
                    upstream_url.clone()
                };

                // 声明模型名 C（工具以为自己调用的模型）：
                // 若配置了伪装则是 masquerade_model，否则直接是所选取的供应商模型 B。
                let claimed_model = req.masquerade_model.clone()
                    .filter(|c| !c.is_empty())
                    .or_else(|| req.model_id.clone());

                // 代理模式：本次启动了本地代理（统计 + 转换 + 伪装映射）时为 true。
                let proxy_mode = proxy_port != 0;

                if !upstream_url.is_empty() || proxy_mode {
                    eprintln!("[config_file] 写入参数:");
                    eprintln!("[config_file]   tool_id: {}", req.tool_id);
                    eprintln!("[config_file]   provider: id={}, name={}", p.id, p.name);
                    eprintln!("[config_file]   protocol: {}", tool_config.api_protocol);
                    eprintln!("[config_file]   upstream_url: {}", upstream_url);
                    eprintln!("[config_file]   effective_base_url: {}", effective_base_url);
                    eprintln!("[config_file]   model_id(B): {:?}", req.model_id);
                    eprintln!("[config_file]   claimed_model(C): {:?}", claimed_model);
                    eprintln!("[config_file]   proxy_mode: {}", proxy_mode);
                    match write_tool_config_from_spec(
                        &req.tool_id,
                        &tool_config,
                        req.model_id.as_deref(),
                        claimed_model.as_deref(),
                        Some(&effective_base_url),
                        &effective_base_url,
                        &p.api_key,
                        req.fallback_model_id.as_deref(),
                        req.fallback_masquerade_model.as_deref(),
                        req.one_m_context,
                        req.fallback_one_m_context,
                        proxy_mode,
                    ) {
                        Ok(_) => {
                            eprintln!("[config_file] ✓ 配置文件写入完成");
                            if let Some(ref cf) = tool_config.config_file {
                                eprintln!("[config_file]   路径: {:?}", cf.path);
                                eprintln!("[config_file]   格式: {:?}", cf.format);
                            }
                        }
                        Err(e) => {
                            eprintln!("[config_file] ✗ 写入失败: {}", e);
                        }
                    }
                } else {
                    eprintln!("[config_file] (未配置上游 URL，跳过)");
                }
            } else {
                eprintln!("[config_file] (未配置 API Key，跳过)");
            }
        } else {
            eprintln!("[config_file] (未选择 Provider，跳过)");
        }
    } else {
        eprintln!("[config_file] (无 configFile 定义，跳过配置写入)");
    }

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 3: 构建 CLI 参数");
    eprintln!("──────────────────────────────────────────────────────────────");

    // 获取终端 exe（从 JSON 配置）
    let terminal_exe = get_terminal_exe_cfg(&req.terminal_id);

    // 从 detect_cmd 提取真实可执行文件名（用于 prefix stripping）
    let tool_exe = tool_paths.detect_cmd
        .split_whitespace()
        .next()
        .unwrap_or(&tool_config.id)
        .to_string();

    // 启动命令（来自 startCommand，可能包含默认参数如 "mimo ."）
    let start_cmd = tool_paths.start_command.clone();

    // resume / continue 参数
    let exe_prefix = format!("{} ", &tool_exe);
    let extra_args = if req.session_mode == "resume" {
        req.session_id.as_ref().and_then(|sid| {
            tool_config.resume_cmd.as_ref().map(|s| {
                s.replace("{session_id}", sid)
                    .strip_prefix(&exe_prefix)
                    .unwrap_or(&s.replace("{session_id}", sid))
                    .to_string()
            })
        }).unwrap_or_default()
    } else if req.session_mode == "continue" {
        tool_config.continue_cmd.as_ref().map(|s| {
            s.strip_prefix(&exe_prefix).unwrap_or(s).to_string()
        }).unwrap_or_default()
    } else {
        String::new()
    };

    // 所有模型 / baseUrl / apiKey 均已写入工具配置文件（configFile），不再通过 CLI 传递任何模型参数。
    // 仅保留 resume/continue 等会话参数（extra_args）与启动命令（start_command）。
    let tool_args = extra_args.clone();

    eprintln!("\n[cli] session_mode={}, extra_args={:?}", req.session_mode, extra_args);
    eprintln!("[cli] start_command={:?}", start_cmd);
    eprintln!("[cli] tool_args={:?}", tool_args);
    eprintln!("[cli] terminal_id={:?}, terminal_exe={:?}", req.terminal_id, terminal_exe);
    eprintln!("[cli] 注：模型/凭证均来自配置文件，未注入任何 CLI 模型参数");

    let mut cmd = hidden_cmd::hidden_cmd(&terminal_exe);
    cmd.current_dir(&req.project_path);

    let tool_arg_parts: Vec<&str> = extra_args
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .collect();

    // start_command 拆分为多个参数（如 "mimo ." → ["mimo", "."]）
    let start_cmd_parts: Vec<&str> = start_cmd.split_whitespace().collect();

    if terminal_exe.to_lowercase().contains("cmd") {
        cmd.arg("/c").arg("start").arg("/d").arg(&req.project_path)
           .arg("cmd").arg("/k");
        for p in &start_cmd_parts { cmd.arg(p); }
        for a in &tool_arg_parts { cmd.arg(a); }
    } else if terminal_exe.to_lowercase().contains("wt") {
        cmd.arg("-d").arg(&req.project_path).arg("cmd").arg("/k");
        for p in &start_cmd_parts { cmd.arg(p); }
        for a in &tool_arg_parts { cmd.arg(a); }
    } else if is_ext_terminal(&req.terminal_id) {
        let launch_args = registry().terminals().terminals.get(&req.terminal_id)
            .and_then(|t| t.launch_args.as_ref())
            .map(|a| a.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|| vec!["-e", "cmd", "/k"]);
        for s in &launch_args { cmd.arg(*s); }
        for p in &start_cmd_parts { cmd.arg(p); }
        for a in &tool_arg_parts { cmd.arg(a); }
    } else {
        let escaped_path = req.project_path.replace('\'', "''");
        let run_cmd = if tool_args.is_empty() {
            format!("Set-Location -LiteralPath '{}'; {}", escaped_path, &start_cmd)
        } else {
            format!("Set-Location -LiteralPath '{}'; {} {}", escaped_path, &start_cmd, &tool_args)
        };
        cmd.args(["-NoExit", "-Command", &run_cmd]);
    }

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 4: spawn 子进程（注入 env.* 环境变量）");
    eprintln!("──────────────────────────────────────────────────────────────");
    eprintln!("[spawn] 工作目录: {:?}", req.project_path);
    eprintln!("[spawn] 配置来源: 工具配置文件（configFile） + env.* 环境变量注入");

    // 从 config_file 的 write 映射中提取 env.* 前缀的键，作为环境变量注入到子进程
    if let Some(ref p) = provider {
        // env 注入的 baseUrl 始终指向本次启动的本地代理端口（未启动则回退供应商 base_url）
        let upstream_fallback = p.url_for(&chosen_outbound);
        let effective_base_url = if proxy_port != 0 {
            format!("http://127.0.0.1:{}", proxy_port)
        } else {
            upstream_fallback.clone()
        };
        // env 注入的 model：声明名 C（伪装优先，否则所选取模型 B）
        let model = req.masquerade_model.clone()
            .filter(|c| !c.is_empty())
            .or_else(|| req.model_id.clone())
            .unwrap_or_default();
        let envs = build_env_vars(&tool_config, &p.api_key, &effective_base_url, &model);
        for (k, v) in &envs {
            eprintln!("[spawn] env {} = {}", k, mask_secret(v));
            cmd.env(k, v);
        }
    }

    cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

    eprintln!("[spawn] ✓ 进程已启动");

    // 保存会话信息
    let mut sessions = load_sessions();
    let session_id = req.session_id.unwrap_or_else(|| {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    });
    // 在 move 前克隆后续所需字段
    let (lc_tool_id, lc_project_path, lc_model_id, lc_provider_id, lc_fallback_model_id) = (
        req.tool_id.clone(),
        req.project_path.clone(),
        req.model_id.clone(),
        req.provider_id.clone(),
        req.fallback_model_id.clone(),
    );
    sessions.sessions.retain(|s| !(s.tool_id == req.tool_id && s.project_path == req.project_path && s.session_id.as_deref() == Some(&session_id)));
    sessions.sessions.push(AiSession {
        tool_id: req.tool_id,
        project_path: req.project_path,
        session_id: Some(session_id.clone()),
        last_used: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        model_id: req.model_id,
    });
    save_sessions_to_file(&sessions)?;

    // 保存本次启动配置（供下次切换工具时恢复 UI 状态）
    let is_official = lc_provider_id.is_none() && lc_model_id.is_none();
    let last_config = LastLaunchConfig {
        provider_id: lc_provider_id.clone(),
        provider_name: lc_provider_id.as_ref().and_then(|pid| {
            let cfg = load_ai_config();
            cfg.providers.iter().find(|p| &p.id == pid).map(|p| p.name.clone())
        }),
        model_id: lc_model_id,
        fallback_model_id: lc_fallback_model_id,
        fallback_provider_id: None,
        fallback_masquerade_model: req.fallback_masquerade_model.clone(),
        use_official_model: is_official,
        terminal_id: req.terminal_id.clone(),
        one_m_context: req.one_m_context,
        fallback_one_m_context: req.fallback_one_m_context,
        project_path: lc_project_path,
        masquerade_model: req.masquerade_model.clone(),
        optimizer_enabled: req.optimizer_enabled,
        rectifier_enabled: req.rectifier_enabled,
        last_launched_at: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    let mut configs = load_last_launch_configs();
    configs.configs.insert(lc_tool_id, last_config);
    let _ = save_last_launch_configs(&configs);

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 5: 保存会话");
    eprintln!("──────────────────────────────────────────────────────────────");
    eprintln!("[session] session_id={}", session_id);
    eprintln!("[session] ✓ 写入 ai_sessions.json");

    eprintln!("\n══════════════════════════════════════════════════════════════");
    eprintln!("                    启动成功 ✅");
    eprintln!("══════════════════════════════════════════════════════════════");

    Ok(serde_json::json!({
        "success": true,
        "message": "启动成功".to_string(),
    }))
}

/// 根据工具 config.json 中的 configFile 字段，自动写入工具配置文件。
/// 不注入任何环境变量，全部参数（模型 / baseUrl / apiKey / 别名）都写进配置文件，
/// 便于对照各工具官方文档逐项核对。
///
/// - `model_id`：实际模型 B（供应商模型）。
/// - `claimed_model`：声明模型名 C（工具以为自己调用的模型；伪装时 = masquerade_model，
///   否则 = B）。配置文件中的 `model` 字段写入 C，由本地代理按 masquerade 映射 C → B。
pub(crate) fn write_tool_config_from_spec(
    tool_id: &str,
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    claimed_model: Option<&str>,
    base_url: Option<&str>,
    fallback_url: &str,
    api_key: &str,
    fallback_model_id: Option<&str>,
    fallback_masquerade_model: Option<&str>,
    one_m_context: bool,
    fallback_one_m_context: bool,
    proxy_mode: bool,
) -> Result<(), String> {
    // write_tool_config_generic 内部会检查 config_file 是否存在，无 configFile 时直接返回 Ok(())
    let _ = tool_id;
    write_tool_config_generic(tool_config, model_id, claimed_model, base_url.unwrap_or(fallback_url), api_key, fallback_model_id, fallback_masquerade_model, one_m_context, fallback_one_m_context, proxy_mode)
}

/// 从 config_file.write 映射中提取 env.* 前缀的键，构建环境变量 HashMap。
/// 值模板匹配：apiKey → api_key, baseUrl → base_url, model/modelName → model, 其他 → 字面值。
/// 空值不注入。供 launch_ai_tool 和 collab dispatch_to_tool 共用。
pub(crate) fn build_env_vars(
    tool_config: &ToolConfig,
    api_key: &str,
    base_url: &str,
    model: &str,
) -> HashMap<String, String> {
    let mut envs = HashMap::new();
    if let Some(ref cf) = tool_config.config_file {
        if let Some(ref write_map) = cf.write {
            for (path, value_template) in write_map {
                if path.starts_with("env.") {
                    let env_key = &path[4..];
                    let env_value = match value_template.as_str() {
                        "apiKey" => api_key.to_string(),
                        "baseUrl" => base_url.to_string(),
                        "model" | "modelName" => model.to_string(),
                        other => other.to_string(),
                    };
                    if !env_value.is_empty() {
                        envs.insert(env_key.to_string(), env_value);
                    }
                }
            }
        }
    }
    envs
}

/// 根据 modelFormat 配置格式化模型名
pub(crate) fn format_model_name(raw: &str, tool_config: &ToolConfig) -> String {
    if raw.is_empty() { return String::new(); }
    if let Some(ref fmt) = tool_config.model_format {
        let prefix = fmt.prefix.as_deref().unwrap_or("");
        if fmt.extract_last {
            let name = raw.split('/').next_back().unwrap_or(raw);
            format!("{}{}", prefix, name)
        } else {
            format!("{}{}", prefix, raw)
        }
    } else {
        raw.to_string()
    }
}

/// 格式化模型名，并按需追加 [1m]（1M 上下文后缀，仅 Anthropic 协议工具需要）
fn format_model_name_with_ctx(raw: &str, tool_config: &ToolConfig, one_m: bool) -> String {
    let mut s = format_model_name(raw, tool_config);
    if one_m && !s.contains("[1m]") {
        s = format!("{}[1m]", s);
    }
    s
}

/// 通用工具配置文件写入：根据 config.json 的 configFile.write 映射写入。
/// 支持 json / jsonc（serde_json）与 toml（行式）两种格式。
///
/// `model` 字段写入**声明模型名 C**（claimed_model，回退到实际模型 B），交由本地代理
/// 按 masquerade 映射 C → B 转发到上游。代理模式下不再跳过模型字段——工具必须以 C
/// 发起请求，代理才能正确改写。模型伪装（C → B 的具体映射）由启动时代理动态持有，
/// 这里不再写 ANTHROPIC_DEFAULT_* 之类的别名环境变量。
fn write_tool_config_generic(
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    claimed_model: Option<&str>,
    base_url: &str,
    api_key: &str,
    fallback_model_id: Option<&str>,
    fallback_masquerade_model: Option<&str>,
    one_m_context: bool,
    fallback_one_m_context: bool,
    _proxy_mode: bool,
) -> Result<(), String> {
    let cfg = match &tool_config.config_file {
        Some(c) => c,
        None => return Ok(()),
    };
    let write_map = match &cfg.write {
        Some(w) => w,
        None => return Ok(()),
    };

    // 解析路径（~ → HOME）
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let resolved_path = if cfg.path.starts_with("~/") {
        home.join(&cfg.path[2..])
    } else {
        PathBuf::from(&cfg.path)
    };

    // 确保父目录存在
    if let Some(parent) = resolved_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // 仅 Anthropic 协议工具才追加 [1m] 后缀（对齐原 env 注入行为）
    let apply_one_m = one_m_context
        && tool_config.support_one_m_context
        && (tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both");
    // fallback/小模型可独立勾选 1M
    let apply_one_m_fb = fallback_one_m_context
        && tool_config.support_one_m_context
        && (tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both");

    // 组装待写入的 (路径, 值) 列表
    let mut writes: Vec<(String, String)> = Vec::new();
    // 声明模型名 C 优先；否则回退到实际模型 B
    let effective_model_id = claimed_model.or(model_id);
    let has_model = effective_model_id.is_some();
    let model = effective_model_id
        .map(|m| format_model_name_with_ctx(m, tool_config, apply_one_m))
        .unwrap_or_default();
    let model_name = model.split('/').next_back().unwrap_or(&model).to_string();
    // fallback/小模型：声明名（伪装优先，否则实际模型 B）。无 fallback 时为 None。
    let fallback_claimed = fallback_model_id.and_then(|fm| {
        if fm.is_empty() { return None; }
        match fallback_masquerade_model {
            Some(c) if !c.is_empty() => Some(format_model_name_with_ctx(c, tool_config, apply_one_m_fb)),
            _ => Some(format_model_name_with_ctx(fm, tool_config, apply_one_m_fb)),
        }
    });

    for (path, value_template) in write_map {
        // 动态键名替换：{model_name} → 实际模型名
        let resolved_path = path.replace("{model_name}", &model_name);
        let value = match value_template.as_str() {
            "model" | "modelName" if !has_model => {
                eprintln!("[config_file] skip {} (no model)", resolved_path);
                continue;
            },
            "model" => model.clone(),
            "modelName" => model_name.clone(),
            "fallbackModel" => match &fallback_claimed {
                Some(v) => v.clone(),
                None => {
                    eprintln!("[config_file] skip {} (no fallback model)", resolved_path);
                    continue;
                }
            },
            "baseUrl" => base_url.to_string(),
            "apiKey" => {
                // API Key 为空时不写入配置文件，避免写入空字符串被解析器判定为非法凭证
                if api_key.is_empty() {
                    eprintln!("[config_file] skip {} (empty apiKey, 不写入)", resolved_path);
                    continue;
                }
                api_key.to_string()
            },
            "" => String::new(),
            other => other.to_string(),
        };
        eprintln!("[config_file] set {} = {}", resolved_path, mask_secret(&value));
        writes.push((resolved_path, value));
    }

    let existing = if resolved_path.exists() {
        fs::read_to_string(&resolved_path).unwrap_or_default()
    } else {
        String::new()
    };

    eprintln!("[config_file] 目标路径: {} (format={})", resolved_path.display(), cfg.format);
    match cfg.format.as_str() {
        "toml" => write_toml_config(&resolved_path, &existing, &writes)?,
        _ => write_json_config(&resolved_path, &existing, &writes, cfg.schema.as_deref(), tool_config)?,
    }
    eprintln!("[config_file] ✓ 已写入配置到 {}", resolved_path.display());
    Ok(())
}

/// 掩码打印含密钥的值
pub(crate) fn mask_secret(v: &str) -> String {
    if v.is_empty() {
        String::new()
    } else if v.len() <= 12 {
        "***".to_string()
    } else {
        format!("{}...{}", &v[..8], &v[v.len() - 4..])
    }
}

/// 写入 JSON / JSONC 配置文件（serde_json；jsonc 读取失败时按空文档处理，保留写入内容）
fn write_json_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, String)],
    schema: Option<&str>,
    tool_config: &ToolConfig,
) -> Result<(), String> {
    let mut doc: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(existing).unwrap_or(serde_json::json!({}))
    };
    if let Some(s) = schema {
        doc.as_object_mut()
            .unwrap()
            .entry("$schema")
            .or_insert(serde_json::json!(s));
    }
    for (p, v) in writes {
        set_json_path(&mut doc, p, v);
    }

    // 清除本工具管理的、但本次未写入的残留键。
    // 合并写入不会删除旧键，导致切换供应商/模型后旧模型字段（如其它供应商的
    // ANTHROPIC_DEFAULT_*_MODEL、已弃用的 ANTHROPIC_SMALL_FAST_MODEL、历史遗留的
    // 顶层 model）残留在配置里，干扰本次启动。这里移除受管前缀中不在本次写入集合内的键。
    // 注意：清理逻辑仅在 Anthropic 系列协议下执行，避免误清除非 Anthropic 协议工具的 env 设置。
    let is_anthropic = tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both";
    if is_anthropic {
        cleanup_managed_model_keys(&mut doc, writes);
    }

    let content = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化配置失败: {}", e))?;
    fs::write(path, content)
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 移除本工具管理的、但本次未写入的残留键。
///
/// - `env` 下以 `ANTHROPIC_MODEL` / `ANTHROPIC_DEFAULT_` / `ANTHROPIC_SMALL_FAST_MODEL`
///   开头的模型键，若不在本次写入集合内，则删除（避免旧供应商/旧模型字段残留）。
/// - `env.ANTHROPIC_AUTH_TOKEN`：当配置只写 `ANTHROPIC_API_KEY`（API Key 代理模式）时，
///   若旧版本曾写入 `ANTHROPIC_AUTH_TOKEN`，需清除以免与 `ANTHROPIC_API_KEY` 冲突导致
///   Claude Code 报 "Both ANTHROPIC_AUTH_TOKEN and ANTHROPIC_API_KEY set"。
/// - 顶层 `model` 字段本应用从不写入（历史版本残留），一律清除以免干扰 Claude Code 的模型解析。
fn cleanup_managed_model_keys(doc: &mut serde_json::Value, writes: &[(String, String)]) {
    let managed_prefixes = ["ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_", "ANTHROPIC_SMALL_FAST_MODEL", "ANTHROPIC_AUTH_TOKEN"];
    let current_env_keys: std::collections::HashSet<String> = writes
        .iter()
        .filter_map(|(p, _)| p.strip_prefix("env.").map(|k| k.to_string()))
        .collect();

    if let Some(env_obj) = doc.get_mut("env").and_then(|v| v.as_object_mut()) {
        let stale: Vec<String> = env_obj
            .keys()
            .filter(|k| {
                managed_prefixes.iter().any(|p| k.starts_with(p))
                    && !current_env_keys.contains(*k)
            })
            .cloned()
            .collect();
        for k in stale {
            env_obj.remove(&k);
            eprintln!("[config_file] cleanup stale env.{}", k);
        }
    }

    // 顶层 model 字段：仅当本次未写入时才清除。
    // 说明：claude-code 从不写顶层 model（只用 env.ANTHROPIC_MODEL），旧版本残留的
    // 顶层 model（如 "fable"）会干扰模型解析，需清除；但 opencode/mimocode/deveco 等
    // 工具本就写顶层 model，必须保留——故以"是否在本次写入集合内"为判据。
    let writes_top_model = writes.iter().any(|(p, _)| p == "model");
    if !writes_top_model {
        if let Some(obj) = doc.as_object_mut() {
            if obj.remove("model").is_some() {
                eprintln!("[config_file] cleanup stale top-level model");
            }
        }
    }
}

/// 写入 TOML 配置文件（支持顶层 key 和 dotted keys 如 `model_providers.x.base_url`）
fn write_toml_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, String)],
) -> Result<(), String> {
    let mut lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();
    let mut updated: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 过滤掉 env.* 前缀的键（这些键通过环境变量注入，不写入 TOML 文件）
    let toml_writes: Vec<&(String, String)> = writes
        .iter()
        .filter(|(p, _)| !p.starts_with("env."))
        .collect();

    // 更新已存在的行（仅顶层 key）
    for line in lines.iter_mut() {
        if let Some((k, _)) = parse_toml_kv(line) {
            if let Some((_, v)) = toml_writes.iter().find(|(p, _)| p == &k && !p.contains('.')) {
                *line = format!("{} = \"{}\"", k, v);
                updated.insert(k);
            }
        }
    }
    // 追加新的顶层 key
    for (p, v) in &toml_writes {
        if !p.contains('.') && !updated.contains(p) {
            lines.push(format!("{} = \"{}\"", p, v));
            updated.insert(p.clone());
        }
    }
    // 追加 dotted keys（如 model_providers.anyversion.base_url = "..."）
    // 简单策略：直接在文件末尾追加，重复的键由工具自身的 TOML 解析器去重处理
    for (p, v) in &toml_writes {
        if p.contains('.') && !updated.contains(p) {
            lines.push(format!("{} = \"{}\"", p, v));
            updated.insert(p.clone());
        }
    }
    let content = lines.join("\n");
    fs::write(path, content)
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 解析 TOML 顶层 `key = "value"` 行
fn parse_toml_kv(line: &str) -> Option<(String, String)> {
    let re = regex::Regex::new(r#"^\s*([A-Za-z_][\w-]*)\s*=\s*"(.*)"\s*$"#).ok()?;
    let caps = re.captures(line)?;
    Some((caps.get(1)?.as_str().to_string(), caps.get(2)?.as_str().to_string()))
}

/// 根据点分路径设置 JSON 文档中的值（自动创建中间对象）
fn set_json_path(doc: &mut serde_json::Value, path: &str, value: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = doc;
    for (i, p) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if !cur.is_object() {
                *cur = serde_json::json!({});
            }
            cur.as_object_mut().unwrap().insert(p.to_string(), serde_json::json!(value));
            return;
        }
        if !cur.is_object() {
            *cur = serde_json::json!({});
        }
        if cur.get(*p).is_none() || !cur[*p].is_object() {
            cur.as_object_mut().unwrap().insert(p.to_string(), serde_json::json!({}));
        }
        cur = cur.as_object_mut().unwrap().get_mut(*p).unwrap();
    }
}

/// 轮询代理服务器的 /health 端点，等待代理就绪。
/// 最多重试 15 次（每次 200ms），总计最多 3 秒。
async fn wait_for_proxy_ready(listen_address: &str, port: u16) {
    let health_url = format!("http://{}:{}/health", listen_address, port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap_or_default();
    for i in 0..15u32 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if client.get(&health_url).send().await.is_ok() {
            eprintln!("[proxy] ✓ 代理就绪 (尝试 {} 次)", i + 1);
            return;
        }
    }
    eprintln!("[proxy] ⚠ 代理未在 3 秒内就绪，继续启动（可能稍后可用）");
}


