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
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::commands::config::get_data_dir;

use super::gitignore::IgnoreStack;

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
    /// 建立索引时是否遵循 .gitignore
    pub respect_gitignore: bool,
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
            elapsed_ms INTEGER NOT NULL DEFAULT 0,
            respect_gitignore INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .map_err(|e| format!("初始化索引表失败: {}", e))?;
    // v2 之前的库缺这一列；幂等补齐，不强制重建索引。
    if !column_exists(conn, "fs_meta", "respect_gitignore") {
        conn.execute(
            "ALTER TABLE fs_meta ADD COLUMN respect_gitignore INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| format!("补充索引状态字段失败: {}", e))?;
    }
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
    respect_gitignore: bool,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO fs_meta (drive, status, item_count, indexed_at, elapsed_ms, respect_gitignore)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(drive) DO UPDATE SET
            status = excluded.status,
            item_count = excluded.item_count,
            indexed_at = excluded.indexed_at,
            elapsed_ms = excluded.elapsed_ms,
            respect_gitignore = excluded.respect_gitignore",
        params![
            drive,
            status,
            item_count,
            indexed_at,
            elapsed_ms,
            respect_gitignore as i64
        ],
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

/// 任意层级都跳过的目录名（小写比较）：系统目录、版本库内部与常见依赖/构建缓存。
/// 这些内容没有检索价值，索引进来只会让索引膨胀、搜索变慢。
const SKIP_DIR_NAMES: &[&str] = &[
    "$recycle.bin",
    "system volume information",
    ".git",
    "node_modules",
    ".pnpm-store",
    ".yarn",
    ".npm",
    ".gradle",
    ".m2",
    ".nuget",
    ".next",
    ".nuxt",
    ".turbo",
    ".venv",
    // Python
    "__pycache__",
    ".tox",
    ".nox",
    // 前端框架与打包产物
    ".webpack",
    ".nyc_output",
    ".angular",
    ".svelte-kit",
    ".astro",
    ".docusaurus",
    // 语言工具链
    ".dart_tool",
    ".pub-cache",
    ".stack-work",
    // 编辑器 / 笔记本的会话与索引
    ".vs",
    ".vscode-server",
    ".vscode-test",
    ".ipynb_checkpoints",
    ".history",
    "ipch",
    "workspacestorage",
    "blob_storage",
    "service worker",
    "crashdumps",
];

/// 只在特定位置跳过的缓存目录（相对盘根的小写路径后缀，避免误伤同名目录）。
const SKIP_REL_SUFFIXES: &[&str] = &[
    "appdata/local/temp",
    "appdata/local/yarn/cache",
    "appdata/local/npm-cache",
    "appdata/roaming/npm-cache",
    "appdata/local/pnpm/store",
    "appdata/local/pip/cache",
    "appdata/local/nuget/v3-cache",
    "appdata/local/go-build",
    "appdata/local/microsoft/windows/inetcache",
    // IDE：索引 / 日志 / 临时文件集中在这里（配置在 Roaming，不受影响）
    "appdata/local/jetbrains",
    // 浏览器：User Data 下是各 profile 的缓存、代码缓存与 GPU 缓存
    "appdata/local/google/chrome/user data",
    "appdata/local/microsoft/edge/user data",
    // 其他应用缓存
    "appdata/local/unity/cache",
    "appdata/local/steam/htmlcache",
    "anaconda3/pkgs",
    "miniconda3/pkgs",
    "cargo/registry",
    "windows/temp",
    "windows/softwaredistribution",
    "windows/installer",
    "windows/winsxs",
    "windows/servicing",
];

/// 路径后缀匹配，要求落在目录边界上（`.../temp` 而不是 `.../attempt`）。
fn ends_with_path(rel: &str, suffix: &str) -> bool {
    match rel.strip_suffix(suffix) {
        Some(prefix) => prefix.is_empty() || prefix.ends_with('/'),
        None => false,
    }
}

/// 名字含 cache / session 的目录按缓存处理（含 `webpack-cache`、`sessions` 等变体）。
fn has_cache_word(name_lower: &str) -> bool {
    name_lower.contains("cache") || name_lower.contains("session")
}

/// GUID（8-4-4-4-12）或 ≥16 位纯十六进制串：缓存键、提交哈希这类不可读标识。
fn is_opaque_id(name_lower: &str) -> bool {
    let bytes = name_lower.as_bytes();
    let guid = bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && name_lower
            .chars()
            .filter(|c| *c != '-')
            .all(|c| c.is_ascii_hexdigit());
    guid || (bytes.len() >= 16 && bytes.iter().all(|b| b.is_ascii_hexdigit()))
}

/// 该目录是否需要整棵剪枝（`rel` 为相对盘根的小写路径）。
fn is_skipped_dir(rel: &str, name_lower: &str) -> bool {
    if SKIP_DIR_NAMES.iter().any(|s| *s == name_lower) {
        return true;
    }
    if has_cache_word(name_lower) || is_opaque_id(name_lower) {
        return true;
    }
    SKIP_REL_SUFFIXES.iter().any(|s| ends_with_path(rel, s))
}

/// 相对盘根的路径，统一成 `/` 分隔并转小写（.gitignore 规则按大小写不敏感匹配）。
fn relative_slash(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
        .unwrap_or_default()
}

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
    respect_gitignore: bool,
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
    let root_path = Path::new(&root);
    let started = Instant::now();
    let mut scanned = 0usize;
    let mut inserted = 0usize;
    let mut batch: Vec<BatchRow> = Vec::with_capacity(2048);
    let mut last_emit = Instant::now();
    let mut ignores = IgnoreStack::new();

    let mut walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter();

    while let Some(res) = walker.next() {
        if CANCEL_INDEX.load(Ordering::SeqCst) {
            let _ = flush_batch(&mut conn, drive, &mut batch);
            let _ = conn.execute(
                "DELETE FROM fs_entry_tmp WHERE drive = ?1",
                params![drive],
            );
            return Ok(ScanOutcome::Canceled { scanned });
        }
        let entry = match res {
            Ok(e) => e,
            Err(_) => continue,
        };
        let depth = entry.depth();
        // 离开更深的目录后，丢弃那一层装载的 .gitignore 规则
        ignores.leave_to(depth);
        if depth == 0 {
            if respect_gitignore {
                ignores.enter(0, entry.path(), "");
            }
            continue;
        }
        // 不跟随 junction / 符号链接，避免目录环与重复项。
        if entry.file_type().is_symlink() {
            continue;
        }
        let is_dir = entry.file_type().is_dir();
        let file_name = entry.file_name().to_string_lossy();
        if file_name.is_empty() {
            continue;
        }
        let name_lower = file_name.to_ascii_lowercase();
        // 目录要判缓存/系统路径，文件只在遵循 gitignore 时才需要相对路径
        let rel = if is_dir || respect_gitignore {
            relative_slash(entry.path(), root_path)
        } else {
            String::new()
        };
        if is_dir && is_skipped_dir(&rel, &name_lower) {
            walker.skip_current_dir();
            continue;
        }
        // 纯 GUID / 长哈希文件名（无扩展名的缓存键）同样没有检索价值
        if !is_dir && is_opaque_id(&name_lower) {
            continue;
        }
        if respect_gitignore && ignores.is_ignored(&rel, is_dir) {
            if is_dir {
                walker.skip_current_dir();
            }
            continue;
        }
        if is_dir && respect_gitignore {
            ignores.enter(depth, entry.path(), &rel);
        }

        let name = file_name.to_string();
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
        respect_gitignore,
    )?;

    Ok(ScanOutcome::Done { scanned, count })
}

// ============================ 命令：索引进度 / 管理 ============================

#[tauri::command]
pub async fn file_search_list_drives() -> Result<Vec<DriveInfo>, String> {
    let mut meta_map: std::collections::HashMap<String, (String, i64, i64, i64, bool)> =
        std::collections::HashMap::new();
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT drive, status, item_count, indexed_at, elapsed_ms, respect_gitignore
                 FROM fs_meta",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)? != 0,
                ))
            })
            .map_err(|e| e.to_string())?;
        for (drive, status, count, indexed_at, elapsed, respect) in rows.flatten() {
            meta_map.insert(drive, (status, count, indexed_at, elapsed, respect));
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
            let (status, item_count, indexed_at, elapsed_ms, respect_gitignore) = meta_map
                .get(&letter)
                .cloned()
                .unwrap_or_else(|| ("idle".to_string(), 0, 0, 0, false));
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
                respect_gitignore,
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
    respect_gitignore: Option<bool>,
) -> Result<(), String> {
    let drive = normalize_drive(&drive)?;
    let root = format!("{}\\", drive);
    if !Path::new(&root).exists() {
        return Err(format!("盘符不存在: {}", root));
    }
    let mode = mode.unwrap_or_else(|| "update".to_string());
    let respect_gitignore = respect_gitignore.unwrap_or(true);
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
        let result = scan_drive(&app_for_thread, &drive_for_thread, &mode, respect_gitignore);
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

/// 结果类型过滤：文件夹 / 普通文件，以及扩展名白名单（空 = 不限）。
///
/// 必须下推到 SQL：前端过滤会被 100 条上限先截断，反而搜不到目标类型。
fn push_type_filter(
    values: &mut Vec<Value>,
    conds: &mut Vec<String>,
    alias: &str,
    kind: &str,
    exts: &[String],
) {
    let col = |name: &str| {
        if alias.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", alias, name)
        }
    };
    match kind {
        "dir" => conds.push(format!("{} = 1", col("is_dir"))),
        "file" => conds.push(format!("{} = 0", col("is_dir"))),
        _ => {}
    }
    if !exts.is_empty() {
        let mut ors = Vec::new();
        for ext in exts {
            values.push(Value::Text(format!("%.{}", escape_like(ext))));
            ors.push(format!(
                "{} LIKE ?{} ESCAPE '\\'",
                col("name_lower"),
                values.len()
            ));
        }
        conds.push(format!("({})", ors.join(" OR ")));
    }
}

/// 输入解析结果：盘符 / 目录前缀 / 关键词。
///
/// 支持 `e pro gld 1234` 这类写法：首词是盘符，随后连续的词只要能在索引里
/// 逐级命中目录，就当作目录前缀，剩下的词才是关键词。
#[derive(Debug, Default, PartialEq)]
struct QueryScope {
    drive: Option<String>,
    /// 已解析出的目录，如 `E:\pro\gld`（不含结尾反斜杠）
    path_prefix: Option<String>,
    tokens: Vec<String>,
}

/// 首词是否形如盘符：`e`、`e:`、`e:\`。
fn token_drive(tok: &str) -> Option<String> {
    let t = tok.trim_end_matches(|c| c == '\\' || c == '/');
    let bytes = t.as_bytes();
    let letter = match bytes.len() {
        1 if bytes[0].is_ascii_alphabetic() => bytes[0],
        2 if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' => bytes[0],
        _ => return None,
    };
    Some(format!("{}:", (letter as char).to_ascii_uppercase()))
}

/// 取命名目录作为 `parent` 的直接子目录（大小写不敏感）。
fn find_child_dir(
    conn: &Connection,
    drive: &str,
    parent: &str,
    name: &str,
) -> Result<Option<String>, String> {
    let base = parent.trim_end_matches('\\');
    let pattern = format!("{}\\\\%", escape_like(base));
    let len = base.chars().count() + 1 + name.chars().count();
    conn.query_row(
        "SELECT path FROM fs_entry
         WHERE drive = ?1 AND is_dir = 1 AND name_lower = ?2
           AND path LIKE ?3 ESCAPE '\\' AND length(path) = ?4
         LIMIT 1",
        params![drive, name, pattern, len as i64],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("解析搜索目录失败: {}", e))
}

/// 把原始输入拆成盘符 + 目录前缀 + 关键词。
///
/// 只有拿到确定的盘符时才解析目录：否则 `pro` 既可能是目录名也可能是关键词，
/// 贸然吞掉会让普通搜索搜不到东西。目录解析至少留一个词作关键词。
fn resolve_scope(conn: &Connection, raw: &str, drives: &[String]) -> QueryScope {
    let parts: Vec<String> = raw
        .split_whitespace()
        .take(8)
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if parts.is_empty() {
        return QueryScope::default();
    }
    let (typed, start) = match token_drive(&parts[0]) {
        Some(d) => (Some(d), 1),
        None => (None, 0),
    };
    let drive = typed.clone().or_else(|| {
        if drives.len() == 1 {
            Some(drives[0].clone())
        } else {
            None
        }
    });
    let Some(drive) = drive else {
        return QueryScope {
            drive: None,
            path_prefix: None,
            tokens: parts,
        };
    };

    let mut prefix = format!("{}\\", drive);
    let mut consumed = 0usize;
    // 至少留一个词当关键词；`e pro gld` 最后一个词仍是关键词
    while start + consumed + 1 < parts.len() {
        let token = &parts[start + consumed];
        match find_child_dir(conn, &drive, &prefix, token) {
            Ok(Some(path)) => {
                prefix = path;
                consumed += 1;
            }
            Ok(None) => break,
            // 索引读取异常不阻塞搜索，按普通关键词处理
            Err(_) => break,
        }
    }
    let tokens = parts[start + consumed..].to_vec();
    let path_prefix = if consumed > 0 {
        Some(prefix.trim_end_matches('\\').to_string())
    } else {
        None
    };
    QueryScope {
        drive: Some(drive),
        path_prefix,
        tokens,
    }
}

fn search_impl(
    query: &str,
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
) -> Result<Vec<FileHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 500);

    with_conn(|conn| {
        let scope = resolve_scope(conn, q, drives);
        if scope.tokens.is_empty() {
            return Ok(Vec::new());
        }
        // 输入里写了盘符就以它为准（优先于下拉选择）
        let scoped_drives: Vec<String> = match &scope.drive {
            Some(d) => vec![d.clone()],
            None => drives.to_vec(),
        };
        // 排序用的「整体词」不包含盘符/目录，否则永远命不中前缀
        let q_lower = scope.tokens.join(" ");
        search_with_conn(
            conn,
            &q_lower,
            &scope.tokens,
            limit,
            &scoped_drives,
            kind,
            exts,
            scope.path_prefix.as_deref(),
        )
    })
}

fn search_with_conn(
    conn: &Connection,
    q: &str,
    tokens: &[String],
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
) -> Result<Vec<FileHit>, String> {
    // trigram 只索引 ≥3 字符的片段，短词无法命中全文索引，交给 LIKE。
    if tokens.iter().all(|t| t.chars().count() >= 3) {
        match fts_search(conn, q, tokens, limit, drives, kind, exts, path_prefix) {
            Ok(hits) => return Ok(hits),
            // 全文索引异常（缺失/损坏）时退回 LIKE：先保证搜得到，再谈搜得快。
            Err(err) => crate::exit_log!("[文件搜索] 全文检索失败，回退 LIKE: {}", err),
        }
    }
    like_search(conn, q, tokens, limit, drives, kind, exts, path_prefix)
}

/// 目录范围下推到 SQL：只要该前缀下（含更深层级）的条目。
fn push_path_filter(
    values: &mut Vec<Value>,
    conds: &mut Vec<String>,
    alias: &str,
    path_prefix: Option<&str>,
) {
    let Some(prefix) = path_prefix else {
        return;
    };
    let base = prefix.trim_end_matches('\\');
    if base.is_empty() {
        return;
    }
    let col = |name: &str| {
        if alias.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", alias, name)
        }
    };
    values.push(Value::Text(format!("{}\\\\%", escape_like(base))));
    conds.push(format!(
        "{} LIKE ?{} ESCAPE '\\'",
        col("path"),
        values.len()
    ));
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
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
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
    push_path_filter(&mut values, &mut conds, "e", path_prefix);
    push_type_filter(&mut values, &mut conds, "e", kind, exts);

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
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
) -> Result<Vec<FileHit>, String> {
    let mut rows = prefix_search(conn, q, limit, drives, kind, exts, path_prefix)?;
    if rows.len() < limit {
        let exclude: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
        rows.extend(infix_search(
            conn,
            tokens,
            limit - rows.len(),
            drives,
            kind,
            exts,
            path_prefix,
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
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
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
    push_path_filter(&mut values, &mut conds, "", path_prefix);
    push_type_filter(&mut values, &mut conds, "", kind, exts);
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
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
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
    push_path_filter(&mut values, &mut conds, "", path_prefix);
    push_type_filter(&mut values, &mut conds, "", kind, exts);
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
    kind: Option<String>,
    exts: Option<Vec<String>>,
) -> Result<Vec<FileHit>, String> {
    let normalized: Vec<String> = drives
        .unwrap_or_default()
        .iter()
        .filter_map(|d| normalize_drive(d).ok())
        .collect();
    let limit = limit.unwrap_or(100);
    let kind = match kind.as_deref() {
        Some("dir") => "dir",
        Some("file") => "file",
        _ => "all",
    }
    .to_string();
    let exts: Vec<String> = exts
        .unwrap_or_default()
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 16 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .take(24)
        .collect();
    tauri::async_runtime::spawn_blocking(move || {
        search_impl(&query, limit, &normalized, &kind, &exts)
    })
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
        escape_like, fts_expr, init_schema, is_opaque_id, is_skipped_dir, normalize_drive,
        resolve_scope, search_with_conn, token_drive, FileHit, QueryScope,
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
        for (path, name, is_dir) in [
            ("C:\\docs", "docs", 1),
            ("C:\\docs\\report.docx", "report.docx", 0),
            ("C:\\docs\\my_report_2024.docx", "my_report_2024.docx", 0),
            ("C:\\apps\\report.exe", "report.exe", 0),
            ("C:\\tmp\\ab.txt", "ab.txt", 0),
        ] {
            conn.execute(
                "INSERT INTO fs_entry (drive, path, name, name_lower, is_dir, size, modified)
                 VALUES ('C:', ?1, ?2, ?3, ?4, 0, 0)",
                params![path, name, name.to_lowercase(), is_dir],
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
        let hits = search_with_conn(&conn, "report", &tokens, 10, &[], "all", &[], None).unwrap();
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
        let hits = search_with_conn(&conn, "ab", &tokens, 10, &[], "all", &[], None).unwrap();
        assert_eq!(names(&hits), vec!["ab.txt".to_string()]);
    }

    #[test]
    fn multi_token_requires_all_terms() {
        let conn = seeded_db();
        let tokens = vec!["report".to_string(), "2024".to_string()];
        let hits =
            search_with_conn(&conn, "report 2024", &tokens, 10, &[], "all", &[], None).unwrap();
        assert_eq!(names(&hits), vec!["my_report_2024.docx".to_string()]);
    }

    #[test]
    fn drive_filter_limits_results() {
        let conn = seeded_db();
        let tokens = vec!["report".to_string()];
        let hits =
            search_with_conn(&conn, "report", &tokens, 10, &["D:".to_string()], "all", &[], None)
                .unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn dir_only_filter_excludes_files() {
        let conn = seeded_db();
        let tokens = vec!["docs".to_string()];
        let hits = search_with_conn(&conn, "docs", &tokens, 10, &[], "dir", &[], None).unwrap();
        assert_eq!(names(&hits), vec!["docs".to_string()]);
    }

    #[test]
    fn ext_filter_matches_suffix_only() {
        let conn = seeded_db();
        let tokens = vec!["report".to_string()];
        let hits =
            search_with_conn(&conn, "report", &tokens, 10, &[], "file", &["docx".to_string()], None)
                .unwrap();
        let got = names(&hits);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|n| n.ends_with(".docx")));
    }

    #[test]
    fn ext_filter_applies_to_like_fallback() {
        let conn = seeded_db();
        let tokens = vec!["ab".to_string()];
        let hits =
            search_with_conn(&conn, "ab", &tokens, 10, &[], "file", &["txt".to_string()], None)
                .unwrap();
        assert_eq!(names(&hits), vec!["ab.txt".to_string()]);
    }

    fn insert_entry(conn: &Connection, drive: &str, path: &str, name: &str, is_dir: bool) {
        conn.execute(
            "INSERT INTO fs_entry (drive, path, name, name_lower, is_dir, size, modified)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
            params![drive, path, name, name.to_lowercase(), is_dir],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO fs_entry_fts (rowid, name) VALUES (?1, ?2)",
            params![id, name],
        )
        .unwrap();
    }

    /// 目录范围测试用：E:/D: 各有一组同名目录，确保前缀比较不是全局命中。
    fn seeded_scope_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        for (drive, path, name, is_dir) in [
            ("E:", "E:\\pro", "pro", true),
            ("E:", "E:\\pro\\gld", "gld", true),
            ("E:", "E:\\pro\\gld\\a1234.txt", "a1234.txt", false),
            ("E:", "E:\\pro\\gld\\b.txt", "b.txt", false),
            ("E:", "E:\\pro\\other", "other", true),
            ("E:", "E:\\pro\\other\\c1234.txt", "c1234.txt", false),
            ("E:", "E:\\work\\gld", "gld", true),
            ("E:", "E:\\work\\gld\\d1234.txt", "d1234.txt", false),
            ("D:", "D:\\pro\\gld", "gld", true),
            ("D:", "D:\\pro\\gld\\e1234.txt", "e1234.txt", false),
        ] {
            insert_entry(&conn, drive, path, name, is_dir);
        }
        conn
    }

    #[test]
    fn drive_token_is_recognized() {
        assert_eq!(token_drive("e").as_deref(), Some("E:"));
        assert_eq!(token_drive("E:").as_deref(), Some("E:"));
        assert_eq!(token_drive("e:\\").as_deref(), Some("E:"));
        assert_eq!(token_drive("c:/").as_deref(), Some("C:"));
        assert!(token_drive("pro").is_none());
        assert!(token_drive("1").is_none());
    }

    #[test]
    fn scope_parses_drive_directories_and_keyword() {
        let conn = seeded_scope_db();
        // e pro gld 1234 → E:\pro\gld 下搜索 1234
        let scope = resolve_scope(&conn, "e pro gld 1234", &[]);
        assert_eq!(
            scope,
            QueryScope {
                drive: Some("E:".to_string()),
                path_prefix: Some("E:\\pro\\gld".to_string()),
                tokens: vec!["1234".to_string()],
            }
        );
        // 大小写与 `E:` 写法都等价
        let scope = resolve_scope(&conn, "E: PRO GLD 1234", &[]);
        assert_eq!(scope.drive.as_deref(), Some("E:"));
        assert_eq!(scope.path_prefix.as_deref(), Some("E:\\pro\\gld"));
        assert_eq!(scope.tokens, vec!["1234".to_string()]);
    }

    #[test]
    fn scope_keeps_unknown_directory_as_keyword() {
        let conn = seeded_scope_db();
        // 不存在的目录不吞词，退回普通多关键词搜索
        let scope = resolve_scope(&conn, "e pro nope 1234", &[]);
        assert_eq!(scope.path_prefix.as_deref(), Some("E:\\pro"));
        assert_eq!(
            scope.tokens,
            vec!["nope".to_string(), "1234".to_string()]
        );
    }

    #[test]
    fn scope_uses_drive_dropdown_when_typed_drive_absent() {
        let conn = seeded_scope_db();
        let scope = resolve_scope(&conn, "pro gld 1234", &["E:".to_string()]);
        assert_eq!(scope.path_prefix.as_deref(), Some("E:\\pro\\gld"));
        assert_eq!(scope.tokens, vec!["1234".to_string()]);
        // 没有盘符也没选盘时，词全部按关键词处理
        let scope = resolve_scope(&conn, "pro gld 1234", &[]);
        assert_eq!(scope.drive, None);
        assert_eq!(scope.path_prefix, None);
        assert_eq!(scope.tokens.len(), 3);
    }

    #[test]
    fn path_prefix_limits_both_fts_and_like_paths() {
        let conn = seeded_scope_db();
        // 全文索引路径：同名文件在 E:\pro\other、E:\work\gld、D: 下都不应出现
        let hits = search_with_conn(
            &conn,
            "1234",
            &["1234".to_string()],
            10,
            &["E:".to_string()],
            "all",
            &[],
            Some("E:\\pro\\gld"),
        )
        .unwrap();
        assert_eq!(names(&hits), vec!["a1234.txt".to_string()]);

        // 短词退化 LIKE 路径同样受目录范围限制
        let hits = search_with_conn(
            &conn,
            "b",
            &["b".to_string()],
            10,
            &["E:".to_string()],
            "all",
            &[],
            Some("E:\\pro\\gld"),
        )
        .unwrap();
        assert_eq!(names(&hits), vec!["b.txt".to_string()]);
    }

    #[test]
    fn cache_dirs_are_pruned() {
        assert!(is_skipped_dir("proj/node_modules", "node_modules"));
        assert!(is_skipped_dir("users/me/appdata/local/temp", "temp"));
        assert!(is_skipped_dir("users/me/appdata/local/yarn/cache", "cache"));
        assert!(is_skipped_dir("windows/winsxs", "winsxs"));
        // 同名但不在缓存位置的目录不误伤
        assert!(!is_skipped_dir("proj/temp", "temp"));
        assert!(!is_skipped_dir("proj/build", "build"));
        // 目录边界校验：`attempt` 不能被 `temp` 后缀命中
        assert!(!is_skipped_dir("proj/attempt", "attempt"));
        // 名字含 cache / session 一律跳过
        assert!(is_skipped_dir("proj/src/cache", "cache"));
        assert!(is_skipped_dir("proj/src/sessions", "sessions"));
        assert!(is_skipped_dir("proj/webpack-cache", "webpack-cache"));
        // GUID / 长哈希目录名
        assert!(is_skipped_dir(
            "files/fromc/codebuddyextension/data/1eacae31-92db-4f15-8c46-19edd7ca5276",
            "1eacae31-92db-4f15-8c46-19edd7ca5276"
        ));
        assert!(is_skipped_dir("proj/.dart_tool", ".dart_tool"));
        assert!(is_skipped_dir(
            "users/me/appdata/local/jetbrains",
            "jetbrains"
        ));
        // 普通目录不误伤
        assert!(!is_skipped_dir("proj/src/utils", "utils"));
        assert!(!is_skipped_dir("proj/docs/history", "history"));
        // 带扩展名的哈希文件名不是不透明 ID
        assert!(!is_opaque_id("a1b2c3d4e5f6a7b8.js"));
        assert!(is_opaque_id("a3f9c81b7e2d4f60"));
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
        assert!(
            search_with_conn(&conn, "report", &tokens, 10, &[], "all", &[], None)
                .unwrap()
                .is_empty()
        );
    }
}
