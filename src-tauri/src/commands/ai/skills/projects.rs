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
    read_projects_checked(path).unwrap_or_default()
}

fn read_projects_checked(path: &Path) -> Result<ProjectsFile, String> {
    let data = match fs::read_to_string(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProjectsFile::default())
        }
        Err(error) => return Err(format!("读取项目列表失败: {error}")),
    };
    serde_json::from_str(&data).map_err(|error| format!("项目列表格式无效: {error}"))
}

pub(super) fn save_projects_to(path: &Path, f: &ProjectsFile) -> Result<(), String> {
    let data = serde_json::to_string_pretty(f).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(path, data.as_bytes())
}

/// 列出全部项目（安装作用域）。
#[tauri::command]
pub fn list_projects() -> Result<Vec<SkillProject>, String> {
    Ok(read_projects_checked(&projects_path())?.projects)
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

/// Open only an existing directory belonging to a registered AI project.
#[tauri::command]
pub fn open_skill_project_directory(
    app: tauri::AppHandle,
    project_id: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let directory = registered_directory_at(&projects_path(), &project_id)?;
    app.opener()
        .open_path(directory.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| format!("打开项目目录失败: {e}"))
}

fn registered_directory_at(file: &Path, id: &str) -> Result<PathBuf, String> {
    let project = read_projects_checked(file)?
        .projects
        .into_iter()
        .find(|project| project.id == id)
        .ok_or("项目不存在，请刷新项目列表")?;
    let path = PathBuf::from(project.path);
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!(
            "项目目录不存在或不是有效的绝对目录: {}",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod directory_tests {
    use super::*;
    #[test]
    fn refresh_reports_corrupt_or_unreadable_lists_without_replacing_them() {
        let root = std::env::temp_dir().join(gen_project_id());
        fs::create_dir(&root).unwrap();
        let file = root.join("projects.json");
        assert!(read_projects_checked(&file).unwrap().projects.is_empty());
        fs::write(&file, "broken json").unwrap();
        assert!(read_projects_checked(&file).is_err());
        assert!(registered_directory_at(&file, "anything").is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "broken json");
        assert!(read_projects_checked(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opens_only_registered_existing_directories() {
        let root = std::env::temp_dir().join(gen_project_id());
        fs::create_dir(&root).unwrap();
        let config = root.join("projects.json");
        let folder = root.join("project with spaces");
        fs::create_dir(&folder).unwrap();
        let project = add_project_at(&config, &folder.to_string_lossy()).unwrap();
        assert_eq!(
            registered_directory_at(&config, &project.id).unwrap(),
            folder
        );
        assert!(registered_directory_at(&config, &folder.to_string_lossy()).is_err());
        fs::remove_dir(&folder).unwrap();
        assert!(registered_directory_at(&config, &project.id).is_err());
        fs::write(&folder, "not a directory").unwrap();
        assert!(registered_directory_at(&config, &project.id).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
