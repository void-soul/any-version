//! Application settings retained in the legacy launcher database for compatibility.
//! Mutations are field patches applied to the latest row inside one transaction.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use super::models::LauncherSetting;

fn save_on(conn: &Connection, settings: &LauncherSetting) -> Result<(), String> {
    let value = serde_json::to_string(settings).map_err(|e| e.to_string())?;
    conn.execute("INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)", params![value])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn get_on(conn: &Connection) -> Result<LauncherSetting, String> {
    let raw: Option<String> = conn.query_row(
        "SELECT value FROM launcher_setting WHERE key = 'global'", [], |row| row.get(0)
    ).optional().map_err(|e| e.to_string())?;
    let Some(raw) = raw else { return Ok(LauncherSetting::default()); };
    let original: Value = serde_json::from_str(&raw).map_err(|e| format!("读取应用设置失败: {e}"))?;
    let mut settings: LauncherSetting = serde_json::from_value(original.clone())
        .map_err(|e| format!("读取应用设置失败: {e}"))?;
    // Missing fields get serde defaults. Explicitly cleared hotkeys stay cleared.
    if !settings.show_hide_shortcut_key.is_empty() {
        let has_launcher = original.get("moduleHotkeys").and_then(Value::as_object)
            .is_some_and(|hotkeys| hotkeys.contains_key("launcher"));
        if !has_launcher {
            settings.module_hotkeys.insert("launcher".into(), settings.show_hide_shortcut_key.clone());
        }
        settings.show_hide_shortcut_key.clear();
        save_on(conn, &settings)?;
    }
    Ok(settings)
}

fn patch_on(conn: &mut Connection, patch: Value) -> Result<LauncherSetting, String> {
    let patch = patch.as_object().ok_or("设置修改必须是对象")?;
    if serde_json::to_vec(patch).map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return Err("设置修改内容过大".into());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut latest = serde_json::to_value(get_on(&tx)?).map_err(|e| e.to_string())?;
    let fields = latest.as_object_mut().ok_or("应用设置内容无效")?;
    for (key, value) in patch {
        if !fields.contains_key(key) { return Err(format!("不支持的应用设置: {key}")); }
        if key == "moduleHotkeys" {
            let updates = value.as_object().ok_or("模块快捷键修改必须是对象")?;
            let hotkeys = fields.get_mut(key).and_then(Value::as_object_mut).ok_or("模块快捷键内容无效")?;
            for (module, hotkey) in updates {
                if module.len() > 128 { return Err("模块名称过长".into()); }
                if hotkey.is_null() { hotkeys.remove(module); }
                else if hotkey.as_str().is_some_and(|s| s.len() <= 128) { hotkeys.insert(module.clone(), hotkey.clone()); }
                else { return Err("快捷键必须是字符串，最多128字符".into()); }
            }
        } else { fields.insert(key.clone(), value.clone()); }
    }
    let settings: LauncherSetting = serde_json::from_value(latest)
        .map_err(|e| format!("应用设置参数无效: {e}"))?;
    save_on(&tx, &settings)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(settings)
}

pub fn get_settings() -> Result<LauncherSetting, String> { super::db::with_conn(|conn| get_on(conn)) }
pub fn save_settings(settings: &LauncherSetting) -> Result<(), String> { super::db::with_conn(|conn| save_on(conn, settings)) }
pub fn patch_settings(patch: Value) -> Result<LauncherSetting, String> { super::db::with_conn(|conn| patch_on(conn, patch)) }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE launcher_setting (key TEXT PRIMARY KEY, value TEXT NOT NULL)", []).unwrap();
        conn
    }
    #[test]
    fn independent_field_patches_preserve_other_modules() {
        let mut conn = db();
        patch_on(&mut conn, json!({"musicPlayPauseHotkey":"F8", "moduleHotkeys":{"music":"F9"}})).unwrap();
        let next = patch_on(&mut conn, json!({"mindmapQuickHotkey":"F10", "moduleHotkeys":{"mindmap":"F11"}})).unwrap();
        assert_eq!(next.music_play_pause_hotkey, "F8");
        assert_eq!(next.mindmap_quick_hotkey, "F10");
        assert_eq!(next.module_hotkeys.get("music").unwrap(), "F9");
        assert_eq!(next.module_hotkeys.get("mindmap").unwrap(), "F11");
    }
    #[test]
    fn invalid_patch_is_rejected_without_partial_write() {
        let mut conn = db();
        patch_on(&mut conn, json!({"musicPlayPauseHotkey":"F8"})).unwrap();
        assert!(patch_on(&mut conn, json!({"musicPlayPauseHotkey":"F9","unknown":true})).is_err());
        assert!(patch_on(&mut conn, json!({"musicPlayPauseHotkey":42})).is_err());
        assert_eq!(get_on(&conn).unwrap().music_play_pause_hotkey, "F8");
    }
    #[test]
    fn explicitly_cleared_hotkeys_do_not_reappear_on_read() {
        let mut conn = db();
        patch_on(&mut conn, json!({"selectionTranslateHotkey":"", "mindmapQuickHotkey":""})).unwrap();
        let next = get_on(&conn).unwrap();
        assert!(next.selection_translate_hotkey.is_empty());
        assert!(next.mindmap_quick_hotkey.is_empty());
    }
}
