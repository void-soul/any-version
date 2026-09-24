use crate::commands::ai_registry::registry;
use std::path::{Path, PathBuf};
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
fn still_installed(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> bool {
    detect_exe_any(config, paths).is_some()
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
#[tauri::command]
pub async fn uninstall_ai_tool(app: AppHandle, tool_id: String) -> Result<ToolOpResult, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "uninstalling");

    let pkg = config.pkg_name.as_deref().unwrap_or(&config.id);
    let mut notes: Vec<String> = Vec::new();

    if let Some(cmd) = paths
        .uninstall_cmd
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_string)
    {
        match run_streaming(&app, &tool_id, "uninstalling", &cmd).await {
            Ok(()) if !still_installed(&config, &paths) => {
                return Ok(ToolOpResult {
                    ok: true,
                    message: format!("已通过官方卸载命令卸载：{}", cmd),
                })
            }
            Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
            Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
        }
    }

    for cmd in pm_commands(config.pkg_manager.as_deref(), pkg, PmOp::Uninstall) {
        match run_streaming(&app, &tool_id, "uninstalling", &cmd).await {
            Ok(()) if !still_installed(&config, &paths) => {
                return Ok(ToolOpResult {
                    ok: true,
                    message: format!("已通过包管理器卸载：{}", cmd),
                })
            }
            Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
            Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
        }
    }

    match remove_detected_install(&config, &paths) {
        Ok(summary) => Ok(ToolOpResult {
            ok: true,
            message: summary,
        }),
        Err(e) => {
            let mut notes = notes;
            notes.push(e);
            Err(all_channels_failed(&config, &paths, "卸载", &notes))
        }
    }
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
