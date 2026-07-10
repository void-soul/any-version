use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use tauri::Emitter;
use crate::commands::ai_registry::registry;
use crate::commands::config::get_base_dir;
use crate::commands::cache::create_junction;
use super::config::load_ai_config;
use super::models::*;


fn skills_path() -> PathBuf {
    get_base_dir().join("skills.json")
}

/// 默认 AnyVersion 技能仓库根目录：`~/.any-version/skills`。
/// 注意：`~/.agents/skills` 是 skills.sh 的仓库，不是 AnyVersion 的。
pub(crate) fn default_skills_dir() -> PathBuf {
    get_base_dir().join("skills")
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
/// - 空字符串 → 默认 `~/.any-version/skills`（AnyVersion 自身仓库）；
/// - 支持 `~` 前缀与 Windows `%VAR%` 占位符展开；
/// - **相对路径**（无 `~`、非绝对、无 `%VAR%`）以应用根目录 `~/.any-version` 为基准，
///   确保结果总是绝对路径——否则 `mklink /J` 会以链接父目录解析相对目标而失败。
///
/// 这是 AnyVersion 托管技能的唯一权威目录，与 skills.sh 的 `~/.agents/skills` 无关。
pub(crate) fn resolve_skills_dir(cfg_value: &str) -> PathBuf {
    if cfg_value.is_empty() {
        return default_skills_dir();
    }
    let home = home_dir();
    // 先展开 `~` 前缀
    let with_tilde = if cfg_value.starts_with("~/") {
        home.join(&cfg_value[2..]).to_string_lossy().to_string()
    } else if cfg_value.starts_with('~') && cfg_value.len() > 1 {
        home.join(&cfg_value[1..]).to_string_lossy().to_string()
    } else {
        cfg_value.to_string()
    };
    // 再展开 Windows 环境变量占位符
    let resolved = with_tilde
        .replace("%USERPROFILE%", &home.to_string_lossy())
        .replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default())
        .replace("%APPDATA%", &std::env::var("APPDATA").unwrap_or_default())
        .replace("%PROGRAMFILES%", &std::env::var("ProgramFiles").unwrap_or_default());
    let mut p = PathBuf::from(resolved);
    // 相对路径一律以应用根目录（默认技能目录即位于 `~/.any-version/skills` 之下）为基准
    if p.is_relative() {
        p = get_base_dir().join(&p);
    }
    p
}

/// 当前 AnyVersion 技能仓库（读取全局配置 `AiConfig.skills_dir`，空字符串回退默认）。
pub(crate) fn skills_dir() -> PathBuf {
    let cfg = load_ai_config();
    let resolved = resolve_skills_dir(&cfg.skills_dir);
    // 一次性修复历史遗留：旧版本把技能误写到 CWD 下的相对目录（如 `skills/`），
    // 这里把那个游离目录合并进解析后的绝对仓库，避免数据丢失。
    static HEALED: std::sync::Once = std::sync::Once::new();
    HEALED.call_once(|| {
        heal_stray_relative_skills_dir(&cfg.skills_dir, &resolved);
    });
    resolved
}

/// 若配置值是「纯相对路径」（无 `~`、无 `%VAR%`、非绝对），历史上可能把技能数据
/// 误写到当前工作目录下的同名相对目录。把其中尚缺于绝对仓库的技能合并进来（幂等、一次性）。
/// 跨盘用拷贝+删除，避免 `rename` 跨卷失败。
fn heal_stray_relative_skills_dir(cfg_value: &str, abs_store: &PathBuf) {
    let is_plain_relative = !cfg_value.is_empty()
        && !cfg_value.starts_with('~')
        && !cfg_value.contains('%')
        && !PathBuf::from(cfg_value).is_absolute();
    if !is_plain_relative {
        return;
    }
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return,
    };
    let stray = cwd.join(cfg_value);
    if !stray.exists() || stray == *abs_store {
        return;
    }
    if let Ok(entries) = fs::read_dir(&stray) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            let dest = abs_store.join(p.file_name().unwrap());
            if !dest.exists() {
                let _ = fs::create_dir_all(abs_store);
                // 优先 rename，失败则拷贝后删除（跨盘场景）
                if fs::rename(&p, &dest).is_err() {
                    let _ = copy_dir_recursive(&p, &dest);
                    let _ = fs::remove_dir_all(&p);
                }
            }
        }
    }
    let _ = fs::remove_dir(&stray);
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

/// 技能工具信息（用于前端勾选目标工具）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillToolInfo {
    pub id: String,
    pub label: String,
}

/// 解析 SKILL.md 元数据。
///
/// 遵循 skills.sh 规范：优先读取 YAML frontmatter 中的 `name` / `description`；
/// 去掉字符串首尾成对的引号（" 或 '），用于解析 frontmatter 中带引号或不带引号的值。
/// `name: "imagegen"` 与 `name: imagegen` 都会被解析为 `imagegen`。
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

/// 若文件不以 `---` 开头（无 frontmatter），则回退为「首行 `# 标题` 作为描述、文件夹名作为名称」的旧行为，
/// 以兼容未带 frontmatter 的技能包。
fn parse_skill_md(content: &str, folder_name: &str) -> (String, String) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map_or(false, |l| l.trim() == "---") {
        // 找到第二个 `---` 作为 frontmatter 结束符
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
    // 回退：首行 `# 标题`
    let desc = lines.first().unwrap_or(&"").trim_start_matches('#').trim().to_string();
    (folder_name.to_string(), desc)
}

// ─── 技能管理 ───

#[tauri::command]
pub fn get_skills() -> Result<Vec<SkillView>, String> {
    let manifest = load_skills().skills;
    let reg = registry();
    let global_store = skills_dir();
    let mut views = Vec::with_capacity(manifest.len());
    for skill in manifest {
        let mut installed = Vec::new();
        let mut foreign = Vec::new();
        for tool_id in reg.skills_scan().tool_skills_dirs.keys() {
            let target = reg.resolve_skill_junction_target(tool_id, &skill.id);
            if !target.exists() {
                continue;
            }
            // 由 AnyVersion 统一安装的技能：junction 指向 AnyVersion 托管仓库（~/.any-version/skills，配置可改，含 .system 嵌套）
            if is_managed(&target, &global_store) {
                installed.push(tool_id.clone());
            } else {
                // 工具自行安装（真实目录）或 junction 到其他目录
                foreign.push(tool_id.clone());
            }
        }
        views.push(SkillView {
            id: skill.id,
            name: skill.name,
            description: skill.description,
            directory: skill.directory,
            installed_tools: installed,
            foreign_tools: foreign,
            installed_at: skill.installed_at,
            install_method: skill.install_method,
        });
    }
    Ok(views)
}

/// `get_skill_overview` 的临时构建器
struct OverviewBuilder {
    name: String,
    description: String,
    /// 是否已收录进 AnyVersion manifest（skills.json）
    registered: bool,
    /// 是否位于 AnyVersion 技能目录（默认 ~/.any-version/skills，配置可改）
    in_store: bool,
    directory: String,
    installed_at: String,
    install_method: String,
    tools: std::collections::HashMap<String, String>,
}

impl OverviewBuilder {
    fn new() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            registered: false,
            in_store: false,
            directory: String::new(),
            installed_at: String::new(),
            install_method: String::new(),
            tools: std::collections::HashMap::new(),
        }
    }
}

/// 统一技能总览：合并 AnyVersion 托管技能与各工具私自安装的技能，
/// 并给出每个技能在每个工具上的安装现状（managed / foreign / none）。
///
/// 这是前端「技能管理」界面的唯一数据源，避免多张列表并存导致的混乱。
#[tauri::command]
/// 构建技能总览（全量，不过滤）。供 `get_skill_overview`（前端「技能列表」选项卡）使用，
/// 包含仓库内技能与工具私有技能（情况A/B）。
fn build_skill_overview() -> Result<Vec<SkillOverview>, String> {
    let reg = registry();
    let global_store = skills_dir();
    let scan_dirs = reg.get_skill_scan_dirs();
    let tool_ids: Vec<String> = reg.skills_scan().tool_skills_dirs.keys().cloned().collect();

    let mut builders: std::collections::HashMap<String, OverviewBuilder> =
        std::collections::HashMap::new();

    // 1) 全局仓库中的技能（含 .system 嵌套容器内的技能）
    if global_store.exists() {
        for (id, path) in collect_skill_entries(&global_store) {
            let skill_md = path.join("SKILL.md");
            let (name, description) = if skill_md.exists() {
                let c = fs::read_to_string(&skill_md).unwrap_or_default();
                parse_skill_md(&c, &id)
            } else {
                (id.clone(), String::new())
            };
            let b = builders.entry(id.clone()).or_insert_with(OverviewBuilder::new);
            b.in_store = true;
            b.name = name;
            b.description = description;
            b.directory = path.to_string_lossy().to_string();
        }
    }
    // 补充 manifest 元数据（registered / installed_at / install_method）
    let manifest = load_skills();
    for m in &manifest.skills {
        if let Some(b) = builders.get_mut(&m.id) {
            b.registered = true;
            b.installed_at = m.installed_at.clone();
            b.install_method = m.install_method.clone();
            if b.name.is_empty() {
                b.name = m.name.clone();
            }
            if b.description.is_empty() {
                b.description = m.description.clone();
            }
        }
    }

    // 2) 各工具目录：判定每个技能的安装现状
    for (dir, label) in &scan_dirs {
        if label.as_str() == "any-version" {
            continue; // 跳过 AnyVersion 自身仓库（已作为 global_store 单独处理）
        }
        if !dir.exists() {
            continue;
        }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = match path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if id == SYSTEM_SKILLS_CONTAINER {
                continue; // 嵌套技能容器，不作为工具的技能
            }
            let global_path = find_store_path(&global_store, &id);
            let b = builders.entry(id.clone()).or_insert_with(OverviewBuilder::new);
            if global_path.is_some() {
                let status = if is_managed(&path, &global_store) {
                    "managed"
                } else {
                    "foreign"
                };
                b.tools.insert(label.clone(), status.to_string());
            } else {
                // 外部技能（未入全局仓库）
                if b.name.is_empty() {
                    let skill_md = path.join("SKILL.md");
                    let (name, description) = if skill_md.exists() {
                        let c = fs::read_to_string(&skill_md).unwrap_or_default();
                        parse_skill_md(&c, &id)
                    } else {
                        (id.clone(), String::new())
                    };
                    b.name = name;
                    b.description = description;
                }
                b.tools.insert(label.clone(), "foreign".to_string());
            }
        }
    }

    // 补全每个 builder 的工具状态（缺省 none）并按名称排序
    let mut out: Vec<SkillOverview> = builders
        .into_iter()
        .map(|(id, b)| {
            let mut tools: Vec<SkillToolStatus> = tool_ids
                .iter()
                .map(|tid| {
                    let status = b
                        .tools
                        .get(tid)
                        .cloned()
                        .unwrap_or_else(|| "none".to_string());
                    SkillToolStatus {
                        tool_id: tid.clone(),
                        status,
                    }
                })
                .collect();
            tools.sort_by(|a, b| a.tool_id.cmp(&b.tool_id));
            SkillOverview {
                id: id.clone(),
                name: if b.name.is_empty() { id.clone() } else { b.name },
                description: b.description,
                in_store: b.in_store,
                registered: b.registered,
                directory: b.directory,
                installed_at: b.installed_at,
                install_method: b.install_method,
                tools,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// 前端「AnyVersion 技能」列表（区块二）：仅列出已位于 AnyVersion 技能仓库
/// （配置驱动，默认 `~/.any-version/skills`，含 `.system` 嵌套）的技能，并据各工具状态点亮安装情况（情况C）。
/// 工具私有技能（情况A/B）不在此列出，改由 `get_discoverable_skills` 单独呈现。
#[tauri::command]
pub fn get_skill_overview() -> Result<Vec<SkillOverview>, String> {
    let mut out = build_skill_overview()?;
    out.retain(|s| s.in_store);
    Ok(out)
}





/// `.system` 是技能容器目录（存放系统技能），本身不是技能，需递归其内部目录。
const SYSTEM_SKILLS_CONTAINER: &str = ".system";

/// 判断工具目录下的技能条目是否为 AnyVersion 托管：
/// 其真实路径（跟随 symlink/junction）解析后位于 AnyVersion 技能仓库之内
/// （含扁平布局 `<store>/<id>` 与嵌套布局 `<store>/.system/<id>`，store 默认 `~/.any-version/skills`）。
/// 这样同时覆盖：
/// - 工具目录是 junction 指向 AnyVersion 仓库（扁平或 .system 嵌套）；
/// - 工具目录本身即 AnyVersion 仓库。
/// 真实目录、或指向仓库之外位置的 junction 均视为「外部/私自安装」。
fn is_managed(tool_entry: &PathBuf, global_store: &PathBuf) -> bool {
    let a = match fs::canonicalize(tool_entry) {
        Ok(p) => normalize_path(&p.to_string_lossy()),
        Err(_) => return false,
    };
    let store = match fs::canonicalize(global_store) {
        Ok(p) => normalize_path(&p.to_string_lossy()),
        Err(_) => return false,
    };
    if a == store {
        return true;
    }
    a.starts_with(&format!("{}/", store)) || a.starts_with(&format!("{}\\", store))
}

/// 在全局仓库中定位某技能的真实目录（兼容扁平 `.system` 嵌套布局）。
/// 返回 `Some(path)` 表示技能已存在于仓库（无论是否由 AnyVersion 登记）。
fn find_store_path(global_store: &PathBuf, skill_id: &str) -> Option<PathBuf> {
    let direct = global_store.join(skill_id);
    if direct.is_dir() {
        return Some(direct);
    }
    let nested = global_store.join(SYSTEM_SKILLS_CONTAINER).join(skill_id);
    if nested.is_dir() {
        return Some(nested);
    }
    None
}

/// 收集某目录下的技能条目，返回 `(skill_id, real_path)`。
/// - 跳过文件（如 `.codex-system-skills.marker`）；
/// - `.system` 本身是嵌套技能容器，递归其内部子目录作为技能；
/// - 其余一级子目录视为技能。
fn collect_skill_entries(dir: &PathBuf) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue; // 跳过文件（marker 等）
        }
        let id = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };
        if id == SYSTEM_SKILLS_CONTAINER {
            // 嵌套技能容器：递归其内部子目录
            if let Ok(inner) = fs::read_dir(&path) {
                for e2 in inner.flatten() {
                    let p2 = e2.path();
                    if !p2.is_dir() {
                        continue;
                    }
                    if let Some(id2) = p2.file_name() {
                        out.push((id2.to_string_lossy().to_string(), p2));
                    }
                }
            }
        } else {
            out.push((id, path));
        }
    }
    out
}

/// 扫描各工具 skills 目录，列出「不在全局仓库、未由 AnyVersion 托管」的技能。
/// `kind` 区分：工具真实目录（`real`）或 junction 到其他目录（`external_junction`）。
/// 这些技能可由用户显式迁移为 AnyVersion 托管方式。
#[tauri::command]
/// 扫描各工具技能目录，列出「工具私自安装 / 外部 junction」的技能。
///
/// - `include_in_store`：是否保留「已在 AnyVersion 全局仓库中存在」的技能。
///   - `false`（旧 `get_foreign_skills` 行为）：仅返回纯外部技能；
///   - `true`：`get_discoverable_skills` 用，保留这些技能并标记 `already_in_anyversion=true`
///     （整理时只需为工具重建 junction，无需拷贝数据）。
/// 按 `tool:skill` 去重（同一技能在同一工具只列一次）。
fn scan_tool_skills(include_in_store: bool) -> Vec<ForeignSkill> {
    let global_store = skills_dir();
    let reg = registry();
    let scan_dirs = reg.get_skill_scan_dirs();
    let mut out: Vec<ForeignSkill> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (dir, label) in &scan_dirs {
        if label.as_str() == "any-version" {
            continue; // 跳过 AnyVersion 自身仓库（已作为 global_store 单独处理）
        }
        if !dir.exists() {
            continue;
        }
        // 用 collect_skill_entries：跳过文件、递归 .system 嵌套容器，
        // 从而 skills.sh 仓库（~/.agents/skills）中的 .system 技能也能被发现。
        for (skill_id, path) in collect_skill_entries(dir) {
            // 跳过 Case C：junction 指向 AnyVersion 仓库（已托管），不是可移动目标
            if is_managed(&path, &global_store) {
                continue;
            }
            let in_store = find_store_path(&global_store, &skill_id).is_some();
            // 非工具来源（如 skills.sh）：若技能已在 AnyVersion 仓库中存在，则忽略——
            // 这些目录不是工具目录，无法建 junction，数据已在仓库中就无需关注。
            if in_store && label == "skills.sh" {
                continue;
            }
            // 工具目录下已在全局仓库的技能：include_in_store=false 时跳过；true 时保留并标记
            if in_store && !include_in_store {
                continue;
            }
            let key = format!("{}:{}", label, skill_id);
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);

            let is_junction = fs::symlink_metadata(&path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            let (kind, source_path) = if is_junction {
                let target = fs::read_link(&path).unwrap_or_else(|_| path.clone());
                ("external_junction".to_string(), target.to_string_lossy().to_string())
            } else {
                ("real".to_string(), path.to_string_lossy().to_string())
            };

            let skill_md = path.join("SKILL.md");
            let (name, description) = if skill_md.exists() {
                let content = fs::read_to_string(&skill_md).unwrap_or_default();
                parse_skill_md(&content, &skill_id)
            } else {
                (skill_id.clone(), skill_id.clone())
            };

            out.push(ForeignSkill {
                tool_id: label.clone(),
                skill_id,
                name,
                description,
                kind,
                source_path,
                already_in_anyversion: in_store,
            });
        }
    }
    out
}

/// 列出工具私自安装、未由 AnyVersion 托管的纯外部技能（仓库中不存在的）。
#[tauri::command]
pub fn get_foreign_skills() -> Result<Vec<ForeignSkill>, String> {
    Ok(scan_tool_skills(false))
}

/// 发现的可移动到 AnyVersion 目录的技能（对应前端「发现的可移动技能」列表）。
/// 聚合各工具的技能位置，按 `skill_id` 去重（忽略重名），并区分情况A（直装）/情况B（外部 junction）。
/// `already_in_anyversion` 表示同名技能已存在于 AnyVersion 全局仓库：
/// - true → 整理时只需为该工具重建 junction（relink）；
/// - false → 需先将数据拷贝进仓库，再建 junction（导入外部技能）。
#[tauri::command]
pub fn get_discoverable_skills() -> Result<Vec<DiscoverableSkill>, String> {
    let raw = scan_tool_skills(true);
    let mut map: std::collections::HashMap<String, DiscoverableSkill> = std::collections::HashMap::new();
    for f in raw {
        let entry = map.entry(f.skill_id.clone()).or_insert_with(|| DiscoverableSkill {
            skill_id: f.skill_id.clone(),
            name: f.name.clone(),
            description: f.description.clone(),
            already_in_anyversion: f.already_in_anyversion,
            locations: Vec::new(),
        });
        entry.already_in_anyversion = entry.already_in_anyversion || f.already_in_anyversion;
        let case = if f.kind == "external_junction" { "B" } else { "A" }.to_string();
        entry.locations.push(SkillLocation {
            tool_id: f.tool_id,
            case,
            link_target: f.source_path,
        });
    }
    let mut out: Vec<DiscoverableSkill> = map.into_values().collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// 将已位于 AnyVersion 仓库但未登记 manifest 的技能纳入管理（仅写元数据，不移动数据）。
/// 对应前端「纳入管理」按钮。
#[tauri::command]
pub fn register_store_skill(skill_id: String) -> Result<(), String> {
    let global_store = skills_dir();
    let store_path = find_store_path(&global_store, &skill_id)
        .ok_or_else(|| format!("技能 {} 不在 AnyVersion 仓库中", skill_id))?;

    let mut skills = load_skills();
    if skills.skills.iter().any(|s| s.id == skill_id) {
        return Ok(()); // 已登记
    }

    let skill_md = store_path.join("SKILL.md");
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        parse_skill_md(&content, &skill_id)
    } else {
        (skill_id.clone(), skill_id.clone())
    };

    skills.skills.push(Skill {
        id: skill_id,
        name,
        description,
        directory: store_path.to_string_lossy().to_string(),
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: "adopted".to_string(),
    });
    save_skills(&skills)
}

// ─── 问题检测与修复 ───

/// 技能问题检测：扫描 skills.sh 管理工具目录 + 各 AI 编程工具目录，
/// 返回所有可收纳的问题列表（按顺序：先 skills.sh，再 AI 工具）。
///
/// 问题类型：
/// - `"skills_sh"`: skills.sh 中存在但 AnyVersion 仓库中不存在的技能
/// - `"A"`: AI 工具目录中直接安装的技能（真实目录）
/// - `"B"`: AI 工具目录中 junction 指向非 AnyVersion 仓库的技能
/// - `"D"`: AI 工具目录中 junction 目标已失效（断链）
///
/// Case C（junction 指向 AnyVersion 仓库 = 已托管）不算问题。
#[tauri::command]
pub fn get_skill_issues() -> Result<Vec<SkillIssue>, String> {
    let global_store = skills_dir();
    let reg = registry();
    let scan_dirs = reg.get_skill_scan_dirs();
    let mut issues: Vec<SkillIssue> = Vec::new();

    // 1. 先扫描管理工具目录（skills.sh）
    for (dir, label) in &scan_dirs {
        if label != "skills.sh" {
            continue;
        }
        if !dir.exists() {
            continue;
        }
        // collect_skill_entries 会递归 .system 嵌套容器
        for (skill_id, path) in collect_skill_entries(dir) {
            // 已在仓库中 → 不值得关注
            if find_store_path(&global_store, &skill_id).is_some() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            let (name, description) = if skill_md.exists() {
                let c = fs::read_to_string(&skill_md).unwrap_or_default();
                parse_skill_md(&c, &skill_id)
            } else {
                (skill_id.clone(), String::new())
            };
            issues.push(SkillIssue {
                issue_type: "skills_sh".to_string(),
                tool_id: label.clone(),
                skill_id,
                name,
                description,
                source_path: path.to_string_lossy().to_string(),
                link_target: String::new(),
                already_in_store: false,
            });
        }
    }

    // 2. 再扫描各 AI 编程工具目录
    for (dir, label) in &scan_dirs {
        if label == "any-version" || label == "skills.sh" {
            continue;
        }
        if !dir.exists() {
            continue;
        }
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let id = match path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if id == SYSTEM_SKILLS_CONTAINER {
                continue;
            }

            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let is_junction = meta.file_type().is_symlink();
            let target_exists = path.exists(); // 断链时返回 false
            let in_store = find_store_path(&global_store, &id).is_some();

            let (issue_type, link_target) = if is_junction {
                let target = fs::read_link(&path).unwrap_or_else(|_| path.clone());
                let target_str = target.to_string_lossy().to_string();
                if !target_exists {
                    ("D".to_string(), target_str)
                } else if is_managed(&path, &global_store) {
                    // Case C: junction 指向 AnyVersion 仓库（已托管），不算问题
                    continue;
                } else {
                    ("B".to_string(), target_str)
                }
            } else if path.is_dir() {
                // Case A: 直接安装（真实目录）
                ("A".to_string(), String::new())
            } else {
                continue; // 非目录非链接，跳过
            };

            // 读取技能元数据：优先从源路径，其次从仓库（Case D 断链时源路径不可用）
            let (name, description) = {
                let md_path = path.join("SKILL.md");
                if md_path.exists() {
                    let c = fs::read_to_string(&md_path).unwrap_or_default();
                    parse_skill_md(&c, &id)
                } else if in_store {
                    let sp = find_store_path(&global_store, &id).unwrap();
                    let smd = sp.join("SKILL.md");
                    if smd.exists() {
                        let c = fs::read_to_string(&smd).unwrap_or_default();
                        parse_skill_md(&c, &id)
                    } else {
                        (id.clone(), String::new())
                    }
                } else {
                    (id.clone(), String::new())
                }
            };

            issues.push(SkillIssue {
                issue_type,
                tool_id: label.clone(),
                skill_id: id,
                name,
                description,
                source_path: path.to_string_lossy().to_string(),
                link_target,
                already_in_store: in_store,
            });
        }
    }

    Ok(issues)
}

/// 将技能登记进 manifest（如果尚未登记）。供 fix_skill_issue_inner 复用。
fn register_if_needed(skill_id: &str, store_path: &PathBuf) -> Result<(), String> {
    let mut skills = load_skills();
    if skills.skills.iter().any(|s| s.id == skill_id) {
        return Ok(());
    }
    let skill_md = store_path.join("SKILL.md");
    let (name, description) = if skill_md.exists() {
        let c = fs::read_to_string(&skill_md).unwrap_or_default();
        parse_skill_md(&c, skill_id)
    } else {
        (skill_id.to_string(), skill_id.to_string())
    };
    skills.skills.push(Skill {
        id: skill_id.to_string(),
        name,
        description,
        directory: store_path.to_string_lossy().to_string(),
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: "adopted".to_string(),
    });
    save_skills(&skills)
}

/// 原子地把工具目录条目替换为指向仓库的 junction：
/// 先在临时名上建好 junction，成功后再删除原条目并换名。
/// 这样即便创建 junction 失败（盘符不可达、权限不足等），原工具条目仍保留，
/// 技能不会丢失链接。
fn link_tool_to_store(tool_entry: &PathBuf, store_path: &PathBuf) -> Result<(), String> {
    if let Some(parent) = tool_entry.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp_name = format!(
        "{}.anyversion_tmp",
        tool_entry
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    );
    let tmp_link = tool_entry.parent().unwrap().join(tmp_name);
    let _ = fs::remove_dir_all(&tmp_link);
    create_ai_junction(&tmp_link, store_path)
        .map_err(|e| format!("创建 junction 失败: {}", e))?;
    // junction 创建成功，才移除原条目（junction 或真实目录）并换名
    if tool_entry.exists() {
        let is_junction = fs::symlink_metadata(tool_entry)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if is_junction {
            let _ = fs::remove_dir(tool_entry);
        } else {
            let _ = fs::remove_dir_all(tool_entry);
        }
    }
    let _ = fs::rename(&tmp_link, tool_entry);
    Ok(())
}

/// 修复单个技能问题（内部实现，供 fix_skill_issue 和 fix_all_issues 共用）。
///
/// 修复逻辑遵循需求：
/// - 不覆盖原则：移动到 AnyVersion 仓库时，如存在同名文件夹则跳过拷贝；
/// - junction 到 AnyVersion 时：先删除旧条目，再创建新 junction；
/// - skills.sh：仅拷贝到仓库 + 登记，不建 junction。
fn fix_skill_issue_inner(tool_id: &str, skill_id: &str) -> Result<(), String> {
    let global_store = skills_dir();
    let _ = fs::create_dir_all(&global_store);

    // ── skills.sh 来源：拷贝到仓库 + 登记 ──
    if tool_id == "skills.sh" {
        let reg = registry();
        let sh_dir = reg.get_skill_scan_dirs().iter()
            .find(|(_, label)| label == "skills.sh")
            .map(|(dir, _)| dir.clone())
            .ok_or("找不到 skills.sh 目录")?;
        let source = find_store_path(&sh_dir, skill_id)
            .unwrap_or_else(|| sh_dir.join(skill_id));
        if !source.exists() {
            return Err(format!("技能 {} 在 skills.sh 中不存在", skill_id));
        }
        let dest = find_store_path(&global_store, skill_id)
            .unwrap_or_else(|| global_store.join(skill_id));
        if !dest.exists() {
            copy_dir_recursive(&source, &dest)?;
        }
        register_if_needed(skill_id, &dest)?;
        return Ok(());
    }

    // ── AI 工具来源 ──
    let reg = registry();
    let tool_entry = reg.resolve_skill_junction_target(tool_id, skill_id);
    let in_store = find_store_path(&global_store, skill_id).is_some();
    let store_path = if in_store {
        find_store_path(&global_store, skill_id).unwrap()
    } else {
        global_store.join(skill_id)
    };

    let meta = fs::symlink_metadata(&tool_entry);

    match meta {
        // junction（符号链接）
        Ok(m) if m.file_type().is_symlink() => {
            if tool_entry.exists() {
                // Case B: junction 目标存在但非 AnyVersion 仓库
                if !in_store {
                    let target = fs::read_link(&tool_entry).unwrap_or_else(|_| tool_entry.clone());
                    if !store_path.exists() {
                        copy_dir_recursive(&target, &store_path)?;
                    }
                }
                // 原子地把工具条目替换为指向仓库的 junction（辅助函数内部先建临时 junction 再换名）
                link_tool_to_store(&tool_entry, &store_path)?;
            } else {
                // Case D: 断链（junction 目标不存在）
                let _ = fs::remove_dir(&tool_entry);
                if in_store {
                    link_tool_to_store(&tool_entry, &store_path)?;
                }
                // 不在仓库中：仅删除断链，无法恢复数据
            }
        }
        // 真实目录 — Case A
        Ok(_) => {
            if !in_store {
                if !store_path.exists() {
                    copy_dir_recursive(&tool_entry, &store_path)?;
                }
            }
            // 先拷后链：原子地把工具目录条目替换为指向仓库的 junction
            link_tool_to_store(&tool_entry, &store_path)?;
        }
        // 条目不存在 — 可能已被修复
        Err(_) => {
            if in_store {
                link_tool_to_store(&tool_entry, &store_path)?;
            }
        }
    }

    register_if_needed(skill_id, &store_path)?;
    Ok(())
}

/// 修复单个技能问题（前端命令入口）。
#[tauri::command]
pub fn fix_skill_issue(tool_id: String, skill_id: String) -> Result<(), String> {
    fix_skill_issue_inner(&tool_id, &skill_id)
}

/// 批量修复技能问题（一键收纳）。
/// 按传入顺序依次修复；汇总成功数与失败详情，若有失败则返回 Err 让前端提示。
#[tauri::command]
pub fn fix_all_issues(issues: Vec<IssueRef>) -> Result<usize, String> {
    let mut count = 0;
    let mut errors: Vec<String> = Vec::new();
    for r in &issues {
        match fix_skill_issue_inner(&r.tool_id, &r.skill_id) {
            Ok(()) => count += 1,
            Err(e) => {
                eprintln!(
                    "[skill] fix_skill_issue failed for {} in {}: {}",
                    r.skill_id, r.tool_id, e
                );
                errors.push(format!("{} ({}): {}", r.skill_id, r.tool_id, e));
            }
        }
    }
    if !errors.is_empty() {
        return Err(format!(
            "成功 {} 个，失败 {} 个: {}",
            count,
            errors.len(),
            errors.join("; ")
        ));
    }
    Ok(count)
}

#[tauri::command]
pub fn install_skill(skill_dir: String) -> Result<(), String> {
    let src = PathBuf::from(&skill_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 从 SKILL.md 读取名称（优先 frontmatter）
    let skill_md = src.join("SKILL.md");
    let folder_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        parse_skill_md(&content, &folder_name)
    } else {
        (folder_name.clone(), folder_name.clone())
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
        // 先移除各工具目录下的 junction（避免悬空链接）
        remove_skill_junctions(&skill_id);
        if !dir.is_empty() {
            let _ = fs::remove_dir_all(&dir);
        }
        skills.skills.remove(pos);
    }
    save_skills(&skills)
}

#[tauri::command]
pub fn toggle_skill_tool(skill_id: String, tool_id: String, enabled: bool) -> Result<(), String> {
    let skills = load_skills();
    let skill = skills.skills.iter().find(|s| s.id == skill_id).ok_or("技能不存在")?;
    // junction 目标必须是「当前配置仓库」(skills_dir) 中的技能目录，
    // 不能依赖 manifest.directory —— 它记录的是登记时的路径，用户改过 skills_dir
    // 或技能被迁移后会失效/指向旧路径，导致 junction 指向错误目录、is_managed 判定失败。
    let global_store = skills_dir();
    let mut canonical = find_store_path(&global_store, &skill_id)
        .unwrap_or_else(|| global_store.join(&skill_id));
    // 当前仓库中找不到时，回退到 manifest 记录的历史目录（兼容旧数据）
    if !canonical.exists() {
        canonical = PathBuf::from(&skill.directory);
    }
    if !canonical.exists() {
        return Err("技能全局目录不存在，请重新安装".to_string());
    }
    let target = registry().resolve_skill_junction_target(&tool_id, &skill_id);

    if enabled {
        // 确保父目录存在，并清理已有条目（旧 junction 或工具私有真实目录）后建 junction
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if target.exists() {
            let is_junction = fs::symlink_metadata(&target)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if is_junction {
                let _ = fs::remove_dir(&target);
            } else {
                let _ = fs::remove_dir_all(&target);
            }
        }
        create_ai_junction(&target, &canonical)
            .map_err(|e| format!("创建 junction 失败: {}", e))?;
    } else {
        // 仅移除 junction；真实目录（工具私有数据）不删除，避免误删
        if target.exists() {
            let is_junction = fs::symlink_metadata(&target)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if is_junction {
                let _ = fs::remove_dir(&target);
            } else {
                eprintln!("[skill] 跳过移除 {}：不是 junction（可能是工具私有真实目录）", target.display());
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_skill_files(skill_id: String) -> Result<(String, Vec<SkillFile>), String> {
    let skills = load_skills();
    // 优先从「当前配置仓库」定位目录（兼容改过 skills_dir 导致 manifest.directory 过期的情况）；
    // 未登记（如 .system 系统技能）或当前仓库找不到时，再回退 manifest 记录的历史目录。
    let dir = match find_store_path(&skills_dir(), &skill_id) {
        Some(p) => p,
        None => match skills.skills.iter().find(|s| s.id == skill_id) {
            Some(s) => PathBuf::from(&s.directory),
            None => return Err("技能不存在".to_string()),
        },
    };
    let name = skills
        .skills
        .iter()
        .find(|s| s.id == skill_id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| skill_id.clone());
    if !dir.exists() {
        return Err("技能目录不存在".to_string());
    }
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
        for (name, path) in collect_skill_entries(base_dir) {
                let skill_md = path.join("SKILL.md");
                let (parsed_name, description) = if skill_md.exists() {
                    let content = fs::read_to_string(&skill_md).unwrap_or_default();
                    parse_skill_md(&content, &name)
                } else {
                    (name.clone(), String::new())
                };

                let full_path = path.to_string_lossy().to_string();
                if seen.contains(&full_path) {
                    // 已扫描过（通过前面的 any-version / skills.sh 仓库目录），追加位置标签
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
                let in_global = find_store_path(&skills_dir(), &name).is_some();

                results.push(ScannedSkill {
                    name: parsed_name,
                    description,
                    directory: name,
                    full_path,
                    found_in: vec![location_label.to_string()],
                    is_symlink,
                    in_global,
                });
        }
    }
    Ok(results)
}

#[tauri::command]
pub fn import_existing_skill(skill_path: String) -> Result<(), String> {
    install_skill(skill_path)
}

/// 返回可安装技能的目标工具列表（由注册表驱动，仅包含具备技能 JUNCTION 目标的已管理工具）。
/// 修复此前硬编码工具列表导致的：mimocode/deveco/qwencode 缺失、kilocode/aider 误列等问题。
#[tauri::command]
pub fn get_skill_tools() -> Result<Vec<SkillToolInfo>, String> {
    let reg = registry();
    let mut tools: Vec<SkillToolInfo> = Vec::new();
    for id in reg.tool_ids() {
        // 仅返回具备技能 JUNCTION 目标的工具
        if reg.skills_scan().tool_skills_dirs.contains_key(id) {
            let label = reg.get_tool_config(id)
                .map(|c| c.display_name.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| id.to_string());
            tools.push(SkillToolInfo { id: id.to_string(), label });
        }
    }
    // 按展示名排序，结果稳定
    tools.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(tools)
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

        // 重建 JUNCTION 链接：扫描各工具的 skills 目录，若已存在同名 junction（说明此前部署过），
        // 则重建指向新 canonical 目录的 junction。无元数据依赖，纯靠目录推导。
        let reg = registry();
        let tool_skill_dirs: Vec<(String, PathBuf)> = reg.skills_scan().tool_skills_dirs.keys()
            .map(|t| (t.clone(), reg.resolve_skill_junction_target(t, skill_id)))
            .collect();

        let has_existing = tool_skill_dirs.iter().any(|(_, d)| d.exists());
        if has_existing {
            emit_progress("重建链接", i + 1, total, &skill.name);
            for (_tool_id, tool_dir) in &tool_skill_dirs {
                if !tool_dir.exists() {
                    continue;
                }
                let is_junction = fs::symlink_metadata(tool_dir)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                // 仅重建此前由 AnyVersion 创建的 junction；工具私有真实目录不碰
                if !is_junction {
                    continue;
                }
                let _ = fs::remove_dir(tool_dir);
                if let Some(parent) = tool_dir.parent() {
                    let _ = fs::create_dir_all(parent);
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

/// 标准化路径用于比较（统一大小写、去尾部分隔符、去除 Windows `\\?\` 前缀）
pub(crate) fn normalize_path(path: &str) -> String {
    let p = path.trim_end_matches('\\').trim_end_matches('/').to_lowercase();
    p.strip_prefix("\\\\?\\").unwrap_or(&p).to_string()
}

fn create_ai_junction(link_path: &PathBuf, target_path: &PathBuf) -> Result<(), String> {
    create_junction(link_path, target_path)
}

/// 卸载时移除该技能在所有工具目录下的 junction，避免悬空链接
fn remove_skill_junctions(skill_id: &str) {
    let reg = registry();
    for tool_id in reg.skills_scan().tool_skills_dirs.keys() {
        let target = reg.resolve_skill_junction_target(tool_id, skill_id);
        if target.exists() {
            let is_junction = fs::symlink_metadata(&target)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if is_junction {
                let _ = fs::remove_dir(&target);
            } else {
                eprintln!("[skill] 跳过删除 {}：不是 junction", target.display());
            }
        }
    }
}
// ─── Skills.sh 本地安装集成 ───

/// 安装进度（新技能安装时实时推送）
#[derive(Serialize, Clone, Debug)]
pub struct SkillInstallProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub skill_name: String,
    pub message: String,
}

/// 向所有窗口推送安装进度事件（忽略发送失败，不阻断安装流程）
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

/// 从在线路径安装 skill：clone 到 anyversion 核心 skill 仓库，再通过 JUNCTION 链接给各工具
#[tauri::command]
pub async fn install_skill_from_online(
    app: tauri::AppHandle,
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
    emit_install_progress(&app, "准备", 0, 0, "", "准备安装技能...");

    // 解析为 Git URL
    let repo_url = if src_trimmed.starts_with("http://") || src_trimmed.starts_with("https://") {
        src_trimmed.to_string()
    } else if src_trimmed.contains('/') && !src_trimmed.contains('\\') {
        format!("https://github.com/{}", src_trimmed)
    } else {
        // 本地路径
        let local_path = PathBuf::from(src_trimmed);
        if local_path.exists() && local_path.is_dir() {
            return install_skill_with_junctions(local_path.to_string_lossy().to_string(), &target_tools, &app);
        }
        return Err("无效的来源格式（需要 Git URL 或 owner/repo）".to_string());
    };

    // 1. Git clone 到临时目录
    let temp_dir = get_base_dir().join("_temp_skill_clone");
    let _ = fs::remove_dir_all(&temp_dir);
    emit_install_progress(&app, "克隆", 0, 0, "", "正在克隆技能源仓库...");

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
    let result = install_skill_with_junctions(temp_dir.to_string_lossy().to_string(), &target_tools, &app);
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// 安装 skill：先复制到核心仓库，再为每个工具创建 JUNCTION。
/// `app` 用于实时推送安装进度事件。
fn install_skill_with_junctions(src_dir: String, target_tools: &[String], app: &tauri::AppHandle) -> Result<(), String> {
    let src = PathBuf::from(&src_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 从 SKILL.md 读取名称（优先 frontmatter）
    let skill_md = src.join("SKILL.md");
    let folder_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        parse_skill_md(&content, &folder_name)
    } else {
        (folder_name.clone(), folder_name.clone())
    };

    let id = name.to_lowercase().replace(' ', "-");
    let total = target_tools.len() + 1;

    // 1. 复制到核心 skill 仓库（默认 ~/.any-version/skills）`<id>`/
    emit_install_progress(app, "复制到仓库", 1, total, &name, "正在安装到 AnyVersion 技能仓库...");
    let canonical_dir = skills_dir().join(&id);
    if canonical_dir.exists() {
        let _ = fs::remove_dir_all(&canonical_dir);
    }
    copy_dir_recursive(&src, &canonical_dir)?;
    emit_install_progress(app, "已复制", 1, total, &name, "已安装到仓库");

    // 2. 为每个目标工具创建 JUNCTION（路径由 registry JSON 配置驱动）
    let tool_skill_dirs: Vec<(String, PathBuf)> = target_tools.iter().map(|t| {
        (t.clone(), registry().resolve_skill_junction_target(t, &id))
    }).collect();

    for (i, (tool_id, tool_dir)) in tool_skill_dirs.iter().enumerate() {
        emit_install_progress(
            app,
            "链接工具",
            2 + i,
            total,
            &name,
            &format!("正在链接到工具 {}", tool_id),
        );
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
        }
    }
    emit_install_progress(app, "完成", total, total, &name, "安装完成！已创建 junction 链接");

    // 3. 保存到 skills.json（安装状态由 get_skills 扫描目录实时推导，无需在此持久化）
    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != id);
    skills.skills.push(Skill {
        id: id.clone(),
        name: name.clone(),
        description,
        directory: canonical_dir.to_string_lossy().to_string(),
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
