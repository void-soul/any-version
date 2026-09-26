use std::sync::Mutex;
use chrono::Local;
use rusqlite::{params, Connection};
use crate::commands::config::get_data_dir;
use super::models::{
    Classification, ClassificationData, DeleteClassificationResult, Item, ItemData, LauncherSetting,
};

static DB_CONN: Mutex<Option<Connection>> = Mutex::new(None);

pub(crate) fn migrate_item_usage_columns(conn: &Connection) -> Result<(), String> {
    // 兼容早期已经创建的启动器数据库：为旧表补齐统计列。
    let _ = conn.execute("ALTER TABLE launcher_item ADD COLUMN open_number INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE launcher_item ADD COLUMN last_open INTEGER NOT NULL DEFAULT 0", []);
    conn.execute("CREATE INDEX IF NOT EXISTS idx_item_open ON launcher_item(open_number)", [])
        .map_err(|e| format!("初始化启动统计索引失败: {}", e))?;
    Ok(())
}

pub(crate) fn increment_item_open_count_on(conn: &Connection, id: i64) -> Result<(), String> {
    let now_ts = Local::now().timestamp_millis();
    conn.execute(
        "UPDATE launcher_item SET open_number = open_number + 1, last_open = ?1 WHERE id = ?2",
        params![now_ts, id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

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

        CREATE TABLE IF NOT EXISTS launcher_setting (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("初始化启动器表失败: {}", e))?;

    migrate_item_usage_columns(&conn)?;

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

/// 收集 id 及其**全部**子孙分类（不只是直接子级）。
///
/// 旧实现只查一层子分类，孙级分类会留下 `parent_id` 指向已删除的父节点 ——
/// 它们在树上不可达、里面的项目也跟着看不见删不掉，成了永久孤儿数据。
fn descendant_ids_on(conn: &Connection, id: i64) -> Result<Vec<i64>, String> {
    let mut all = vec![id];
    let mut frontier = vec![id];
    let mut guard = 0usize;
    while !frontier.is_empty() && guard < 1000 {
        guard += 1;
        let mut stmt = conn
            .prepare("SELECT id FROM launcher_classification WHERE parent_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([frontier[0]], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        let mut next: Vec<i64> = Vec::new();
        for r in rows {
            if let Ok(cid) = r {
                // 防环兜底：脏数据成环时不重复入列
                if !all.contains(&cid) {
                    all.push(cid);
                    next.push(cid);
                }
            }
        }
        frontier.remove(0);
        frontier.extend(next);
    }
    Ok(all)
}

/// target 是否是 id 的子孙（用于拦截「把分类迁到它自己的子树里」）。
fn is_descendant_of(conn: &Connection, id: i64, target: i64) -> Result<bool, String> {
    let mut cursor = Some(target);
    let mut guard = 0;
    while let Some(cid) = cursor {
        if cid == id {
            return Ok(true);
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
    Ok(false)
}

/// 删除分类的两种方式。
pub enum DeleteMode {
    /// 连同子分类与所有项目一起删
    Cascade,
    /// 保留内容：把子分类与项目迁到 new_parent_id 之下，只删这个空壳分类
    Reassign { new_parent_id: i64 },
}

/// 删除分类，并**保证不留孤儿**。
///
/// - `Cascade`：整棵子树 + 树下所有项目一起删（旧实现漏了孙级，这里按整棵子树删）；
/// - `Reassign`：先把直接子分类与直属项目迁到新上级，再删分类本身 ——
///   这样内容都能在新位置看到，不会出现「看不见也删不掉」的项目。
pub fn delete_classification(id: i64, mode: DeleteMode) -> Result<DeleteClassificationResult, String> {
    with_conn(|conn| delete_classification_on(conn, id, mode))
}

/// 对给定连接执行删除（与全局连接解耦，单测直接跑这段真实逻辑）。
pub(crate) fn delete_classification_on(
    conn: &Connection,
    id: i64,
    mode: DeleteMode,
) -> Result<DeleteClassificationResult, String> {
    {
        // 生产侧全局连接由 with_conn 的 Mutex 串行化，这里不会再有并发写；
        // 用 unchecked_transaction 是为了让函数收 &Connection（单测也要调它）。
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        match mode {
            DeleteMode::Cascade => {
                let ids = descendant_ids_on(&tx, id)?;
                let mut deleted_items = 0usize;
                for cid in &ids {
                    deleted_items += tx
                        .execute(
                            "DELETE FROM launcher_item WHERE classification_id = ?1",
                            params![cid],
                        )
                        .map_err(|e| e.to_string())?;
                }
                let mut deleted_categories = 0usize;
                for cid in &ids {
                    deleted_categories += tx
                        .execute("DELETE FROM launcher_classification WHERE id = ?1", params![cid])
                        .map_err(|e| e.to_string())?;
                }
                tx.commit().map_err(|e| e.to_string())?;
                Ok(DeleteClassificationResult {
                    deleted_categories,
                    deleted_items,
                    ..Default::default()
                })
            }
            DeleteMode::Reassign { new_parent_id } => {
                if new_parent_id == id {
                    return Err("不能把内容迁移到自己之下".to_string());
                }
                if is_descendant_of(&tx, id, new_parent_id)? {
                    return Err("目标分类不能位于待删除分类之下（会把自己也一起搬走）".to_string());
                }
                // 新上级必须真实存在
                let exists: bool = tx
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM launcher_classification WHERE id = ?1",
                        params![new_parent_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if !exists {
                    return Err("目标分类不存在".to_string());
                }

                // ① 直接子分类搬过去（保留各自的孙级关系与顺序）
                let child_ids: Vec<i64> = {
                    let mut stmt = tx
                        .prepare("SELECT id FROM launcher_classification WHERE parent_id = ?1 ORDER BY sort_order ASC")
                        .map_err(|e| e.to_string())?;
                    let rows = stmt
                        .query_map([id], |row| row.get::<_, i64>(0))
                        .map_err(|e| e.to_string())?;
                    let mut v = Vec::new();
                    for r in rows {
                        if let Ok(c) = r {
                            v.push(c);
                        }
                    }
                    v
                };
                let max_order: i32 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_classification WHERE parent_id IS ?1",
                        params![new_parent_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                for (i, cid) in child_ids.iter().enumerate() {
                    tx.execute(
                        "UPDATE launcher_classification SET parent_id = ?1, sort_order = ?2 WHERE id = ?3",
                        params![new_parent_id, max_order + i as i32, cid],
                    )
                    .map_err(|e| e.to_string())?;
                }

                // ② 直属项目也搬过去：否则它们会挂在已删除的分类 id 上成为孤儿
                let moved_items = tx
                    .execute(
                        "UPDATE launcher_item SET classification_id = ?1 WHERE classification_id = ?2",
                        params![new_parent_id, id],
                    )
                    .map_err(|e| e.to_string())?;

                // ③ 删掉已经搬空的分类本身
                let deleted_categories = tx
                    .execute("DELETE FROM launcher_classification WHERE id = ?1", params![id])
                    .map_err(|e| e.to_string())?;

                tx.commit().map_err(|e| e.to_string())?;
                Ok(DeleteClassificationResult {
                    deleted_categories,
                    moved_categories: child_ids.len(),
                    moved_items,
                    ..Default::default()
                })
            }
        }
    }
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
            "SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open
             FROM launcher_item
             WHERE classification_id = ?1
             ORDER BY sort_order ASC, id ASC"
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

            let open_number: i64 = row.get(8)?;
            let last_open: i64 = row.get(9)?;
            let mut data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();
            // 启动次数/最近启动时间属于独立数据库列，覆盖旧版本 data 缓存。
            data.open_number = open_number;
            data.last_open = last_open;

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
            "SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open
             FROM launcher_item
             ORDER BY sort_order ASC, id ASC"
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

            let open_number: i64 = row.get(8)?;
            let last_open: i64 = row.get(9)?;
            let mut data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();
            // 启动次数/最近启动时间属于独立数据库列，覆盖旧版本 data 缓存。
            data.open_number = open_number;
            data.last_open = last_open;

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
        let mut data = item.data.clone();
        data.open_number = item.data.open_number;
        data.last_open = item.data.last_open;
        let data_json = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());

        if item.id > 0 {
            conn.execute(
                r#"
                UPDATE launcher_item
                SET classification_id = ?1, name = ?2, item_type = ?3, data = ?4,
                    shortcut_key = ?5, global_shortcut_key = ?6, sort_order = ?7,
                    open_number = ?8, last_open = ?9, updated_at = ?10
                WHERE id = ?11
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    item.order,
                    item.data.open_number,
                    item.data.last_open,
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
                INSERT INTO launcher_item (classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    item.data.open_number,
                    item.data.last_open,
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

            let data = item.data.clone();
            let data_json = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());

            tx.execute(
                r#"
                INSERT INTO launcher_item (classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    item.data.open_number,
                    item.data.last_open,
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

/// 批量删除所有检测结果为「不存在」的项目（data.exists == false），返回删除数量。
/// 未检测过（exists 为 null）或存在（exists == true）的项目不受影响。
pub fn delete_invalid_items() -> Result<usize, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, data FROM launcher_item WHERE data LIKE '%\"exists\":false%'",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let data_str: String = row.get(1)?;
                Ok((id, data_str))
            })
            .map_err(|e| e.to_string())?;

        let mut invalid_ids: Vec<i64> = Vec::new();
        for r in rows {
            if let Ok((id, data_str)) = r {
                // 精确判定：反序列化后 exists 为 false 才删除，避免误伤文本中偶然出现该串的项目
                if let Ok(data) = serde_json::from_str::<ItemData>(&data_str) {
                    if data.exists == Some(false) {
                        invalid_ids.push(id);
                    }
                }
            }
        }
        if invalid_ids.is_empty() {
            return Ok(0);
        }
        drop(stmt);
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for id in &invalid_ids {
            tx.execute("DELETE FROM launcher_item WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(invalid_ids.len())
    })
}

pub fn reorder_items(orders: Vec<(i64, i32)>) -> Result<(), String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (id, sort_order) in orders {
            // 不能吞掉错误：序号写失败必须让前端弹出「保存排序失败」，否则会表现为
            // 「拖完看着生效了，重新打开又变回原样」。
            tx.execute(
                "UPDATE launcher_item SET sort_order = ?1 WHERE id = ?2",
                params![sort_order, id],
            ).map_err(|e| format!("更新项目排序失败: {}", e))?;
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
    with_conn(|conn| increment_item_open_count_on(conn, id))
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
        // 数据迁移：思维导图速记热键默认 Shift+F3（同上，老配置缺字段时补齐）。
        if setting.mindmap_quick_hotkey.trim().is_empty() {
            setting.mindmap_quick_hotkey = crate::commands::launcher::models::default_mindmap_quick_hotkey();
            let json_str = serde_json::to_string(&setting).map_err(|e| e.to_string())?;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)",
                params![json_str],
            );
        }
        // 数据迁移：思维导图贴纸热键默认 Shift+F4。
        if setting.mindmap_sticker_hotkey.trim().is_empty() {
            setting.mindmap_sticker_hotkey = crate::commands::launcher::models::default_mindmap_sticker_hotkey();
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 与生产库同构的内存库（两张表），测试直接对它跑真实 SQL。
    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE launcher_classification (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                parent_id INTEGER,
                name TEXT NOT NULL,
                classification_type INTEGER NOT NULL DEFAULT 0,
                data TEXT NOT NULL DEFAULT '{}',
                shortcut_key TEXT,
                global_shortcut_key INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE launcher_item (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                classification_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                item_type INTEGER NOT NULL DEFAULT 0,
                data TEXT NOT NULL DEFAULT '{}',
                shortcut_key TEXT,
                global_shortcut_key INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            "#,
        )
        .unwrap();
        conn
    }

    fn add_cat(conn: &Connection, id: i64, parent: Option<i64>, name: &str) {
        conn.execute(
            "INSERT INTO launcher_classification (id, parent_id, name, sort_order) VALUES (?1, ?2, ?3, 0)",
            params![id, parent, name],
        )
        .unwrap();
    }

    fn add_item(conn: &Connection, cls: i64, name: &str) {
        conn.execute(
            "INSERT INTO launcher_item (classification_id, name) VALUES (?1, ?2)",
            params![cls, name],
        )
        .unwrap();
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    /// 孤儿检测：① 项目挂在已不存在的分类上；② 分类的 parent_id 指向已不存在的分类。
    /// 这两种都是「界面上永远看不到、也删不掉」的死数据。
    fn orphans(conn: &Connection) -> (i64, i64) {
        let items: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM launcher_item i
                 WHERE NOT EXISTS (SELECT 1 FROM launcher_classification c WHERE c.id = i.classification_id)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let cats: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM launcher_classification c
                 WHERE c.parent_id IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM launcher_classification p WHERE p.id = c.parent_id)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        (items, cats)
    }

    /// 回归：**孙级**分类不能留成孤儿 —— 旧实现只查一层直接子分类，
    /// 孙级分类与其项目会永久失联（看不见也删不掉）。
    #[test]
    fn cascade_deletes_the_whole_subtree_without_orphans() {
        let conn = mem();
        add_cat(&conn, 1, None, "一级");
        add_cat(&conn, 2, Some(1), "二级");
        add_cat(&conn, 3, Some(2), "三级");
        add_item(&conn, 1, "i1");
        add_item(&conn, 2, "i2");
        add_item(&conn, 3, "i3");

        let r = delete_classification_on(&conn, 1, DeleteMode::Cascade).unwrap();
        assert_eq!(r.deleted_categories, 3, "整棵子树都要删: {r:?}");
        assert_eq!(r.deleted_items, 3);
        assert_eq!(count(&conn, "launcher_classification"), 0);
        assert_eq!(count(&conn, "launcher_item"), 0);
        assert_eq!(orphans(&conn), (0, 0), "不该留下任何孤儿");
    }

    /// 迁移模式：子分类与项目都搬到新上级，只删空壳分类，同样不留孤儿。
    #[test]
    fn reassign_moves_children_and_items_then_deletes_the_shell() {
        let conn = mem();
        add_cat(&conn, 1, None, "一级");
        add_cat(&conn, 2, None, "新家");
        add_cat(&conn, 3, Some(1), "子分类");
        add_item(&conn, 1, "直属项目");
        add_item(&conn, 3, "孙级项目");

        let r = delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 2 }).unwrap();
        assert_eq!(r.deleted_categories, 1, "只删分类本身: {r:?}");
        assert_eq!(r.moved_categories, 1);
        assert_eq!(r.moved_items, 1);
        assert_eq!(r.deleted_items, 0, "保留模式下不能删项目");

        // 内容都还在，只是换了归属
        assert_eq!(count(&conn, "launcher_classification"), 2);
        assert_eq!(count(&conn, "launcher_item"), 2);
        assert_eq!(orphans(&conn), (0, 0));
        let parent: Option<i64> = conn
            .query_row("SELECT parent_id FROM launcher_classification WHERE id = 3", [], |r| r.get(0))
            .unwrap();
        assert_eq!(parent, Some(2));
        let cls: i64 = conn
            .query_row(
                "SELECT classification_id FROM launcher_item WHERE name = '直属项目'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cls, 2);
    }

    /// 非法迁移目标必须被拒：迁到自己 / 迁到自己的子树 / 目标不存在。
    #[test]
    fn reassign_rejects_illegal_targets() {
        let conn = mem();
        add_cat(&conn, 1, None, "一级");
        add_cat(&conn, 2, Some(1), "子级");

        assert!(delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 1 }).is_err());
        assert!(
            delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 2 }).is_err(),
            "不能迁到自己的子孙下"
        );
        assert!(
            delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 99 }).is_err(),
            "目标不存在要报错"
        );
        // 被拒后数据必须原封不动
        assert_eq!(count(&conn, "launcher_classification"), 2);
    }

    /// 分类层级成环（脏数据）时不能死循环。
    #[test]
    fn descendant_scan_survives_cycles() {
        let conn = mem();
        add_cat(&conn, 1, Some(2), "A");
        add_cat(&conn, 2, Some(1), "B");
        let ids = descendant_ids_on(&conn, 1).unwrap();
        assert!(ids.contains(&1) && ids.contains(&2));
    }
}
