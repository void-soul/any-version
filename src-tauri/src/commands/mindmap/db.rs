//! 思维导图 SQLite 持久化：{data_dir}/mindmap.db

use std::sync::Mutex;
use crate::commands::config::get_data_dir;
use super::models::*;

static DB_CONN: Mutex<Option<rusqlite::Connection>> = Mutex::new(None);

fn db_path() -> std::path::PathBuf { get_data_dir().join("mindmap.db") }

/// 打开并初始化连接（不持有全局锁；由调用方在锁内调用，避免并发重复初始化）。
fn build_connection() -> Result<rusqlite::Connection, String> {
    let path = db_path();
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
    let conn = rusqlite::Connection::open(&path).map_err(|e| format!("打开思维导图数据库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| format!("WAL: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON").ok();

    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS mindmap_folders (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, sort_order INTEGER NOT NULL DEFAULT 0,
            parent_id TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS mindmap_documents (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
            source_type TEXT NOT NULL DEFAULT 'manual', source_desc TEXT NOT NULL DEFAULT '',
            folder_id TEXT, background_texture TEXT NOT NULL DEFAULT 'dots', layout_dir TEXT NOT NULL DEFAULT 'lr', created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            FOREIGN KEY(folder_id) REFERENCES mindmap_folders(id) ON DELETE SET NULL
        );
        CREATE TABLE IF NOT EXISTS mindmap_nodes (
            id TEXT PRIMARY KEY, document_id TEXT NOT NULL, parent_id TEXT, name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '', detail TEXT NOT NULL DEFAULT '',
            kind TEXT NOT NULL DEFAULT 'other', color TEXT NOT NULL DEFAULT '#f59e0b',
            progress INTEGER NOT NULL DEFAULT 0, position_x REAL NOT NULL DEFAULT 0,
            position_y REAL NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            FOREIGN KEY(document_id) REFERENCES mindmap_documents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_mm_nodes_doc ON mindmap_nodes(document_id);
        CREATE TABLE IF NOT EXISTS mindmap_stickers (
            id TEXT PRIMARY KEY, document_id TEXT NOT NULL, content TEXT NOT NULL DEFAULT '',
            image_data TEXT NOT NULL DEFAULT '', rotation REAL, color TEXT NOT NULL DEFAULT '#fef3c7', position_x REAL NOT NULL DEFAULT 0,
            position_y REAL NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            FOREIGN KEY(document_id) REFERENCES mindmap_documents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_mm_stickers_doc ON mindmap_stickers(document_id);
        CREATE TABLE IF NOT EXISTS mindmap_links (
            id TEXT PRIMARY KEY, document_id TEXT NOT NULL,
            source_id TEXT NOT NULL, target_id TEXT NOT NULL,
            label TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            FOREIGN KEY(document_id) REFERENCES mindmap_documents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_mm_links_doc ON mindmap_links(document_id);
    "#).map_err(|e| format!("初始化思维导图表失败: {}", e))?;

    // 旧版本贴纸表没有 image_data，启动时幂等补列，保留已有文字贴纸。
    let sticker_cols: Vec<String> = conn.prepare("PRAGMA table_info(mindmap_stickers)")
        .and_then(|mut stmt| stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>())
        .unwrap_or_default();
    if !sticker_cols.iter().any(|c| c == "image_data") {
        conn.execute_batch("ALTER TABLE mindmap_stickers ADD COLUMN image_data TEXT NOT NULL DEFAULT ''")
            .map_err(|e| format!("迁移 image_data 失败: {}", e))?;
    }
    if !sticker_cols.iter().any(|c| c == "rotation") {
        conn.execute_batch("ALTER TABLE mindmap_stickers ADD COLUMN rotation REAL")
            .map_err(|e| format!("迁移 rotation 失败: {}", e))?;
    }

    let doc_cols: Vec<String> = conn.prepare("PRAGMA table_info(mindmap_documents)")
        .and_then(|mut stmt| stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>())
        .unwrap_or_default();
    if !doc_cols.iter().any(|c| c == "background_texture") {
        conn.execute_batch("ALTER TABLE mindmap_documents ADD COLUMN background_texture TEXT NOT NULL DEFAULT 'dots'")
            .map_err(|e| format!("迁移 background_texture 失败: {}", e))?;
    }
    if !doc_cols.iter().any(|c| c == "layout_dir") {
        conn.execute_batch("ALTER TABLE mindmap_documents ADD COLUMN layout_dir TEXT NOT NULL DEFAULT 'lr'")
            .map_err(|e| format!("迁移 layout_dir 失败: {}", e))?;
    }
    if !doc_cols.iter().any(|c| c == "folder_id") {
        conn.execute_batch("ALTER TABLE mindmap_documents ADD COLUMN folder_id TEXT")
            .map_err(|e| format!("迁移 folder_id 失败: {}", e))?;
    }
    // AI 用量留痕：累计导入次数 / 输入 / 输出 token（幂等补列）
    if !doc_cols.iter().any(|c| c == "ai_imports") {
        conn.execute_batch("ALTER TABLE mindmap_documents ADD COLUMN ai_imports INTEGER NOT NULL DEFAULT 0")
            .map_err(|e| format!("迁移 ai_imports 失败: {}", e))?;
    }
    if !doc_cols.iter().any(|c| c == "ai_input_tokens") {
        conn.execute_batch("ALTER TABLE mindmap_documents ADD COLUMN ai_input_tokens INTEGER NOT NULL DEFAULT 0")
            .map_err(|e| format!("迁移 ai_input_tokens 失败: {}", e))?;
    }
    if !doc_cols.iter().any(|c| c == "ai_output_tokens") {
        conn.execute_batch("ALTER TABLE mindmap_documents ADD COLUMN ai_output_tokens INTEGER NOT NULL DEFAULT 0")
            .map_err(|e| format!("迁移 ai_output_tokens 失败: {}", e))?;
    }

    // 旧版本文件夹表没有 parent_id（扁平），启动时幂等补列以支持层级整理。
    let folder_cols: Vec<String> = conn.prepare("PRAGMA table_info(mindmap_folders)")
        .and_then(|mut stmt| stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>())
        .unwrap_or_default();
    if !folder_cols.iter().any(|c| c == "parent_id") {
        conn.execute_batch("ALTER TABLE mindmap_folders ADD COLUMN parent_id TEXT")
            .map_err(|e| format!("迁移 parent_id 失败: {}", e))?;
    }

    // 旧版本节点表没有 plan_at（计划时间）/ repeat（重复），幂等补列。
    let node_cols: Vec<String> = conn.prepare("PRAGMA table_info(mindmap_nodes)")
        .and_then(|mut stmt| stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>())
        .unwrap_or_default();
    if !node_cols.iter().any(|c| c == "plan_at") {
        conn.execute_batch("ALTER TABLE mindmap_nodes ADD COLUMN plan_at TEXT")
            .map_err(|e| format!("迁移 plan_at 失败: {}", e))?;
    }
    if !node_cols.iter().any(|c| c == "repeat") {
        conn.execute_batch("ALTER TABLE mindmap_nodes ADD COLUMN repeat TEXT NOT NULL DEFAULT 'none'")
            .map_err(|e| format!("迁移 repeat 失败: {}", e))?;
    }
    // 证据锚定：sources 列存 JSON 数组（项目相对路径）
    if !node_cols.iter().any(|c| c == "sources") {
        conn.execute_batch("ALTER TABLE mindmap_nodes ADD COLUMN sources TEXT NOT NULL DEFAULT '[]'")
            .map_err(|e| format!("迁移 sources 失败: {}", e))?;
    }

    Ok(conn)
}

/// 初始化数据库（幂等）。
pub fn init_db() -> Result<(), String> {
    let conn = build_connection()?;
    *DB_CONN.lock().map_err(|e| e.to_string())? = Some(conn);
    Ok(())
}

pub fn with_conn<T>(f: impl FnOnce(&mut rusqlite::Connection) -> Result<T, String>) -> Result<T, String> {
    // 检查 + 初始化 + 使用放在同一锁临界区，避免并发重复初始化覆盖连接。
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁: {}", e))?;
    if guard.is_none() {
        *guard = Some(build_connection()?);
    }
    f(guard.as_mut().ok_or("数据库未初始化")?)
}

pub fn now_ts() -> String { chrono::Utc::now().to_rfc3339() }
pub fn new_id(prefix: &str) -> String { format!("{}_{}_{}", prefix, now_ts().replace(&['-',':','.'][..],""), std::process::id()) }
pub fn sql<T>(r: rusqlite::Result<T>) -> Result<T, String> { r.map_err(|e| e.to_string()) }

// ─── 文件夹 ───

pub fn list_folders() -> Result<Vec<MindmapFolder>, String> {
    with_conn(|c| {
        let mut s = c.prepare("SELECT f.id,f.name,f.sort_order,f.parent_id,f.created_at,f.updated_at,(SELECT COUNT(*) FROM mindmap_documents WHERE folder_id=f.id) FROM mindmap_folders f ORDER BY f.sort_order").map_err(|e| e.to_string())?;
        let rows = s.query_map([], |r| Ok(MindmapFolder { id: r.get(0)?, name: r.get(1)?, sort_order: r.get(2)?, parent_id: r.get(3)?, created_at: r.get(4)?, updated_at: r.get(5)?, document_count: r.get(6)? })).map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    })
}

pub fn create_folder(name: &str, parent_id: Option<&str>) -> Result<MindmapFolder, String> {
    with_conn(|c| {
        let id = new_id("mf"); let ts = now_ts();
        let next: i64 = c.query_row("SELECT COALESCE(MAX(sort_order),-1)+1 FROM mindmap_folders", [], |r| r.get(0)).unwrap_or(0);
        sql(c.execute("INSERT INTO mindmap_folders (id,name,sort_order,parent_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6)", rusqlite::params![id, name, next, parent_id, ts, ts]))?;
        Ok(MindmapFolder { id, name: name.to_string(), sort_order: next, document_count: 0, parent_id: parent_id.map(|s| s.to_string()), created_at: ts.clone(), updated_at: ts })
    })
}

/// 判断 possible_parent_id 是否位于 id（含 id 自身）的父链上。用于防环。
fn has_cycle(c: &rusqlite::Connection, folder_id: &str, new_parent: Option<&str>) -> Result<bool, String> {
    let Some(nw) = new_parent else { return Ok(false) };
    if nw == folder_id { return Ok(true); }
    let mut cur = Some(nw.to_string());
    let mut guard = 0;
    while let Some(pid) = cur {
        guard += 1;
        if guard > 1000 { return Ok(true); } // 数据异常：链过长视为环
        if pid == folder_id { return Ok(true); }
        cur = c.query_row("SELECT parent_id FROM mindmap_folders WHERE id=?1", rusqlite::params![pid], |r| r.get::<_, Option<String>>(0)).unwrap_or(None);
    }
    Ok(false)
}

/// 移动文件夹到另一文件夹下；parent_id 为 None 表示移到根目录。
/// 拒绝移入自身或其后代（防环）。
pub fn move_folder(folder_id: &str, parent_id: Option<&str>) -> Result<(), String> {
    with_conn(|c| {
        if has_cycle(c, folder_id, parent_id)? {
            return Err("不能把文件夹移动到自身或其子目录中".into());
        }
        sql(c.execute("UPDATE mindmap_folders SET parent_id=?1,updated_at=?2 WHERE id=?3", rusqlite::params![parent_id, now_ts(), folder_id]))?;
        Ok(())
    })
}

pub fn update_folder(id: &str, name: Option<&str>) -> Result<(), String> {
    with_conn(|c| {
        if let Some(n) = name { sql(c.execute("UPDATE mindmap_folders SET name=?1,updated_at=?2 WHERE id=?3", rusqlite::params![n, now_ts(), id]))?; }
        Ok(())
    })
}

pub fn delete_folder(id: &str) -> Result<(), String> {
    with_conn(|c| {
        // 收集 id 及其所有子孙文件夹，统一删除；文档一律移回根目录
        let mut ids = vec![id.to_string()]; let mut i = 0;
        while i < ids.len() {
            let pid = &ids[i];
            let mut s = c.prepare("SELECT id FROM mindmap_folders WHERE parent_id=?1").map_err(|e| e.to_string())?;
            let rows = s.query_map(rusqlite::params![pid], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
            ids.extend(rows.filter_map(|x| x.ok()));
            i += 1;
        }
        for fid in &ids {
            sql(c.execute("UPDATE mindmap_documents SET folder_id=NULL WHERE folder_id=?1", rusqlite::params![fid]))?;
            sql(c.execute("DELETE FROM mindmap_folders WHERE id=?1", rusqlite::params![fid]))?;
        }
        Ok(())
    })
}

// ─── 文档 ───

pub fn list_documents(folder_id: Option<&str>) -> Result<Vec<MindmapDocument>, String> {
    with_conn(|c| {
        let sql_str = "SELECT d.id,d.name,d.description,d.source_type,d.source_desc,d.folder_id,d.background_texture,d.layout_dir,d.created_at,d.updated_at,(SELECT COUNT(*) FROM mindmap_nodes WHERE document_id=d.id),(SELECT COUNT(*) FROM mindmap_stickers WHERE document_id=d.id),d.ai_imports,d.ai_input_tokens,d.ai_output_tokens FROM mindmap_documents d WHERE (?1 IS NULL AND d.folder_id IS NULL) OR d.folder_id=?1 ORDER BY d.updated_at DESC";
        let mut s = c.prepare(sql_str).map_err(|e| e.to_string())?;
        let mapped = s.query_map(rusqlite::params![folder_id], |r| Ok(MindmapDocument {
            id: r.get(0)?, name: r.get(1)?, description: r.get(2)?, source_type: r.get(3)?,
            source_desc: r.get(4)?, folder_id: r.get(5)?, background_texture: r.get(6)?, layout_dir: r.get(7)?,
            node_count: r.get(10)?, sticker_count: r.get(11)?, ai_imports: r.get(12)?, ai_input_tokens: r.get(13)?, ai_output_tokens: r.get(14)?,
            created_at: r.get(8)?, updated_at: r.get(9)?,
        })).map_err(|e| e.to_string())?;
        mapped.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    })
}

pub fn create_document(name: &str, description: &str, source_type: &str, folder_id: Option<&str>) -> Result<MindmapDocument, String> {
    with_conn(|c| {
        let id = new_id("mm"); let ts = now_ts();
        sql(c.execute("INSERT INTO mindmap_documents (id,name,description,source_type,source_desc,folder_id,background_texture,layout_dir,ai_imports,ai_input_tokens,ai_output_tokens,created_at,updated_at) VALUES (?1,?2,?3,?4,'',?5,'dots','lr',0,0,0,?6,?7)", rusqlite::params![id, name, description, source_type, folder_id, ts, ts]))?;
        let root_id = new_id("nd");
        sql(c.execute("INSERT INTO mindmap_nodes (id,document_id,parent_id,name,description,detail,kind,color,progress,plan_at,repeat,sources,position_x,position_y,created_at,updated_at) VALUES (?1,?2,NULL,?3,'根节点','','root','#f8fafc',0,NULL,'none','[]',0,0,?4,?5)", rusqlite::params![root_id, id, name, ts, ts]))?;
        Ok(MindmapDocument { id, name: name.to_string(), description: description.to_string(), source_type: source_type.to_string(), source_desc: String::new(), folder_id: folder_id.map(|s| s.to_string()), background_texture: "dots".to_string(), layout_dir: "lr".to_string(), node_count: 1, sticker_count: 0, ai_imports: 0, ai_input_tokens: 0, ai_output_tokens: 0, created_at: ts.clone(), updated_at: ts })
    })
}

/// 记录一次 AI 导入的 token 消耗：导入次数 +1，输入/输出 token 累加（留痕）。
pub fn add_ai_usage(document_id: &str, input_tokens: u64, output_tokens: u64) -> Result<(), String> {
    with_conn(|c| {
        sql(c.execute(
            "UPDATE mindmap_documents SET ai_imports=ai_imports+1, ai_input_tokens=ai_input_tokens+?1, ai_output_tokens=ai_output_tokens+?2, updated_at=?3 WHERE id=?4",
            rusqlite::params![input_tokens as i64, output_tokens as i64, now_ts(), document_id],
        ))?;
        Ok(())
    })
}

pub fn move_document(document_id: &str, folder_id: Option<&str>) -> Result<(), String> {
    with_conn(|c| {
        sql(c.execute("UPDATE mindmap_documents SET folder_id=?1,updated_at=?2 WHERE id=?3", rusqlite::params![folder_id, now_ts(), document_id]))?;
        Ok(())
    })
}

pub fn update_background_texture(id: &str, texture: &str) -> Result<(), String> {
    const ALLOWED: [&str; 6] = ["none", "grid", "dots", "diagonal", "cross", "paper"];
    let value = if ALLOWED.contains(&texture) { texture } else { "dots" };
    with_conn(|c| {
        sql(c.execute("UPDATE mindmap_documents SET background_texture=?1,updated_at=?2 WHERE id=?3", rusqlite::params![value, now_ts(), id]))?;
        Ok(())
    })
}

pub fn update_layout_dir(id: &str, dir: &str) -> Result<(), String> {
    const ALLOWED: [&str; 4] = ["lr", "rl", "tb", "bt"];
    let value = if ALLOWED.contains(&dir) { dir } else { "lr" };
    with_conn(|c| {
        sql(c.execute("UPDATE mindmap_documents SET layout_dir=?1,updated_at=?2 WHERE id=?3", rusqlite::params![value, now_ts(), id]))?;
        Ok(())
    })
}

pub fn update_document(id: &str, name: Option<&str>, description: Option<&str>, folder_id: Option<Option<&str>>) -> Result<(), String> {
    with_conn(|c| {
        let ts = now_ts();
        if let Some(n) = name { sql(c.execute("UPDATE mindmap_documents SET name=?1,updated_at=?2 WHERE id=?3", rusqlite::params![n, ts, id]))?; }
        if let Some(d) = description { sql(c.execute("UPDATE mindmap_documents SET description=?1,updated_at=?2 WHERE id=?3", rusqlite::params![d, ts, id]))?; }
        if let Some(fid) = folder_id { sql(c.execute("UPDATE mindmap_documents SET folder_id=?1,updated_at=?2 WHERE id=?3", rusqlite::params![fid, ts, id]))?; }
        Ok(())
    })
}

pub fn delete_document(id: &str) -> Result<(), String> {
    with_conn(|c| { sql(c.execute("DELETE FROM mindmap_documents WHERE id=?1", rusqlite::params![id]))?; Ok(()) })
}

fn touch_document_inner(c: &rusqlite::Connection, id: &str) -> Result<(), String> {
    sql(c.execute("UPDATE mindmap_documents SET updated_at=?1 WHERE id=?2", rusqlite::params![now_ts(), id]))?;
    Ok(())
}

pub fn touch_document(id: &str) -> Result<(), String> {
    with_conn(|c| touch_document_inner(c, id))
}

/// 记录文档来源（AI 项目导入时存项目根路径，供证据文件定位）。
pub fn update_source_desc(id: &str, desc: &str) -> Result<(), String> {
    with_conn(|c| {
        sql(c.execute(
            "UPDATE mindmap_documents SET source_desc=?1, updated_at=?2 WHERE id=?3",
            rusqlite::params![desc, now_ts(), id],
        ))?;
        Ok(())
    })
}

// ─── 节点 ───

fn row_to_node(r: &rusqlite::Row) -> rusqlite::Result<MindmapNode> {
    let sources: Vec<String> = r
        .get::<_, Option<String>>(15)?
        .map(|s| serde_json::from_str(&s).unwrap_or_default())
        .unwrap_or_default();
    Ok(MindmapNode { id: r.get(0)?, document_id: r.get(1)?, parent_id: r.get(2)?, name: r.get(3)?, description: r.get(4)?, detail: r.get(5)?, kind: r.get(6)?, color: r.get(7)?, progress: r.get(8)?, plan_at: r.get(9)?, position_x: r.get(10)?, position_y: r.get(11)?, created_at: r.get(12)?, updated_at: r.get(13)?, repeat: r.get::<_, Option<String>>(14)?.unwrap_or_default(), sources })
}

fn list_nodes_inner(c: &rusqlite::Connection, document_id: &str) -> Result<Vec<MindmapNode>, String> {
    let mut s = c.prepare("SELECT id,document_id,parent_id,name,description,detail,kind,color,progress,plan_at,position_x,position_y,created_at,updated_at,repeat,sources FROM mindmap_nodes WHERE document_id=?1").map_err(|e| e.to_string())?;
    let rows = s.query_map(rusqlite::params![document_id], |r| row_to_node(r)).map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

pub fn list_nodes(document_id: &str) -> Result<Vec<MindmapNode>, String> {
    with_conn(|c| list_nodes_inner(c, document_id))
}

/// 指定日期范围 [start, end]（YYYY-MM-DD，含端点）内所有具体计划发生记录。
/// 重复计划（daily / weekly）在 SQL 中用递归 CTE 展开为范围内的逐次发生，
/// 并按本地时区归日（plan_at 存的是 UTC，前端按本地日期查看计划）。
pub fn list_planned_occurrences(start: &str, end: &str) -> Result<Vec<PlannedOccurrence>, String> {
    // 防御：非法或超长范围直接拒绝，避免递归 CTE 失控（正常日历视图至多一个多月）。
    let s = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .map_err(|_| format!("无效的开始日期: {}", start))?;
    let e = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .map_err(|_| format!("无效的结束日期: {}", end))?;
    let (s, e) = if s > e { (e, s) } else { (s, e) };
    if (e - s).num_days() > 730 {
        return Err("日期范围过大（最多 730 天）".into());
    }
    let s_str = s.format("%Y-%m-%d").to_string();
    let e_str = e.format("%Y-%m-%d").to_string();
    with_conn(|c| {
        let mut stmt = c
            .prepare(
                r#"WITH RECURSIVE
                plans AS (
                    SELECT n.id, n.document_id, d.name AS doc_name, n.name, n.kind, n.color, n.plan_at, n.repeat,
                           -- 本地化：完整时间戳转本地时间（去毫秒/时区后缀，SQLite 把无时区视为 UTC）；纯日期原样保留
                           CASE WHEN instr(n.plan_at, 'T') > 0 OR instr(n.plan_at, ' ') > 0
                                THEN datetime(substr(n.plan_at, 1, 19), 'localtime')
                                ELSE datetime(substr(n.plan_at, 1, 10)) END AS p_local,
                           CASE WHEN instr(n.plan_at, 'T') > 0 OR instr(n.plan_at, ' ') > 0
                                THEN date(datetime(substr(n.plan_at, 1, 19), 'localtime'))
                                ELSE date(substr(n.plan_at, 1, 10)) END AS p_date
                    FROM mindmap_nodes n
                    JOIN mindmap_documents d ON d.id = n.document_id
                    WHERE n.plan_at IS NOT NULL AND n.plan_at != ''
                ),
                days(day) AS (
                    SELECT date(?1)
                    UNION ALL
                    SELECT date(day, '+1 day') FROM days WHERE day < date(?2)
                )
                SELECT p.id, p.document_id, p.doc_name, p.name, p.kind, p.color, p.plan_at, p.repeat,
                       d.day AS occur_day,
                       -- 具体发生时间：原计划的钟点（本地）落在展开日当天
                       d.day || 'T' || substr(p.p_local, 12) AS occur_local
                FROM plans p JOIN days d
                WHERE p.p_date <= d.day
                  AND (p.repeat = 'daily'
                       OR (p.repeat = 'weekly' AND CAST(strftime('%w', p.p_date) AS INTEGER) = CAST(strftime('%w', d.day) AS INTEGER))
                       OR (p.repeat NOT IN ('daily', 'weekly') AND p.p_date = d.day))
                ORDER BY d.day, p.p_local"#,
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![s_str, e_str], |r| {
                Ok(PlannedOccurrence {
                    id: r.get(0)?,
                    document_id: r.get(1)?,
                    document_name: r.get(2)?,
                    name: r.get(3)?,
                    kind: r.get(4)?,
                    color: r.get(5)?,
                    plan_at: r.get(6)?,
                    repeat: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                    occur_day: r.get(8)?,
                    occur_at: r.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())
    })
}

/// 按拖拽天数差改写 plan_at：完整时间戳保留本地钟点、只平移本地日期（再转回 UTC 存储）；
/// 纯日期（YYYY-MM-DD）直接平移日期。
fn shifted_plan_at(plan_at: &str, from_day: &str, to_day: &str) -> Result<String, String> {
    use chrono::{Duration, NaiveDate, TimeZone};
    let f = NaiveDate::parse_from_str(from_day, "%Y-%m-%d")
        .map_err(|_| format!("无效日期: {}", from_day))?;
    let t = NaiveDate::parse_from_str(to_day, "%Y-%m-%d")
        .map_err(|_| format!("无效日期: {}", to_day))?;
    let delta = (t - f).num_days();
    if delta == 0 {
        return Ok(plan_at.to_string());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(plan_at) {
        let local = dt.with_timezone(&chrono::Local);
        // 直接平移日期部分、保留钟点，避免按 24h 加跨 DST 时钟点漂移
        let new_date = local.date_naive() + Duration::days(delta);
        let shifted = chrono::Local
            .from_local_datetime(&chrono::NaiveDateTime::new(new_date, local.time()))
            .single()
            .unwrap_or_else(|| local + Duration::days(delta));
        return Ok(shifted.with_timezone(&chrono::Utc).to_rfc3339());
    }
    if let Ok(nd) = NaiveDate::parse_from_str(plan_at, "%Y-%m-%d") {
        return Ok((nd + Duration::days(delta)).format("%Y-%m-%d").to_string());
    }
    Err(format!("无法解析计划时间: {}", plan_at))
}

/// 拖拽移动某次计划发生：按 from_day → to_day 的天数差改写节点 plan_at。
/// 不重复计划即单次移动；daily/weekly 整条按相同天数顺延（保留钟点与重复规则）。
pub fn move_plan_occurrence(node_id: &str, from_day: &str, to_day: &str) -> Result<(), String> {
    use rusqlite::OptionalExtension;
    with_conn(|c| {
        let row = c
            .query_row(
                "SELECT document_id, plan_at FROM mindmap_nodes WHERE id=?1",
                rusqlite::params![node_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("节点不存在")?;
        let (document_id, plan_at) = row;
        let plan_at = plan_at.ok_or("该节点没有计划时间")?;
        let new_plan_at = shifted_plan_at(&plan_at, from_day, to_day)?;
        sql(c.execute(
            "UPDATE mindmap_nodes SET plan_at=?1, updated_at=?2 WHERE id=?3",
            rusqlite::params![new_plan_at, now_ts(), node_id],
        ))?;
        touch_document_inner(c, &document_id)?;
        Ok(())
    })
}

fn upsert_node_inner(c: &rusqlite::Connection, node: &MindmapNode) -> Result<(), String> {
    let ts = now_ts();
    let exists: i64 = c.query_row("SELECT COUNT(*) FROM mindmap_nodes WHERE id=?1", rusqlite::params![node.id], |r| r.get(0)).unwrap_or(0);
    if exists > 0 {
        sql(c.execute("UPDATE mindmap_nodes SET parent_id=?1,name=?2,description=?3,detail=?4,kind=?5,color=?6,progress=?7,plan_at=?8,repeat=?9,sources=?10,position_x=?11,position_y=?12,updated_at=?13 WHERE id=?14", rusqlite::params![node.parent_id, node.name, node.description, node.detail, node.kind, node.color, node.progress, node.plan_at, node.repeat, serde_json::to_string(&node.sources).unwrap_or_else(|_| "[]".into()), node.position_x, node.position_y, ts, node.id]))?;
    } else {
        sql(c.execute("INSERT INTO mindmap_nodes (id,document_id,parent_id,name,description,detail,kind,color,progress,plan_at,repeat,sources,position_x,position_y,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)", rusqlite::params![node.id, node.document_id, node.parent_id, node.name, node.description, node.detail, node.kind, node.color, node.progress, node.plan_at, node.repeat, serde_json::to_string(&node.sources).unwrap_or_else(|_| "[]".into()), node.position_x, node.position_y, ts, ts]))?;
    }
    touch_document_inner(c, &node.document_id)?;
    Ok(())
}

pub fn upsert_node(node: &MindmapNode) -> Result<(), String> {
    with_conn(|c| upsert_node_inner(c, node))
}

pub fn delete_node(document_id: &str, node_id: &str) -> Result<(), String> {
    with_conn(|c| {
        let mut ids = vec![node_id.to_string()]; let mut i = 0;
        while i < ids.len() {
            let pid = &ids[i];
            let mut s = c.prepare("SELECT id FROM mindmap_nodes WHERE parent_id=?1").map_err(|e| e.to_string())?;
            let rows = s.query_map(rusqlite::params![pid], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
            ids.extend(rows.filter_map(|x| x.ok()));
            i += 1;
        }
        for id in &ids {
            sql(c.execute("DELETE FROM mindmap_nodes WHERE id=?1", rusqlite::params![id]))?;
            // 连带删除与这些节点相关的额外连线（作为来源或目标）
            sql(c.execute("DELETE FROM mindmap_links WHERE source_id=?1 OR target_id=?1", rusqlite::params![id]))?;
        }
        touch_document_inner(c, document_id)?;
        Ok(())
    })
}

/// 批量保存节点坐标/内容：整批包在单个事务里，中途失败整体回滚，
/// 避免拖拽保存时部分节点落盘、部分未写导致坐标不一致。
pub fn batch_save_nodes(nodes: &[MindmapNode]) -> Result<(), String> {
    with_conn(|c| {
        let tx = c.transaction().map_err(|e| format!("开启事务失败: {}", e))?;
        for n in nodes {
            upsert_node_inner(&tx, n)?;
        }
        tx.commit().map_err(|e| format!("提交事务失败: {}", e))?;
        Ok(())
    })
}

// ─── 贴纸 ───

fn row_to_sticker(r: &rusqlite::Row) -> rusqlite::Result<MindmapSticker> {
    Ok(MindmapSticker { id: r.get(0)?, document_id: r.get(1)?, content: r.get(2)?, image_data: r.get(3)?, rotation: r.get(4)?, color: r.get(5)?, position_x: r.get(6)?, position_y: r.get(7)?, created_at: r.get(8)?, updated_at: r.get(9)? })
}

fn list_stickers_inner(c: &rusqlite::Connection, document_id: &str) -> Result<Vec<MindmapSticker>, String> {
    let mut s = c.prepare("SELECT id,document_id,content,image_data,rotation,color,position_x,position_y,created_at,updated_at FROM mindmap_stickers WHERE document_id=?1").map_err(|e| e.to_string())?;
    let rows = s.query_map(rusqlite::params![document_id], |r| row_to_sticker(r)).map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

pub fn list_stickers(document_id: &str) -> Result<Vec<MindmapSticker>, String> {
    with_conn(|c| list_stickers_inner(c, document_id))
}

pub fn upsert_sticker(s: &MindmapSticker) -> Result<(), String> {
    with_conn(|c| {
        let ts = now_ts();
        let exists: i64 = c.query_row("SELECT COUNT(*) FROM mindmap_stickers WHERE id=?1", rusqlite::params![s.id], |r| r.get(0)).unwrap_or(0);
        if exists > 0 {
            sql(c.execute("UPDATE mindmap_stickers SET content=?1,image_data=?2,rotation=?3,color=?4,position_x=?5,position_y=?6,updated_at=?7 WHERE id=?8", rusqlite::params![s.content, s.image_data, s.rotation, s.color, s.position_x, s.position_y, ts, s.id]))?;
        } else {
            sql(c.execute("INSERT INTO mindmap_stickers (id,document_id,content,image_data,rotation,color,position_x,position_y,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", rusqlite::params![s.id, s.document_id, s.content, s.image_data, s.rotation, s.color, s.position_x, s.position_y, ts, ts]))?;
        }
        touch_document_inner(c, &s.document_id)?;
        Ok(())
    })
}

pub fn delete_sticker(document_id: &str, sticker_id: &str) -> Result<(), String> {
    with_conn(|c| { sql(c.execute("DELETE FROM mindmap_stickers WHERE id=?1", rusqlite::params![sticker_id]))?; touch_document_inner(c, document_id)?; Ok(()) })
}

// ─── 额外连线（多输入 DAG） ───

fn list_links_inner(c: &rusqlite::Connection, document_id: &str) -> Result<Vec<MindmapLink>, String> {
    let mut s = c.prepare("SELECT id,document_id,source_id,target_id,label,created_at,updated_at FROM mindmap_links WHERE document_id=?1").map_err(|e| e.to_string())?;
    let rows = s.query_map(rusqlite::params![document_id], |r| Ok(MindmapLink { id: r.get(0)?, document_id: r.get(1)?, source_id: r.get(2)?, target_id: r.get(3)?, label: r.get(4)?, created_at: r.get(5)?, updated_at: r.get(6)? })).map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

pub fn list_links(document_id: &str) -> Result<Vec<MindmapLink>, String> {
    with_conn(|c| list_links_inner(c, document_id))
}

pub fn upsert_link(l: &MindmapLink) -> Result<(), String> {
    with_conn(|c| {
        let ts = now_ts();
        let exists: i64 = c.query_row("SELECT COUNT(*) FROM mindmap_links WHERE id=?1", rusqlite::params![l.id], |r| r.get(0)).unwrap_or(0);
        if exists > 0 {
            sql(c.execute("UPDATE mindmap_links SET source_id=?1,target_id=?2,label=?3,updated_at=?4 WHERE id=?5", rusqlite::params![l.source_id, l.target_id, l.label, ts, l.id]))?;
        } else {
            sql(c.execute("INSERT INTO mindmap_links (id,document_id,source_id,target_id,label,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", rusqlite::params![l.id, l.document_id, l.source_id, l.target_id, l.label, ts, ts]))?;
        }
        touch_document_inner(c, &l.document_id)?;
        Ok(())
    })
}

pub fn delete_link(document_id: &str, link_id: &str) -> Result<(), String> {
    with_conn(|c| {
        sql(c.execute("DELETE FROM mindmap_links WHERE id=?1", rusqlite::params![link_id]))?;
        touch_document_inner(c, document_id)?;
        Ok(())
    })
}

pub fn load_full(document_id: &str) -> Result<Option<DocumentFull>, String> {
    with_conn(|c| {
        let mut s = c.prepare("SELECT id,name,description,source_type,source_desc,folder_id,background_texture,layout_dir,created_at,updated_at,ai_imports,ai_input_tokens,ai_output_tokens FROM mindmap_documents WHERE id=?1").map_err(|e| e.to_string())?;
        let mut rows = s.query_map(rusqlite::params![document_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?,r.get::<_,i64>(10)?,r.get::<_,i64>(11)?,r.get::<_,i64>(12)?))).map_err(|e| e.to_string())?;
        if let Some(Ok((id,name,desc,st,sd,fid,bt,ld,ca,ua,aii,ait,aot))) = rows.next() {
            let n = list_nodes_inner(c, &id)?; let sc = list_stickers_inner(c, &id)?;
            let lk = list_links_inner(c, &id)?;
            Ok(Some(DocumentFull { document: MindmapDocument { id, name, description: desc, source_type: st, source_desc: sd, folder_id: fid, background_texture: bt, layout_dir: ld, node_count: n.len(), sticker_count: sc.len(), ai_imports: aii, ai_input_tokens: ait, ai_output_tokens: aot, created_at: ca, updated_at: ua }, nodes: n, stickers: sc, links: lk }))
        } else { Ok(None) }
    })
}