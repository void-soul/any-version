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
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS mindmap_documents (
            id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
            source_type TEXT NOT NULL DEFAULT 'manual', source_desc TEXT NOT NULL DEFAULT '',
            folder_id TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
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
            color TEXT NOT NULL DEFAULT '#fef3c7', position_x REAL NOT NULL DEFAULT 0,
            position_y REAL NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
            FOREIGN KEY(document_id) REFERENCES mindmap_documents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_mm_stickers_doc ON mindmap_stickers(document_id);
    "#).map_err(|e| format!("初始化思维导图表失败: {}", e))?;

    let doc_cols: Vec<String> = conn.prepare("PRAGMA table_info(mindmap_documents)")
        .and_then(|mut stmt| stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>())
        .unwrap_or_default();
    if !doc_cols.iter().any(|c| c == "folder_id") {
        conn.execute_batch("ALTER TABLE mindmap_documents ADD COLUMN folder_id TEXT")
            .map_err(|e| format!("迁移 folder_id 失败: {}", e))?;
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
        let mut s = c.prepare("SELECT f.id,f.name,f.sort_order,f.created_at,f.updated_at,(SELECT COUNT(*) FROM mindmap_documents WHERE folder_id=f.id) FROM mindmap_folders f ORDER BY f.sort_order").map_err(|e| e.to_string())?;
        let rows = s.query_map([], |r| Ok(MindmapFolder { id: r.get(0)?, name: r.get(1)?, sort_order: r.get(2)?, document_count: r.get(5)?, created_at: r.get(3)?, updated_at: r.get(4)? })).map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    })
}

pub fn create_folder(name: &str) -> Result<MindmapFolder, String> {
    with_conn(|c| {
        let id = new_id("mf"); let ts = now_ts();
        let next: i64 = c.query_row("SELECT COALESCE(MAX(sort_order),-1)+1 FROM mindmap_folders", [], |r| r.get(0)).unwrap_or(0);
        sql(c.execute("INSERT INTO mindmap_folders (id,name,sort_order,created_at,updated_at) VALUES (?1,?2,?3,?4,?5)", rusqlite::params![id, name, next, ts, ts]))?;
        Ok(MindmapFolder { id, name: name.to_string(), sort_order: next, document_count: 0, created_at: ts.clone(), updated_at: ts })
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
        sql(c.execute("UPDATE mindmap_documents SET folder_id=NULL WHERE folder_id=?1", rusqlite::params![id]))?;
        sql(c.execute("DELETE FROM mindmap_folders WHERE id=?1", rusqlite::params![id]))?;
        Ok(())
    })
}

// ─── 文档 ───

pub fn list_documents(folder_id: Option<&str>) -> Result<Vec<MindmapDocument>, String> {
    with_conn(|c| {
        let sql_str = "SELECT d.id,d.name,d.description,d.source_type,d.source_desc,d.folder_id,d.created_at,d.updated_at,(SELECT COUNT(*) FROM mindmap_nodes WHERE document_id=d.id),(SELECT COUNT(*) FROM mindmap_stickers WHERE document_id=d.id) FROM mindmap_documents d WHERE (?1 IS NULL OR d.folder_id=?1) ORDER BY d.updated_at DESC";
        let mut s = c.prepare(sql_str).map_err(|e| e.to_string())?;
        let mapped = s.query_map(rusqlite::params![folder_id], |r| Ok(MindmapDocument {
            id: r.get(0)?, name: r.get(1)?, description: r.get(2)?, source_type: r.get(3)?,
            source_desc: r.get(4)?, folder_id: r.get(5)?,
            node_count: r.get(8)?, sticker_count: r.get(9)?,
            created_at: r.get(6)?, updated_at: r.get(7)?,
        })).map_err(|e| e.to_string())?;
        mapped.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    })
}

pub fn create_document(name: &str, description: &str, source_type: &str, folder_id: Option<&str>) -> Result<MindmapDocument, String> {
    with_conn(|c| {
        let id = new_id("mm"); let ts = now_ts();
        sql(c.execute("INSERT INTO mindmap_documents (id,name,description,source_type,source_desc,folder_id,created_at,updated_at) VALUES (?1,?2,?3,?4,'',?5,?6,?7)", rusqlite::params![id, name, description, source_type, folder_id, ts, ts]))?;
        let root_id = new_id("nd");
        sql(c.execute("INSERT INTO mindmap_nodes (id,document_id,parent_id,name,description,detail,kind,color,progress,position_x,position_y,created_at,updated_at) VALUES (?1,?2,NULL,?3,'根节点','','root','#f8fafc',0,0,0,?4,?5)", rusqlite::params![root_id, id, name, ts, ts]))?;
        Ok(MindmapDocument { id, name: name.to_string(), description: description.to_string(), source_type: source_type.to_string(), source_desc: String::new(), folder_id: folder_id.map(|s| s.to_string()), node_count: 1, sticker_count: 0, created_at: ts.clone(), updated_at: ts })
    })
}

pub fn move_document(document_id: &str, folder_id: Option<&str>) -> Result<(), String> {
    with_conn(|c| {
        sql(c.execute("UPDATE mindmap_documents SET folder_id=?1,updated_at=?2 WHERE id=?3", rusqlite::params![folder_id, now_ts(), document_id]))?;
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

// ─── 节点 ───

fn row_to_node(r: &rusqlite::Row) -> rusqlite::Result<MindmapNode> {
    Ok(MindmapNode { id: r.get(0)?, document_id: r.get(1)?, parent_id: r.get(2)?, name: r.get(3)?, description: r.get(4)?, detail: r.get(5)?, kind: r.get(6)?, color: r.get(7)?, progress: r.get(8)?, position_x: r.get(9)?, position_y: r.get(10)?, created_at: r.get(11)?, updated_at: r.get(12)? })
}

fn list_nodes_inner(c: &rusqlite::Connection, document_id: &str) -> Result<Vec<MindmapNode>, String> {
    let mut s = c.prepare("SELECT id,document_id,parent_id,name,description,detail,kind,color,progress,position_x,position_y,created_at,updated_at FROM mindmap_nodes WHERE document_id=?1").map_err(|e| e.to_string())?;
    let rows = s.query_map(rusqlite::params![document_id], |r| row_to_node(r)).map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

pub fn list_nodes(document_id: &str) -> Result<Vec<MindmapNode>, String> {
    with_conn(|c| list_nodes_inner(c, document_id))
}

fn upsert_node_inner(c: &rusqlite::Connection, node: &MindmapNode) -> Result<(), String> {
    let ts = now_ts();
    let exists: i64 = c.query_row("SELECT COUNT(*) FROM mindmap_nodes WHERE id=?1", rusqlite::params![node.id], |r| r.get(0)).unwrap_or(0);
    if exists > 0 {
        sql(c.execute("UPDATE mindmap_nodes SET parent_id=?1,name=?2,description=?3,detail=?4,kind=?5,color=?6,progress=?7,position_x=?8,position_y=?9,updated_at=?10 WHERE id=?11", rusqlite::params![node.parent_id, node.name, node.description, node.detail, node.kind, node.color, node.progress, node.position_x, node.position_y, ts, node.id]))?;
    } else {
        sql(c.execute("INSERT INTO mindmap_nodes (id,document_id,parent_id,name,description,detail,kind,color,progress,position_x,position_y,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)", rusqlite::params![node.id, node.document_id, node.parent_id, node.name, node.description, node.detail, node.kind, node.color, node.progress, node.position_x, node.position_y, ts, ts]))?;
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
        for id in &ids { sql(c.execute("DELETE FROM mindmap_nodes WHERE id=?1", rusqlite::params![id]))?; }
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
    Ok(MindmapSticker { id: r.get(0)?, document_id: r.get(1)?, content: r.get(2)?, color: r.get(3)?, position_x: r.get(4)?, position_y: r.get(5)?, created_at: r.get(6)?, updated_at: r.get(7)? })
}

fn list_stickers_inner(c: &rusqlite::Connection, document_id: &str) -> Result<Vec<MindmapSticker>, String> {
    let mut s = c.prepare("SELECT id,document_id,content,color,position_x,position_y,created_at,updated_at FROM mindmap_stickers WHERE document_id=?1").map_err(|e| e.to_string())?;
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
            sql(c.execute("UPDATE mindmap_stickers SET content=?1,color=?2,position_x=?3,position_y=?4,updated_at=?5 WHERE id=?6", rusqlite::params![s.content, s.color, s.position_x, s.position_y, ts, s.id]))?;
        } else {
            sql(c.execute("INSERT INTO mindmap_stickers (id,document_id,content,color,position_x,position_y,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)", rusqlite::params![s.id, s.document_id, s.content, s.color, s.position_x, s.position_y, ts, ts]))?;
        }
        touch_document_inner(c, &s.document_id)?;
        Ok(())
    })
}

pub fn delete_sticker(document_id: &str, sticker_id: &str) -> Result<(), String> {
    with_conn(|c| { sql(c.execute("DELETE FROM mindmap_stickers WHERE id=?1", rusqlite::params![sticker_id]))?; touch_document_inner(c, document_id)?; Ok(()) })
}

pub fn load_full(document_id: &str) -> Result<Option<DocumentFull>, String> {
    with_conn(|c| {
        let mut s = c.prepare("SELECT id,name,description,source_type,source_desc,folder_id,created_at,updated_at FROM mindmap_documents WHERE id=?1").map_err(|e| e.to_string())?;
        let mut rows = s.query_map(rusqlite::params![document_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?))).map_err(|e| e.to_string())?;
        if let Some(Ok((id,name,desc,st,sd,fid,ca,ua))) = rows.next() {
            let n = list_nodes_inner(c, &id)?; let sc = list_stickers_inner(c, &id)?;
            Ok(Some(DocumentFull { document: MindmapDocument { id, name, description: desc, source_type: st, source_desc: sd, folder_id: fid, node_count: n.len(), sticker_count: sc.len(), created_at: ca, updated_at: ua }, nodes: n, stickers: sc }))
        } else { Ok(None) }
    })
}