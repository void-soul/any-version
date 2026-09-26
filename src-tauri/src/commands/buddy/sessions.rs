//! Buddy 会话管理：列出本机 WorkBuddy / CodeBuddy CN 的 AI 会话。
//!
//! 复刻自 cockpit-tools `codebuddy_session.rs`：
//! - WorkBuddy：会话存在 `~/.workbuddy/workbuddy.db`（单库，按 user_id 分区）
//! - CodeBuddy CN：会话存在 `%APPDATA%/CodeBuddy CN/codebuddy-sessions.vscdb`
//!   （ItemTable 中 key 以 `session:` 开头的 JSON）

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models::BuddyPlatform;

/// 单个会话位置（来源实例）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySessionLocation {
    pub instance_id: String,
    pub instance_name: String,
}

/// 去重后的会话记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySessionRecord {
    pub conversation_id: String,
    pub title: String,
    pub cwd: String,
    pub user_id: String,
    pub status: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    /// 活动时间：`last_activity_at` → `updated_at` → `created_at` 依次回退（对齐参考实现的 COALESCE 排序）
    #[serde(default)]
    pub last_activity_at: Option<i64>,
    pub is_playground: bool,
    /// 本机已找不到该会话的正文（扩展目录 index.json 无登记或正文目录缺失）
    #[serde(default)]
    pub content_missing: bool,
    pub locations: Vec<BuddySessionLocation>,
}

#[derive(Debug, Clone, Default)]
pub struct BuddySessionFilter {
    pub keyword: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone)]
struct RawSession {
    conversation_id: String,
    title: String,
    cwd: String,
    user_id: String,
    status: String,
    created_at: Option<i64>,
    updated_at: Option<i64>,
    last_activity_at: Option<i64>,
    is_deleted: bool,
    is_playground: bool,
}

/// 会话活动时间：取 `last_activity_at` / `updated_at` / `created_at` 中**最新**者。
///
/// 真机数据（WorkBuddy 5.x）显示 `updated_at` 恒 ≥ `last_activity_at`
/// （客户端刷新会 bump updated_at），取最新者才符合「最近活跃」的直觉；
/// 缺失列（老库）自动退化为 updated_at / created_at。
fn activity_at(raw: &RawSession) -> Option<i64> {
    [raw.last_activity_at, raw.updated_at, raw.created_at]
        .into_iter()
        .flatten()
        .max()
}

/// 表是否含某列（老版本 `workbuddy.db` 没有 `last_activity_at`，直接写进 SQL 会报错）。
fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return false;
    };
    while let Ok(Some(row)) = rows.next() {
        if row.get::<_, String>(1).map(|name| name == column).unwrap_or(false) {
            return true;
        }
    }
    false
}

/// WorkBuddy 会话库路径：`~/.workbuddy/workbuddy.db`
fn workbuddy_sessions_db_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home.join(".workbuddy").join("workbuddy.db"))
}

/// CodeBuddy CN 会话库路径：`{data_dir}/codebuddy-sessions.vscdb`
fn codebuddy_cn_sessions_db_path() -> Result<PathBuf, String> {
    let data_dir = super::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())?;
    Ok(data_dir.join("codebuddy-sessions.vscdb"))
}

/// 读取 WorkBuddy `workbuddy.db` 的 sessions 表
fn read_sessions_from_workbuddy_db(db_path: &Path) -> Vec<RawSession> {
    if !db_path.exists() {
        return Vec::new();
    }

    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[BuddySession] 打开 WorkBuddy db 失败 {}: {}", db_path.display(), e);
            return Vec::new();
        }
    };

    // 老版本库没有 last_activity_at（5.x 才有），缺列时用 NULL 占位，排序退化为 updated_at/created_at
    let has_last_activity = table_has_column(&conn, "sessions", "last_activity_at");
    let activity_column = if has_last_activity {
        "last_activity_at"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, cwd, user_id, title, status, created_at, updated_at, deleted_at, is_playground, {} \
         FROM sessions",
        activity_column
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[BuddySession] 准备 WorkBuddy 查询失败 {}: {}", db_path.display(), e);
            return Vec::new();
        }
    };

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, i64>(8).unwrap_or(0),
            row.get::<_, Option<i64>>(9)?,
        ))
    });

    let mut sessions = Vec::new();
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let (
                id,
                cwd,
                user_id,
                title,
                status,
                created_at,
                updated_at,
                deleted_at,
                is_play,
                last_activity_at,
            ) = row;
            sessions.push(RawSession {
                conversation_id: id,
                title: title.unwrap_or_default(),
                cwd: cwd.unwrap_or_default(),
                user_id: user_id.unwrap_or_default(),
                status: status.unwrap_or_default(),
                created_at,
                updated_at,
                last_activity_at,
                is_deleted: deleted_at.is_some(),
                is_playground: is_play != 0,
            });
        }
    }

    sessions
}

/// 收集扩展数据目录中「本机仍有正文」的会话 id。
///
/// 判定与参考实现一致：工作区 `index.json` 登记了该会话，**且** `<workspace>/<conversationId>/` 目录存在。
/// 返回 `None` 表示本机没有可识别的旧版目录布局（5.x 会话只落 DB、或全新装机），
/// 此时调用方跳过「正文缺失」标记，避免把正常会话全标成缺失。
pub(crate) fn collect_local_conversation_ids_from_roots(
    roots: &[PathBuf],
    uid: &str,
) -> Option<HashSet<String>> {
    if super::session_transfer::codebuddy::validate_uid(uid).is_err() {
        return None;
    }
    let mut ids = HashSet::new();
    let mut layout_found = false;
    for root in roots {
        let account_outer = root.join(uid);
        let Ok(ide_entries) = std::fs::read_dir(&account_outer) else {
            continue;
        };
        for ide_entry in ide_entries.flatten() {
            let Ok(metadata) = std::fs::symlink_metadata(ide_entry.path()) else {
                continue;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let history_root = ide_entry.path().join(uid).join("history");
            if !history_root.is_dir() {
                continue;
            }
            layout_found = true;
            let Ok(workspace_entries) = std::fs::read_dir(&history_root) else {
                continue;
            };
            for workspace_entry in workspace_entries.flatten() {
                let Ok(metadata) = std::fs::symlink_metadata(workspace_entry.path()) else {
                    continue;
                };
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    continue;
                }
                let workspace_path = workspace_entry.path();
                let Ok(index) =
                    super::session_transfer::codebuddy::read_workspace_index(&workspace_path.join("index.json"))
                else {
                    continue;
                };
                let Some(conversations) = index.get("conversations").and_then(Value::as_array) else {
                    continue;
                };
                for conversation in conversations {
                    let Some(id) = conversation.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if workspace_path.join(id).is_dir() {
                        ids.insert(id.to_string());
                    }
                }
            }
        }
    }
    layout_found.then_some(ids)
}

/// 标注哪些会话在本机已经没有正文（`contentMissing`）。`local` 为 `None` 时不动任何记录。
fn apply_content_probe(records: &mut [BuddySessionRecord], local: Option<&HashSet<String>>) {
    let Some(local) = local else {
        return;
    };
    for record in records.iter_mut() {
        record.content_missing = !local.contains(&record.conversation_id);
    }
}

/// 读取 vscdb 中的会话 JSON（ItemTable key 以 `session:` 开头）
fn read_sessions_from_db(db_path: &Path) -> Vec<RawSession> {
    if !db_path.exists() {
        return Vec::new();
    }

    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[BuddySession] 打开 {} 失败: {}", db_path.display(), e);
            return Vec::new();
        }
    };

    let mut stmt = match conn.prepare("SELECT value FROM ItemTable WHERE key LIKE 'session:%'") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[BuddySession] 准备查询失败 {}: {}", db_path.display(), e);
            return Vec::new();
        }
    };

    let rows = stmt.query_map([], |row| {
        let value: String = row.get(0)?;
        Ok(value)
    });

    let mut sessions = Vec::new();
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            if let Some(s) = parse_raw_session(&row) {
                sessions.push(s);
            }
        }
    }

    sessions
}

fn parse_raw_session(json_str: &str) -> Option<RawSession> {
    let v: Value = serde_json::from_str(json_str).ok()?;

    let conversation_id = v.get("conversationId")?.as_str()?.to_string();
    let title = v.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let cwd = v.get("cwd").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let user_id = v.get("userId").and_then(|u| u.as_str()).unwrap_or("").to_string();
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("Unknown").to_string();
    let created_at = v.get("createdAt").and_then(|t| t.as_i64());
    let updated_at = v.get("updatedAt").and_then(|t| t.as_i64());
    let last_activity_at = v.get("lastActivityAt").and_then(|t| t.as_i64());
    let is_deleted = v.get("deletedAt").and_then(|t| t.as_i64()).is_some();
    let is_playground = v
        .get("isPlayground")
        .and_then(|p| p.as_bool())
        .unwrap_or(false);

    Some(RawSession {
        conversation_id,
        title,
        cwd,
        user_id,
        status,
        created_at,
        updated_at,
        last_activity_at,
        is_deleted,
        is_playground,
    })
}

fn aggregate_sessions(
    raw_sessions: Vec<RawSession>,
    filter: &BuddySessionFilter,
    instance_id: &str,
    instance_name: &str,
) -> Vec<BuddySessionRecord> {
    let mut aggregated: HashMap<String, BuddySessionRecord> = HashMap::new();

    let keyword = filter
        .keyword
        .as_deref()
        .map(|k| k.to_lowercase())
        .unwrap_or_default();
    let status_filter = filter.status.as_deref().unwrap_or("");

    for raw in raw_sessions {
        if raw.is_deleted {
            continue;
        }
        if !status_filter.is_empty() && raw.status != status_filter {
            continue;
        }
        if !keyword.is_empty() {
            let title_lower = raw.title.to_lowercase();
            let cwd_lower = raw.cwd.to_lowercase();
            if !title_lower.contains(&keyword) && !cwd_lower.contains(&keyword) {
                continue;
            }
        }

        let conversation_id = raw.conversation_id.clone();
        let location = BuddySessionLocation {
            instance_id: instance_id.to_string(),
            instance_name: instance_name.to_string(),
        };

        aggregated
            .entry(conversation_id.clone())
            .and_modify(|existing| {
                if let Some(new_updated) = raw.updated_at {
                    if existing.updated_at.map_or(true, |old| new_updated > old) {
                        existing.updated_at = Some(new_updated);
                    }
                }
                if let Some(new_activity) = activity_at(&raw) {
                    if existing.last_activity_at.map_or(true, |old| new_activity > old) {
                        existing.last_activity_at = Some(new_activity);
                    }
                }
                if !existing
                    .locations
                    .iter()
                    .any(|l| l.instance_id == instance_id)
                {
                    existing.locations.push(location.clone());
                }
            })
            .or_insert_with(|| {
                // 先算出活动时间：下面的结构体字面量会移动 raw 的字段
                let raw_activity = activity_at(&raw);
                BuddySessionRecord {
                conversation_id: raw.conversation_id,
                title: raw.title,
                cwd: raw.cwd,
                user_id: raw.user_id,
                status: raw.status,
                created_at: raw.created_at,
                updated_at: raw.updated_at,
                last_activity_at: raw_activity,
                is_playground: raw.is_playground,
                content_missing: false,
                locations: vec![location],
                }
            });
    }

    let mut records: Vec<BuddySessionRecord> = aggregated.into_values().collect();
    // 排序：活动时间倒序，其次 created_at 倒序（对齐参考实现的 COALESCE 排序）
    records.sort_by(|a, b| {
        b.last_activity_at
            .unwrap_or(0)
            .cmp(&a.last_activity_at.unwrap_or(0))
            .then_with(|| b.created_at.unwrap_or(0).cmp(&a.created_at.unwrap_or(0)))
    });
    records
}

/// 列出会话（platform: "workbuddy" | "codebuddy-cn"）
pub fn list_sessions(
    platform: &str,
    filter: &BuddySessionFilter,
) -> Result<Vec<BuddySessionRecord>, String> {
    match platform {
        "workbuddy" => {
            let db_path = workbuddy_sessions_db_path()?;
            let mut records = aggregate_sessions(
                read_sessions_from_workbuddy_db(&db_path),
                filter,
                "workbuddy",
                "WorkBuddy",
            );
            // 正文存在性探测：仅在能定位到旧版扩展目录布局时生效（5.x 只留 DB 时跳过）
            let local = current_platform_uid(BuddyPlatform::Workbuddy).and_then(|uid| {
                let roots: Vec<PathBuf> =
                    super::session_transfer::workbuddy::select_extension_data_roots(&uid)
                        .ok()?
                        .into_iter()
                        .map(|(_, dir)| dir)
                        .collect();
                collect_local_conversation_ids_from_roots(&roots, &uid)
            });
            apply_content_probe(&mut records, local.as_ref());
            Ok(records)
        }
        "codebuddy-cn" | "codebuddy_cn" => {
            let db_path = codebuddy_cn_sessions_db_path()?;
            let mut records = aggregate_sessions(
                read_sessions_from_db(&db_path),
                filter,
                "default",
                "CodeBuddy CN",
            );
            let local = current_platform_uid(BuddyPlatform::CodebuddyCn).and_then(|uid| {
                let root = super::session_transfer::codebuddy::codebuddy_extension_data_dir().ok()?;
                collect_local_conversation_ids_from_roots(&[root], &uid)
            });
            apply_content_probe(&mut records, local.as_ref());
            Ok(records)
        }
        other => Err(format!("未知平台: {}", other)),
    }
}

// ─── 删除会话（数据库记录 + 本地文件） ───

/// 删除会话的结果报告。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySessionDeleteReport {
    /// 数据库中删除的记录数（vscdb 键 / sessions 行）
    pub db_deleted: usize,
    /// 删除的 history 会话目录数
    pub history_dirs_removed: usize,
    /// 删除的辅助目录数（check-point 等 5 类）
    pub auxiliary_dirs_removed: usize,
    /// 非致命错误（如文件被客户端占用），不中断整体删除
    pub errors: Vec<String>,
}

/// 删除指定会话：数据库记录 + 本地会话文件。
///
/// 文件部分按当前登录账号的 uid 定位（`<uid>/<IDE>/<uid>/history/<workspace>/`），
/// 包括：工作区 index.json 中的会话条目（current 指向被删会话时一并清空）、
/// 会话目录本体与 5 类辅助目录。客户端正在运行时也可删除
/// （SQLite WAL 允许并发写）；被占用的文件记入 errors，不中断整体流程。
pub fn delete_sessions(
    platform: &str,
    conversation_ids: &[String],
) -> Result<BuddySessionDeleteReport, String> {
    let ids: Vec<String> = conversation_ids
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if ids.is_empty() {
        return Err("未指定要删除的会话".to_string());
    }
    for id in &ids {
        super::session_transfer::codebuddy::validate_conversation_id(id)?;
    }
    match platform {
        "workbuddy" => delete_workbuddy_sessions(&ids),
        "codebuddy-cn" | "codebuddy_cn" => delete_codebuddy_sessions(&ids),
        other => Err(format!("未知平台: {}", other)),
    }
}

/// 当前登录账号的 uid（会话文件按 uid 落盘）。
fn current_platform_uid(platform: super::models::BuddyPlatform) -> Option<String> {
    let accounts = super::store::list_accounts(platform);
    let current_id = super::store::get_current_account_id(platform).or_else(|| match platform {
        super::models::BuddyPlatform::Workbuddy => {
            super::workbuddy::resolve_current_account_id(platform, &accounts)
        }
        super::models::BuddyPlatform::CodebuddyCn => {
            super::codebuddy_cn::resolve_current_account_id(&accounts)
        }
    })?;
    accounts
        .iter()
        .find(|a| a.id == current_id)
        .and_then(|a| a.uid.clone())
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
}

fn delete_codebuddy_sessions(ids: &[String]) -> Result<BuddySessionDeleteReport, String> {
    let user_data_dir = super::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())?;
    let uid = current_platform_uid(super::models::BuddyPlatform::CodebuddyCn)
        .ok_or_else(|| "无法确定当前登录的 CodeBuddy CN 账号 uid".to_string())?;
    let extension_data_dir =
        super::session_transfer::codebuddy::codebuddy_extension_data_dir().ok();
    let mut report = BuddySessionDeleteReport::default();
    delete_codebuddy_sessions_at(&user_data_dir, extension_data_dir.as_deref(), &uid, ids, &mut report);
    Ok(report)
}

/// CN 删除核心（路径参数化以便测试）。
fn delete_codebuddy_sessions_at(
    user_data_dir: &Path,
    extension_data_dir: Option<&Path>,
    uid: &str,
    ids: &[String],
    report: &mut BuddySessionDeleteReport,
) {
    let db_path = user_data_dir.join("codebuddy-sessions.vscdb");
    if db_path.is_file() {
        delete_vscdb_session_keys(&db_path, ids, report);
    }
    if let Some(extension_data_dir) = extension_data_dir {
        delete_session_files_in_extension(extension_data_dir, uid, ids, report);
    }
}

fn delete_workbuddy_sessions(ids: &[String]) -> Result<BuddySessionDeleteReport, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let uid = current_platform_uid(super::models::BuddyPlatform::Workbuddy);
    let mut report = BuddySessionDeleteReport::default();

    // 1) WorkBuddy 5.x 主存储：workbuddy.db 的 sessions 行
    let db_path = home.join(".workbuddy").join("workbuddy.db");
    if db_path.is_file() {
        delete_workbuddy_db_rows(&db_path, ids, &mut report);
    }

    // 2) 旧版 per-uid 扩展目录（5.x 通常不存在，仅兼容旧版落盘结构）
    if let Some(uid) = uid.as_deref() {
        match super::session_transfer::workbuddy::select_extension_data_roots(uid) {
            Ok(roots) => {
                for (_, extension_data_dir) in roots {
                    delete_session_files_in_extension(&extension_data_dir, uid, ids, &mut report);
                }
            }
            Err(e) => report.errors.push(e),
        }
    }
    Ok(report)
}

/// vscdb：删除 `session:<id>` 键（可写事务 + busy_timeout，客户端运行时可并发）。
fn delete_vscdb_session_keys(db_path: &Path, ids: &[String], report: &mut BuddySessionDeleteReport) {
    if let Err(e) = super::session_transfer::codebuddy::reject_symlink_if_exists(db_path) {
        report.errors.push(e);
        return;
    }
    let mut conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!("打开会话数据库失败: {}", e));
            return;
        }
    };
    if let Err(e) = conn.busy_timeout(Duration::from_secs(5)) {
        report.errors.push(format!("设置数据库超时失败: {}", e));
        return;
    }
    let transaction = match conn.transaction() {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(format!("开启数据库事务失败: {}", e));
            return;
        }
    };
    for id in ids {
        let key = format!("session:{}", id);
        match transaction.execute(
            "DELETE FROM ItemTable WHERE key = ?1",
            rusqlite::params![key],
        ) {
            Ok(n) => report.db_deleted += n,
            Err(e) => report.errors.push(format!("删除会话记录失败 {}: {}", id, e)),
        }
    }
    if let Err(e) = transaction.commit() {
        report.errors.push(format!("提交数据库删除失败: {}", e));
    }
}

/// workbuddy.db：删除 sessions 表行（5.x 主存储）。
fn delete_workbuddy_db_rows(db_path: &Path, ids: &[String], report: &mut BuddySessionDeleteReport) {
    if let Err(e) = super::session_transfer::workbuddy::reject_symlink_if_exists(db_path) {
        report.errors.push(e);
        return;
    }
    let mut conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!("打开 WorkBuddy 数据库失败: {}", e));
            return;
        }
    };
    if let Err(e) = conn.busy_timeout(Duration::from_secs(5)) {
        report.errors.push(format!("设置数据库超时失败: {}", e));
        return;
    }
    let has_sessions_table: bool = match conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions')",
        [],
        |row| row.get(0),
    ) {
        Ok(v) => v,
        Err(e) => {
            report.errors.push(format!("检查数据库结构失败: {}", e));
            return;
        }
    };
    if !has_sessions_table {
        return;
    }
    let transaction = match conn.transaction() {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(format!("开启数据库事务失败: {}", e));
            return;
        }
    };
    for id in ids {
        match transaction.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id]) {
            Ok(n) => report.db_deleted += n,
            Err(e) => report.errors.push(format!("删除会话记录失败 {}: {}", id, e)),
        }
    }
    if let Err(e) = transaction.commit() {
        report.errors.push(format!("提交数据库删除失败: {}", e));
    }
}

/// 在扩展数据目录 `<uid>/<IDE>/<uid>/` 中查找并删除会话文件：
/// 命中工作区的 index.json 会话条目、会话目录本体与 5 类辅助目录。
fn delete_session_files_in_extension(
    extension_data_dir: &Path,
    uid: &str,
    ids: &[String],
    report: &mut BuddySessionDeleteReport,
) {
    use super::session_transfer::codebuddy as transfer;
    if transfer::validate_uid(uid).is_err() {
        report.errors.push(format!("uid 不安全，跳过文件删除: {}", uid));
        return;
    }
    let account_outer = extension_data_dir.join(uid);
    if !account_outer.is_dir() {
        return;
    }
    let Ok(ide_entries) = std::fs::read_dir(&account_outer) else {
        return;
    };
    for ide_entry in ide_entries.flatten() {
        let Ok(metadata) = std::fs::symlink_metadata(ide_entry.path()) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let account_root = ide_entry.path().join(uid);
        let history_root = account_root.join("history");
        if !history_root.is_dir() {
            continue;
        }
        let Ok(workspace_entries) = std::fs::read_dir(&history_root) else {
            continue;
        };
        for workspace_entry in workspace_entries.flatten() {
            let Ok(metadata) = std::fs::symlink_metadata(workspace_entry.path()) else {
                continue;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let workspace_path = workspace_entry.path();
            let index_path = workspace_path.join("index.json");
            let Ok(index) = transfer::read_workspace_index(&index_path) else {
                continue;
            };
            let Some(conversations) = index.get("conversations").and_then(Value::as_array) else {
                continue;
            };
            let hit = ids.iter().any(|id| {
                conversations
                    .iter()
                    .any(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str()))
            });
            if !hit {
                continue;
            }

            // index.json：移除被删会话条目；current 指向被删会话时清空
            let mut new_index = index.clone();
            if let Some(array) = new_index
                .get_mut("conversations")
                .and_then(Value::as_array_mut)
            {
                array.retain(|c| {
                    let cid = c.get("id").and_then(Value::as_str).unwrap_or_default();
                    !ids.iter().any(|id| id == cid)
                });
            }
            if let Some(object) = new_index.as_object_mut() {
                let current = object
                    .get("current")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if !current.is_empty() && ids.iter().any(|id| id == &current) {
                    object.insert("current".to_string(), Value::String(String::new()));
                }
            }
            let serialized = match serde_json::to_string_pretty(&new_index)
                .map_err(|e| format!("序列化工作区索引失败: {}", e))
            {
                Ok(s) => s,
                Err(e) => {
                    report.errors.push(e);
                    continue;
                }
            };
            if let Err(e) = super::store::write_atomic(&index_path, &serialized) {
                report.errors.push(e);
                continue;
            }

            // 删除会话目录与辅助目录
            for id in ids {
                let conv_dir = workspace_path.join(id);
                if conv_dir.is_dir() {
                    match std::fs::remove_dir_all(&conv_dir) {
                        Ok(()) => report.history_dirs_removed += 1,
                        Err(e) => report
                            .errors
                            .push(format!("删除会话目录失败 {}: {}", conv_dir.display(), e)),
                    }
                }
                for kind in transfer::AUXILIARY_KINDS {
                    let aux_dir = account_root
                        .join(kind)
                        .join(workspace_entry.file_name())
                        .join(id);
                    if aux_dir.is_dir() {
                        match std::fs::remove_dir_all(&aux_dir) {
                            Ok(()) => report.auxiliary_dirs_removed += 1,
                            Err(e) => report.errors.push(format!(
                                "删除辅助目录失败 {}: {}",
                                aux_dir.display(),
                                e
                            )),
                        }
                    }
                }
            }
        }
    }
}

// ─── 会话分叉 ───

/// 分叉结果：新会话 + 正文复制情况（正文没拷到要如实告诉前端，避免开出个空会话还以为成功了）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySessionForkReport {
    pub conversation_id: String,
    pub title: String,
    /// 成功复制的正文文件数（WorkBuddy 为 jsonl / artifact-index）
    pub files_copied: usize,
    /// 正文一个都没拷到：新会话只有库记录
    pub content_missing: bool,
    /// 非致命问题（个别文件被占用等），不中断分叉
    pub errors: Vec<String>,
}

/// 分叉会话：复制会话正文 + 用新 id 在会话库里注册一条记录，原会话原样保留。
///
/// - WorkBuddy：`~/.workbuddy/projects/<hash>/<id>.jsonl` 是对话正文（复制时把文件里出现的
///   旧会话 id 全部换成新 id，否则库里两条记录指向同一份 sessionId，打开/续聊会串台）；
///   `artifact-index/<id>.json` 一并复制。**不复制** `workspace/sessions`、`tasks`、
///   `file-history`——那是工作区快照与文件历史，实测单个会话可达 700MB，分叉不该背这个体积。
/// - CodeBuddy CN：会话索引在 vscdb 的 `session:<id>` 键，正文在扩展目录的
///   `<workspace>/<id>/`，两边都要复制一份。
pub fn fork_session(
    platform: &str,
    conversation_id: &str,
    title: Option<String>,
) -> Result<BuddySessionForkReport, String> {
    let src = conversation_id.trim();
    super::session_transfer::codebuddy::validate_conversation_id(src)?;
    let new_id = new_uuid_v4()?;
    match platform {
        "workbuddy" => fork_workbuddy_session(src, &new_id, title.as_deref()),
        "codebuddy-cn" | "codebuddy_cn" => fork_codebuddy_cn_session(src, &new_id, title.as_deref()),
        other => Err(format!("未知平台: {}", other)),
    }
}

/// 分叉后的默认标题：沿用原标题并加标记（原标题为空时只有标记）。
fn fork_title(src_title: &str, title: Option<&str>) -> String {
    let custom = title.map(|t| t.trim()).unwrap_or("");
    if !custom.is_empty() {
        return custom.to_string();
    }
    if src_title.trim().is_empty() {
        "（分叉）".to_string()
    } else {
        format!("{}（分叉）", src_title.trim())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// UUID v4（本仓库没有 uuid crate，用 getrandom 自己拼）。
fn new_uuid_v4() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|e| format!("生成会话 id 失败: {}", e))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

/// 表的列顺序（`SELECT *` 与 `PRAGMA table_info` 同序；写死列名会随版本炸）。
fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| format!("读取表结构失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("读取表结构失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows.flatten() {
        out.push(r);
    }
    if out.is_empty() {
        return Err(format!("表 {} 不存在", table));
    }
    Ok(out)
}

fn value_as_text(v: &rusqlite::types::Value) -> String {
    match v {
        rusqlite::types::Value::Text(t) => t.clone(),
        rusqlite::types::Value::Integer(n) => n.to_string(),
        rusqlite::types::Value::Real(f) => f.to_string(),
        _ => String::new(),
    }
}

/// WorkBuddy 分叉：复制 sessions 行（换 id/标题/时间）+ 复制正文文件。
fn fork_workbuddy_session(
    src_id: &str,
    new_id: &str,
    title: Option<&str>,
) -> Result<BuddySessionForkReport, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    fork_workbuddy_session_at(&home.join(".workbuddy"), src_id, new_id, title)
}

/// 分叉核心（数据根目录参数化，便于用临时目录做端到端测试）。
fn fork_workbuddy_session_at(
    data_root: &Path,
    src_id: &str,
    new_id: &str,
    title: Option<&str>,
) -> Result<BuddySessionForkReport, String> {
    let db_path = data_root.join("workbuddy.db");
    if !db_path.is_file() {
        return Err("未找到 WorkBuddy 会话库（~/.workbuddy/workbuddy.db）".to_string());
    }
    super::session_transfer::workbuddy::reject_symlink_if_exists(&db_path)?;

    let mut conn = Connection::open(&db_path).map_err(|e| format!("打开 WorkBuddy 数据库失败: {}", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置数据库超时失败: {}", e))?;

    // 1) 复制库记录：按列名动态取值（列集合随版本增减，写死 SQL 会炸）
    let columns = table_columns(&conn, "sessions")?;
    let mut values: Vec<rusqlite::types::Value> = Vec::with_capacity(columns.len());
    {
        let mut stmt = conn
            .prepare("SELECT * FROM sessions WHERE id = ?1")
            .map_err(|e| format!("读取源会话失败: {}", e))?;
        let mut rows = stmt
            .query(rusqlite::params![src_id])
            .map_err(|e| format!("查询源会话失败: {}", e))?;
        let row = rows
            .next()
            .map_err(|e| format!("读取源会话失败: {}", e))?
            .ok_or_else(|| format!("会话不存在: {}", src_id))?;
        for i in 0..columns.len() {
            values.push(match row.get_ref(i).map_err(|e| format!("读取会话字段失败: {}", e))? {
                rusqlite::types::ValueRef::Null => rusqlite::types::Value::Null,
                rusqlite::types::ValueRef::Integer(n) => rusqlite::types::Value::Integer(n),
                rusqlite::types::ValueRef::Real(f) => rusqlite::types::Value::Real(f),
                rusqlite::types::ValueRef::Text(t) => {
                    rusqlite::types::Value::Text(String::from_utf8_lossy(t).to_string())
                }
                rusqlite::types::ValueRef::Blob(b) => rusqlite::types::Value::Blob(b.to_vec()),
            });
        }
    }

    let src_title = columns
        .iter()
        .position(|c| c == "title")
        .map(|i| value_as_text(&values[i]))
        .unwrap_or_default();
    let new_title = fork_title(&src_title, title);
    let now = now_ms();
    for (i, c) in columns.iter().enumerate() {
        match c.as_str() {
            "id" => values[i] = rusqlite::types::Value::Text(new_id.to_string()),
            "title" => values[i] = rusqlite::types::Value::Text(new_title.clone()),
            "custom_title" => values[i] = rusqlite::types::Value::Text(new_title.clone()),
            // 时间戳刷新，保证新会话排在列表最前
            "created_at" | "updated_at" | "last_activity_at" => {
                values[i] = rusqlite::types::Value::Integer(now)
            }
            // 源会话若被软删，分叉出来的应当是干净的
            "deleted_at" => values[i] = rusqlite::types::Value::Null,
            _ => {}
        }
    }

    let cols_sql = columns
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=columns.len())
        .map(|i| format!("?{}", i))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("INSERT INTO sessions ({}) VALUES ({})", cols_sql, placeholders);
    {
        let tx = conn.transaction().map_err(|e| format!("开启数据库事务失败: {}", e))?;
        tx.execute(&sql, rusqlite::params_from_iter(values))
            .map_err(|e| format!("写入新会话记录失败: {}", e))?;
        tx.commit().map_err(|e| format!("提交新会话记录失败: {}", e))?;
    }

    // 2) 复制正文
    let mut files_copied = 0usize;
    let mut errors: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(data_root.join("projects")) {
        for entry in entries.flatten() {
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let src_file = project_dir.join(format!("{}.jsonl", src_id));
            if !src_file.is_file() {
                continue;
            }
            match copy_text_replacing(&src_file, &project_dir.join(format!("{}.jsonl", new_id)), src_id, new_id) {
                Ok(()) => files_copied += 1,
                Err(e) => errors.push(e),
            }
        }
    }
    let artifact = data_root.join("artifact-index").join(format!("{}.json", src_id));
    if artifact.is_file() {
        match copy_text_replacing(
            &artifact,
            &data_root.join("artifact-index").join(format!("{}.json", new_id)),
            src_id,
            new_id,
        ) {
            Ok(()) => files_copied += 1,
            Err(e) => errors.push(e),
        }
    }

    Ok(BuddySessionForkReport {
        conversation_id: new_id.to_string(),
        title: new_title,
        files_copied,
        content_missing: files_copied == 0,
        errors,
    })
}

/// 复制文本文件，并把内容里出现的旧会话 id 换成新 id（正文里每条记录都带 sessionId）。
/// 非 UTF-8 时退化为原样复制——宁可少改一个 id，也不要把二进制正文写坏。
fn copy_text_replacing(src: &Path, dst: &Path, old_id: &str, new_id: &str) -> Result<(), String> {
    if let Err(e) = super::session_transfer::codebuddy::reject_symlink_if_exists(src) {
        return Err(e);
    }
    let bytes = std::fs::read(src).map_err(|e| format!("读取正文失败 {}: {}", src.display(), e))?;
    let out = match String::from_utf8(bytes.clone()) {
        Ok(text) => text.replace(old_id, new_id).into_bytes(),
        // 非 UTF-8：原样复制（宁可少改一个 id，也不要把二进制正文写坏）
        Err(e) => {
            eprintln!("[BuddySession] 正文非 UTF-8，原样复制: {}", src.display());
            let _ = e;
            bytes
        }
    };
    std::fs::write(dst, out).map_err(|e| format!("写入正文失败 {}: {}", dst.display(), e))?;
    Ok(())
}

/// CodeBuddy CN 分叉：复制 vscdb 的会话索引 + 扩展目录里的会话正文。
fn fork_codebuddy_cn_session(
    src_id: &str,
    new_id: &str,
    title: Option<&str>,
) -> Result<BuddySessionForkReport, String> {
    let data_dir = super::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())?;
    let db_path = data_dir.join("codebuddy-sessions.vscdb");
    if !db_path.is_file() {
        return Err("未找到 CodeBuddy CN 会话库（codebuddy-sessions.vscdb）".to_string());
    }
    super::session_transfer::codebuddy::reject_symlink_if_exists(&db_path)?;
    let mut conn = Connection::open(&db_path).map_err(|e| format!("打开 CodeBuddy CN 会话库失败: {}", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置数据库超时失败: {}", e))?;

    let key = format!("session:{}", src_id);
    let raw: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .map_err(|_| format!("会话不存在: {}", src_id))?;
    let mut session: Value =
        serde_json::from_str(&raw).map_err(|e| format!("解析会话失败: {}", e))?;
    let obj = session
        .as_object_mut()
        .ok_or_else(|| "会话格式异常（不是对象）".to_string())?;
    let src_title = obj
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let new_title = fork_title(&src_title, title);
    let now = now_ms();
    obj.insert("conversationId".to_string(), Value::String(new_id.to_string()));
    if obj.contains_key("id") {
        obj.insert("id".to_string(), Value::String(new_id.to_string()));
    }
    obj.insert("title".to_string(), Value::String(new_title.clone()));
    obj.remove("deletedAt");
    for k in ["createdAt", "updatedAt", "lastActivityAt"] {
        if obj.contains_key(k) {
            obj.insert(k.to_string(), Value::from(now));
        }
    }
    let serialized = serde_json::to_string(&session).map_err(|e| format!("序列化会话失败: {}", e))?;
    {
        let tx = conn.transaction().map_err(|e| format!("开启数据库事务失败: {}", e))?;
        tx.execute(
            "INSERT OR REPLACE INTO ItemTable(key, value) VALUES(?1, ?2)",
            rusqlite::params![format!("session:{}", new_id), serialized],
        )
        .map_err(|e| format!("写入新会话记录失败: {}", e))?;
        tx.commit().map_err(|e| format!("提交新会话记录失败: {}", e))?;
    }

    // 正文：扩展目录 `<uid>/<IDE>/<uid>/history/<workspace>/<id>/`
    let mut files_copied = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let uid = current_platform_uid(super::models::BuddyPlatform::CodebuddyCn);
    let extension_dir = super::session_transfer::codebuddy::codebuddy_extension_data_dir().ok();
    if let (Some(uid), Some(extension_dir)) = (uid.as_deref(), extension_dir.as_deref()) {
        copy_session_files_in_extension(extension_dir, uid, src_id, new_id, &mut files_copied, &mut errors);
    } else {
        errors.push("未定位到扩展数据目录，只复制了会话索引".to_string());
    }

    Ok(BuddySessionForkReport {
        conversation_id: new_id.to_string(),
        title: new_title,
        files_copied,
        content_missing: files_copied == 0,
        errors,
    })
}

/// 在扩展目录里复制会话正文：会话目录 + 5 类辅助目录，并向工作区 index.json 登记新会话。
/// 结构与 `delete_session_files_in_extension` 对称（同一套遍历规则）。
fn copy_session_files_in_extension(
    extension_data_dir: &Path,
    uid: &str,
    src_id: &str,
    new_id: &str,
    files_copied: &mut usize,
    errors: &mut Vec<String>,
) {
    use super::session_transfer::codebuddy as transfer;
    if transfer::validate_uid(uid).is_err() {
        errors.push(format!("uid 不安全，跳过正文复制: {}", uid));
        return;
    }
    let account_outer = extension_data_dir.join(uid);
    if !account_outer.is_dir() {
        return;
    }
    let Ok(ide_entries) = std::fs::read_dir(&account_outer) else {
        return;
    };
    for ide_entry in ide_entries.flatten() {
        let Ok(metadata) = std::fs::symlink_metadata(ide_entry.path()) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let account_root = ide_entry.path().join(uid);
        let history_root = account_root.join("history");
        if !history_root.is_dir() {
            continue;
        }
        let Ok(workspace_entries) = std::fs::read_dir(&history_root) else {
            continue;
        };
        for workspace_entry in workspace_entries.flatten() {
            let Ok(metadata) = std::fs::symlink_metadata(workspace_entry.path()) else {
                continue;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let workspace_path = workspace_entry.path();
            let index_path = workspace_path.join("index.json");
            let Ok(index) = transfer::read_workspace_index(&index_path) else {
                continue;
            };
            let Some(conversations) = index.get("conversations").and_then(Value::as_array) else {
                continue;
            };
            let Some(src_entry) = conversations
                .iter()
                .find(|c| c.get("id").and_then(Value::as_str) == Some(src_id))
            else {
                continue;
            };

            // 目录本体 + 辅助目录
            let conv_dir = workspace_path.join(src_id);
            if conv_dir.is_dir() {
                match copy_dir_recursive(&conv_dir, &workspace_path.join(new_id)) {
                    Ok(n) => *files_copied += n,
                    Err(e) => errors.push(format!("复制会话目录失败: {}", e)),
                }
            }
            for kind in transfer::AUXILIARY_KINDS {
                let aux_src = account_root
                    .join(kind)
                    .join(workspace_entry.file_name())
                    .join(src_id);
                if aux_src.is_dir() {
                    let aux_dst = account_root
                        .join(kind)
                        .join(workspace_entry.file_name())
                        .join(new_id);
                    if let Err(e) = copy_dir_recursive(&aux_src, &aux_dst) {
                        errors.push(format!("复制辅助目录失败 {}: {}", kind, e));
                    }
                }
            }

            // index.json：登记新会话（沿用源条目属性，只换 id 与标题），current 不动
            let mut new_entry = src_entry.clone();
            if let Some(obj) = new_entry.as_object_mut() {
                obj.insert("id".to_string(), Value::String(new_id.to_string()));
                if obj.contains_key("title") {
                    obj.insert(
                        "title".to_string(),
                        Value::String(fork_title(
                            obj.get("title").and_then(Value::as_str).unwrap_or_default(),
                            None,
                        )),
                    );
                }
            }
            let mut new_index = index.clone();
            if let Some(array) = new_index
                .get_mut("conversations")
                .and_then(Value::as_array_mut)
            {
                array.push(new_entry);
            }
            match serde_json::to_string_pretty(&new_index)
                .map_err(|e| format!("序列化工作区索引失败: {}", e))
                .and_then(|s| super::store::write_atomic(&index_path, &s))
            {
                Ok(()) => {}
                Err(e) => errors.push(e),
            }
        }
    }
}

/// 递归复制目录（跳过符号链接，返回复制的文件数）。
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("创建目录失败 {}: {}", dst.display(), e))?;
    let mut count = 0usize;
    for entry in std::fs::read_dir(src)
        .map_err(|e| format!("读取目录失败 {}: {}", src.display(), e))?
        .flatten()
    {
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let target = dst.join(entry.file_name());
        if metadata.is_dir() {
            count += copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(&entry.path(), &target)
                .map_err(|e| format!("复制文件失败 {}: {}", entry.path().display(), e))?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小 workbuddy.db（列比真机少，验证「按列名动态取」不是写死 SQL）。
    fn make_workbuddy_db(dir: &Path, id: &str, title: &str) {
        let conn = Connection::open(dir.join("workbuddy.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY, cwd TEXT NOT NULL, user_id TEXT NOT NULL,
                title TEXT, custom_title TEXT, status TEXT NOT NULL DEFAULT 'Pending',
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                deleted_at INTEGER, is_playground INTEGER NOT NULL DEFAULT 0,
                last_activity_at INTEGER
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id,cwd,user_id,title,custom_title,status,created_at,updated_at,deleted_at,is_playground,last_activity_at)
             VALUES (?1,'/work/app','u1',?2,NULL,'Completed',1000,2000,NULL,0,2500)",
            rusqlite::params![id, title],
        )
        .unwrap();
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("buddy-fork-{}-{}", tag, now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 端到端：库记录复制 + 正文复制且正文里的旧 id 全部换成新 id，原会话纹丝不动。
    #[test]
    fn fork_workbuddy_copies_row_and_transcript() {
        let root = temp_dir("wb");
        let src_id = "11111111-2222-4333-8444-555555555555";
        let new_id = new_uuid_v4().unwrap();
        make_workbuddy_db(&root, src_id, "重构 API");

        let project = root.join("projects").join("c-work-app");
        std::fs::create_dir_all(&project).unwrap();
        let transcript = format!(
            "{{\"id\":\"m1\",\"sessionId\":\"{}\",\"role\":\"user\"}}\n{{\"id\":\"m2\",\"sessionId\":\"{}\",\"role\":\"assistant\"}}\n",
            src_id, src_id
        );
        std::fs::write(project.join(format!("{}.jsonl", src_id)), &transcript).unwrap();

        let report = fork_workbuddy_session_at(&root, src_id, &new_id, None).unwrap();
        assert_eq!(report.conversation_id, new_id);
        assert_eq!(report.title, "重构 API（分叉）");
        assert_eq!(report.files_copied, 1);
        assert!(!report.content_missing);
        assert!(report.errors.is_empty());

        // 新行：id/标题/时间都换了，cwd/user_id/status 沿用
        let conn = Connection::open(root.join("workbuddy.db")).unwrap();
        let (cwd, user_id, title, status, created, deleted): (String, String, String, String, i64, Option<i64>) = conn
            .query_row(
                "SELECT cwd, user_id, title, status, created_at, deleted_at FROM sessions WHERE id = ?1",
                rusqlite::params![new_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert_eq!(cwd, "/work/app");
        assert_eq!(user_id, "u1");
        assert_eq!(title, "重构 API（分叉）");
        assert_eq!(status, "Completed");
        assert!(created > 2000);
        assert!(deleted.is_none());

        // 正文：新文件里的 sessionId 全换成新 id，旧文件一个字节都没动
        let copied = std::fs::read_to_string(project.join(format!("{}.jsonl", new_id))).unwrap();
        assert!(!copied.contains(src_id));
        assert_eq!(copied.matches(&format!("\"sessionId\":\"{}\"", new_id)).count(), 2);
        assert_eq!(
            std::fs::read_to_string(project.join(format!("{}.jsonl", src_id))).unwrap(),
            transcript
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 库里有记录但正文已丢：仍要建出新会话，并如实回报 contentMissing。
    #[test]
    fn fork_workbuddy_reports_missing_content() {
        let root = temp_dir("wb-missing");
        let src_id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
        make_workbuddy_db(&root, src_id, "孤儿会话");
        let report = fork_workbuddy_session_at(&root, src_id, &new_uuid_v4().unwrap(), None).unwrap();
        assert_eq!(report.files_copied, 0);
        assert!(report.content_missing);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 源会话不存在 → 报错，且不留下半条记录。
    #[test]
    fn fork_workbuddy_rejects_unknown_session() {
        let root = temp_dir("wb-unknown");
        make_workbuddy_db(&root, "known-id", "x");
        let err = fork_workbuddy_session_at(&root, "missing-id", &new_uuid_v4().unwrap(), None).unwrap_err();
        assert!(err.contains("会话不存在"), "实际错误: {}", err);
        let conn = Connection::open(root.join("workbuddy.db")).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fork_title_falls_back_to_source_title() {
        assert_eq!(fork_title("重构 API", None), "重构 API（分叉）");
        assert_eq!(fork_title("重构 API", Some("  ")), "重构 API（分叉）");
        assert_eq!(fork_title("重构 API", Some("新方案 A")), "新方案 A");
        assert_eq!(fork_title("", None), "（分叉）");
    }

    #[test]
    fn new_uuid_v4_is_unique_and_versioned() {
        let a = new_uuid_v4().unwrap();
        let b = new_uuid_v4().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4", "版本号位应为 v4: {}", a);
    }

    #[test]
    fn parse_session_json() {
        let json = r#"{"conversationId":"conv1","title":"重构 API","cwd":"/work/app","userId":"u1","status":"Completed","createdAt":1000,"updatedAt":2000,"isPlayground":false}"#;
        let raw = parse_raw_session(json).unwrap();
        assert_eq!(raw.conversation_id, "conv1");
        assert_eq!(raw.title, "重构 API");
        assert_eq!(raw.status, "Completed");
        assert!(!raw.is_deleted);
    }

    #[test]
    fn parse_session_with_deleted() {
        let json = r#"{"conversationId":"conv2","title":"t","cwd":"/x","userId":"u1","status":"InProgress","deletedAt":999}"#;
        let raw = parse_raw_session(json).unwrap();
        assert!(raw.is_deleted);
    }

    #[test]
    fn aggregate_filters_and_dedupes() {
        let sessions = vec![
            RawSession {
                conversation_id: "a".into(),
                title: "Alpha".into(),
                cwd: "/p1".into(),
                user_id: "u1".into(),
                status: "Completed".into(),
                created_at: Some(1),
                updated_at: Some(10),
                is_deleted: false,
                is_playground: false,
                last_activity_at: None,
            },
            RawSession {
                conversation_id: "b".into(),
                title: "Beta".into(),
                cwd: "/p2".into(),
                user_id: "u1".into(),
                status: "InProgress".into(),
                created_at: Some(2),
                updated_at: Some(20),
                is_deleted: false,
                is_playground: false,
                last_activity_at: None,
            },
            RawSession {
                conversation_id: "a".into(),
                title: "Alpha".into(),
                cwd: "/p1".into(),
                user_id: "u1".into(),
                status: "Completed".into(),
                created_at: Some(1),
                updated_at: Some(30), // 更新
                is_deleted: false,
                is_playground: false,
                last_activity_at: None,
            },
            RawSession {
                conversation_id: "gone".into(),
                title: "Deleted".into(),
                cwd: "/p3".into(),
                user_id: "u1".into(),
                status: "Completed".into(),
                created_at: Some(3),
                updated_at: Some(40),
                is_deleted: true,
                is_playground: false,
                last_activity_at: None,
            },
        ];

        // 关键字过滤 + 去重 + 排除已删除
        let filter = BuddySessionFilter {
            keyword: Some("alpha".to_string()),
            status: None,
        };
        let records = aggregate_sessions(sessions.clone(), &filter, "default", "CB");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].conversation_id, "a");
        assert_eq!(records[0].updated_at, Some(30));
        assert_eq!(records[0].locations.len(), 1);

        // 状态过滤
        let filter = BuddySessionFilter {
            keyword: None,
            status: Some("InProgress".to_string()),
        };
        let records = aggregate_sessions(sessions, &filter, "default", "CB");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].conversation_id, "b");
    }

    #[test]
    fn delete_sessions_at_removes_db_row_and_files() {
        let dest = std::env::temp_dir().join(format!(
            "kira-session-delete-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dest).unwrap();

        let uid = "uid-a";
        // vscdb：两条会话记录
        let db_path = dest.join("codebuddy-sessions.vscdb");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO ItemTable VALUES ('session:conv1', '{\"userId\":\"uid-a\"}');
                INSERT INTO ItemTable VALUES ('session:conv2', '{\"userId\":\"uid-a\"}');",
            )
            .unwrap();
        }
        // 会话文件：conv1（含 messages 与辅助目录）、conv2
        let extension = dest.join("CodeBuddyExtension").join("Data");
        let account_root = extension.join(uid).join("VSCode").join(uid);
        let history = account_root.join("history").join("ws1");
        std::fs::create_dir_all(history.join("conv1").join("messages")).unwrap();
        std::fs::write(history.join("conv1").join("messages").join("0.json"), "[]").unwrap();
        std::fs::create_dir_all(history.join("conv2")).unwrap();
        std::fs::write(
            history.join("index.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "conversations": [
                    {"id": "conv1", "lastMessageAt": 1},
                    {"id": "conv2", "lastMessageAt": 2}
                ],
                "current": "conv1"
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(account_root.join("check-point").join("ws1").join("conv1")).unwrap();

        let mut report = BuddySessionDeleteReport::default();
        delete_codebuddy_sessions_at(
            &dest,
            Some(&extension),
            uid,
            &["conv1".to_string()],
            &mut report,
        );

        // 数据库记录删除
        assert_eq!(report.db_deleted, 1);
        // 会话目录删除，其余会话不受影响
        assert_eq!(report.history_dirs_removed, 1);
        assert!(!history.join("conv1").exists());
        assert!(history.join("conv2").exists());
        // 辅助目录删除
        assert_eq!(report.auxiliary_dirs_removed, 1);
        assert!(!account_root.join("check-point").join("ws1").join("conv1").exists());
        // index.json：条目移除 + current 清空
        let index: Value = serde_json::from_slice(&std::fs::read(history.join("index.json")).unwrap()).unwrap();
        assert_eq!(index["conversations"].as_array().unwrap().len(), 1);
        assert_eq!(index["conversations"][0]["id"].as_str(), Some("conv2"));
        assert_eq!(index["current"].as_str(), Some(""));
        assert!(report.errors.is_empty());

        let _ = std::fs::remove_dir_all(&dest);
    }

    // ─── 列表排序：活动时间回退（last_activity_at → updated_at → created_at） ───

    fn raw_session(
        id: &str,
        created_at: Option<i64>,
        updated_at: Option<i64>,
        last_activity_at: Option<i64>,
    ) -> RawSession {
        RawSession {
            conversation_id: id.to_string(),
            title: id.to_string(),
            cwd: "/ws".to_string(),
            user_id: "uid".to_string(),
            status: "Completed".to_string(),
            created_at,
            updated_at,
            last_activity_at,
            is_deleted: false,
            is_playground: false,
        }
    }

    #[test]
    fn session_sort_uses_newest_activity_time() {
        let raws = vec![
            raw_session("only-created", Some(300), None, None),
            raw_session("by-activity", Some(100), Some(200), Some(500)),
            raw_session("by-updated", Some(100), Some(400), None),
        ];
        let records = aggregate_sessions(
            raws,
            &BuddySessionFilter::default(),
            "workbuddy",
            "WorkBuddy",
        );
        let ids: Vec<&str> = records
            .iter()
            .map(|r| r.conversation_id.as_str())
            .collect();
        // 活动时间 = 三者取最新；缺失 last_activity_at 时回退 updated_at / created_at
        assert_eq!(ids, vec!["by-activity", "by-updated", "only-created"]);
        assert_eq!(records[0].last_activity_at, Some(500));
        assert_eq!(records[1].last_activity_at, Some(400));
        assert_eq!(records[2].last_activity_at, Some(300));
    }

    #[test]
    fn session_activity_takes_newest_when_updated_beats_last_activity() {
        // 真机模式（WorkBuddy 5.x）：客户端刷新会 bump updated_at，使其新于 last_activity_at；
        // 此时活动时间必须取 updated_at，否则最近用过的会话会沉底
        let raws = vec![raw_session("real", Some(100), Some(900), Some(500))];
        let records = aggregate_sessions(
            raws,
            &BuddySessionFilter::default(),
            "workbuddy",
            "WorkBuddy",
        );
        assert_eq!(records[0].last_activity_at, Some(900));
    }

    // ─── 正文存在性探测 ───

    #[test]
    fn collect_local_conversation_ids_requires_index_entry_and_dir() {
        let root = std::env::temp_dir().join(format!(
            "kira-session-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let uid = "uid-a";
        let history = root
            .join(uid)
            .join("VSCode")
            .join(uid)
            .join("history")
            .join("ws1");
        std::fs::create_dir_all(history.join("conv-ok")).unwrap();
        std::fs::write(
            history.join("index.json"),
            r#"{"conversations":[{"id":"conv-ok"},{"id":"conv-no-dir"}]}"#,
        )
        .unwrap();

        let ids = collect_local_conversation_ids_from_roots(&[root.clone()], uid)
            .expect("存在 history 布局时必须返回集合");
        assert!(ids.contains("conv-ok"));
        // index.json 里登记了、但正文目录不存在 → 不算「本地仍有正文」
        assert!(!ids.contains("conv-no-dir"));

        // 没有 history 布局（5.x 只留 DB / 全新装机）→ None，调用方跳过误报
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(collect_local_conversation_ids_from_roots(&[empty], uid).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn content_probe_marks_only_unknown_ids() {
        let make = |id: &str| BuddySessionRecord {
            conversation_id: id.to_string(),
            title: id.to_string(),
            cwd: "/ws".to_string(),
            user_id: "uid".to_string(),
            status: "Completed".to_string(),
            created_at: Some(1),
            updated_at: Some(1),
            last_activity_at: Some(1),
            is_playground: false,
            content_missing: false,
            locations: Vec::new(),
        };
        let mut records = vec![make("a"), make("b")];

        // 定位不到本地布局 → 一律不标记，避免全量误报
        apply_content_probe(&mut records, None);
        assert!(records.iter().all(|r| !r.content_missing));

        let local: HashSet<String> = ["a".to_string()].into_iter().collect();
        apply_content_probe(&mut records, Some(&local));
        assert!(!records[0].content_missing);
        assert!(records[1].content_missing);
    }
}