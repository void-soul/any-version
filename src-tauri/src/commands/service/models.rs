use super::*;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceInfo {
    pub name: String,
    pub status: String, // "running" | "stopped" | "not_installed" | "port_conflict"
    pub active_version: String,
    pub port: String,
    pub pid: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedDataDir {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) path: String,
    pub(crate) kind: Option<String>,
    pub(crate) source: String,
    pub(crate) auto_create: bool,
    pub(crate) required_for_start: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceRuntime {
    pub(crate) install_root: Option<PathBuf>,
    pub(crate) config_file: Option<PathBuf>,
    pub(crate) port: Option<u16>,
    pub(crate) data_dirs: Vec<ResolvedDataDir>,
    pub(crate) data_dir: String,
    pub(crate) log_dir: String,
}

#[derive(Clone, Debug)]
pub(super) struct ProcessInfo {
    pub(super) pid: u32,
    pub(super) name: String,
    pub(super) exe_path: Option<String>,
}
