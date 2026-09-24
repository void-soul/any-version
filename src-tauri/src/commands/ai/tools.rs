use crate::commands::ai_registry::registry;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};

/// 安装 / 卸载 / 升级的实时进度事件名：每行命令输出推一次，前端工具面板逐行显示。
const TOOL_PROGRESS_EVENT: &str = "ai-tool-progress";

/// 进度事件载荷：一条命令输出行。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolProgressPayload {
    tool_id: String,
    /// "installing" | "uninstalling" | "upgrading"
    phase: String,
    line: String,
}

/// 安装 / 卸载 / 升级的结果。
///
/// 这三个命令此前返回 `Result<String, String>`，成败全靠前端 `msg.includes("成功")` 猜 ——
/// 「已清理 3 处安装文件」这种正常成功文案不含「成功」二字，会被渲染成红色报错样式，
/// 用户看到的就是「明明卸干净了却报错」。改成结构化结果，前端不再猜。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOpResult {
    pub ok: bool,
    pub message: String,
}

/// 跟踪正在进行中的 升级/安装/卸载 操作，作为“进行中”状态的权威来源。
/// 前端在切换 Agent、切换页面或组件重新挂载后，仍可从 detect / versions 结果中
/// 读取到 busy 标记，从而持续显示“升级中/安装中/卸载中”，而不会被回退为“可升级 + 升级按钮”。
static TOOL_OPS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub fn set_tool_busy(tool_id: &str, op: &str) {
    if let Ok(mut m) = TOOL_OPS.lock() {
        m.get_or_insert_with(HashMap::new)
            .insert(tool_id.to_string(), op.to_string());
    }
}

pub fn clear_tool_busy(tool_id: &str) {
    if let Ok(mut m) = TOOL_OPS.lock() {
        if let Some(map) = m.as_mut() {
            map.remove(tool_id);
        }
    }
}

/// 供 detect / versions 命令读取某工具是否正在进行中操作
pub fn get_tool_busy(tool_id: &str) -> Option<String> {
    TOOL_OPS
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|map| map.get(tool_id).cloned()))
}

/// 借用型守卫：离开作用域（含提前 return / ? 错误返回）时自动清除 busy 标记
struct ToolBusyGuard {
    id: String,
}

impl Drop for ToolBusyGuard {
    fn drop(&mut self) {
        clear_tool_busy(&self.id);
    }
}

// ─── 流式执行（安装/卸载/升级的实时进度） ───

/// 逐行读取子进程的一个管道：每行推一次进度事件，并把输出收进 `sink`（失败时回显原因）。
///
/// 两个管道必须**并发**读：只读其中一个时，另一个写满内核缓冲区会让子进程阻塞在写，
/// 表现为「进度卡住不动、命令永不结束」。
async fn stream_pipe<R: tokio::io::AsyncRead + Unpin>(
    app: &AppHandle,
    tool_id: &str,
    phase: &str,
    pipe: R,
    sink: &Mutex<Vec<String>>,
) {
    let mut lines = BufReader::new(pipe).lines();
    while let Ok(Some(raw)) = lines.next_line().await {
        let line = raw.trim_end().to_string();
        if line.trim().is_empty() {
            continue;
        }
        let _ = app.emit(
            TOOL_PROGRESS_EVENT,
            ToolProgressPayload {
                tool_id: tool_id.to_string(),
                phase: phase.to_string(),
                line: line.clone(),
            },
        );
        if let Ok(mut sink) = sink.lock() {
            // npm 的进度输出可能很长，只留尾部用于报错回显
            if sink.len() < 300 {
                sink.push(line);
            }
        }
    }
}

/// 执行一条命令，边跑边把输出推给前端。成功返回 Ok，失败返回原因（含输出尾部）。
async fn run_streaming(
    app: &AppHandle,
    tool_id: &str,
    phase: &str,
    cmd: &str,
) -> Result<(), String> {
    let mut c = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    c.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
    let mut child = c
        .args(["/c", cmd])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行命令: {}", e))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let sink = Mutex::new(Vec::<String>::new());
    let out_task = async {
        if let Some(p) = stdout {
            stream_pipe(app, tool_id, phase, p, &sink).await
        }
    };
    let err_task = async {
        if let Some(p) = stderr {
            stream_pipe(app, tool_id, phase, p, &sink).await
        }
    };
    let (status, _, _) = tokio::join!(child.wait(), out_task, err_task);
    let status = status.map_err(|e| format!("等待命令结束失败: {}", e))?;
    if status.success() {
        return Ok(());
    }
    let lines = sink.into_inner().unwrap_or_default();
    let tail: Vec<String> = lines.iter().rev().take(5).rev().cloned().collect();
    Err(if tail.is_empty() {
        format!("命令以 {} 退出", status)
    } else {
        tail.join("\n")
    })
}

/// 安装工具：跑工具自带的官方安装命令，输出实时推给前端。
#[tauri::command]
pub async fn install_ai_tool(app: AppHandle, tool_id: String) -> Result<ToolOpResult, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "installing");
    let install_cmd = paths.install_cmd.trim().to_string();
    if install_cmd.is_empty() {
        return Err(format!("{} 未配置安装命令", config.display_name));
    }
    match run_streaming(&app, &tool_id, "installing", &install_cmd).await {
        Ok(()) => Ok(ToolOpResult {
            ok: true,
            message: format!("安装完成：{}", install_cmd),
        }),
        Err(e) => Err(format!("安装失败：{}", e)),
    }
}

/// 包管理器操作类型。
#[derive(Clone, Copy)]
enum PmOp {
    Install,
    Uninstall,
}

/// 由工具声明的包管理器生成升级 / 卸载命令**候选**（按尝试顺序）；不认识的包管理器返回空，
/// 调用方此时应回落到工具自带的官方命令（`installCmd` / `uninstallCmd`）。
///
/// 这里**刻意不**要求「包管理器全局注册表里查得到该包」再去执行：npm 的全局前缀
/// 随 node 版本变化（nvm / fnm / volta），Kira 查询用的 npm 与用户当初安装用的
/// 可能不是同一个 —— 据此拒绝操作会让「本机明明装了」的工具变成卸载不了的死结。
///
/// 由**检测到的可执行文件所在目录**推断「当初装这个包的是哪个包管理器」，
/// 并生成对应命令（用该目录里的包管理器，而不是 PATH 里的第一个）。
///
/// 这是卸载能真正生效的关键：本机往往有多个 node/npm（Kira 自带 SDK、系统
/// node、scoop、pnpm、bun…），每个的全局前缀互不相通。用 PATH 里那个 npm 去
/// 卸「另一个前缀装的包」，npm 对未安装的包照样返回成功，上层就以为卸不掉，
/// 最后落到「按文件清理」把 exe 直接删了 —— 留下包管理器里的一份记录。
fn pm_command_for_install_dir(dir: &Path, pkg: &str, op: PmOp) -> Option<String> {
    let pick = |names: &[&str]| -> Option<PathBuf> {
        names
            .iter()
            .map(|n| dir.join(n))
            .find(|p| p.is_file())
    };
    let install = matches!(op, PmOp::Install);

    if let Some(npm) = pick(&["npm.cmd", "npm"]) {
        let spec = if install { format!("{}@latest", pkg) } else { pkg.to_string() };
        // 带完整路径调用，避免 PATH 里那个 npm 抢走
        return Some(format!(
            "\"{}\" {} -g {}",
            npm.display(),
            if install { "install" } else { "uninstall" },
            spec
        ));
    }
    if let Some(pnpm) = pick(&["pnpm.cmd", "pnpm.exe", "pnpm"]) {
        return Some(format!(
            "\"{}\" {} -g {}",
            pnpm.display(),
            if install { "add" } else { "remove" },
            pkg
        ));
    }
    if let Some(bun) = pick(&["bun.exe", "bun"]) {
        return Some(format!(
            "\"{}\" {} -g {}",
            bun.display(),
            if install { "add" } else { "remove" },
            pkg
        ));
    }
    // scoop 的 shims 目录里没有包管理器，交给 scoop 自己
    if dir.file_name().and_then(|n| n.to_str()) == Some("shims") {
        return Some(format!("scoop uninstall {}", pkg));
    }
    None
}

/// 本机所有可能的 npm 全局前缀（用来回答「这个包到底装在哪」）。
///
/// 一台机器上常有多个 node（Kira 自带 SDK、系统安装、nvm/fnm/volta…），
/// 每个的全局前缀互不相通；只问 PATH 里第一个 npm 会答错。
fn npm_prefix_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let has_npm = dir.join("npm.cmd").is_file() || dir.join("npm").is_file();
            if has_npm && !out.contains(&dir) {
                out.push(dir);
            }
        }
    }
    // 常见固定位置：即使没进 PATH 也可能装着包
    let mut extras: Vec<PathBuf> = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        extras.push(PathBuf::from(appdata).join("npm"));
    }
    if cfg!(windows) {
        extras.push(PathBuf::from(r"C:\Program Files\nodejs"));
    }
    for dir in extras {
        let has_npm = dir.join("npm.cmd").is_file() || dir.join("npm").is_file();
        if has_npm && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// 该 npm 布局目录里是否真的装了这个包（`prefix/node_modules/<pkg>`）。
///
/// 用来在执行前拦一道：npm 卸载「没装的包」也返回成功，不拦就会被误判成
/// 「命令成功但仍能检测到」，白跑一趟还误导后续渠道。
fn package_present_in_dir(dir: &Path, pkg: &str) -> bool {
    if pkg.is_empty() {
        return false;
    }
    // 只挡路径穿越与绝对路径；npm 的 scope 包名（`@openai/codex`）本身就是合法的
    // 两级目录，不能因为它含 `/` 就拒掉
    if Path::new(pkg)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return false;
    }
    dir.join("node_modules").join(pkg).is_dir()
}

/// pip 给两条候选：Windows 上 `pip.exe` 常常没进 PATH（只有 `python` 进了），
/// 只写 `pip ...` 会直接「命令找不到」，于是工具明明装着却卸不掉。
fn pm_commands(pkg_manager: Option<&str>, pkg: &str, op: PmOp) -> Vec<String> {
    match (pkg_manager, op) {
        (Some("npm"), PmOp::Install) => vec![format!("npm install -g {}@latest", pkg)],
        (Some("npm"), PmOp::Uninstall) => vec![format!("npm uninstall -g {}", pkg)],
        (Some("pip"), PmOp::Install) => vec![
            format!("pip install --upgrade {}", pkg),
            format!("python -m pip install --upgrade {}", pkg),
        ],
        (Some("pip"), PmOp::Uninstall) => vec![
            format!("pip uninstall -y {}", pkg),
            format!("python -m pip uninstall -y {}", pkg),
        ],
        _ => Vec::new(),
    }
}

/// 同名可执行文件的垫片后缀（顺序即删除顺序，空串代表无后缀）。
const SHIM_EXTS: [&str; 6] = ["exe", "cmd", "bat", "ps1", "sh", ""];

/// 检测到的可执行文件本身，加上同目录下它的各种垫片变体（只保留存在的）。
///
/// npm / pip 在全局 bin 目录里放的是一组同名文件（`codex` + `codex.cmd` +
/// `codex.ps1`），检测只命中其中一个；只删命中那个会留下另一个仍能启动的残壳。
/// `exists` 由调用方注入，便于在无文件系统的场景下断言候选集合。
fn shim_variants_with(exe: &Path, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![exe.to_path_buf()];
    if let (Some(dir), Some(stem)) = (exe.parent(), exe.file_stem().and_then(|s| s.to_str())) {
        for ext in SHIM_EXTS {
            candidates.push(dir.join(if ext.is_empty() {
                stem.to_string()
            } else {
                format!("{}.{}", stem, ext)
            }));
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if !out.contains(&c) && exists(&c) {
            out.push(c);
        }
    }
    out
}

/// 见 [`shim_variants_with`]。
fn shim_variants(exe: &Path) -> Vec<PathBuf> {
    shim_variants_with(exe, |p| p.exists())
}

/// 由垫片位置推断包管理器真正落盘的包目录（npm 全局 / Python site-packages）。
///
/// 只返回**存在**的目录，且候选路径末尾恰好是包名、中间含 `node_modules` 或
/// `site-packages`，不会误删用户其它目录。包名形如 `@scope/name` 时可直接按
/// 相对路径拼接。
fn pkg_dir_candidates_with(shim: &Path, pkg: &str, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let Some(dir) = shim.parent() else {
        return Vec::new();
    };
    let mut bases: Vec<PathBuf> = vec![
        // 垫片直接躺在 <prefix> 下（Windows 的 npm 全局）
        dir.join("node_modules"),
        dir.join("Lib").join("site-packages"),
        // 垫片在 <prefix>/bin 下（Linux 的 npm 全局）
        dir.join("lib").join("node_modules"),
        dir.join("lib").join("site-packages"),
    ];
    if let Some(prefix) = dir.parent() {
        // 垫片在 <prefix>/bin 或 <prefix>/Scripts 下（Windows 的 pip）
        bases.push(prefix.join("node_modules"));
        bases.push(prefix.join("lib").join("node_modules"));
        bases.push(prefix.join("Lib").join("site-packages"));
        bases.push(prefix.join("lib").join("site-packages"));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for base in bases {
        let target = base.join(pkg);
        if !out.contains(&target) && exists(&target) {
            out.push(target);
        }
    }
    out
}

/// 见 [`pkg_dir_candidates_with`]。
fn pkg_dir_candidates(shim: &Path, pkg: &str) -> Vec<PathBuf> {
    pkg_dir_candidates_with(shim, pkg, |p| p.exists())
}

// ─── 工具数据目录（`~/.pi` / `~/.codex` …） ───

/// 用户主目录（与 AI 模块其它地方同一口径：USERPROFILE 优先，其次 HOME）。
fn user_home_dir() -> Option<PathBuf> {
    let raw = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// `cache_dirs` 声明 → 主目录下的绝对路径（只收真正存在的目录）。
///
/// 安全过滤（卸载时删的是**用户数据**，比删安装文件更危险，口径必须更严）：
/// - 只接受相对路径，拒绝绝对路径与盘符；
/// - 拒绝含 `..` 的路径（防越出主目录）；
/// - 解析结果必须**严格位于主目录之内**，等于主目录本身也拒绝。
fn tool_data_dirs(cache_dirs: &[String], home: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in cache_dirs {
        let name = entry.trim();
        if name.is_empty() || Path::new(name).is_absolute() {
            continue;
        }
        if Path::new(name)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            continue;
        }
        let full = home.join(name);
        if full == home || !full.starts_with(home) {
            continue;
        }
        if full.is_dir() {
            out.push(full);
        }
    }
    out
}

/// 删除工具的数据目录（用户数据，一律**移入回收站**而不是直接删）。
///
/// Junction / 软链接只删链接本身，不跟随到真实目录（迁移过缓存的工具，
/// 真实数据可能在别的盘，删掉链接即可）。返回实际处理的路径清单。
fn remove_tool_data_dirs(config: &crate::commands::ai_registry::ToolConfig) -> Result<Vec<String>, String> {
    let Some(home) = user_home_dir() else {
        return Err("无法定位用户主目录，未删除任何数据目录".to_string());
    };
    let targets = tool_data_dirs(&config.cache_dirs, &home);
    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let mut removed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for target in targets {
        let is_link = std::fs::symlink_metadata(&target)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let result = if is_link {
            // 只删链接，绝不递归删链接指向的真实目录
            std::fs::remove_dir(&target).map_err(|e| e.to_string())
        } else {
            trash::delete(&target).map_err(|e| format!("{}", e))
        };
        match result {
            Ok(()) => removed.push(target.display().to_string()),
            Err(e) => failed.push(format!("{}（{}）", target.display(), e)),
        }
    }

    if removed.is_empty() {
        return Err(format!("数据目录删除失败：{}", failed.join("；")));
    }
    if !failed.is_empty() {
        // 部分成功：把没删掉的如实带回去，不假装全成
        return Err(format!(
            "已移入回收站：{}；另有未删除：{}",
            removed.join("、"),
            failed.join("；")
        ));
    }
    Ok(removed)
}

/// 系统目录保护：即使某个工具的注册表项被误配成系统可执行文件，
/// 按文件清理也不允许动到系统自带的东西。
fn is_protected_path(p: &Path) -> bool {
    let Some(dir) = p.parent() else {
        return true;
    };
    let dir = dir.to_string_lossy().to_ascii_lowercase();
    dir.contains("\\windows\\system32")
        || dir.contains("\\windows\\syswow64")
        || dir == "/usr/bin"
        || dir == "/bin"
        || dir == "/usr/sbin"
        || dir == "/sbin"
}

/// 定位工具的可执行文件（只看注册表声明的路径）。
fn detect_exe(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> Option<PathBuf> {
    super::tool_paths::find_declared_exe(&config.id, &paths.paths, &paths.start_command)
}

/// 在给定的目录列表里按垫片后缀找可执行文件（顺序即优先级）。目录列表由调用方给出，便于单测。
fn find_exe_in_dirs(
    dirs: impl IntoIterator<Item = PathBuf>,
    name: &str,
) -> Option<PathBuf> {
    for dir in dirs {
        for ext in SHIM_EXTS {
            let cand = dir.join(if ext.is_empty() {
                name.to_string()
            } else {
                format!("{}.{}", name, ext)
            });
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// 在 PATH 里定位工具的可执行文件。
///
/// 检测（策略 2/3）本来就能靠 PATH 发现工具，但「按文件清理」此前只认注册表里声明的
/// 路径 —— 于是出现「界面明明检测到、卸载却说找不到它的可执行文件」的死结（Q-0191）。
/// 这里用 `start_command` 的首个 token 在 PATH 各目录里按垫片后缀回头找一遍。
fn find_exe_on_path(command: &str) -> Option<PathBuf> {
    let name = command.split_whitespace().next()?.trim();
    if name.is_empty() {
        return None;
    }
    let raw = std::env::var_os("PATH")?;
    find_exe_in_dirs(std::env::split_paths(&raw), name)
}

/// 「可执行文件在哪」的统一口径：先看注册表声明的路径，再看 PATH。
///
/// 两条渠道必须一致 —— 若检测走 PATH、清理只看声明路径，就会出现
/// 「检测得到但清理说找不到」，或者「清理报成功但 PATH 里那份还在」。
fn detect_exe_any(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> Option<PathBuf> {
    detect_exe(config, paths).or_else(|| find_exe_on_path(&paths.start_command))
}

/// 「现在还能不能检测到这个工具」——用于确认某条命令报成功是否真的生效。
///
/// 必须与 `detect.rs::detect_single_tool` **同一口径**（PM 记录 / detect_cmd /
/// 声明路径，任一命中即算已安装）。只看可执行文件是否存在是不够的：
/// npm 卸载掉垫片后 exe 没了，但 `npm ls` 或 `node -e require.resolve(...)` 仍
/// 能命中（包还在某个前缀里），界面就会显示「卸载成功、却仍是已安装」。
fn still_installed(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> bool {
    let pm_hit = config
        .pkg_manager
        .as_deref()
        .zip(config.pkg_name.as_deref())
        .and_then(|(pm, pkg)| super::detect::detect_via_pm(pm, pkg))
        .is_some();
    let cmd_hit = super::detect::detect_via_cmd(&paths.detect_cmd).is_some();
    let exe_hit = detect_exe_any(config, paths).is_some();
    still_installed_from(pm_hit, cmd_hit, exe_hit)
}

/// 三条检测口径的组合（拆出来便于单测：命中任一即视为仍安装）。
fn still_installed_from(pm_hit: bool, cmd_hit: bool, exe_hit: bool) -> bool {
    pm_hit || cmd_hit || exe_hit
}

/// 出错时的落尾：把 Kira 实际检测到的可执行文件路径给出来，便于手动处理。
fn all_channels_failed(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
    action: &str,
    notes: &[String],
) -> String {
    let detail = if notes.is_empty() {
        "没有可用的安装渠道".to_string()
    } else {
        notes.join("；")
    };
    match detect_exe_any(config, paths) {
        Some(exe) => format!(
            "{} {}失败（{}）。Kira 检测到的可执行文件：{}",
            config.display_name,
            action,
            detail,
            exe.display()
        ),
        None => format!("{} {}失败（{}）", config.display_name, action, detail),
    }
}

/// 按文件清理：删掉检测到的可执行文件（连同同目录的垫片变体），
/// 以及能确认归属该包的 `node_modules` / `site-packages` 目录。
///
/// 这是包管理器与官方命令都不可用时的最后手段：npm 前缀不一致、包已卸但垫片
/// 残留、官方脚本装的单文件 CLI 等。返回实际删除清单。
///
/// 两个关键点：
/// ① 可执行文件用 [`detect_exe_any`] 定位（声明路径 → PATH），别处安装的工具也能找到；
/// ② 清理完**必须复核**：Windows 上正在运行的 exe 删不掉（文件锁），以前只要删掉任意
///    一项就返回成功，界面显示「已清理」而工具还在 —— 现在复核未过一律算失败。
fn remove_detected_install(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> Result<String, String> {
    let exe = detect_exe_any(config, paths).ok_or_else(|| {
        "Kira 既没在声明的路径、也没在 PATH 里找到它的可执行文件，无法按文件清理".to_string()
    })?;
    let pkg = config.pkg_name.as_deref().unwrap_or(&config.id);

    let mut removed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for target in shim_variants(&exe)
        .into_iter()
        .chain(pkg_dir_candidates(&exe, pkg))
    {
        if is_protected_path(&target) {
            failed.push(format!("{}（系统目录，已跳过）", target.display()));
            continue;
        }
        if !target.exists() {
            continue;
        }
        let result = if target.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        match result {
            Ok(()) => removed.push(target.display().to_string()),
            Err(e) => failed.push(format!("{}（{}）", target.display(), e)),
        }
    }

    if removed.is_empty() {
        return Err(if failed.is_empty() {
            "没有找到可清理的安装文件".to_string()
        } else {
            format!("清理失败：{}", failed.join("；"))
        });
    }

    // 复核：文件删了不等于工具没了（另一份安装、或被占用的文件仍在）
    if let Some(left) = detect_exe_any(config, paths) {
        return Err(format!(
            "已删除 {}，但 {} 仍能被检测到（{}）—— 可能还有一份安装，或文件正被运行中的进程占用；请先退出该工具再试",
            removed.join("、"),
            config.display_name,
            left.display()
        ));
    }

    let mut msg = format!("已清理 {} 处安装文件：{}", removed.len(), removed.join("、"));
    if !failed.is_empty() {
        msg.push_str(&format!("；另有 {} 处未清理：{}", failed.len(), failed.join("；")));
    }
    Ok(msg)
}

/// 升级工具。
///
/// 渠道顺序：① 声明的包管理器 → ② 工具自带的官方安装命令。
/// 两者都不可用（或都失败）时才报错，报错里带上各渠道的输出尾部与检测到的文件位置。
/// 全过程逐行推送进度事件，前端实时显示命令输出。
#[tauri::command]
pub async fn upgrade_ai_tool(app: AppHandle, tool_id: String) -> Result<ToolOpResult, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "upgrading");

    let pkg = config.pkg_name.as_deref().unwrap_or(&config.id);
    let mut notes: Vec<String> = Vec::new();

    for cmd in pm_commands(config.pkg_manager.as_deref(), pkg, PmOp::Install) {
        match run_streaming(&app, &tool_id, "upgrading", &cmd).await {
            Ok(()) => {
                return Ok(ToolOpResult {
                    ok: true,
                    message: format!("升级完成：{}", cmd),
                })
            }
            Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
        }
    }

    if !paths.install_cmd.trim().is_empty() {
        match run_streaming(&app, &tool_id, "upgrading", &paths.install_cmd).await {
            Ok(()) => {
                return Ok(ToolOpResult {
                    ok: true,
                    message: format!("已通过官方安装命令升级：{}", paths.install_cmd),
                })
            }
            Err(e) => notes.push(format!("官方安装命令失败：{}", e)),
        }
    }

    Err(all_channels_failed(&config, &paths, "升级", &notes))
}

/// 卸载工具。
///
/// 渠道顺序：① 配置的官方卸载命令 → ② 声明的包管理器 → ③ 按文件清理。
/// 前两条命令「报成功但工具仍能被检测到」时不算数（`npm uninstall -g` 对不在
/// 该前缀下的包可能静默成功），继续往下走，避免出现「界面说卸载成功、图标还在」。
/// 第三环对**非包管理器安装**的工具同样有效：可执行文件先按声明路径找、再按 PATH 找。
///
/// `remove_data_dirs` = true 时，卸载成功后顺带把该工具的数据目录
/// （`~/.pi`、`~/.codex` 等 `cacheDirs`）**移入回收站**——由用户在确认框里勾选，
/// 默认不动（卸载程序 ≠ 删除用户数据，这两件事不该被捆死）。
#[tauri::command]
pub async fn uninstall_ai_tool(
    app: AppHandle,
    tool_id: String,
    remove_data_dirs: Option<bool>,
) -> Result<ToolOpResult, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "uninstalling");

    let pkg = config.pkg_name.as_deref().unwrap_or(&config.id);
    let mut notes: Vec<String> = Vec::new();
    // 卸载成功的文案（各渠道分散在下面几段，统一收集后再决定要不要动数据目录）
    let mut uninstalled: Option<String> = None;

    if uninstalled.is_none() {
        if let Some(cmd) = paths
            .uninstall_cmd
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string)
        {
            match run_streaming(&app, &tool_id, "uninstalling", &cmd).await {
                Ok(()) if !still_installed(&config, &paths) => {
                    uninstalled = Some(format!("已通过官方卸载命令卸载：{}", cmd));
                }
                Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
                Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
            }
        }
    }

    // ②' 按**实际安装位置**找包管理器：检测到的 exe 所在目录里有 npm/pnpm/bun
    //    就用它 —— 这才是当初装这个包的那一个（PATH 里的可能是 Kira 自带 SDK 的 npm，
    //    在它自己的前缀下根本没有这个包，卸载会「成功」但毫无效果）。
    if uninstalled.is_none() {
        if let Some(exe) = detect_exe_any(&config, &paths) {
            if let Some(dir) = exe.parent() {
                if let Some(cmd) = pm_command_for_install_dir(dir, pkg, PmOp::Uninstall) {
                    // npm 布局下先确认这个前缀真装了它，避免对「没装的包」白跑一趟
                    let npm_layout = dir.join("npm.cmd").is_file() || dir.join("npm").is_file();
                    if npm_layout && !package_present_in_dir(dir, pkg) {
                        notes.push(format!(
                            "{} 前缀（{}）下没有 {}，已跳过",
                            dir.display(),
                            "npm",
                            pkg
                        ));
                    } else {
                        match run_streaming(&app, &tool_id, "uninstalling", &cmd).await {
                            Ok(()) if !still_installed(&config, &paths) => {
                                uninstalled = Some(format!("已通过 {} 卸载：{}", dir.display(), cmd));
                            }
                            Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
                            Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
                        }
                    }
                }
            }
        }
    }

    // ②'' 逐个排查本机所有 npm 前缀：哪个前缀里真有这个包，就用那个前缀的 npm 卸。
    //     覆盖「垫片已不在 PATH 里、但 npm 仍记着这个包」的情况（工具界面一直显示
    //     已安装，却怎么都卸不掉）。
    if uninstalled.is_none() && config.pkg_manager.as_deref() == Some("npm") {
        for dir in npm_prefix_candidates() {
            if !package_present_in_dir(&dir, pkg) {
                continue;
            }
            let Some(cmd) = pm_command_for_install_dir(&dir, pkg, PmOp::Uninstall) else {
                continue;
            };
            match run_streaming(&app, &tool_id, "uninstalling", &cmd).await {
                Ok(()) if !still_installed(&config, &paths) => {
                    uninstalled = Some(format!("已在 {} 前缀下卸载：{}", dir.display(), cmd));
                    break;
                }
                Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
                Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
            }
        }
    }

    if uninstalled.is_none() {
        for cmd in pm_commands(config.pkg_manager.as_deref(), pkg, PmOp::Uninstall) {
            match run_streaming(&app, &tool_id, "uninstalling", &cmd).await {
                Ok(()) if !still_installed(&config, &paths) => {
                    uninstalled = Some(format!("已通过包管理器卸载：{}", cmd));
                    break;
                }
                Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
                Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
            }
        }
    }

    let mut message = match uninstalled {
        Some(text) => text,
        None => match remove_detected_install(&config, &paths) {
            Ok(summary) => {
                // 按文件清理只删掉了可执行文件（垫片/包目录）。若包管理器或
                // detect_cmd 仍能命中，说明本机还有另一处安装 —— 必须说出来，
                // 否则界面一边报「卸载成功」、一边仍显示已安装，用户只会觉得程序在骗人
                if still_installed(&config, &paths) {
                    format!(
                        "{}；⚠ 检测仍显示已安装：本机可能还有另一处安装（如另一个 npm 前缀），请在该前缀下执行卸载",
                        summary
                    )
                } else if config.pkg_manager.as_deref() == Some("npm") {
                    format!(
                        "{}；注意：npm 里可能仍留有记录，如需彻底清除请在对应 npm 前缀下执行 npm uninstall -g {}",
                        summary, pkg
                    )
                } else {
                    summary
                }
            }
            Err(e) => {
                let mut notes = notes;
                notes.push(e);
                return Err(all_channels_failed(&config, &paths, "卸载", &notes));
            }
        },
    };

    // 工具已经卸掉了，再动数据目录：顺序反了会撞上「文件正被运行中进程占用」
    if remove_data_dirs == Some(true) {
        match remove_tool_data_dirs(&config) {
            Ok(list) => {
                if !list.is_empty() {
                    message = format!("{}；数据目录已移入回收站：{}", message, list.join("、"));
                }
            }
            Err(e) => {
                // 卸载本身成功了：数据目录没删掉只作提醒，不能把整个操作判成失败
                message = format!("{}；⚠ {}", message, e);
            }
        }
    }

    Ok(ToolOpResult { ok: true, message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn pm_command_maps_npm_and_pip() {
        // 只认 npm / pip；其它包管理器回落到工具自带的官方命令
        assert_eq!(
            pm_commands(Some("npm"), "@openai/codex", PmOp::Install),
            vec!["npm install -g @openai/codex@latest"]
        );
        assert_eq!(
            pm_commands(Some("npm"), "@openai/codex", PmOp::Uninstall),
            vec!["npm uninstall -g @openai/codex"]
        );
        // pip 有两条候选：Windows 上 pip.exe 常不在 PATH，第二备用 `python -m pip`
        assert_eq!(
            pm_commands(Some("pip"), "headroom-ai", PmOp::Install),
            vec![
                "pip install --upgrade headroom-ai",
                "python -m pip install --upgrade headroom-ai"
            ]
        );
        assert_eq!(
            pm_commands(Some("pip"), "headroom-ai", PmOp::Uninstall),
            vec![
                "pip uninstall -y headroom-ai",
                "python -m pip uninstall -y headroom-ai"
            ]
        );
        assert!(pm_commands(None, "pi", PmOp::Install).is_empty());
        assert!(pm_commands(Some("go"), "pi", PmOp::Uninstall).is_empty());
    }

    #[test]
    fn still_installed_uses_the_same_verdict_as_detection() {
        // 卸载的「成功」判定必须和 detect 同一口径：PM 记录 / detect_cmd / 声明路径
        // 任一命中都还算已安装。只看 exe 会漏掉「垫片删了、包还在 npm 里」的情况，
        // 表现为界面报卸载成功、却仍显示已安装。
        assert!(still_installed_from(true, false, false));
        assert!(still_installed_from(false, true, false));
        assert!(still_installed_from(false, false, true));
        assert!(!still_installed_from(false, false, false));
    }

    #[test]
    fn pm_command_for_install_dir_uses_the_package_manager_that_owns_the_shim() {
        // 卸载必须用「当初装这个包的那一个」包管理器：目录里有 npm.cmd 就用它
        // （而不是 PATH 里第一个 npm —— 它可能在另一个全局前缀下，卸载毫无效果）
        let dir = std::env::temp_dir().join(format!("kira_pm_probe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 目录里什么都没有 → 认不出包管理器
        assert!(pm_command_for_install_dir(&dir, "pkg", PmOp::Uninstall).is_none());

        std::fs::write(dir.join("npm.cmd"), "").unwrap();
        let cmd = pm_command_for_install_dir(&dir, "@openai/codex", PmOp::Uninstall).unwrap();
        assert!(cmd.contains("uninstall -g @openai/codex"), "实际: {cmd}");
        assert!(cmd.contains(dir.join("npm.cmd").display().to_string().as_str()), "应带完整路径: {cmd}");
        let install = pm_command_for_install_dir(&dir, "@openai/codex", PmOp::Install).unwrap();
        assert!(install.contains("install -g @openai/codex@latest"), "实际: {install}");

        // pnpm / bun 各自的动词
        std::fs::remove_file(dir.join("npm.cmd")).unwrap();
        std::fs::write(dir.join("pnpm.cmd"), "").unwrap();
        assert!(pm_command_for_install_dir(&dir, "pkg", PmOp::Uninstall)
            .unwrap()
            .contains("remove -g pkg"));
        std::fs::remove_file(dir.join("pnpm.cmd")).unwrap();
        std::fs::write(dir.join("bun.exe"), "").unwrap();
        assert!(pm_command_for_install_dir(&dir, "pkg", PmOp::Uninstall)
            .unwrap()
            .contains("remove -g pkg"));

        // scoop shims 目录里没有包管理器，交给 scoop
        let shims = dir.join("shims");
        std::fs::create_dir_all(&shims).unwrap();
        assert_eq!(
            pm_command_for_install_dir(&shims, "pkg", PmOp::Uninstall),
            Some("scoop uninstall pkg".to_string())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_present_in_dir_checks_global_node_modules_layout() {
        // 预检：npm 卸载「没装的包」也返回成功，不拦会让上层误判渠道有效
        let dir = std::env::temp_dir().join(format!("kira_pkg_probe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules").join("@openai/codex")).unwrap();
        assert!(package_present_in_dir(&dir, "@openai/codex"));
        assert!(!package_present_in_dir(&dir, "@openai/other"));
        // 路径穿越一律拒绝
        assert!(!package_present_in_dir(&dir, "../evil"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_data_dirs_only_accepts_existing_subdirs_of_home() {
        // 卸载时删的是用户数据（~/.pi、~/.codex），口径必须比删安装文件更严：
        // 只认主目录下真实存在的子目录，绝对路径 / `..` / 主目录本身一律拒绝
        let home = std::env::temp_dir().join(format!("kira_data_dirs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".pi")).unwrap();
        std::fs::create_dir_all(home.join(".config/deveco")).unwrap();

        let got = tool_data_dirs(
            &[
                ".pi".to_string(),
                ".config/deveco".to_string(),
                ".missing".to_string(),
                "..".to_string(),
                "../evil".to_string(),
            ],
            &home,
        );
        let names: Vec<String> = got
            .iter()
            .map(|p| p.strip_prefix(&home).unwrap().display().to_string())
            .collect();
        assert_eq!(names, vec![".pi", ".config/deveco"]);

        // 绝对路径与空串也要挡掉（防止注册表被改坏时误删系统目录）
        let abs = if cfg!(windows) { "C:\\Windows" } else { "/etc" };
        assert!(tool_data_dirs(&[abs.to_string(), "  ".to_string()], &home).is_empty());
        // 主目录本身绝不能被当成「数据目录」删掉
        assert!(tool_data_dirs(&[".".to_string()], &home).is_empty());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn find_exe_in_dirs_walks_dirs_and_shim_suffixes_in_order() {
        // 非包管理器安装的工具往往只在 PATH 里（注册表声明的路径找不到）：
        // 按文件清理要能在目录列表里找到它，且优先命中 .cmd 垫片
        let dir = std::env::temp_dir().join(format!("kira_path_probe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pi.cmd"), "").unwrap();

        let missing = dir.join("does-not-exist");
        let found = find_exe_in_dirs(vec![missing, dir.clone()], "pi");
        assert_eq!(found.as_deref(), Some(dir.join("pi.cmd").as_path()));

        // 名字对不上就找不到（不能瞎猜）
        assert!(find_exe_in_dirs(vec![dir.clone()], "other-tool").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shim_variants_include_existing_siblings() {
        // npm 在 Windows 上放的是 codex / codex.cmd / codex.ps1 三个垫片，
        // 检测只命中一个，卸载必须把同目录的兄弟一起收走
        let shim = Path::new("/npm/codex.cmd");
        let existing = ["codex.cmd", "codex", "codex.ps1"];
        let got = shim_variants_with(shim, |p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| existing.contains(&n))
        });
        let names: Vec<String> = got
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["codex.cmd", "codex.ps1", "codex"]);
        assert!(got.iter().all(|p| p.parent() == shim.parent()));
    }

    #[test]
    fn shim_variants_do_not_duplicate_the_detected_exe() {
        // 无后缀的 Unix 安装（~/.local/bin/pi）只会命中它自己，不能重复出现
        let shim = Path::new("/home/u/.local/bin/pi");
        let got = shim_variants_with(shim, |p| {
            p.file_name().and_then(|n| n.to_str()) == Some("pi")
        });
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], shim);
    }

    #[test]
    fn pkg_dir_candidates_only_return_existing_package_dirs() {
        // 垫片在 <prefix>/bin 下时，包目录在 <prefix>/lib/node_modules 里
        let shim = Path::new("/home/u/.npm-global/bin/codex");
        let existing = Path::new("/home/u/.npm-global")
            .join("lib")
            .join("node_modules")
            .join("@openai/codex");
        let got = pkg_dir_candidates_with(shim, "@openai/codex", |p| p == existing);
        assert_eq!(got, vec![existing]);
    }

    #[test]
    fn is_protected_path_guards_system_dirs() {
        if cfg!(windows) {
            assert!(is_protected_path(Path::new(r"C:\Windows\System32\where.exe")));
            assert!(!is_protected_path(Path::new(
                r"C:\Users\u\AppData\Roaming\npm\codex.cmd"
            )));
        } else {
            assert!(is_protected_path(Path::new("/usr/bin/pi")));
            assert!(!is_protected_path(Path::new("/home/u/.local/bin/pi")));
        }
    }
}
