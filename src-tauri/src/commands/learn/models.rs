use serde::{Deserialize, Serialize};

/// 学习结构中的一个节点（模块/组件/类/函数/服务等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnNode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    /// 一句话概述（画布节点副标题）。
    #[serde(default)]
    pub description: String,
    /// 详细说明（markdown，弹窗查看）。
    #[serde(default)]
    pub detail: String,
    /// 节点类型：module / component / class / function / service / route / file / config / other
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

/// 一次分析生成的完整结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnGraph {
    pub project_path: String,
    pub project_name: String,
    /// 项目整体概述（markdown）。
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub nodes: Vec<LearnNode>,
}

/// 索引条目（历史记录列表）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnMeta {
    pub project_path: String,
    pub project_name: String,
    pub generated_at: String,
    pub node_count: usize,
}

/// 生成入参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateLearnInput {
    pub project_path: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

/// 从文本生成需求的入参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateFromTextInput {
    pub text: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

/// 重新分析某节点子树的入参。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegenerateNodeInput {
    pub project_path: String,
    pub node_id: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

/// 单个节点位置更新。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePosition {
    pub node_id: String,
    pub x: f64,
    pub y: f64,
}
