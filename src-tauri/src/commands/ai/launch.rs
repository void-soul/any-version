use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use crate::commands::ai_registry::{registry, ToolConfig};
use crate::commands::hidden_cmd;
use super::models::*;

use super::config::{load_ai_config, load_last_launch_configs, save_last_launch_configs, load_sessions, save_sessions_to_file};
use super::terminal::{get_terminal_exe_cfg, is_ext_terminal};

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
        Some(p) => eprintln!("  ✓ 找到: name={}, anthropic_url={}, openai_url={}", p.name, p.anthropic_url, p.openai_url),
        None => eprintln!("  ✗ 未找到，将使用官方默认模型"),
    }

    // 确定各协议是否需要启动代理（双端口：
    //   P1 Anthropic 代理 = config.proxy_port，处理 /v1/messages
    //   P2 OpenAI 代理   = config.proxy_port + 1，处理 /v1/chat/completions
    // 两个协议可独立启用，互不影响。
    let anthropic_use_proxy = provider.map_or(false, |p| p.anthropic_use_proxy);
    let openai_use_proxy = provider.map_or(false, |p| p.openai_use_proxy);
    eprintln!("\n[proxy] anthropic_use_proxy={}, openai_use_proxy={}", anthropic_use_proxy, openai_use_proxy);

    if let Some(p) = provider {
        if !p.api_key.is_empty() {
            let proxy_settings = &registry().terminals().proxy_settings;
            let timeout = proxy_settings.timeout_seconds as u64;

            // ── P1: Anthropic 代理（config.proxy_port）──
            if anthropic_use_proxy {
                let proxy_config = crate::proxy::types::ProxyConfig {
                    listen_address: proxy_settings.listen_address.clone(),
                    listen_port: config.proxy_port,
                    upstream_base_url: p.openai_url.clone(),
                    upstream_api_key: p.api_key.clone(),
                    upstream_anthropic_url: p.anthropic_url.clone(),
                    upstream_protocol: "anthropic".to_string(),
                    target_model: req.model_id.clone().unwrap_or_default(),
                    timeout_secs: timeout,
                    model_aliases: p.anthropic_model_aliases.clone(),
                    default_model: p.anthropic_default_model.clone(),
                    rectifier_enabled: config.rectifier.enabled,
                    rectifier_thinking_signature: config.rectifier.thinking_signature,
                    rectifier_thinking_budget: config.rectifier.thinking_budget,
                    rectifier_media_fallback: config.rectifier.media_fallback,
                    optimizer_enabled: config.optimizer.enabled,
                    optimizer_cache_injection: config.optimizer.cache_injection,
                    optimizer_thinking: config.optimizer.thinking_optimizer,
                    optimizer_deepseek: config.optimizer.deepseek_normalize,
                };
                eprintln!("[proxy] ✓ 启动 Anthropic 代理 -> 127.0.0.1:{}", config.proxy_port);
                tokio::spawn(async move {
                    if let Err(e) = crate::proxy::server::start_proxy_server(proxy_config).await {
                        eprintln!("[proxy] Anthropic 代理错误: {}", e);
                    }
                });
                wait_for_proxy_ready(&proxy_settings.listen_address, config.proxy_port).await;
            }

            // ── P2: OpenAI 代理（config.proxy_port + 1）──
            if openai_use_proxy {
                let proxy_config = crate::proxy::types::ProxyConfig {
                    listen_address: proxy_settings.listen_address.clone(),
                    listen_port: config.proxy_port + 1,
                    upstream_base_url: p.openai_url.clone(),
                    upstream_api_key: p.api_key.clone(),
                    upstream_anthropic_url: String::new(),
                    upstream_protocol: "openai".to_string(),
                    target_model: String::new(),
                    timeout_secs: timeout,
                    model_aliases: p.openai_model_aliases.clone(),
                    default_model: p.openai_default_model.clone(),
                    // OpenAI 透传无需 Anthropic 整流器/优化器
                    rectifier_enabled: false,
                    rectifier_thinking_signature: false,
                    rectifier_thinking_budget: false,
                    rectifier_media_fallback: false,
                    optimizer_enabled: false,
                    optimizer_cache_injection: false,
                    optimizer_thinking: false,
                    optimizer_deepseek: false,
                };
                eprintln!("[proxy] ✓ 启动 OpenAI 代理 -> 127.0.0.1:{}", config.proxy_port + 1);
                tokio::spawn(async move {
                    if let Err(e) = crate::proxy::server::start_proxy_server(proxy_config).await {
                        eprintln!("[proxy] OpenAI 代理错误: {}", e);
                    }
                });
                wait_for_proxy_ready(&proxy_settings.listen_address, config.proxy_port + 1).await;
            }
        }
    }

    eprintln!("\n──────────────────────────────────────────────────────────────");
    eprintln!(" Step 2: 写入工具配置文件（含 env.* 前缀的环境变量注入）");
    eprintln!("──────────────────────────────────────────────────────────────");

    // 写入工具的配置文件（由 config.json 的 configFile 字段驱动）
    // 无论是否使用代理，都需要写入上游 URL、模型等信息到配置文件
    if tool_config.config_file.is_some() {
        if let Some(ref p) = provider {
            if !p.api_key.is_empty() {
                // 根据工具协议选择对应的上游 URL
                // 关键：代理供应商（anthropic_use_proxy=true）的 anthropic_url 可能为空，
                // 实际 API 端点在 openai_url。这里做 fallback 确保写入不会因为空 URL 被跳过。
                let upstream_url = match tool_config.api_protocol.as_str() {
                    "openai" => {
                        if !p.openai_url.is_empty() { &p.openai_url }
                        else { &p.anthropic_url }
                    }
                    _ => {
                        if !p.anthropic_url.is_empty() { &p.anthropic_url }
                        else { &p.openai_url }
                    }
                };
                // 使用代理时，baseUrl 指向对应协议的本地代理端口：
                //   anthropic/both 协议 → P1 (proxy_port)，openai 协议 → P2 (proxy_port+1)；
                //   未启用对应协议代理时写真实上游 URL。以前这步只靠 env 注入，现直接写配置文件。
                let effective_base_url: String = match tool_config.api_protocol.as_str() {
                    "openai" if openai_use_proxy => format!("http://127.0.0.1:{}", config.proxy_port + 1),
                    "anthropic" | "both" if anthropic_use_proxy => format!("http://127.0.0.1:{}", config.proxy_port),
                    _ => upstream_url.to_string(),
                };
                // 根据工具协议选择对应的模型别名和默认模型
                let effective_aliases: &HashMap<String, String> = if tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both" {
                    &p.anthropic_model_aliases
                } else if tool_config.api_protocol == "openai" {
                    &p.openai_model_aliases
                } else {
                    &p.google_model_aliases
                };
                let effective_default: &Option<String> = if tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both" {
                    &p.anthropic_default_model
                } else if tool_config.api_protocol == "openai" {
                    &p.openai_default_model
                } else {
                    &p.google_default_model
                };
                if !upstream_url.is_empty() {
                    // 代理模式：仅当 anthropic_use_proxy 且工具为 anthropic/both 协议时生效，
                    // 配置文件中只写 baseUrl，模型/别名保持原版由代理按请求模型转发。
                    let proxy_mode = anthropic_use_proxy
                        && (tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both");
                    eprintln!("[config_file] 写入参数:");
                    eprintln!("[config_file]   tool_id: {}", req.tool_id);
                    eprintln!("[config_file]   provider: id={}, name={}", p.id, p.name);
                    eprintln!("[config_file]   protocol: {}", tool_config.api_protocol);
                    eprintln!("[config_file]   upstream_url: {}", upstream_url);
                    eprintln!("[config_file]   effective_base_url: {}", effective_base_url);
                    eprintln!("[config_file]   model_id: {:?}", req.model_id);
                    eprintln!("[config_file]   proxy_mode: {}", proxy_mode);
                    eprintln!("[config_file]   alias_count: {}, default_model: {:?}", effective_aliases.len(), effective_default);
                    match write_tool_config_from_spec(
                        &req.tool_id,
                        &tool_config,
                        req.model_id.as_deref(),
                        Some(&effective_base_url),
                        &effective_base_url,
                        &p.api_key,
                        req.fallback_model_id.as_deref(),
                        req.one_m_context,
                        effective_aliases,
                        effective_default.as_deref(),
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
    if let Some(ref cf) = tool_config.config_file {
        if let Some(ref write_map) = cf.write {
            if let Some(ref p) = provider {
                let effective_base_url = match tool_config.api_protocol.as_str() {
                    "openai" if openai_use_proxy => format!("http://127.0.0.1:{}", config.proxy_port + 1),
                    "anthropic" | "both" if anthropic_use_proxy => format!("http://127.0.0.1:{}", config.proxy_port),
                    _ => p.openai_url.clone(),
                };
                let model = req.model_id.as_deref().unwrap_or("");
                for (path, value_template) in write_map {
                    if path.starts_with("env.") {
                        let env_key = &path[4..];
                        let env_value = match value_template.as_str() {
                            "apiKey" => p.api_key.clone(),
                            "baseUrl" => effective_base_url.clone(),
                            "model" | "modelName" => model.to_string(),
                            other => other.to_string(),
                        };
                        if !env_value.is_empty() {
                            eprintln!("[spawn] env {} = {}", env_key, mask_secret(&env_value));
                            cmd.env(env_key, env_value);
                        }
                    }
                }
            }
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
        use_official_model: is_official,
        terminal_id: req.terminal_id.clone(),
        one_m_context: req.one_m_context,
        project_path: lc_project_path,
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
fn write_tool_config_from_spec(
    tool_id: &str,
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    base_url: Option<&str>,
    fallback_url: &str,
    api_key: &str,
    fallback_model_id: Option<&str>,
    one_m_context: bool,
    model_aliases: &HashMap<String, String>,
    default_model: Option<&str>,
    proxy_mode: bool,
) -> Result<(), String> {
    if let Some(ref _cfg) = tool_config.config_file {
        return write_tool_config_generic(tool_config, model_id, base_url.unwrap_or(fallback_url), api_key, fallback_model_id, model_aliases, default_model, one_m_context, proxy_mode);
    }
    // 无 configFile 定义的工具完全依赖 CLI 参数，无需写配置
    let _ = tool_id;
    Ok(())
}

/// 根据 modelFormat 配置格式化模型名
fn format_model_name(raw: &str, tool_config: &ToolConfig) -> String {
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

/// 反查 `anthropic_model_aliases`（role → 供应商模型），找出选中供应商模型对应的
/// 角色别名（sonnet/opus/haiku/fable）。用于将 `ANTHROPIC_MODEL` 写成角色别名，
/// 而不是供应商模型名（后者会被 Claude Code 原样发往上游而被拒）。
fn role_for_model(model_id: Option<&str>, aliases: &HashMap<String, String>) -> Option<String> {
    let target = model_id?
        .replace("[1m]", "")
        .replace("[1M]", "")
        .trim()
        .to_lowercase();
    for (role, mapped) in aliases {
        let m = mapped
            .replace("[1m]", "")
            .replace("[1M]", "")
            .trim()
            .to_lowercase();
        if m == target {
            return Some(role.clone());
        }
    }
    None
}

/// 通用工具配置文件写入：根据 config.json 的 configFile.write 映射写入。
/// 支持 json / jsonc（serde_json）与 toml（行式）两种格式。
fn write_tool_config_generic(
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    base_url: &str,
    api_key: &str,
    fallback_model_id: Option<&str>,
    model_aliases: &HashMap<String, String>,
    default_model: Option<&str>,
    one_m_context: bool,
    proxy_mode: bool,
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

    // 组装待写入的 (路径, 值) 列表
    let mut writes: Vec<(String, String)> = Vec::new();
    let effective_model_id = model_id.or(default_model);
    let has_model = effective_model_id.is_some();
    let model = effective_model_id
        .map(|m| format_model_name_with_ctx(m, tool_config, apply_one_m))
        .unwrap_or_default();
    let model_name = model.split('/').next_back().unwrap_or(&model).to_string();
    let fallback_model = fallback_model_id.and_then(|fm| {
        if fm.is_empty() { return None; }
        Some(format_model_name(fm, tool_config))
    });

    for (path, value_template) in write_map {
        // 动态键名替换：{model_name} → 实际模型名
        let resolved_path = path.replace("{model_name}", &model_name);
        // 代理模式：只写连接参数（baseUrl/apiKey 等），模型相关字段保持原版，
        // 由本地代理按请求中的官方模型名转发到供应商对应模型。
        if proxy_mode
            && matches!(value_template.as_str(), "model" | "modelName" | "fallbackModel")
        {
            eprintln!("[config_file] skip {} (proxy_mode, 模型由代理解析)", resolved_path);
            continue;
        }
        let value = match value_template.as_str() {
            "model" | "modelName" | "fallbackModel" if !has_model => {
                eprintln!("[config_file] skip {} (no model)", resolved_path);
                continue;
            },
            "model" => {
                // anthropic/both 协议：ANTHROPIC_MODEL 必须是 Claude 角色别名
                // （sonnet/opus/haiku/fable）或完整 Claude 模型名，不能是供应商模型名
                // （官方 model-config.md：ANTHROPIC_MODEL=<alias|name>，且值会原样发给上游）。
                // 反查 anthropic_model_aliases，把选中的供应商模型改写为对应 role 别名，
                // 由 ANTHROPIC_DEFAULT_<ROLE>_MODEL / 代理按角色映射到供应商模型。
                if tool_config.api_protocol == "anthropic" || tool_config.api_protocol == "both" {
                    if let Some(role) = role_for_model(effective_model_id, model_aliases) {
                        role
                    } else {
                        model.clone()
                    }
                } else {
                    model.clone()
                }
            },
            "modelName" => model_name.clone(),
            "fallbackModel" => fallback_model.clone().unwrap_or_default(),
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

    // Provider 模型别名映射（sonnet/haiku/opus/fable → 实际模型名）。
    // 代理模式下不写入：模型保持官方原版，由代理按请求模型转发。
    if !proxy_mode {
    for (role, mapped_model) in model_aliases {
        let env_key = match role.to_lowercase().as_str() {
            "sonnet" => "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "opus" => "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "haiku" => "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "fable" => "ANTHROPIC_DEFAULT_FABLE_MODEL",
            _ => {
                eprintln!("[config_file] alias {} → {} (未识别的 role，跳过)", role, mapped_model);
                continue;
            },
        };
        let one_m_alias = apply_one_m && role.to_lowercase() != "haiku";
        let formatted = format_model_name_with_ctx(mapped_model, tool_config, one_m_alias);
        eprintln!("[config_file] set env.{} = {} (alias {} → {})", env_key, mask_secret(&formatted), role, mapped_model);
        writes.push((format!("env.{}", env_key), formatted));
    }
    } // end if !proxy_mode

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
fn mask_secret(v: &str) -> String {
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

/// 移除本工具管理的、但本次未写入的残留模型键。
///
/// - `env` 下以 `ANTHROPIC_MODEL` / `ANTHROPIC_DEFAULT_` / `ANTHROPIC_SMALL_FAST_MODEL`
///   开头的键，若不在本次写入集合内，则删除（避免旧供应商/旧模型字段残留）。
/// - 顶层 `model` 字段本应用从不写入（历史版本残留），一律清除以免干扰 Claude Code 的模型解析。
fn cleanup_managed_model_keys(doc: &mut serde_json::Value, writes: &[(String, String)]) {
    let managed_prefixes = ["ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_", "ANTHROPIC_SMALL_FAST_MODEL"];
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
