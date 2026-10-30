use super::*;

pub(super) fn resolve_custom_exe(cmd: &str, config: &crate::commands::config::Config) -> String {
    let mut parts = cmd.split_whitespace();
    if let Some(first_word) = parts.next() {
        let registry = crate::commands::project::registry::registry();
        for proj in &registry {
            let matches_proj = proj.id == first_word
                || proj.version_exe.as_ref().map(|s| s.as_str()) == Some(first_word);
            if matches_proj {
                if let Some(custom_path) = config.custom_install_paths.get(&proj.id) {
                    let path = std::path::Path::new(custom_path);
                    let exe = proj
                        .version_exe
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or(&proj.id);
                    let mut candidates = Vec::new();
                    candidates.push(path.join(exe));
                    candidates.push(path.join("bin").join(exe));
                    #[cfg(windows)]
                    {
                        let exe_lower = exe.to_lowercase();
                        if !exe_lower.ends_with(".exe")
                            && !exe_lower.ends_with(".cmd")
                            && !exe_lower.ends_with(".bat")
                        {
                            candidates.push(path.join(format!("{}.exe", exe)));
                            candidates.push(path.join(format!("{}.cmd", exe)));
                            candidates.push(path.join(format!("{}.bat", exe)));
                            candidates.push(path.join("bin").join(format!("{}.exe", exe)));
                            candidates.push(path.join("bin").join(format!("{}.cmd", exe)));
                            candidates.push(path.join("bin").join(format!("{}.bat", exe)));
                        }
                    }
                    for candidate in candidates {
                        if candidate.exists() {
                            let resolved_first = format!("\"{}\"", candidate.to_string_lossy());
                            let rest = &cmd[first_word.len()..];
                            return format!("{}{}", resolved_first, rest);
                        }
                    }
                }
            } else if let Some(pm) = proj
                .package_managers
                .iter()
                .find(|m| m.id == first_word || m.version_exe.as_deref() == Some(first_word))
            {
                // pm 级可执行（如 rust 项目下的 cargo）：补全到 links_dir/{proj.id} 下各 bin 目录。
                // 这样 pm 的 version_cmd（如 "cargo --version"）在未被注入全局 PATH 时也能解析到正确 exe。
                let exe = pm.version_exe.as_deref().unwrap_or(first_word);
                let link_dir = std::path::Path::new(&config.links_dir).join(&proj.id);
                let bin_dirs: Vec<String> = proj
                    .bin_dirs
                    .clone()
                    .unwrap_or_else(|| vec!["bin".to_string(), String::new()]);
                let exe_lower = exe.to_lowercase();
                let has_ext = exe_lower.ends_with(".exe")
                    || exe_lower.ends_with(".cmd")
                    || exe_lower.ends_with(".bat");
                let mut candidates: Vec<std::path::PathBuf> = Vec::new();
                for bin_dir in &bin_dirs {
                    let base = if bin_dir.is_empty() {
                        link_dir.clone()
                    } else {
                        link_dir.join(bin_dir)
                    };
                    if has_ext {
                        candidates.push(base.join(exe));
                    } else {
                        candidates.push(base.join(format!("{}.exe", exe)));
                        candidates.push(base.join(exe));
                    }
                }
                for candidate in candidates {
                    if candidate.exists() {
                        let resolved_first = format!("\"{}\"", candidate.to_string_lossy());
                        let rest = &cmd[first_word.len()..];
                        return format!("{}{}", resolved_first, rest);
                    }
                }
            }
        }
    }
    cmd.to_string()
}

/// 在系统中查找运行时可执行文件（PATH 或已检测到的安装路径）
pub(super) fn find_system_runtime(project_id: &str, exe_name: &str) -> Option<std::path::PathBuf> {
    let mut check_names = vec![exe_name.to_string()];
    #[cfg(windows)]
    {
        let exe_lower = exe_name.to_lowercase();
        if !exe_lower.ends_with(".exe")
            && !exe_lower.ends_with(".cmd")
            && !exe_lower.ends_with(".bat")
        {
            check_names.push(format!("{}.exe", exe_name));
        }
    }

    // 1. 在 PATH 中查找
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            for name in &check_names {
                let full = dir.join(name);
                if full.exists() {
                    return Some(full);
                }
            }
        }
    }

    // 2. 通过 find_rules 检测到的安装路径中查找
    if let Some(def) = crate::commands::project::registry::find_by_id(project_id) {
        if let Some(ref install_root) =
            crate::commands::project::scanner::detect_install_source(&def).1
        {
            let root = std::path::Path::new(install_root);
            for name in &check_names {
                let candidate = root.join(name);
                if candidate.exists() {
                    return Some(candidate);
                }
                // 也检查 Scripts 子目录（Windows 上常见）
                let candidate_scripts = root.join("Scripts").join(name);
                if candidate_scripts.exists() {
                    return Some(candidate_scripts);
                }
            }
        }
    }

    None
}

/// 检测命令字符串是否含 shell 注入元字符（命令分隔符、重定向、变量展开、反引号等）。
/// 允许引号与路径/参数符号（runtime 替换产生的 "exe" 路径），仅拦截可改变执行语义的危险字符。
pub(super) fn contains_shell_injection(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(c, '&' | '|' | ';' | '<' | '>' | '$' | '`' | '(' | ')'))
}

/// 终止命令进程树（Windows 用 taskkill /T 连子进程一起杀，避免孤儿进程残留）
pub(super) fn kill_cmd_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    }
}

/// 读取 SDK 模块设置的 GitHub API Token（空串表示未设置）
#[tauri::command]
pub fn project_get_github_token() -> Result<String, String> {
    Ok(crate::commands::config::load_config()
        .github_token
        .unwrap_or_default())
}

/// 设置/清除 SDK 模块的 GitHub API Token（存入 config.json；清空则回退环境变量）
#[tauri::command]
pub fn project_set_github_token(token: String) -> Result<(), String> {
    let trimmed = token.trim().to_string();
    crate::commands::config::mutate_config(|c| {
        c.github_token = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.clone())
        };
        Ok(())
    })
}

/// 执行 shell 命令并捕获输出（用于包管理器版本检测、镜像切换等）
#[tauri::command]
pub fn run_cmd_capture(cmd: String, project_id: Option<String>) -> Result<String, String> {
    if cmd == "install_pip" {
        return tauri::async_runtime::block_on(async {
            install_pip_internal(project_id.as_deref().unwrap_or("python")).await
        });
    }

    let config = crate::commands::config::load_config();
    let mut resolved_cmd = resolve_custom_exe(&cmd, &config);

    // PowerShell 命令直接调用 powershell.exe，避免 cmd /c 的引号嵌套问题
    let is_powershell = cfg!(windows) && {
        let lower = resolved_cmd.trim_start().to_lowercase();
        lower.starts_with("powershell ") || lower.starts_with("powershell.exe ")
    };

    // 运行时防护：非 PowerShell 的 cmd 命令拒绝含 shell 命令分隔符/注入元字符的命令（引号与路径符号允许，
    // 因为 run_via_runtime_args 替换会产生带引号的 exe 路径）。注册表层已做白名单，
    // 此处为第二道防线，确保任何来源的命令都不会执行注入。
    if !is_powershell && contains_shell_injection(&resolved_cmd) {
        return Err(format!(
            "拒绝执行含 shell 注入元字符的命令: {}",
            resolved_cmd
        ));
    }

    // Generic resolution: 仅对 AnyVersion 托管的项目，将 `pip xxx` 替换为 `"python.exe" -m pip xxx`
    // 非托管项目保持原命令（如 `pip --version`），直接尝试执行，失败时再 fallback
    let registry = crate::commands::project::registry::registry();
    for proj in &registry {
        for pm in &proj.package_managers {
            if let Some(ref run_args) = pm.run_via_runtime_args {
                let pm_prefix = format!("{} ", pm.id);
                if resolved_cmd == pm.id || resolved_cmd.starts_with(&pm_prefix) {
                    let link_dir = std::path::PathBuf::from(&config.links_dir).join(&proj.id);
                    let runtime_exe_name = proj.version_exe.as_deref().unwrap_or(&proj.id);
                    let link_runtime = if cfg!(windows) {
                        if !runtime_exe_name.ends_with(".exe") {
                            link_dir.join(format!("{}.exe", runtime_exe_name))
                        } else {
                            link_dir.join(runtime_exe_name)
                        }
                    } else {
                        link_dir.join(runtime_exe_name)
                    };
                    // 仅当 links_dir 中存在运行时时才替换（AnyVersion 托管场景）
                    if link_runtime.exists() {
                        let pm_run_str = format!(
                            "\"{}\" {}",
                            link_runtime.to_string_lossy(),
                            run_args.join(" ")
                        );
                        if resolved_cmd == pm.id {
                            resolved_cmd = pm_run_str;
                        } else {
                            resolved_cmd = resolved_cmd.replacen(&pm.id, &pm_run_str, 1);
                        }
                    }
                }
            }
        }
    }

    let mut command = if is_powershell {
        // 解析 "powershell -Command ..." 结构，提取脚本内容
        // 直接调用 powershell.exe，脚本作为 -Command 的参数传递
        let trimmed = resolved_cmd.trim_start();
        let after_exe = if trimmed.to_lowercase().starts_with("powershell.exe ") {
            trimmed["powershell.exe ".len()..].trim_start()
        } else {
            trimmed["powershell ".len()..].trim_start()
        };
        // 提取 -Command 后面的脚本
        let script = if after_exe.starts_with("-Command ") {
            after_exe["-Command ".len()..].trim()
        } else if after_exe.starts_with("-command ") {
            after_exe["-command ".len()..].trim()
        } else {
            after_exe
        };
        let mut c = crate::commands::hidden_cmd::hidden_cmd("powershell.exe");
        c.args(&["-Command", script]);
        c
    } else {
        let mut c = crate::commands::hidden_cmd::hidden_cmd("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.raw_arg(&format!("/c \"chcp 65001 >nul && {}\"", resolved_cmd));
        }
        #[cfg(not(windows))]
        {
            c.args(&["/c", &resolved_cmd]);
        }
        c
    };

    // 注入 NPM_CONFIG_PREFIX，确保 npm install -g 安装到正确的 link_dir
    // （当前进程环境变量可能未同步注册表，这里兜底保证子进程用对 prefix）
    if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
        command.env("NPM_CONFIG_PREFIX", &prefix);
    }

    // 注入托管 SDK 的可执行目录到子进程 PATH（排在本进程 PATH 之前）。
    //
    // 用户 PATH 写在注册表里，应用进程启动后不会自动同步 —— 托管后新装的
    // mvn/gradle/deno/dart 等会因「本进程 PATH 里没有 junction 目录」而在
    // 包管理器选项卡的检测命令（mvn -v 等）中找不到可执行文件，被误报「未安装」。
    // 注入**所有**托管 SDK（不只是目标项目）：mvn/gradle 这类检测命令还依赖
    // 链式工具（java），它们可能也是托管的。
    let mut extra: Vec<std::path::PathBuf> = Vec::new();
    let mut ordered: Vec<String> = Vec::new();
    if let Some(pid) = project_id.as_deref() {
        if config.managed_items.contains(pid) {
            // 目标项目排最前，保证检测命中的是它自己（而非外部的同名工具）
            ordered.push(pid.to_string());
        }
    }
    ordered.extend(
        config
            .managed_items
            .iter()
            .filter(|x| Some(x.as_str()) != project_id.as_deref())
            .cloned(),
    );
    for sid in &ordered {
        let link_dir = std::path::PathBuf::from(&config.links_dir).join(sid);
        for p in crate::commands::project::scanner::get_bin_paths(sid, &link_dir.to_string_lossy())
        {
            let pb = std::path::PathBuf::from(p);
            if pb.exists() && !extra.contains(&pb) {
                extra.push(pb);
            }
        }
        // 缓存型 bin（如 rust 的 caches\cargo\bin 里的 rustup shim）
        if let Some(def) = crate::commands::project::registry::find_by_id(sid) {
            for p in crate::commands::env::sub_dir_bin_paths(
                &def.env_vars,
                &crate::commands::config::get_cache_root(),
            ) {
                let pb = std::path::PathBuf::from(p);
                if pb.exists() && !extra.contains(&pb) {
                    extra.push(pb);
                }
            }
        }
    }
    if !extra.is_empty() {
        let mut new_path: Vec<std::path::PathBuf> = extra;
        new_path.extend(std::env::split_paths(
            &std::env::var("PATH").unwrap_or_default(),
        ));
        if let Ok(joined) = std::env::join_paths(&new_path) {
            command.env("PATH", joined);
        }
    }

    // JAVA_HOME 失效兜底：注册表里的 JAVA_HOME 可能指向已卸载/不存在的目录
    // （实测：指向已删除的 Unity 自带 OpenJDK），mvn.cmd/gradle.bat 会因它直接失败。
    // 此时从子进程环境移除，让它们回退到 PATH 上的 java（上面已注入托管 java 的 bin）。
    if let Ok(jh) = std::env::var("JAVA_HOME") {
        let valid = std::path::Path::new(&jh)
            .join("bin")
            .join("java.exe")
            .exists()
            || std::path::Path::new(&jh).join("java.exe").exists();
        if !jh.trim().is_empty() && !valid {
            command.env_remove("JAVA_HOME");
        }
    }

    // 仅记录命令与状态，绝不打印 stdout/stderr 明文（可能含 token 等敏感信息）。
    // 命令本身也可能内嵌 `://user:pass@` 凭据，落盘前脱敏。
    fn redact_creds(s: &str) -> String {
        // 匹配 http(s)://user:pass@ 形式（含 user 为空、pass 含常见合法字符）
        let re = regex::Regex::new(r"(://)([^/@\s:]*)(:[^/@\s]*)?(@)").unwrap();
        re.replace_all(s, "$1***$4").to_string()
    }
    eprintln!(
        "[run_cmd_capture] cmd={}, resolved_cmd={}",
        redact_creds(&cmd),
        redact_creds(&resolved_cmd)
    );

    // 为外部命令设置超时（默认 5 分钟），防止恶意/错误命令无限挂起。
    // run_cmd_capture 是同步函数：用带 piped stdio 的子进程 + 轮询 try_wait 实现同步超时；
    // 超时后 kill 进程树，避免孤儿进程与挂起线程泄漏。
    use std::io::Read;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动命令失败: {}", e))?;

    // 并发读取 stdout/stderr（等价于 command.output() 的内部行为），防止管道积满后子进程写阻塞
    let (tx, rx) = mpsc::channel();
    if let Some(mut o) = child.stdout.take() {
        let t = tx.clone();
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = o.read_to_end(&mut buf);
            let _ = t.send((b"o", buf));
        });
    }
    if let Some(mut e) = child.stderr.take() {
        let t = tx.clone();
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = e.read_to_end(&mut buf);
            let _ = t.send((b"e", buf));
        });
    }
    drop(tx);

    let deadline = Instant::now() + Duration::from_secs(300);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(format!("等待命令失败: {}", e)),
        }
        if Instant::now() >= deadline {
            kill_cmd_process_tree(child.id());
            let _ = child.wait();
            eprintln!(
                "[run_cmd_capture] 命令超时，已终止进程树: {}",
                redact_creds(&resolved_cmd)
            );
            return Err(format!(
                "命令执行超时（5 分钟）: {}",
                redact_creds(&resolved_cmd)
            ));
        }
        thread::sleep(Duration::from_millis(50));
    };

    // 进程已退出，读取线程排空管道后关闭发送端；收集两部分输出
    let mut out_stdout = Vec::new();
    let mut out_stderr = Vec::new();
    for (tag, buf) in rx.iter() {
        if tag == b"o" {
            out_stdout = buf;
        } else {
            out_stderr = buf;
        }
    }

    let stdout = String::from_utf8_lossy(&out_stdout)
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    let stderr = String::from_utf8_lossy(&out_stderr).trim().to_string();
    // 不打印 stdout/stderr 内容（避免泄露 token 等敏感数据），仅记录长度与状态。
    eprintln!(
        "[run_cmd_capture] status={}, stdout_len={}, stderr_len={}",
        status.success(),
        stdout.len(),
        stderr.len()
    );

    if status.success() {
        return Ok(stdout);
    }

    // 命令执行失败，依次尝试两种回退方式：
    // 1. 直接执行原命令（如 pip --version），适用于 pip 在 PATH 中但不在 AnyVersion 托管 python 中的场景
    // 2. 通过 find_system_runtime 找到运行时再执行（如 python -m pip --version）
    let original_cmd = resolve_custom_exe(&cmd, &config);
    if resolved_cmd != original_cmd {
        // 转换后的命令失败了，先尝试原命令直接执行
        let mut direct = crate::commands::hidden_cmd::hidden_cmd("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            direct.raw_arg(&format!("/c \"chcp 65001 >nul && {}\"", original_cmd));
        }
        if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
            direct.env("NPM_CONFIG_PREFIX", &prefix);
        }
        if let Ok(out) = direct.output() {
            if out.status.success() {
                let stdout = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim()
                    .to_string();
                eprintln!(
                    "[run_cmd_capture] fallback: 直接执行原命令成功, stdout=[{}]",
                    stdout
                );
                return Ok(stdout);
            }
        }
    }
    // 原命令也失败了，尝试通过 find_system_runtime 找到运行时再执行
    for proj in &registry {
        for pm in &proj.package_managers {
            if let Some(ref run_args) = pm.run_via_runtime_args {
                let pm_prefix = format!("{} ", pm.id);
                if cmd == pm.id || cmd.starts_with(&pm_prefix) {
                    let runtime_exe_name = proj.version_exe.as_deref().unwrap_or(&proj.id);
                    if let Some(runtime_path) = find_system_runtime(&proj.id, runtime_exe_name) {
                        // 避免重复使用已失败的同一路径
                        if runtime_path.to_string_lossy().to_lowercase()
                            == config.links_dir.to_lowercase()
                        {
                            continue;
                        }
                        let pm_run_str = format!(
                            "\"{}\" {}",
                            runtime_path.to_string_lossy(),
                            run_args.join(" ")
                        );
                        let fallback_cmd = if cmd == pm.id {
                            pm_run_str
                        } else {
                            cmd.replacen(&pm.id, &pm_run_str, 1)
                        };
                        let mut fallback = crate::commands::hidden_cmd::hidden_cmd("cmd");
                        #[cfg(windows)]
                        {
                            use std::os::windows::process::CommandExt;
                            fallback
                                .raw_arg(&format!("/c \"chcp 65001 >nul && {}\"", fallback_cmd));
                        }
                        if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
                            fallback.env("NPM_CONFIG_PREFIX", &prefix);
                        }
                        if let Ok(out) = fallback.output() {
                            if out.status.success() {
                                let stdout = String::from_utf8_lossy(&out.stdout)
                                    .trim()
                                    .trim_matches('"')
                                    .trim_matches('\'')
                                    .trim()
                                    .to_string();
                                eprintln!("[run_cmd_capture] fallback: find_system_runtime 成功, stdout=[{}]", stdout);
                                return Ok(stdout);
                            }
                        }
                    }
                }
            }
        }
    }

    if !stderr.is_empty() {
        Err(stderr)
    } else {
        Err(format!(
            "命令执行失败 (exit code: {})",
            status.code().unwrap_or(-1)
        ))
    }
}
