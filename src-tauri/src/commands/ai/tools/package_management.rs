//! package management responsibility.
use super::*;

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
pub(super) fn pm_command_for_install_dir(dir: &Path, pkg: &str, op: PmOp) -> Option<String> {
    let pick = |names: &[&str]| -> Option<PathBuf> {
        names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
    };
    let install = matches!(op, PmOp::Install);

    if let Some(npm) = pick(&["npm.cmd", "npm"]) {
        let spec = if install {
            format!("{}@latest", pkg)
        } else {
            pkg.to_string()
        };
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
pub(super) fn npm_prefix_candidates() -> Vec<PathBuf> {
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
pub(super) fn package_present_in_dir(dir: &Path, pkg: &str) -> bool {
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
pub(super) fn pm_commands(pkg_manager: Option<&str>, pkg: &str, op: PmOp) -> Vec<String> {
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
pub(super) const SHIM_EXTS: [&str; 6] = ["exe", "cmd", "bat", "ps1", "sh", ""];

/// 检测到的可执行文件本身，加上同目录下它的各种垫片变体（只保留存在的）。
///
/// npm / pip 在全局 bin 目录里放的是一组同名文件（`codex` + `codex.cmd` +
/// `codex.ps1`），检测只命中其中一个；只删命中那个会留下另一个仍能启动的残壳。
/// `exists` 由调用方注入，便于在无文件系统的场景下断言候选集合。
pub(super) fn shim_variants_with(exe: &Path, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
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
pub(super) fn shim_variants(exe: &Path) -> Vec<PathBuf> {
    shim_variants_with(exe, |p| p.exists())
}

/// 由垫片位置推断包管理器真正落盘的包目录（npm 全局 / Python site-packages）。
///
/// 只返回**存在**的目录，且候选路径末尾恰好是包名、中间含 `node_modules` 或
/// `site-packages`，不会误删用户其它目录。包名形如 `@scope/name` 时可直接按
/// 相对路径拼接。
pub(super) fn pkg_dir_candidates_with(
    shim: &Path,
    pkg: &str,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
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
pub(super) fn pkg_dir_candidates(shim: &Path, pkg: &str) -> Vec<PathBuf> {
    pkg_dir_candidates_with(shim, pkg, |p| p.exists())
}

// ─── 工具数据目录（`~/.pi` / `~/.codex` …） ───

/// 用户主目录（与 AI 模块其它地方同一口径：USERPROFILE 优先，其次 HOME）。
pub(super) fn user_home_dir() -> Option<PathBuf> {
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
pub(super) fn tool_data_dirs(cache_dirs: &[String], home: &Path) -> Vec<PathBuf> {
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
pub(super) fn remove_tool_data_dirs(
    config: &crate::commands::ai_registry::ToolConfig,
) -> Result<Vec<String>, String> {
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
pub(super) fn is_protected_path(p: &Path) -> bool {
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
pub(super) fn detect_exe(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> Option<PathBuf> {
    crate::commands::ai::tool_paths::find_declared_exe(
        &config.id,
        &paths.paths,
        &paths.start_command,
    )
}

/// 在给定的目录列表里按垫片后缀找可执行文件（顺序即优先级）。目录列表由调用方给出，便于单测。
pub(super) fn find_exe_in_dirs(
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
pub(super) fn find_exe_on_path(command: &str) -> Option<PathBuf> {
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
pub(super) fn detect_exe_any(
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
pub(super) fn still_installed(
    config: &crate::commands::ai_registry::ToolConfig,
    paths: &crate::commands::ai_registry::PathConfig,
) -> bool {
    let pm_hit = config
        .pkg_manager
        .as_deref()
        .zip(config.pkg_name.as_deref())
        .and_then(|(pm, pkg)| crate::commands::ai::detect::detect_via_pm(pm, pkg))
        .is_some();
    let cmd_hit = crate::commands::ai::detect::detect_via_cmd(&paths.detect_cmd).is_some();
    let exe_hit = detect_exe_any(config, paths).is_some();
    still_installed_from(pm_hit, cmd_hit, exe_hit)
}

/// 三条检测口径的组合（拆出来便于单测：命中任一即视为仍安装）。
pub(super) fn still_installed_from(pm_hit: bool, cmd_hit: bool, exe_hit: bool) -> bool {
    pm_hit || cmd_hit || exe_hit
}

/// 出错时的落尾：把 Kira 实际检测到的可执行文件路径给出来，便于手动处理。
pub(super) fn all_channels_failed(
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
pub(super) fn remove_detected_install(
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

    let mut msg = format!(
        "已清理 {} 处安装文件：{}",
        removed.len(),
        removed.join("、")
    );
    if !failed.is_empty() {
        msg.push_str(&format!(
            "；另有 {} 处未清理：{}",
            failed.len(),
            failed.join("；")
        ));
    }
    Ok(msg)
}
