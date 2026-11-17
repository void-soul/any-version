use super::models::RdpConnection;
use std::path::PathBuf;
use std::sync::Mutex;

static STORE_LOCK: Mutex<()> = Mutex::new(());

fn store_path() -> PathBuf {
    crate::commands::config::get_data_dir()
        .join("rdp")
        .join("connections.json")
}

/// 文件不存在 = 空列表；文件损坏 = 报错且不覆盖原文件。
pub fn load() -> Result<Vec<RdpConnection>, String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = store_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取远程桌面连接失败 {}: {e}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("远程桌面连接文件无效，未覆盖原文件 {}: {e}", path.display()))
}

pub fn save(items: &[RdpConnection]) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bytes = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    crate::commands::foundation::atomic_write_file(&store_path(), &bytes)
}

/// 从 FinalShell 原始配置推导「驱动器映射」开关。
/// 只有 `source=finalshell` 且原始 JSON 明确写了 `drivestoredirect` 才有答案；
/// 其余情况返回 `None`（表示「无从判断」，不要覆盖用户的选择）。
pub fn derive_map_drives(source: &str, raw: Option<&str>) -> Option<bool> {
    if source != "finalshell" {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(raw?).ok()?;
    parsed.get("drivestoredirect")?.as_bool()
}

/// 一次性迁移：旧记录没有 `mapDrives` / `clipboard` 键（字段是后加的）。
///
/// 驱动器：能从 FinalShell 原始配置读出来就用它，读不到按新默认（开启）。
/// 剪贴板：FinalShell 配置里没有对应字段，一律按新默认（开启）。
/// 只处理「键不存在」的记录，用户手动改过的一律不动。
pub fn migrate_legacy_switches() -> Result<bool, String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = store_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(false);
    };
    let Some(items) = value.as_array_mut() else {
        return Ok(false);
    };

    let mut changed = false;
    for item in items {
        let Some(obj) = item.as_object_mut() else { continue };
        if !obj.contains_key("mapDrives") {
            let source = obj.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let raw = obj.get("finalshellRaw").and_then(|v| v.as_str());
            let flag = derive_map_drives(source, raw).unwrap_or(true);
            obj.insert("mapDrives".into(), serde_json::Value::Bool(flag));
            changed = true;
        }
        if !obj.contains_key("clipboard") {
            obj.insert("clipboard".into(), serde_json::Value::Bool(true));
            changed = true;
        }
    }

    if changed {
        let bytes = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
        crate::commands::foundation::atomic_write_file(&path, &bytes)?;
    }
    Ok(changed)
}

fn groups_path() -> PathBuf {
    crate::commands::config::get_data_dir()
        .join("rdp")
        .join("groups.json")
}

/// 空文件夹清单（连接为空的文件夹没有别处可存）。
pub fn load_groups() -> Result<Vec<String>, String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = groups_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取远程连接文件夹失败 {}: {e}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("远程连接文件夹文件无效，未覆盖原文件 {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_map_drives_only_trusts_finalshell_raw() {
        let raw = r#"{"drivestoredirect":true}"#;
        assert_eq!(derive_map_drives("finalshell", Some(raw)), Some(true));
        let raw_off = r#"{"drivestoredirect":false}"#;
        assert_eq!(derive_map_drives("finalshell", Some(raw_off)), Some(false));
        // 本地连接、原始配置缺失、字段缺失：都无从判断，不能瞎设
        assert_eq!(derive_map_drives("local", Some(raw)), None);
        assert_eq!(derive_map_drives("finalshell", None), None);
        assert_eq!(derive_map_drives("finalshell", Some("{}")), None);
        assert_eq!(derive_map_drives("finalshell", Some("not json")), None);
    }

    /// 迁移只补「缺键」的记录：有原始配置用原值，没有就按新默认开启；已有键不覆盖。
    #[test]
    fn legacy_migration_fills_missing_keys_with_defaults() {
        // 测试环境的数据目录是隔离的（见 get_base_dir 的 cfg(test) 实现）
        let path = store_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("建目录");
        }
        let sample = format!(
            r#"[
              {{"id":"a","kind":"rdp","source":"finalshell","finalshellRaw":{raw}}},
              {{"id":"b","kind":"rdp","source":"finalshell","mapDrives":false,"finalshellRaw":{raw}}},
              {{"id":"c","kind":"rdp","source":"local"}}
            ]"#,
            raw = serde_json::to_string(r#"{"drivestoredirect":true}"#).unwrap()
        );
        std::fs::write(&path, sample).expect("写样本");

        assert!(migrate_legacy_switches().expect("迁移"), "应发生迁移");

        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let items = value.as_array().unwrap();
        assert_eq!(items[0].get("mapDrives").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            items[1].get("mapDrives").and_then(|v| v.as_bool()),
            Some(false),
            "用户选过的不覆盖"
        );
        assert_eq!(
            items[2].get("mapDrives").and_then(|v| v.as_bool()),
            Some(true),
            "无原始配置的旧记录按新默认开启"
        );
        assert_eq!(items[0].get("clipboard").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(items[1].get("clipboard").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(items[2].get("clipboard").and_then(|v| v.as_bool()), Some(true));
        assert!(!migrate_legacy_switches().expect("二次迁移"), "已迁移过应无改动");
    }
}

pub fn save_groups(groups: &[String]) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bytes = serde_json::to_vec_pretty(groups).map_err(|e| e.to_string())?;
    crate::commands::foundation::atomic_write_file(&groups_path(), &bytes)
}


