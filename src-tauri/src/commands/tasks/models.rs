use serde::{Deserialize, Serialize};

/// 任务优先级。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

impl TaskPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskPriority::Low => "low",
            TaskPriority::Medium => "medium",
            TaskPriority::High => "high",
            TaskPriority::Urgent => "urgent",
        }
    }
    pub fn from_str(s: &str) -> TaskPriority {
        match s {
            "low" => TaskPriority::Low,
            "medium" => TaskPriority::Medium,
            "high" => TaskPriority::High,
            "urgent" => TaskPriority::Urgent,
            _ => TaskPriority::Medium,
        }
    }
    /// 排序权重（数字越大越靠前）。
    pub fn weight(&self) -> i64 {
        match self {
            TaskPriority::Urgent => 4,
            TaskPriority::High => 3,
            TaskPriority::Medium => 2,
            TaskPriority::Low => 1,
        }
    }
}

/// 任务状态（由 progress 派生，不单独存储）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Todo,
    InProgress,
    Done,
}

/// 任务项。progress 是唯一真相来源，status 派生自 progress。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// 详细内容（大段 markdown，弹窗编辑/预览）。
    #[serde(default)]
    pub detail: String,
    /// 计划日期（YYYY-MM-DD）。null 表示未排期（Inbox）。
    #[serde(default)]
    pub scheduled_date: Option<String>,
    /// 父任务 ID。null 表示顶层任务。
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default = "default_task_color")]
    pub color: String,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
    #[serde(default)]
    pub priority: String, // low|medium|high|urgent
    pub progress: i64, // 0-100
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub estimate_minutes: i64,
    #[serde(default)]
    pub tags: String, // 逗号分隔
    /// 标记为归档/软删除。
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// 任务的每日复盘日志。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLog {
    pub id: String,
    pub task_id: String,
    pub log_date: String, // YYYY-MM-DD
    pub content: String,
    pub progress_before: i64,
    pub progress_after: i64,
    /// 当日实际投入分钟。
    #[serde(default)]
    pub minutes_spent: i64,
    #[serde(default)]
    pub references: Vec<TaskLogReference>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLogReference {
    pub id: String,
    pub log_id: String,
    /// file | image | picky
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLogReferenceInput {
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub label: String,
}

/// 任务转移到其它日期/状态的记录（用于复盘"为什么没做完"）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMoveRecord {
    pub id: String,
    pub task_id: String,
    /// 原状态/进度快照。
    pub from_status: String,
    pub from_progress: i64,
    pub from_date: Option<String>,
    /// 目标。
    pub to_status: String,
    pub to_progress: i64,
    pub to_date: Option<String>,
    pub reason: String,
    pub moved_at: String,
}

/// 汇总指标（用于头部卡片 / 复盘页）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub date: String,
    pub total: i64,
    pub completed: i64,
    pub in_progress: i64,
    pub pending: i64,
    pub total_estimate: i64,
    pub total_spent: i64,
}

/// 复盘页用的"按天"聚合。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStat {
    pub date: String,
    pub total: i64,
    pub completed: i64,
    pub in_progress: i64,
    pub pending: i64,
    pub minutes_spent: i64,
}

/// 创建任务入参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub scheduled_date: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default = "default_task_color")]
    pub color: String,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub progress: i64,
    #[serde(default)]
    pub estimate_minutes: i64,
    #[serde(default)]
    pub tags: String,
}

fn default_priority() -> String {
    "medium".to_string()
}

fn default_task_color() -> String {
    "#f59e0b".to_string()
}

/// 区分"字段缺失"与"显式传 null"：
///   缺失 -> None（保持原值不变）；null -> Some(None)（清空为收集箱）。
/// 若只用 #[serde(default)]，显式 null 会退化成 None，导致"移入收集箱"失效。
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

/// 更新任务入参（部分更新）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub scheduled_date: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub parent_id: Option<Option<String>>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub position_x: Option<f64>,
    #[serde(default)]
    pub position_y: Option<f64>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub estimate_minutes: Option<i64>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
    /// 显式设置进度（会触发 move record 记录）。
    #[serde(default)]
    pub progress: Option<i64>,
    #[serde(default)]
    pub sort_order: Option<i64>,
}

/// 写进度入参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProgressInput {
    pub progress: i64,
    /// 可选复盘日志内容。
    #[serde(default)]
    pub log_content: Option<String>,
    #[serde(default)]
    pub references: Vec<TaskLogReferenceInput>,
    /// 当日投入分钟。
    #[serde(default)]
    pub minutes_spent: i64,
    /// 若进度未达 100 但希望把任务"结转"到该日期，则填写此字段。
    #[serde(default)]
    pub carry_to_date: Option<String>,
    /// 结转/转移原因（用于 move record）。
    #[serde(default)]
    pub move_reason: Option<String>,
}

/// 转移任务入参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveTaskInput {
    /// 显式 null = 移入收集箱；字段缺失 = 保持原日期。
    #[serde(default, deserialize_with = "double_option")]
    pub to_date: Option<Option<String>>,
    #[serde(default)]
    pub to_progress: Option<i64>,
    #[serde(default)]
    pub reason: String,
}

/// 新增日志入参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddLogInput {
    pub task_id: String,
    pub log_date: String,
    pub content: String,
    #[serde(default)]
    pub minutes_spent: i64,
    #[serde(default)]
    pub references: Vec<TaskLogReferenceInput>,
}

/// 排序入参（批量重排）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderInput {
    pub ids: Vec<String>,
}

/// 状态派生：progress 决定 status。
pub fn derive_status(progress: i64) -> TaskStatus {
    if progress >= 100 {
        TaskStatus::Done
    } else if progress > 0 {
        TaskStatus::InProgress
    } else {
        TaskStatus::Todo
    }
}

/// 提醒弹窗用的精简任务项（避免回传大字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBrief {
    pub id: String,
    pub title: String,
    pub priority: String,
    pub progress: i64,
    /// 计划日期（逾期任务有值，今日任务为 today）。
    pub scheduled_date: Option<String>,
}

impl TaskBrief {
    pub fn from_item(t: TaskItem) -> TaskBrief {
        TaskBrief {
            id: t.id,
            title: t.title,
            priority: t.priority,
            progress: t.progress,
            scheduled_date: t.scheduled_date,
        }
    }
}

/// 启动后的今日待办提醒数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderData {
    pub today: String,
    pub today_pending: Vec<TaskBrief>,
    pub overdue: Vec<TaskBrief>,
}

// ─── 画布贴纸（白板便签） ───

/// 贴纸数据。每个贴纸属于一个系列，可在画布上自由拖放。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSticker {
    pub id: String,
    pub series_id: String,
    #[serde(default)]
    pub content: String,
    /// 便签颜色（hex，如 #fef3c7 淡黄）。
    #[serde(default = "default_sticker_color")]
    pub color: String,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_sticker_color() -> String {
    "#fef3c7".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateStickerInput {
    pub series_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default = "default_sticker_color")]
    pub color: String,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStickerInput {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub position_x: Option<f64>,
    #[serde(default)]
    pub position_y: Option<f64>,
}
