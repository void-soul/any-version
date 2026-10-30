//! process launch responsibility.
use super::*;

// ─── 启动 AI 工具 ───

#[tauri::command]
pub async fn launch_ai_tool(req: LaunchAiToolRequest) -> Result<serde_json::Value, String> {
    eprintln!("══════════════════════════════════════════════════════════════");
    eprintln!("                    启动 AI 工具");
    eprintln!("══════════════════════════════════════════════════════════════");

    let config = load_ai_config();
    let tool_config = registry()
        .get_tool_config(&req.tool_id)
        .ok_or("未知工具")?
        .clone();
    let tool_paths = registry()
        .get_path_config(&req.tool_id)
        .ok_or("未知工具")?
        .clone();
    let provider = req
        .provider_id
        .as_ref()
        .and_then(|pid| config.providers.iter().find(|p| &p.id == pid));

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
    let (proxy_port, _proxy_abort, proxy_token) =
        start_tool_proxy(&tool_config, provider, &config, &req).await;

    // 本次启动对外（配置文件/env）的「鉴权 key」：代理模式下用随机 token（工具只会
    // 连本地代理，真实上游 key 不落盘也不进子进程环境）；未启动代理时回退真实 key。
    let effective_api_key: String = if proxy_port != 0 {
        proxy_token.clone()
    } else {
        provider
            .as_ref()
            .map(|p| p.api_key.clone())
            .unwrap_or_default()
    };

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
                // Claude Desktop / ChatGPT 桌面端由 `effective_claimed_model` 自动补一个
                // 合法官方别名 —— 与上面注册 `model_aliases` 用的是同一个函数，两边必然一致。
                let claimed_model = Some(
                    crate::commands::ai::tool_config_custom::effective_claimed_model(
                        &tool_config.id,
                        req.model_id.as_deref().unwrap_or(""),
                        req.masquerade_model.as_deref().unwrap_or(""),
                    ),
                );

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
                        &effective_api_key,
                        // 真实上游：代理模式下 effective_base_url 是 127.0.0.1，
                        // 写「厂商」这类展示信息要用上游（见 custom 写入器）
                        &upstream_url,
                        req.fallback_model_id.as_deref(),
                        req.fallback_masquerade_model.as_deref(),
                        req.one_m_context,
                        req.fallback_one_m_context,
                        proxy_mode,
                        &req.custom_params,
                        &req.custom_param_values,
                        req.web_search_enabled,
                        &chosen_outbound,
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
            // 勾了「使用官方模型」：不是「什么都不写」，而是**把我们写进去的还原掉**。
            // 否则工具还在用上一次写入的 Kira 代理地址/自定义 provider —— 代理早随着上次
            // 启动退出了，请求只会连不上（或继续走 Kira 的模型，与用户选择相反）。
            eprintln!("[config_file] 使用官方模型 → 还原工具自带配置");
            match crate::commands::ai::tool_config_restore::restore_tool_config(&tool_config) {
                Ok(outcome) => {
                    for note in &outcome.notes {
                        eprintln!("[config_file]   {note}");
                    }
                    for file in &outcome.files {
                        eprintln!("[config_file]   已还原: {file}");
                    }
                }
                Err(e) => eprintln!("[config_file] ⚠ 还原失败: {e}"),
            }
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
    let tool_exe = tool_paths
        .detect_cmd
        .split_whitespace()
        .next()
        .unwrap_or(&tool_config.id)
        .to_string();

    // 启动命令（来自 startCommand，可能包含默认参数如 "mimo ."）。
    // 桌面应用的 startCommand 是空的——它们按 exe 路径启动，这里补上检测到的绝对路径，
    // 否则会拿着一条空命令去启（表现为「点了启动，终端里什么都没有」）。
    let start_cmd = resolve_start_command(&req.tool_id, &tool_paths, &tool_paths.start_command)
        .ok_or_else(|| {
            format!(
                "工具「{}」没有可用的启动命令：请在工具详情里手动指定安装路径",
                tool_config.display_name
            )
        })?;
    eprintln!("[cli] 解析后的启动命令: {:?}", start_cmd);

    // 工作目录：`start /d ""` 与 `Set-Location ''` 都会失败，桌面应用尤其可能不带项目目录
    let work_dir = if req.project_path.trim().is_empty() {
        default_work_dir(&start_cmd)
    } else {
        req.project_path.clone()
    };

    // resume / continue / fork 参数
    let exe_prefix = format!("{} ", &tool_exe);
    let extra_args = if req.session_mode == "resume" {
        req.session_id
            .as_ref()
            .and_then(|sid| {
                tool_config.resume_cmd.as_ref().map(|s| {
                    s.replace("{session_id}", sid)
                        .strip_prefix(&exe_prefix)
                        .unwrap_or(&s.replace("{session_id}", sid))
                        .to_string()
                })
            })
            .unwrap_or_default()
    } else if req.session_mode == "fork" {
        // 分叉：由 CLI 自己复制一份会话再进入（如 claude --resume <sid> --fork-session），
        // 原会话保持不变。未声明 fork_cmd 的工具退化为普通 resume（前端本就不该给出该入口）。
        req.session_id
            .as_ref()
            .and_then(|sid| {
                tool_config
                    .fork_cmd
                    .as_ref()
                    .or(tool_config.resume_cmd.as_ref())
                    .map(|s| {
                        s.replace("{session_id}", sid)
                            .strip_prefix(&exe_prefix)
                            .unwrap_or(&s.replace("{session_id}", sid))
                            .to_string()
                    })
            })
            .unwrap_or_default()
    } else if req.session_mode == "continue" {
        tool_config
            .continue_cmd
            .as_ref()
            .map(|s| s.strip_prefix(&exe_prefix).unwrap_or(s).to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    // 所有模型 / baseUrl / apiKey 均已写入工具配置文件（configFile），不再通过 CLI 传递任何模型参数。
    // 仅保留 resume/continue 等会话参数（extra_args）与启动命令（start_command）。
    let tool_args = extra_args.clone();

    eprintln!(
        "\n[cli] session_mode={}, extra_args={:?}",
        req.session_mode, extra_args
    );
    eprintln!("[cli] start_command={:?}", start_cmd);
    eprintln!("[cli] tool_args={:?}", tool_args);
    eprintln!(
        "[cli] terminal_id={:?}, terminal_exe={:?}",
        req.terminal_id, terminal_exe
    );
    eprintln!("[cli] 注：模型/凭证均来自配置文件，未注入任何 CLI 模型参数");

    // powershell/pwsh 自身即为可见窗口，不能用 hidden_cmd（CREATE_NO_WINDOW），
    // 否则窗口被隐藏，表现为"启动成功但没反应"；cmd/wt/外部终端用 hidden_cmd，
    // 因为它们会再 spawn 出可见子窗口，隐藏父进程无影响。
    let is_powershell = terminal_exe.to_lowercase().contains("powershell")
        || terminal_exe.to_lowercase().contains("pwsh");
    // Store / MSIX 应用没有 exe 可跑，也不该为它开一个终端窗口 ——
    // 用 explorer.exe 打开 `shell:AppsFolder\…` 虚拟路径（抄 EchoBird
    // `process_manager.rs::start_shell_uri`，它实测过这条路径）。
    let is_launch_uri = start_cmd.starts_with(APPS_FOLDER_URI_PREFIX);
    // 桌面应用（非 Store）同样不该套终端包装：`cmd /k <exe>` 会留一个黑色控制台窗口，
    // GUI 应用也用不上它 —— EchoBird 的 GUI exe 是直接 spawn 的（Priority 3）。
    let is_desktop_tool =
        crate::commands::ai_registry::tool_kind_of(Some(&tool_paths.category)) == "desktop";
    let mut cmd = if is_launch_uri {
        hidden_cmd::hidden_cmd("explorer.exe")
    } else if is_desktop_tool {
        // 程序路径可能带空格：Command::new 直接吃整条路径（不经 shell），不能拆词
        Command::new(&start_cmd)
    } else if is_powershell {
        let mut c = Command::new(&terminal_exe);
        // CREATE_NEW_CONSOLE：强制为新控制台子进程分配独立窗口，
        // 避免 GUI 父进程下的 powershell 不弹出可见窗口（表现为"启动成功但没反应"）。
        #[cfg(windows)]
        c.creation_flags(0x00000010);
        c
    } else {
        hidden_cmd::hidden_cmd(&terminal_exe)
    };
    // GUI 应用与 Store 包 URI 不需要工作目录，而设一个**已不存在**的目录会让
    // `CreateProcess` 直接失败（os error 267 目录名称无效）——桌面端与项目目录本来
    // 就无关（界面上也不要求选），所以这两类干脆不设，别让一个无关目录卡住启动。
    if !is_launch_uri && !is_desktop_tool {
        cmd.current_dir(&work_dir);
    }

    // 装了但不在 PATH 里也要能启动：curl/scoop/choco 安装完的 `setx` 只对**之后**启动的
    // 进程生效，本进程 PATH 里没有该目录，裸命令在新终端里会「不是内部或外部命令」。
    // 检测到 exe 就把它所在目录前置进子进程 PATH（抄作业自 EchoBird f86fe961）。
    if let Some(exe_path) = crate::commands::ai::tool_paths::find_declared_exe(
        &req.tool_id,
        &tool_paths.paths,
        &start_cmd,
    ) {
        if let Some(dir) = exe_path.parent() {
            match prepend_to_path(dir) {
                Some(joined) => {
                    cmd.env("PATH", joined);
                    eprintln!("[cli] 已前置 PATH: {}", dir.display());
                }
                None => eprintln!("[cli] PATH 前置失败（join_paths）: {}", dir.display()),
            }
        }
    }

    let tool_arg_parts: Vec<&str> = extra_args
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .collect();

    // start_command 拆分为多个参数（如 "mimo ." → ["mimo", "."]）
    let start_cmd_parts: Vec<&str> = start_cmd.split_whitespace().collect();

    if is_launch_uri {
        // explorer.exe 直接吃这个虚拟路径，无需 `start`、也无需空标题参数
        cmd.arg(&start_cmd);
    } else if is_desktop_tool {
        // 直接 spawn：不附加任何终端参数（GUI 进程自己有窗口）
    } else if terminal_exe.to_lowercase().contains("cmd") {
        cmd.arg("/c")
            .arg("start")
            .arg("/d")
            .arg(&work_dir)
            .arg("cmd")
            .arg("/k");
        for p in &start_cmd_parts {
            cmd.arg(p);
        }
        for a in &tool_arg_parts {
            cmd.arg(a);
        }
    } else if terminal_exe.to_lowercase().contains("wt") {
        cmd.arg("-d").arg(&work_dir).arg("cmd").arg("/k");
        for p in &start_cmd_parts {
            cmd.arg(p);
        }
        for a in &tool_arg_parts {
            cmd.arg(a);
        }
    } else if is_ext_terminal(&req.terminal_id) {
        let launch_args = registry()
            .terminals()
            .terminals
            .get(&req.terminal_id)
            .and_then(|t| t.launch_args.as_ref())
            .map(|a| a.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|| vec!["-e", "cmd", "/k"]);
        for s in &launch_args {
            cmd.arg(*s);
        }
        for p in &start_cmd_parts {
            cmd.arg(p);
        }
        for a in &tool_arg_parts {
            cmd.arg(a);
        }
    } else {
        let escaped_path = work_dir.replace('\'', "''");
        // 安全过滤 PowerShell 命令注入字符（白名单：仅允许字母数字、空格、连字符、点、下划线、斜杠）
        // 注意：PowerShell 支持多种注入方式（子表达式、调用运算符等），白名单比黑名单更安全
        let sanitize_pwsh = |s: &str| -> String {
            s.chars()
                .filter(|c| {
                    c.is_alphanumeric()
                        || matches!(
                            c,
                            ' ' | '-' | '_' | '.' | '/' | '\\' | ':' | '(' | ')' | ','
                        )
                })
                .collect()
        };
        let safe_start = sanitize_pwsh(&start_cmd);
        let safe_args = sanitize_pwsh(&tool_args);
        let run_cmd = if safe_args.is_empty() {
            format!(
                "Set-Location -LiteralPath '{}'; {}",
                escaped_path, &safe_start
            )
        } else {
            format!(
                "Set-Location -LiteralPath '{}'; {} {}",
                escaped_path, &safe_start, &safe_args
            )
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
        let model = req
            .masquerade_model
            .clone()
            .filter(|c| !c.is_empty())
            .or_else(|| req.model_id.clone())
            .unwrap_or_default();
        let envs = build_env_vars(
            &tool_config,
            &effective_api_key,
            &effective_base_url,
            &model,
        );
        for (k, v) in &envs {
            eprintln!("[spawn] env {} = {}", k, mask_secret(v));
            cmd.env(k, v);
        }
    }

    // 注入模型自定义的「env 目标」启动参数（与工具 config.json 的 env.* 约定一致）
    for cp in &req.custom_params {
        if cp.target != "env" {
            continue;
        }
        let env_key = match &cp.env_key {
            Some(k) if !k.is_empty() => k.clone(),
            // 未显式给 env_key 时回退用 key 本身
            _ => cp.key.clone(),
        };
        let value = req
            .custom_param_values
            .get(&cp.key)
            .cloned()
            .or_else(|| cp.default_value.clone())
            .unwrap_or_default();
        if value.is_empty() {
            continue;
        }
        eprintln!(
            "[spawn] custom-env {} = {} (param: {})",
            env_key,
            mask_secret(&value),
            cp.label
        );
        cmd.env(env_key, value);
    }

    // 启动失败时把**实际执行的程序/命令**一起报出来：光一句「系统找不到指定的路径」根本
    // 定位不到问题（曾经据此排查过一次：真实原因是从注册表缓存里取到一条已删除的旧安装
    // 路径，而报错文案里没有任何路径可看）。
    cmd.spawn()
        .map_err(|e| format!("启动失败: {}｜实际执行的命令: {}", e, start_cmd))?;

    eprintln!("[spawn] ✓ 进程已启动");

    // 保存会话信息
    let mut sessions = load_sessions();
    let session_id = req
        .session_id
        .unwrap_or_else(|| chrono::Local::now().format("%Y%m%d-%H%M%S").to_string());
    // 在 move 前克隆后续所需字段
    let (lc_tool_id, lc_project_path, lc_model_id, lc_provider_id, lc_fallback_model_id) = (
        req.tool_id.clone(),
        req.project_path.clone(),
        req.model_id.clone(),
        req.provider_id.clone(),
        req.fallback_model_id.clone(),
    );
    sessions.sessions.retain(|s| {
        !(s.tool_id == req.tool_id
            && s.project_path == req.project_path
            && s.session_id.as_deref() == Some(&session_id))
    });
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
            cfg.providers
                .iter()
                .find(|p| &p.id == pid)
                .map(|p| p.name.clone())
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
