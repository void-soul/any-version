//! projects responsibility.
use super::*;

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

pub(super) fn projects_path() -> PathBuf {
    get_data_dir().join("projects.json")
}

pub(super) fn load_projects() -> ProjectsFile {
    load_projects_from(&projects_path())
}

/// 读写分离出「路径可注入」的版本：单测必须能指向临时目录，
/// 否则测试会往用户真实的数据目录里写 projects.json。
pub(super) fn load_projects_from(path: &Path) -> ProjectsFile {
    if let Ok(data) = fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<ProjectsFile>(&data) {
            return v;
        }
    }
    ProjectsFile::default()
}

pub(super) fn save_projects_to(path: &Path, f: &ProjectsFile) -> Result<(), String> {
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

pub(super) fn add_project_at(file: &Path, raw: &str) -> Result<SkillProject, String> {
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

pub(super) fn remove_project_at(file: &Path, id: &str) -> Result<(), String> {
    let mut f = load_projects_from(file);
    let before = f.projects.len();
    f.projects.retain(|p| p.id != id);
    if f.projects.len() == before {
        return Err("项目不存在".to_string());
    }
    save_projects_to(file, &f)
}

/// 项目 id：时间戳 + 进程号（同进程内纳秒级不重复，与 temp_clone_dir 同一风格）。
pub(super) fn gen_project_id() -> String {
    let stamp = chrono::Local::now().timestamp_nanos_opt().unwrap_or(0);
    format!("p{}_{}", std::process::id(), stamp)
}

/// 按 id 取项目目录。
///
/// 「项目」这个概念被技能与 MCP 共用（两者都是「装给某个项目」），所以解析放在这里，
/// 免得 MCP 模块复制一份 projects.json 的读法——那种复制迟早会漂。
pub(crate) fn project_dir_of(project_id: &str) -> Result<String, String> {
    load_projects()
        .projects
        .into_iter()
        .find(|p| p.id == project_id)
        .map(|p| p.path)
        .ok_or_else(|| format!("项目不存在: {}", project_id))
}
