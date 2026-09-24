use crate::commands::ai_registry::registry;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::collections::HashMap;
use std::sync::Mutex;

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

#[tauri::command]
pub async fn install_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (_, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "installing");
    let install_cmd = &paths.install_cmd;
    let mut cmd = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
    let output = cmd
        .args(["/c", install_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("安装失败: {}", e))?;

    if output.status.success() {
        Ok("安装成功".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() {
            "安装失败".to_string()
        } else {
            err
        })
    }
}

/// 包管理器操作类型。
#[derive(Clone, Copy)]
enum PmOp {
    Install,
    Uninstall,
}

/// 由工具声明的包管理器生成升级 / 卸载命令；不认识的包管理器返回 `None`，
/// 调用方此时应回落到工具自带的官方命令（`installCmd` / `uninstallCmd`）。
///
/// 这里**刻意不**要求「包管理器全局注册表里查得到该包」再去执行：npm 的全局前缀
/// 随 node 版本变化（nvm / fnm / volta），Kira 查询用的 npm 与用户当初安装用的
/// 可能不是同一个 —— 据此拒绝操作会让「本机明明装了」的工具变成卸载不了的死结。
fn pm_command(pkg_manager: Option<&str>, pkg: &str, op: PmOp) -> Option<String> {
    match (pkg_manager, op) {
        (Some("npm"), PmOp::Install) => Some(format!("npm install -g {}@latest", pkg)),
        (Some("npm"), PmOp::Uninstall) => Some(format!("npm uninstall -g {}", pkg)),
        (Some("pip"), PmOp::Install) => Some(format!("pip install --upgrade {}", pkg)),
        (Some("pip"), PmOp::Uninstall) => Some(format!("pip uninstall -y {}", pkg)),
        _ => None,
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

/// 在后台静默跑一条命令；失败时把 stderr 带回来（空则用退出码兜底）。
async fn run_shell(cmd: &str) -> Result<(), String> {
    let mut c = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    c.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
    let output = c
        .args(["/c", cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("无法执行命令: {}", e))?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if err.is_empty() {
        format!("命令以 {} 退出", output.status)
    } else {
        err
    })
}

/// 定位工具的可执行文件。
fn detect_exe(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> Option<PathBuf> {
    super::tool_paths::find_declared_exe(&config.id, &paths.paths, &paths.start_command)
}

/// 「现在还能不能检测到这个工具」——用于确认某条命令报成功是否真的生效。
fn still_installed(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> bool {
    detect_exe(config, paths).is_some()
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
    match detect_exe(config, paths) {
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
/// 残留、官方脚本装的单文件 CLI 等。返回实际删除清单；一项都没删掉时返回错误，
/// 并带上每个路径失败的原因（Windows 上文件被正在运行的工具占用很常见）。
fn remove_detected_install(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> Result<String, String> {
    let exe = detect_exe(config, paths)
        .ok_or_else(|| "Kira 找不到它的可执行文件位置，无法按文件清理".to_string())?;
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
    let mut msg = format!("已清理 {} 处安装文件：{}", removed.len(), removed.join("、"));
    if !failed.is_empty() {
        msg.push_str(&format!("；另有 {} 处未清理：{}", failed.len(), failed.join("；")));
    }
    Ok(msg)
}

/// 升级工具。
///
/// 渠道顺序：① 声明的包管理器 → ② 工具自带的官方安装命令。
/// 两者都不可用（或都失败）时才报错，报错里带上各渠道的 stderr 与检测到的文件位置。
#[tauri::command]
pub async fn upgrade_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "upgrading");

    let pkg = config.pkg_name.as_deref().unwrap_or(&config.id);
    let mut notes: Vec<String> = Vec::new();

    if let Some(cmd) = pm_command(config.pkg_manager.as_deref(), pkg, PmOp::Install) {
        match run_shell(&cmd).await {
            Ok(()) => return Ok(format!("升级完成：{}", cmd)),
            Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
        }
    }

    if !paths.install_cmd.trim().is_empty() {
        match run_shell(&paths.install_cmd).await {
            Ok(()) => return Ok(format!("已通过官方安装命令升级：{}", paths.install_cmd)),
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
#[tauri::command]
pub async fn uninstall_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "uninstalling");

    let pkg = config.pkg_name.as_deref().unwrap_or(&config.id);
    let mut notes: Vec<String> = Vec::new();

    if let Some(cmd) = paths
        .uninstall_cmd
        .as_deref()
        .filter(|c| !c.trim().is_empty())
    {
        match run_shell(cmd).await {
            Ok(()) if !still_installed(&config, &paths) => {
                return Ok(format!("已通过官方卸载命令卸载：{}", cmd))
            }
            Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
            Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
        }
    }

    if let Some(cmd) = pm_command(config.pkg_manager.as_deref(), pkg, PmOp::Uninstall) {
        match run_shell(&cmd).await {
            Ok(()) if !still_installed(&config, &paths) => {
                return Ok(format!("已通过包管理器卸载：{}", cmd))
            }
            Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
            Err(e) => notes.push(format!("{} 失败：{}", cmd, e)),
        }
    }

    match remove_detected_install(&config, &paths) {
        Ok(summary) => Ok(summary),
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
            pm_command(Some("npm"), "@openai/codex", PmOp::Install).unwrap(),
            "npm install -g @openai/codex@latest"
        );
        assert_eq!(
            pm_command(Some("npm"), "@openai/codex", PmOp::Uninstall).unwrap(),
            "npm uninstall -g @openai/codex"
        );
        assert_eq!(
            pm_command(Some("pip"), "headroom-ai", PmOp::Install).unwrap(),
            "pip install --upgrade headroom-ai"
        );
        assert_eq!(
            pm_command(Some("pip"), "headroom-ai", PmOp::Uninstall).unwrap(),
            "pip uninstall -y headroom-ai"
        );
        assert!(pm_command(None, "pi", PmOp::Install).is_none());
        assert!(pm_command(Some("go"), "pi", PmOp::Uninstall).is_none());
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
