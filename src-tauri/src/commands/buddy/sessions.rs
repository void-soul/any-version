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

#[cfg(test)]
mod tests {
    use super::*;

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