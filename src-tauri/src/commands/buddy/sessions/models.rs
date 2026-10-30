use super::*;

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
pub(super) struct RawSession {
    pub(super) conversation_id: String,
    pub(super) title: String,
    pub(super) cwd: String,
    pub(super) user_id: String,
    pub(super) status: String,
    pub(super) created_at: Option<i64>,
    pub(super) updated_at: Option<i64>,
    pub(super) last_activity_at: Option<i64>,
    pub(super) is_deleted: bool,
    pub(super) is_playground: bool,
}
