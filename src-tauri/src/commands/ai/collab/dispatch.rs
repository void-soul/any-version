//! dispatch responsibility.
use super::*;

/// 派发到工具：流式读取 stdout，逐段 emit；带会话绑定 / 取消 / 超时。
/// 代理为房间+工具级常驻：首次派发时启动代理 + 写配置文件，后续消息复用同一代理。
pub(super) async fn dispatch_to_tool(
    app: &tauri::AppHandle,
    room_id: String,
    tool_id: String,
    project_path: String,
    prompt: String,
    placeholder_id: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    options: CollabDispatchOptions,
) {
    eprintln!("[collab] ═══ 派发开始 ═══ tool={}, room={}, placeholder={}, prompt_len={}, model={:?}, provider={:?}",
        tool_id, room_id, placeholder_id, prompt.len(), model_id, provider_id);
    // 全局并行上限：拿不到槽位就在这里排队（permit 是 RAII，函数任何路径返回都会归还）。
    // 放在派发入口而不是某一处调用点，是因为 `dispatch_to_tool` 有两个调用方
    // （turn 调度器与压缩请求），只堵一处等于没堵。
    let _slot = match agent_slots().acquire().await {
        Ok(permit) => permit,
        Err(_) => {
            eprintln!("[collab] ✗ 并发槽位已关闭，放弃派发: {}", tool_id);
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "error",
                "⚠ 协同派发服务已关闭".to_string(),
                None,
                None,
                None,
                None,
            );
            return;
        }
    };
    let (_tool_config, tool_paths) = match registry().get_tool(&tool_id) {
        Some((c, p)) => (c.clone(), p.clone()),
        None => {
            eprintln!("[collab] ✗ 未知工具: {}", tool_id);
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "error",
                "⚠ 未知工具".to_string(),
                None,
                None,
                None,
                None,
            );
            return;
        }
    };
    // 协同只认 CLI 工具：桌面端是 GUI 程序，spawn 起来也接不到这条任务，
    // 与其启动后失败，不如在这里直接说清楚（前端 @ 列表同样只列 CLI）
    let kind = crate::commands::ai_registry::tool_kind_of(Some(&tool_paths.category));
    if kind != "cli" {
        eprintln!(
            "[collab] ✗ 该工具不是 CLI（kind={}），不支持协同: {}",
            kind, tool_id
        );
        finalize_message(
            app,
            &room_id,
            &placeholder_id,
            "error",
            format!(
                "⚠ {} 是桌面端工具，不支持协同派发（只有 CLI 工具可以）",
                tool_id
            ),
            None,
            None,
            None,
            None,
        );
        return;
    }
    let tool_config = _tool_config;

    // 会话绑定：已有 id 则续聊（不再开新会话）
    let session_key = format!("{}::{}", room_id, tool_id);
    // 单次 load_store 同时取出会话 id 与上下文快照，避免重复全量解析 collab.json
    let store = load_store();
    let existing_sid = store.tool_sessions.get(&session_key).cloned();
    let has_session = existing_sid.is_some();
    let snapshot = if !has_session {
        store.context_snapshots.get(&session_key).cloned()
    } else {
        None
    };
    eprintln!(
        "[collab] 会话状态: has_session={}, existing_sid={:?}",
        has_session, existing_sid
    );

    // ─── 上下文快照注入：新会话首次派发时，将压缩摘要注入提示词 ───
    let prompt = if let Some(s) = snapshot {
        format!(
            "--- 上下文快照（来自上一会话的压缩摘要）---\n\n{}\n\n--- 以上为上下文快照，请基于此继续工作 ---\n\n--- 当前任务 ---\n\n{}",
            s.summary, prompt
        )
    } else {
        prompt
    };

    // ─── 协同协议注入：告诉 agent 本房间可用工具与 @委派 规则（实现 agent 主动互聊，对齐 open-tag system-prompt 注入） ───
    // 仅在「新会话首次派发」注入：协议在首轮已进入上下文；续聊（has_session）时重复注入只会污染对话、浪费 token。
    let prompt = if has_session {
        prompt
    } else {
        format!("{}\n\n{}", build_collab_protocol_prompt(&room_id), prompt)
    };

    let tmp_dir = get_data_dir().join("collab_tmp");
    let _ = fs::create_dir_all(&tmp_dir);
    let prompt_path = tmp_dir.join(format!(
        "{}_{}.txt",
        room_id.replace(|c: char| !c.is_alphanumeric(), ""),
        new_id()
    ));
    if let Err(e) = fs::write(&prompt_path, &prompt) {
        finalize_message(
            app,
            &room_id,
            &placeholder_id,
            "error",
            format!("⚠ 写入提示词失败: {}", e),
            None,
            None,
            None,
            None,
        );
        return;
    }

    // ─── 获取或创建房间级常驻代理 + 配置文件（首次启动，后续复用） ───
    let (_port, base_url, api_key) = ensure_room_proxy(
        &room_id,
        &tool_id,
        &tool_config,
        provider_id.as_deref(),
        model_id.as_deref(),
        &options,
        app,
    )
    .await;

    // 注册当前 msg_id 到全局表（供常驻代理 emit 事件和缓存文本时查询）
    crate::proxy::server::set_collab_msg_id(&room_id, &tool_id, placeholder_id.clone());

    // 代理未启动（port==0）时明确报错并中止：否则 opencode 会经 env(baseUrl→OPENAI_BASE_URL)
    // 直连真实上游 https 端点，报出误导性的 "unknown certificate verification error"。
    if _port == 0 {
        eprintln!("[collab] ✗ 代理未启动（port==0），中止派发，避免绕过代理直连上游");
        finalize_message(
            app, &room_id, &placeholder_id, "error",
            "⚠ 代理未启动，无法派发。请检查「设置」中该供应商的密钥与协议 URL 是否已正确配置（OpenCode 需 openai 协议出站）。".to_string(),
            None, None, None, None,
        );
        return;
    }

    // 加载 provider 用于 env 注入
    let config = crate::commands::ai::config::load_ai_config();
    let provider = provider_id
        .as_deref()
        .and_then(|pid| config.providers.iter().find(|p| p.id == pid));

    let runner = tool_config.runner.clone().unwrap_or_default();
    let tmpl = if has_session {
        tool_config
            .dispatch_resume_cmd
            .clone()
            .or_else(|| tool_config.dispatch_continue_cmd.clone())
    } else {
        tool_config.dispatch_cmd.clone()
    };
    let tmpl = match tmpl {
        Some(t) => t,
        None => {
            eprintln!(
                "[collab] ✗ 未配置派发命令: tool={}, has_session={}",
                tool_config.display_name, has_session
            );
            let _ = fs::remove_file(&prompt_path);
            crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "error",
                format!("⚠ 工具 {} 未配置派发命令", tool_config.display_name),
                None,
                None,
                None,
                None,
            );
            return;
        }
    };
    eprintln!(
        "[collab] 模板选择: has_session={}, tmpl_len={}",
        has_session,
        tmpl.len()
    );

    let prompt_str = prompt_path.to_string_lossy().replace('\\', "/");
    let quoted = format!("\"{}\"", prompt_str);
    let prompt_mode = tool_config.prompt_mode.clone().unwrap_or_default();
    let use_stdin = prompt_mode == "stdin";

    // argv 直启：把模板拆成参数数组，{prompt}/{prompt_file}/{session_id} 作为独立参数，
    // 不再整体经 shell 解释，规避命令注入（参考 skills 仓库：never spawn through a shell）
    // 必须在 tmpl 被 move 进 cmd 之前完成分词。
    // 模型名：伪装优先，否则取所选模型
    let model_for_placeholders = options
        .masquerade_model
        .clone()
        .filter(|c| !c.is_empty())
        .or_else(|| model_id.clone())
        .unwrap_or_default();

    let argv: Vec<String> = {
        let mut out = Vec::new();
        for tok in tokenize_template(&tmpl) {
            match tok.as_str() {
                "{prompt}" => {
                    if !use_stdin {
                        out.push(prompt.clone());
                    }
                }
                "{prompt_file}" => {
                    out.push(prompt_str.clone());
                }
                "{session_id}" => {
                    out.push(existing_sid.clone().unwrap_or_default());
                }
                "{session_file}" => {
                    // reasonix 等工具：resume 需要会话文件路径而非 id（空则随 flag 一起被移除）
                    let f = existing_sid
                        .as_deref()
                        .map(|sid| reasonix_session_file(sid, &project_path))
                        .unwrap_or_default();
                    out.push(f);
                }
                "{model}" => {
                    out.push(model_for_placeholders.clone());
                }
                other => out.push(other.to_string()),
            }
        }
        out
    };

    // 抄 open-tag spawn：占位符解析为空（如未选模型 / 无 session）时，移除其前的 flag
    //（-m/--model/-s 等），避免残留 `--model ` 把后续 token 误当成模型名（opencode/cursor
    // 此类 node 系 CLI 会因此无法正常启动）。
    let argv: Vec<String> = {
        let mut f: Vec<String> = Vec::with_capacity(argv.len());
        for tok in argv.iter() {
            if tok.is_empty() {
                if let Some(prev) = f.last() {
                    if prev.starts_with('-') {
                        f.pop();
                        continue;
                    }
                }
                continue;
            }
            f.push(tok.clone());
        }
        f
    };

    // shell 命令串（仅用于 Windows .cmd 垫片回退与日志；提示词经 cmd 转义）
    let mut cmd = tmpl;
    cmd = cmd.replace(
        "{session_id}",
        &escape_cmd_arg(existing_sid.as_deref().unwrap_or("")),
    );
    if cmd.contains("{session_file}") {
        let sf = existing_sid
            .as_deref()
            .map(|sid| reasonix_session_file(sid, &project_path))
            .unwrap_or_default();
        cmd = cmd.replace("{session_file}", &escape_cmd_arg(&sf));
    }
    cmd = cmd.replace("{model}", &escape_cmd_arg(&model_for_placeholders));
    if cmd.contains("{prompt_file}") {
        cmd = cmd.replace("{prompt_file}", &quoted);
    }
    if cmd.contains("{prompt}") {
        cmd = cmd.replace("{prompt}", &escape_cmd_arg(&prompt));
    }
    // ─── 环境变量注入：从 config_file.write 中的 env.* 键注入（与正常启动一致） ───
    let mut envs = if let Some(ref p) = provider {
        crate::commands::ai::launch::build_env_vars(
            &tool_config,
            &p.api_key,
            &base_url,
            &model_for_placeholders,
        )
    } else {
        HashMap::new()
    };
    eprintln!(
        "[collab] 命令构建: use_stdin={}, prompt_mode={:?}, env_vars={}",
        use_stdin,
        prompt_mode,
        envs.len()
    );
    eprintln!("[collab] 环境变量: {:?}", envs.keys().collect::<Vec<_>>());
    // 兜底：若工具无 config_file 定义，仍按协议注入基础 env vars
    if envs.is_empty() {
        match tool_config.native_protocol().as_str() {
            "anthropic" => {
                envs.insert("ANTHROPIC_BASE_URL".to_string(), base_url.clone());
                envs.insert("ANTHROPIC_API_KEY".to_string(), api_key.clone());
                // 与 build_env_vars 保持一致：伪装优先，否则所选取模型，避免两条路径注入的 model 名不同
                if !model_for_placeholders.is_empty() {
                    envs.insert(
                        "ANTHROPIC_MODEL".to_string(),
                        model_for_placeholders.clone(),
                    );
                }
            }
            "google" => {
                envs.insert("GOOGLE_API_BASE_URL".to_string(), base_url.clone());
                envs.insert("GOOGLE_API_KEY".to_string(), api_key.clone());
            }
            _ => {
                envs.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
                envs.insert("OPENAI_API_KEY".to_string(), api_key.clone());
            }
        }
    }

    eprintln!(
        "[collab] ▶ 派发 {} → cmd: {}",
        tool_config.display_name, cmd
    );
    eprintln!("[collab] ▶ argv: {:?}", argv);
    eprintln!("[collab] ▶ 工作目录: {}", project_path);
    // 优先直接 spawn 工具二进制（argv 数组，不经 shell，规避注入）。
    // 仅当 Windows 上工具是 .cmd/.bat 垫片（CreateProcess 无法直接执行）时才退回 cmd /c。
    let program_raw = argv.first().cloned().unwrap_or_default();
    let program = resolve_program(&program_raw);
    let needs_shell = is_windows_shell_shim(&program);
    eprintln!(
        "[collab] ▶ 解析程序: raw={:?} → resolved={:?} shim={}",
        program_raw, program, needs_shell
    );
    // 用解析后的完整路径替换 argv[0]，后续统一以 argv 数组派发（对齐 open-tag / cross-spawn：
    // 直接 spawn 二进制，仅 .cmd/.bat 垫片退回 cmd /d /s /c）。
    let mut argv_resolved = argv.clone();
    if !argv_resolved.is_empty() {
        argv_resolved[0] = program.clone();
    }
    let mut command = if needs_shell {
        // Windows .cmd/.bat 垫片无法被 CreateProcess 直接执行，必须退回 cmd。
        // 对齐 cross-spawn：构造 `cmd /d /s /c "<windows_quote 逐元素转义并 join 的 argv>"`。
        //   /s 是关键——它修正默认 /c 的“删除首尾引号”规则，使含空格的程序路径/参数被正确识别；
        //   /d 禁止自动执行 AutoRun 注册表项。各 argv 元素用 windows_quote 加引号（含空格/引号时），
        //   整体再用一对引号括住作为 /c 的单个参数。raw_arg 原样透传，避免 Rust 重新加引号破坏 /s 解析。
        // 这是 cross-spawn 在 Windows 上经实战验证的成熟写法（opencode/gemini 等 .cmd 垫片均走此路径），
        // 取代此前易踩坑的 `cmd /c` 字符串拼接与 @ 前缀 trick。
        #[cfg(windows)]
        {
            let full = argv_resolved
                .iter()
                .map(|a| windows_quote(a))
                .collect::<Vec<_>>()
                .join(" ");
            let quoted = format!("\"{}\"", full);
            let mut c = Command::new("cmd");
            c.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
            c.raw_arg("/d")
                .raw_arg("/s")
                .raw_arg("/c")
                .raw_arg(quoted.as_str());
            c
        }
        #[cfg(not(windows))]
        {
            // 非 Windows 的 shell 脚本（.sh 等）回退：sh -c "<join 后的字符串>"
            let full = argv_resolved.join(" ");
            let mut c = Command::new("sh");
            c.arg("-c").arg(full);
            c
        }
    } else {
        // 直接 spawn 工具二进制（argv 数组，不经 shell，规避注入）——对齐 cross-spawn 对可执行文件的路径
        let mut c = Command::new(&program);
        c.args(&argv_resolved[1..]);
        #[cfg(windows)]
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
        c
    };
    command
        .current_dir(&project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // 抄 open-tag spawn：剥离从父进程继承的 NODE_OPTIONS。若其中含代理 require hook，
    // 会导致 opencode/cursor 等 node 系 CLI 拒绝启动（open-tag 在子进程 env 中主动 strip）。
    // 我们从不依赖 NODE_OPTIONS，移除只消除该隐患。
    command.env_remove("NODE_OPTIONS");
    // ─── 协同总线 env：让被派发的 agent 能主动同步委派其它工具（C 方案，对齐 open-tag 的 agent CLI） ───
    // agent 在推理中可运行 curl 调 $COLLAB_BUS_URL/collab/agent/send 并阻塞拿到对方回复。
    command.env("COLLAB_BUS_URL", format!("http://127.0.0.1:{}", _port));
    command.env("COLLAB_BUS_TOKEN", room_bus_token(&room_id));
    command.env("COLLAB_ROOM_ID", &room_id);
    command.env("COLLAB_TOOL_ID", &tool_id);
    // 提示词传入方式：stdin 模式把临时文件作为子进程 stdin；否则用 null（避免工具检测到管道 stdin 后阻塞读取）
    let mut child_stdin_opt: Option<ChildStdin> = None;
    if use_stdin {
        match fs::File::open(&prompt_path) {
            Ok(f) => {
                command.stdin(Stdio::from(f));
            }
            Err(e) => {
                let _ = fs::remove_file(&prompt_path);
                finalize_message(
                    app,
                    &room_id,
                    &placeholder_id,
                    "error",
                    format!("⚠ 打开提示词失败: {}", e),
                    None,
                    None,
                    None,
                    None,
                );
                return;
            }
        }
    } else {
        command.stdin(Stdio::null());
    }
    for (k, v) in &envs {
        command.env(k, v);
    }

    let mut child = match command.spawn() {
        Ok(c) => {
            eprintln!("[collab] ✓ 子进程已启动: pid={}", c.id());
            c
        }
        Err(e) => {
            eprintln!("[collab] ✗ 启动失败: {}", e);
            let _ = fs::remove_file(&prompt_path);
            crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "error",
                format!("⚠ 启动工具失败: {}", e),
                None,
                None,
                None,
                None,
            );
            return;
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = fs::remove_file(&prompt_path);
            crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "error",
                "⚠ 无法读取工具输出".to_string(),
                None,
                None,
                None,
                None,
            );
            return;
        }
    };
    // 取 stdin 句柄（非 stdin 模式时用于交互式应答）
    if !use_stdin {
        child_stdin_opt = child.stdin.take();
    }
    // 取 stderr 并在后台线程中持续读取，防止管道缓冲区满后子进程阻塞
    let stderr = child.stderr.take();
    let child_arc = Arc::new(Mutex::new(Some(child)));
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let map = DISPATCH_STATE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut g = map.lock();
        g.insert(
            placeholder_id.clone(),
            DispatchCtrl {
                child: child_arc.clone(),
                cancel: cancel.clone(),
            },
        );
    }

    // stderr 后台读取线程：收集内容到 stderr_buf，供无输出时诊断；同时检测交互式 prompt
    // 使用 timeout 读取，防止子进程挂起导致线程泄漏
    let stderr_cancel = cancel.clone();
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf_clone = stderr_buf.clone();
    let stderr_room = room_id.clone();
    let stderr_msg = placeholder_id.clone();
    let stderr_app = app.clone();
    let stderr_handle = if let Some(stderr) = stderr {
        Some(std::thread::spawn(move || {
            use std::io::Read;
            let mut reader = stderr;
            let mut buf = [0u8; 1024];
            loop {
                if stderr_cancel.load(Ordering::SeqCst) {
                    break;
                }
                // 使用非阻塞读取 + sleep 轮询，使 cancel 标志能及时生效
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        for line in text.lines() {
                            let line = line.trim_end();
                            if !line.is_empty() {
                                eprintln!("[collab] stderr: {}", line);
                                if let Some((question, options)) = detect_prompt(line) {
                                    emit_prompt(
                                        &stderr_app,
                                        &stderr_room,
                                        &stderr_msg,
                                        &question,
                                        options,
                                    );
                                }
                                let mut g = stderr_buf_clone.lock();
                                if g.len() < 8192 {
                                    g.push_str(line);
                                    g.push('\n');
                                }
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // 无数据可读，短暂休眠后重试
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        }))
    } else {
        None
    };

    // 使用独立线程读取 stdout，主循环通过 channel + recv_timeout 实现非阻塞语义，
    // 使取消/超时检测在子进程长时间无输出时仍能生效
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let stdout_cancel = cancel.clone();
    let stdout_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = stdout;
        let mut buf = [0u8; 1024];
        loop {
            if stdout_cancel.load(Ordering::SeqCst) {
                let _ = tx.send(None);
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(None);
                    break;
                }
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    for line in text.lines() {
                        let _ = tx.send(Some(line.to_string()));
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => {
                    let _ = tx.send(None);
                    break;
                }
            }
        }
    });

    // 注册 PromptCtrl（若有 stdin 句柄）
    let prompt_stdin = Arc::new(Mutex::new(child_stdin_opt));
    let prompt_pending: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
    if !use_stdin {
        let map = PROMPT_STATE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut g = map.lock();
        g.insert(
            placeholder_id.clone(),
            PromptCtrl {
                stdin: prompt_stdin.clone(),
                pending: prompt_pending.clone(),
            },
        );
    }

    let start = Instant::now();
    let timeout = Duration::from_secs(DISPATCH_TIMEOUT_SECS);
    let poll_interval = Duration::from_millis(200);
    let mut accumulated = String::new();
    let mut captured_sid: Option<String> = None;
    let mut captured_usage: Option<TokenUsage> = None;
    let dispatch_start = std::time::Instant::now();
    let mut err_msg: Option<String> = None;
    let mut raw_lines: Vec<String> = Vec::new(); // 用于 JSON 解析失败时回退显示原始输出
    let adapter = runner_adapter(&runner);
    let is_json_runner = adapter.is_json();
    eprintln!(
        "[collab] ▶ 读取循环开始: runner={}, is_json={}, timeout={}s",
        runner, is_json_runner, DISPATCH_TIMEOUT_SECS
    );

    loop {
        if cancel.load(Ordering::SeqCst) {
            err_msg = Some("⚠ 已取消".to_string());
            kill_tree(&child_arc);
            break;
        }
        if start.elapsed() > timeout {
            err_msg = Some(format!("⚠ 派发超时（>{}秒）", DISPATCH_TIMEOUT_SECS));
            kill_tree(&child_arc);
            break;
        }
        // 检查 prompt 超时自动应答
        if let Some(ref pend) = *prompt_pending.lock() {
            if pend.1.elapsed() > Duration::from_secs(PROMPT_AUTO_RESPOND_SECS) {
                eprintln!("[collab] prompt 超时自动应答: y");
                write_stdin_response(&placeholder_id, "y");
            }
        }
        // 非阻塞接收：timeout 轮询，使取消/超时检测生效
        match rx.recv_timeout(poll_interval) {
            Ok(Some(line)) => {
                let l = line.trim_end().to_string();
                if l.is_empty() {
                    continue;
                }
                // 检测工具错误事件（如 opencode 的 {"type":"error",...}）：直接上报，避免被误判为"无输出"
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
                        let msg = v
                            .get("error")
                            .and_then(|e| {
                                e.get("data")
                                    .and_then(|d| d.get("message"))
                                    .or_else(|| e.get("message"))
                            })
                            .and_then(|m| m.as_str())
                            .unwrap_or("未知错误")
                            .to_string();
                        err_msg = Some(format!("⚠ 工具返回错误: {}", msg));
                        break;
                    }
                }
                if is_json_runner {
                    // 保留原始行用于回退
                    if raw_lines.len() < 50 {
                        raw_lines.push(l.clone());
                    }
                    // 检测 prompt（JSON 解析失败时也检查）
                    if let Some((question, options)) = detect_prompt(&l) {
                        emit_prompt(app, &room_id, &placeholder_id, &question, options);
                    }
                    if let Some((ev, sid)) = adapter.parse(&l) {
                        if let Some(s) = sid {
                            captured_sid = Some(s);
                        }
                        match ev {
                            StreamEvent::Delta(d) => {
                                eprintln!("[collab] ▸ Delta: len={}", d.len());
                                accumulated.push_str(&d);
                                let _ = app.emit(
                                    "collab:delta",
                                    CollabDeltaPayload {
                                        room_id: room_id.clone(),
                                        msg_id: placeholder_id.clone(),
                                        delta: d,
                                    },
                                );
                            }
                            StreamEvent::Activity(a) => {
                                eprintln!("[collab] ▸ Activity: {}", a);
                                let _ = app.emit(
                                    "collab:activity",
                                    CollabActivityPayload {
                                        room_id: room_id.clone(),
                                        msg_id: placeholder_id.clone(),
                                        activity: a,
                                    },
                                );
                            }
                            StreamEvent::Result(t, u) => {
                                eprintln!(
                                    "[collab] ▸ Result: len={}, non_empty={}",
                                    t.len(),
                                    !t.is_empty()
                                );
                                // 仅当收尾文本非空才覆盖（避免空 result 抹掉已流式内容）
                                if !t.is_empty() {
                                    accumulated = t;
                                }
                                // 捕获 token 消耗（claude 的 result 事件提供；其他工具为 None）
                                if let Some(us) = u {
                                    captured_usage = Some(us);
                                }
                            }
                            StreamEvent::Ignore => {}
                        }
                    } else {
                        // JSON 解析失败，作为原始文本兜底
                        eprintln!(
                            "[collab] ⚠ JSON 解析失败 (runner={}): {}",
                            runner,
                            l.chars().take(200).collect::<String>()
                        );
                    }
                } else {
                    // 非 JSON runner：检测 prompt
                    if let Some((question, options)) = detect_prompt(&l) {
                        emit_prompt(app, &room_id, &placeholder_id, &question, options);
                    }
                    accumulated.push_str(&l);
                    accumulated.push('\n');
                }
            }
            Ok(None) => break, // EOF
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if cancel.load(Ordering::SeqCst) && err_msg.is_none() {
        err_msg = Some("⚠ 已取消".to_string());
    }

    // 等待 stdout 读取线程结束
    let _ = stdout_reader.join();
    // 等待 stderr 读取线程结束
    if let Some(h) = stderr_handle {
        let _ = h.join();
    }

    let _ = fs::remove_file(&prompt_path);
    eprintln!(
        "[collab] 派发循环结束: accumulated_len={}, captured_sid={:?}, err={:?}, raw_lines={}",
        accumulated.len(),
        captured_sid,
        err_msg.is_some(),
        raw_lines.len()
    );
    // 代理为房间级常驻，不在每条消息结束时停止
    if let Some(map) = DISPATCH_STATE.get() {
        map.lock().remove(&placeholder_id);
    }
    // 清理 PromptCtrl
    if let Some(map) = PROMPT_STATE.get() {
        map.lock().remove(&placeholder_id);
    }

    // 获取 stderr 内容
    let stderr_content = stderr_buf.lock().clone();
    if !stderr_content.is_empty() {
        eprintln!("[collab] stderr 总量: {} 字节", stderr_content.len());
    }

    let had_error = err_msg.is_some();
    let mut compact_result: Option<String> = None;

    match err_msg {
        Some(e) => {
            eprintln!(
                "[collab] ═══ 派发结束(错误) ═══ placeholder={}, err={}",
                placeholder_id, e
            );
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "error",
                e,
                None,
                None,
                None,
                None,
            );
        }
        None => {
            let mut content = accumulated.trim().to_string();
            eprintln!(
                "[collab] 派发完成: content_len={}, is_json={}, raw_lines={}, stderr_len={}",
                content.len(),
                is_json_runner,
                raw_lines.len(),
                stderr_content.len()
            );
            // ── 代理回退：stdout 无内容时，使用代理缓存的响应文本 ──
            if content.is_empty() || content == "(无输出)" {
                if let Some(proxy_text) = crate::proxy::server::take_proxy_text(&placeholder_id) {
                    eprintln!(
                        "[collab] ✓ 代理回退: 使用代理响应文本 (len={})",
                        proxy_text.len()
                    );
                    if !proxy_text.is_empty() {
                        content = proxy_text;
                    }
                }
            }
            // JSON runner 但无解析结果 → 回退显示原始 stdout + stderr
            if content.is_empty() && is_json_runner && !raw_lines.is_empty() {
                eprintln!(
                    "[collab] ⚠ JSON runner 无解析结果，回退原始输出 ({} 行)",
                    raw_lines.len()
                );
                content = format!("⚠ 未解析到有效内容，原始输出：\n{}", raw_lines.join("\n"));
                if !stderr_content.is_empty() {
                    content.push_str(&format!("\n\n--- stderr ---\n{}", stderr_content.trim()));
                }
            } else if content.is_empty() {
                // 非 JSON runner 也无输出 → 显示 stderr
                if !stderr_content.is_empty() {
                    content = format!("⚠ 无 stdout 输出，stderr 内容：\n{}", stderr_content.trim());
                } else {
                    content = "(无输出)".to_string();
                }
            }
            // 合并 session 绑定写入 finalize_message，避免额外的 load_store + save_store 循环
            let session_update = captured_sid
                .as_ref()
                .map(|sid| (session_key.clone(), sid.clone()));
            eprintln!("[collab] ═══ 派发结束(成功) ═══ placeholder={}, content_len={}, sid={:?}, session_update={}",
                placeholder_id, content.len(), captured_sid, session_update.is_some());
            compact_result = Some(content.clone());
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "done",
                content.clone(),
                captured_sid.clone(),
                Some(dispatch_start.elapsed().as_millis() as u64),
                captured_usage.clone(),
                session_update,
            );
            // B 层：工具回复后解析 @ 提及，触发因果唤醒（多 agent 协同）
            {
                let s = load_store();
                if let Some(turn) = s
                    .turns
                    .iter()
                    .find(|t| t.dispatch_message_id.as_deref() == Some(&placeholder_id))
                {
                    let root = turn.causal_root_id.clone();
                    let depth = turn.causal_depth;
                    drop(s);
                    wake_tools_from_reply(
                        &room_id,
                        &tool_id,
                        &content,
                        &root,
                        depth,
                        &placeholder_id,
                    );
                }
            }
        }
    }

    // ─── 压缩回调：通知 collab_compact_session 命令 ───
    if let Some(map) = COMPACT_CALLBACKS.get() {
        if let Some(sender) = map.lock().remove(&placeholder_id) {
            let _ = sender.send(compact_result.unwrap_or_default());
        }
    }
    // ─── 快照清理：新会话成功创建后清除已注入的快照（避免重复注入） ───
    if !had_error && captured_sid.is_some() {
        let mut store = load_store();
        if store.context_snapshots.remove(&session_key).is_some() {
            let _ = save_store(&store);
        }
    }
    // 清理全局 msg_id 注册
    crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
    // 清理代理文本缓存（防止内存泄漏：工具有 stdout 输出时代理文本不会被消费）
    let _ = crate::proxy::server::take_proxy_text(&placeholder_id);
}
