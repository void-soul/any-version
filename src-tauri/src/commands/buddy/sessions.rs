//! Buddy 会话管理：列出本机 WorkBuddy / CodeBuddy CN 的 AI 会话。
//!
//! 复刻自 cockpit-tools `codebuddy_session.rs`：
//! - WorkBuddy：会话存在 `~/.workbuddy/workbuddy.db`（单库，按 user_id 分区）
//! - CodeBuddy CN：会话存在 `%APPDATA%/CodeBuddy CN/codebuddy-sessions.vscdb`
//!   （ItemTable 中 key 以 `session:` 开头的 JSON）

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub is_playground: bool,
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
    is_deleted: bool,
    is_playground: bool,
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

    let mut stmt = match conn.prepare(
        "SELECT id, cwd, user_id, title, status, created_at, updated_at, deleted_at, is_playground \
         FROM sessions",
    ) {
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
        ))
    });

    let mut sessions = Vec::new();
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let (id, cwd, user_id, title, status, created_at, updated_at, deleted_at, is_play) = row;
            sessions.push(RawSession {
                conversation_id: id,
                title: title.unwrap_or_default(),
                cwd: cwd.unwrap_or_default(),
                user_id: user_id.unwrap_or_default(),
                status: status.unwrap_or_default(),
                created_at,
                updated_at,
                is_deleted: deleted_at.is_some(),
                is_playground: is_play != 0,
            });
        }
    }

    sessions
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
                if !existing
                    .locations
                    .iter()
                    .any(|l| l.instance_id == instance_id)
                {
                    existing.locations.push(location.clone());
                }
            })
            .or_insert_with(|| BuddySessionRecord {
                conversation_id: raw.conversation_id,
                title: raw.title,
                cwd: raw.cwd,
                user_id: raw.user_id,
                status: raw.status,
                created_at: raw.created_at,
                updated_at: raw.updated_at,
                is_playground: raw.is_playground,
                locations: vec![location],
            });
    }

    let mut records: Vec<BuddySessionRecord> = aggregated.into_values().collect();
    records.sort_by(|a, b| b.updated_at.unwrap_or(0).cmp(&a.updated_at.unwrap_or(0)));
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
            Ok(aggregate_sessions(
                read_sessions_from_workbuddy_db(&db_path),
                filter,
                "workbuddy",
                "WorkBuddy",
            ))
        }
        "codebuddy-cn" | "codebuddy_cn" => {
            let db_path = codebuddy_cn_sessions_db_path()?;
            Ok(aggregate_sessions(
                read_sessions_from_db(&db_path),
                filter,
                "default",
                "CodeBuddy CN",
            ))
        }
        other => Err(format!("未知平台: {}", other)),
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
}