use super::config::load_ai_config;
use super::models::*;
use crate::commands::cache::create_junction;
use crate::commands::config::get_data_dir;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::Emitter;

// ─── 基础路径 ───

fn skills_path() -> PathBuf {
    get_data_dir().join("skills.json")
}

/// 默认技能仓库：Kira 数据目录下的 `skills` 子目录。
///
/// 刻意**不用** `~/.agents/skills`：那个目录是 claude-code / codex-cli / opencode /
/// cursor / cline / firebender 的原生扫描目录，放进去的技能会对**所有**项目全局生效，
/// 每次会话都把全部技能的 name + description 注入上下文（正文虽是按需读取，但描述常驻）。
/// 放进 Kira 自己的数据目录后，只有显式安装给某个项目的技能才会生效。
pub(crate) fn default_skills_dir() -> PathBuf {
    get_data_dir().join("skills")
}

/// 取 HOME 目录（兼容 Windows `USERPROFILE` / Unix `HOME`）
fn home_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default(),
    )
}

/// 把用户填的路径解析成绝对路径：
/// - 空字符串 → 默认「数据目录下的 skills」（见 `default_skills_dir`）；
/// - 支持 `~` 前缀与 Windows `%VAR%` 占位符展开；
/// - 相对路径以数据目录为基准。
pub(crate) fn resolve_skills_dir(cfg_value: &str) -> PathBuf {
    let cfg_value = cfg_value.trim();
    if cfg_value.is_empty() {
        return default_skills_dir();
    }
    let home = home_dir();
    let with_tilde = if cfg_value.starts_with("~/") {
        home.join(&cfg_value[2..]).to_string_lossy().to_string()
    } else if cfg_value.starts_with('~') && cfg_value.len() > 1 {
        home.join(&cfg_value[1..]).to_string_lossy().to_string()
    } else {
        cfg_value.to_string()
    };
    let resolved = with_tilde
        .replace("%USERPROFILE%", &home.to_string_lossy())
        .replace(
            "%LOCALAPPDATA%",
            &std::env::var("LOCALAPPDATA").unwrap_or_default(),
        )
        .replace("%APPDATA%", &std::env::var("APPDATA").unwrap_or_default())
        .replace(
            "%PROGRAMFILES%",
            &std::env::var("ProgramFiles").unwrap_or_default(),
        );
    let mut p = PathBuf::from(resolved);
    if p.is_relative() {
        p = get_data_dir().join(&p);
    }
    p
}

/// 当前技能库目录：`AiConfig.skills_dir` 为空时用默认（数据目录下的 skills）。
///
/// 用户可自定义（设置入口在技能管理页），但**默认值刻意避开** `~/.agents/skills`——
/// 那是原生扫描目录，放进去的技能会对所有项目全局生效。详见 `default_skills_dir`。
pub(crate) fn skills_dir() -> PathBuf {
    resolve_skills_dir(&load_ai_config().skills_dir)
}

/// 当前生效的技能库目录（前端要如实显示「技能装到哪了」）。
#[tauri::command]
pub fn get_skills_dir() -> String {
    skills_dir().to_string_lossy().to_string()
}

/// 切换技能库位置的结果。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillsDirChanged {
    /// 切换后的实际路径
    dir: String,
    /// 搬过去的技能数（0 = 原来就是空的，没东西可搬）
    moved: usize,
    /// 重建的项目内链接数（junction 记绝对路径，不重建就全断）
    relinked: usize,
    /// 位置没变时的提示（此时不写配置、不迁移）
    unchanged: bool,
}

/// 设置技能库位置。传空字符串 = 恢复默认。
///
/// 改位置会把**已装技能整体搬过去**：项目里那些 junction 指向的是技能库里的实体目录，
/// 不跟着搬就会全部变成悬空链接。所以这里必须同时做两件事——迁移实体 + 改写
/// `skills.json` 里记录的 `directory`，顺序不能反（先搬后写，失败时配置还是旧的）。
#[tauri::command]
pub fn set_skills_dir(path: String) -> Result<SkillsDirChanged, String> {
    let old_dir = skills_dir();
    let new_dir = resolve_skills_dir(&path);

    if normalize_path(&old_dir.to_string_lossy()) == normalize_path(&new_dir.to_string_lossy()) {
        return Ok(SkillsDirChanged {
            dir: new_dir.to_string_lossy().to_string(),
            moved: 0,
            relinked: 0,
            unchanged: true,
        });
    }
    // junction 不能指向网络路径，提前拦一道免得搬完才报错
    if new_dir.to_string_lossy().starts_with(r"\\") {
        return Err("技能库不能放在网络路径（UNC）上：项目里的目录联接无法指向它".to_string());
    }

    let mut cfg = load_ai_config();
    cfg.skills_dir = path.trim().to_string();
    super::config::save_ai_config_to_file(&cfg)?;

    let mut skills = load_skills();
    let report = do_migrate_skills(
        &old_dir.to_string_lossy(),
        &new_dir.to_string_lossy(),
        &skills.skills,
        None,
    );
    // 技能搬走了，清单里的 directory 也要跟着改，否则更新检测会指向旧位置
    for s in skills.skills.iter_mut() {
        let old_path = PathBuf::from(&s.directory);
        if let Ok(rel) = old_path.strip_prefix(&old_dir) {
            s.directory = new_dir.join(rel).to_string_lossy().to_string();
        }
    }
    save_skills(&skills)?;

    // **必须重建项目里的 junction**：目录联接记的是**绝对路径**，技能搬走后就全成了
    // 悬空链接 —— 不重建的话，改一次路径等于把所有项目里已装的技能全部弄断。
    let relinked = rebuild_project_links(&old_dir, &new_dir);

    Ok(SkillsDirChanged {
        dir: new_dir.to_string_lossy().to_string(),
        moved: report.moved_count,
        relinked,
        unchanged: false,
    })
}

/// 把各项目里指向旧库的 junction 重建到新库（junction 只能用绝对路径，无法原地跟随）。
/// 返回重建成功的数量。
fn rebuild_project_links(old_dir: &Path, new_dir: &Path) -> usize {
    let mut rebuilt = 0;
    for p in load_projects().projects {
        let dir = PathBuf::from(&p.path).join(".agents").join("skills");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let link = entry.path();
            let is_link = fs::symlink_metadata(&link)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_link {
                continue;
            }
            let Ok(target) = fs::read_link(&link) else {
                continue;
            };
            // 只动指向旧库的；用户自己建的、指向别处的链接不碰
            let Ok(rel) = target.strip_prefix(old_dir) else {
                continue;
            };
            let new_target = new_dir.join(rel);
            if !new_target.is_dir() {
                continue;
            }
            // 先删链接本身再重建（remove_dir 不会跟随删除目标内容）
            if fs::remove_dir(&link).is_err() {
                continue;
            }
            if create_junction(&link, &new_target).is_ok() {
                rebuilt += 1;
            }
        }
    }
    rebuilt
}

/// 把技能从旧库整体搬到新库，逐个技能搬（目录改名兜底，跨卷时先复制再删）。
pub(crate) fn do_migrate_skills(
    old_dir: &str,
    new_dir: &str,
    skills: &[Skill],
    _app_handle: Option<&tauri::AppHandle>,
) -> SkillMigrateResult {
    let old_path = PathBuf::from(old_dir);
    let new_path = PathBuf::from(new_dir);
    let mut result = SkillMigrateResult {
        moved_count: 0,
        errors: Vec::new(),
    };
    if old_path == new_path {
        return result;
    }
    if !old_path.exists() {
        // 原来就没有技能，无可搬
        return result;
    }
    if fs::create_dir_all(&new_path).is_err() {
        result
            .errors
            .push(format!("创建 {} 失败", new_path.display()));
        return result;
    }
    for s in skills {
        let from = PathBuf::from(&s.directory);
        if from.strip_prefix(&old_path).is_err() {
            continue; // 不在旧库里，跳过
        }
        let rel = from.strip_prefix(&old_path).unwrap();
        let to = new_path.join(rel);
        if to.exists() {
            continue; // 目标已有同名，不覆盖
        }
        if let Some(parent) = to.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let ok = fs::rename(&from, &to).is_ok() || {
            // 跨卷时 rename 会失败，退化成复制
            copy_dir(&from, &to).is_ok() && fs::remove_dir_all(&from).is_ok()
        };
        if ok {
            result.moved_count += 1;
        } else {
            result.errors.push(format!("搬运 {} 失败", s.name));
        }
    }
    result
}

/// 技能目录迁移的结果
pub struct SkillMigrateResult {
    pub moved_count: usize,
    pub errors: Vec<String>,
}

fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let d = dest.join(p.file_name().unwrap());
        if p.is_dir() {
            // 跳过链接子目录，避免顺着链接把别处内容复制进来
            if fs::symlink_metadata(&p)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                continue;
            }
            copy_dir(&p, &d)?;
        } else {
            fs::copy(&p, &d).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ─── Manifest 读写 ───

pub(crate) fn load_skills() -> SkillsFile {
    let path = skills_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(skills) = serde_json::from_str::<SkillsFile>(&data) {
                return skills;
            }
        }
    }
    SkillsFile::default()
}

pub(crate) fn save_skills(skills: &SkillsFile) -> Result<(), String> {
    let data = serde_json::to_string_pretty(skills).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&skills_path(), data.as_bytes())
}

mod projects;
pub use projects::*;

mod project_install;
pub use project_install::*;

mod project_scan;
pub use project_scan::*;

mod library;
pub use library::*;

mod sources;
pub use sources::*;

#[cfg(test)]
mod tests;

mod builtin;
pub use builtin::*;
