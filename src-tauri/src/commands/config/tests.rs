
use super::*;

#[test]
fn stale_config_patch_preserves_other_fields_and_map_keys() {
    let baseline = serde_json::json!({"language":"zh", "global_font":"", "active_versions":{"node":"20", "go":"1"}});
    let desired = serde_json::json!({"language":"en", "global_font":"", "active_versions":{"node":"22", "go":"1"}});
    let mut latest = serde_json::json!({"language":"zh", "global_font":"Consolas", "active_versions":{"node":"20", "go":"2", "java":"21"}});
    merge_config_patch(&mut latest, &baseline, &desired, "");
    assert_eq!(
        latest,
        serde_json::json!({"language":"en", "global_font":"Consolas", "active_versions":{"node":"22", "go":"2", "java":"21"}})
    );
}

#[test]
fn stale_config_patch_merges_set_members_and_deletes_only_changed_map_keys() {
    let baseline = serde_json::json!({"managed_items":["node"],"active_versions":{"node":"20"}});
    let desired = serde_json::json!({"managed_items":["node","go"],"active_versions":{}});
    let mut latest = serde_json::json!({"managed_items":["node","java"],"active_versions":{"node":"20","java":"21"}});
    merge_config_patch(&mut latest, &baseline, &desired, "");
    let members = latest["managed_items"].as_array().unwrap();
    assert!(
        members.contains(&serde_json::json!("node"))
            && members.contains(&serde_json::json!("go"))
            && members.contains(&serde_json::json!("java"))
    );
    assert_eq!(latest["active_versions"], serde_json::json!({"java":"21"}));
}

/// 落盘的 config.json 只能有数据路径；业务选项必须落进 settings.json。
#[test]
fn config_json_carries_only_paths() {
    let mut config = default_config();
    config.data_dir = "D:/any-versions".to_string();
    config.node_projects_dir = "D:/any-versions/node-projects".to_string();
    config.managed_items.insert("nodejs".to_string());
    config.rss_sources = vec!["https://example.com/feed".to_string()];
    config.tray_menu.enabled = false;

    let (paths, settings) = split_config(&config);
    let json = serde_json::to_value(&paths).unwrap();
    let keys: Vec<String> = json.as_object().unwrap().keys().cloned().collect();
    // 显式设了 node_projects_dir 才会落盘
    assert_eq!(keys.len(), 2, "config.json 字段: {:?}", keys);
    assert!(keys.iter().any(|k| k == "data_dir"));
    assert!(keys.iter().any(|k| k == "node_projects_dir"));
    assert!(
        !keys.iter().any(|k| k.starts_with("rss_")
            || k.starts_with("managed")
            || k.starts_with("module_")
            || k == "tray_menu"),
        "业务选项不该出现在 config.json: {:?}",
        keys
    );

    // 合并回来后一个字段都不能丢
    let merged = merge_config(paths, settings);
    assert_eq!(merged.data_dir, "D:/any-versions");
    assert_eq!(merged.node_projects_dir, "D:/any-versions/node-projects");
    assert!(merged.managed_items.contains("nodejs"));
    assert_eq!(
        merged.rss_sources,
        vec!["https://example.com/feed".to_string()]
    );
    assert!(!merged.tray_menu.enabled);
}

/// 旧版 config.json（选项和路径混在一起）要能被拆开读：路径归路径，选项归选项。
#[test]
fn legacy_combined_config_splits_on_read() {
    let legacy = r#"{
            "data_dir": "D:/any-versions",
            "managed_items": ["go", "nodejs"],
            "rss_sources": ["https://a/feed"],
            "tray_menu": { "enabled": false, "show_mihomo": true }
        }"#;
    let paths: ConfigPathsFile = serde_json::from_str(legacy).unwrap();
    let settings: SettingsFile = serde_json::from_str(legacy).unwrap();
    assert_eq!(paths.data_dir, "D:/any-versions");
    assert!(settings.managed_items.contains("go"));
    assert_eq!(settings.rss_sources, vec!["https://a/feed".to_string()]);
    assert!(!settings.tray_menu.enabled);
}

/// node_projects_dir 默认派生自 data_dir：没显式改过就不该出现在 config.json 里。
#[test]
fn derived_paths_are_not_persisted() {
    let mut config = default_config();
    assert_eq!(config.node_projects_dir, "", "默认应为派生（空）");
    config.data_dir = "D:/any-versions".to_string();
    let (paths, _) = split_config(&config);
    let json = serde_json::to_value(&paths).unwrap();
    let keys: Vec<String> = json.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        vec!["data_dir".to_string()],
        "config.json 字段: {:?}",
        keys
    );

    // 等于派生值的显式路径也要被清掉（fill_legacy_dirs 里做）
    config.node_projects_dir = "D:/any-versions/node-projects".to_string();
    fill_legacy_dirs(&mut config, &get_base_dir());
    assert_eq!(config.node_projects_dir, "", "等于默认值应被归一化为空");

    // 用户自定义到别处的路径必须保留
    config.node_projects_dir = "E:/node-projects".to_string();
    fill_legacy_dirs(&mut config, &get_base_dir());
    assert_eq!(config.node_projects_dir, "E:/node-projects");
}

/// 动效偏好只有 `always` 一个「开」值，其余一律按「跟随系统」，
/// 避免后端存进奇怪的值后前端判断不出来。
#[test]
fn motion_preference_only_always_enables() {
    assert_eq!(normalize_motion_preference("always"), "always");
    assert_eq!(normalize_motion_preference("Always"), "always");
    assert_eq!(normalize_motion_preference("  always  "), "always");
    assert_eq!(normalize_motion_preference(""), "");
    assert_eq!(normalize_motion_preference("system"), "");
    assert_eq!(normalize_motion_preference("always-on"), "");
}

/// 密度只有 `compact` 一个「紧凑」值，其余一律按「舒适」，
/// 避免后端存进奇怪的值后前端 `data-density` 判断不出来。
#[test]
fn density_only_compact_tightens() {
    assert_eq!(normalize_density("compact"), "compact");
    assert_eq!(normalize_density("Compact"), "compact");
    assert_eq!(normalize_density("  compact  "), "compact");
    assert_eq!(normalize_density(""), "");
    assert_eq!(normalize_density("comfortable"), "");
    assert_eq!(normalize_density("ultra-compact"), "");
}

/// 缺省字段的旧 settings.json 也要能读（全 default，不能因为缺字段整份解析失败）。
#[test]
fn settings_file_tolerates_missing_fields() {
    let settings: SettingsFile = serde_json::from_str("{}").unwrap();
    assert!(settings.managed_items.is_empty());
    assert!(settings.rss_sources.is_empty());
    assert!(settings.tray_menu.enabled);
    assert_eq!(settings.language, "");
    assert_eq!(settings.density, "");
}
