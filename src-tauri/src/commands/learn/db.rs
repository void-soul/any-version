//! SQLite 存储层：projects / modules / graphs。
//! 数据库文件：`{data_dir}/learn.db`
//! 已废弃——所有功能已迁移至 mindmap 模块。保留仅用于旧命令兼容。

use std::sync::Mutex;
use crate::commands::config::get_data_dir;
use super::models::*;

static DB_CONN: Mutex<Option<rusqlite::Connection>> = Mutex::new(None);

fn db_path() -> std::path::PathBuf { get_data_dir().join("learn.db") }

pub fn init_db() -> Result<(), String> {
    let path = db_path();
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let conn = rusqlite::Connection::open(&path).map_err(|e| format!("打开需求数据库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| format!("WAL: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON").ok();
    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS req_projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS req_modules (id TEXT PRIMARY KEY, project_id TEXT NOT NULL, name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', sort_order INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, FOREIGN KEY(project_id) REFERENCES req_projects(id) ON DELETE CASCADE);
        CREATE INDEX IF NOT EXISTS idx_req_mod_project ON req_modules(project_id);
        CREATE TABLE IF NOT EXISTS req_graphs (module_id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', source_type TEXT NOT NULL DEFAULT 'manual', source_desc TEXT NOT NULL DEFAULT '', graph_json TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL, updated_at TEXT NOT NULL, FOREIGN KEY(module_id) REFERENCES req_modules(id) ON DELETE CASCADE);
    "#).map_err(|e| format!("初始化需求表失败: {}", e))?;
    let cols: Vec<String> = conn.prepare("PRAGMA table_info(req_graphs)").and_then(|mut stmt| stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>()).unwrap_or_default();
    for (col, def) in [("source_type","TEXT NOT NULL DEFAULT 'manual'"),("source_desc","TEXT NOT NULL DEFAULT ''"),("name","TEXT NOT NULL DEFAULT ''")] {
        if !cols.iter().any(|c| c == col) { conn.execute_batch(&format!("ALTER TABLE req_graphs ADD COLUMN {} {}", col, def)).map_err(|e| format!("迁移 req_graphs.{} 失败: {}", col, e))?; }
    }
    *DB_CONN.lock().map_err(|e| e.to_string())? = Some(conn);
    Ok(())
}

fn ensure_db() -> Result<(), String> {
    if DB_CONN.lock().map_err(|e| e.to_string())?.is_some() { return Ok(()); }
    init_db()
}

pub fn with_conn<T, F>(f: F) -> Result<T, String> where F: FnOnce(&mut rusqlite::Connection) -> Result<T, String> {
    ensure_db()?;
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁: {}", e))?;
    f(guard.as_mut().ok_or("数据库未初始化")?)
}

pub fn sql<T>(r: rusqlite::Result<T>) -> Result<T, String> { r.map_err(|e| e.to_string()) }

pub fn now_ts() -> String { chrono::Utc::now().to_rfc3339() }

pub fn new_id(prefix: &str) -> String { format!("{}_{}_{}", prefix, now_ts().replace(&['-',':','.'][..],""), std::process::id()) }

// ─── 项目 ───

pub fn list_projects() -> Result<Vec<ReqProject>, String> {
    with_conn(|c| {
        let mut stmt = c.prepare("SELECT id,name,description,created_at,updated_at FROM req_projects ORDER BY created_at DESC").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok(ReqProject { id: r.get(0)?, name: r.get(1)?, description: r.get(2)?, created_at: r.get(3)?, updated_at: r.get(4)? })).map_err(|e| e.to_string())?;
        let items: Vec<ReqProject> = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
        Ok(items)
    })
}

pub fn create_project(name: &str, description: &str) -> Result<ReqProject, String> {
    with_conn(|c| {
        let id = new_id("req"); let ts = now_ts();
        sql(c.execute("INSERT INTO req_projects (id,name,description,created_at,updated_at) VALUES (?1,?2,?3,?4,?5)", rusqlite::params![id, name, description, ts, ts]))?;
        Ok(ReqProject { id, name: name.to_string(), description: description.to_string(), created_at: ts.clone(), updated_at: ts })
    })
}

pub fn update_project(id: &str, name: Option<&str>, description: Option<&str>) -> Result<(), String> {
    with_conn(|c| {
        let ts = now_ts();
        if let Some(n) = name { sql(c.execute("UPDATE req_projects SET name=?1,updated_at=?2 WHERE id=?3", rusqlite::params![n, ts, id]))?; }
        if let Some(d) = description { sql(c.execute("UPDATE req_projects SET description=?1,updated_at=?2 WHERE id=?3", rusqlite::params![d, ts, id]))?; }
        Ok(())
    })
}

pub fn delete_project(id: &str) -> Result<(), String> {
    with_conn(|c| { sql(c.execute("DELETE FROM req_projects WHERE id=?1", rusqlite::params![id]))?; Ok(()) })
}

// ─── 模块 ───

pub fn list_modules(project_id: &str) -> Result<Vec<ReqModule>, String> {
    with_conn(|c| {
        let mut stmt = c.prepare("SELECT id,project_id,name,description,sort_order,created_at,updated_at FROM req_modules WHERE project_id=?1 ORDER BY sort_order,created_at").map_err(|e| e.to_string())?;
        let rows = stmt.query_map(rusqlite::params![project_id], |r| Ok(ReqModule { id: r.get(0)?, project_id: r.get(1)?, name: r.get(2)?, description: r.get(3)?, sort_order: r.get(4)?, created_at: r.get(5)?, updated_at: r.get(6)? })).map_err(|e| e.to_string())?;
        let items: Vec<ReqModule> = rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
        Ok(items)
    })
}

pub fn create_module(project_id: &str, name: &str, description: &str) -> Result<ReqModule, String> {
    with_conn(|c| {
        let id = new_id("mod"); let ts = now_ts();
        let next_order: i64 = c.query_row("SELECT COALESCE(MAX(sort_order),-1)+1 FROM req_modules WHERE project_id=?1", rusqlite::params![project_id], |r| r.get(0)).unwrap_or(0);
        sql(c.execute("INSERT INTO req_modules (id,project_id,name,description,sort_order,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", rusqlite::params![id, project_id, name, description, next_order, ts, ts]))?;
        Ok(ReqModule { id, project_id: project_id.to_string(), name: name.to_string(), description: description.to_string(), sort_order: next_order, created_at: ts.clone(), updated_at: ts })
    })
}

pub fn update_module(id: &str, name: Option<&str>, description: Option<&str>) -> Result<(), String> {
    with_conn(|c| {
        let ts = now_ts();
        if let Some(n) = name { sql(c.execute("UPDATE req_modules SET name=?1,updated_at=?2 WHERE id=?3", rusqlite::params![n, ts, id]))?; }
        if let Some(d) = description { sql(c.execute("UPDATE req_modules SET description=?1,updated_at=?2 WHERE id=?3", rusqlite::params![d, ts, id]))?; }
        Ok(())
    })
}

pub fn delete_module(id: &str) -> Result<(), String> {
    with_conn(|c| { sql(c.execute("DELETE FROM req_modules WHERE id=?1", rusqlite::params![id]))?; Ok(()) })
}

// ─── 图谱 ───

pub fn load_graph(module_id: &str) -> Result<Option<LearnGraph>, String> {
    with_conn(|c| {
        let mut stmt = c.prepare("SELECT name,source_type,source_desc,graph_json,created_at,updated_at FROM req_graphs WHERE module_id=?1").map_err(|e| e.to_string())?;
        let mut rows = stmt.query_map(rusqlite::params![module_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))).map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok((name, source_type, source_desc, json, _ca, ua))) => {
                let mut graph: LearnGraph = serde_json::from_str(&json).map_err(|e| format!("JSON: {}", e))?;
                graph.module_id = module_id.to_string();
                if graph.source_type.is_empty() { graph.source_type = source_type; }
                if graph.source_desc.is_empty() { graph.source_desc = source_desc; }
                if graph.project_name.is_empty() { graph.project_name = name; }
                graph.generated_at = ua;
                Ok(Some(graph))
            }
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    })
}

pub fn save_graph(graph: &LearnGraph) -> Result<(), String> {
    with_conn(|c| {
        let ts = now_ts();
        let json = serde_json::to_string(graph).map_err(|e| format!("序列化: {}", e))?;
        let existing: i64 = c.query_row("SELECT COUNT(*) FROM req_graphs WHERE module_id=?1", rusqlite::params![graph.module_id], |r| r.get(0)).unwrap_or(0);
        if existing > 0 {
            sql(c.execute("UPDATE req_graphs SET name=?1,source_type=?2,source_desc=?3,graph_json=?4,updated_at=?5 WHERE module_id=?6", rusqlite::params![graph.project_name, graph.source_type, graph.source_desc, json, ts, graph.module_id]))?;
        } else {
            sql(c.execute("INSERT INTO req_graphs (module_id,name,source_type,source_desc,graph_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", rusqlite::params![graph.module_id, graph.project_name, graph.source_type, graph.source_desc, json, ts, ts]))?;
        }
        Ok(())
    })
}

pub fn delete_graph(module_id: &str) -> Result<(), String> {
    with_conn(|c| { sql(c.execute("DELETE FROM req_graphs WHERE module_id=?1", rusqlite::params![module_id]))?; Ok(()) })
}

pub fn create_empty_graph(module_id: &str, name: &str) -> Result<LearnGraph, String> {
    let ts = now_ts();
    let graph = LearnGraph { module_id: module_id.to_string(), project_name: name.to_string(), summary: String::new(), generated_at: ts, source_type: "manual".to_string(), source_desc: "手动创建".to_string(), nodes: vec![LearnNode { id: "root".to_string(), name: name.to_string(), parent_id: None, description: "根".to_string(), detail: String::new(), kind: "root".to_string(), position_x: 0.0, position_y: 0.0 }] };
    save_graph(&graph)?;
    Ok(graph)
}