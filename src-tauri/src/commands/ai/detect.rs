use serde::Serialize;
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use std::sync::OnceLock;
use crate::commands::ai_registry::{registry, AiToolDefDto, ToolConfig, PathConfig};
use super::tools::get_tool_busy;
use crate::commands::tool_version::is_newer;
use crate::commands::hidden_cmd;
use crate::commands::utils::{find_in_path, get_http_client};

use super::config::DetectedAiTool;

/// 全局缓存的 semver 提取正则表达式（避免每次调用都重新编译）
static SEMVER_RE: OnceLock<regex::Regex> = OnceLock::new();

/// 官网地址三级解析（抄作业自 EchoBird）：homepage > website > github，
/// 过滤空白值；全部缺省时返回空字符串。
/// 用途：未安装工具在列表中展示官网入口，避免 website 指向仓库时把用户带到 GitHub。
fn resolve_tool_website(
    homepage: Option<&str>,
    website: Option<&str>,
    github: Option<&str>,
) -> String {
    [homepage, website, github]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|url| !url.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn detect_single_tool(config: &ToolConfig, paths: &PathConfig) -> DetectedAiTool {
    eprintln!("[detect] ========== {} ({}) ==========", config.display_name, config.id);

    // 先看磁盘上是否真的有可执行文件（`%APPDATA%/npm/claude.cmd` 等声明路径）。
    // 两个用途：① 版本探测都失败时的兜底判据；② 回填给前端/启动逻辑做 PATH 前置。
    let declared_exe = super::tool_paths::find_declared_exe(
        &config.id,
        &paths.paths,
        &paths.start_command,
    );
    if let Some(exe) = &declared_exe {
        eprintln!("[detect]   磁盘命中: {}", exe.display());
    }
    // 声明路径全落空时的兜底：npm 的**实际**全局前缀 / 注册表卸载项里的安装位置。
    // 声明里写的都是**默认位置**，用户把工具装到别的盘就一条都不命中
    // （实测 `WorkBuddy` 在 `D:\sim-tool\WorkBuddy`、`openscience` 在 `D:\Program Files\openscience`、
    // npm 前缀在 `D:\any-versions\sdk\nodejs`），只靠 PATH 兜不住 →
    // 表现为「明明装了却显示未安装、启动按钮不可用」。两条来源都有进程内缓存。
    let resolved_exe = declared_exe.clone().or_else(|| {
        let found = super::tool_paths::find_fallback_exe(&config.id, &paths.command, &paths.paths);
        if let Some(exe) = &found {
            eprintln!(
                "[detect]   兜底命中（非默认安装位置）: {}",
                exe.display()
            );
        }
        found
    });

    let upgrade_cmd = match config.pkg_manager.as_deref() {
        Some("npm") => format!("npm install -g {}@latest", config.pkg_name.as_deref().unwrap_or(&config.id)),
        Some("pip") => format!("pip install --upgrade {}", config.pkg_name.as_deref().unwrap_or(&config.id)),
        _ => paths.install_cmd.clone(),
    };

    let not_found = AiToolDefDto {
        id: config.id.clone(),
        display_name: config.display_name.clone(),
        avatar: config.avatar.clone(),
        nickname: config.nickname.clone(),
        installed: false,
        // 下面每个策略命中时各自覆盖；默认「不是 PM 管理」
        pm_managed: false,
        version: None,
        latest_version_cmd: None,
        install_cmd: paths.install_cmd.clone(),
        upgrade_cmd,
        uninstall_cmd: String::new(),
        website: resolve_tool_website(config.homepage.as_deref(), Some(&config.website), config.github.as_deref()),
        api_protocol: config.api_protocol.clone(),
        supports_model: config.support_model,
        supports_fallback_model: config.support_fallback_model,
        resume_cmd: config.resume_cmd.clone(),
        continue_cmd: config.continue_cmd.clone(),
        fork_cmd: config.fork_cmd.clone(),
        cache_dirs: config.cache_dirs.clone(),
        category: config.category.clone(),
        support_one_m_context: config.support_one_m_context,
        supports_openai: config.supports_openai,
        supports_anthropic: config.supports_anthropic,
        supports_google: config.supports_google,
        builtin_models: config.builtin_models.clone(),
        supports_optimizer: config.supports_optimizer,
        supports_rectifier: config.supports_rectifier,
        supports_plugin_marketplace: config.supports_plugin_marketplace,
        plugin_marketplace_kind: config.plugin_marketplace_kind.clone(),
        launch_uri: paths.launch_uri.clone(),
        detected_path: resolved_exe
            .as_ref()
            .map(|exe| exe.to_string_lossy().to_string()),
        custom_path: super::tool_paths::custom_path_for(&config.id),
        config_file: config.config_file.as_ref().map(|cf| {
            crate::commands::ai_registry::ToolConfigFileDto {
                path: cf.path.clone(),
                format: cf.format.clone(),
            }
        }),
        tool_category: Some(paths.category.clone()),
        tool_kind: Some(crate::commands::ai_registry::tool_kind_of(Some(&paths.category))),
        busy: get_tool_busy(&config.id),
    };

    // 策略 1：按 PM 类型精准查询版本
    if let Some(pm) = config.pkg_manager.as_deref() {
        if let Some(pkg) = config.pkg_name.as_deref() {
            eprintln!("[detect]   [策略 1] PM={}, pkg={}", pm, pkg);
            if let Some(ver) = detect_via_pm(pm, pkg) {
                eprintln!("[detect]   [策略 1] ✓ 成功 → version={}", ver);
                return DetectedAiTool {
                    installed: true,
                    // 策略 1 = 该包确实装在 PM 全局注册表里。只作前端提示用：
                    // 为 false 不代表不能升级/卸载（后端会回落官方命令或按文件清理）
                    pm_managed: true,
                    version: Some(ver),
                    ..not_found
                };
            } else {
                eprintln!("[detect]   [策略 1] ✗ PM 查询失败");
            }
        }
    }

    // 策略 2：回退到 detect_cmd（调用工具自身的 --version）
    let detect_cmd = &paths.detect_cmd;
    eprintln!("[detect]   [策略 2] detect_cmd=\"{}\"", detect_cmd);
    if let Some(ver) = detect_via_cmd(detect_cmd) {
        eprintln!("[detect]   [策略 2] ✓ 成功 → version={}", ver);
        return DetectedAiTool {
            installed: true,
            version: Some(ver),
            ..not_found
        };
    }
    eprintln!("[detect]   [策略 2] ✗ 失败");

    // 策略 3：声明路径上确实有可执行文件，只是命令跑不起来（PATH 里还没有该目录）。
    // 按「未安装」处理会让用户明明装了却看到未安装、启动按钮不可用，
    // 所以这里判为已安装但版本未知（抄作业自 EchoBird f86fe961）。
    if let Some(exe) = resolved_exe {
        eprintln!(
            "[detect]   [策略 3] ✓ 磁盘命中（版本未知）→ {}",
            exe.display()
        );
        return DetectedAiTool {
            installed: true,
            version: None,
            ..not_found
        };
    }

    // 策略 4：Store（MSIX）应用 —— ChatGPT / Claude 桌面端这类「只有包注册、没有普通
    // exe 安装路径」的应用，策略 1~3 全都命中不了（%LOCALAPPDATA%\Programs\ChatGPT\
    // ChatGPT.exe 在 Store 版里根本不存在），只能问系统：包注册了没有。
    if let Some(ver) = detect_via_store_package(paths.launch_uri.as_deref()) {
        eprintln!("[detect]   [策略 4] ✓ Store 包已注册 → version={}", ver);
        // Store 应用没有可声明的 exe 路径（WindowsApps 目录名带版本号，装一次变一次），
        // 「检测到的路径」只能取包的 InstallLocation，否则界面上这条永远是空的，
        // 用户会以为检测失灵了。
        let install_location = store_install_location(paths.launch_uri.as_deref());
        eprintln!(
            "[detect]   [策略 4]   安装目录={}",
            install_location.as_deref().unwrap_or("(取不到)")
        );
        return DetectedAiTool {
            installed: true,
            version: Some(ver),
            detected_path: install_location,
            ..not_found
        };
    }

    eprintln!("[detect] ✗ 未检测到安装");
    not_found
}

/// 从 `shell:AppsFolder\OpenAI.Codex_2p2nqsd0c76g0!App` 里取出包名（`OpenAI.Codex`）。
pub(crate) fn package_name_from_launch_uri(uri: &str) -> Option<String> {
    let after_scheme = uri.strip_prefix("shell:AppsFolder\\")?;
    // 形如 `<包族名>!<应用 Id>`；包族名是 `<包名>_<发布者 Id>`
    let family = after_scheme.split('!').next()?.trim();
    if family.is_empty() {
        return None;
    }
    let name = family.split('_').next()?.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Store（MSIX）应用是否已在系统里注册。返回包版本号。
///
/// 为什么必须走这条路：Store 版应用既不写 `%LOCALAPPDATA%\Programs\…`，也不往 PATH 里
/// 放命令行入口，检测不到它就一直显示「未安装」—— 用户明明装了却只能看到「安装」按钮，
/// 点下去 winget 回「已安装且已是最新」（旧版本还会被当成安装失败）。
fn detect_via_store_package(launch_uri: Option<&str>) -> Option<String> {
    #[cfg(not(windows))]
    {
        let _ = launch_uri;
        return None;
    }

    #[cfg(windows)]
    {
        let pkg = package_name_from_launch_uri(launch_uri?)?;
        crate::commands::project::scanner::msix_package_version(&pkg)
            .ok()
            .flatten()
    }
}

/// Store（MSIX）应用的安装目录（`C:\Program Files\WindowsApps\<名>_<版本>_<架构>__<发布者>`）。
///
/// 用途只有一个：填界面上的「检测到的路径」。这类路径**不能**写进 `paths.json`（版本号
/// 每次更新都变），所以只能现查。取不到就返回 `None`，让界面保持空白而不是显示假路径。
fn store_install_location(launch_uri: Option<&str>) -> Option<String> {
    #[cfg(not(windows))]
    {
        let _ = launch_uri;
        None
    }

    #[cfg(windows)]
    {
        let pkg = package_name_from_launch_uri(launch_uri?)?;
        crate::commands::project::scanner::msix_package_install_location(&pkg)
            .ok()
            .flatten()
    }
}

/// 通过包管理器查询已安装版本（npm / pip）。
///
/// 同时被卸载/升级入口复用：只有这里查到版本，才说明工具**确实由包管理器安装**，
/// 才轮得到 `npm uninstall -g` / `npm install -g` 这类操作。
pub(crate) fn detect_via_pm(pm: &str, pkg_name: &str) -> Option<String> {
    match pm {
        "npm" => {
            // npm ls {pkg} -g --json
            let npm_args = &["ls", pkg_name, "-g", "--depth=0", "--json"];
            eprintln!("[detect]     npm cmd: npm {}", npm_args.join(" "));
            let (stdout, stderr) = run_cmd_output_full("cmd", &["/c", "npm", "ls", pkg_name, "-g", "--depth=0", "--json"]);
            let stdout = stdout?;
            eprintln!("[detect]     npm stdout: {}", safe_slice(&stdout, 500));
            if !stderr.is_empty() {
                eprintln!("[detect]     npm stderr: {}", safe_slice(&stderr, 500));
            }
            // 解析 JSON：{"dependencies": {"@scope/pkg": {"version": "1.2.3"}}}
            match serde_json::from_str::<JsonValue>(&stdout) {
                Ok(val) => {
                    eprintln!("[detect]     npm JSON parsed OK");
                    // 兼容两种格式：dependencies 为对象 或 直接为空
                    if let Some(deps) = val.get("dependencies") {
                        // 先精确匹配
                        if let Some(pkg_info) = deps.get(pkg_name) {
                            if let Some(ver) = pkg_info.get("version").and_then(|v| v.as_str()) {
                                eprintln!("[detect]     npm found {}@{} (exact)", pkg_name, ver);
                                return Some(ver.to_string());
                            }
                        }
                        // 再尝试 scoped package 的可能命名变体
                        eprintln!("[detect]     npm deps keys: {:?}", deps.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                        for (k, v) in deps.as_object()? {
                            let vinfo = v.get("version").and_then(|v| v.as_str());
                            eprintln!("[detect]     npm dep: {}={}", k, vinfo.unwrap_or("?"));
                        }
                    } else {
                        eprintln!("[detect]     npm JSON has no 'dependencies' key, keys: {:?}", val.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                    }
                }
                Err(e) => {
                    eprintln!("[detect]     npm JSON parse error: {}", e);
                }
            }
            None
        }
        "pip" => {
            let pip_args = &["show", pkg_name];
            eprintln!("[detect]     pip cmd: pip {}", pip_args.join(" "));
            let (stdout, stderr) = run_cmd_output_full("cmd", &["/c", "pip", "show", pkg_name]);
            let stdout = stdout?;
            eprintln!("[detect]     pip stdout: {}", safe_slice(&stdout, 300));
            if !stderr.is_empty() {
                eprintln!("[detect]     pip stderr: {}", safe_slice(&stderr, 300));
            }
            // 输出包含 "Version: 1.2.3" 行
            for line in stdout.lines() {
                if let Some(ver) = line.strip_prefix("Version:").or_else(|| line.strip_prefix("version:")) {
                    let v = ver.trim().to_string();
                    eprintln!("[detect]     pip found version: {}", v);
                    return Some(v);
                }
            }
            eprintln!("[detect]     pip 'Version:' line not found");
            None
        }
        _ => None,
    }
}

/// 检测命令的工作目录：一个专用的空目录（见 [`detect_via_cmd`] 的说明）。
///
/// 取不到就用 None（不设置 cwd，行为与改动前一致），不因此让检测失败。
fn neutral_detect_cwd() -> Option<std::path::PathBuf> {
    let dir = crate::commands::config::get_data_dir().join("detect-cwd");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 安装后校验的重试窗口：15 次 × 2 秒 ≈ 28 秒。
///
/// 依据实测（Claude Desktop 1.44121.2）：`winget install` 返回时 `claude.exe` 还没落盘，
/// 稍后才写入（中间要解 Squirrel 包与 224MB 主程序）。慢盘 / 杀毒软件扫描会更久。
const INSTALL_VERIFY_ATTEMPTS: u32 = 15;
const INSTALL_VERIFY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// 在预算内反复探针，命中即返回；预算耗尽返回 `None`。
///
/// `attempts` 为 0 也**至少探一次** —— 否则调用方算出 0 时会一次都不查就直接报「未安装」，
/// 那比误报「未安装」更糟（会把装好的工具说成没装）。
fn probe_with_retry<F>(attempts: u32, interval: std::time::Duration, mut probe: F) -> Option<String>
where
    F: FnMut() -> Option<String>,
{
    let budget = attempts.max(1);
    for i in 0..budget {
        if let Some(v) = probe() {
            return Some(v);
        }
        if i + 1 < budget {
            std::thread::sleep(interval);
        }
    }
    None
}

/// 安装后置校验（抄 EchoBird `auto_fix.rs` / `validate_install_intent`）：
/// **安装命令返回成功 ≠ 工具真的装对了、真的能跑**。
///
/// 要拦住两类问题：
/// 1. **装成了别的包**（EchoBird 的 identity 校验就是这个意图）：命令成功、包也装上了，
///    但工具自己的可执行文件根本不在 —— 版本探测与声明路径双双落空；
/// 2. **装完但不在 PATH 里**：curl / scoop / choco 装的只对**之后**启动的进程生效，
///    本进程 PATH 里还没有它（winget 被 PATH 过滤掉那个坑就是这一类）。
///
/// 因此判定口径与 `detect_single_tool` 的策略 2 / 策略 3 一致：
/// 「版本探测成功」或「声明路径上确实有可执行文件」任一命中即算装好。
///
/// 磁盘判定**必须在窗口内重试**（`probe_with_retry`）：安装器是异步落盘的，命令返回
/// 成功时文件可能还没到。改之前只查一次，于是装好的工具被报成「找不到可执行文件」。
pub(crate) fn verify_installed(tool_id: &str, paths: &PathConfig) -> Result<String, String> {
    if !paths.detect_cmd.trim().is_empty() {
        if let Some(ver) = detect_via_cmd(&paths.detect_cmd) {
            return Ok(ver);
        }
    }
    let found = probe_with_retry(
        INSTALL_VERIFY_ATTEMPTS,
        INSTALL_VERIFY_INTERVAL,
        || {
            super::tool_paths::find_declared_exe(tool_id, &paths.paths, &paths.command)
                .or_else(|| {
                    super::tool_paths::find_fallback_exe(tool_id, &paths.command, &paths.paths)
                })
                .map(|exe| format!("已安装（{}）", exe.display()))
        },
    );
    if let Some(msg) = found {
        return Ok(msg);
    }
    Err(describe_install_miss(paths))
}

/// 安装失败的说明。**有值才插**：桌面端的 `command` 与 `detect_cmd` 本来就是空串，
/// 直接插进模板会得到「找不到**（空）**的可执行文件」这种读不懂的话。
fn describe_install_miss(paths: &PathConfig) -> String {
    let cmd = paths.command.trim();
    let probe = paths.detect_cmd.trim();
    let mut msg = String::from("安装命令已执行成功，但本机找不到");
    if cmd.is_empty() {
        msg.push_str("该工具的可执行文件");
    } else {
        msg.push_str(cmd);
        msg.push_str(" 的可执行文件");
    }
    if probe.is_empty() {
        msg.push_str("，也没有可用的版本探测命令");
    } else {
        msg.push_str(&format!("，也没能通过 `{probe}` 探测到版本"));
    }
    msg.push_str("。可能是装成了别的包，或安装目录尚未进入 PATH —— 请重新打开本应用后再试。");
    msg
}

/// 通过 detect_cmd 回退检测（执行工具自身的 --version 命令）
pub(crate) fn detect_via_cmd(detect_cmd: &str) -> Option<String> {
    let parts: Vec<&str> = detect_cmd.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // 先在 PATH 中找可执行文件
    let exe = find_in_path_local(parts[0])?;
    eprintln!("[detect]     find_in_path({}) → {:?}", parts[0], exe);

    let output = {
        let mut cmd = hidden_cmd::hidden_cmd(&exe);
        if parts.len() > 1 {
            cmd.args(&parts[1..]);
        }
        // 用中性工作目录执行：`node -e require.resolve("<pkg>")` 这类检测命令会
        // 从**当前目录**的 node_modules 向上查找，Kira 的工作目录若恰好能解析到
        // （或曾经能），就会把「本地刚好有这个依赖」误报成「全局已安装」——
        // 表现为界面一直显示已安装，而 npm uninstall -g 什么都删不掉。
        if let Some(cwd) = neutral_detect_cwd() {
            cmd.current_dir(cwd);
        }
        match run_command_with_timeout(&mut cmd, 10) {
            Some(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                eprintln!("[detect]     cmd exit_code={}", out.status.code().map_or(-1, |c| c));
                if !stdout.is_empty() {
                    eprintln!("[detect]     cmd stdout: {}", trimmed_first_line(&stdout, 500));
                }
                if !stderr.is_empty() {
                    eprintln!("[detect]     cmd stderr: {}", trimmed_first_line(&stderr, 500));
                }
                if out.status.success() {
                    Some(format!("{}{}", stdout, stderr))
                } else {
                    None
                }
            }
            None => {
                eprintln!("[detect]     cmd spawn/超时失败");
                None
            }
        }
    }?;

    let stdout = output.trim().to_string();
    if stdout.is_empty() {
        // 空输出 = 没有任何「工具存在」的证据，必须算**未检测到**。
        //
        // pi 这类工具的 detectCmd 写成 `node -e "try{require.resolve(x);console.log('ok')}
        // catch(e){console.log('')}"`：包不存在时**退出码仍然是 0** 且什么都不打印。
        // 把空输出当成「已安装但拿不到版本」，会让这类工具永远显示已安装、
        // 且永远卸不掉（npm 全局根本没这个包，卸载自然毫无效果）——Q-0204。
        eprintln!("[detect]     empty output → 视为未检测到（拿不到任何证据）");
        return None;
    }

    // 用正则提取纯净的 semver 版本号
    let ver = extract_semver(&stdout);
    eprintln!("[detect]     extract_semver({}) → {:?}", trimmed_first_line(&stdout, 100), ver);
    ver
}

/// 在 PATH 中查找可执行文件的绝对路径
fn find_in_path_local(exe_name: &str) -> Option<PathBuf> {
    find_in_path(exe_name)
}

/// 带超时执行命令并完整收集输出（stdout/stderr）。
/// 超时后 kill 进程树（Windows taskkill /T）并返回 None，避免检测工具 --version 卡死线程池。
///
/// `pub(crate)`：插件市场要跑 `codex plugin …`（官方 CLI），同一套超时 + 杀进程树逻辑，
/// 不再重复实现一份。
pub(crate) fn run_command_with_timeout(cmd: &mut std::process::Command, timeout_secs: u64) -> Option<std::process::Output> {
    use std::io::Read;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    let (tx, rx) = mpsc::channel();
    if let Some(mut o) = child.stdout.take() {
        let t = tx.clone();
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = o.read_to_end(&mut buf);
            let _ = t.send((0u8, buf));
        });
    }
    if let Some(mut e) = child.stderr.take() {
        let t = tx.clone();
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = e.read_to_end(&mut buf);
            let _ = t.send((1u8, buf));
        });
    }
    drop(tx);

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            kill_cmd_tree(child.id());
            let _ = child.wait();
            eprintln!("[detect] 命令超时（{}s），已终止进程树", timeout_secs);
            return None;
        }
        thread::sleep(Duration::from_millis(30));
    };

    let mut out_stdout = Vec::new();
    let mut out_stderr = Vec::new();
    for (tag, buf) in rx.iter() {
        if tag == 0 {
            out_stdout = buf;
        } else {
            out_stderr = buf;
        }
    }
    Some(std::process::Output {
        status,
        stdout: out_stdout,
        stderr: out_stderr,
    })
}

/// 终止命令进程树（Windows 用 taskkill /T 连子进程一起杀，避免孤儿进程残留）
fn kill_cmd_tree(pid: u32) {
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

/// 从字符串中提取 semver 版本号（如 1.2.3, 0.45.0-alpha）
fn extract_semver(text: &str) -> Option<String> {
    let re = SEMVER_RE.get_or_init(|| {
        regex::Regex::new(r"(\d+\.\d+\.\d+(?:-[a-zA-Z0-9.]+)?)").unwrap_or_else(|_| regex::Regex::new(r"\d+\.\d+\.\d+").unwrap())
    });
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// 执行命令并返回 (stdout, stderr)，返回 None 时表示失败
fn run_cmd_output_full(exe: &str, args: &[&str]) -> (Option<String>, String) {
    let mut binding = hidden_cmd::hidden_cmd(exe);
    let mut cmd = binding.args(args);
    match run_command_with_timeout(&mut cmd, 10) {
        Some(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            eprintln!("[detect]     run_cmd: {} {}, exit={}",
                exe, args.join(" "),
                output.status.code().map_or(-1, |c| c));
            if output.status.success() {
                (if stdout.is_empty() { None } else { Some(stdout) }, stderr)
            } else {
                (None, stderr)
            }
        }
        None => {
            eprintln!("[detect]     run_cmd FAILED/超时: {} {}", exe, args.join(" "));
            (None, String::new())
        }
    }
}

/// 安全截取字符串，避免在 UTF-8 字符边界中间切片导致 Panic
fn safe_slice(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn trimmed_first_line(s: &str, max_len: usize) -> &str {
    let line = s.lines().next().unwrap_or(s);
    safe_slice(line, max_len)
}

#[tauri::command]
pub async fn detect_ai_tools() -> Result<Vec<DetectedAiTool>, String> {
    let tools_reg = registry();
    let tool_ids: Vec<String> = tools_reg.tool_ids().into_iter().map(|s| s.clone()).collect();
    let mut handles = Vec::with_capacity(tool_ids.len());
    for id in &tool_ids {
        let id = id.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let reg = registry();
            let (config, paths) = match reg.get_tool(&id) {
                Some(t) => (t.0.clone(), t.1.clone()),
                None => return None,
            };
            Some(detect_single_tool(&config, &paths))
        });
        handles.push(handle);
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(Some(result)) => results.push(result),
            Ok(None) => {},
            Err(e) => eprintln!("[detect_ai_tools] task join error: {}", e),
        }
    }
    Ok(results)
}

/// AI 工具版本状态
#[derive(Serialize, Clone, Debug)]
pub struct AiToolVersionStatus {
    pub tool_id: String,
    pub display_name: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub status: String,
    pub busy: Option<String>,
}

/// 检查所有 AI 工具的最新版本（npm/pip 在线查询）
#[tauri::command]
pub async fn check_ai_tool_versions() -> Result<Vec<AiToolVersionStatus>, String> {
    let tools_reg = registry();
    let tool_ids: Vec<String> = tools_reg.tool_ids().into_iter().map(|s| s.clone()).collect();

    let mut handles = Vec::with_capacity(tool_ids.len());
    for id in &tool_ids {
        let id = id.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let reg = registry();
            let (config, paths) = match reg.get_tool(&id) {
                Some(t) => (t.0.clone(), t.1.clone()),
                None => return None,
            };
            let result = detect_single_tool(&config, &paths);
            Some((id, config.display_name.clone(), result))
        });
        handles.push(handle);
    }

    let mut tools: Vec<(String, String, DetectedAiTool)> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Some((id, name, result))) => tools.push((id, name, result)),
            Ok(None) => {},
            Err(e) => eprintln!("[ai_ver] task join error: {}", e),
        }
    }

    // 只对已安装且有 pkg_manager 的工具查最新版本——并发请求
    let mut version_tasks = Vec::new();
    for (id, name, tool) in &tools {
        let tools_reg = registry();
        let pkg_manager = tools_reg.get_tool_config(id).and_then(|c| c.pkg_manager.clone());
        let pkg_name = tools_reg.get_tool_config(id).and_then(|c| c.pkg_name.clone());

        if tool.installed && pkg_manager.is_some() {
            let id = id.clone();
            let name = name.clone();
            let pm = pkg_manager.unwrap();
            let pn = pkg_name.unwrap_or_default();
            version_tasks.push(tokio::spawn(async move {
                let latest = match pm.as_str() {
                    "npm" => fetch_npm_latest_version(&pn).await,
                    "pip" => fetch_pypi_latest_version(&pn).await,
                    _ => None,
                };
                (id, name, latest)
            }));
        }
    }

    let mut latest_map: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    for task in version_tasks {
        if let Ok((id, _name, latest)) = task.await {
            latest_map.insert(id, latest);
        }
    }

    let mut results = Vec::new();
    for (id, name, tool) in &tools {
        let latest = latest_map.get(id).cloned().flatten();
        let busy = get_tool_busy(id);

        let status = if let Some(op) = &busy {
            op.clone()
        } else {
            match (&tool.version, &latest) {
                (None, _) => "not_installed".to_string(),
                (Some(_), None) => "unknown".to_string(),
                (Some(cur), Some(ver)) => {
                    if is_newer(ver, cur) {
                        "outdated".to_string()
                    } else {
                        "latest".to_string()
                    }
                }
            }
        };

        eprintln!("[ai_ver] {}: installed={}, current={:?}, latest={:?}, status={}, busy={:?}",
            id, tool.installed, tool.version, latest, status, busy);

        results.push(AiToolVersionStatus {
            tool_id: id.to_string(),
            display_name: name.to_string(),
            current_version: tool.version.clone(),
            latest_version: latest,
            status,
            busy,
        });
    }

    Ok(results)
}

/// 查询 npm registry 获取最新版本号
async fn fetch_npm_latest_version(package: &str) -> Option<String> {
    let client = get_http_client();
    let url = format!("https://registry.npmjs.org/{}/latest", package);
    eprintln!("[ai_ver] npm fetch: {}", url);

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ai_ver] npm request failed: {}", e);
            return None;
        }
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[ai_ver] npm read body failed: {}", e);
            return None;
        }
    };

    if let Ok(info) = serde_json::from_str::<JsonValue>(&body) {
        if let Some(ver) = info.get("version").and_then(|v| v.as_str()) {
            eprintln!("[ai_ver] npm latest for {}: {}", package, ver);
            return Some(ver.to_string());
        }
    }
    eprintln!("[ai_ver] npm response for {} had no 'version' field", package);
    None
}

/// 查询 PyPI 获取最新版本号
async fn fetch_pypi_latest_version(package: &str) -> Option<String> {
    let client = get_http_client();
    let url = format!("https://pypi.org/pypi/{}/json", package);
    eprintln!("[ai_ver] pypi fetch: {}", url);

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ai_ver] pypi request failed: {}", e);
            return None;
        }
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[ai_ver] pypi read body failed: {}", e);
            return None;
        }
    };

    if let Ok(info) = serde_json::from_str::<JsonValue>(&body) {
        if let Some(ver) = info
            .get("info")
            .and_then(|i| i.get("version"))
            .and_then(|v| v.as_str())
        {
            eprintln!("[ai_ver] pypi latest for {}: {}", package, ver);
            return Some(ver.to_string());
        }
    }
    eprintln!("[ai_ver] pypi response for {} had no 'info.version' field", package);
    None
}

// ─── skills / usage 文件路径 ───


#[cfg(test)]
mod tests {
    use super::{probe_with_retry, resolve_tool_website};

    #[test]
    fn website_prefers_homepage_over_website_and_github() {
        assert_eq!(
            resolve_tool_website(
                Some("https://omp.sh/"),
                Some("https://github.com/can1357/oh-my-pi"),
                Some("https://github.com/can1357/oh-my-pi"),
            ),
            "https://omp.sh/"
        );
    }

    #[test]
    fn website_keeps_website_when_no_homepage() {
        assert_eq!(
            resolve_tool_website(None, Some("https://claude.ai/code"), Some("https://github.com/anthropics/claude-code")),
            "https://claude.ai/code"
        );
    }

    #[test]
    fn website_falls_back_to_github_when_others_blank() {
        assert_eq!(
            resolve_tool_website(Some("  "), Some(""), Some("https://github.com/openai/codex")),
            "https://github.com/openai/codex"
        );
    }

    #[test]
    fn website_empty_without_any_source() {
        assert_eq!(resolve_tool_website(None, None, None), "");
        assert_eq!(resolve_tool_website(None, Some(""), None), "");
    }

    // ═══════════════ 安装后校验的重试（竞态修复） ═══════════════

    /// 首次就命中 → 只探一次，不浪费时间
    #[test]
    fn retry_stops_at_the_first_hit() {
        let mut calls = 0;
        let got = probe_with_retry(5, std::time::Duration::ZERO, || {
            calls += 1;
            Some("v1.44121.2".to_string())
        });
        assert_eq!(got.as_deref(), Some("v1.44121.2"));
        assert_eq!(calls, 1, "首次命中就不该再探");
    }

    /// 前几次探不到、后来才落盘 → 必须等到它出现（这正是 Squirrel 的情形）
    #[test]
    fn retry_waits_for_a_slow_installer_to_finish() {
        let mut calls = 0;
        let got = probe_with_retry(5, std::time::Duration::ZERO, || {
            calls += 1;
            if calls < 3 {
                None
            } else {
                Some("已安装".to_string())
            }
        });
        assert_eq!(got.as_deref(), Some("已安装"));
        assert_eq!(calls, 3, "第 3 次才落盘 → 应该探到，且不多探一次");
    }

    /// 一直探不到 → 探满次数后放弃，把错误抛给调用方（不能让安装流程卡死）
    #[test]
    fn retry_gives_up_after_the_attempt_budget() {
        let mut calls = 0;
        let got: Option<String> = probe_with_retry(4, std::time::Duration::ZERO, || {
            calls += 1;
            None
        });
        assert!(got.is_none());
        assert_eq!(calls, 4, "探满 4 次就放弃");
    }

    /// 0 次预算 = 不探（防止调用方算出 0 导致一次都不查就报「未安装」）
    #[test]
    fn retry_with_zero_budget_still_probes_once() {
        let mut calls = 0;
        let got = probe_with_retry(0, std::time::Duration::ZERO, || {
            calls += 1;
            Some("x".to_string())
        });
        assert_eq!(got.as_deref(), Some("x"));
        assert_eq!(calls, 1, "预算为 0 也至少探一次，否则会误报未安装");
    }
}
