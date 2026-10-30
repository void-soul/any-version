use super::*;

/// 处理拖入的文件或目录列表，解析并生成 Item 列表
pub fn process_dropped_paths(
    paths: Vec<String>,
    classification_id: i64,
) -> Result<Vec<Item>, String> {
    use crate::commands::launcher::db;
    use crate::commands::launcher::models::{Item, ItemData};

    let mut created_items = Vec::new();

    for path_str in paths {
        let p = Path::new(&path_str);
        if !p.exists() {
            continue;
        }

        let is_dir = p.is_dir();
        let file_name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&path_str)
            .to_string();
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&file_name)
            .to_string();
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let item = if ext == "lnk" {
            // 快捷方式
            if let Some(info) = resolve_shortcut(&path_str) {
                // 图标优先级：快捷方式显式指定的自定义图标(IconLocation，如 WorkDaddy.ico) > 目标文件图标
                let icon = info
                    .icon_path
                    .as_deref()
                    .and_then(extract_shortcut_custom_icon)
                    .or(info.icon_base64);
                Item {
                    id: 0,
                    classification_id,
                    name: if !info.name.is_empty() {
                        info.name
                    } else {
                        stem
                    },
                    item_type: if info.is_dir { 1 } else { 0 },
                    data: ItemData {
                        target: Some(info.target_path),
                        params: if !info.arguments.is_empty() {
                            Some(info.arguments)
                        } else {
                            None
                        },
                        start_location: if !info.working_dir.is_empty() {
                            Some(info.working_dir)
                        } else {
                            None
                        },
                        run_as_admin: false,
                        icon,
                        ..Default::default()
                    },
                    shortcut_key: None,
                    global_shortcut_key: false,
                    order: 0,
                }
            } else {
                let icon = extract_file_icon(&path_str);
                Item {
                    id: 0,
                    classification_id,
                    name: stem,
                    item_type: 0,
                    data: ItemData {
                        target: Some(path_str.clone()),
                        run_as_admin: false,
                        icon,
                        ..Default::default()
                    },
                    shortcut_key: None,
                    global_shortcut_key: false,
                    order: 0,
                }
            }
        } else if ext == "url" {
            // 网页快捷方式 .url 文件
            let content = std::fs::read_to_string(p).unwrap_or_default();
            let mut url_target = String::new();
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.to_lowercase().starts_with("url=") {
                    url_target = trimmed[4..].trim().to_string();
                    break;
                }
            }
            if url_target.is_empty() {
                url_target = path_str.clone();
            }
            Item {
                id: 0,
                classification_id,
                name: stem,
                item_type: 2,
                data: ItemData {
                    target: Some(url_target),
                    run_as_admin: false,
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
            }
        } else if is_dir {
            // 目录 / 文件夹
            let icon = extract_file_icon(&path_str);
            Item {
                id: 0,
                classification_id,
                name: file_name,
                item_type: 1, // 文件夹
                data: ItemData {
                    target: Some(path_str.clone()),
                    start_location: Some(path_str.clone()),
                    run_as_admin: false,
                    icon,
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
            }
        } else {
            // 任意文件 / .exe / .bat / .cmd / .ps1 / .pdf / .docx / .png 等
            let icon = extract_file_icon(&path_str);
            let display_name = if ext == "exe" || ext == "bat" || ext == "cmd" || ext == "ps1" {
                stem
            } else {
                file_name
            };
            let parent_dir = p
                .parent()
                .map(|parent| parent.to_string_lossy().to_string());
            Item {
                id: 0,
                classification_id,
                name: display_name,
                item_type: 0,
                data: ItemData {
                    target: Some(path_str.clone()),
                    start_location: parent_dir,
                    run_as_admin: false,
                    icon,
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
            }
        };

        let item_id = db::save_item(&item)?;
        let mut saved_item = item;
        saved_item.id = item_id;
        created_items.push(saved_item);
    }

    Ok(created_items)
}
