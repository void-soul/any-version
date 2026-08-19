use std::collections::HashMap;
use std::path::{Path, PathBuf};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::db;
use super::models::{Classification, ClassificationData, Item, ItemData};
use super::sqleet::{decrypt_sqleet_db, is_sqleet_db, DAWNLAUNCHER_DB_KEY};

/// 打开 DawnLauncher 数据库连接。
/// 普通 SQLite 直接只读打开；sqleet(chacha20) 加密的数据库先解密到临时文件再打开。
/// 返回 (连接, 需要清理的临时文件路径)。
fn open_dawn_db(path: &Path) -> Result<(Connection, Option<PathBuf>), String> {
    if !path.exists() {
        return Err(format!("文件不存在: {}", path.to_string_lossy()));
    }

    let head = std::fs::read(path)
        .map_err(|e| format!("读取数据库文件失败: {}", e))?
        .chunks(16)
        .next()
        .unwrap_or(&[])
        .to_vec();

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;

    // 标准 SQLite 头 → 直接打开
    if head.starts_with(b"SQLite format 3\0") {
        let conn = Connection::open_with_flags(path, flags)
            .map_err(|e| format!("打开数据库失败 (请确认文件格式有效): {}", e))?;
        return Ok((conn, None));
    }

    // 非标准头 → 按 Dawn Launcher sqleet(chacha20) 加密格式解密
    if is_sqleet_db(&head) {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("读取数据库文件失败: {}", e))?;
        let plain = decrypt_sqleet_db(&bytes, DAWNLAUNCHER_DB_KEY)
            .map_err(|e| format!("解密 Dawn Launcher 数据库失败: {}", e))?;

        // 写入系统临时目录
        let tmp_path = std::env::temp_dir().join(format!(
            "anyversion_dawn_import_{}.db",
            std::process::id()
        ));
        std::fs::write(&tmp_path, &plain)
            .map_err(|e| format!("写入解密临时文件失败: {}", e))?;

        let conn = match Connection::open_with_flags(&tmp_path, flags) {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(format!("打开解密后的数据库失败: {}", e));
            }
        };
        return Ok((conn, Some(tmp_path)));
    }

    Err("无法识别的数据库格式（既不是标准 SQLite，也不是 Dawn Launcher 加密数据库）".to_string())
}

/// 导入 DawnLauncher SQLite .db 文件或 AnyVersion 备份文件
pub fn import_dawn_or_any_db(db_path: &str) -> Result<usize, String> {
    let path = Path::new(db_path);
    if !path.exists() {
        return Err(format!("文件不存在: {}", db_path));
    }

    let (src_conn, tmp_file) = open_dawn_db(path)?;

    // 用闭包包裹主逻辑，确保无论成功失败都清理临时文件
    let result = run_import(&src_conn);

    // 关闭连接并清理临时文件
    drop(src_conn);
    if let Some(tmp) = tmp_file {
        let _ = std::fs::remove_file(&tmp);
    }

    result
}

fn run_import(src_conn: &Connection) -> Result<usize, String> {
    // 检查表结构 (忽略大小写)
    let is_dawn_format: bool = src_conn
        .query_row(
            "SELECT count(1) FROM sqlite_master WHERE type='table' AND LOWER(name)='classification'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    let is_anyversion_format: bool = src_conn
        .query_row(
            "SELECT count(1) FROM sqlite_master WHERE type='table' AND LOWER(name)='launcher_classification'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !is_dawn_format && !is_anyversion_format {
        return Err("不支持的数据库格式: 未在文件中检测到 classification 或 launcher_classification 数据表".to_string());
    }

    let mut imported_items_count = 0usize;

    if is_dawn_format {
        // 读取 DawnLauncher 分类
        let mut old_id_to_new_id: HashMap<i64, i64> = HashMap::new();
        let mut raw_classifications = Vec::new();

        if let Ok(mut stmt) = src_conn.prepare("SELECT id, parent_id, name, type, data, shortcut_key, global_shortcut_key, `order` FROM classification ORDER BY `order` ASC") {
            let cls_iter = stmt.query_map([], |row| {
                let id: i64 = row.get::<_, Option<i64>>(0).unwrap_or(None).unwrap_or(0);
                let parent_id: Option<i64> = row.get::<_, Option<i64>>(1).unwrap_or(None);
                let name: String = row.get::<_, Option<String>>(2).unwrap_or(None).unwrap_or_else(|| "分类".to_string());
                let cls_type: i32 = row.get::<_, Option<i32>>(3).unwrap_or(None).unwrap_or(0);
                let data_str: String = row.get::<_, Option<String>>(4).unwrap_or(None).unwrap_or_else(|| "{}".to_string());
                let shortcut_key: Option<String> = row.get::<_, Option<String>>(5).unwrap_or(None);
                let global_shortcut_key: bool = row.get::<_, Option<i64>>(6).unwrap_or(None).map(|v| v != 0).unwrap_or(false);
                let order: i32 = row.get::<_, Option<i32>>(7).unwrap_or(None).unwrap_or(0);

                Ok((id, parent_id, name, cls_type, data_str, shortcut_key, global_shortcut_key, order))
            });

            if let Ok(rows) = cls_iter {
                for r in rows {
                    if let Ok(c) = r {
                        raw_classifications.push(c);
                    }
                }
            }
        }

        // 先导入顶级分类 (parent_id 为 None 或 0)
        for (old_id, parent_id, name, cls_type, data_str, shortcut_key, global_shortcut_key, order) in &raw_classifications {
            if parent_id.is_none() || *parent_id == Some(0) {
                let parsed_data: Value = serde_json::from_str(data_str).unwrap_or(Value::Null);
                let icon = parsed_data.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string());

                let cls = Classification {
                    id: 0,
                    parent_id: None,
                    name: name.clone(),
                    classification_type: *cls_type,
                    data: ClassificationData {
                        icon,
                        ..Default::default()
                    },
                    shortcut_key: shortcut_key.clone(),
                    global_shortcut_key: *global_shortcut_key,
                    order: *order,
                    child_list: None,
                    item_count: None,
                };
                if let Ok(new_id) = db::save_classification(&cls) {
                    old_id_to_new_id.insert(*old_id, new_id);
                }
            }
        }

        // 再导入子分类
        for (old_id, parent_id, name, cls_type, data_str, shortcut_key, global_shortcut_key, order) in &raw_classifications {
            if let Some(pid) = parent_id {
                if *pid > 0 {
                    let new_parent_id = old_id_to_new_id.get(pid).copied();
                    let parsed_data: Value = serde_json::from_str(data_str).unwrap_or(Value::Null);
                    let icon = parsed_data.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string());

                    let cls = Classification {
                        id: 0,
                        parent_id: new_parent_id,
                        name: name.clone(),
                        classification_type: *cls_type,
                        data: ClassificationData {
                            icon,
                            ..Default::default()
                        },
                        shortcut_key: shortcut_key.clone(),
                        global_shortcut_key: *global_shortcut_key,
                        order: *order,
                        child_list: None,
                        item_count: None,
                    };
                    if let Ok(new_id) = db::save_classification(&cls) {
                        old_id_to_new_id.insert(*old_id, new_id);
                    }
                }
            }
        }

        // 读取 DawnLauncher 项目
        if let Ok(mut item_stmt) = src_conn.prepare("SELECT id, classification_id, name, type, data, shortcut_key, global_shortcut_key, `order` FROM item ORDER BY `order` ASC") {
            let item_iter = item_stmt.query_map([], |row| {
                let id: i64 = row.get::<_, Option<i64>>(0).unwrap_or(None).unwrap_or(0);
                let classification_id: i64 = row.get::<_, Option<i64>>(1).unwrap_or(None).unwrap_or(1);
                let name: String = row.get::<_, Option<String>>(2).unwrap_or(None).unwrap_or_else(|| "项目".to_string());
                let item_type: i32 = row.get::<_, Option<i32>>(3).unwrap_or(None).unwrap_or(0);
                let data_str: String = row.get::<_, Option<String>>(4).unwrap_or(None).unwrap_or_else(|| "{}".to_string());
                let shortcut_key: Option<String> = row.get::<_, Option<String>>(5).unwrap_or(None);
                let global_shortcut_key: bool = row.get::<_, Option<i64>>(6).unwrap_or(None).map(|v| v != 0).unwrap_or(false);
                let order: i32 = row.get::<_, Option<i32>>(7).unwrap_or(None).unwrap_or(0);

                Ok((id, classification_id, name, item_type, data_str, shortcut_key, global_shortcut_key, order))
            });

            if let Ok(rows) = item_iter {
                for r in rows {
                    if let Ok((_id, old_cls_id, name, item_type, data_str, shortcut_key, global_shortcut_key, order)) = r {
                        let target_cls_id = old_id_to_new_id.get(&old_cls_id).copied().unwrap_or(old_cls_id);
                        let parsed_data: Value = serde_json::from_str(&data_str).unwrap_or(Value::Null);

                        let target = parsed_data.get("target")
                            .or_else(|| parsed_data.get("path"))
                            .or_else(|| parsed_data.get("url"))
                            .or_else(|| parsed_data.get("shell"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        let params = parsed_data.get("params").and_then(|v| v.as_str()).map(|s| s.to_string());
                        let start_location = parsed_data.get("startLocation").and_then(|v| v.as_str()).map(|s| s.to_string());
                        let run_as_admin = parsed_data.get("runAsAdmin")
                            .or_else(|| parsed_data.get("admin"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let icon = parsed_data.get("icon").and_then(|v| v.as_str()).map(|s| s.to_string());
                        let html_icon = parsed_data.get("htmlIcon").and_then(|v| v.as_str()).map(|s| s.to_string());
                        let remark = parsed_data.get("remark").and_then(|v| v.as_str()).map(|s| s.to_string());

                        let item = Item {
                            id: 0,
                            classification_id: target_cls_id,
                            name,
                            item_type,
                            data: ItemData {
                                target,
                                params,
                                start_location,
                                run_as_admin,
                                icon,
                                html_icon,
                                remark,
                                ..Default::default()
                            },
                            shortcut_key,
                            global_shortcut_key,
                            order,
                        };
                        if db::save_item(&item).is_ok() {
                            imported_items_count += 1;
                        }
                    }
                }
            }
        }
    } else if is_anyversion_format {
        // 读取 AnyVersion 原生 SQLite 分类与项目
        let mut old_id_to_new_id: HashMap<i64, i64> = HashMap::new();
        let mut raw_classifications = Vec::new();

        if let Ok(mut stmt) = src_conn.prepare("SELECT id, parent_id, name, classification_type, data, shortcut_key, global_shortcut_key, sort_order FROM launcher_classification ORDER BY sort_order ASC") {
            let cls_iter = stmt.query_map([], |row| {
                let id: i64 = row.get::<_, Option<i64>>(0).unwrap_or(None).unwrap_or(0);
                let parent_id: Option<i64> = row.get::<_, Option<i64>>(1).unwrap_or(None);
                let name: String = row.get::<_, Option<String>>(2).unwrap_or(None).unwrap_or_else(|| "分类".to_string());
                let cls_type: i32 = row.get::<_, Option<i32>>(3).unwrap_or(None).unwrap_or(0);
                let data_str: String = row.get::<_, Option<String>>(4).unwrap_or(None).unwrap_or_else(|| "{}".to_string());
                let shortcut_key: Option<String> = row.get::<_, Option<String>>(5).unwrap_or(None);
                let global_shortcut_key: bool = row.get::<_, Option<i64>>(6).unwrap_or(None).map(|v| v != 0).unwrap_or(false);
                let order: i32 = row.get::<_, Option<i32>>(7).unwrap_or(None).unwrap_or(0);

                Ok((id, parent_id, name, cls_type, data_str, shortcut_key, global_shortcut_key, order))
            });

            if let Ok(rows) = cls_iter {
                for r in rows {
                    if let Ok(c) = r {
                        raw_classifications.push(c);
                    }
                }
            }
        }

        // 顶级分类
        for (old_id, parent_id, name, cls_type, data_str, shortcut_key, global_shortcut_key, order) in &raw_classifications {
            if parent_id.is_none() || *parent_id == Some(0) {
                let data: ClassificationData = serde_json::from_str(data_str).unwrap_or_default();
                let cls = Classification {
                    id: 0,
                    parent_id: None,
                    name: name.clone(),
                    classification_type: *cls_type,
                    data,
                    shortcut_key: shortcut_key.clone(),
                    global_shortcut_key: *global_shortcut_key,
                    order: *order,
                    child_list: None,
                    item_count: None,
                };
                if let Ok(new_id) = db::save_classification(&cls) {
                    old_id_to_new_id.insert(*old_id, new_id);
                }
            }
        }

        // 子分类
        for (old_id, parent_id, name, cls_type, data_str, shortcut_key, global_shortcut_key, order) in &raw_classifications {
            if let Some(pid) = parent_id {
                if *pid > 0 {
                    let new_parent_id = old_id_to_new_id.get(pid).copied();
                    let data: ClassificationData = serde_json::from_str(data_str).unwrap_or_default();
                    let cls = Classification {
                        id: 0,
                        parent_id: new_parent_id,
                        name: name.clone(),
                        classification_type: *cls_type,
                        data,
                        shortcut_key: shortcut_key.clone(),
                        global_shortcut_key: *global_shortcut_key,
                        order: *order,
                        child_list: None,
                        item_count: None,
                    };
                    if let Ok(new_id) = db::save_classification(&cls) {
                        old_id_to_new_id.insert(*old_id, new_id);
                    }
                }
            }
        }

        // AnyVersion 项目
        if let Ok(mut item_stmt) = src_conn.prepare("SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order FROM launcher_item ORDER BY sort_order ASC") {
            let item_iter = item_stmt.query_map([], |row| {
                let id: i64 = row.get::<_, Option<i64>>(0).unwrap_or(None).unwrap_or(0);
                let classification_id: i64 = row.get::<_, Option<i64>>(1).unwrap_or(None).unwrap_or(1);
                let name: String = row.get::<_, Option<String>>(2).unwrap_or(None).unwrap_or_else(|| "项目".to_string());
                let item_type: i32 = row.get::<_, Option<i32>>(3).unwrap_or(None).unwrap_or(0);
                let data_str: String = row.get::<_, Option<String>>(4).unwrap_or(None).unwrap_or_else(|| "{}".to_string());
                let shortcut_key: Option<String> = row.get::<_, Option<String>>(5).unwrap_or(None);
                let global_shortcut_key: bool = row.get::<_, Option<i64>>(6).unwrap_or(None).map(|v| v != 0).unwrap_or(false);
                let order: i32 = row.get::<_, Option<i32>>(7).unwrap_or(None).unwrap_or(0);

                Ok((id, classification_id, name, item_type, data_str, shortcut_key, global_shortcut_key, order))
            });

            if let Ok(rows) = item_iter {
                for r in rows {
                    if let Ok((_id, old_cls_id, name, item_type, data_str, shortcut_key, global_shortcut_key, order)) = r {
                        let target_cls_id = old_id_to_new_id.get(&old_cls_id).copied().unwrap_or(old_cls_id);
                        let data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();

                        let item = Item {
                            id: 0,
                            classification_id: target_cls_id,
                            name,
                            item_type,
                            data,
                            shortcut_key,
                            global_shortcut_key,
                            order,
                        };
                        if db::save_item(&item).is_ok() {
                            imported_items_count += 1;
                        }
                    }
                }
            }
        }
    }

    Ok(imported_items_count)
}

/// 导入 JSON 数据（支持 AnyVersion 备份与 DawnLauncher JSON 备份）
pub fn import_dawn_or_any_json(json_content: &str) -> Result<usize, String> {
    let parsed: Value = serde_json::from_str(json_content)
        .map_err(|e| format!("解析 JSON 格式失败: {}", e))?;

    let mut imported_count = 0usize;

    // 检查是否为 DawnLauncher JSON 结构 (含 list 字段)
    if let Some(list) = parsed.get("list").and_then(|v| v.as_array()) {
        let mut old_id_to_new_id: HashMap<i64, i64> = HashMap::new();

        for parent in list {
            let p_id = parent.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let p_name = parent.get("name").and_then(|v| v.as_str()).unwrap_or("新建分类");
            let p_type = parent.get("type").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let p_icon = parent.get("icon").or_else(|| parent.get("data").and_then(|d| d.get("icon"))).and_then(|v| v.as_str()).map(|s| s.to_string());

            let p_cls = Classification {
                id: 0,
                parent_id: None,
                name: p_name.to_string(),
                classification_type: p_type,
                data: ClassificationData {
                    icon: p_icon,
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
                child_list: None,
                item_count: None,
            };

            if let Ok(new_parent_id) = db::save_classification(&p_cls) {
                if p_id > 0 {
                    old_id_to_new_id.insert(p_id, new_parent_id);
                }

                // 子分类
                if let Some(child_list) = parent.get("childList").and_then(|v| v.as_array()) {
                    for child in child_list {
                        let c_id = child.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                        let c_name = child.get("name").and_then(|v| v.as_str()).unwrap_or("新建子分类");
                        let c_type = child.get("type").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        let c_icon = child.get("icon").or_else(|| child.get("data").and_then(|d| d.get("icon"))).and_then(|v| v.as_str()).map(|s| s.to_string());

                        let c_cls = Classification {
                            id: 0,
                            parent_id: Some(new_parent_id),
                            name: c_name.to_string(),
                            classification_type: c_type,
                            data: ClassificationData {
                                icon: c_icon,
                                ..Default::default()
                            },
                            shortcut_key: None,
                            global_shortcut_key: false,
                            order: 0,
                            child_list: None,
                            item_count: None,
                        };

                        if let Ok(new_child_id) = db::save_classification(&c_cls) {
                            if c_id > 0 {
                                old_id_to_new_id.insert(c_id, new_child_id);
                            }

                            // 子分类下的项目
                            if let Some(item_list) = child.get("itemList").and_then(|v| v.as_array()) {
                                for item_val in item_list {
                                    if let Some(item) = parse_dawn_item_json(item_val, new_child_id) {
                                        if db::save_item(&item).is_ok() {
                                            imported_count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // 顶级分类下的项目
                if let Some(item_list) = parent.get("itemList").and_then(|v| v.as_array()) {
                    for item_val in item_list {
                        if let Some(item) = parse_dawn_item_json(item_val, new_parent_id) {
                            if db::save_item(&item).is_ok() {
                                imported_count += 1;
                            }
                        }
                    }
                }
            }
        }
        return Ok(imported_count);
    }

    // 尝试按照 AnyVersion 标准 JSON 导入
    if parsed.get("classifications").is_some() || parsed.get("items").is_some() {
        db::import_backup(json_content)?;
        return Ok(1);
    }

    Err("未能识别的备份数据格式".to_string())
}

fn parse_dawn_item_json(val: &Value, classification_id: i64) -> Option<Item> {
    let name = val.get("name").and_then(|v| v.as_str())?.to_string();
    let item_type = val.get("type").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

    let target = val.get("target")
        .or_else(|| val.get("path"))
        .or_else(|| val.get("url"))
        .or_else(|| val.get("shell"))
        .or_else(|| val.get("data").and_then(|d| d.get("target")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let params = val.get("params")
        .or_else(|| val.get("data").and_then(|d| d.get("params")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let start_location = val.get("startLocation")
        .or_else(|| val.get("data").and_then(|d| d.get("startLocation")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let run_as_admin = val.get("runAsAdmin")
        .or_else(|| val.get("admin"))
        .or_else(|| val.get("data").and_then(|d| d.get("runAsAdmin")))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let icon = val.get("icon")
        .or_else(|| val.get("data").and_then(|d| d.get("icon")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let html_icon = val.get("htmlIcon")
        .or_else(|| val.get("data").and_then(|d| d.get("htmlIcon")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let remark = val.get("remark")
        .or_else(|| val.get("data").and_then(|d| d.get("remark")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(Item {
        id: 0,
        classification_id,
        name,
        item_type,
        data: ItemData {
            target,
            params,
            start_location,
            run_as_admin,
            icon,
            html_icon,
            remark,
            ..Default::default()
        },
        shortcut_key: None,
        global_shortcut_key: false,
        order: 0,
    })
}
