use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use tauri::Emitter;
use crate::commands::ai_registry::registry;
use crate::commands::config::get_data_dir;
use crate::commands::cache::create_junction;
use super::config::load_ai_config;
use super::models::*;

// ─── 基础路径 ───

fn skills_path() -> PathBuf {
    get_data_dir().join("skills.json")
}

/// 默认技能仓库：`~/.agents/skills`（与 skills.sh 生态对齐）。
pub(crate) fn default_skills_dir() -> PathBuf {
    home_dir().join(".agents").join("skills")
}

/// 取 HOME 目录（兼容 Windows `USERPROFILE` / Unix `HOME`）
fn home_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default(),
    )
}

/// 根据配置字符串解析技能目录：
/// - 空字符串 → 默认 `~/.agents/skills`；
/// - 支持 `~` 前缀与 Windows `%VAR%` 占位符展开；
/// - 相对路径以应用根目录为基准。
pub(crate) fn resolve_skills_dir(cfg_value: &str) -> PathBuf {
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
        .replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default())
        .replace("%APPDATA%", &std::env::var("APPDATA").unwrap_or_default())
        .replace("%PROGRAMFILES%", &std::env::var("ProgramFiles").unwrap_or_default());
    let mut p = PathBuf::from(resolved);
    if p.is_relative() {
        p = get_data_dir().join(&p);
    }
    p
}

/// 当前技能仓库路径。
/// 作为 skills.sh 的 GUI，托管仓库即 skills.sh 公共仓库（默认 ~/.agents/skills），
/// 由 AiConfig.skills_dir 配置；非空时回退到默认（deprecated 字段，GUI 不再暴露）。
pub(crate) fn skills_dir() -> PathBuf {
    let cfg = load_ai_config();
    resolve_skills_dir(&cfg.skills_dir)
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
    let path = skills_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(skills).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

// ─── 技能元数据（分类/标签）读写 ───
// 与 skills.sh 规范解耦：不污染 SKILL.md，单独存于点文件 .meta.json。

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct SkillMetaFile {
    #[serde(default)]
    skills: HashMap<String, SkillMetaItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SkillMetaItem {
    #[serde(default)]
    category: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// 元数据文件路径：`~/.agents/skills/.meta.json`（点文件，扫描时跳过）
fn skill_meta_path() -> PathBuf {
    skills_dir().join(".meta.json")
}

fn load_skill_meta() -> SkillMetaFile {
    let path = skill_meta_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(meta) = serde_json::from_str::<SkillMetaFile>(&data) {
                return meta;
            }
        }
    }
    SkillMetaFile::default()
}

fn save_skill_meta(meta: &SkillMetaFile) -> Result<(), String> {
    let path = skill_meta_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

// ─── SKILL.md 解析 ───

fn strip_quotes(s: &str) -> String {
    let t = s.trim();
    if t.len() < 2 {
        return t.to_string();
    }
    match (t.chars().next(), t.chars().last()) {
        (Some('"'), Some('"')) | (Some('\''), Some('\'')) => t[1..t.len() - 1].to_string(),
        _ => t.to_string(),
    }
}

fn parse_skill_md(content: &str, folder_name: &str) -> (String, String) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map_or(false, |l| l.trim() == "---") {
        if let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---") {
            let mut name: Option<String> = None;
            let mut desc: Option<String> = None;
            for line in &lines[1..=end] {
                let line = line.trim();
                if name.is_none() {
                    if let Some(v) = line.strip_prefix("name:") {
                        name = Some(strip_quotes(v));
                        if desc.is_some() { break; }
                        continue;
                    }
                }
                if desc.is_none() {
                    if let Some(v) = line.strip_prefix("description:") {
                        desc = Some(strip_quotes(v));
                        if name.is_some() { break; }
                    }
                }
            }
            let name = name.unwrap_or_else(|| folder_name.to_string());
            let desc = desc.unwrap_or_default();
            return (name, desc);
        }
    }
    let desc = lines.first().unwrap_or(&"").trim_start_matches('#').trim().to_string();
    (folder_name.to_string(), desc)
}

// ─── 数据结构 ───

/// 技能工具信息（用于前端勾选目标工具 / MCP 工具列表）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillToolInfo {
    pub id: String,
    pub label: String,
}

/// 工具技能目录管理状态
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillToolStatusView {
    pub tool_id: String,
    pub label: String,
    pub skills_dir: String,
    /// "managed" | "unmanaged" | "empty"
    pub status: String,
    pub skill_count: usize,
    /// 用户设置中是否开启软链接安装
    pub symlink_enabled: bool,
    /// 该工具是否原生读取公共技能库 ~/.agents/skills
    pub reads_agents_skills: bool,
}

/// 一键管理所有工具的结果
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ManageAllResult {
    pub managed_count: usize,
    pub skipped_count: usize,
    pub errors: Vec<String>,
}

/// 技能条目（简化版总览）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed_at: String,
    pub install_method: String,
    /// 用户自定义分类（来自 .meta.json，缺省空字符串）
    #[serde(default)]
    pub category: String,
    /// 用户自定义标签（来自 .meta.json，缺省空数组）
    #[serde(default)]
    pub tags: Vec<String>,
}

/// 安装进度事件
#[derive(Serialize, Clone, Debug)]
pub struct SkillInstallProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub skill_name: String,
    pub message: String,
}

fn emit_install_progress(
    app: &tauri::AppHandle,
    stage: &str,
    current: usize,
    total: usize,
    skill_name: &str,
    message: &str,
) {
    let _ = app.emit(
        "skill-install-progress",
        SkillInstallProgress {
            stage: stage.to_string(),
            current,
            total,
            skill_name: skill_name.to_string(),
            message: message.to_string(),
        },
    );
}

// ─── 工具技能目录管理（整目录 junction） ───

/// 解析某工具的技能目录路径（取 skills-scan.json 中该工具的第一个目录）
fn resolve_tool_skills_dir(tool_id: &str) -> Option<PathBuf> {
    let reg = registry();
    let scan = reg.skills_scan();
    scan.tool_skills_dirs.get(tool_id).and_then(|dirs| dirs.first()).map(|d| {
        let home = home_dir();
        let resolved = if d.starts_with("~/") {
            home.join(&d[2..])
        } else if d.starts_with('~') {
            home.join(&d[1..])
        } else {
            PathBuf::from(d)
        };
        resolved
    })
}

/// 判断路径是否为 junction/symlink
fn is_junction(path: &PathBuf) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// 扫描各工具技能目录的管理状态
#[tauri::command]
pub fn get_skill_tools_status() -> Result<Vec<SkillToolStatusView>, String> {
    let reg = registry();
    let scan = reg.skills_scan();
    let ai_cfg = load_ai_config();
    let mut out: Vec<SkillToolStatusView> = Vec::new();

    for tool_id in scan.tool_skills_dirs.keys() {
        let label = reg.get_tool_config(tool_id)
            .map(|c| c.display_name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| tool_id.clone());

        let dir = match resolve_tool_skills_dir(tool_id) {
            Some(d) => d,
            None => continue,
        };

        let (status, skill_count) = if !dir.exists() {
            ("empty".to_string(), 0)
        } else if is_junction(&dir) {
            let count = count_skills_in_dir(&dir);
            ("managed".to_string(), count)
        } else {
            let count = count_skills_in_dir(&dir);
            if count == 0 {
                ("empty".to_string(), 0)
            } else {
                ("unmanaged".to_string(), count)
            }
        };

        let reads_agents = scan.reads_agents_skills.iter().any(|t| t == tool_id);
        // 若用户专门配置过，以用户配置为准；否则原生读取公共库的默认不弹软链接（即 false）
        let symlink_enabled = ai_cfg.tool_symlinks.get(tool_id)
            .cloned()
            .unwrap_or(!reads_agents);

        out.push(SkillToolStatusView {
            tool_id: tool_id.clone(),
            label,
            skills_dir: dir.to_string_lossy().to_string(),
            status,
            skill_count,
            symlink_enabled,
            reads_agents_skills: reads_agents,
        });
    }

    out.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(out)
}

/// 切换工具的软链接部署开关（保存并自动解除或创建 junction）
#[tauri::command]
pub fn toggle_tool_symlink_setting(tool_id: String, enabled: bool) -> Result<(), String> {
    let mut cfg = load_ai_config();
    cfg.tool_symlinks.insert(tool_id.clone(), enabled);
    let _ = super::config::save_ai_config_to_file(&cfg);

    if enabled {
        let _ = manage_tool_skills_forced(tool_id);
    } else {
        let _ = unmanage_tool_skills(tool_id);
    }
    Ok(())
}

/// 强制创建/恢复软链接（不受 reads_agents_skills 阻断）
fn manage_tool_skills_forced(tool_id: String) -> Result<(), String> {
    let store = skills_dir();
    let _ = fs::create_dir_all(&store);

    let dir = resolve_tool_skills_dir(&tool_id)
        .ok_or_else(|| format!("工具 {} 未配置技能目录", tool_id))?;

    if dir.exists() && is_junction(&dir) {
        return Ok(());
    }

    if dir.exists() && dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() { continue; }
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') { continue; }
                let dest = store.join(&name);
                if !dest.exists() {
                    let _ = copy_dir_recursive(&path, &dest);
                }
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    if let Some(parent) = dir.parent() {
        let _ = fs::create_dir_all(parent);
    }

    create_junction(&store, &dir).map_err(|e| format!("创建软链接失败: {}", e))
}

/// 统计目录中的技能数量（子目录数，跳过文件和隐藏目录）
fn count_skills_in_dir(dir: &PathBuf) -> usize {
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(dir)
        .map(|entries| {
            entries.flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    e.path().is_dir() && !name.starts_with('.')
                })
                .count()
        })
        .unwrap_or(0)
}

/// 将某工具的技能目录纳入管理：迁移内容到仓库 + 创建整目录 junction
#[tauri::command]
pub fn manage_tool_skills(tool_id: String) -> Result<(), String> {
    let reg = registry();
    let scan = reg.skills_scan();
    let ai_cfg = load_ai_config();

    let reads_agents = scan.reads_agents_skills.iter().any(|t| t == &tool_id);
    let symlink_enabled = ai_cfg.tool_symlinks.get(&tool_id)
        .cloned()
        .unwrap_or(!reads_agents);

    // 若用户未开启该工具软链接（或原生读公共库且未手动开软链接），确保清理软链接
    if !symlink_enabled {
        if let Some(d) = resolve_tool_skills_dir(&tool_id) {
            if d.exists() && is_junction(&d) {
                let _ = unmanage_tool_skills(tool_id.clone());
            }
        }
        return Ok(());
    }

    manage_tool_skills_forced(tool_id)
}

/// 取消管理：移除 junction，重建为空目录
#[tauri::command]
pub fn unmanage_tool_skills(tool_id: String) -> Result<(), String> {
    let dir = resolve_tool_skills_dir(&tool_id)
        .ok_or_else(|| format!("工具 {} 未配置技能目录", tool_id))?;

    if !dir.exists() {
        return Ok(());
    }

    if is_junction(&dir) {
        fs::remove_dir(&dir).map_err(|e| format!("移除 junction 失败: {}", e))?;
    }

    // 重建为空目录
    fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {}", e))?;
    Ok(())
}

/// 一键管理所有工具
#[tauri::command]
pub fn manage_all_tool_skills() -> Result<ManageAllResult, String> {
    let reg = registry();
    let scan = reg.skills_scan();
    let mut managed_count = 0;
    let mut skipped_count = 0;
    let mut errors: Vec<String> = Vec::new();

    for tool_id in scan.tool_skills_dirs.keys() {
        // universal agent 原生读取 ~/.agents/skills，junction 会导致重复技能告警；
        // 若历史上已创建 junction，这里移除它（清理告警）；否则直接跳过。
        if scan.reads_agents_skills.iter().any(|t| t == tool_id) {
            if let Some(d) = resolve_tool_skills_dir(tool_id) {
                if d.exists() && is_junction(&d) {
                    let _ = unmanage_tool_skills(tool_id.clone());
                    skipped_count += 1;
                }
            }
            continue;
        }

        let dir = match resolve_tool_skills_dir(tool_id) {
            Some(d) => d,
            None => continue,
        };

        // 已管理 → 跳过
        if dir.exists() && is_junction(&dir) {
            skipped_count += 1;
            continue;
        }

        match manage_tool_skills(tool_id.clone()) {
            Ok(()) => managed_count += 1,
            Err(e) => errors.push(format!("{}: {}", tool_id, e)),
        }
    }

    Ok(ManageAllResult { managed_count, skipped_count, errors })
}

// ─── 技能列表（仓库扫描） ───

/// 技能总览：扫描技能仓库（skills.sh 公共仓库，默认 ~/.agents/skills）。
#[tauri::command]
pub fn get_skill_overview() -> Result<Vec<SkillEntry>, String> {
    let store = skills_dir();
    // 全局 manifest / meta 按 id 关联（与具体仓库目录无关）
    let manifest = load_skills();
    let meta = load_skill_meta();
    let mut entries: Vec<SkillEntry> = Vec::new();

    if !store.exists() {
        return Ok(entries);
    }

    let dir_entries = fs::read_dir(&store).map_err(|e| e.to_string())?;
    for entry in dir_entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        if id.starts_with('.') {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        let (name, description) = if skill_md.exists() {
            let c = fs::read_to_string(&skill_md).unwrap_or_default();
            parse_skill_md(&c, &id)
        } else {
            (id.clone(), String::new())
        };

        // 从 manifest 补充元数据
        let (installed_at, install_method) = manifest.skills.iter()
            .find(|s| s.id == id)
            .map(|s| (s.installed_at.clone(), s.install_method.clone()))
            .unwrap_or_default();

        // 从 .meta.json 补充用户分类/标签
        let m = meta.skills.get(&id);
        let category = m.map(|x| x.category.clone()).unwrap_or_default();
        let tags = m.map(|x| x.tags.clone()).unwrap_or_default();

        entries.push(SkillEntry {
            id,
            name,
            description,
            installed_at,
            install_method,
            category,
            tags,
        });
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(entries)
}

/// 返回全部技能元数据（分类/标签），用于前端构建筛选选项
#[tauri::command]
pub fn get_skill_meta() -> Result<HashMap<String, SkillMetaItemDto>, String> {
    let meta = load_skill_meta();
    let mut out: HashMap<String, SkillMetaItemDto> = HashMap::new();
    for (k, v) in meta.skills {
        out.insert(k, SkillMetaItemDto { category: v.category, tags: v.tags });
    }
    Ok(out)
}

/// 更新技能的分类与标签（写入 .meta.json）
#[tauri::command]
pub fn update_skill_meta(
    skill_id: String,
    category: String,
    tags: Vec<String>,
) -> Result<(), String> {
    if skill_id.is_empty() {
        return Err("技能 id 不能为空".to_string());
    }
    let mut meta = load_skill_meta();
    let tags: Vec<String> = tags.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    meta.skills.insert(skill_id.clone(), SkillMetaItem { category: category.trim().to_string(), tags });
    save_skill_meta(&meta)
}

/// 元数据项 DTO（前端筛选用）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetaItemDto {
    pub category: String,
    pub tags: Vec<String>,
}

// ─── 安装 / 卸载 ───

#[tauri::command]
pub fn install_skill(skill_dir: String) -> Result<(), String> {
    let src = PathBuf::from(&skill_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    let skill_md = src.join("SKILL.md");
    let folder_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
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

    if dest_dir.exists() {
        let _ = fs::remove_dir_all(&dest_dir);
    }
    copy_dir_recursive(&src, &dest_dir)?;

    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != id);
    skills.skills.push(Skill {
        id,
        name,
        description,
        directory: dest_dir.to_string_lossy().to_string(),
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: "local".to_string(),
    });
    save_skills(&skills)
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
    let name = skills.skills.iter()
        .find(|s| s.id == skill_id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| skill_id.clone());

    let mut files = Vec::new();
    collect_skill_files(&dir, &dir, &mut files)?;
    Ok((name, files))
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

// ─── 在线安装 ───

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

    let temp_dir = get_data_dir().join("_temp_skill_clone");
    let _ = fs::remove_dir_all(&temp_dir);

    let mut cmd = tokio::process::Command::new("git");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = cmd
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

    // 本地路径
    let local_path = PathBuf::from(src_trimmed);
    if local_path.exists() && local_path.is_dir() {
        emit_install_progress(&app, "安装", 1, 1, "", "正在安装到仓库...");
        let result = install_skill(local_path.to_string_lossy().to_string());
        if result.is_ok() {
            emit_install_progress(&app, "完成", 1, 1, "", "安装完成！已管理工具自动获得新技能");
        }
        return result;
    }

    // Git URL 或 owner/repo
    let repo_url = if src_trimmed.starts_with("http://") || src_trimmed.starts_with("https://") {
        src_trimmed.to_string()
    } else if src_trimmed.contains('/') && !src_trimmed.contains('\\') {
        format!("https://github.com/{}", src_trimmed)
    } else {
        return Err("无效的来源格式（需要 Git URL 或 owner/repo）".to_string());
    };

    // Git clone
    let temp_dir = get_data_dir().join("_temp_skill_clone");
    let _ = fs::remove_dir_all(&temp_dir);
    emit_install_progress(&app, "克隆", 0, 0, "", "正在克隆技能源仓库...");

    let mut cmd = tokio::process::Command::new("git");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let output = cmd
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

    // 安装到仓库
    emit_install_progress(&app, "安装", 1, 1, "", "正在安装到仓库...");
    let result = install_skill(temp_dir.to_string_lossy().to_string());
    let _ = fs::remove_dir_all(&temp_dir);

    if result.is_ok() {
        emit_install_progress(&app, "完成", 1, 1, "", "安装完成！已管理工具自动获得新技能");
    }
    result
}

// ─── 旧数据迁移 ───

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

/// 执行技能目录迁移（配置变更时由 save_ai_config 调用）
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

    if let Err(e) = fs::create_dir_all(&new_path) {
        result.errors.push(format!("创建新目录失败: {}", e));
        return result;
    }

    let total = skills.len();
    for (i, skill) in skills.iter().enumerate() {
        let skill_id = &skill.id;
        if let Some(handle) = app_handle {
            let _ = handle.emit("skill-migrate-progress", SkillMigrateProgress {
                stage: "移动技能".to_string(),
                current: i + 1,
                total,
                skill_name: skill.name.clone(),
            });
        }

        let old_skill_dir = old_path.join(skill_id);
        let new_skill_dir = new_path.join(skill_id);

        if old_skill_dir.exists() && old_skill_dir != new_skill_dir {
            if new_skill_dir.exists() {
                let _ = fs::remove_dir_all(&new_skill_dir);
            }
            match fs::rename(&old_skill_dir, &new_skill_dir) {
                Ok(()) => { result.moved_count += 1; }
                Err(e) => {
                    if let Err(e2) = copy_dir_recursive(&old_skill_dir, &new_skill_dir) {
                        result.errors.push(format!("迁移 {} 失败: {} -> {}", skill.name, e, e2));
                        continue;
                    } else {
                        let _ = fs::remove_dir_all(&old_skill_dir);
                        result.moved_count += 1;
                    }
                }
            }
        }
    }

    if let Some(handle) = app_handle {
        let _ = handle.emit("skill-migrate-progress", SkillMigrateProgress {
            stage: "完成".to_string(),
            current: total,
            total,
            skill_name: String::new(),
        });
    }
    result
}

/// 一次性迁移：若旧仓库 ~/.any-version/skills 存在且有内容，合并到新仓库
#[tauri::command]
pub fn migrate_legacy_skills() -> Result<usize, String> {
    let old_store = get_data_dir().join("skills");
    let new_store = skills_dir();

    // 如果旧目录不存在或与新目录相同，无需迁移
    if !old_store.exists() || normalize_path(&old_store.to_string_lossy()) == normalize_path(&new_store.to_string_lossy()) {
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
    let p = path.trim_end_matches('\\').trim_end_matches('/').to_lowercase();
    p.strip_prefix("\\\\?\\").unwrap_or(&p).to_string()
}

fn copy_dir_recursive(src: &PathBuf, dest: &PathBuf) -> Result<(), String> {
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
