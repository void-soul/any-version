use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::commands::config::get_data_dir;

/// 全局数据库连接（Mutex 保护，WAL 模式），与 tasks.db 同一风格。
static DB_CONN: Mutex<Option<rusqlite::Connection>> = Mutex::new(None);

fn db_path() -> std::path::PathBuf {
    get_data_dir().join("api.db")
}

/// 初始化数据库（幂等）。
pub fn init_db() -> Result<(), String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = rusqlite::Connection::open(&path).map_err(|e| format!("打开 API 数据库失败: {}", e))?;

    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON").ok();

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS api_projects (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            description   TEXT NOT NULL DEFAULT '',
            active_env_id TEXT,
            common_headers  TEXT NOT NULL DEFAULT '[]',
            common_params   TEXT NOT NULL DEFAULT '[]',
            created_at    TEXT NOT NULL,
            updated_at    TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS api_environments (
            id         TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            name       TEXT NOT NULL,
            variables  TEXT NOT NULL DEFAULT '{}',
            sort_order INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(project_id) REFERENCES api_projects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_api_env_project ON api_environments(project_id);

        CREATE TABLE IF NOT EXISTS api_modules (
            id          TEXT PRIMARY KEY,
            project_id  TEXT NOT NULL,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            sort_order  INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(project_id) REFERENCES api_projects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_api_mod_project ON api_modules(project_id);

        CREATE TABLE IF NOT EXISTS api_endpoints (
            id          TEXT PRIMARY KEY,
            project_id  TEXT NOT NULL,
            module_id   TEXT,
            name        TEXT NOT NULL,
            method      TEXT NOT NULL DEFAULT 'GET',
            url         TEXT NOT NULL,
            headers     TEXT NOT NULL DEFAULT '[]',
            query_params TEXT NOT NULL DEFAULT '[]',
            path_params  TEXT NOT NULL DEFAULT '[]',
            body        TEXT NOT NULL DEFAULT '',
            body_type   TEXT NOT NULL DEFAULT 'none',
            description TEXT NOT NULL DEFAULT '',
            docs_md     TEXT NOT NULL DEFAULT '',
            timeout_ms  INTEGER NOT NULL DEFAULT 15000,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            FOREIGN KEY(project_id) REFERENCES api_projects(id) ON DELETE CASCADE,
            FOREIGN KEY(module_id) REFERENCES api_modules(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_api_ep_project ON api_endpoints(project_id);
        CREATE INDEX IF NOT EXISTS idx_api_ep_module  ON api_endpoints(module_id);

        CREATE TABLE IF NOT EXISTS api_unit_tests (
            id         TEXT PRIMARY KEY,
            endpoint_id TEXT NOT NULL,
            name       TEXT NOT NULL,
            assertions TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            FOREIGN KEY(endpoint_id) REFERENCES api_endpoints(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_api_test_endpoint ON api_unit_tests(endpoint_id);

        CREATE TABLE IF NOT EXISTS api_load_runs (
            id          TEXT PRIMARY KEY,
            endpoint_id TEXT NOT NULL,
            name        TEXT NOT NULL DEFAULT '',
            config      TEXT NOT NULL DEFAULT '{}',
            report      TEXT NOT NULL DEFAULT '',
            created_at  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_api_load_endpoint ON api_load_runs(endpoint_id);

        CREATE TABLE IF NOT EXISTS api_preset_headers (
            id         TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            name       TEXT NOT NULL,
            headers    TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            FOREIGN KEY(project_id) REFERENCES api_projects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_api_preset_project ON api_preset_headers(project_id);

        CREATE TABLE IF NOT EXISTS api_history (
            id          TEXT PRIMARY KEY,
            project_id  TEXT NOT NULL,
            endpoint_id TEXT,
            name        TEXT NOT NULL,
            method      TEXT NOT NULL,
            url         TEXT NOT NULL,
            input       TEXT NOT NULL DEFAULT '{}',
            created_at  TEXT NOT NULL,
            FOREIGN KEY(project_id) REFERENCES api_projects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_api_hist_project ON api_history(project_id);
        "#,
    )
    .map_err(|e| format!("初始化 API 数据库表失败: {}", e))?;

    migrate_endpoint_columns(&conn)?;
    migrate_module_columns(&conn)?;
    migrate_project_columns(&conn)?;

    *DB_CONN.lock().map_err(|e| e.to_string())? = Some(conn);
    Ok(())
}

/// 为既有 api_projects 表补齐新列（幂等）。
fn migrate_project_columns(conn: &rusqlite::Connection) -> Result<(), String> {
    let has_column = |name: &str| -> Result<bool, String> {
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('api_projects') WHERE name = ?1")
            .map_err(|e| e.to_string())?;
        let n: i64 = stmt
            .query_row(rusqlite::params![name], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    };
    for (name, def) in [("common_headers", "TEXT NOT NULL DEFAULT '[]'"), ("common_params", "TEXT NOT NULL DEFAULT '[]'")] {
        if !has_column(name)? {
            conn.execute_batch(&format!("ALTER TABLE api_projects ADD COLUMN {} {}", name, def))
                .map_err(|e| format!("迁移 api_projects.{} 失败: {}", name, e))?;
        }
    }
    Ok(())
}

/// 为既有 api_modules 表补齐新列（幂等）。
fn migrate_module_columns(conn: &rusqlite::Connection) -> Result<(), String> {
    let has_column = |name: &str| -> Result<bool, String> {
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('api_modules') WHERE name = ?1")
            .map_err(|e| e.to_string())?;
        let n: i64 = stmt
            .query_row(rusqlite::params![name], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    };
    if !has_column("description")? {
        conn.execute_batch("ALTER TABLE api_modules ADD COLUMN description TEXT NOT NULL DEFAULT ''")
            .map_err(|e| format!("迁移 api_modules.description 失败: {}", e))?;
    }
    Ok(())
}

/// 为既有 api_endpoints 表补齐新列（幂等）。
fn migrate_endpoint_columns(conn: &rusqlite::Connection) -> Result<(), String> {
    let has_column = |name: &str| -> Result<bool, String> {
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('api_endpoints') WHERE name = ?1")
            .map_err(|e| e.to_string())?;
        let n: i64 = stmt
            .query_row(rusqlite::params![name], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    };
    let cols: &[(&str, &str)] = &[
        ("body_form", "TEXT NOT NULL DEFAULT '[]'"),
        ("body_urlencoded", "TEXT NOT NULL DEFAULT '[]'"),
        ("body_graphql_query", "TEXT NOT NULL DEFAULT ''"),
        ("body_graphql_variables", "TEXT NOT NULL DEFAULT ''"),
        ("authorization", "TEXT NOT NULL DEFAULT '{}'"),
        ("cookies", "TEXT NOT NULL DEFAULT '[]'"),
        ("settings", "TEXT NOT NULL DEFAULT '{}'"),
        ("response_comment", "TEXT NOT NULL DEFAULT ''"),
        ("is_favorite", "INTEGER NOT NULL DEFAULT 0"),
    ];
    for (name, def) in cols {
        if !has_column(name)? {
            conn.execute_batch(&format!("ALTER TABLE api_endpoints ADD COLUMN {} {}", name, def))
                .map_err(|e| format!("迁移 api_endpoints.{} 失败: {}", name, e))?;
        }
    }
    Ok(())
}

/// 在数据库连接上执行闭包（自动初始化）。
pub fn with_db<T>(f: impl FnOnce(&mut rusqlite::Connection) -> Result<T, String>) -> Result<T, String> {
    if DB_CONN.lock().map_err(|e| e.to_string())?.is_none() {
        init_db()?;
    }
    let mut guard = DB_CONN.lock().map_err(|e| e.to_string())?;
    let conn = guard.as_mut().ok_or("API 数据库未初始化")?;
    f(conn)
}

/// 当前时间戳（RFC3339 UTC）。
pub fn now_ts() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 进程内自增序号：保证同一毫秒内批量创建（如默认模块）不撞 id。
static ID_SEQ: AtomicU64 = AtomicU64::new(0);

/// 生成唯一 id。
pub fn new_id(prefix: &str) -> String {
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed) % 1000;
    format!(
        "{}_{}_{}_{}",
        prefix,
        chrono::Local::now().format("%Y%m%d%H%M%S%3f"),
        std::process::id(),
        seq
    )
}

// ─── 通用查询辅助 ───

/// 从 JSON 文本解析变量表，失败返回空表。
pub fn parse_variables(raw: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// 解析键值对列表，失败返回空。
pub fn parse_kv(raw: &str) -> Vec<super::models::KeyValueItem> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// 读取项目行。
pub fn project_row(row: &rusqlite::Row) -> Result<super::models::ApiProject, rusqlite::Error> {
    let common_headers_raw: String = row.get(4)?;
    let common_params_raw: String = row.get(5)?;
    Ok(super::models::ApiProject {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        active_env_id: row.get(3)?,
        common_headers: parse_kv(&common_headers_raw),
        common_params: parse_kv(&common_params_raw),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

/// 读取环境行。
pub fn env_row(row: &rusqlite::Row) -> Result<super::models::ApiEnvironment, rusqlite::Error> {
    let variables_raw: String = row.get(3)?;
    Ok(super::models::ApiEnvironment {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        variables: parse_variables(&variables_raw),
        sort_order: row.get(4)?,
    })
}

/// 读取模块行。
pub fn module_row(row: &rusqlite::Row) -> Result<super::models::ApiModule, rusqlite::Error> {
    Ok(super::models::ApiModule {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        sort_order: row.get(4)?,
    })
}

/// 读取接口行。
pub fn endpoint_row(row: &rusqlite::Row) -> Result<super::models::ApiEndpoint, rusqlite::Error> {
    let headers_raw: String = row.get(6)?;
    let query_raw: String = row.get(7)?;
    let path_raw: String = row.get(8)?;
    let body_form_raw: String = row.get(16)?;
    let body_urlencoded_raw: String = row.get(17)?;
    let auth_raw: String = row.get(20)?;
    let cookies_raw: String = row.get(21)?;
    let settings_raw: String = row.get(22)?;
    Ok(super::models::ApiEndpoint {
        id: row.get(0)?,
        project_id: row.get(1)?,
        module_id: row.get(2)?,
        name: row.get(3)?,
        method: row.get(4)?,
        url: row.get(5)?,
        headers: parse_kv(&headers_raw),
        query_params: parse_kv(&query_raw),
        path_params: parse_kv(&path_raw),
        body: row.get(9)?,
        body_type: row.get(10)?,
        body_form: serde_json::from_str(&body_form_raw).unwrap_or_default(),
        body_urlencoded: parse_kv(&body_urlencoded_raw),
        body_graphql_query: row.get(18)?,
        body_graphql_variables: row.get(19)?,
        authorization: serde_json::from_str(&auth_raw).unwrap_or_default(),
        cookies: parse_kv(&cookies_raw),
        settings: serde_json::from_str(&settings_raw).unwrap_or_default(),
        response_comment: row.get(23)?,
        is_favorite: row.get::<_, i64>(24)? != 0,
        description: row.get(11)?,
        docs_md: row.get(12)?,
        timeout_ms: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

/// 模块即接口文件夹，创建项目时不自动生成模块（由用户按需添加）。
/// 保留此常量仅作占位说明。
pub const DEFAULT_MODULES: &[&str] = &[];
