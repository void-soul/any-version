use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::Emitter;
use crate::commands::config::get_data_dir;
use crate::commands::cache::create_junction;
use super::config::load_ai_config;
use super::models::*;

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
        .replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default())
        .replace("%APPDATA%", &std::env::var("APPDATA").unwrap_or_default())
        .replace("%PROGRAMFILES%", &std::env::var("ProgramFiles").unwrap_or_default());
    let mut p = PathBuf::from(resolved);
    if p.is_relative() {
        p = get_data_dir().join(&p);
    }
    p
}

/// 当前技能库目录：`AiConfig.skills_dir` 为空时用默认（数据目录下的 skills）。
///
/// 这是**第一个**技能库（也就是「默认库」）：新增技能默认装进它。
/// 用户可自定义（设置入口在技能管理页），但**默认值刻意避开** `~/.agents/skills`——
/// 那是原生扫描目录，放进去的技能会对所有项目全局生效。详见 `default_skills_dir`。
pub(crate) fn skills_dir() -> PathBuf {
    skill_stores()
        .into_iter()
        .next()
        .unwrap_or_else(default_skills_dir)
}

/// 库路径的比较键：展开 8.3 短名 + 统一分隔符与大小写。
///
/// Windows 上不归一就会出两类玄学问题：`ADMINI~1` 与 `Administrator` 认不出是同一个目录；
/// `C:\x` 与 `c:/x` 也认不出。前者会让「已添加的库」变成两个，后者会让 junction 匹配失败。
fn store_key(p: &Path) -> String {
    let real = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// 全部技能库目录：**默认库 + 用户追加的库**。
///
/// 已解析成绝对路径（支持 `~` / `%VAR%`）、按 store_key 去重，默认库永远排第一。
/// 界面上每个库是一个 tab；同名技能在不同库里各存一份（`.meta.json` 也在各库根下）。
pub(crate) fn skill_stores() -> Vec<PathBuf> {
    let cfg = load_ai_config();
    let mut out: Vec<PathBuf> = Vec::new();
    for raw in std::iter::once(&cfg.skills_dir).chain(cfg.extra_skills_dirs.iter()) {
        let p = resolve_skills_dir(raw);
        let key = store_key(&p);
        if !out.iter().any(|x| store_key(x) == key) {
            out.push(p);
        }
    }
    if out.is_empty() {
        out.push(default_skills_dir());
    }
    out
}

/// 命令的 `store` 参数解析：空 = 自动挑一个库。
///
/// 顺序是「显式指定 → 有该技能的库 → 默认库」。自动挑「有该技能的库」这条很关键：
/// 同名技能可能同时存在于多个库里，命令只拿到 id 时必须挑对，否则会出现
/// 「A 库的命令读到了 B 库同名技能」这种最难查的错位。
fn resolve_store_arg(store: &str, skill_id: &str) -> PathBuf {
    let stores = skill_stores();
    let trimmed = store.trim();
    if !trimmed.is_empty() {
        let want = store_key(&resolve_skills_dir(trimmed));
        if let Some(p) = stores.iter().find(|p| store_key(p) == want) {
            return p.clone();
        }
    }
    if !skill_id.is_empty() {
        if let Some(p) = stores.iter().find(|p| p.join(skill_id).is_dir()) {
            return p.clone();
        }
    }
    stores.into_iter().next().unwrap_or_else(default_skills_dir)
}

/// 库里最后一个有内容的目录名，用作 tab 标题（`team-skills` → `team-skills`）。
/// 取不到就用完整路径的末段兜底，再不行给个通用名。
fn store_display_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| p.to_string_lossy().to_string())
}

/// 技能库视图（前端一个 tab = 一条）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillStoreView {
    /// 绝对路径：前端把它当作库的稳定标识传回命令
    pub path: String,
    /// tab 标题（目录名）
    pub name: String,
    /// 第一个库：新增技能默认装进它
    pub is_default: bool,
    /// 目录是否已存在（配好了路径但还没建目录时为 false，界面要能如实显示）
    pub exists: bool,
    /// 目录下的技能数
    pub skill_count: usize,
}

fn store_views() -> Vec<SkillStoreView> {
    skill_stores()
        .into_iter()
        .enumerate()
        .map(|(index, p)| {
            let exists = p.is_dir();
            let skill_count = if exists {
                fs::read_dir(&p)
                    .map(|entries| {
                        entries
                            .flatten()
                            .filter(|e| {
                                let name = e.file_name().to_string_lossy().to_string();
                                !name.starts_with('.') && e.path().is_dir()
                            })
                            .count()
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            SkillStoreView {
                path: p.to_string_lossy().to_string(),
                name: store_display_name(&p),
                is_default: index == 0,
                exists,
                skill_count,
            }
        })
        .collect()
}

/// 当前生效的技能库目录（前端要如实显示「技能装到哪了」）。
#[tauri::command]
pub fn get_skills_dir() -> String {
    skills_dir().to_string_lossy().to_string()
}

/// 列出全部技能库（前端渲染成 tabs）
#[tauri::command]
pub fn list_skill_stores() -> Vec<SkillStoreView> {
    store_views()
}

/// 追加一个技能库目录。**不搬动任何文件**，只是把路径记进配置。
#[tauri::command]
pub fn add_skill_store(path: String) -> Result<Vec<SkillStoreView>, String> {
    let mut cfg = load_ai_config();
    let resolved = resolve_skills_dir(&path);
    let key = store_key(&resolved);
    // 与已有库重复：直接当成功处理（幂等），否则用户会以为「加了两次」
    if skill_stores().iter().any(|p| store_key(p) == key) {
        return Ok(store_views());
    }
    // 禁止把某个库加进另一个库里：junction 的目标路径前缀匹配会变得含糊
    // （`A/a-skill` 与 `A/B/a-skill` 谁才是「技能所在」），排查起来极其痛苦。
    for existing in skill_stores() {
        let ek = store_key(&existing);
        if key.starts_with(&format!("{}/", ek.trim_end_matches('/'))) {
            return Err(format!(
                "「{}」在已添加的库「{}」里面，不能再作为独立的技能库",
                resolved.display(),
                existing.display()
            ));
        }
    }
    if !resolved.is_dir() {
        // 路径不存在不拦着：用户可能打算先把技能拷进去。目录由装第一个技能时创建。
        let _ = fs::create_dir_all(&resolved);
    }
    cfg.extra_skills_dirs.push(resolved.to_string_lossy().to_string());
    super::config::save_ai_config_to_file(&cfg)?;
    Ok(store_views())
}

/// 移除一个技能库（**只删配置记录，不删目录，也不动项目里的 junction**）。
///
/// 刻意不做「连带删除」：那等于替用户删数据。移除后该库里的技能不再出现在列表里，
/// 已经装到项目里的 junction 会变成悬空链接 —— UI 会标成 broken 并给清理入口。
#[tauri::command]
pub fn remove_skill_store(path: String) -> Result<Vec<SkillStoreView>, String> {
    let mut cfg = load_ai_config();
    let key = store_key(&resolve_skills_dir(&path));
    if cfg.extra_skills_dirs.is_empty() {
        return Err("默认库不能在这里移除；改位置请用「修改默认库」".to_string());
    }
    let before = cfg.extra_skills_dirs.len();
    cfg.extra_skills_dirs
        .retain(|raw| store_key(&resolve_skills_dir(raw)) != key);
    if cfg.extra_skills_dirs.len() == before {
        return Err("这个库不在列表里".to_string());
    }
    super::config::save_ai_config_to_file(&cfg)?;
    Ok(store_views())
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
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let link = entry.path();
            let is_link = fs::symlink_metadata(&link)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_link {
                continue;
            }
            let Ok(target) = fs::read_link(&link) else { continue };
            // 只动指向旧库的；用户自己建的、指向别处的链接不碰
            let Ok(rel) = target.strip_prefix(old_dir) else { continue };
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
        result.errors.push(format!("创建 {} 失败", new_path.display()));
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

// ─── 项目（安装作用域） ───
//
// 技能装给项目 = junction 到 `<项目>/.agents/skills/<技能 id>`。
// 早先的模型是「按工具」（给某个 IDE / 全局装一份），现已改为「按项目」：
// 真正需要技能的是项目，IDE 只是读取技能的宿主。

/// 项目条目：本质就是一个目录路径。
///
/// 刻意**不存**「这个项目装了哪些技能」—— 安装状态靠扫描目标目录推断。
/// 持久化清单与磁盘实况必然会漂移（用户手动删 junction、技能库删技能、移动项目目录），
/// 扫描推断则永远等于真实状态。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillProject {
    pub id: String,
    /// 规范化后的绝对路径（add_project 时转绝对路径并去重）
    pub path: String,
    /// 目录名，仅用于列表展示
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub added_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProjectsFile {
    #[serde(default)]
    pub projects: Vec<SkillProject>,
}

fn projects_path() -> PathBuf {
    get_data_dir().join("projects.json")
}

fn load_projects() -> ProjectsFile {
    load_projects_from(&projects_path())
}

/// 读写分离出「路径可注入」的版本：单测必须能指向临时目录，
/// 否则测试会往用户真实的数据目录里写 projects.json。
fn load_projects_from(path: &Path) -> ProjectsFile {
    if let Ok(data) = fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<ProjectsFile>(&data) {
            return v;
        }
    }
    ProjectsFile::default()
}

fn save_projects_to(path: &Path, f: &ProjectsFile) -> Result<(), String> {
    let data = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(path, data.as_bytes())
}

/// 列出全部项目（安装作用域）。
#[tauri::command]
pub fn list_projects() -> Vec<SkillProject> {
    load_projects().projects
}

/// 添加项目：只收「存在且确实是目录」的路径，规范化后按同一路径去重。
#[tauri::command]
pub fn add_project(path: String) -> Result<SkillProject, String> {
    add_project_at(&projects_path(), &path)
}

fn add_project_at(file: &Path, raw: &str) -> Result<SkillProject, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("项目路径不能为空".to_string());
    }
    let abs = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        std::env::current_dir().unwrap_or_default().join(trimmed)
    };
    if !abs.is_dir() {
        return Err(format!("不是有效的目录：{}", abs.display()));
    }
    let mut f = load_projects_from(file);
    let norm = normalize_path(&abs.to_string_lossy());
    if f.projects.iter().any(|p| normalize_path(&p.path) == norm) {
        return Err("该项目已添加".to_string());
    }
    let project = SkillProject {
        id: gen_project_id(),
        path: abs.to_string_lossy().to_string(),
        name: abs
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        added_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    f.projects.push(project.clone());
    save_projects_to(file, &f)?;
    Ok(project)
}

/// 移除项目条目：**只删清单里的这一行**，项目目录里的 `.agents/skills` 一概不动
/// （junction 是技能库指向项目的链接，删它等于删技能的入口，需用户自己另行卸载）。
#[tauri::command]
pub fn remove_project(id: String) -> Result<(), String> {
    remove_project_at(&projects_path(), &id)
}

fn remove_project_at(file: &Path, id: &str) -> Result<(), String> {
    let mut f = load_projects_from(file);
    let before = f.projects.len();
    f.projects.retain(|p| p.id != id);
    if f.projects.len() == before {
        return Err("项目不存在".to_string());
    }
    save_projects_to(file, &f)
}

/// 项目 id：时间戳 + 进程号（同进程内纳秒级不重复，与 temp_clone_dir 同一风格）。
fn gen_project_id() -> String {
    let stamp = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0);
    format!("p{}_{}", std::process::id(), stamp)
}

// ─── 安装到项目（junction） ───
//
// 安装 = 在 `<项目>/.agents/skills/<技能id>` 建一个指向技能库该技能的目录联接。
// 卸载 = 只删链接本身。两者都不在项目目录里放任何实体文件，所以「装给项目」
// 不会让技能副本散落到各处，技能库一改，所有项目立刻同步。

/// 解析项目记录 → `<项目>/.agents/skills`，顺带校验项目目录还在。
/// 清单路径可注入（单测用；否则测试会写用户真实的数据目录）。
fn resolve_project_skills_dir_in(file: &Path, project_id: &str) -> Result<PathBuf, String> {
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
    Ok(Path::new(&p.path).join(".agents").join("skills"))
}

/// 这个路径是不是「指向技能库内某个技能」的 junction？返回那个技能 id。
///
/// 用 `read_link` 解析真实目标，而不是「目录名在技能库里存在就算」——
/// 后者会把用户自己建的同名链接、以及指错了位置的链接误判成已安装。
/// 悬空 junction（目标已删）返回 None：它是链接，但不再指向任何现存技能。
fn junction_target_skill(link: &Path, stores: &[PathBuf]) -> Option<(PathBuf, String)> {
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
    // 自己归一分隔符：normalize_path 只统一大小写与**末尾**分隔符，中间的反斜杠不管，
    // 直接拿它做 prefix 匹配会永远失败。
    let t = t.to_string_lossy().replace('\\', "/").to_lowercase();
    // 按库路径长度倒序匹配：库之间理论上不会互相嵌套（add_skill_store 已禁止），
    // 但用户可能手工摆成 `A` 与 `A/B`。先长后短能保证命中最具体的那个库。
    let mut candidates: Vec<(PathBuf, String)> = stores
        .iter()
        .map(|s| {
            let real = fs::canonicalize(s).unwrap_or_else(|_| s.to_path_buf());
            (s.clone(), format!("{}/", store_key(&real)))
        })
        .collect();
    candidates.sort_by_key(|(_, prefix)| std::cmp::Reverse(prefix.len()));
    for (store, prefix) in candidates {
        if let Some(rest) = t.strip_prefix(&prefix) {
            let id = rest.split('/').next().unwrap_or("").to_string();
            if !id.is_empty() {
                return Some((store, id));
            }
        }
    }
    None
}

/// 把技能装到项目。返回 junction 的实际路径。
///
/// `store` 为空时自动挑库（优先「有该技能」的库）。同名技能在多个库里都存在时，
/// **后装的覆盖先装的**（与安装到技能库本身的语义一致）。
#[tauri::command]
pub fn install_skill_to_project(
    skill_id: String,
    project_id: String,
    store: Option<String>,
) -> Result<String, String> {
    let store = resolve_store_arg(store.as_deref().unwrap_or(""), &skill_id);
    install_into(&store, &projects_path(), &skill_id, &project_id)
}

fn install_into(store: &Path, projects_file: &Path, skill_id: &str, project_id: &str) -> Result<String, String> {
    let skill_dir = store.join(skill_id);
    if !skill_dir.is_dir() {
        return Err(format!("技能库中不存在技能「{}」", skill_id));
    }
    let dir = resolve_project_skills_dir_in(projects_file, project_id)?;
    fs::create_dir_all(&dir).map_err(|e| format!("创建 {} 失败: {}", dir.display(), e))?;
    let link = dir.join(skill_id);

    // 三种前置状态：真实目录（覆盖）/ 已正确链接（幂等）/ 指向别处或悬空（重建）
    if let Ok(md) = fs::symlink_metadata(&link) {
        if !md.file_type().is_symlink() {
            // 真实目录 = 项目自带的同名技能：**允许覆盖**，先整个删掉再建链接。
            // 不可逆操作，所以 UI 在应用前必须把「会被替换的真实目录」列给用户确认。
            // （早先这里是拒绝，因为怕无声吃掉用户数据；现在用户明确要求同名可覆盖。）
            fs::remove_dir_all(&link)
                .map_err(|e| format!("无法替换已存在的目录 {}: {}", link.display(), e))?;
        } else {
            // 是链接：已正确指向本技能则幂等成功
            if junction_target_skill(&link, &[store.to_path_buf()]).is_some_and(|(_, id)| id == skill_id) {
                return Ok(link.to_string_lossy().to_string());
            }
            // 指向别处 / 悬空：交给 create_junction 删链接后重建
        }
    }

    // junction 跨本地卷可行，但不支持网络路径（UNC）：先拦一道，避免 mklink 报错难懂
    if skill_dir.to_string_lossy().starts_with(r"\\") {
        return Err("技能库位于网络路径（UNC）时无法创建目录联接，请把技能库移到本地磁盘".to_string());
    }
    create_junction(&link, &skill_dir).map_err(|e| format!("安装失败: {}", e))?;
    Ok(link.to_string_lossy().to_string())
}

/// 从项目卸载技能。
///
/// - **链接**（指向技能库）：只删链接本身。Windows 上删 junction 必须用 remove_dir ——
///   若用会跟随删除的操作，删掉的会是**技能库里的实体技能内容**。这里刻意不用 remove_dir_all。
/// - **真实目录**（项目自带的技能）：按用户要求与链接同等对待，整个删除。
///   不可逆，UI 必须先把「会被删除的真实目录」列出来让用户确认。
#[tauri::command]
pub fn remove_skill_from_project(skill_id: String, project_id: String) -> Result<(), String> {
    uninstall_from(&projects_path(), &skill_id, &project_id)
}

fn uninstall_from(projects_file: &Path, skill_id: &str, project_id: &str) -> Result<(), String> {
    let dir = resolve_project_skills_dir_in(projects_file, project_id)?;
    let link = dir.join(skill_id);
    let md = fs::symlink_metadata(&link).map_err(|_| format!("该项目未安装技能「{}」", skill_id))?;
    if md.file_type().is_symlink() {
        if fs::remove_dir(&link).is_err() {
            // 占用 / 权限等情况下 remove_dir 可能失败，rmdir 同样只删链接本身
            fs::remove_file(&link).map_err(|e| format!("卸载失败: {}", e))?;
        }
    } else {
        fs::remove_dir_all(&link).map_err(|e| format!("卸载失败 {}: {}", link.display(), e))?;
    }
    Ok(())
}

/// 项目里已安装的技能（扫描推断，不持久化）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillEntry {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    /// `linked` 指向技能库的链接 · `local` 项目自带的真实目录 · `broken` 悬空链接（技能库里的技能已被删除）
    pub state: String,
    /// SKILL.md 字符数（与技能库列表同一指标，便于两边对照）
    pub skill_md_chars: usize,
    /// 技能库 `.meta.json` 里设的标签 —— 项目列表要跟着显示，
    /// 否则用户在技能库整理的分类/标签到了项目里就全丢了。
    pub tags: Vec<String>,
    pub category: String,
}

/// 取项目里某个技能的 SKILL.md 全文，供 UI 渲染预览。
///
/// **按需取**而不是随列表一起返回：一份 SKILL.md 动辄几千字符，几十个技能
/// 一起塞进列表会把 IPC 撑大，而用户通常一次只看一个。
#[tauri::command]
pub fn get_project_skill_md(project_id: String, skill_id: String) -> Result<String, String> {
    let dir = resolve_project_skills_dir_in(&projects_path(), &project_id)?;
    let path = dir.join(&skill_id).join("SKILL.md");
    // junction 可穿透：读到的是技能库里的实体文件
    fs::read_to_string(&path).map_err(|e| format!("{} 读取失败: {}", path.display(), e))
}

/// 取技能库里某个技能的 SKILL.md 全文，供 UI 渲染预览。
///
/// 与 `get_project_skill_md` 分开而不是合并成一个带可选参数的命令：两个入口的
/// 失败语义不同（项目里可能是悬空链接，技能库里是真的没这个技能），分开报错更准。
#[tauri::command]
pub fn get_library_skill_md(skill_id: String, store: Option<String>) -> Result<String, String> {
    let dir = resolve_store_arg(store.as_deref().unwrap_or(""), &skill_id);
    let path = dir.join(&skill_id).join("SKILL.md");
    fs::read_to_string(&path).map_err(|e| format!("{} 读取失败: {}", path.display(), e))
}

/// 列出某个项目装了什么：扫 `.agents/skills`，看哪些是指向技能库的 junction。
///
/// 状态不落盘 —— 清单与实况必然会漂移（用户手动删链接、技能库删技能、移动项目），
/// 扫描推断则永远等于磁盘真实状态。
#[tauri::command]
pub fn list_project_skills(project_id: String) -> Result<Vec<ProjectSkillEntry>, String> {
    list_installed(&skill_stores(), &projects_path(), &project_id)
}

fn list_installed(stores: &[PathBuf], projects_file: &Path, project_id: &str) -> Result<Vec<ProjectSkillEntry>, String> {
    let dir = resolve_project_skills_dir_in(projects_file, project_id)?;
    let mut out: Vec<ProjectSkillEntry> = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        // 项目还没装过任何技能（.agents/skills 不存在）：返回空而不是报错
        Err(_) => return Ok(out),
    };
    // 每个库的 .meta.json 各读一次（标签跟着库走：同名技能在不同库里可以有不同标签）；
    // 每个库读一次就够，别在循环里反复读盘
    let metas: Vec<(PathBuf, SkillMetaFile)> = stores
        .iter()
        .map(|s| (s.clone(), load_skill_meta_from(s)))
        .collect();
    let meta_of = |store: &Path| -> SkillMetaFile {
        metas
            .iter()
            .find(|(p, _)| store_key(p) == store_key(store))
            .map(|(_, m)| m.clone())
            .unwrap_or_default()
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let fallback_id = entry.file_name().to_string_lossy().to_string();
        if fallback_id.starts_with('.') {
            continue;
        }
        if !fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            // 真实目录 = 项目**自带**的技能（用户手工维护、没进技能库）。
            // 同样要认出来：不然它在 UI 上完全隐形，用户既看不到也清不掉。
            // 名字/描述就地读它自己的 SKILL.md，不去技能库找。
            let md = path.join("SKILL.md");
            let (name, description, chars) = if md.is_file() {
                let c = fs::read_to_string(&md).unwrap_or_default();
                let (n, d) = parse_skill_md(&c, &fallback_id);
                (n, d, c.chars().count())
            } else {
                (fallback_id.clone(), String::new(), 0)
            };
            out.push(ProjectSkillEntry {
                skill_id: fallback_id.clone(),
                name,
                description,
                state: "local".to_string(),
                skill_md_chars: chars,
                // 项目自带技能不在技能库里，没有 .meta.json 可查
                tags: Vec::new(),
                category: String::new(),
            });
            continue;
        }
        // junction 归属哪个库由真实目标决定（多库时要认得出是哪一个）
        let target = junction_target_skill(&path, stores);
        let (hit_store, skill_id, state) = match target {
            Some((store, id)) if store.join(&id).is_dir() => (Some(store), id, "linked"),
            // 是链接但目标已不在任何技能库里 → 悬空，UI 要能标出来并给清理入口
            _ => (None, fallback_id.clone(), "broken"),
        };
        let meta = hit_store.as_ref().map(|s| meta_of(s)).unwrap_or_default();
        let (name, description, chars) = match state {
            "linked" => {
                let md = hit_store
                    .as_ref()
                    .map(|s| s.join(&skill_id).join("SKILL.md"))
                    .unwrap_or_default();
                if md.is_file() {
                    let c = fs::read_to_string(&md).unwrap_or_default();
                    let (n, d) = parse_skill_md(&c, &skill_id);
                    (n, d, c.chars().count())
                } else {
                    (skill_id.clone(), String::new(), 0)
                }
            }
            _ => (fallback_id.clone(), String::new(), 0),
        };
        // 标签/分类从技能库的 .meta.json 取：用户在技能库整理的元数据
        // 在项目列表里也要看得见，否则两边的信息会各说各话。
        let m = meta.skills.get(&skill_id);
        let tags = m.map(|x| x.tags.clone()).unwrap_or_default();
        let category = m.map(|x| x.category.clone()).unwrap_or_default();
        out.push(ProjectSkillEntry {
            skill_id,
            name,
            description,
            state: state.to_string(),
            skill_md_chars: chars,
            tags,
            category,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

// ─── 项目技能计数（一次扫完所有项目） ───

/// 单个项目的技能计数
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSkillCount {
    pub project_id: String,
    /// 有效技能总数（= linked + local，**不含** broken）
    pub total: usize,
    /// 来自技能库（junction 指到某个库里真实存在的技能）
    pub linked: usize,
    /// 项目自带的真实目录（用户手工维护、没进技能库）
    pub local: usize,
    /// 悬空链接：是指向技能库的链接，但目标已不在任何库里（最容易被漏掉的一类）
    pub broken: usize,
}

/// 一次扫完所有项目的技能计数。
///
/// 与 `list_project_skills` 的区别是**只数、不读**：不碰 `SKILL.md`，不解析名称/描述/字符数。
/// 项目列表里每行都要显示技能数，如果用完整清单就得为每个项目的每个技能读一次盘——
/// 几十个项目就是上百次读盘，只为了拿一个数字。
///
/// 判定口径与 `list_installed` 严格一致（含「非点目录一律当项目自带」这条），
/// 否则列表上显示的数字会和点进去看到的条目数对不上。
#[tauri::command]
pub fn list_project_skill_counts() -> Vec<ProjectSkillCount> {
    count_project_skills(&projects_path(), &skill_stores())
}

/// 可注入的版本（单测必须能指向临时 projects 文件，否则会读用户真实清单）
fn count_project_skills(projects_file: &Path, stores: &[PathBuf]) -> Vec<ProjectSkillCount> {
    let mut out: Vec<ProjectSkillCount> = Vec::new();
    for p in load_projects_from(projects_file).projects {
        let dir = Path::new(&p.path).join(".agents").join("skills");
        let (mut linked, mut local, mut broken) = (0usize, 0usize, 0usize);
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let is_link = fs::symlink_metadata(&path)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if !is_link {
                    // 真实目录 = 项目自带（与 list_installed 同口径：不额外要求 is_dir，
                    // 否则这里数出来的会比点进去看到的条目数少）
                    local += 1;
                    continue;
                }
                // 是链接：真实目标还落在某个库里才算 linked，否则算悬空
                match junction_target_skill(&path, &stores) {
                    Some((store, id)) if store.join(&id).is_dir() => linked += 1,
                    _ => broken += 1,
                }
            }
        }
        out.push(ProjectSkillCount {
            project_id: p.id,
            total: linked + local,
            linked,
            local,
            broken,
        });
    }
    out
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

/// 元数据文件路径：`<技能库>/.meta.json`（点文件，扫描时跳过）
///
/// **每个库各有一份** `.meta.json`：标签/分类跟着它所在的那个库走，
/// 所以同名技能在两个库里可以被打上不同的分类与标签。
fn skill_meta_path(store: &Path) -> PathBuf {
    store.join(".meta.json")
}

fn load_skill_meta_from(store: &Path) -> SkillMetaFile {
    let path = skill_meta_path(store);
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(meta) = serde_json::from_str::<SkillMetaFile>(&data) {
                return meta;
            }
        }
    }
    SkillMetaFile::default()
}

fn save_skill_meta_to(store: &Path, meta: &SkillMetaFile) -> Result<(), String> {
    let data = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&skill_meta_path(store), data.as_bytes())
}

/// 默认库的元数据（AI 扫描等只需要主库的场景）
fn load_skill_meta() -> SkillMetaFile {
    load_skill_meta_from(&skills_dir())
}

fn save_skill_meta(meta: &SkillMetaFile) -> Result<(), String> {
    save_skill_meta_to(&skills_dir(), meta)
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
    /// SKILL.md 的字符数。UI 拿它一眼看出「这个技能有多重」——
    /// 描述在每次会话都常驻，而正文只在用到时读，字符数是这个技能的成本指标。
    pub skill_md_chars: usize,
    /// 所属技能库的绝对路径（多库后前端据此分 tab、并把它作为命令的 store 参数传回）
    pub store: String,
    /// 所属库的显示名（目录名），冗余一份免得前端自己再切路径
    pub store_name: String,
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

/// 判断路径是否为 junction/symlink
fn is_junction(path: &PathBuf) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// ─── 技能列表（仓库扫描） ───

/// 技能总览：扫描技能库目录（默认「数据目录下的 skills」）。
/// 手工拷进去的技能同样会被列出来——它和安装进来的技能没有区别对待。
#[tauri::command]
pub fn get_skill_overview() -> Result<Vec<SkillEntry>, String> {
    // 多库：逐个库扫描，每条记录带上它所属的库（前端据此分 tab，并把它作为 store 参数传回命令）
    let manifest = load_skills();
    let mut entries: Vec<SkillEntry> = Vec::new();
    for store in skill_stores() {
        collect_store_entries(&store, &manifest, &mut entries)?;
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(entries)
}

/// 扫一个技能库的目录，把技能条目追加进 `out`。
///
/// 拆出来是因为多库后这段逻辑要对每个库各跑一遍，而 `.meta.json` 必须**按库**读——
/// 同名技能在两个库里可能有不同标签，用默认库那份会把其中一个的标签串到另一个上。
fn collect_store_entries(
    store: &Path,
    manifest: &SkillsFile,
    entries: &mut Vec<SkillEntry>,
) -> Result<(), String> {
    let meta = load_skill_meta_from(store);
    let store_path = store.to_string_lossy().to_string();
    let store_name = store_display_name(store);
    if !store.exists() {
        return Ok(());
    }

    let dir_entries = fs::read_dir(store).map_err(|e| e.to_string())?;
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
        let (name, description, skill_md_chars) = if skill_md.is_file() {
            let c = fs::read_to_string(&skill_md).unwrap_or_default();
            let (n, d) = parse_skill_md(&c, &id);
            (n, d, c.chars().count())
        } else {
            (id.clone(), String::new(), 0)
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
            skill_md_chars,
            store: store_path.clone(),
            store_name: store_name.clone(),
        });
    }
    Ok(())
}

/// 返回全部技能元数据（分类/标签），用于前端构建筛选选项
#[tauri::command]
pub fn get_skill_meta(store: Option<String>) -> Result<HashMap<String, SkillMetaItemDto>, String> {
    // 每个库的分类/标签各自一份，筛选选项也要按当前 tab 的库来
    let dir = resolve_store_arg(store.as_deref().unwrap_or(""), "");
    let meta = load_skill_meta_from(&dir);
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
    store: Option<String>,
) -> Result<(), String> {
    if skill_id.is_empty() {
        return Err("技能 id 不能为空".to_string());
    }
    // 元数据写进**该技能所在那个库**：同名技能在两个库里各存一份标签，互不干扰
    let dir = resolve_store_arg(store.as_deref().unwrap_or(""), &skill_id);
    let mut meta = load_skill_meta_from(&dir);
    let tags: Vec<String> = tags.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    meta.skills.insert(skill_id.clone(), SkillMetaItem { category: category.trim().to_string(), tags });
    save_skill_meta_to(&dir, &meta)
}

/// 元数据项 DTO（前端筛选用）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetaItemDto {
    pub category: String,
    pub tags: Vec<String>,
}

// ─── 克隆来源解析（子目录标记 / 技能根定位 / 临时目录） ───

/// 拆分来源里的子目录标记：`owner/repo#skills/foo` → (`owner/repo`, Some("skills/foo"))。
///
/// 一个仓库里放多个技能时需要指定装哪个；也是「技能 id 与目录名不一致」时的补救入口
/// （参考 cc-switch「support mismatched skillId directories / preserve source path during updates」）。
fn split_source_subdir(source: &str) -> (String, Option<String>) {
    match source.split_once('#') {
        Some((base, sub)) => {
            let base = base.trim();
            let sub = sub.trim().trim_matches('/');
            if base.is_empty() || sub.is_empty() {
                (source.to_string(), None)
            } else {
                (base.to_string(), Some(sub.to_string()))
            }
        }
        None => (source.to_string(), None),
    }
}

/// SKILL.md 的最深搜索层级：仓库根 → `*/` → `*/*`。
const SKILL_ROOT_MAX_DEPTH: usize = 2;

/// 在克隆下来的仓库里定位真正的技能目录。
///
/// 此前直接把 clone 目录当技能装：当仓库根没有 SKILL.md（`skills/<name>` 布局、
/// 或多技能集合仓库）时，技能 id 会退化成临时目录名，之后更新 / 卸载就找不到同一个技能。
///
/// 0 个 → 报错（不是技能仓库）；1 个 → 直接用；多个 → 报错并列出候选，让用户用 `#子目录` 指定。
fn find_skill_root(clone_dir: &std::path::Path, explicit_subdir: Option<&str>) -> Result<PathBuf, String> {
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
fn temp_clone_dir() -> PathBuf {
    let stamp = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0);
    get_data_dir().join(format!(
        "_temp_skill_clone_{}_{}",
        std::process::id(),
        stamp
    ))
}

// ─── 安装 / 卸载 ───

#[tauri::command]
pub fn install_skill(skill_dir: String, store: Option<String>) -> Result<(), String> {
    install_skill_from_dir(&skill_dir, None, store.as_deref()).map(|_| ())
}

/// 安装技能到仓库，返回技能 id。
///
/// `origin` = `(来源, commit sha)`：在线安装时传，用来支持后续的「有没有更新」比对；
/// 本地目录安装传 `None`（没有远端可取）。
pub(crate) fn install_skill_from_dir(
    skill_dir: &str,
    origin: Option<(&str, &str)>,
    store: Option<&str>,
) -> Result<String, String> {
    let src = PathBuf::from(skill_dir);
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
    // 装进用户当前选中的那个库（不传 = 默认库）
    let store = resolve_store_arg(store.unwrap_or(""), "");
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
        install_method: if source.is_empty() { "local".to_string() } else { "online".to_string() },
        source,
        commit,
    });
    save_skills(&skills)?;
    Ok(id)
}

#[tauri::command]
pub fn uninstall_skill(skill_id: String, store: Option<String>) -> Result<(), String> {
    // 删的是「用户当前所在那个库」里的技能：同名技能在多个库并存时，
    // 删错库比不删更糟，所以不猜——前端一定把 store 传过来
    let store_dir = resolve_store_arg(store.as_deref().unwrap_or(""), &skill_id);
    let store = store_dir;
    let dir = store.join(&skill_id);

    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }

    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != skill_id);
    save_skills(&skills)
}

#[tauri::command]
pub fn get_skill_files(skill_id: String, store: Option<String>) -> Result<(String, Vec<SkillFile>), String> {
    let store_dir = resolve_store_arg(store.as_deref().unwrap_or(""), &skill_id);
    let dir = store_dir.join(&skill_id);
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
pub async fn install_skill_from_source(source: String, store: Option<String>) -> Result<(), String> {
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
        return install_skill(root.to_string_lossy().to_string(), store);
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
            "clone",
            "--depth",
            "1",
            &repo_url,
        ]))
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

    // 定位技能根：不再把整个 clone 目录当技能，否则技能 id 会退化成临时目录名
    let root = match find_skill_root(&temp_dir, subdir.as_deref()) {
        Ok(root) => root,
        Err(e) => {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(e);
        }
    };
    let sha = rev_parse_head(&temp_dir).await.unwrap_or_default();
    let result = install_skill_from_dir(
        &root.to_string_lossy(),
        Some((&base, &sha)),
        store.as_deref(),
    )
    .map(|_| ());
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// 从在线源安装技能到仓库（已管理工具通过 junction 自动获得）
#[tauri::command]
pub async fn install_skill_from_online(
    app: tauri::AppHandle,
    source: String,
    store: Option<String>,
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
        let result = install_skill(root.to_string_lossy().to_string(), store);
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
            "clone",
            "--depth",
            "1",
            &repo_url,
        ]))
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
    let result = install_skill_from_dir(
        &root.to_string_lossy(),
        Some((&base, &sha)),
        store.as_deref(),
    )
    .map(|_| ());
    let _ = fs::remove_dir_all(&temp_dir);

    if result.is_ok() {
        emit_install_progress(&app, "完成", 1, 1, "", "安装完成！已管理工具自动获得新技能");
    }
    result
}

// ─── 预置来源与更新检测 ───

/// 取某个 git 仓库当前 HEAD 的 sha（失败返回 None，不阻断安装）。
async fn rev_parse_head(repo_dir: &std::path::Path) -> Option<String> {
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
fn source_to_url(source: &str) -> String {
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
    ("OpenAI 官方技能", "openai/skills", "Codex / ChatGPT 官方维护的 curated 技能"),
    ("Anthropic 官方技能", "anthropics/skills", "Claude 官方技能合集"),
    ("awesome-claude-skills", "ComposioHQ/awesome-claude-skills", "社区精选，覆盖面最广"),
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

#[cfg(test)]
mod tests {
    use super::{
        add_project_at, count_project_skills, default_skills_dir, find_skill_root, install_into,
        junction_target_skill, list_installed, load_projects_from, remove_project_at,
        source_to_url, split_source_subdir, store_key, uninstall_from, PRESET_SKILL_SOURCES,
    };
    use std::path::{Path, PathBuf};

    /// `owner/repo` 要补成完整 URL 才能喂给 `git ls-remote`；已是 URL 的不能被二次拼接。
    #[test]
    fn source_url_normalizes_repo_shorthand() {
        assert_eq!(source_to_url("openai/skills"), "https://github.com/openai/skills");
        assert_eq!(
            source_to_url("https://github.com/anthropics/skills"),
            "https://github.com/anthropics/skills"
        );
    }

    /// 预置源里不能出现重复仓库（前端是按钮列表，重复会出两个一样的按钮）。
    #[test]
    fn preset_sources_are_unique_and_well_formed() {
        let mut repos: Vec<&str> = PRESET_SKILL_SOURCES.iter().map(|(_, r, _)| *r).collect();
        let total = repos.len();
        repos.sort_unstable();
        repos.dedup();
        assert_eq!(repos.len(), total, "预置源有重复仓库");
        for (label, repo, hint) in PRESET_SKILL_SOURCES {
            assert!(!label.is_empty() && !hint.is_empty(), "{repo} 缺少标签或说明");
            assert_eq!(repo.split('/').count(), 2, "{repo} 应是 owner/repo 形态");
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-skills-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo\ndescription: d\n---\n",
        )
        .unwrap();
    }

    #[test]
    fn split_source_subdir_parses_optional_subdir() {
        assert_eq!(
            split_source_subdir("owner/repo"),
            ("owner/repo".to_string(), None)
        );
        assert_eq!(
            split_source_subdir("owner/repo#skills/foo"),
            ("owner/repo".to_string(), Some("skills/foo".to_string()))
        );
        // 只有一侧有内容时按普通来源处理，避免把来源拆坏
        assert_eq!(split_source_subdir("#x"), ("#x".to_string(), None));
        assert_eq!(
            split_source_subdir("owner/repo#"),
            ("owner/repo#".to_string(), None)
        );
    }

    /// 仓库根就是技能：照旧直接用根目录。
    #[test]
    fn skill_root_prefers_repository_root() {
        let dir = temp_dir("root");
        write_skill(&dir);
        assert_eq!(find_skill_root(&dir, None).unwrap(), dir);
    }

    /// 仓库根没有 SKILL.md 时要下钻找到真正的技能目录（此前会退化成临时目录名）。
    #[test]
    fn skill_root_finds_single_nested_skill() {
        let dir = temp_dir("nested");
        write_skill(&dir.join("skills").join("ppt-master"));
        let root = find_skill_root(&dir, None).unwrap();
        assert!(root.ends_with("skills/ppt-master"), "实际: {}", root.display());
        // 技能目录名才是技能 id 的来源
        assert_eq!(root.file_name().unwrap().to_string_lossy(), "ppt-master");
    }

    #[test]
    fn skill_root_accepts_explicit_subdir_and_rejects_missing_one() {
        let dir = temp_dir("explicit");
        write_skill(&dir.join("skills").join("alpha"));
        let root = find_skill_root(&dir, Some("skills/alpha")).unwrap();
        assert!(root.ends_with("skills/alpha"));
        assert!(find_skill_root(&dir, Some("skills/missing")).is_err());
    }

    /// 多技能仓库必须报错并让用户用 `#子目录` 指定，而不是随便装一个。
    #[test]
    fn skill_root_reports_ambiguous_repositories() {
        let dir = temp_dir("ambiguous");
        write_skill(&dir.join("skills").join("alpha"));
        write_skill(&dir.join("skills").join("beta"));
        let err = find_skill_root(&dir, None).unwrap_err();
        assert!(
            err.contains("alpha") && err.contains("beta"),
            "错误信息应列出候选: {}",
            err
        );
        assert!(find_skill_root(&dir, Some("skills/beta")).is_ok());
    }

    #[test]
    fn skill_root_rejects_non_skill_repositories() {
        let dir = temp_dir("plain");
        std::fs::write(dir.join("README.md"), "no skill").unwrap();
        assert!(find_skill_root(&dir, None).is_err());
    }

    // ─── 项目（安装作用域） ───

    /// 设计不变量：默认技能库**不能**是 `~/.agents/skills`。
    ///
    /// 那是 claude-code / codex-cli / opencode / cursor / cline / firebender 的原生扫描目录，
    /// 放进去的技能会对**所有**项目全局生效，每次会话都注入全部 name + description。
    /// 这条测试是为了防止有人日后"顺手改回默认值"。
    #[test]
    fn default_skills_dir_avoids_native_agent_scan_dir() {
        let d = default_skills_dir().to_string_lossy().replace('\\', "/");
        assert!(
            !d.ends_with("/.agents/skills"),
            "默认技能库不能落在原生扫描目录：{}",
            d
        );
        assert!(d.ends_with("/skills"), "默认技能库应是数据目录下的 skills：{}", d);
    }

    #[test]
    fn add_project_rejects_empty_and_non_directory() {
        let base = temp_dir("add-invalid");
        let file = base.join("projects.json");
        assert!(add_project_at(&file, "   ").is_err(), "空路径应被拒");
        let not_dir = base.join("a.txt");
        std::fs::write(&not_dir, "x").unwrap();
        assert!(
            add_project_at(&file, &not_dir.to_string_lossy()).is_err(),
            "文件不是目录，应被拒"
        );
        assert!(!file.exists(), "被拒时不应留下 projects.json");
    }

    #[test]
    fn add_project_persists_and_dedupes() {
        let base = temp_dir("add-ok");
        let file = base.join("projects.json");
        let proj = base.join("my-app");
        std::fs::create_dir_all(&proj).unwrap();

        let p = add_project_at(&file, &proj.to_string_lossy()).expect("应添加成功");
        assert_eq!(p.name, "my-app");
        assert!(!p.id.is_empty(), "项目必须有 id");

        // 重新从文件读回：验证落盘 + 反序列化
        let loaded = load_projects_from(&file);
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].id, p.id);

        // 同一目录再添一次必须被拒
        assert!(add_project_at(&file, &proj.to_string_lossy()).is_err());
        // 多一个尾分隔符仍是同一路径，也应被拒
        let with_sep = format!("{}{}", proj.to_string_lossy(), std::path::MAIN_SEPARATOR);
        assert!(add_project_at(&file, &with_sep).is_err());
    }

    #[test]
    fn remove_project_only_touches_manifest() {
        let base = temp_dir("rm-project");
        let file = base.join("projects.json");
        let proj = base.join("app");
        std::fs::create_dir_all(&proj).unwrap();
        let p = add_project_at(&file, &proj.to_string_lossy()).unwrap();

        assert!(remove_project_at(&file, "no-such-id").is_err(), "删不存在的 id 应报错");
        remove_project_at(&file, &p.id).unwrap();
        assert!(load_projects_from(&file).projects.is_empty());
        assert!(proj.is_dir(), "移除项目条目不应删除项目目录");
    }

    // ─── 安装到项目 ───

    /// 建一套隔离环境：技能库 store + 项目 proj + 项目清单，返回 (base, store, 清单, 项目id)。
    fn setup_project_env(name: &str) -> (PathBuf, PathBuf, PathBuf, String) {
        let base = temp_dir(name);
        let store = base.join("store");
        std::fs::create_dir_all(&store).unwrap();
        let proj = base.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let file = base.join("projects.json");
        let p = add_project_at(&file, &proj.to_string_lossy()).unwrap();
        (base, store, file, p.id)
    }

    fn make_skill(store: &Path, id: &str) {
        let d = store.join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!("---\nname: {}\ndescription: demo {}\n---\n", id, id),
        )
        .unwrap();
    }

    #[test]
    fn install_rejects_unknown_skill_or_project() {
        let (_base, store, file, pid) = setup_project_env("unknown");
        make_skill(&store, "demo");
        assert!(install_into(&store, &file, "nope", &pid).is_err(), "技能库没有该技能");
        assert!(
            install_into(&store, &file, "demo", "no-such-project").is_err(),
            "项目不存在应报错"
        );
    }

    /// 安装 → 幂等 → 卸载 → 悬空识别，一条龙。
    ///
    /// 依赖真实 junction（Windows `mklink /J`），故限定 Windows。
    #[cfg(windows)]
    #[test]
    fn project_skill_lifecycle() {
        let (_base, store, file, pid) = setup_project_env("lifecycle");
        make_skill(&store, "demo");

        // 没装过时列表为空，且**不报错**（.agents/skills 还不存在）
        assert!(list_installed(std::slice::from_ref(&store), &file, &pid).unwrap().is_empty());

        let link = install_into(&store, &file, "demo", &pid).expect("应安装成功");
        assert!(
            Path::new(&link).join("SKILL.md").is_file(),
            "junction 应能穿透读到 SKILL.md"
        );

        let list = list_installed(std::slice::from_ref(&store), &file, &pid).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].state, "linked");
        assert_eq!(list[0].name, "demo", "名称应来自 SKILL.md 的 frontmatter");

        // 幂等：重复安装不报错、列表不重复
        install_into(&store, &file, "demo", &pid).expect("重复安装应幂等");
        assert_eq!(list_installed(std::slice::from_ref(&store), &file, &pid).unwrap().len(), 1);

        // 卸载：链接消失，**技能库内容必须还在**
        uninstall_from(&file, "demo", &pid).unwrap();
        assert!(!Path::new(&link).exists(), "链接应被删除");
        assert!(
            store.join("demo").join("SKILL.md").is_file(),
            "卸载绝不能删掉技能库里的实体内容"
        );
        assert!(list_installed(std::slice::from_ref(&store), &file, &pid).unwrap().is_empty());

        // 悬空：装好后把技能库里的技能删掉，UI 必须能识别出来
        install_into(&store, &file, "demo", &pid).unwrap();
        std::fs::remove_dir_all(store.join("demo")).unwrap();
        let list = list_installed(std::slice::from_ref(&store), &file, &pid).unwrap();
        assert_eq!(list.len(), 1, "悬空链接仍应被列出，否则用户看不到残留");
        assert_eq!(list[0].state, "broken");
    }

    /// 项目列表上显示的技能数必须和点进去看到的条目数**完全一致**。
///
/// 这是计数命令存在的全部理由，所以直接拿 `list_installed` 的结果做基准逐项比：
    /// 一旦两边的判定口径分家（比如这里多要求了 is_dir），用户看到的数字就会对不上，
    /// 而且这种错没有任何报错，只有「数字不对」。
    #[cfg(windows)]
    #[test]
    fn project_skill_counts_match_the_installed_list() {
        let (base, store, file, pid) = setup_project_env("counts");
        make_skill(&store, "a");
        make_skill(&store, "b");
        install_into(&store, &file, "a", &pid).unwrap();
        install_into(&store, &file, "b", &pid).unwrap();
        // 再加一个项目自带（真实目录）与一个悬空链接
        let skills_dir = base.join("proj").join(".agents").join("skills");
        std::fs::create_dir_all(skills_dir.join("project-own")).unwrap();
        std::fs::write(
            skills_dir.join("project-own").join("SKILL.md"),
            "---\nname: Project Own\ndescription: 手写的技能\n---\n",
        )
        .unwrap();
        // 装好再把库里的实体删掉 → 项目里留下一个悬空链接
        make_skill(&store, "gone");
        install_into(&store, &file, "gone", &pid).expect("悬空用例的技能应装得上");
        std::fs::remove_dir_all(store.join("gone")).unwrap();

        let listed = list_installed(std::slice::from_ref(&store), &file, &pid).unwrap();
        let counts = count_project_skills(&file, std::slice::from_ref(&store));
        let c = counts
            .iter()
            .find(|c| c.project_id == pid)
            .expect("项目应出现在计数里");

        let alive = listed.iter().filter(|e| e.state != "broken").count();
        let linked = listed.iter().filter(|e| e.state == "linked").count();
        let local = listed.iter().filter(|e| e.state == "local").count();
        let broken = listed.iter().filter(|e| e.state == "broken").count();
        assert_eq!((c.total, c.linked, c.local, c.broken), (alive, linked, local, broken));
        assert_eq!((c.total, c.linked, c.local, c.broken), (3, 2, 1, 1));
    }

    /// 多库下必须认出 junction 指向**哪一个**库。认错库的后果很隐蔽：
    /// 卸载会去删另一个库的同名技能，或把「已安装」误判成「悬空」。
    #[cfg(windows)]
    #[test]
    fn junction_resolves_to_the_right_store_among_many() {
        let (_base, store_a, file, pid) = setup_project_env("multi-store");
        let store_b = store_a.parent().unwrap().join("store-b");
        std::fs::create_dir_all(&store_b).unwrap();
        make_skill(&store_a, "in-a");
        make_skill(&store_b, "in-b");

        let stores = vec![store_a.clone(), store_b.clone()];
        // A 库的技能 → 归属 A；B 库的技能 → 归属 B
        let link_a = install_into(&store_a, &file, "in-a", &pid).expect("A 库技能应安装成功");
        let link_b = install_into(&store_b, &file, "in-b", &pid).expect("B 库技能应安装成功");
        let got_a = junction_target_skill(Path::new(&link_a), &stores).expect("应认出 A 库的链接");
        let got_b = junction_target_skill(Path::new(&link_b), &stores).expect("应认出 B 库的链接");
        assert_eq!(store_key(&got_a.0), store_key(&store_a));
        assert_eq!(got_a.1, "in-a");
        assert_eq!(store_key(&got_b.0), store_key(&store_b));
        assert_eq!(got_b.1, "in-b");

        // 两个库同时列出时，各自的技能都要被认成 linked（不是 broken）
        let list = list_installed(&stores, &file, &pid).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|e| e.state == "linked"), "两库都该是 linked");
    }

    /// 项目自带的真实目录也要被列出来（state=`local`），名字取自它自己的 SKILL.md。
    /// 不列出来的话它在 UI 上完全隐形，用户既看不到也清不掉。
    #[cfg(windows)]
    #[test]
    fn list_includes_local_real_directory_skills() {
        let (base, store, file, pid) = setup_project_env("local-listed");
        make_skill(&store, "from-library");
        let skills_dir = base.join("proj").join(".agents").join("skills");
        let real = skills_dir.join("project-own");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(
            real.join("SKILL.md"),
            "---\nname: Project Own\ndescription: 手写的技能\n---\n",
        )
        .unwrap();

        let list = list_installed(std::slice::from_ref(&store), &file, &pid).unwrap();
        assert_eq!(list.len(), 1, "项目自带技能应出现在列表里");
        assert_eq!(list[0].state, "local");
        assert_eq!(list[0].skill_id, "project-own");
        assert_eq!(list[0].name, "Project Own", "名称应读它自己的 SKILL.md");
        assert_eq!(list[0].description, "手写的技能");
    }

    /// 同名覆盖：项目里已有同名**真实目录**时允许覆盖 —— 删掉旧目录、建 junction。
    ///
    /// 不可逆，所以 UI 必须在应用前把「会被替换的真实目录」列给用户确认。
    #[cfg(windows)]
    #[test]
    fn install_overwrites_real_directory() {
        let (base, store, file, pid) = setup_project_env("overwrite");
        make_skill(&store, "demo");
        let skills_dir = base.join("proj").join(".agents").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let real = skills_dir.join("demo");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("MY_FILE.txt"), "old").unwrap();

        let link = install_into(&store, &file, "demo", &pid).expect("同名真实目录应被覆盖");
        assert!(Path::new(&link).join("SKILL.md").is_file(), "应变成指向技能库的 junction");
        assert!(
            !real.join("MY_FILE.txt").exists(),
            "旧目录内容随覆盖一并移除"
        );
    }

    /// 卸载同理：真实目录与链接同等对待，整个删除。
    #[cfg(windows)]
    #[test]
    fn uninstall_deletes_real_directory() {
        let (base, store, file, pid) = setup_project_env("uninstall-real");
        make_skill(&store, "demo");
        let skills_dir = base.join("proj").join(".agents").join("skills");
        std::fs::create_dir_all(skills_dir.join("demo")).unwrap();
        std::fs::write(skills_dir.join("demo").join("MY_FILE.txt"), "bye").unwrap();

        uninstall_from(&file, "demo", &pid).expect("真实目录应被删除");
        assert!(!skills_dir.join("demo").exists(), "目录应整个删掉");
        // 关键：技能库本体不能被牵连（只有链接才该这样，真实目录不会碰到技能库）
        assert!(store.join("demo").join("SKILL.md").is_file(), "技能库本体不受影响");
    }

    /// 技能库与项目路径含空格时也要能装。
    ///
    /// `create_junction` 走 `Command::args()` 而不是拼命令字符串，Rust 会自行正确引用，
    /// 所以这里不需要额外加引号 —— 这条测试把该结论锁住，防止将来"顺手补引号"反而弄坏。
    #[cfg(windows)]
    #[test]
    fn install_works_with_spaces_in_paths() {
        let base = temp_dir("with space");
        let store = base.join("my skills");
        std::fs::create_dir_all(&store).unwrap();
        make_skill(&store, "demo");
        let proj = base.join("my project");
        std::fs::create_dir_all(&proj).unwrap();
        let file = base.join("projects.json");
        let p = add_project_at(&file, &proj.to_string_lossy()).unwrap();

        let link = install_into(&store, &file, "demo", &p.id).expect("含空格路径应能安装");
        assert!(Path::new(&link).join("SKILL.md").is_file());
    }
}

// ─── 内置技能（随 Kira 分发） ───
//
// 技能文件放在 `src-tauri/resources/builtin-skills/<id>/`，编译期用 include_str! 嵌入：
// 既不必在 tauri.conf 里声明 bundle.resources（开发态与打包态是两套路径逻辑），
// 也不会因为用户换了数据目录就找不到。
//
// 名称与描述**不在这里另抄一份** —— 直接解析 SKILL.md 的 frontmatter，
// 保证技能自我介绍只有一处真源（改 SKILL.md 即改列表）。

const MINDMAP_IMPORT_SKILL_MD: &str =
    include_str!("../../../resources/builtin-skills/mindmap-import/SKILL.md");
const MINDMAP_IMPORT_SCHEMA_MD: &str =
    include_str!("../../../resources/builtin-skills/mindmap-import/references/schema.md");
const MINDMAP_IMPORT_VALIDATE_PY: &str =
    include_str!("../../../resources/builtin-skills/mindmap-import/scripts/validate.py");

// 数据库设计器技能：Agent 产出 .dbdesign.json（设计文档 §6「Agent 友好」第 2 条）
const DB_DESIGNER_SKILL_MD: &str =
    include_str!("../../../resources/builtin-skills/db-designer/SKILL.md");
const DB_DESIGNER_SCHEMA_MD: &str =
    include_str!("../../../resources/builtin-skills/db-designer/references/schema.md");
const DB_DESIGNER_VALIDATE_PY: &str =
    include_str!("../../../resources/builtin-skills/db-designer/scripts/validate.py");

struct BuiltinSkill {
    id: &'static str,
    /// SKILL.md 原文（用来取 frontmatter 里的 name / description）
    skill_md: &'static str,
    /// 文件清单：相对技能目录的路径 → 内容
    files: &'static [(&'static str, &'static str)],
}

const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill {
        id: "mindmap-import",
        skill_md: MINDMAP_IMPORT_SKILL_MD,
        files: &[
            ("SKILL.md", MINDMAP_IMPORT_SKILL_MD),
            ("references/schema.md", MINDMAP_IMPORT_SCHEMA_MD),
            ("scripts/validate.py", MINDMAP_IMPORT_VALIDATE_PY),
        ],
    },
    BuiltinSkill {
        id: "db-designer",
        skill_md: DB_DESIGNER_SKILL_MD,
        files: &[
            ("SKILL.md", DB_DESIGNER_SKILL_MD),
            ("references/schema.md", DB_DESIGNER_SCHEMA_MD),
            ("scripts/validate.py", DB_DESIGNER_VALIDATE_PY),
        ],
    },
];

#[derive(Serialize, Clone, Debug)]
pub struct BuiltinSkillView {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// 读取 SKILL.md 开头 YAML frontmatter 里的某个 key（只认 `key: value` 单行形态）。
fn frontmatter_value(md: &str, key: &str) -> Option<String> {
    let body = md.strip_prefix("---")?;
    let end = body.find("\n---")?;
    for line in body[..end].lines() {
        if let Some(rest) = line.trim().strip_prefix(key) {
            if let Some(v) = rest.trim_start().strip_prefix(':') {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// 列出内置技能（供前端「内置」页展示）
#[tauri::command]
pub fn list_builtin_skills() -> Vec<BuiltinSkillView> {
    BUILTIN_SKILLS
        .iter()
        .map(|s| BuiltinSkillView {
            id: s.id.to_string(),
            name: frontmatter_value(s.skill_md, "name").unwrap_or_else(|| s.id.to_string()),
            description: frontmatter_value(s.skill_md, "description").unwrap_or_default(),
        })
        .collect()
}

/// 把内置技能装进**技能库**：写出 `<技能库>/<id>/` 下的全部文件。
///
/// 非破坏性：目标已存在且非空就直接报错，绝不覆盖用户自己的技能。
/// 装完它只是技能库里的一个普通技能，仍需用户显式安装给某个项目才生效。
#[tauri::command]
pub fn install_builtin_skill(skill_id: String, store: Option<String>) -> Result<String, String> {
    // 装进技能库即可 —— 装完它就是一个普通技能，由用户显式安装给某个项目。
    // 早先这里要求用户先选一个目标目录（通常是 ~/.agents/skills，装完即全局生效），
    // 那正是上下文暴涨的来源，已随「项目」模型一起取消。
    // 装进哪个库 = 当前 UI 上的那个 tab（前端传 store；不传则默认库）。
    let target = resolve_store_arg(store.as_deref().unwrap_or(""), "");
    install_builtin_into(&skill_id, &target)
}

/// 目标目录可注入的版本（单测必须能指向临时目录，否则会往真实技能库里写）。
fn install_builtin_into(skill_id: &str, target: &Path) -> Result<String, String> {
    let skill = BUILTIN_SKILLS
        .iter()
        .find(|s| s.id == skill_id)
        .ok_or_else(|| format!("没有内置技能「{}」", skill_id))?;

    let dest = target.join(skill.id);
    if dest.exists() {
        let occupied = fs::read_dir(&dest)
            .map(|mut it| it.next().is_some())
            .unwrap_or(true);
        if occupied {
            return Err(format!(
                "技能库中已存在同名技能：{}（不覆盖，请先删掉它）",
                dest.display()
            ));
        }
    }

    for (rel, content) in skill.files {
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败 {}: {}", parent.display(), e))?;
        }
        fs::write(&path, content)
            .map_err(|e| format!("写入失败 {}: {}", path.display(), e))?;
    }

    Ok(dest.to_string_lossy().to_string())
}

#[cfg(test)]
mod builtin_skill_tests {
    use super::*;

    #[test]
    fn frontmatter_is_parsed_from_skill_md() {
        let s = &BUILTIN_SKILLS[0];
        assert_eq!(
            frontmatter_value(s.skill_md, "name").as_deref(),
            Some("mindmap-import")
        );
        let desc = frontmatter_value(s.skill_md, "description").unwrap_or_default();
        assert!(desc.contains("思维导图"), "description 应来自 frontmatter: {}", desc);
        assert_eq!(frontmatter_value(s.skill_md, "no_such_key"), None);
    }

    #[test]
    fn listed_builtin_skills_are_complete() {
        let list = list_builtin_skills();
        assert!(!list.is_empty());
        for v in list {
            assert!(!v.id.is_empty());
            assert!(!v.name.is_empty(), "{} 缺 name（frontmatter 解析失败？）", v.id);
            assert!(!v.description.is_empty(), "{} 缺 description", v.id);
        }
    }

    #[test]
    fn install_writes_every_file_and_refuses_overwrite() {
        let dir = std::env::temp_dir().join(format!("kira-builtin-skill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let installed = install_builtin_into("mindmap-import", &dir).expect("首次安装应成功");
        let root = PathBuf::from(&installed);
        assert!(root.join("SKILL.md").exists(), "SKILL.md 未落盘");
        assert!(root.join("references").join("schema.md").exists(), "schema.md 未落盘");
        assert!(root.join("scripts").join("validate.py").exists(), "validate.py 未落盘");

        // 已存在且非空 → 拒绝（不能覆盖用户的技能）
        assert!(install_builtin_into("mindmap-import", &dir).is_err());
        // 未知 id → 拒绝
        assert!(install_builtin_into("nope", &dir).is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
