use super::*;

// ─── 注册表 ───

#[derive(Default)]
pub struct NodeProjectRegistry {
    pub(super) projects: Vec<NodeProjectDef>,
}

impl NodeProjectRegistry {
    pub(super) fn load() -> Self {
        let mut projects = Vec::new();
        if let Some(dir) = find_node_projects_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            if let Ok(def) = serde_json::from_str::<NodeProjectDef>(&content) {
                                projects.push(def);
                            } else {
                                eprintln!("[node_manager] 解析 {} 失败", path.display());
                            }
                        }
                    }
                }
            }
        }
        projects.sort_by(|a, b| a.id.cmp(&b.id));
        Self { projects }
    }

    pub fn all(&self) -> &[NodeProjectDef] {
        &self.projects
    }

    pub fn find(&self, id: &str) -> Option<&NodeProjectDef> {
        self.projects.iter().find(|p| p.id == id)
    }
}

pub(super) static REGISTRY: Mutex<Option<Vec<NodeProjectDef>>> = Mutex::new(None);

/// 获取全局项目定义列表（首次加载后缓存）。
/// 注意：空列表不做缓存，以便后续配置目录就绪后能重新加载。
pub fn registry() -> Vec<NodeProjectDef> {
    {
        let g = REGISTRY.lock().unwrap();
        if let Some(list) = g.as_ref() {
            return list.clone();
        }
    }
    let loaded = NodeProjectRegistry::load().all().to_vec();
    if !loaded.is_empty() {
        let mut g = REGISTRY.lock().unwrap();
        *g = Some(loaded.clone());
    }
    loaded
}

/// 按 id 查找项目定义。
pub fn find_project(id: &str) -> Option<NodeProjectDef> {
    registry().into_iter().find(|p| p.id == id)
}

/// 定位 node-projects 配置目录。
/// 搜索策略与 ai-tools / projects 注册表保持一致：
/// 依次在「资源目录 / exe 同目录及向上 5 层 / 当前工作目录 / 用户配置目录」中
/// 查找 `node-projects` 或 `_up_/node-projects`（Tauri 打包时资源被拷贝进 `_up_` 前缀）。
pub(crate) fn find_node_projects_dir() -> Option<PathBuf> {
    let mut search_dirs: Vec<PathBuf> = Vec::new();

    // 开发构建：**仓库源目录排第一**（原因见 `utils::dev_repo_root`）
    if let Some(repo) = crate::commands::utils::dev_repo_root() {
        search_dirs.push(repo);
    }

    // 优先在 Tauri 2 打包后的官方资源目录下查找
    if let Some(res_dir) = crate::commands::utils::get_resource_dir() {
        search_dirs.push(res_dir);
    }

    // exe 同目录及向上 5 层
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            search_dirs.push(exe_dir.to_path_buf());
            let mut dir = exe_dir.to_path_buf();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                    search_dirs.push(dir.clone());
                }
            }
        }
    }

    // 当前工作目录
    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd);
    }

    // 用户配置目录（~/.any-version）
    search_dirs.push(get_base_dir());

    // 每个候选目录下查找 node-projects 目录（含 Tauri 打包时的 `_up_` 前缀布局）
    for dir in &search_dirs {
        for candidate in [
            dir.join("_up_").join("node-projects"),
            dir.join("node-projects"),
        ] {
            if candidate.exists() && candidate.is_dir() {
                eprintln!("[node_manager] 命中 node-projects: {}", candidate.display());
                return Some(candidate);
            }
        }
    }
    None
}
