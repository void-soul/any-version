// 数据库设计器的数据模型（对外即 `.dbdesign.json` 交换格式）。
//
// 三条刻意的设计约束（详见 docs/design/db-designer.md）：
// 1. 字段类型存「逻辑类型 + 参数」而不是 `VARCHAR(255)` 整串 —— 导出四种方言要做映射，
//    存整串就只能靠字符串解析（脆）。
// 2. 连线引用「节点 id + **字段名**」而不是字段 id —— Agent 手写文件时不该先造 id。
//    代价：字段改名必须级联更新关联（见 store::rename_field）。
// 3. 视图 / 函数只存 SQL 文本，不做字段级建模 —— 否则等于要写一个 SQL 解析器。
use serde::{Deserialize, Serialize};

// 只做关系型三种：MongoDB 是文档库、没有 schema / 外键 / 建表语句，
// 硬塞进来只会让数据模型一半字段对它无意义（已按用户决定移除）。
pub const DIALECTS: [&str; 3] = ["mysql", "postgres", "sqlite"];
// 视图保留（它就是一段 SQL 文本，成本极低）；函数不做：SQLite 没有、
// MySQL / PG 语法差异大，容易导出跑不起来的东西。
pub const NODE_KINDS: [&str; 2] = ["table", "view"];
pub const RELATION_KINDS: [&str; 3] = ["1-1", "1-n", "n-n"];
pub const INDEX_KINDS: [&str; 4] = ["primary", "unique", "index", "fulltext"];

fn default_dialect() -> String {
    "mysql".to_string()
}
fn default_node_kind() -> String {
    "table".to_string()
}
fn default_relation_kind() -> String {
    "1-n".to_string()
}
fn default_index_kind() -> String {
    "index".to_string()
}

/// 逻辑类型：跨方言的**内部**表示，导出时才映射成各家的具体类型。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbLogicalType {
    /// 见 docs/design/db-designer.md §2.3 白名单
    pub base: String,
    #[serde(default)]
    pub length: Option<i64>,
    #[serde(default)]
    pub precision: Option<i64>,
    #[serde(default)]
    pub scale: Option<i64>,
    #[serde(default)]
    pub unsigned: bool,
    /// enum 的取值列表
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbField {
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: DbLogicalType,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub pk: bool,
    #[serde(default)]
    pub auto_increment: bool,
    #[serde(default)]
    pub unique: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbIndex {
    pub name: String,
    #[serde(default = "default_index_kind")]
    pub kind: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbTableBody {
    #[serde(default)]
    pub fields: Vec<DbField>,
    #[serde(default)]
    pub indexes: Vec<DbIndex>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbViewBody {
    #[serde(default)]
    pub sql: String,
}

/// 一个节点 = 一张表 / 一个视图（函数不做，见 NODE_KINDS 注释）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbDesignNode {
    pub id: String,
    #[serde(default = "default_node_kind")]
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub comment: String,
    /// 标签：自由文本，画布上按标签筛选用（PowerDesigner 的对象分类）。
    /// 存字符串数组而不是固定枚举 —— 各项目对表的分类完全不一样，写死枚举反而不通用。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 主题色（`#rrggbb`）。连线用它着色：一条关联画成它所属表（外键所在那张）的颜色。
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub table: Option<DbTableBody>,
    #[serde(default)]
    pub view: Option<DbViewBody>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbRelationEnd {
    /// 节点 id
    pub node: String,
    /// **字段名**（不是字段 id）
    pub field: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbDesignRelation {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub from: DbRelationEnd,
    pub to: DbRelationEnd,
    #[serde(default = "default_relation_kind")]
    pub kind: String,
    #[serde(default)]
    pub on_delete: String,
    #[serde(default)]
    pub on_update: String,
    /// 这条关系是「镜像父表主键」产生的（从父表整表/全部主键锚点拖出来的），
    /// 且 `from` 那一列是**我们复制过去的副本**。
    ///
    /// 为什么需要这个标记：复合主键的引用 intent（"B 要跟着 A 的主键走"）没法从
    /// 单条字段对推断出来 —— 没有它就无法回答"A 又加了一个主键，B 要不要跟着加"。
    /// 字段级手动拖出来的关系 mirror = false：A 之后加主键**不**自动推给 B
    /// （那属于用户显式要的一部分）。
    #[serde(default)]
    pub mirror: bool,
}

/// 设计文档：应用内落库，同时可导出为 `.dbdesign.json` 给 Agent 用。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbDesignDocument {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_dialect")]
    pub dialect: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub nodes: Vec<DbDesignNode>,
    #[serde(default)]
    pub relations: Vec<DbDesignRelation>,
    #[serde(default)]
    pub updated_at: String,
}

impl DbDesignDocument {
    pub fn node(&self, id: &str) -> Option<&DbDesignNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// 可变版 node()：同步副本字段时需要就地改表体（`node()` 拿到的引用不能改）。
    pub fn node_mut(&mut self, id: &str) -> Option<&mut DbDesignNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// 表节点才有字段；视图 / 函数没有字段概念。
    pub fn fields_of(&self, node_id: &str) -> &[DbField] {
        match self.node(node_id).and_then(|n| n.table.as_ref()) {
            Some(t) => &t.fields,
            None => &[],
        }
    }
}
