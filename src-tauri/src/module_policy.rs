//! IPC admission at the composition root. Shared settings and AI transport configuration
//! remain usable by enabled modules; disabled business pages cannot start their commands.
use std::collections::HashMap;
use std::sync::OnceLock;

pub const MODULE_IDS: &[&str] = &[
    "launcher",
    "news",
    "sdk",
    "ai",
    "mindmap",
    "api",
    "node",
    "mihomo",
    "cert",
    "clipboard",
    "otp",
    "translate",
    "picky",
    "buddy",
    "settings",
    "network",
    "httpServer",
    "rtspServer",
    "imageBase64",
    "env",
    "logViewer",
    "jsonBrowser",
    "markdownReader",
    "serial",
    "wsDebugger",
    "music",
    "favorites",
    "suno",
    "tasks",
    "dbdesigner",
];

fn owner(path: &str) -> Option<&'static str> {
    if path.starts_with("ai::translate::") {
        return Some("translate");
    }
    // This is shared model/provider configuration, used by translation, mindmap and API tools.
    if ["ai::config::", "ai::cache::", "ai::usage::"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        || path.ends_with("::save_global_default_model")
    {
        return None;
    }
    match path.split("::").next()? {
        "ai" | "ai_registry" => Some("ai"),
        "project" => Some("sdk"),
        "node_manager" => Some("node"),
        "cert" | "cert_receiver" => Some("cert"),
        "mindmap" => Some("mindmap"),
        "db_designer" => Some("dbdesigner"),
        "music" => Some("music"),
        "buddy" => Some("buddy"),
        "clipboard" => Some("clipboard"),
        "api" => Some("api"),
        "mihomo" => Some("mihomo"),
        "picky" => Some("picky"),
        "otp" => Some("otp"),
        "favorites" => Some("favorites"),
        "suno" => Some("suno"),
        "tasks" => Some("tasks"),
        "launcher" => Some("launcher"),
        "http_server" => Some("httpServer"),
        "rtsp_server" => Some("rtspServer"),
        "serialtool" => Some("serial"),
        "wstool" => Some("wsDebugger"),
        "img_base64" => Some("imageBase64"),
        "env" | "hosts" => Some("env"),
        "network" => Some("network"),
        // Files, settings, storage administration and SDK utilities are shared capabilities.
        _ => None,
    }
}

fn owners() -> &'static HashMap<&'static str, &'static str> {
    static OWNERS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    OWNERS.get_or_init(|| {
        include_str!("app_commands.rs")
            .lines()
            .filter_map(|line| {
                let path = line
                    .trim()
                    .strip_prefix("commands::")?
                    .trim_end_matches(',');
                Some((path.rsplit("::").next()?, owner(path)?))
            })
            .collect()
    })
}

pub fn command_owner(command: &str) -> Option<&'static str> {
    owners().get(command).copied()
}
pub fn check_command(command: &str) -> Result<(), String> {
    check_with(command, crate::runtime_tasks::is_enabled)
}
fn check_with(command: &str, enabled: impl Fn(&str) -> bool) -> Result<(), String> {
    match command_owner(command) {
        Some(module) if !enabled(module) => Err("该模块已禁用，请先在设置中启用".into()),
        None => Ok(()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_follows_registered_paths_not_command_name_guesses() {
        for (command, module) in [
            ("cert_issue_now", "cert"),
            ("translate_text", "translate"),
            ("mm_ai_from_text", "mindmap"),
            ("dbd_ai_generate", "dbdesigner"),
            ("buddy_oauth_start", "buddy"),
            ("music_play", "music"),
            ("ws_connect", "wsDebugger"),
        ] {
            assert_eq!(command_owner(command), Some(module));
        }
        for command in [
            "get_ai_config",
            "get_appearance_config",
            "set_module_layout",
            "get_config",
        ] {
            assert_eq!(command_owner(command), None);
        }
    }
    #[test]
    fn backend_catalog_matches_navigation_catalog() {
        let ids: std::collections::HashSet<_> = include_str!("../../src/moduleCatalog.ts")
            .lines()
            .filter_map(|line| line.trim().strip_prefix("id: \"")?.split('"').next())
            .collect();
        assert_eq!(ids, MODULE_IDS.iter().copied().collect());
    }
    #[test]
    fn disabled_commands_are_denied_but_shared_settings_stay_available() {
        assert!(check_with("cert_issue_now", |_| false).is_err());
        assert!(check_with("cert_issue_now", |module| module == "cert").is_ok());
        assert!(check_with("get_ai_config", |_| false).is_ok());
        assert!(check_with("get_appearance_config", |_| false).is_ok());
    }
}
