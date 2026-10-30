//! Declarative ownership of persistent application data. No business implementation imports.

pub struct ModuleAssets {
    pub module: &'static str,
    pub files: &'static [&'static str],
    pub directories: &'static [&'static str],
    pub migration_roots: &'static [&'static str],
}

pub const MODULE_ASSETS: &[ModuleAssets] = &[
    ModuleAssets { module: "config", files: &["settings.json", "backups.json"], directories: &["backup", "fonts"], migration_roots: &["settings.json", "backups.json", "backup", "fonts"] },
    ModuleAssets { module: "ai", files: &["ai_config.json", "ai_sessions.json", "last_launch_configs.json", "collab.json", "skills.json", "mcp.json", "projects.json", "ai_usage.db"], directories: &["skills"], migration_roots: &["ai_config.json", "ai_sessions.json", "last_launch_configs.json", "collab.json", "skills.json", "mcp.json", "projects.json", "ai_usage.db", "skills", "mcp-servers", "mcp-repos"] },
    ModuleAssets { module: "launcher", files: &["launcher.db"], directories: &[], migration_roots: &["launcher.db", "file_index.db"] },
    ModuleAssets { module: "tasks", files: &["tasks.db"], directories: &["tasks"], migration_roots: &["tasks.db", "tasks"] },
    ModuleAssets { module: "mindmap", files: &["mindmap.db", "mindmap_settings.json"], directories: &[], migration_roots: &["mindmap.db", "mindmap_settings.json"] },
    ModuleAssets { module: "api", files: &["api.db"], directories: &[], migration_roots: &["api.db"] },
    ModuleAssets { module: "favorites", files: &["favorites.db", "favorites_settings.json"], directories: &[], migration_roots: &["favorites.db", "favorites_settings.json"] },
    ModuleAssets { module: "picky", files: &["picky/picky.db"], directories: &[], migration_roots: &["picky"] },
    ModuleAssets { module: "clipboard", files: &["clipboard/clipboard.db"], directories: &["clipboard/images"], migration_roots: &["clipboard"] },
    ModuleAssets { module: "otp", files: &["otp/otp.db"], directories: &[], migration_roots: &["otp"] },
    ModuleAssets { module: "buddy", files: &[], directories: &["buddy"], migration_roots: &["buddy"] },
    ModuleAssets { module: "suno", files: &["suno/profiles.json", "suno/downloaded.json"], directories: &[], migration_roots: &["suno"] },
    ModuleAssets { module: "db_designer", files: &["db_designer_settings.json"], directories: &["db_designer"], migration_roots: &["db_designer", "db_designer_settings.json"] },
    ModuleAssets { module: "music", files: &["music/settings.json", "music/library.json", "music/plugins/registry.json"], directories: &["music/plugins/scripts"], migration_roots: &["music"] },
    ModuleAssets { module: "mihomo", files: &["mihomo/app.json", "mihomo/controled.json", "mihomo/profile.json", "mihomo/override.json"], directories: &["mihomo/profiles", "mihomo/override"], migration_roots: &["mihomo"] },
    ModuleAssets { module: "cert", files: &[], directories: &["certs"], migration_roots: &["certs"] },
    ModuleAssets { module: "node_manager", files: &[], directories: &[], migration_roots: &["node-projects"] },
    ModuleAssets { module: "docker", files: &[], directories: &["docker"], migration_roots: &["docker"] },
    ModuleAssets { module: "state_sync", files: &["sync_state_config.json", "translate_history.json", "translate_config.json"], directories: &[], migration_roots: &["sync_state_config.json", "translate_history.json", "translate_config.json"] },
];

/// These adapters have no separately owned persistent payload. Their owners above
/// declare shared state (for example env backups belong to config).
const SHARED_OR_EPHEMERAL_MODULES: &[&str] = &[
    "env", "cache", "sdk_resolver", "hosts", "port", "network", "pkg", "mirror", "service", "project",
    "hidden_cmd", "utils", "conflict", "http_server", "img_base64", "ai_registry", "tool_version", "cert_receiver",
    "rtsp_server", "file_io", "secrets", "s3_common", "autostart_admin", "serialtool", "wstool",
    "storage_assets", "storage_maintenance", "storage_migration", "foundation", "module_lifecycle",
];

pub fn snapshot_files() -> impl Iterator<Item = &'static str> {
    MODULE_ASSETS.iter().flat_map(|m| m.files.iter().copied())
}

pub fn snapshot_directories() -> impl Iterator<Item = &'static str> {
    MODULE_ASSETS.iter().flat_map(|m| m.directories.iter().copied())
}

pub fn migration_roots() -> std::collections::BTreeSet<&'static str> {
    MODULE_ASSETS.iter().flat_map(|m| m.migration_roots.iter().copied()).collect()
}

pub fn allows_snapshot_path(rel: &str) -> bool {
    rel == "config.json" || snapshot_files().any(|p| p == rel)
        || snapshot_directories().any(|p| rel.starts_with(&format!("{p}/")))
        // Old snapshots contain WAL sidecars; validation consolidates these in staging.
        || rel.strip_suffix("-wal").is_some_and(|main| snapshot_files().any(|p| p == main))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_commands_module_has_an_explicit_storage_policy() {
        for line in include_str!("mod.rs").lines() {
            if let Some(module) = line.trim().strip_prefix("pub mod ").and_then(|s| s.strip_suffix(';')) {
                assert!(MODULE_ASSETS.iter().any(|m| m.module == module) || SHARED_OR_EPHEMERAL_MODULES.contains(&module), "missing storage ownership policy for {module}");
            }
        }
    }

    #[test]
    fn backup_covers_new_module_data_but_excludes_rebuildable_payloads() {
        for rel in ["buddy/workbuddy.json", "suno/profiles.json", "db_designer/sessions.json", "db_designer_settings.json", "tasks/images/a.png"] {
            assert!(allows_snapshot_path(rel), "{rel}");
        }
        for rel in ["sdk/node/node.exe", "music/plugins/node_modules/package/index.js", "music/plugins/bridge.js", "file_index.db"] {
            assert!(!allows_snapshot_path(rel), "{rel}");
        }
    }
}
