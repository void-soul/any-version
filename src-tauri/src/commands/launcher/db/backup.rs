use super::*;

// ---------------------- 设置操作 ----------------------

pub use crate::commands::launcher::settings::{get_settings, save_settings};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct LauncherBackup {
    pub version: i32,
    pub setting: LauncherSetting,
    pub classifications: Vec<Classification>,
    pub items: Vec<Item>,
}

pub fn export_backup() -> Result<String, String> {
    let setting = get_settings()?;
    let classifications = list_classifications()?;
    let items = list_all_items()?;

    let backup = LauncherBackup {
        version: 1,
        setting,
        classifications,
        items,
    };

    serde_json::to_string_pretty(&backup).map_err(|e| format!("导出 JSON 失败: {}", e))
}

pub fn import_backup(json_str: &str) -> Result<(), String> {
    let backup: LauncherBackup =
        serde_json::from_str(json_str).map_err(|e| format!("解析备份文件失败: {}", e))?;

    save_settings(&backup.setting)?;

    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM launcher_item", []).ok();
        tx.execute("DELETE FROM launcher_classification", []).ok();

        let now = Local::now().to_rfc3339();

        // 扁平化导入分类
        fn insert_cls(
            tx: &rusqlite::Transaction,
            cls: &Classification,
            now: &str,
        ) -> Result<(), rusqlite::Error> {
            let data_json = serde_json::to_string(&cls.data).unwrap_or_else(|_| "{}".to_string());
            tx.execute(
                r#"
                INSERT INTO launcher_classification (id, parent_id, name, classification_type, data, shortcut_key, global_shortcut_key, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                "#,
                params![
                    cls.id,
                    cls.parent_id,
                    cls.name,
                    cls.classification_type,
                    data_json,
                    cls.shortcut_key,
                    if cls.global_shortcut_key { 1 } else { 0 },
                    cls.order,
                    now,
                ],
            )?;
            if let Some(children) = &cls.child_list {
                for child in children {
                    insert_cls(tx, child, now)?;
                }
            }
            Ok(())
        }

        for cls in &backup.classifications {
            insert_cls(&tx, cls, &now).map_err(|e| e.to_string())?;
        }

        for it in &backup.items {
            let data_json = serde_json::to_string(&it.data).unwrap_or_else(|_| "{}".to_string());
            tx.execute(
                r#"
                INSERT INTO launcher_item (id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
                "#,
                params![
                    it.id,
                    it.classification_id,
                    it.name,
                    it.item_type,
                    data_json,
                    it.shortcut_key,
                    if it.global_shortcut_key { 1 } else { 0 },
                    it.order,
                    it.data.open_number,
                    it.data.last_open,
                    now,
                ],
            ).map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}
