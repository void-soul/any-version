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
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
    pub created_at: String,
    pub updated_at: String,
}

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