use serde::{Deserialize, Serialize};

// ─── 文档 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MindmapFolder {
    pub id: String,
    pub name: String,
    pub sort_order: i64,
    pub document_count: usize,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MindmapDocument {
    pub id: String,
    pub name: String,
    pub description: String,
    /// manual | ai_project | ai_text | task
    pub source_type: String,
    /// 来源描述
    pub source_desc: String,
    pub folder_id: Option<String>,
    pub node_count: usize,
    pub sticker_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default = "default_background_texture")]
    pub background_texture: String,
    /// 布局方向：lr=左→右（默认） rl=右→左 tb=上→下 bt=下→上
    #[serde(default = "default_layout_dir")]
    pub layout_dir: String,
}

fn default_background_texture() -> String { "dots".to_string() }
fn default_layout_dir() -> String { "lr".to_string() }

// ─── 节点 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MindmapNode {
    pub id: String,
    pub document_id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// 详细 Markdown
    #[serde(default)]
    pub detail: String,
    /// root/module/requirement/task/constraint/risk/other/component/service/route/config/file
    #[serde(default = "default_kind")]
    pub kind: String,
    /// 节点颜色 (hex)
    #[serde(default = "default_color")]
    pub color: String,
    /// 进度 0-100
    #[serde(default)]
    pub progress: i32,
    /// 计划时间（ISO 8601 字符串，可空；旧数据为 None）
    #[serde(default)]
    pub plan_at: Option<String>,
    /// 计划重复：none=不重复 / daily=每天 / weekly=每周
    #[serde(default = "default_repeat")]
    pub repeat: String,
    /// 证据锚定：该节点对应的真实源码文件（项目相对路径，来自 AI 标注 + 扫描校验）
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
    pub created_at: String,
    pub updated_at: String,
}

fn default_repeat() -> String { "none".to_string() }

fn default_kind() -> String { "other".to_string() }
fn default_color() -> String { "#f59e0b".to_string() }

// ─── 贴纸 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MindmapSticker {
    pub id: String,
    pub document_id: String,
    pub content: String,
    /// 图片贴纸的 data URL；文字贴纸为空
    #[serde(default)]
    pub image_data: String,
    /// 用户调整后的旋转角度；旧数据为空时由前端使用默认角度
    #[serde(default)]
    pub rotation: Option<f64>,
    #[serde(default)]
    pub color: String,
    pub position_x: f64,
    pub position_y: f64,
    pub created_at: String,
    pub updated_at: String,
}

// ─── 入参 ───

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDocumentInput {
    pub name: String,
    pub description: Option<String>,
    pub source_type: Option<String>,
    pub folder_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDocumentInput {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub folder_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderInput {
    pub name: String,
    #[serde(default)]
    pub folder_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFolderInput {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveDocumentInput {
    pub document_id: String,
    pub folder_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFolderInput {
    pub folder_id: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertNodeInput {
    pub document_id: String,
    pub node: MindmapNode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteNodeInput {
    pub document_id: String,
    pub node_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionInput {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertStickerInput {
    pub document_id: String,
    pub sticker: MindmapSticker,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteStickerInput {
    pub document_id: String,
    pub sticker_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGenerateInput {
    pub document_id: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGenerateProjectInput {
    pub document_id: String,
    pub project_path: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGenerateTextInput {
    pub document_id: String,
    pub text: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateNodeInput {
    pub document_id: String,
    pub node_id: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

/// 文档完整负载
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentFull {
    pub document: MindmapDocument,
    pub nodes: Vec<MindmapNode>,
    pub stickers: Vec<MindmapSticker>,
}

/// 指定日期范围内的具体计划发生记录（计划日历聚合展示用）。
/// 重复计划（daily/weekly）已由后端在查询时展开为逐次发生。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedOccurrence {
    pub id: String,
    pub document_id: String,
    pub document_name: String,
    pub name: String,
    pub kind: String,
    pub color: String,
    /// 原始计划时间（ISO 8601，用于打开详情时回显）
    pub plan_at: String,
    /// 计划重复：none / daily / weekly
    #[serde(default = "default_repeat")]
    pub repeat: String,
    /// 本次发生的日期 YYYY-MM-DD（本地时间）
    pub occur_day: String,
    /// 本次发生的具体时间（本地时间字符串，如 2026-08-30T09:00:00）
    pub occur_at: String,
}

/// 拖拽移动计划发生记录：按 from_day → to_day 的天数差改写 plan_at
/// （保留本地钟点；daily/weekly 整条顺延、none 单次移动）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePlanOccurrenceInput {
    pub node_id: String,
    /// 拖拽来源日期 YYYY-MM-DD（该次发生的 occur_day）
    pub from_day: String,
    /// 拖拽目标日期 YYYY-MM-DD
    pub to_day: String,
}

/// 某个视图生成失败的原因（不影响其它已成功的视图）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiImportFailure {
    pub view: String,
    pub reason: String,
}

/// 单个视图的校验报告（导入完成弹窗展示用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiImportReport {
    pub document_id: String,
    /// 视图类型：architecture / workflow / dataflow / sequence / lifecycle
    pub view: String,
    /// 导入的节点总数（含根）
    pub node_count: usize,
    /// 实际发生的 AI 调用轮数（1 = 首次即通过；>1 = 经过修复重试）
    pub repair_rounds: usize,
    /// 修复循环耗尽后仍残留的校验错误（空 = 完全通过）
    pub diagnostics: Vec<String>,
    /// 引用的证据文件总数（所有节点 sources 之和）
    pub evidence_count: usize,
    /// 命中真实文件的证据数（evidence_verified=false 时等于 evidence_count）
    pub evidence_hit_count: usize,
    /// 证据是否经过文件集核验（项目模式 true，文本模式 false）
    pub evidence_verified: bool,
    /// 有证据的节点数（无证据节点 = node_count - evidence_nodes，即纯 AI 推断）
    pub evidence_nodes: usize,
}

/// AI 类型路由导入结果：一次生成多个视图，各自落在独立文档。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiImportResult {
    /// 本次生成的全部文档（含内容），按视图重要程度排序
    pub documents: Vec<DocumentFull>,
    /// 应切换到的主文档 id（第一个成功的视图）
    pub primary_id: String,
    /// 失败的视图
    #[serde(default)]
    pub failures: Vec<AiImportFailure>,
    /// 各视图的校验报告
    #[serde(default)]
    pub reports: Vec<AiImportReport>,
}