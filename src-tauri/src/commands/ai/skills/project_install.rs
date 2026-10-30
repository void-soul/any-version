//! project install responsibility.
use super::*;

// ─── 安装到项目（junction） ───
//
// 安装 = 在 `<项目>/<扫描目录>/skills/<技能id>` 建一个指向技能库该技能的目录联接。
// 卸载 = 只删链接本身。两者都不在项目目录里放任何实体文件，所以「装给项目」
// 不会让技能副本散落到各处，技能库一改，所有项目立刻同步。

/// 技能在各 AI 工具里的**项目级扫描目录**（相对项目根）。
///
/// 取自 CodeBuddy 服务端下发的 `skillScanDirs`（实测所有日志完全一致：
/// `.codebuddy, .workbuddy, .claude, .agents, .openclaw`）。安装时按用户勾选，
/// 在**每个勾选的目录下各建一个 junction 指向库里同一份实体**—— 不复制内容，
/// 所以库一改，所有目录同时生效。
///
/// `.codebuddy` 与 `.agents` 身份不同：前者是 CodeBuddy 的安装 scope（技能管理页能看见、
/// 能勾选），后者是跨工具标准目录（模型能调用，但管理页看不见）。两个都写，正好补上这条鸿沟。
pub const SKILL_SCAN_DIRS: &[&str] = &[
    ".agents",
    ".codebuddy",
    ".claude",
    ".workbuddy",
    ".openclaw",
];

/// 解析项目记录 → 项目根目录，顺带校验项目目录还在。
/// 清单路径可注入（单测用；否则测试会写用户真实的数据目录）。
pub(super) fn resolve_project_root_in(file: &Path, project_id: &str) -> Result<PathBuf, String> {
    let f = load_projects_from(file);
    let p = f
        .projects
        .iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| "项目不存在".to_string())?;
    if !Path::new(&p.path).is_dir() {
        return Err(format!(
            "项目目录不存在或已被移动：{}（请移除后重新添加）",
            p.path
        ));
    }
    Ok(PathBuf::from(&p.path))
}

/// 项目下全部扫描目录的 skills 路径（目录不存在也照样返回，调用方按需创建）。
pub(super) fn project_skill_dirs_in(file: &Path, project_id: &str) -> Result<Vec<PathBuf>, String> {
    let root = resolve_project_root_in(file, project_id)?;
    Ok(SKILL_SCAN_DIRS
        .iter()
        .map(|d| root.join(d).join("skills"))
        .collect())
}

/// 用户勾选的扫描目录；空/缺省 = 全部。未知名字直接忽略（宁可少建一个目录，
/// 也不接受任意外部路径 —— 这里是 junction 的写入点）。
pub(super) fn selected_dirs(dirs: Option<&Vec<String>>) -> Vec<String> {
    let picked: Vec<&str> = match dirs {
        Some(list) if !list.is_empty() => SKILL_SCAN_DIRS
            .iter()
            .copied()
            .filter(|d| list.iter().any(|x| x == d))
            .collect(),
        // 空 / 缺省 = 全部
        _ => SKILL_SCAN_DIRS.to_vec(),
    };
    picked.into_iter().map(|s| s.to_string()).collect()
}

/// 这个路径是不是「指向技能库内某个技能」的 junction？返回那个技能 id。
///
/// 用 `read_link` 解析真实目标，而不是「目录名在技能库里存在就算」——
/// 后者会把用户自己建的同名链接、以及指错了位置的链接误判成已安装。
/// 悬空 junction（目标已删）返回 None：它是链接，但不再指向任何现存技能。
pub(super) fn junction_target_skill(link: &Path, store: &Path) -> Option<String> {
    // symlink_metadata 而非 metadata：读链接自己而不是跟随后的目标（悬空时会失败）
    if !fs::symlink_metadata(link)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return None;
    }
    let raw = fs::read_link(link).ok()?;
    // 相对目标按链接所在目录解析（mklink /J 总给绝对路径，但手工建的可能是相对）
    let target = if raw.is_absolute() {
        raw
    } else {
        link.parent()?.join(raw)
    };
    // 两边都 canonicalize：展开 8.3 短名（%TEMP% 常是 ADMINI~1 形式），
    // 否则「技能库记的是长名、链接里存的是短名」就会匹配不上。
    // canonicalize 对悬空目标会失败，退回原路径即可（后续 is_dir() 会判为 broken）。
    let t = fs::canonicalize(&target).unwrap_or(target);
    let s = fs::canonicalize(store).unwrap_or_else(|_| store.to_path_buf());
    // 自己归一分隔符：normalize_path 只统一大小写与**末尾**分隔符，中间的反斜杠不管，
    // 直接拿它做 prefix 匹配会永远失败。
    let slashes = |p: &str| p.replace('\\', "/").to_lowercase();
    let t = slashes(&t.to_string_lossy());
    let s = slashes(&s.to_string_lossy());
    let prefix = format!("{}/", s.trim_end_matches('/'));
    let id = t.strip_prefix(&prefix)?.split('/').next()?.to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// 一次安装的逐目录结果。
///
/// 刻意**不是**一个总成功/失败：5 个目录里 `.openclaw` 建失败而其余成功时，
/// 报「装好了」是假的 —— 用户会在那个目录里找不到技能，又不知道发生了什么。
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    /// 成功建立/已存在的目录名
    pub linked: Vec<String>,
    /// `目录: 原因`
    pub failed: Vec<String>,
    /// 被覆盖的真实目录（项目自带的同名技能）——UI 必须先让用户确认再装
    pub replaced: Vec<String>,
}

/// 把技能装到项目（在被勾选的每个扫描目录下各建一个 junction）。
#[tauri::command]
pub fn install_skill_to_project(
    skill_id: String,
    project_id: String,
    dirs: Option<Vec<String>>,
    gitignore: Option<bool>,
) -> Result<InstallOutcome, String> {
    let outcome = install_into(
        &skills_dir(),
        &projects_path(),
        &skill_id,
        &project_id,
        dirs.as_ref(),
    )?;
    // 只在真的装了东西时才动 .gitignore：否则「只是打开面板」也会留下规则
    if gitignore.unwrap_or(false) && !outcome.linked.is_empty() {
        if let Err(e) = ensure_skill_gitignore(&projects_path(), &project_id, &outcome.linked) {
            eprintln!("[skills] 维护 .gitignore 失败: {}", e);
        }
    }
    Ok(outcome)
}

pub(super) fn install_into(
    store: &Path,
    projects_file: &Path,
    skill_id: &str,
    project_id: &str,
    dirs: Option<&Vec<String>>,
) -> Result<InstallOutcome, String> {
    let skill_dir = store.join(skill_id);
    if !skill_dir.is_dir() {
        return Err(format!("技能库中不存在技能「{}」", skill_id));
    }
    // junction 跨本地卷可行，但不支持网络路径（UNC）：先拦一道，避免 mklink 报错难懂
    if skill_dir.to_string_lossy().starts_with(r"\\") {
        return Err(
            "技能库位于网络路径（UNC）时无法创建目录联接，请把技能库移到本地磁盘".to_string(),
        );
    }
    let all = project_skill_dirs_in(projects_file, project_id)?;
    let want = selected_dirs(dirs);
    let mut out = InstallOutcome::default();
    for dir_name in want {
        let Some(skills_root) = all
            .iter()
            .zip(SKILL_SCAN_DIRS.iter())
            .find(|(p, n)| **n == dir_name)
            .map(|(p, _)| p.clone())
        else {
            continue;
        };
        if let Err(e) = fs::create_dir_all(&skills_root) {
            out.failed.push(format!("{}: 创建目录失败 {}", dir_name, e));
            continue;
        }
        let link = skills_root.join(skill_id);
        // 三种前置状态：真实目录（覆盖）/ 已正确链接（幂等）/ 指向别处或悬空（重建）
        if let Ok(md) = fs::symlink_metadata(&link) {
            if !md.file_type().is_symlink() {
                // 真实目录 = 项目自带的同名技能：**允许覆盖**，先整个删掉再建链接。
                // 不可逆操作，所以 UI 在应用前必须把「会被替换的真实目录」列给用户确认。
                if let Err(e) = fs::remove_dir_all(&link) {
                    out.failed
                        .push(format!("{}: 无法替换已存在的目录 {}", dir_name, e));
                    continue;
                }
                out.replaced.push(dir_name.clone());
            } else if junction_target_skill(&link, store).as_deref() == Some(skill_id) {
                // 已正确链接 = 幂等成功
                out.linked.push(dir_name);
                continue;
            }
            // 指向别处 / 悬空：交给 create_junction 删链接后重建
        }
        match create_junction(&link, &skill_dir) {
            Ok(()) => out.linked.push(dir_name),
            Err(e) => out.failed.push(format!("{}: {}", dir_name, e)),
        }
    }
    if out.linked.is_empty() && out.failed.is_empty() {
        return Err("没有勾选任何技能目录".to_string());
    }
    Ok(out)
}

/// 在项目的 `.gitignore` 里确保忽略这些技能目录（幂等）。
///
/// 为什么必须忽略：junction 会被 git 当成符号链接或直接跟随进内容，
/// 不忽略的话「装给项目」会把技能库的内容拖进仓库（而且是 5 份目录里各一份）。
pub(super) fn ensure_skill_gitignore(
    projects_file: &Path,
    project_id: &str,
    linked: &[String],
) -> Result<(), String> {
    let root = resolve_project_root_in(projects_file, project_id)?;
    let path = root.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let already_ignored: Vec<String> = existing
        .lines()
        .map(|l| l.trim().trim_end_matches('/').to_string())
        .collect();
    let mut added: Vec<String> = Vec::new();
    for d in linked {
        let pattern = format!("{}/skills/", d);
        if already_ignored
            .iter()
            .any(|l| l == &pattern || *l == format!("{}/skills", d))
        {
            continue;
        }
        added.push(pattern);
    }
    if added.is_empty() {
        return Ok(());
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.contains(KIRA_GITIGNORE_BEGIN) {
        out.push_str(KIRA_GITIGNORE_BEGIN);
        out.push('\n');
    }
    for p in added {
        out.push_str(&p);
        out.push('\n');
    }
    if !out.contains(KIRA_GITIGNORE_END) {
        out.push_str(KIRA_GITIGNORE_END);
        out.push('\n');
    }
    fs::write(&path, out).map_err(|e| format!("写入 {} 失败: {}", path.display(), e))
}

pub(super) const KIRA_GITIGNORE_BEGIN: &str = "# >>> kira skills >>>";
pub(super) const KIRA_GITIGNORE_END: &str = "# <<< kira skills <<<";

/// 从项目卸载技能：**所有扫描目录**里的链接都清掉。
///
/// - **链接**（指向技能库）：只删链接本身。Windows 上删 junction 必须用 remove_dir ——
///   若用会跟随删除的操作，删掉的会是**技能库里的实体技能内容**。这里刻意不用 remove_dir_all。
/// - **真实目录**（项目自带的技能）：按用户要求与链接同等对待，整个删除。
///   不可逆，UI 必须先把「会被删除的真实目录」列出来让用户确认。
/// 卸载技能。`dir_name` 给定时**只从这一个扫描目录**卸载，给 `None` 时从全部目录卸载。
///
/// 单目录卸载是主流程：一个技能常同时装在 `.agents` 与 `.codebuddy`，用户要能逐个
/// 去掉，而不是只有「全卸」和「都不卸」两个极端。全部目录卸载保留给脚本化调用。
#[tauri::command]
pub fn remove_skill_from_project(
    skill_id: String,
    project_id: String,
    dir_name: Option<String>,
) -> Result<Vec<String>, String> {
    uninstall_from(
        &projects_path(),
        &skill_id,
        &project_id,
        dir_name.as_deref(),
    )
}

/// 目录是否**真的空**（没有任何条目）。
///
/// 只删空目录这条是硬约束：`.codebuddy` / `.agents` 这类目录里常驻着别的东西
/// （`skills.json`、settings、用户自己的其它文件）。批量卸载后如果无脑删掉整个
/// 目录，用户的东西就没了 —— 而且这类删除不可逆。
pub(super) fn dir_is_empty(path: &Path) -> bool {
    fs::read_dir(path)
        .map(|mut it| it.next().is_none())
        .unwrap_or(false)
}

/// 技能卸载后清理空掉的壳目录：`skills/` 空了就删，`skills` 的父目录（`.agents` 等）
/// 也空了一并删。
///
/// 逐级向上且**每一级都重新判空**：`.codebuddy/skills` 空了不代表 `.codebuddy` 空
/// （可能有 settings.json），所以每层都要单独确认。
///
/// **刻意不返回被删的目录列表**：调用方那个 `removed` 的语义是「哪些扫描目录里的
/// 链接被清掉了」，把目录清理信息塞进去会破坏契约 —— 前端拿它计数就会把
/// 「3 个目录」数成 9。这个错误真的发生过，是既有测试抓到的。
pub(super) fn prune_empty_skill_dirs(project_root: &Path, scan_dir_name: &str) {
    let scan_dir = project_root.join(scan_dir_name);
    let skills_dir = scan_dir.join("skills");
    if dir_is_empty(&skills_dir) && fs::remove_dir(&skills_dir).is_ok() {
        // skills/ 没了，父目录可能也空了（比如这个项目只装过这一个技能）
        if dir_is_empty(&scan_dir) {
            let _ = fs::remove_dir(&scan_dir);
        }
    }
}

pub(super) fn uninstall_from(
    projects_file: &Path,
    skill_id: &str,
    project_id: &str,
    only_dir: Option<&str>,
) -> Result<Vec<String>, String> {
    // 先校验目录名合法：传进来一个任意字符串就让它拼进路径，等于开了条删任意目录的路
    if let Some(d) = only_dir {
        if !SKILL_SCAN_DIRS.contains(&d) {
            return Err(format!(
                "{} 不是技能扫描目录（允许 {}）",
                d,
                SKILL_SCAN_DIRS.join(" / ")
            ));
        }
    }
    let mut removed: Vec<String> = Vec::new();
    let project_root = resolve_project_root_in(projects_file, project_id)?;
    for (dir_name, dir) in SKILL_SCAN_DIRS
        .iter()
        .zip(project_skill_dirs_in(projects_file, project_id)?)
    {
        if let Some(only) = only_dir {
            if *dir_name != only {
                continue;
            }
        }
        let link = dir.join(skill_id);
        let Ok(md) = fs::symlink_metadata(&link) else {
            continue;
        };
        if md.file_type().is_symlink() {
            if fs::remove_dir(&link).is_err() {
                // 占用 / 权限等情况下 remove_dir 可能失败，rmdir 同样只删链接本身
                fs::remove_file(&link).map_err(|e| format!("卸载失败: {}", e))?;
            }
        } else {
            fs::remove_dir_all(&link).map_err(|e| format!("卸载失败 {}: {}", link.display(), e))?;
        }
        removed.push((*dir_name).to_string());
        // 只有真的清掉了这个目录里的技能，才去看它空没空
        prune_empty_skill_dirs(&project_root, dir_name);
    }
    if removed.is_empty() {
        return Err(match only_dir {
            Some(d) => format!("项目的 {} 目录下未安装技能「{}」", d, skill_id),
            None => format!("该项目未安装技能「{}」", skill_id),
        });
    }
    Ok(removed)
}
