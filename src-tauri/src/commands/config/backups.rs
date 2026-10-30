use super::*;

// ─── Backup storage (separate file) ───

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BackupStore {
    /// project_id -> env var name -> original value
    pub env_vars: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// project_id -> list of PATH entries that were removed
    pub path_entries: std::collections::HashMap<String, Vec<String>>,
}

pub(super) fn backup_path() -> PathBuf {
    get_data_dir().join("backups.json")
}

pub fn load_backups() -> BackupStore {
    let path = backup_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(store) = serde_json::from_str::<BackupStore>(&data) {
                return store;
            }
        }
    }
    BackupStore::default()
}

pub fn save_backups(store: &BackupStore) -> Result<(), String> {
    let path = backup_path();
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())?;
    Ok(())
}

/// 兼容反序列化 original_envs：同时接受旧版扁平结构 `{变量名: 值}` 与新版按项目隔离的
/// `{project_id: {变量名: 值}}`。旧版扁平数据归入 "__legacy__" 项目，避免多项目同名的原始值丢失。
pub(super) fn deserialize_original_envs<'de, D>(
    deserializer: D,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw = serde_json::Value::deserialize(deserializer)?;
    let mut out: std::collections::HashMap<String, std::collections::HashMap<String, String>> =
        std::collections::HashMap::new();
    if let serde_json::Value::Object(map) = raw {
        for (k, v) in map {
            match v {
                serde_json::Value::String(s) => {
                    // 旧版扁平格式：{变量名: 值}
                    out.entry("__legacy__".to_string())
                        .or_default()
                        .insert(k, s);
                }
                serde_json::Value::Object(nested) => {
                    // 新版：{project_id: {变量名: 值}}
                    let inner = nested
                        .into_iter()
                        .map(|(nk, nv)| (nk, nv.as_str().unwrap_or_default().to_string()))
                        .collect();
                    out.insert(k, inner);
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Migrate old backups from config.json into backups.json (one-time)
pub fn migrate_backups_from_config(config: &mut Config) {
    if !config.original_envs.is_empty() || !config.original_paths.is_empty() {
        let mut store = load_backups();
        for (project_id, vars) in config.original_envs.drain() {
            for (k, v) in vars {
                store
                    .env_vars
                    .entry(project_id.clone())
                    .or_default()
                    .insert(k, v);
            }
        }
        for (k, v) in config.original_paths.drain() {
            store.path_entries.entry(k).or_default().extend(v);
        }
        let _ = save_backups(&store);
        let _ = save_config(config);
    }
}
