//! sources responsibility.
use super::*;

/// 在克隆下来的仓库里定位真正的技能目录。
///
/// 此前直接把 clone 目录当技能装：当仓库根没有 SKILL.md（`skills/<name>` 布局、
/// 或多技能集合仓库）时，技能 id 会退化成临时目录名，之后更新 / 卸载就找不到同一个技能。
///
/// 0 个 → 报错（不是技能仓库）；1 个 → 直接用；多个 → 报错并列出候选，让用户用 `#子目录` 指定。
pub(super) fn find_skill_root(
    clone_dir: &std::path::Path,
    explicit_subdir: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(sub) = explicit_subdir {
        let candidate = clone_dir.join(sub);
        if candidate.join("SKILL.md").is_file() {
            return Ok(candidate);
        }
        return Err(format!("指定子目录里没有 SKILL.md: {}", sub));
    }

    if clone_dir.join("SKILL.md").is_file() {
        return Ok(clone_dir.to_path_buf());
    }

    let mut found: Vec<PathBuf> = Vec::new();
    let mut frontier: Vec<(PathBuf, usize)> = vec![(clone_dir.to_path_buf(), 0)];
    while let Some((dir, depth)) = frontier.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // 跳过版本控制目录与依赖目录
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            if path.join("SKILL.md").is_file() {
                found.push(path);
            } else if depth + 1 < SKILL_ROOT_MAX_DEPTH {
                frontier.push((path, depth + 1));
            }
        }
    }

    match found.len() {
        0 => Err("仓库里没有找到 SKILL.md（不是技能仓库？）".to_string()),
        1 => Ok(found.remove(0)),
        _ => {
            let mut names: Vec<String> = found
                .iter()
                .filter_map(|path| path.strip_prefix(clone_dir).ok())
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .collect();
            names.sort();
            Err(format!(
                "仓库里有 {} 个技能，请用「来源#子目录」指定要装哪个: {}",
                names.len(),
                names.join(", ")
            ))
        }
    }
}

/// 本次安装专用的临时克隆目录。
///
/// 此前两个安装入口共用 `_temp_skill_clone` 并在前后各自 `remove_dir_all`：
/// 重复点击或并发安装时，后一次会把前一次正在使用的克隆删掉。
pub(super) fn temp_clone_dir() -> PathBuf {
    let stamp = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0);
    get_data_dir().join(format!(
        "_temp_skill_clone_{}_{}",
        std::process::id(),
        stamp
    ))
}

// ─── 安装 / 卸载 ───

#[tauri::command]
pub fn install_skill(skill_dir: String) -> Result<(), String> {
    install_skill_from_dir(&skill_dir, None).map(|_| ())
}

/// 安装技能到仓库，返回技能 id。
///
/// `origin` = `(来源, commit sha)`：在线安装时传，用来支持后续的「有没有更新」比对；
/// 本地目录安装传 `None`（没有远端可取）。
pub(crate) fn install_skill_from_dir(
    skill_dir: &str,
    origin: Option<(&str, &str)>,
) -> Result<String, String> {
    let src = PathBuf::from(skill_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    let skill_md = src.join("SKILL.md");
    let folder_name = src
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        parse_skill_md(&content, &folder_name)
    } else {
        (folder_name.clone(), folder_name.clone())
    };

    let id = name.to_lowercase().replace(' ', "-");
    let store = skills_dir();
    let _ = fs::create_dir_all(&store);
    let dest_dir = store.join(&id);

    // 先复制到隐藏的 staging 目录，成功后再整体替换：
    // 复制失败（磁盘满 / 权限）时旧技能仍然完好，不会「新旧一起丢」。
    // staging 以 '.' 开头，不会被技能列表扫描到（见 get_skill_overview）。
    let staging = store.join(format!(".{}.staging", id));
    let _ = fs::remove_dir_all(&staging);
    copy_dir_recursive(&src, &staging)?;

    if dest_dir.exists() {
        let _ = fs::remove_dir_all(&dest_dir);
    }
    if let Err(move_error) = fs::rename(&staging, &dest_dir) {
        // 目标被占用 / 跨卷时 rename 可能失败：回退成再复制一次
        if let Err(copy_error) = copy_dir_recursive(&staging, &dest_dir) {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!(
                "安装技能失败（移动: {}；复制: {}）",
                move_error, copy_error
            ));
        }
        let _ = fs::remove_dir_all(&staging);
    }

    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != id);
    let (source, commit) = match origin {
        Some((s, c)) => (s.to_string(), c.to_string()),
        None => (String::new(), String::new()),
    };
    skills.skills.push(Skill {
        id: id.clone(),
        name,
        description,
        directory: dest_dir.to_string_lossy().to_string(),
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: if source.is_empty() {
            "local".to_string()
        } else {
            "online".to_string()
        },
        source,
        commit,
    });
    save_skills(&skills)?;
    Ok(id)
}

#[tauri::command]
pub fn uninstall_skill(skill_id: String) -> Result<(), String> {
    let store = skills_dir();
    let dir = store.join(&skill_id);

    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }

    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != skill_id);
    save_skills(&skills)
}

#[tauri::command]
pub fn get_skill_files(skill_id: String) -> Result<(String, Vec<SkillFile>), String> {
    let store = skills_dir();
    let dir = store.join(&skill_id);
    if !dir.exists() {
        return Err("技能目录不存在".to_string());
    }

    let skills = load_skills();
    let name = skills
        .skills
        .iter()
        .find(|s| s.id == skill_id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| skill_id.clone());

    let mut files = Vec::new();
    collect_skill_files(&dir, &dir, &mut files)?;
    Ok((name, files))
}

pub(super) fn collect_skill_files(
    base: &PathBuf,
    current: &PathBuf,
    files: &mut Vec<SkillFile>,
) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if path.is_dir() {
            collect_skill_files(base, &path, files)?;
        } else if path.is_file() {
            let contents = fs::read_to_string(&path).unwrap_or_default();
            files.push(SkillFile {
                path: rel,
                contents,
            });
        }
    }
    Ok(())
}

// ─── 在线安装 ───

#[tauri::command]
pub async fn install_skill_from_source(source: String) -> Result<(), String> {
    let src_trimmed = source.trim();
    if src_trimmed.is_empty() {
        return Err("来源不能为空".to_string());
    }

    // 支持 `owner/repo#子目录`：仓库里有多个技能时用它指定装哪个
    let (base, subdir) = split_source_subdir(src_trimmed);

    // 本地路径
    let local_path = PathBuf::from(&base);
    if local_path.exists() && local_path.is_dir() {
        let root = find_skill_root(&local_path, subdir.as_deref())?;
        return install_skill(root.to_string_lossy().to_string());
    }

    // Git URL 或 owner/repo
    let repo_url = if base.starts_with("http://") || base.starts_with("https://") {
        base.clone()
    } else if base.contains('/') && !base.contains('\\') {
        format!("https://github.com/{}", base)
    } else {
        return Err("无效的来源格式".to_string());
    };

    let temp_dir = temp_clone_dir();

    let mut cmd = tokio::process::Command::new("git");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = cmd
        .args(crate::commands::utils::with_git_long_paths(&[
            "clone", "--depth", "1", &repo_url,
        ]))
        .arg(&temp_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("git clone 失败: {}", e))?;

    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!(
            "git clone 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // 定位技能根：不再把整个 clone 目录当技能，否则技能 id 会退化成临时目录名
    let root = match find_skill_root(&temp_dir, subdir.as_deref()) {
        Ok(root) => root,
        Err(e) => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(e);
        }
    };
    let sha = rev_parse_head(&temp_dir).await.unwrap_or_default();
    let result = install_skill_from_dir(&root.to_string_lossy(), Some((&base, &sha))).map(|_| ());
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// 从在线源安装技能到仓库（已管理工具通过 junction 自动获得）
#[tauri::command]
pub async fn install_skill_from_online(
    app: tauri::AppHandle,
    source: String,
) -> Result<(), String> {
    let src_trimmed = source.trim();
    if src_trimmed.is_empty() {
        return Err("来源不能为空".to_string());
    }
    emit_install_progress(&app, "准备", 0, 0, "", "准备安装技能...");

    // 支持 `owner/repo#子目录`：仓库里有多个技能时用它指定装哪个
    let (base, subdir) = split_source_subdir(src_trimmed);

    // 本地路径
    let local_path = PathBuf::from(&base);
    if local_path.exists() && local_path.is_dir() {
        emit_install_progress(&app, "安装", 1, 1, "", "正在安装到仓库...");
        let root = find_skill_root(&local_path, subdir.as_deref())?;
        let result = install_skill(root.to_string_lossy().to_string());
        if result.is_ok() {
            emit_install_progress(&app, "完成", 1, 1, "", "安装完成！已管理工具自动获得新技能");
        }
        return result;
    }

    // Git URL 或 owner/repo
    let repo_url = if base.starts_with("http://") || base.starts_with("https://") {
        base.clone()
    } else if base.contains('/') && !base.contains('\\') {
        format!("https://github.com/{}", base)
    } else {
        return Err("无效的来源格式（需要 Git URL 或 owner/repo）".to_string());
    };

    // Git clone
    let temp_dir = temp_clone_dir();
    emit_install_progress(&app, "克隆", 0, 0, "", "正在克隆技能源仓库...");

    let mut cmd = tokio::process::Command::new("git");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = cmd
        .args(crate::commands::utils::with_git_long_paths(&[
            "clone", "--depth", "1", &repo_url,
        ]))
        .arg(&temp_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("git clone 失败: {}", e))?;

    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!(
            "git clone 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // 安装到仓库：先定位技能根，避免把整个仓库当技能（技能 id 会退化成临时目录名）
    emit_install_progress(&app, "安装", 1, 1, "", "正在安装到仓库...");
    let root = match find_skill_root(&temp_dir, subdir.as_deref()) {
        Ok(root) => root,
        Err(e) => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(e);
        }
    };
    let sha = rev_parse_head(&temp_dir).await.unwrap_or_default();
    let result = install_skill_from_dir(&root.to_string_lossy(), Some((&base, &sha))).map(|_| ());
    let _ = fs::remove_dir_all(&temp_dir);

    if result.is_ok() {
        emit_install_progress(&app, "完成", 1, 1, "", "安装完成！已管理工具自动获得新技能");
    }
    result
}

// ─── 预置来源与更新检测 ───

/// 取某个 git 仓库当前 HEAD 的 sha（失败返回 None，不阻断安装）。
pub(super) async fn rev_parse_head(repo_dir: &std::path::Path) -> Option<String> {
    let mut cmd = tokio::process::Command::new("git");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = cmd
        .args(["-C", &repo_dir.to_string_lossy(), "rev-parse", "HEAD"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

/// 把 `owner/repo` 形态的来源补成完整 URL（已是 URL 的原样返回）。
pub(super) fn source_to_url(source: &str) -> String {
    if source.starts_with("http://") || source.starts_with("https://") {
        source.to_string()
    } else {
        format!("https://github.com/{source}")
    }
}

/// 预置技能源（抄 CodexPlusPlus 的默认源清单）。
///
/// 它的默认 4 源正好覆盖「官方 curated + 另一个官方 + 社区合集 + 中文向社区」，
/// 直接照搬，用户不必自己去翻仓库地址。
pub(crate) const PRESET_SKILL_SOURCES: &[(&str, &str, &str)] = &[
    (
        "OpenAI 官方技能",
        "openai/skills",
        "Codex / ChatGPT 官方维护的 curated 技能",
    ),
    (
        "Anthropic 官方技能",
        "anthropics/skills",
        "Claude 官方技能合集",
    ),
    (
        "awesome-claude-skills",
        "ComposioHQ/awesome-claude-skills",
        "社区精选，覆盖面最广",
    ),
    ("myclaude", "cexll/myclaude", "社区实用技能，中文向"),
];

#[derive(Serialize, Clone, Debug)]
pub struct SkillSource {
    pub label: String,
    pub repo: String,
    pub hint: String,
}

/// 列出预置技能源（前端「在线」页直接展示，点一下即装）。
#[tauri::command]
pub fn get_skill_sources() -> Vec<SkillSource> {
    PRESET_SKILL_SOURCES
        .iter()
        .map(|(label, repo, hint)| SkillSource {
            label: label.to_string(),
            repo: repo.to_string(),
            hint: hint.to_string(),
        })
        .collect()
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateInfo {
    pub skill_id: String,
    pub name: String,
    pub source: String,
    pub current: String,
    pub latest: String,
}

/// 检查已装技能是否有更新（只查记录过来源的）。
///
/// 用 `git ls-remote <url> HEAD` 比对安装时记下的 sha：**不 clone、不需要 API token**。
/// （CodexPlusPlus 走 GitHub trees API 算子树的 SHA256，那个接口不带 token 会限流。）
#[tauri::command]
pub async fn check_skill_updates() -> Result<Vec<SkillUpdateInfo>, String> {
    let skills = load_skills();
    let mut updates = Vec::new();
    for skill in skills.skills.iter().filter(|s| !s.source.is_empty()) {
        let url = source_to_url(&skill.source);
        let mut cmd = tokio::process::Command::new("git");
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let out = cmd
            .args(["ls-remote", &url, "HEAD"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await;
        let Ok(out) = out else { continue };
        if !out.status.success() {
            continue;
        }
        let latest = String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        if latest.is_empty() || latest == skill.commit {
            continue;
        }
        updates.push(SkillUpdateInfo {
            skill_id: skill.id.clone(),
            name: skill.name.clone(),
            source: skill.source.clone(),
            current: skill.commit.clone(),
            latest,
        });
    }
    Ok(updates)
}

// ─── 旧数据迁移 ───

/// 一次性迁移：若旧仓库 ~/.any-version/skills 存在且有内容，合并到新仓库
#[tauri::command]
pub fn migrate_legacy_skills() -> Result<usize, String> {
    let old_store = get_data_dir().join("skills");
    let new_store = skills_dir();

    // 如果旧目录不存在或与新目录相同，无需迁移
    if !old_store.exists()
        || normalize_path(&old_store.to_string_lossy())
            == normalize_path(&new_store.to_string_lossy())
    {
        return Ok(0);
    }

    let _ = fs::create_dir_all(&new_store);
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(&old_store) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let dest = new_store.join(&name);
            if !dest.exists() {
                if copy_dir_recursive(&path, &dest).is_ok() {
                    count += 1;
                }
            }
        }
    }

    Ok(count)
}

// ─── 工具函数 ───

/// 标准化路径用于比较（统一大小写、去尾部分隔符、去除 Windows `\\?\` 前缀）
pub(crate) fn normalize_path(path: &str) -> String {
    let p = path
        .trim_end_matches('\\')
        .trim_end_matches('/')
        .to_lowercase();
    p.strip_prefix("\\\\?\\").unwrap_or(&p).to_string()
}

pub(super) fn copy_dir_recursive(src: &PathBuf, dest: &PathBuf) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest_path = dest.join(path.file_name().unwrap());
        if path.is_dir() {
            // 跳过 junction/symlink 子目录（避免循环）
            if is_junction(&path) {
                continue;
            }
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
