use crate::commands::ai_registry::registry;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncReadExt;

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

/// 让 Store（MSIX）包的版本缓存立即失效。
///
/// 装 / 卸之后下一次 detect 若读到这份缓存，会继续用旧的「未安装」状态 ——
/// 明明刚装好界面还显示未安装、明明卸干净了还显示已安装。
fn invalidate_store_cache(paths: &crate::commands::ai_registry::PathConfig) {
    let Some(uri) = paths.launch_uri.as_deref() else {
        return;
    };
    if let Some(pkg) = super::detect::package_name_from_launch_uri(uri) {
        crate::commands::project::scanner::invalidate_msix_version_cache(&pkg);
    }
}

/// 安装工具：跑工具自带的官方安装命令，输出实时推给前端。
#[tauri::command]
pub async fn install_ai_tool(app: AppHandle, tool_id: String) -> Result<ToolOpResult, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard {
        id: tool_id.clone(),
    };
    set_tool_busy(&tool_id, "installing");
    let install_cmd = paths.install_cmd.trim().to_string();
    if install_cmd.is_empty() {
        return Err(format!("{} 未配置安装命令", config.display_name));
    }
    invalidate_store_cache(&paths);
    match run_streaming(&app, &tool_id, "installing", &install_cmd).await {
        Ok(()) => {
            // 后置校验（抄 EchoBird `auto_fix.rs`）：命令成功不等于装对了 ——
            // 装成别的包、或装完不在 PATH 里，都在这里被拦住。
            let _ = app.emit(
                TOOL_PROGRESS_EVENT,
                ToolProgressPayload {
                    tool_id: tool_id.clone(),
                    phase: "installing".to_string(),
                    line: "正在校验安装结果…".to_string(),
                },
            );
            let verified = super::detect::verify_installed(&tool_id, &paths);
            match verified {
                Ok(ver) => {
                    let _ = app.emit(
                        TOOL_PROGRESS_EVENT,
                        ToolProgressPayload {
                            tool_id: tool_id.clone(),
                            phase: "installing".to_string(),
                            line: format!("校验通过：{ver}"),
                        },
                    );
                    Ok(ToolOpResult {
                        ok: true,
                        message: format!("安装完成（{}）：{}", ver, install_cmd),
                    })
                }
                Err(e) => {
                    let _ = app.emit(
                        TOOL_PROGRESS_EVENT,
                        ToolProgressPayload {
                            tool_id: tool_id.clone(),
                            phase: "installing".to_string(),
                            line: format!("⚠ 校验失败：{e}"),
                        },
                    );
                    Err(e)
                }
            }
        }
        Err(e) => {
            // winget「已安装且已是最新」是非零退出，但对「安装」这个目标来说是成功的
            if e.is_winget_already_latest() {
                let _ = app.emit(
                    TOOL_PROGRESS_EVENT,
                    ToolProgressPayload {
                        tool_id: tool_id.clone(),
                        phase: "installing".to_string(),
                        line: "本机已安装且已是最新版本，无需重新安装".to_string(),
                    },
                );
                return Ok(ToolOpResult {
                    ok: true,
                    message: format!("{} 已安装且已是最新版本，无需重新安装", config.display_name),
                });
            }
            Err(format!("安装失败：{}", e.message()))
        }
    }
}

/// 包管理器操作类型。
#[derive(Clone, Copy)]
enum PmOp {
    Install,
    Uninstall,
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
    let _busy_guard = ToolBusyGuard {
        id: tool_id.clone(),
    };
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
            Err(e) => notes.push(format!("{} 失败：{}", cmd, e.message())),
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
            Err(e) => {
                // 「没有可用升级」= 已经是最新，对升级操作而言同样是成功
                if e.is_winget_already_latest() {
                    return Ok(ToolOpResult {
                        ok: true,
                        message: format!("{} 已是最新版本，无需升级", config.display_name),
                    });
                }
                notes.push(format!("官方安装命令失败：{}", e.message()))
            }
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
    let _busy_guard = ToolBusyGuard {
        id: tool_id.clone(),
    };
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
                Err(e) => notes.push(format!("{} 失败：{}", cmd, e.message())),
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
                                uninstalled =
                                    Some(format!("已通过 {} 卸载：{}", dir.display(), cmd));
                            }
                            Ok(()) => notes.push(format!("{} 执行成功，但仍能检测到该工具", cmd)),
                            Err(e) => notes.push(format!("{} 失败：{}", cmd, e.message())),
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
                Err(e) => notes.push(format!("{} 失败：{}", cmd, e.message())),
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
                Err(e) => notes.push(format!("{} 失败：{}", cmd, e.message())),
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

    invalidate_store_cache(&paths);
    Ok(ToolOpResult { ok: true, message })
}

mod process_output;
use process_output::*;

mod package_management;
use package_management::*;

#[cfg(test)]
mod tests;
