use serde::{Deserialize, Serialize};

// ─── 需求项目（顶层容器） ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReqProject {
    pub id: String,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReqProjectInput {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReqProjectInput {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

// ─── 需求模块（项目下的分类，每个模块 = 一个图谱） ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReqModule {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReqModuleInput {
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReqModuleInput {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

// ─── 图谱节点（与旧 LearnNode 完全兼容） ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnNode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
}

fn default_kind() -> String {
    "other".to_string()
}

/// 图谱（属于一个模块）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnGraph {
    /// 模块 ID（新主键）或旧式 project_path（兼容）。
    pub module_id: String,
    pub project_name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub generated_at: String,
    /// 来源类型：manual / ai_project / ai_text / import
    #[serde(default)]
    pub source_type: String,
    /// 来源描述（如项目路径、文本标题、导入文件名）。
    #[serde(default)]
    pub source_desc: String,
    #[serde(default)]
    pub nodes: Vec<LearnNode>,
}

// ─── 旧兼容类型（保留，但新命令不再使用） ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnMeta {
    pub project_path: String,
    pub project_name: String,
    pub generated_at: String,
    pub node_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateLearnInput {
    pub project_path: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateFromTextInput {
    pub text: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateNodeInput {
    pub module_id: String,
    pub node_id: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePosition {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}

// ─── 新命令入参 ───

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateGraphFromProjectInput {
    pub module_id: String,
    pub project_path: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateGraphFromTextInput {
    pub module_id: String,
    pub text: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}
