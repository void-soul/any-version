use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use tauri::Emitter;
use crate::commands::ai_registry::registry;
use crate::commands::config::get_base_dir;
use crate::commands::cache::create_junction;
use super::models::*;


fn skills_path() -> PathBuf {
    get_base_dir().join("skills.json")
}
fn skills_dir() -> PathBuf {
    // 使用 ~/.agents/skills 作为 canonical 目录（与 skills.sh 规范一致）
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let home = if home.as_os_str().is_empty() {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    } else {
        home
    };
    home.join(".agents").join("skills")
}

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
    let path = skills_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(skills).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

// ─── 技能管理 ───

#[tauri::command]
pub fn get_skills() -> Result<Vec<Skill>, String> {
    Ok(load_skills().skills)
}

#[tauri::command]
pub fn install_skill(skill_dir: String) -> Result<(), String> {
    let src = PathBuf::from(&skill_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 从 SKILL.md 读取名称
    let skill_md = src.join("SKILL.md");
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        let desc = content.lines().next().unwrap_or("").trim_start_matches('#').trim().to_string();
        let folder_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (folder_name, desc)
    } else {
        let n = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (n.clone(), n)
    };

    let id = name.to_lowercase().replace(' ', "-");
    let dest_dir = skills_dir().join(&id);

    // 如果已存在则删除
    if dest_dir.exists() {
        let _ = fs::remove_dir_all(&dest_dir);
    }
    copy_dir_recursive(&src, &dest_dir)?;

    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != id);
    skills.skills.push(Skill {
        id: id.clone(),
        name: name.clone(),
        description,
        directory: dest_dir.to_string_lossy().to_string(),
        enabled_tools: vec![],
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: "local".to_string(),
    });
    save_skills(&skills)
}

#[tauri::command]
pub fn uninstall_skill(skill_id: String) -> Result<(), String> {
    let mut skills = load_skills();
    if let Some(pos) = skills.skills.iter().position(|s| s.id == skill_id) {
        let dir = skills.skills[pos].directory.clone();
        if !dir.is_empty() {
            let _ = fs::remove_dir_all(&dir);
        }
        skills.skills.remove(pos);
    }
    save_skills(&skills)
}

#[tauri::command]
pub fn toggle_skill_tool(skill_id: String, tool_id: String, enabled: bool) -> Result<(), String> {
    let mut skills = load_skills();
    if let Some(skill) = skills.skills.iter_mut().find(|s| s.id == skill_id) {
        if enabled {
            if !skill.enabled_tools.contains(&tool_id) {
                skill.enabled_tools.push(tool_id);
            }
        } else {
            skill.enabled_tools.retain(|t| t != &tool_id);
        }
    } else {
        return Err("技能不存在".to_string());
    }
    save_skills(&skills)
}

#[tauri::command]
pub fn get_skill_files(skill_id: String) -> Result<(String, Vec<SkillFile>), String> {
    let skills = load_skills();
    let skill = skills.skills.iter().find(|s| s.id == skill_id).ok_or("技能不存在")?;
    let dir = PathBuf::from(&skill.directory);
    if !dir.exists() {
        return Err("技能目录不存在".to_string());
    }
    let mut files = Vec::new();
    collect_skill_files(&dir, &dir, &mut files)?;
    Ok((skill.name.clone(), files))
}

fn collect_skill_files(base: &PathBuf, current: &PathBuf, files: &mut Vec<SkillFile>) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();
        if path.is_dir() {
            collect_skill_files(base, &path, files)?;
        } else if path.is_file() {
            let contents = fs::read_to_string(&path).unwrap_or_default();
            files.push(SkillFile { path: rel, contents });
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn install_skill_from_source(source: String) -> Result<(), String> {
    let src_trimmed = source.trim();
    if src_trimmed.is_empty() {
        return Err("来源不能为空".to_string());
    }

    // 本地路径
    let local_path = PathBuf::from(src_trimmed);
    if local_path.exists() && local_path.is_dir() {
        return install_skill(local_path.to_string_lossy().to_string());
    }

    // Git URL 或 owner/repo
    let repo_url = if src_trimmed.starts_with("http://") || src_trimmed.starts_with("https://") {
        src_trimmed.to_string()
    } else if src_trimmed.contains('/') && !src_trimmed.contains('\\') {
        format!("https://github.com/{}", src_trimmed)
    } else {
        return Err("无效的来源格式".to_string());
    };

    let temp_dir = get_base_dir().join("_temp_skill_clone");
    let _ = fs::remove_dir_all(&temp_dir);

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo_url])
        .arg(&temp_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("git clone 失败: {}", e))?;

    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!("git clone 失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let result = install_skill(temp_dir.to_string_lossy().to_string());
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

#[tauri::command]
pub fn scan_existing_skills() -> Result<Vec<ScannedSkill>, String> {
    let mut results: Vec<ScannedSkill> = Vec::new();

    // 从 skills-scan.json 驱动扫描目录列表
    let scan_dirs = registry().get_skill_scan_dirs();

    let mut seen = std::collections::HashSet::new();
    for (base_dir, location_label) in &scan_dirs {
        if !base_dir.exists() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let skill_md = path.join("SKILL.md");
                let description = if skill_md.exists() {
                    fs::read_to_string(&skill_md).unwrap_or_default()
                        .lines().next().unwrap_or("")
                        .trim_start_matches('#').trim().to_string()
                } else {
                    String::new()
                };

                let full_path = path.to_string_lossy().to_string();
                if seen.contains(&full_path) {
                    // 已扫描过（通过前面的 .agents/skills），追加位置标签
                    if let Some(existing) = results.iter_mut().find(|s| s.full_path == full_path) {
                        let loc = location_label.to_string();
                        if !existing.found_in.contains(&loc) {
                            existing.found_in.push(loc);
                        }
                    }
                    continue;
                }
                seen.insert(full_path.clone());

                let is_symlink = path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);

                results.push(ScannedSkill {
                    name: name.clone(),
                    description,
                    directory: name,
                    full_path,
                    found_in: vec![location_label.to_string()],
                    is_symlink,
                });
            }
        }
    }
    Ok(results)
}

#[tauri::command]
pub fn import_existing_skill(skill_path: String) -> Result<(), String> {
    install_skill(skill_path)
}
// ─── 技能目录迁移 ───

/// 技能迁移进度
#[derive(Serialize, Clone, Debug)]
pub struct SkillMigrateProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub skill_name: String,
}

/// 技能迁移结果
#[derive(Serialize, Clone, Debug)]
pub struct SkillMigrateResult {
    pub moved_count: usize,
    pub rebuilt_junctions: usize,
    pub errors: Vec<String>,
}

/// 执行技能目录迁移：移动文件 + 重建 JUNCTION
pub(crate) fn do_migrate_skills(
    old_dir: &str,
    new_dir: &str,
    skills: &[Skill],
    app_handle: Option<&tauri::AppHandle>,
) -> SkillMigrateResult {
    let old_path = PathBuf::from(old_dir);
    let new_path = PathBuf::from(new_dir);

    let mut result = SkillMigrateResult {
        moved_count: 0,
        rebuilt_junctions: 0,
        errors: Vec::new(),
    };

    let emit_progress = |stage: &str, current: usize, total: usize, skill_name: &str| {
        if let Some(handle) = app_handle {
            let _ = handle.emit("skill-migrate-progress", SkillMigrateProgress {
                stage: stage.to_string(),
                current,
                total,
                skill_name: skill_name.to_string(),
            });
        }
    };

    // 确保新目录存在
    if let Err(e) = fs::create_dir_all(&new_path) {
        result.errors.push(format!("创建新目录失败: {}", e));
        return result;
    }

    let total = skills.len();

    for (i, skill) in skills.iter().enumerate() {
        let skill_id = &skill.id;
        emit_progress("移动技能", i + 1, total, &skill.name);

        // 移动技能目录：old_skills_dir/skill_id -> new_skills_dir/skill_id
        let old_skill_dir = old_path.join(skill_id);
        let new_skill_dir = new_path.join(skill_id);

        if old_skill_dir.exists() && old_skill_dir != new_skill_dir {
            if new_skill_dir.exists() {
                let _ = fs::remove_dir_all(&new_skill_dir);
            }
            match fs::rename(&old_skill_dir, &new_skill_dir) {
                Ok(()) => {
                    result.moved_count += 1;
                }
                Err(e) => {
                    // rename 失败时尝试拷贝
                    if let Err(e2) = copy_dir_recursive(&old_skill_dir, &new_skill_dir) {
                        result.errors.push(format!("迁移 {} 失败: {} -> {}", skill.name, e, e2));
                        continue;
                    } else {
                        let _ = fs::remove_dir_all(&old_skill_dir);
                        result.moved_count += 1;
                    }
                }
            }
        } else if !old_skill_dir.exists() && new_skill_dir.exists() {
            // 已在新位置，跳过
            continue;
        } else if !old_skill_dir.exists() && !new_skill_dir.exists() {
            continue;
        }

        // 重建 JUNCTION 链接
        if !skill.enabled_tools.is_empty() {
            emit_progress("重建链接", i + 1, total, &skill.name);

            // 由 registry JSON 配置驱动的路径映射
            let tool_skill_dirs: Vec<(String, PathBuf)> = skill.enabled_tools.iter().map(|t| {
                (t.clone(), registry().resolve_skill_junction_target(t, skill_id))
            }).collect();

            for (_tool_id, tool_dir) in &tool_skill_dirs {
                if let Some(parent) = tool_dir.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if tool_dir.exists() {
                    let is_junction = fs::symlink_metadata(tool_dir)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                    if is_junction {
                        let _ = fs::remove_dir(tool_dir);
                    } else {
                        let _ = fs::remove_dir_all(tool_dir);
                    }
                }
                if let Err(e) = create_ai_junction(tool_dir, &new_skill_dir) {
                    result.errors.push(format!("JUNCTION 失败 for {}: {}", skill.name, e));
                } else {
                    result.rebuilt_junctions += 1;
                }
            }
        }
    }

    emit_progress("完成", total, total, "");
    result
}

/// 标准化路径用于比较
pub(crate) fn normalize_path(path: &str) -> String {
    path.trim_end_matches('\\').trim_end_matches('/').to_lowercase()
}

fn create_ai_junction(link_path: &PathBuf, target_path: &PathBuf) -> Result<(), String> {
    create_junction(link_path, target_path)
}
// ─── Skills.sh 本地安装集成 ───

/// 从在线路径安装 skill：clone 到 anyversion 核心 skill 仓库，再通过 JUNCTION 链接给各工具
#[tauri::command]
pub async fn install_skill_from_online(
    source: String,
    target_tools: Vec<String>,
) -> Result<(), String> {
    let src_trimmed = source.trim();
    if src_trimmed.is_empty() {
        return Err("来源不能为空".to_string());
    }
    if target_tools.is_empty() {
        return Err("请至少选择一个目标工具".to_string());
    }

    // 解析为 Git URL
    let repo_url = if src_trimmed.starts_with("http://") || src_trimmed.starts_with("https://") {
        src_trimmed.to_string()
    } else if src_trimmed.contains('/') && !src_trimmed.contains('\\') {
        format!("https://github.com/{}", src_trimmed)
    } else {
        // 本地路径
        let local_path = PathBuf::from(src_trimmed);
        if local_path.exists() && local_path.is_dir() {
            return install_skill_with_junctions(local_path.to_string_lossy().to_string(), &target_tools);
        }
        return Err("无效的来源格式（需要 Git URL 或 owner/repo）".to_string());
    };

    // 1. Git clone 到临时目录
    let temp_dir = get_base_dir().join("_temp_skill_clone");
    let _ = fs::remove_dir_all(&temp_dir);

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo_url])
        .arg(&temp_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("git clone 失败: {}", e))?;

    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!("git clone 失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    // 2. 安装到核心 skill 仓库 + 创建 JUNCTION
    let result = install_skill_with_junctions(temp_dir.to_string_lossy().to_string(), &target_tools);
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// 安装 skill：先复制到核心仓库，再为每个工具创建 JUNCTION
fn install_skill_with_junctions(src_dir: String, target_tools: &[String]) -> Result<(), String> {
    let src = PathBuf::from(&src_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 从 SKILL.md 读取名称
    let skill_md = src.join("SKILL.md");
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        let desc = content.lines().next().unwrap_or("").trim_start_matches('#').trim().to_string();
        let folder_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (folder_name, desc)
    } else {
        let n = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (n.clone(), n)
    };

    let id = name.to_lowercase().replace(' ', "-");

    // 1. 复制到核心 skill 仓库 ~/.agents/skills/<id>/
    let canonical_dir = skills_dir().join(&id);
    if canonical_dir.exists() {
        let _ = fs::remove_dir_all(&canonical_dir);
    }
    copy_dir_recursive(&src, &canonical_dir)?;

    // 2. 为每个目标工具创建 JUNCTION（路径由 registry JSON 配置驱动）
    let tool_skill_dirs: Vec<(String, PathBuf)> = target_tools.iter().map(|t| {
        (t.clone(), registry().resolve_skill_junction_target(t, &id))
    }).collect();

    let mut enabled_tools: Vec<String> = Vec::new();
    for (tool_id, tool_dir) in &tool_skill_dirs {
        // 确保父目录存在
        if let Some(parent) = tool_dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // 如果目标已存在（非 junction），先删除
        if tool_dir.exists() {
            let is_junction = fs::symlink_metadata(tool_dir).map(|m| m.file_type().is_symlink()).unwrap_or(false);
            if is_junction {
                let _ = fs::remove_dir(tool_dir);
            } else {
                let _ = fs::remove_dir_all(tool_dir);
            }
        }
        // 创建 JUNCTION
        if let Err(e) = create_ai_junction(tool_dir, &canonical_dir) {
            eprintln!("[install_skill] JUNCTION 失败 for {}: {}", tool_id, e);
        } else {
            enabled_tools.push(tool_id.clone());
        }
    }

    // 3. 保存到 skills.json
    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != id);
    skills.skills.push(Skill {
        id: id.clone(),
        name: name.clone(),
        description,
        directory: canonical_dir.to_string_lossy().to_string(),
        enabled_tools,
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: "managed".to_string(),
    });
    save_skills(&skills)
}
fn copy_dir_recursive(src: &PathBuf, dest: &PathBuf) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest_path = dest.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
