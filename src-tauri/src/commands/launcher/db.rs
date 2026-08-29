use std::sync::Mutex;
use chrono::Local;
use rusqlite::{params, Connection};
use crate::commands::config::get_data_dir;
use super::models::{
    Classification, ClassificationData, Item, ItemData, LauncherSetting,
};

static DB_CONN: Mutex<Option<Connection>> = Mutex::new(None);

fn db_path() -> std::path::PathBuf {
    get_data_dir().join("launcher.db")
}

fn build_connection() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let conn = Connection::open(&path).map_err(|e| format!("打开启动器数据库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON").ok();

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS launcher_classification (
            id                    INTEGER PRIMARY KEY AUTOINCREMENT,
            parent_id             INTEGER,
            name                  TEXT    NOT NULL,
            classification_type   INTEGER NOT NULL DEFAULT 0,
            data                  TEXT    NOT NULL DEFAULT '{}',
            shortcut_key          TEXT,
            global_shortcut_key   INTEGER NOT NULL DEFAULT 0,
            sort_order            INTEGER NOT NULL DEFAULT 0,
            created_at            TEXT    NOT NULL,
            updated_at            TEXT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_cls_parent ON launcher_classification(parent_id);
        CREATE INDEX IF NOT EXISTS idx_cls_order  ON launcher_classification(sort_order);

        CREATE TABLE IF NOT EXISTS launcher_item (
            id                    INTEGER PRIMARY KEY AUTOINCREMENT,
            classification_id     INTEGER NOT NULL,
            name                  TEXT    NOT NULL,
            item_type             INTEGER NOT NULL DEFAULT 0,
            data                  TEXT    NOT NULL DEFAULT '{}',
            shortcut_key          TEXT,
            global_shortcut_key   INTEGER NOT NULL DEFAULT 0,
            sort_order            INTEGER NOT NULL DEFAULT 0,
            open_number           INTEGER NOT NULL DEFAULT 0,
            last_open             INTEGER NOT NULL DEFAULT 0,
            created_at            TEXT    NOT NULL,
            updated_at            TEXT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_item_cls   ON launcher_item(classification_id);
        CREATE INDEX IF NOT EXISTS idx_item_order ON launcher_item(sort_order);
        CREATE INDEX IF NOT EXISTS idx_item_open  ON launcher_item(open_number);

        CREATE TABLE IF NOT EXISTS launcher_setting (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("初始化启动器表失败: {}", e))?;

    // 检查是否需要播种初始默认数据
    let count: i64 = conn
        .query_row("SELECT COUNT(1) FROM launcher_classification", [], |row| row.get(0))
        .unwrap_or(0);

    if count == 0 {
        seed_default_data(&conn)?;
    }

    Ok(conn)
}

pub fn init_db() -> Result<(), String> {
    let conn = build_connection()?;
    *DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))? = Some(conn);
    Ok(())
}

fn with_conn<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&mut Connection) -> Result<T, String>,
{
    // 检查 + 初始化 + 使用放在同一锁临界区，避免并发重复初始化覆盖连接。
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    if guard.is_none() {
        *guard = Some(build_connection()?);
    }
    match guard.as_mut() {
        Some(conn) => f(conn),
        None => Err("启动器数据库未连接".to_string()),
    }
}

/// 默认数据播种
fn seed_default_data(conn: &Connection) -> Result<(), String> {
    let now = Local::now().to_rfc3339();

    // 默认分类 1: 常用推荐
    conn.execute(
        "INSERT INTO launcher_classification (id, parent_id, name, classification_type, data, sort_order, created_at, updated_at) VALUES (1, NULL, '常用推荐', 0, '{\"icon\":\"🚀\"}', 1, ?1, ?1)",
        params![now],
    ).ok();

    // 默认分类 2: 系统工具
    conn.execute(
        "INSERT INTO launcher_classification (id, parent_id, name, classification_type, data, sort_order, created_at, updated_at) VALUES (2, NULL, '系统工具', 0, '{\"icon\":\"⚙️\"}', 2, ?1, ?1)",
        params![now],
    ).ok();

    // 默认分类 3: 聚合统计 (聚合分类)
    conn.execute(
        "INSERT INTO launcher_classification (id, parent_id, name, classification_type, data, sort_order, created_at, updated_at) VALUES (3, NULL, '最常打开', 2, '{\"icon\":\"🔥\",\"aggregateItemCount\":20,\"aggregateSort\":\"openNumber\"}', 3, ?1, ?1)",
        params![now],
    ).ok();

    // 默认系统工具预置条目
    let sys_items = vec![
        ("任务管理器", "taskmgr.exe", "", "⚡", 2),
        ("控制面板", "control.exe", "", "🎛️", 2),
        ("系统属性与环境变量", "sysdm.cpl", "", "🌿", 2),
        ("设备管理器", "devmgmt.msc", "", "🖥️", 2),
        ("计算机管理", "compmgmt.msc", "", "🏢", 2),
        ("服务列表", "services.msc", "", "🛠️", 2),
        ("注册表编辑器", "regedit.exe", "", "📝", 2),
        ("锁定计算机", "static:LockWorkstation", "", "🔒", 2),
        ("清空回收站", "static:EmptyRecycleBin", "", "🗑️", 2),
        ("关闭显示器", "static:TurnOffMonitor", "", "🌙", 2),
        ("重启资源管理器", "static:RestartExplorer", "", "🔄", 2),
        ("命令提示符 (CMD)", "cmd.exe", "", "💻", 1),
        ("Windows PowerShell", "powershell.exe", "", "🔷", 1),
        ("Windows 终端", "wt.exe", "", "⬛", 1),
    ];

    for (idx, (name, target, params_str, icon_emoji, cls_id)) in sys_items.into_iter().enumerate() {
        let item_data = serde_json::json!({
            "target": target,
            "params": params_str,
            "htmlIcon": icon_emoji,
            "runAsAdmin": false,
            "openNumber": 0,
            "lastOpen": 0,
        });
        conn.execute(
            "INSERT INTO launcher_item (classification_id, name, item_type, data, sort_order, created_at, updated_at) VALUES (?1, ?2, 3, ?3, ?4, ?5, ?5)",
            params![cls_id, name, item_data.to_string(), (idx + 1) as i32, now],
        ).ok();
    }

    // 默认设置
    let default_setting = LauncherSetting::default();
    let val_json = serde_json::to_string(&default_setting).unwrap_or_default();
    conn.execute(
        "INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)",
        params![val_json],
    ).ok();

    Ok(())
}

// ---------------------- 分类操作 ----------------------

pub fn list_classifications() -> Result<Vec<Classification>, String> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.parent_id, c.name, c.classification_type, c.data, c.shortcut_key, c.global_shortcut_key, c.sort_order,
             (SELECT COUNT(1) FROM launcher_item i WHERE i.classification_id = c.id) as item_cnt
             FROM launcher_classification c
             ORDER BY c.sort_order ASC"
        ).map_err(|e| e.to_string())?;

        let mut all_list = Vec::new();
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let parent_id: Option<i64> = row.get(1)?;
            let name: String = row.get(2)?;
            let classification_type: i32 = row.get(3)?;
            let data_str: String = row.get(4)?;
            let shortcut_key: Option<String> = row.get(5)?;
            let global_shortcut_key_int: i32 = row.get(6)?;
            let sort_order: i32 = row.get(7)?;
            let item_cnt: i64 = row.get(8)?;

            let data: ClassificationData = serde_json::from_str(&data_str).unwrap_or_default();

            Ok(Classification {
                id,
                parent_id,
                name,
                classification_type,
                data,
                shortcut_key,
                global_shortcut_key: global_shortcut_key_int != 0,
                order: sort_order,
                child_list: Some(Vec::new()),
                item_count: Some(item_cnt as usize),
            })
        }).map_err(|e| e.to_string())?;

        for r in rows {
            if let Ok(c) = r {
                all_list.push(c);
            }
        }

        // 返回全部分类（扁平列表），前端按 parentId 自行构建层级树。
        // 注意：不能只返回顶级分类，否则前端 filter(c => c.parentId === cat.id)
        // 永远找不到子分类，导致有子分类的分类显示空白。
        Ok(all_list)
    })
}

pub fn save_classification(cls: &Classification) -> Result<i64, String> {
    with_conn(|conn| {
        let now = Local::now().to_rfc3339();
        let data_json = serde_json::to_string(&cls.data).unwrap_or_else(|_| "{}".to_string());

        if cls.id > 0 {
            conn.execute(
                r#"
                UPDATE launcher_classification
                SET parent_id = ?1, name = ?2, classification_type = ?3, data = ?4,
                    shortcut_key = ?5, global_shortcut_key = ?6, sort_order = ?7, updated_at = ?8
                WHERE id = ?9
                "#,
                params![
                    cls.parent_id,
                    cls.name,
                    cls.classification_type,
                    data_json,
                    cls.shortcut_key,
                    if cls.global_shortcut_key { 1 } else { 0 },
                    cls.order,
                    now,
                    cls.id,
                ],
            ).map_err(|e| e.to_string())?;
            Ok(cls.id)
        } else {
            let max_order: i32 = conn.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_classification WHERE parent_id IS ?1",
                params![cls.parent_id],
                |row| row.get(0)
            ).unwrap_or(0);

            conn.execute(
                r#"
                INSERT INTO launcher_classification (parent_id, name, classification_type, data, shortcut_key, global_shortcut_key, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                "#,
                params![
                    cls.parent_id,
                    cls.name,
                    cls.classification_type,
                    data_json,
                    cls.shortcut_key,
                    if cls.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    now,
                ],
            ).map_err(|e| e.to_string())?;

            let new_id = conn.last_insert_rowid();
            Ok(new_id)
        }
    })
}

pub fn delete_classification(id: i64) -> Result<(), String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        // 查找所有子分类
        let mut child_ids = vec![id];
        {
            let mut stmt = tx.prepare("SELECT id FROM launcher_classification WHERE parent_id = ?1").map_err(|e| e.to_string())?;
            let rows = stmt.query_map([id], |row| row.get(0)).map_err(|e| e.to_string())?;
            for r in rows {
                if let Ok(cid) = r {
                    child_ids.push(cid);
                }
            }
        }

        for cid in child_ids {
            tx.execute("DELETE FROM launcher_item WHERE classification_id = ?1", params![cid]).ok();
            tx.execute("DELETE FROM launcher_classification WHERE id = ?1", params![cid]).ok();
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub fn reorder_classifications(orders: Vec<(i64, i32)>) -> Result<(), String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (id, sort_order) in orders {
            tx.execute(
                "UPDATE launcher_classification SET sort_order = ?1 WHERE id = ?2",
                params![sort_order, id],
            ).ok();
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ---------------------- 项目操作 ----------------------

pub fn list_items_by_classification(cls_id: i64) -> Result<Vec<Item>, String> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order
             FROM launcher_item
             WHERE classification_id = ?1
             ORDER BY sort_order ASC"
        ).map_err(|e| e.to_string())?;

        let rows = stmt.query_map([cls_id], |row| {
            let id: i64 = row.get(0)?;
            let classification_id: i64 = row.get(1)?;
            let name: String = row.get(2)?;
            let item_type: i32 = row.get(3)?;
            let data_str: String = row.get(4)?;
            let shortcut_key: Option<String> = row.get(5)?;
            let global_shortcut_key_int: i32 = row.get(6)?;
            let sort_order: i32 = row.get(7)?;

            let data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();

            Ok(Item {
                id,
                classification_id,
                name,
                item_type,
                data,
                shortcut_key,
                global_shortcut_key: global_shortcut_key_int != 0,
                order: sort_order,
            })
        }).map_err(|e| e.to_string())?;

        let mut items = Vec::new();
        for r in rows {
            if let Ok(it) = r {
                items.push(it);
            }
        }
        Ok(items)
    })
}

pub fn list_all_items() -> Result<Vec<Item>, String> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order
             FROM launcher_item
             ORDER BY sort_order ASC"
        ).map_err(|e| e.to_string())?;

        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let classification_id: i64 = row.get(1)?;
            let name: String = row.get(2)?;
            let item_type: i32 = row.get(3)?;
            let data_str: String = row.get(4)?;
            let shortcut_key: Option<String> = row.get(5)?;
            let global_shortcut_key_int: i32 = row.get(6)?;
            let sort_order: i32 = row.get(7)?;

            let data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();

            Ok(Item {
                id,
                classification_id,
                name,
                item_type,
                data,
                shortcut_key,
                global_shortcut_key: global_shortcut_key_int != 0,
                order: sort_order,
            })
        }).map_err(|e| e.to_string())?;

        let mut items = Vec::new();
        for r in rows {
            if let Ok(it) = r {
                items.push(it);
            }
        }
        Ok(items)
    })
}

pub fn save_item(item: &Item) -> Result<i64, String> {
    with_conn(|conn| {
        let now = Local::now().to_rfc3339();
        let data_json = serde_json::to_string(&item.data).unwrap_or_else(|_| "{}".to_string());

        if item.id > 0 {
            conn.execute(
                r#"
                UPDATE launcher_item
                SET classification_id = ?1, name = ?2, item_type = ?3, data = ?4,
                    shortcut_key = ?5, global_shortcut_key = ?6, sort_order = ?7,
                    updated_at = ?8
                WHERE id = ?9
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    item.order,
                    now,
                    item.id,
                ],
            ).map_err(|e| e.to_string())?;
            Ok(item.id)
        } else {
            let max_order: i32 = conn.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_item WHERE classification_id = ?1",
                params![item.classification_id],
                |row| row.get(0)
            ).unwrap_or(0);

            conn.execute(
                r#"
                INSERT INTO launcher_item (classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    now,
                ],
            ).map_err(|e| e.to_string())?;

            let new_id = conn.last_insert_rowid();
            Ok(new_id)
        }
    })
}

pub fn batch_add_items(items: Vec<Item>) -> Result<Vec<i64>, String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let now = Local::now().to_rfc3339();
        let mut inserted_ids = Vec::new();

        for item in items {
            let max_order: i32 = tx.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_item WHERE classification_id = ?1",
                params![item.classification_id],
                |row| row.get(0)
            ).unwrap_or(0);

            let data_json = serde_json::to_string(&item.data).unwrap_or_else(|_| "{}".to_string());

            tx.execute(
                r#"
                INSERT INTO launcher_item (classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    now,
                ],
            ).map_err(|e| e.to_string())?;

            inserted_ids.push(tx.last_insert_rowid());
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(inserted_ids)
    })
}

pub fn delete_item(id: i64) -> Result<(), String> {
    with_conn(|conn| {
        conn.execute("DELETE FROM launcher_item WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

pub fn reorder_items(orders: Vec<(i64, i32)>) -> Result<(), String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (id, sort_order) in orders {
            tx.execute(
                "UPDATE launcher_item SET sort_order = ?1 WHERE id = ?2",
                params![sort_order, id],
            ).ok();
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// 批量移动子分类：把 source 分类下的所有【直接子分类】整体移动到 target 分类之下。
/// 每个被移动的子分类完整保留其自身的子孙层级（parent_id 不变）与项目（classification_id 不变），
/// 只是把直接子分类的 parent_id 改成 target。返回移动的子分类数量。source 分类自身不移动。
pub fn move_subcategories_to_classification(source_id: i64, target_id: i64) -> Result<usize, String> {
    with_conn(|conn| {
        if source_id == target_id {
            return Err("源分类与目标分类不能相同".to_string());
        }

        // 校验 target 不能是 source 的子孙（否则会把 target 也一并搬走造成循环）
        {
            let mut cursor = Some(target_id);
            let mut guard = 0;
            while let Some(cid) = cursor {
                if cid == source_id {
                    return Err("目标分类不能位于源分类之下".to_string());
                }
                guard += 1;
                if guard > 1000 {
                    return Err("分类层级过深，已中止".to_string());
                }
                cursor = conn
                    .query_row(
                        "SELECT parent_id FROM launcher_classification WHERE id = ?1",
                        params![cid],
                        |row| row.get::<_, Option<i64>>(0),
                    )
                    .map_err(|e| e.to_string())?;
            }
        }

        // 收集 source 的所有直接子分类 id（按原顺序）
        let child_ids: Vec<i64> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM launcher_classification WHERE parent_id = ?1 ORDER BY sort_order ASC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([source_id], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())?;
            let mut v = Vec::new();
            for r in rows {
                if let Ok(i) = r {
                    v.push(i);
                }
            }
            v
        };

        if child_ids.is_empty() {
            return Ok(0);
        }

        // 目标分类下当前最大 sort_order
        let max_order: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_classification WHERE parent_id IS ?1",
                params![target_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (i, cid) in child_ids.iter().enumerate() {
            tx.execute(
                "UPDATE launcher_classification SET parent_id = ?1, sort_order = ?2 WHERE id = ?3",
                params![target_id, max_order + i as i32, cid],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(child_ids.len())
    })
}

pub fn increment_item_open_count(id: i64) -> Result<(), String> {
    with_conn(|conn| {
        let now_ts = Local::now().timestamp_millis();
        conn.execute(
            "UPDATE launcher_item SET open_number = open_number + 1, last_open = ?1 WHERE id = ?2",
            params![now_ts, id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ---------------------- 设置操作 ----------------------

pub fn get_settings() -> Result<LauncherSetting, String> {
    with_conn(|conn| {
        let res: Result<String, _> = conn.query_row(
            "SELECT value FROM launcher_setting WHERE key = 'global'",
            [],
            |row| row.get(0),
        );
        let mut setting: LauncherSetting = match res {
            Ok(json_str) => serde_json::from_str(&json_str).unwrap_or_default(),
            Err(_) => LauncherSetting::default(),
        };

        // 数据迁移：旧版本的「启动」模块快捷键存在 show_hide_shortcut_key，
        // 现统一迁移到 module_hotkeys["launcher"]（所有模块快捷键平等）。
        if !setting.show_hide_shortcut_key.is_empty()
            && !setting.module_hotkeys.contains_key("launcher")
        {
            setting
                .module_hotkeys
                .insert("launcher".to_string(), setting.show_hide_shortcut_key.clone());
            setting.show_hide_shortcut_key.clear();
            // 立即落盘，避免每次读取都重复迁移。
            let json_str = serde_json::to_string(&setting).map_err(|e| e.to_string())?;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)",
                params![json_str],
            );
        }
        // 数据迁移：划词翻译热键默认 F6（老配置为空串时不触发 serde 默认值，需在此补齐）。
        if setting.selection_translate_hotkey.trim().is_empty() {
            setting.selection_translate_hotkey = crate::commands::launcher::models::default_selection_translate_hotkey();
            let json_str = serde_json::to_string(&setting).map_err(|e| e.to_string())?;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)",
                params![json_str],
            );
        }
        // 数据迁移：思维导图速记热键默认 F7（同上，老配置缺字段时补齐）。
        if setting.mindmap_quick_hotkey.trim().is_empty() {
            setting.mindmap_quick_hotkey = crate::commands::launcher::models::default_mindmap_quick_hotkey();
            let json_str = serde_json::to_string(&setting).map_err(|e| e.to_string())?;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)",
                params![json_str],
            );
        }
        Ok(setting)
    })
}

pub fn save_settings(setting: &LauncherSetting) -> Result<(), String> {
    with_conn(|conn| {
        let json_str = serde_json::to_string(setting).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)",
            params![json_str],
        ).map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ---------------------- 备份与恢复 ----------------------

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
    let backup: LauncherBackup = serde_json::from_str(json_str)
        .map_err(|e| format!("解析备份文件失败: {}", e))?;

    save_settings(&backup.setting)?;

    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM launcher_item", []).ok();
        tx.execute("DELETE FROM launcher_classification", []).ok();

        let now = Local::now().to_rfc3339();

        // 扁平化导入分类
        fn insert_cls(tx: &rusqlite::Transaction, cls: &Classification, now: &str) -> Result<(), rusqlite::Error> {
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
                INSERT INTO launcher_item (id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
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
                    now,
                ],
            ).map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}
