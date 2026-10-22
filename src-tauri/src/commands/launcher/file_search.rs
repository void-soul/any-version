//! Everything 式文件搜索。
//!
//! - 按盘符建立 / 更新文件索引（独立 SQLite：`file_index.db`，与启动器数据分离）
//! - 全局快捷键呼出的独立无边框悬浮窗（`index.html?popup=file-search`）
//! - 索引扫描在后台线程执行，进度通过事件 `file-search-index-progress` 实时推送
//!
//! 更新策略：扫描先写入 `fs_entry_tmp`，完成后一次性替换该盘符的旧数据。
//! 因此「更新索引」期间旧索引仍然可搜；「重建索引」会先清空旧数据。
//!
//! 检索策略（v2）：
//! - `fs_entry` 存条目（rowid 表，`UNIQUE(drive, path)` 去重）；
//! - `fs_entry_fts` 是 FTS5 trigram 全文索引，让 `%中缀%` 也能走索引；
//! - 词长全部 ≥3 时走 FTS；否则退回「前缀走 `idx_fs_name` + 中缀补全」两段式查询。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection};
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::commands::config::get_data_dir;

const POPUP_LABEL: &str = "file-search-popup";
const POPUP_W: f64 = 780.0;
const POPUP_H: f64 = 560.0;
const PROGRESS_EVENT: &str = "file-search-index-progress";

/// 索引库连接（与启动器主库分开，避免全盘索引把 launcher.db 撑大）。
static INDEX_DB: Mutex<Option<Connection>> = Mutex::new(None);
/// 当前正在索引的盘符（None = 空闲）。同一时间只允许一个索引任务。
static BUILDING_DRIVE: Mutex<Option<String>> = Mutex::new(None);
/// 取消标志：扫描循环每轮检查一次。
static CANCEL_INDEX: AtomicBool = AtomicBool::new(false);

// ============================ 数据模型 ============================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    /// "C:"
    pub letter: String,
    /// "C:\\"
    pub root: String,
    /// fixed / removable / network / cdrom / ramdisk / unknown
    pub drive_type: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// idle / building / ready
    pub status: String,
    pub item_count: i64,
    /// 最近一次索引完成时间（unix 秒）
    pub indexed_at: i64,
    pub elapsed_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub drive: String,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    /// 修改时间（unix 秒）
    pub modified: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexProgress {
    drive: String,
    scanned: usize,
    inserted: usize,
    current: String,
    elapsed_ms: i64,
    done: bool,
    canceled: bool,
    error: Option<String>,
}

enum ScanOutcome {
    Done { scanned: usize, count: i64 },
    Canceled { scanned: usize },
}

// ============================ 索引库 ============================

fn db_path() -> std::path::PathBuf {
    get_data_dir().join("file_index.db")
}

/// 索引库结构版本。v2：`fs_entry` 改为 rowid 表（与 FTS5 的 rowid 对齐）并新增 trigram 全文索引。
const SCHEMA_VERSION: i64 = 2;

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |_| Ok(()),
    )
    .is_ok()
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let names: Vec<String> = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => return false,
    };
    names.iter().any(|name| name == column)
}

/// 建表 / 升级结构。
///
/// v1 的 `fs_entry` 是 `WITHOUT ROWID`（无 `id` 列），无法与 FTS5 的 rowid 对齐，
/// 直接丢弃旧索引并把盘符状态重置为 idle，用户重新建立索引即可（索引本身可重建）。
fn init_schema(conn: &Connection) -> Result<(), String> {
    let legacy = table_exists(conn, "fs_entry") && !column_exists(conn, "fs_entry", "id");
    if legacy {
        conn.execute_batch(
            "DROP TABLE IF EXISTS fs_entry_fts;
             DROP TABLE IF EXISTS fs_entry_tmp;
             DROP TABLE IF EXISTS fs_entry;",
        )
        .map_err(|e| format!("清理旧版索引表失败: {}", e))?;
    }
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS fs_entry (
            id         INTEGER PRIMARY KEY,
            drive      TEXT    NOT NULL,
            path       TEXT    NOT NULL,
            name       TEXT    NOT NULL,
            name_lower TEXT    NOT NULL,
            is_dir     INTEGER NOT NULL DEFAULT 0,
            size       INTEGER NOT NULL DEFAULT 0,
            modified   INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_fs_path ON fs_entry(drive, path);
        CREATE INDEX IF NOT EXISTS idx_fs_name ON fs_entry(name_lower);

        CREATE TABLE IF NOT EXISTS fs_entry_tmp (
            drive      TEXT    NOT NULL,
            path       TEXT    NOT NULL,
            name       TEXT    NOT NULL,
            name_lower TEXT    NOT NULL,
            is_dir     INTEGER NOT NULL DEFAULT 0,
            size       INTEGER NOT NULL DEFAULT 0,
            modified   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (drive, path)
        ) WITHOUT ROWID;

        -- 文件名全文索引。trigram 分词支持任意位置的中缀匹配（搜索词需 ≥3 字符）。
        CREATE VIRTUAL TABLE IF NOT EXISTS fs_entry_fts USING fts5(
            name,
            tokenize = 'trigram'
        );

        CREATE TABLE IF NOT EXISTS fs_meta (
            drive TEXT PRIMARY KEY,
            status TEXT    NOT NULL DEFAULT 'idle',
            item_count INTEGER NOT NULL DEFAULT 0,
            indexed_at INTEGER NOT NULL DEFAULT 0,
            elapsed_ms INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .map_err(|e| format!("初始化索引表失败: {}", e))?;
    if legacy {
        conn.execute("UPDATE fs_meta SET status = 'idle', item_count = 0", [])
            .map_err(|e| format!("重置索引状态失败: {}", e))?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| format!("写入索引库版本失败: {}", e))?;
    Ok(())
}

fn build_connection() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建索引目录失败: {}", e))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开索引数据库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置索引库 WAL 模式失败: {}", e))?;
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    // 扫描线程与查询线程各持一个连接；写事务（尤其收尾的全文索引填充）期间
    // 给读写留出等待窗口，避免直接抛 database is locked。
    conn.pragma_update(None, "busy_timeout", 5000i64).ok();
    // 排序 / 临时结构放内存，避免大结果集排序落盘。
    conn.pragma_update(None, "temp_store", "MEMORY").ok();
    // 64 MiB 页缓存 + 256 MiB 只读映射：千万级索引的随机页读取尽量不触发系统调用。
    conn.pragma_update(None, "cache_size", -65536i64).ok();
    conn.pragma_update(None, "mmap_size", 268435456i64).ok();
    init_schema(&conn)?;
    Ok(conn)
}

fn with_conn<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&mut Connection) -> Result<T, String>,
{
    let mut guard = INDEX_DB.lock().map_err(|e| format!("索引库锁错误: {}", e))?;
    if guard.is_none() {
        *guard = Some(build_connection()?);
    }
    match guard.as_mut() {
        Some(conn) => f(conn),
        None => Err("索引数据库未连接".to_string()),
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn set_meta(
    conn: &Connection,
    drive: &str,
    status: &str,
    item_count: i64,
    indexed_at: i64,
    elapsed_ms: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO fs_meta (drive, status, item_count, indexed_at, elapsed_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(drive) DO UPDATE SET
            status = excluded.status,
            item_count = excluded.item_count,
            indexed_at = excluded.indexed_at,
            elapsed_ms = excluded.elapsed_ms",
        params![drive, status, item_count, indexed_at, elapsed_ms],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn set_status(drive: &str, status: &str) {
    let _ = with_conn(|conn| {
        conn.execute(
            "INSERT INTO fs_meta (drive, status) VALUES (?1, ?2)
             ON CONFLICT(drive) DO UPDATE SET status = excluded.status",
            params![drive, status],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

/// 取消/失败后恢复状态：按实际剩余数据判定 ready / idle，避免「重建索引被取消」
/// 之后仍显示旧的条目数（此时旧数据已被清空）。
fn restore_status(drive: &str) {
    let _ = with_conn(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fs_entry WHERE drive = ?1",
                params![drive],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let status = if count > 0 { "ready" } else { "idle" };
        conn.execute(
            "INSERT INTO fs_meta (drive, status, item_count) VALUES (?1, ?2, ?3)
             ON CONFLICT(drive) DO UPDATE SET
                status = excluded.status,
                item_count = excluded.item_count",
            params![drive, status, count],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

/// 清理扫描中途留下的临时行（失败/异常退出时调用）。
fn cleanup_tmp(drive: &str) {
    let _ = with_conn(|conn| {
        conn.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        Ok(())
    });
}

fn normalize_drive(drive: &str) -> Result<String, String> {
    let d = drive.trim().trim_end_matches('\\').to_uppercase();
    let bytes = d.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_uppercase() && bytes[1] == b':' {
        Ok(d)
    } else {
        Err(format!("无效盘符: {}", drive))
    }
}

// ============================ 盘符枚举 ============================

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn drive_type_key(t: u32) -> &'static str {
    // GetDriveTypeW 返回值（winbase.h），windows-sys 未导出对应常量。
    match t {
        2 => "removable",
        3 => "fixed",
        4 => "network",
        5 => "cdrom",
        6 => "ramdisk",
        _ => "unknown",
    }
}

#[cfg(windows)]
fn windows_drives() -> Vec<(String, String, u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives,
    };

    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = format!("{}:", (b'A' + i as u8) as char);
        let root = format!("{}\\", letter);
        let w = wide(&root);
        let dtype = drive_type_key(unsafe { GetDriveTypeW(w.as_ptr()) });
        let mut free_to_caller = 0u64;
        let mut total = 0u64;
        let mut free = 0u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                w.as_ptr(),
                &mut free_to_caller,
                &mut total,
                &mut free,
            )
        };
        if ok == 0 {
            total = 0;
            free = 0;
        }
        out.push((letter, dtype.to_string(), total, free));
    }
    out
}

#[cfg(not(windows))]
fn windows_drives() -> Vec<(String, String, u64, u64)> {
    Vec::new()
}

// ============================ 索引扫描 ============================

/// 系统目录：无权限且无检索价值，直接跳过。
const SKIP_DIRS: [&str; 2] = ["$Recycle.Bin", "System Volume Information"];

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn emit_progress(
    app: &tauri::AppHandle,
    drive: &str,
    scanned: usize,
    inserted: usize,
    current: &str,
    elapsed_ms: i64,
    done: bool,
    canceled: bool,
    error: Option<String>,
) {
    let payload = IndexProgress {
        drive: drive.to_string(),
        scanned,
        inserted,
        current: current.to_string(),
        elapsed_ms,
        done,
        canceled,
        error,
    };
    let _ = app.emit(PROGRESS_EVENT, payload);
}

type BatchRow = (String, String, String, i64, i64, i64);

fn flush_batch(
    conn: &mut Connection,
    drive: &str,
    batch: &mut Vec<BatchRow>,
) -> Result<usize, String> {
    if batch.is_empty() {
        return Ok(0);
    }
    let tx = conn
        .transaction()
        .map_err(|e| format!("开启索引事务失败: {}", e))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO fs_entry_tmp
                 (drive, path, name, name_lower, is_dir, size, modified)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|e| e.to_string())?;
        for (path, name, name_lower, is_dir, size, modified) in batch.iter() {
            stmt.execute(params![drive, path, name, name_lower, is_dir, size, modified])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| format!("提交索引批次失败: {}", e))?;
    let n = batch.len();
    batch.clear();
    Ok(n)
}

fn scan_drive(
    app: &tauri::AppHandle,
    drive: &str,
    mode: &str,
) -> Result<ScanOutcome, String> {
    let mut conn = build_connection()?;
    conn.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
        .map_err(|e| e.to_string())?;
    if mode == "rebuild" {
        conn.execute(
            "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
            params![drive],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_entry WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
    }

    let root = format!("{}\\", drive);
    let started = Instant::now();
    let mut scanned = 0usize;
    let mut inserted = 0usize;
    let mut batch: Vec<BatchRow> = Vec::with_capacity(2048);
    let mut last_emit = Instant::now();

    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.file_type().is_dir() {
                let name = e.file_name().to_string_lossy();
                return !SKIP_DIRS.iter().any(|s| name.eq_ignore_ascii_case(s));
            }
            true
        })
        .filter_map(|e| e.ok())
    {
        if CANCEL_INDEX.load(Ordering::SeqCst) {
            let _ = flush_batch(&mut conn, drive, &mut batch);
            let _ = conn.execute(
                "DELETE FROM fs_entry_tmp WHERE drive = ?1",
                params![drive],
            );
            return Ok(ScanOutcome::Canceled { scanned });
        }
        // 不跟随 junction / 符号链接，避免目录环与重复项。
        if entry.depth() == 0 || entry.file_type().is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.is_empty() {
            continue;
        }
        let is_dir = entry.file_type().is_dir();
        let meta = entry.metadata().ok();
        let size = if is_dir {
            0
        } else {
            meta.as_ref().map(|m| m.len() as i64).unwrap_or(0)
        };
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let path = entry.path().to_string_lossy().to_string();
        batch.push((
            path,
            name.clone(),
            name.to_lowercase(),
            if is_dir { 1 } else { 0 },
            size,
            modified,
        ));
        scanned += 1;
        if batch.len() >= 2048 {
            inserted += flush_batch(&mut conn, drive, &mut batch)?;
        }

        if last_emit.elapsed() >= Duration::from_millis(300) {
            last_emit = Instant::now();
            emit_progress(
                app,
                drive,
                scanned,
                inserted + batch.len(),
                &entry.path().to_string_lossy(),
                started.elapsed().as_millis() as i64,
                false,
                false,
                None,
            );
        }
    }

    inserted += flush_batch(&mut conn, drive, &mut batch)?;

    // 全文索引的写入比普通行慢得多，单独提示一次，避免进度条停在 99% 看似卡死。
    emit_progress(
        app,
        drive,
        scanned,
        inserted,
        "正在写入全文索引…",
        started.elapsed().as_millis() as i64,
        false,
        false,
        None,
    );

    // 一次性替换：旧索引在扫描期间始终可搜（update 模式）。
    let tx = conn
        .transaction()
        .map_err(|e| format!("替换索引事务失败: {}", e))?;
    tx.execute(
        "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
        params![drive],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM fs_entry WHERE drive = ?1", params![drive])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO fs_entry (drive, path, name, name_lower, is_dir, size, modified)
         SELECT drive, path, name, name_lower, is_dir, size, modified
         FROM fs_entry_tmp WHERE drive = ?1",
        params![drive],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO fs_entry_fts (rowid, name)
         SELECT id, name FROM fs_entry WHERE drive = ?1",
        params![drive],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| format!("提交索引失败: {}", e))?;

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fs_entry WHERE drive = ?1",
            params![drive],
            |row| row.get(0),
        )
        .unwrap_or(inserted as i64);

    set_meta(
        &conn,
        drive,
        "ready",
        count,
        now_secs(),
        started.elapsed().as_millis() as i64,
    )?;

    Ok(ScanOutcome::Done { scanned, count })
}

// ============================ 命令：索引进度 / 管理 ============================

#[tauri::command]
pub async fn file_search_list_drives() -> Result<Vec<DriveInfo>, String> {
    let mut meta_map: std::collections::HashMap<String, (String, i64, i64, i64)> =
        std::collections::HashMap::new();
    with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT drive, status, item_count, indexed_at, elapsed_ms FROM fs_meta")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for (drive, status, count, indexed_at, elapsed) in rows.flatten() {
            meta_map.insert(drive, (status, count, indexed_at, elapsed));
        }
        Ok(())
    })?;

    let building = BUILDING_DRIVE
        .lock()
        .map(|g| g.clone())
        .unwrap_or(None);

    let mut out: Vec<DriveInfo> = windows_drives()
        .into_iter()
        .map(|(letter, drive_type, total, free)| {
            let (status, item_count, indexed_at, elapsed_ms) = meta_map
                .get(&letter)
                .cloned()
                .unwrap_or_else(|| ("idle".to_string(), 0, 0, 0));
            let status = if building.as_deref() == Some(letter.as_str()) {
                "building".to_string()
            } else {
                status
            };
            DriveInfo {
                root: format!("{}\\", letter),
                letter,
                drive_type,
                total_bytes: total,
                free_bytes: free,
                status,
                item_count,
                indexed_at,
                elapsed_ms,
            }
        })
        .collect();
    out.sort_by(|a, b| a.letter.cmp(&b.letter));
    Ok(out)
}

#[tauri::command]
pub async fn file_search_start_index(
    app: tauri::AppHandle,
    drive: String,
    mode: Option<String>,
) -> Result<(), String> {
    let drive = normalize_drive(&drive)?;
    let root = format!("{}\\", drive);
    if !Path::new(&root).exists() {
        return Err(format!("盘符不存在: {}", root));
    }
    let mode = mode.unwrap_or_else(|| "update".to_string());
    {
        let mut guard = BUILDING_DRIVE
            .lock()
            .map_err(|e| format!("索引状态锁错误: {}", e))?;
        if guard.is_some() {
            return Err("已有索引任务正在进行，请先停止".to_string());
        }
        *guard = Some(drive.clone());
    }
    CANCEL_INDEX.store(false, Ordering::SeqCst);
    set_status(&drive, "building");

    let app_for_thread = app.clone();
    let drive_for_thread = drive.clone();
    std::thread::spawn(move || {
        let result = scan_drive(&app_for_thread, &drive_for_thread, &mode);
        {
            if let Ok(mut guard) = BUILDING_DRIVE.lock() {
                *guard = None;
            }
        }
        match result {
            Ok(ScanOutcome::Done { scanned, count }) => {
                emit_progress(
                    &app_for_thread,
                    &drive_for_thread,
                    scanned,
                    count as usize,
                    "",
                    0,
                    true,
                    false,
                    None,
                );
            }
            Ok(ScanOutcome::Canceled { scanned }) => {
                restore_status(&drive_for_thread);
                emit_progress(
                    &app_for_thread,
                    &drive_for_thread,
                    scanned,
                    0,
                    "",
                    0,
                    true,
                    true,
                    None,
                );
            }
            Err(err) => {
                cleanup_tmp(&drive_for_thread);
                restore_status(&drive_for_thread);
                emit_progress(
                    &app_for_thread,
                    &drive_for_thread,
                    0,
                    0,
                    "",
                    0,
                    true,
                    false,
                    Some(err),
                );
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn file_search_cancel_index() -> Result<(), String> {
    CANCEL_INDEX.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn file_search_delete_index(drive: String) -> Result<(), String> {
    let drive = normalize_drive(&drive)?;
    with_conn(|conn| {
        conn.execute(
            "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
            params![drive],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_entry WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_meta WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

// ============================ 命令：搜索 ============================

/// 一条命中及其 rowid（rowid 用于两段式查询去重）。
type HitRow = (i64, FileHit);

fn map_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<HitRow> {
    Ok((
        row.get(0)?,
        FileHit {
            drive: row.get(1)?,
            path: row.get(2)?,
            name: row.get(3)?,
            is_dir: row.get::<_, i64>(4)? != 0,
            size: row.get(5)?,
            modified: row.get(6)?,
        },
    ))
}

fn search_impl(
    query: &str,
    limit: usize,
    drives: &[String],
) -> Result<Vec<FileHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let tokens: Vec<String> = q
        .split_whitespace()
        .take(5)
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 500);

    with_conn(|conn| search_with_conn(conn, q, &tokens, limit, drives))
}

fn search_with_conn(
    conn: &Connection,
    q: &str,
    tokens: &[String],
    limit: usize,
    drives: &[String],
) -> Result<Vec<FileHit>, String> {
    // trigram 只索引 ≥3 字符的片段，短词无法命中全文索引，交给 LIKE。
    if tokens.iter().all(|t| t.chars().count() >= 3) {
        match fts_search(conn, q, tokens, limit, drives) {
            Ok(hits) => return Ok(hits),
            // 全文索引异常（缺失/损坏）时退回 LIKE：先保证搜得到，再谈搜得快。
            Err(err) => crate::exit_log!("[文件搜索] 全文检索失败，回退 LIKE: {}", err),
        }
    }
    like_search(conn, q, tokens, limit, drives)
}

/// FTS5 查询表达式：每个词用双引号包成短语，词间 AND（即「同时包含」）。
fn fts_expr(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// 走 trigram 全文索引：中缀、前缀、完全匹配都由索引定位，避免全表扫描。
fn fts_search(
    conn: &Connection,
    q: &str,
    tokens: &[String],
    limit: usize,
    drives: &[String],
) -> Result<Vec<FileHit>, String> {
    let mut values: Vec<Value> = vec![Value::Text(fts_expr(tokens))];
    let mut conds: Vec<String> = vec!["fs_entry_fts MATCH ?1".to_string()];

    if !drives.is_empty() {
        let mut placeholders = Vec::new();
        for d in drives {
            values.push(Value::Text(d.clone()));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("e.drive IN ({})", placeholders.join(",")));
    }

    let q_lower = q.to_lowercase();
    values.push(Value::Text(q_lower.clone()));
    let exact_idx = values.len();
    values.push(Value::Text(format!("{}%", escape_like(&q_lower))));
    let prefix_idx = values.len();
    values.push(Value::Integer(limit as i64));
    let limit_idx = values.len();

    let sql = format!(
        "SELECT e.id, e.drive, e.path, e.name, e.is_dir, e.size, e.modified
         FROM fs_entry_fts JOIN fs_entry e ON e.id = fs_entry_fts.rowid
         WHERE {conds}
         ORDER BY CASE
             WHEN e.name_lower = ?{exact} THEN 0
             WHEN e.name_lower LIKE ?{prefix} ESCAPE '\\' THEN 1
             ELSE 2 END,
         length(e.name), e.name COLLATE NOCASE
         LIMIT ?{limit}",
        conds = conds.join(" AND "),
        exact = exact_idx,
        prefix = prefix_idx,
        limit = limit_idx,
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("准备全文检索语句失败: {}", e))?;
    let rows = stmt
        .query_map(params_from_iter(values), map_hit)
        .map_err(|e| format!("执行全文检索失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map(|rows| rows.into_iter().map(|(_, hit)| hit).collect())
        .map_err(|e| format!("读取全文检索结果失败: {}", e))
}

/// 短词（<3 字符）退化路径：前缀命中走 `idx_fs_name`，再用中缀扫描补足剩余名额。
fn like_search(
    conn: &Connection,
    q: &str,
    tokens: &[String],
    limit: usize,
    drives: &[String],
) -> Result<Vec<FileHit>, String> {
    let mut rows = prefix_search(conn, q, limit, drives)?;
    if rows.len() < limit {
        let exclude: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
        rows.extend(infix_search(
            conn,
            tokens,
            limit - rows.len(),
            drives,
            &exclude,
        )?);
    }
    Ok(rows.into_iter().map(|(_, hit)| hit).collect())
}

/// 第一段：完全同名 + 前缀命中（两段都能命中索引）。
fn prefix_search(
    conn: &Connection,
    q: &str,
    limit: usize,
    drives: &[String],
) -> Result<Vec<HitRow>, String> {
    let q_lower = q.to_lowercase();
    let mut values: Vec<Value> = Vec::new();
    values.push(Value::Text(q_lower.clone()));
    let exact_idx = values.len();
    values.push(Value::Text(format!("{}%", escape_like(&q_lower))));
    let prefix_idx = values.len();

    let mut conds: Vec<String> = vec![format!(
        "(name_lower = ?{exact} OR name_lower LIKE ?{prefix} ESCAPE '\\')",
        exact = exact_idx,
        prefix = prefix_idx
    )];
    if !drives.is_empty() {
        let mut placeholders = Vec::new();
        for d in drives {
            values.push(Value::Text(d.clone()));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("drive IN ({})", placeholders.join(",")));
    }
    values.push(Value::Integer(limit as i64));
    let limit_idx = values.len();

    let sql = format!(
        "SELECT id, drive, path, name, is_dir, size, modified FROM fs_entry
         WHERE {conds}
         ORDER BY CASE WHEN name_lower = ?{exact} THEN 0 ELSE 1 END,
         length(name), name COLLATE NOCASE
         LIMIT ?{limit}",
        conds = conds.join(" AND "),
        exact = exact_idx,
        limit = limit_idx,
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("准备搜索语句失败: {}", e))?;
    let rows = stmt
        .query_map(params_from_iter(values), map_hit)
        .map_err(|e| format!("执行搜索失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取搜索结果失败: {}", e))
}

/// 第二段：多词中缀匹配，排除第一段已返回的 rowid，避免重复。
fn infix_search(
    conn: &Connection,
    tokens: &[String],
    limit: usize,
    drives: &[String],
    exclude_ids: &[i64],
) -> Result<Vec<HitRow>, String> {
    let mut values: Vec<Value> = Vec::new();
    let mut conds: Vec<String> = Vec::new();

    for token in tokens {
        values.push(Value::Text(format!("%{}%", escape_like(token))));
        conds.push(format!("name_lower LIKE ?{} ESCAPE '\\'", values.len()));
    }
    if !drives.is_empty() {
        let mut placeholders = Vec::new();
        for d in drives {
            values.push(Value::Text(d.clone()));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("drive IN ({})", placeholders.join(",")));
    }
    if !exclude_ids.is_empty() {
        let mut placeholders = Vec::new();
        for id in exclude_ids {
            values.push(Value::Integer(*id));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("id NOT IN ({})", placeholders.join(",")));
    }
    values.push(Value::Integer(limit as i64));
    let limit_idx = values.len();

    let sql = format!(
        "SELECT id, drive, path, name, is_dir, size, modified FROM fs_entry
         WHERE {conds}
         ORDER BY length(name), name COLLATE NOCASE
         LIMIT ?{limit}",
        conds = conds.join(" AND "),
        limit = limit_idx,
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("准备搜索语句失败: {}", e))?;
    let rows = stmt
        .query_map(params_from_iter(values), map_hit)
        .map_err(|e| format!("执行搜索失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取搜索结果失败: {}", e))
}

#[tauri::command]
pub async fn file_search_query(
    query: String,
    limit: Option<usize>,
    drives: Option<Vec<String>>,
) -> Result<Vec<FileHit>, String> {
    let normalized: Vec<String> = drives
        .unwrap_or_default()
        .iter()
        .filter_map(|d| normalize_drive(d).ok())
        .collect();
    let limit = limit.unwrap_or(100);
    tauri::async_runtime::spawn_blocking(move || search_impl(&query, limit, &normalized))
        .await
        .map_err(|e| format!("搜索线程失败: {}", e))?
}

// ============================ 悬浮窗 ============================

#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut pt) } != 0 {
        Some((pt.x, pt.y))
    } else {
        None
    }
}

#[cfg(windows)]
fn cursor_work_area() -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let (cx, cy) = cursor_position()?;
    let hmon = unsafe { MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_null() {
        return None;
    }
    let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(hmon, &mut mi) } == 0 {
        return None;
    }
    let r = mi.rcWork;
    Some((r.left, r.top, r.right, r.bottom))
}

/// 悬浮窗定位：鼠标所在显示器水平居中、垂直位于上方 16%（Spotlight 式）。
#[cfg(windows)]
fn focused_position(win_w: i32, win_h: i32) -> Option<(i32, i32)> {
    let (left, top, right, bottom) = cursor_work_area()?;
    let x = left + ((right - left) - win_w) / 2;
    let y = top + ((bottom - top) as f64 * 0.16) as i32;
    Some((x.max(left), y.max(top).min((bottom - win_h).max(top))))
}

#[cfg(not(windows))]
fn focused_position(_win_w: i32, _win_h: i32) -> Option<(i32, i32)> {
    None
}

fn ensure_popup(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        return Ok(win);
    }
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        POPUP_LABEL,
        tauri::WebviewUrl::App("index.html?popup=file-search".into()),
    )
    .title("文件搜索")
    .inner_size(POPUP_W, POPUP_H)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .shadow(false);
    if let Some((x, y)) = focused_position(POPUP_W as i32, POPUP_H as i32) {
        builder = builder.position(x as f64, y as f64);
    }
    builder
        .build()
        .map_err(|e| format!("创建文件搜索悬浮窗失败: {}", e))
}

/// 呼出 / 收起文件搜索悬浮窗（已聚焦时再按热键则隐藏）。
pub fn open_search_popup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        if win.is_focused().unwrap_or(false) {
            let _ = win.hide();
            return Ok(());
        }
    }
    ensure_popup(&app)?;
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app_main.get_webview_window(POPUP_LABEL) {
            let _ = w.show();
            #[cfg(windows)]
            if let Some((x, y)) = focused_position(POPUP_W as i32, POPUP_H as i32) {
                let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
            }
            let _ = w.set_focus();
            let webview: &tauri::Webview<tauri::Wry> = w.as_ref();
            let _ = webview.set_focus();
            // 通知前端重新聚焦输入框（窗口复用时不会重新挂载组件）。
            let _ = w.emit("file-search-focus", ());
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn open_file_search_popup(app: tauri::AppHandle) -> Result<(), String> {
    open_search_popup(app)
}

#[tauri::command]
pub async fn hide_file_search_popup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        let _ = win.hide();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        escape_like, fts_expr, init_schema, normalize_drive, search_with_conn, FileHit,
    };
    use rusqlite::{params, Connection};

    #[test]
    fn drive_is_normalized() {
        assert_eq!(normalize_drive("c:").unwrap(), "C:");
        assert_eq!(normalize_drive("d:\\").unwrap(), "D:");
        // 缺少冒号或长度不对一律拒绝
        assert!(normalize_drive("c").is_err());
        assert!(normalize_drive("C:\\Windows").is_err());
    }

    #[test]
    fn like_wildcards_are_escaped() {
        assert_eq!(escape_like("100%_ok"), "100\\%\\_ok");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
    }

    #[test]
    fn fts_terms_are_quoted() {
        assert_eq!(fts_expr(&["a b".to_string()]), "\"a b\"");
        // 词内的双引号需要成对转义，否则会破坏 FTS5 查询语法
        assert_eq!(
            fts_expr(&["x\"y".to_string(), "z".to_string()]),
            "\"x\"\"y\" AND \"z\""
        );
    }

    /// 建一个内存索引库并写入若干条目（同时维护 FTS 行，模拟扫描完成后的状态）。
    fn seeded_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for (path, name) in [
            ("C:\\docs\\report.docx", "report.docx"),
            ("C:\\docs\\my_report_2024.docx", "my_report_2024.docx"),
            ("C:\\apps\\report.exe", "report.exe"),
            ("C:\\tmp\\ab.txt", "ab.txt"),
        ] {
            conn.execute(
                "INSERT INTO fs_entry (drive, path, name, name_lower, is_dir, size, modified)
                 VALUES ('C:', ?1, ?2, ?3, 0, 0, 0)",
                params![path, name, name.to_lowercase()],
            )
            .unwrap();
            let id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO fs_entry_fts (rowid, name) VALUES (?1, ?2)",
                params![id, name],
            )
            .unwrap();
        }
        conn
    }

    fn names(hits: &[FileHit]) -> Vec<String> {
        hits.iter().map(|h| h.name.clone()).collect()
    }

    #[test]
    fn fts_finds_infix_and_ranks_prefix_first() {
        let conn = seeded_db();
        let tokens = vec!["report".to_string()];
        let hits = search_with_conn(&conn, "report", &tokens, 10, &[]).unwrap();
        let got = names(&hits);
        assert!(got.contains(&"my_report_2024.docx".to_string()));
        // 前缀命中排在包含命中之前
        let prefix_pos = got.iter().position(|n| n == "report.exe").unwrap();
        let infix_pos = got.iter().position(|n| n == "my_report_2024.docx").unwrap();
        assert!(prefix_pos < infix_pos);
    }

    #[test]
    fn short_token_falls_back_to_like() {
        let conn = seeded_db();
        let tokens = vec!["ab".to_string()];
        let hits = search_with_conn(&conn, "ab", &tokens, 10, &[]).unwrap();
        assert_eq!(names(&hits), vec!["ab.txt".to_string()]);
    }

    #[test]
    fn multi_token_requires_all_terms() {
        let conn = seeded_db();
        let tokens = vec!["report".to_string(), "2024".to_string()];
        let hits = search_with_conn(&conn, "report 2024", &tokens, 10, &[]).unwrap();
        assert_eq!(names(&hits), vec!["my_report_2024.docx".to_string()]);
    }

    #[test]
    fn drive_filter_limits_results() {
        let conn = seeded_db();
        let tokens = vec!["report".to_string()];
        let hits = search_with_conn(&conn, "report", &tokens, 10, &["D:".to_string()]).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn deleting_entries_clears_fts() {
        let conn = seeded_db();
        conn.execute(
            "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
            params!["C:"],
        )
        .unwrap();
        conn.execute("DELETE FROM fs_entry WHERE drive = ?1", params!["C:"])
            .unwrap();
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM fs_entry_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 0);
        let tokens = vec!["report".to_string()];
        assert!(search_with_conn(&conn, "report", &tokens, 10, &[])
            .unwrap()
            .is_empty());
    }
}
