use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
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
    start_tool_proxy_with_collab(tool_config, provider, config, req, None, None).await
}

/// 带协作上下文的代理启动（collab 调用时传入 app_handle / room_id）
pub(crate) async fn start_tool_proxy_with_collab(
    tool_config: &ToolConfig,
    provider: Option<&AiProvider>,
    config: &AiConfig,
    req: &LaunchAiToolRequest,
    app_handle: Option<tauri::AppHandle>,
    collab_room_id: Option<String>,
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
    // 明确的「不启动」诊断，避免静默 (0, None) 让调用方/用户无从排查
    if provider.is_none() {
        eprintln!("[proxy] ✗ 代理未启动: 未传入 provider（工具未绑定供应商或未选模型）");
        return (0, None);
    }
    let p = provider.unwrap();
    if p.api_key.is_empty() {
        eprintln!("[proxy] ✗ 代理未启动: provider '{}' 的 api_key 为空（请在设置里配置密钥）", p.id);
        return (0, None);
    }
    if chosen_outbound.is_empty() {
        eprintln!(
            "[proxy] ✗ 代理未启动: 无可用出站协议 (inbound={}, provider '{}' 未配置匹配的协议 URL)",
            primary_inbound, p.id
        );
        return (0, None);
    }
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

            // 补登：工具配置文件实际写入的模型名带 modelFormat 前缀（如 anyversion/gpt-4o），
            // 而下面的 routes/aliases 仅按无前缀真实 id 建表。若不补，代理收到带前缀的声明名
            // → 查不到路由 → 回落上游并带错模型名 → 404 / 上游忽略未知模型用默认（"官方模型"）。
            let claimed_cfg = req.masquerade_model.clone()
                .filter(|c| !c.is_empty())
                .or_else(|| req.model_id.clone())
                .unwrap_or_default();
            let claimed_fmt = format_model_name(&claimed_cfg, &tool_config);
            if !claimed_fmt.is_empty() && claimed_fmt != target_model {
                model_aliases.insert(claimed_fmt.clone(), target_model.clone());
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

            // 补登带前缀的 claimed 模型名到路由表（C → 供应商端点），与上方 aliases 配对，
            // 使代理能识别工具发出的 anyversion/xxx 并正确路由。
            if !claimed_fmt.is_empty() && claimed_fmt != target_model {
                model_routes.insert(claimed_fmt.clone(), ModelRoute {
                    base_url: p.url_for(&chosen_outbound),
                    api_key: p.api_key.clone(),
                });
            }

            // 优化器 / 整流器：工具支持时可由启动请求开关覆盖，否则继承全局配置
            let optimizer_on = tool_config.supports_optimizer
                && req.optimizer_enabled.unwrap_or(true)
                && config.optimizer.enabled;
            let rectifier_on = tool_config.supports_rectifier
                && req.rectifier_enabled.unwrap_or(true)
                && config.rectifier.enabled;

            // 协议回退端点：持有"另一协议"的 URL+key，供 anthropic 出站遇
            // 401/404（上游实为 OpenAI 兼容）时自动以 openai 出站（a2o）重发。
            let (fallback_base_url, fallback_api_key) = match chosen_outbound.as_str() {
                "anthropic" => (
                    if p.openai_url.is_empty() { String::new() } else { p.url_for("openai") },
                    p.api_key.clone(),
                ),
                "openai" => (
                    if p.anthropic_url.is_empty() { String::new() } else { p.url_for("anthropic") },
                    p.api_key.clone(),
                ),
                _ => (String::new(), String::new()),
            };

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
                        fallback_base_url: fallback_base_url.clone(),
                        fallback_api_key: fallback_api_key.clone(),
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
                        app_handle: app_handle.clone(),
                        collab_room_id: collab_room_id.clone(),
                    };
                    eprintln!("[proxy] ✓ 启动代理 -> 127.0.0.1:{}  ({} -> {})", port, primary_inbound, outbound_protocol);
                    let handle = tokio::spawn(async move {
                        if let Err(e) = crate::proxy::server::serve_proxy(proxy_config, listener).await {
                            eprintln!("[proxy] 代理错误: {}", e);
                        }
                    });
                    abort_handle = Some(handle.abort_handle());
                    // 必须等到代理真正可服务再返回端口，否则子进程连上未就绪端口会
                    // 触发 undici "fetch failed"（即 dead port）。未就绪则中止并返回 0。
                    let ready = wait_for_proxy_ready(&proxy_settings.listen_address, port).await;
                    if !ready {
                        eprintln!("[proxy] ⚠ 代理就绪检查失败, 中止: port={}", port);
                        handle.abort();
                        return (0, None);
                    }
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
                        &tool_config,
                        req.model_id.as_deref(),
                        claimed_model.as_deref(),
                        &effective_base_url,
                        &p.api_key,
                        req.fallback_model_id.as_deref(),
                        req.fallback_masquerade_model.as_deref(),
                        req.one_m_context,
                        req.fallback_one_m_context,
                        proxy_mode,
                        &req.custom_params,
                        &req.custom_param_values,
                        req.web_search_enabled,
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

    // powershell/pwsh 自身即为可见窗口，不能用 hidden_cmd（CREATE_NO_WINDOW），
    // 否则窗口被隐藏，表现为"启动成功但没反应"；cmd/wt/外部终端用 hidden_cmd，
    // 因为它们会再 spawn 出可见子窗口，隐藏父进程无影响。
    let is_powershell = terminal_exe.to_lowercase().contains("powershell")
        || terminal_exe.to_lowercase().contains("pwsh");
    let mut cmd = if is_powershell {
        let mut c = Command::new(&terminal_exe);
        // CREATE_NEW_CONSOLE：强制为新控制台子进程分配独立窗口，
        // 避免 GUI 父进程下的 powershell 不弹出可见窗口（表现为"启动成功但没反应"）。
        #[cfg(windows)]
        c.creation_flags(0x00000010);
        c
    } else {
        hidden_cmd::hidden_cmd(&terminal_exe)
    };
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
        // 安全过滤 PowerShell 命令注入字符（白名单：仅允许字母数字、空格、连字符、点、下划线、斜杠）
        // 注意：PowerShell 支持多种注入方式（子表达式、调用运算符等），白名单比黑名单更安全
        let sanitize_pwsh = |s: &str| -> String {
            s.chars()
                .filter(|c| {
                    c.is_alphanumeric()
                        || matches!(c, ' ' | '-' | '_' | '.' | '/' | '\\' | ':' | '(' | ')' | ',')
                })
                .collect()
        };
        let safe_start = sanitize_pwsh(&start_cmd);
        let safe_args = sanitize_pwsh(&tool_args);
        let run_cmd = if safe_args.is_empty() {
            format!("Set-Location -LiteralPath '{}'; {}", escaped_path, &safe_start)
        } else {
            format!("Set-Location -LiteralPath '{}'; {} {}", escaped_path, &safe_start, &safe_args)
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

    // 注入模型自定义的「env 目标」启动参数（与工具 config.json 的 env.* 约定一致）
    for cp in &req.custom_params {
        if cp.target != "env" { continue; }
        let env_key = match &cp.env_key {
            Some(k) if !k.is_empty() => k.clone(),
            // 未显式给 env_key 时回退用 key 本身
            _ => cp.key.clone(),
        };
        let value = req.custom_param_values.get(&cp.key)
            .cloned()
            .or_else(|| cp.default_value.clone())
            .unwrap_or_default();
        if value.is_empty() { continue; }
        eprintln!("[spawn] custom-env {} = {} (param: {})", env_key, mask_secret(&value), cp.label);
        cmd.env(env_key, value);
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
        custom_param_values: req.custom_param_values.clone(),
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
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    claimed_model: Option<&str>,
    base_url: &str,
    api_key: &str,
    fallback_model_id: Option<&str>,
    fallback_masquerade_model: Option<&str>,
    one_m_context: bool,
    fallback_one_m_context: bool,
    proxy_mode: bool,
    custom_params: &[ModelCustomParam],
    custom_param_values: &HashMap<String, String>,
    web_search: bool,
) -> Result<(), String> {
    // write_tool_config_generic 内部会检查 config_file 是否存在，无 configFile 时直接返回 Ok(())
    write_tool_config_generic(tool_config, model_id, claimed_model, base_url, api_key, fallback_model_id, fallback_masquerade_model, one_m_context, fallback_one_m_context, proxy_mode, custom_params, custom_param_values, web_search)
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
/// 模型名可能含 "."（如 LongCat-2.0 / gpt-4.1），而配置路径用 "." 作层级分隔符。
/// 写路径前将模型名里的 "." 转义为此占位符，set_json_path 落盘时再还原，
/// 避免 "provider.anyversion.models.LongCat-2.0.name" 被误拆成 LongCat-2 → 0 → name。
const MODEL_NAME_DOT_ESCAPE: &str = "__DOT__";

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
    custom_params: &[ModelCustomParam],
    custom_param_values: &HashMap<String, String>,
    web_search: bool,
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
    // 组装待写入的 (路径, 值) 列表；值可以是标量字符串或 JSON 数组（如 pi 的 models）
    let mut writes: Vec<(String, serde_json::Value)> = Vec::new();
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

    // 小模型（fallback）的模型名（不带 provider 前缀），用于注册进 provider 的 models 映射
    let fallback_model_name = fallback_claimed
        .as_deref()
        .map(|m| m.split('/').next_back().unwrap_or(m).to_string())
        .unwrap_or_default();

    for (path, value_template) in write_map {
        // env.* 键只应作为进程环境变量注入（见 build_env_vars），不应写入工具配置文件：
        // - opencode 及其 fork（mimocode / deveco / kilocode）的配置 schema 不识别顶层 env 键，
        //   写入会触发 "Unrecognized key: env" 导致工具启动失败；
        // - claude / codex / gemini / qwen 等原生配置虽支持 env，但进程 env 注入已足够，
        //   统一跳过既避免重复写入，也规避 opencode 系 fork 的 schema 不兼容。
        if path.starts_with("env.") {
            eprintln!("[config_file] skip {} (env 仅注入进程环境，不写配置文件)", path);
            continue;
        }
        // 动态键名替换：{model_name} → 主模型名；{fallback_model_name} → 小模型名
        // 模型名里的 "." 先转义为占位符，避免被 set_json_path 当成路径分隔符误拆
        if path.contains("{fallback_model_name}") && fallback_claimed.is_none() {
            eprintln!("[config_file] skip {} (no fallback model)", path);
            continue;
        }
        let resolved_path = path
            .replace("{model_name}", &model_name.replace('.', MODEL_NAME_DOT_ESCAPE))
            .replace("{fallback_model_name}", &fallback_model_name.replace('.', MODEL_NAME_DOT_ESCAPE));
        let value: serde_json::Value = match value_template.as_str() {
            "model" | "modelName" if !has_model => {
                eprintln!("[config_file] skip {} (no model)", resolved_path);
                continue;
            },
            "model" => serde_json::json!(model.clone()),
            "modelName" => serde_json::json!(model_name.clone()),
            "fallbackModel" => match &fallback_claimed {
                Some(v) => serde_json::json!(v.clone()),
                None => {
                    eprintln!("[config_file] skip {} (no fallback model)", resolved_path);
                    continue;
                }
            },
            "fallbackModelName" => match &fallback_claimed {
                Some(_) => serde_json::json!(fallback_model_name.clone()),
                None => {
                    eprintln!("[config_file] skip {} (no fallback model)", resolved_path);
                    continue;
                }
            },
            // pi：把主模型（及可选 fallback）注册进 providers.<p>.models 数组
            // 官方 models.json 要求自定义 provider 在 models 数组里声明可用模型（id/name），
            // 否则 /model 列表为空、--model 也可能找不到模型。
            "piModels" => {
                if !has_model {
                    eprintln!("[config_file] skip {} (no model)", resolved_path);
                    continue;
                }
                let mut arr = vec![serde_json::json!({ "id": model_name, "name": model_name })];
                if let Some(fb) = &fallback_claimed {
                    let fb_name = fb.split('/').next_back().unwrap_or(fb).to_string();
                    arr.push(serde_json::json!({ "id": fb_name, "name": fb_name }));
                }
                serde_json::Value::Array(arr)
            },
            "baseUrl" => serde_json::json!(base_url.to_string()),
            // 布尔字面量（如 pi 的 compat.supportsDeveloperRole=false）
            "boolFalse" => serde_json::json!(false),
            "boolTrue" => serde_json::json!(true),
            // Codex 的 web_search 开关：开启 → "live"（真实实时检索）；关闭 → 不写该键，
            // 保留 Codex 默认 "cached"（OpenAI 维护索引，对第三方上游无实际 web 访问）。
            // 默认关，用户开启才写 live。
            "webSearchLive" => {
                if !web_search {
                    eprintln!("[config_file] skip {} (web_search 未开启)", resolved_path);
                    continue;
                }
                serde_json::json!("live")
            },
            "apiKey" => {
                // API Key 为空时不写入配置文件，避免写入空字符串被解析器判定为非法凭证
                if api_key.is_empty() {
                    eprintln!("[config_file] skip {} (empty apiKey, 不写入)", resolved_path);
                    continue;
                }
                serde_json::json!(api_key.to_string())
            },
            "" => serde_json::json!(""),
            other => serde_json::json!(other.to_string()),
        };
        let log = if value.is_string() {
            mask_secret(value.as_str().unwrap())
        } else {
            "<json>".to_string()
        };
        eprintln!("[config_file] set {} = {}", resolved_path, log);
        writes.push((resolved_path, value));
    }

    // 追加模型自定义的「config 目标」启动参数（写入工具配置文件指定 JSON 路径）
    for cp in custom_params {
        if cp.target != "config" { continue; }
        let path = match &cp.config_path {
            Some(p) if !p.is_empty() => p.clone(),
            _ => continue, // 未给 config_path 则无法落盘，跳过
        };
        let value = custom_param_values.get(&cp.key)
            .cloned()
            .or_else(|| cp.default_value.clone())
            .unwrap_or_default();
        eprintln!("[config_file] set (custom) {} = {}", path, mask_secret(&value));
        writes.push((path, serde_json::json!(value)));
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
/// 注意：对于 fallback 模型相关的环境变量（如 ANTHROPIC_DEFAULT_HAIKU_MODEL），如果已有值不为空，
/// 则不覆盖，避免影响其他正在运行的工具实例。
fn write_json_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
    schema: Option<&str>,
    _tool_config: &ToolConfig,
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
        // 对于 fallback 模型环境变量，如果已有值不为空，则不覆盖
        // 这样可以避免影响其他正在运行的工具实例
        if p.starts_with("env.ANTHROPIC_DEFAULT_") && p.ends_with("_MODEL") {
            if let Some(existing_val) = doc.get(p).and_then(|v| v.as_str()) {
                if !existing_val.is_empty() {
                    eprintln!("[config_file] skip {} (已有值={}, 避免影响其他实例)", p, existing_val);
                    continue;
                }
            }
        }
        set_json_path(&mut doc, p, v.clone());
    }

    // 清除受管 provider 的 models 映射中因历史 dot bug 产生的畸形条目
    // （形如 { "LongCat-2": { "0": { "name": ... } } }），避免工具配置解析失败
    cleanup_broken_model_entries(&mut doc, writes);

    // 清除本工具管理的、但本次未写入的残留键。
    // 合并写入不会删除旧键，导致切换供应商/模型后旧模型字段残留在配置里，干扰本次启动。
    cleanup_managed_model_keys(&mut doc, writes);

    let content = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化配置失败: {}", e))?;
    fs::write(path, content)
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 移除本工具管理的、但本次未写入的残留键。
///
/// 动态基于写入列表与通用 AI 供应商环境变量前缀池（ANTHROPIC_, OPENAI_, GEMINI_, DEVECO_ 等）
/// 清理属于受管范围但本次未写入的旧模型/Auth键，无硬编码适用于所有 CLI 工具。
fn cleanup_managed_model_keys(doc: &mut serde_json::Value, writes: &[(String, serde_json::Value)]) {
    let current_env_keys: std::collections::HashSet<String> = writes
        .iter()
        .filter_map(|(p, _)| p.strip_prefix("env.").map(|k| k.to_string()))
        .collect();

    let mut managed_prefixes: Vec<String> = vec![
        "ANTHROPIC_".to_string(),
        "OPENAI_".to_string(),
        "GEMINI_".to_string(),
        "DEVECO_".to_string(),
        "KILO_".to_string(),
    ];

    for key in &current_env_keys {
        if let Some(pos) = key.find('_') {
            let prefix = key[..=pos].to_string();
            if !managed_prefixes.contains(&prefix) {
                managed_prefixes.push(prefix);
            }
        }
    }

    if let Some(env_obj) = doc.get_mut("env").and_then(|v| v.as_object_mut()) {
        let stale: Vec<String> = env_obj
            .keys()
            .filter(|k| {
                let is_managed = managed_prefixes.iter().any(|p| k.starts_with(p))
                    && (k.contains("MODEL") || k.contains("AUTH_TOKEN") || k.contains("API_KEY") || k.contains("BASE_URL"));
                is_managed && !current_env_keys.contains(*k)
            })
            .cloned()
            .collect();
        for k in stale {
            env_obj.remove(&k);
            eprintln!("[config_file] cleanup stale env.{}", k);
        }
    }

    let writes_top_model = writes.iter().any(|(p, _)| p == "model");
    if !writes_top_model {
        if let Some(obj) = doc.as_object_mut() {
            if obj.remove("model").is_some() {
                eprintln!("[config_file] cleanup stale top-level model");
            }
        }
    }
}

/// 清理受管 provider 的 `models` 映射中，因历史 bug（模型名含 "." 被误拆成嵌套键）
/// 产生的畸形条目。畸形条目表现为：值不是合法的模型定义对象（缺少 `name` 键，
/// 而是形如 `{ "0": { "name": ... } }` 的嵌套结构）。仅删除畸形条目，保留合法条目
/// （含当前正写入的模型与历史正常注册的模型），避免工具配置整体解析失败。
fn cleanup_broken_model_entries(doc: &mut serde_json::Value, writes: &[(String, serde_json::Value)]) {
    // 从本次写入收集受管 provider（路径形如 provider.<P>.models.<M>.name）
    let mut managed: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (p, _) in writes {
        if let Some(rest) = p.strip_prefix("provider.") {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 4 && parts[1] == "models" && parts.last() == Some(&"name") {
                let provider = parts[0].to_string();
                let model_key = parts[2].replace(MODEL_NAME_DOT_ESCAPE, ".");
                managed.insert(provider, model_key);
            }
        }
    }
    if managed.is_empty() {
        return;
    }
    if let Some(provider_obj) = doc.get_mut("provider").and_then(|v| v.as_object_mut()) {
        for (provider, current_model) in &managed {
            if let Some(models_obj) = provider_obj
                .get_mut(provider)
                .and_then(|v| v.as_object_mut())
                .and_then(|p| p.get_mut("models"))
                .and_then(|m| m.as_object_mut())
            {
                let stale: Vec<String> = models_obj
                    .iter()
                    .filter(|(k, v)| {
                        // 保留当前模型；其余若值不是合法模型定义（缺 name 键）则视为畸形，删除
                        **k != *current_model && !(v.is_object() && v.get("name").is_some())
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in stale {
                    models_obj.remove(&k);
                    eprintln!("[config_file] cleanup broken model entry provider.{}.models.{}", provider, k);
                }
            }
        }
    }
}

/// 把 JSON 值降级为 TOML 标量字符串（数组/对象退回 JSON 文本；codex 不会用到后者）
fn toml_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 写入 TOML 配置文件（支持顶层 key 和 dotted keys 如 `model_providers.x.base_url`）。
///
/// 关键修复：codex 等严格 TOML 解析器在 `[model_providers.anyversion]` 表头内已有
/// `env_key`/`name`/`base_url`，旧逻辑只剥离顶层 dotted key、却保留表内同键，末位再追加
/// `model_providers.anyversion.*` dotted key 会与表内键冲突 → duplicate key 报错。
/// 新版逐行跟踪当前 `[table]` 上下文，算出每行完整点分 key 再做去重/原地替换。
fn write_toml_config(
    path: &PathBuf,
    existing: &str,
    writes: &[(String, serde_json::Value)],
) -> Result<(), String> {
    // 仅处理非 env.* 的键（env.* 走环境变量注入）；JSON 值统一降级为标量字符串
    // （codex 等 TOML 工具的写入值均为标量，不会传入数组/对象）
    let toml_writes: Vec<(String, String)> = writes
        .iter()
        .filter(|(p, _)| !p.starts_with("env."))
        .map(|(p, v)| (p.clone(), toml_scalar(v)))
        .collect();
    let mut pending: std::collections::HashMap<String, String> = toml_writes.into_iter().collect();

    let lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();

    // 第一遍：找出文件中已存在的目标键（含 [table] 上下文），这些键原地替换；
    // 其余(新增)键插入对应 [table] 头之后或追加到末尾。
    let mut current_table: Vec<String> = Vec::new();
    let mut existing_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in &lines {
        let trimmed = line.trim_start();
        if let Some(stripped) = trimmed.strip_prefix('[') {
            if !trimmed.starts_with("[[") {
                if let Some(close) = stripped.find(']') {
                    let header = &stripped[..close];
                    current_table = header.split('.').map(|s| s.to_string()).collect();
                }
            }
            continue;
        }
        if let Some(k) = leading_toml_key(line) {
            // TOML 语义：[table] 内的键（含 dotted key）都相对于当前表，必须拼表前缀；
            // 只有表外（文件头部）的键才是顶层全限定键
            let full = if current_table.is_empty() {
                k.clone()
            } else {
                format!("{}.{}", current_table.join("."), k)
            };
            if pending.contains_key(&full) {
                existing_keys.insert(full);
            }
        }
    }

    // 第二遍：生成输出，原地替换已存在的键
    current_table.clear();
    let mut out: Vec<String> = Vec::new();
    for line in &lines {
        let trimmed = line.trim_start();
        if let Some(stripped) = trimmed.strip_prefix('[') {
            if !trimmed.starts_with("[[") {
                if let Some(close) = stripped.find(']') {
                    let header = &stripped[..close];
                    current_table = header.split('.').map(|s| s.to_string()).collect();
                    out.push(line.clone());
                    // 把"新增"的、属于该表的直接子键插入表头之后
                    let prefix = current_table.join(".");
                    let mut inserted: Vec<String> = Vec::new();
                    for (k, v) in pending.iter() {
                        if let Some(rest) = k.strip_prefix(&format!("{}.", prefix)) {
                            if !rest.contains('.') && !existing_keys.contains(k) {
                                out.push(format!("{} = \"{}\"", rest, v));
                                inserted.push(k.clone());
                            }
                        }
                    }
                    for k in &inserted {
                        pending.remove(k);
                    }
                    continue;
                }
            }
            out.push(line.clone());
            continue;
        }
        if let Some(k) = leading_toml_key(line) {
            // TOML 语义：[table] 内的键（含 dotted key）都相对于当前表，必须拼表前缀
            let full = if current_table.is_empty() {
                k.clone()
            } else {
                format!("{}.{}", current_table.join("."), k)
            };
            if let Some(v) = pending.remove(&full) {
                let indent = line.len() - line.trim_start().len();
                out.push(format!("{}{} = \"{}\"", " ".repeat(indent), k, v));
            } else {
                out.push(line.clone());
            }
        } else {
            out.push(line.clone());
        }
    }

    // 仍未处理的键（文件里没有对应表头）：
    // - 无点顶层键必须插到首个 [table] 之前（追加到末尾会落进最后一个表的作用域）
    // - 带点键按父路径分组，生成 `[parent]` 表头 + 短键。旧逻辑直接把
    //   `model_providers.anyversion.*` dotted key 追加到末尾，落进 [model_providers.custom]
    //   作用域变成 custom.model_providers.anyversion.* → codex 报 "provider anyversion not found"
    let mut rest: Vec<(String, String)> = pending.into_iter().collect();
    rest.sort();
    let mut top_level: Vec<String> = Vec::new();
    let mut grouped: std::collections::BTreeMap<String, Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for (k, v) in rest {
        match k.rsplit_once('.') {
            None => top_level.push(format!("{} = \"{}\"", k, v)),
            Some((parent, leaf)) => grouped
                .entry(parent.to_string())
                .or_default()
                .push((leaf.to_string(), v)),
        }
    }
    if !top_level.is_empty() {
        let insert_at = out
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .unwrap_or(out.len());
        for (i, line) in top_level.into_iter().enumerate() {
            out.insert(insert_at + i, line);
        }
    }
    for (parent, kvs) in grouped {
        out.push(String::new());
        out.push(format!("[{}]", parent));
        for (leaf, v) in kvs {
            out.push(format!("{} = \"{}\"", leaf, v));
        }
    }

    let content = out.join("\n");
    fs::write(path, content)
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

/// 提取 TOML 赋值行的完整键名（支持 dotted key，如 model_providers.anyversion.env_key）。
/// 非赋值行（注释 / [table] 头 / 空行）返回 None。
fn leading_toml_key(line: &str) -> Option<String> {
    let re = regex::Regex::new(r"^\s*[A-Za-z_][\w.]*(?:\.[\w.]+)*\s*=").expect("valid regex");
    re.captures(line)
        .and_then(|c| c.get(0))
        .map(|m| {
            let s = m.as_str();
            s[..s.len() - 1].trim().to_string() // 去掉末尾的 '='
        })
}

/// 根据点分路径设置 JSON 文档中的值（自动创建中间对象）
fn set_json_path(doc: &mut serde_json::Value, path: &str, value: serde_json::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }
    let mut cur = doc;
    for (i, p) in parts.iter().enumerate() {
        // 还原模型名中被转义的 "."（如 LongCat-2__DOT__0 → LongCat-2.0）
        let key = p.replace(MODEL_NAME_DOT_ESCAPE, ".");
        if i == parts.len() - 1 {
            if !cur.is_object() {
                *cur = serde_json::json!({});
            }
            cur.as_object_mut().unwrap().insert(key, value);
            return;
        }
        if !cur.is_object() {
            *cur = serde_json::json!({});
        }
        if cur.get(&key).is_none() || !cur[&key].is_object() {
            cur.as_object_mut().unwrap().insert(key.clone(), serde_json::json!({}));
        }
        cur = cur.as_object_mut().unwrap().get_mut(&key).unwrap();
    }
}

/// 轮询代理服务器的 /health 端点，等待代理就绪。
/// 最多重试 50 次（每次 100ms），总计最多 5 秒。返回是否就绪。
async fn wait_for_proxy_ready(listen_address: &str, port: u16) -> bool {
    let health_url = format!("http://{}:{}/health", listen_address, port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap_or_default();
    for i in 0..50u32 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if client.get(&health_url).send().await.is_ok() {
            eprintln!("[proxy] ✓ 代理就绪 (尝试 {} 次)", i + 1);
            return true;
        }
    }
    eprintln!("[proxy] ⚠ 代理未在 5 秒内就绪");
    false
}


