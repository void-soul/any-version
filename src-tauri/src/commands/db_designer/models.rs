// 数据库设计器的数据模型（对外即 `.dbdesign.json` 交换格式）。
//
// 三条刻意的设计约束（详见 docs/design/db-designer.md）：
// 1. 字段类型存「逻辑类型 + 参数」而不是 `VARCHAR(255)` 整串 —— 导出四种方言要做映射，
//    存整串就只能靠字符串解析（脆）。
// 2. 连线引用「节点 id + **字段名**」而不是字段 id —— Agent 手写文件时不该先造 id。
//    代价：字段改名必须级联更新关联（见 store::rename_field）。
// 3. 视图 / 函数只存 SQL 文本，不做字段级建模 —— 否则等于要写一个 SQL 解析器。
use serde::{Deserialize, Deserializer, Serialize};

// ---------------------------------------------------------------------------
// 宽容反序列化
//
// 为什么需要：AI 产出里「该是字符串的地方给了别的 JSON 类型」是常态而非异常 ——
// `"default": false`（boolean 字段配 false 默认值）、`"default": 0`、`"length": "64"`、
// `"x": null`。serde 默认严格，撞上就整份解析失败，用户只看到一句
// 「AI 返回的不是合法设计文件：invalid type: boolean `false`, expected a string」——
// 几十秒的生成全部白费，而且提示词再怎么强调也压不住（模型并不觉得 boolean 默认值有错）。
//
// 这里统一**收成目标标量类型，转不了的当缺省**：设计文件里这些字段的表达能力不受影响
// （默认值本来就是拼进 SQL 的文本），却能把「一个字段的写法差异」从「整份产出作废」
// 降级成「按缺省处理」。反过来，把非法的 object/array 硬转成文本才是真的脏数据。
// ---------------------------------------------------------------------------

/// 标量 → 字符串（布尔/数字按 JSON 字面量转，如 `false` → `"false"`、`0` → `"0"`）。
/// 对象 / 数组**不转**：默认值是拼进 `DEFAULT ...` 的文本，把一坨 JSON 塞进去只会产出
/// 跑不起来的 SQL，宁可当没写。
fn scalar_to_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => Some(v.to_string()),
        _ => None,
    }
}

/// `Option<String>`：字符串原样，其它标量转文本，`null`/对象/数组 → `None`。
fn de_string_or_scalar<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(scalar_to_string(&v))
}

/// `Option<i64>`：数字、或「看起来是数字」的字符串（`"64"` / `"64.0"`）。
fn de_opt_i64<'de, D>(d: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match &v {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        serde_json::Value::String(s) => s
            .trim()
            .parse::<i64>()
            .ok()
            .or_else(|| s.trim().parse::<f64>().ok().map(|f| f as i64)),
        _ => None,
    })
}

/// `f64`：数字或数字字符串；`null` / 非法值 → `0`（坐标本来就靠前端自动布局兜底）。
fn de_f64<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match &v {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    })
}

/// `bool`：真正的布尔，或 `"true"` / `"false"` / `"0"` / `"1"` 这类常见写法。
fn de_bool<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match &v {
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        serde_json::Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" => true,
            "false" | "0" | "no" | "n" | "" => false,
            other => other.parse::<f64>().map(|n| n != 0.0).unwrap_or(false),
        },
        _ => false,
    })
}

/// `String`（非 Option 的文本字段）：`null` / 对象 / 数组 → 空串。
///
/// `#[serde(default)]` **只管「字段缺失」，管不了「字段在但值是 null」** —— 后者照样报
/// `invalid type: null, expected a string`。而 AI 写 `"comment": null`（想表达「没注释」）
/// 比写空串常见得多；`name` / `sql` 这类必填字段为 null 时留空串，让校验层去报错并说人话，
/// 比整份解析崩掉划算（用户看到的是「表 xxx 没有名称」这种能定位的信息）。
fn de_text<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(scalar_to_string(&v).unwrap_or_default())
}

/// `Vec<String>`：元素为字符串或数字（enum 取值写成 `[1, 2]` 时也能收）。
fn de_string_vec<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items.iter().filter_map(scalar_to_string).collect(),
        other => scalar_to_string(&other).into_iter().collect(),
    })
}

/// 文本为空时回落默认值（`kind` 这类枚举字段专用）。
///
/// 不能只按「空串」处理：节点 `kind` 为空时既过不了校验（`kind= 不在允许列表`），
/// 更糟的是导出按 `kind == "table"` 过滤，**空 kind 的表会被静默丢掉** ——
/// 用户看到的是「导出成功但少了一张表」，比报错更难查。
fn text_or<'de, D>(d: D, fallback: &str) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = de_text(d)?;
    Ok(if s.trim().is_empty() {
        fallback.to_string()
    } else {
        s
    })
}

fn de_node_kind<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    text_or(d, &default_node_kind())
}

fn de_index_kind<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    text_or(d, &default_index_kind())
}

fn de_relation_kind<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    text_or(d, &default_relation_kind())
}

/// 方言：空 / `null` → `mysql`（默认值），其它值原样保留交给校验层报错
/// —— 直接吞掉未知方言会让人以为设置生效了。
fn de_dialect<'de, D>(d: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    text_or(d, &default_dialect())
}

/// `Vec<T>`：`null` → 空数组；**逐元素解析，坏元素跳过而不是整份作废**。
///
/// 为什么单独一套而不是复用 `de_string_vec`：节点的元素是结构体，其中一个节点写坏了
/// 不该把另外 23 个好表一起判死刑（AI 在流式产出中途写错一个括号很常见）。
fn de_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|it| serde_json::from_value::<T>(it).ok())
            .collect(),
        _ => Vec::new(),
    })
}

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
    #[serde(deserialize_with = "de_text")]
    pub base: String,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub length: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub precision: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    pub scale: Option<i64>,
    #[serde(default, deserialize_with = "de_bool")]
    pub unsigned: bool,
    /// enum 的取值列表
    #[serde(default, deserialize_with = "de_string_vec")]
    pub values: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbField {
    #[serde(deserialize_with = "de_text")]
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: DbLogicalType,
    #[serde(default, deserialize_with = "de_bool")]
    pub nullable: bool,
    /// 默认值**统一按字符串存**（导出时原样拼进 `DEFAULT ...`）。
    /// 宽容反序列化：模型给 boolean 字段写 `"default": false` 是高频写法，
    /// 严格解析会让整份设计报「invalid type: boolean `false`, expected a string」作废。
    #[serde(default, deserialize_with = "de_string_or_scalar")]
    pub default: Option<String>,
    #[serde(default, deserialize_with = "de_text")]
    pub comment: String,
    #[serde(default, deserialize_with = "de_bool")]
    pub pk: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub auto_increment: bool,
    #[serde(default, deserialize_with = "de_bool")]
    pub unique: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbIndex {
    #[serde(deserialize_with = "de_text")]
    pub name: String,
    #[serde(default = "default_index_kind", deserialize_with = "de_index_kind")]
    pub kind: String,
    #[serde(default, deserialize_with = "de_string_vec")]
    pub fields: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbTableBody {
    #[serde(default, deserialize_with = "de_vec")]
    pub fields: Vec<DbField>,
    #[serde(default, deserialize_with = "de_vec")]
    pub indexes: Vec<DbIndex>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbViewBody {
    #[serde(default, deserialize_with = "de_text")]
    pub sql: String,
}

/// 一个节点 = 一张表 / 一个视图（函数不做，见 NODE_KINDS 注释）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbDesignNode {
    /// AI 生成时经常整个字段都不写（提示词要求了但模型会漏）。
    /// 所以缺 id 按「空」处理而不是解析失败 —— 解析失败会让整份产出作废，
    /// 而空 id 可以在收尾时补一个（`parse_document` / `extract_partial` 都会补）。
    #[serde(default, deserialize_with = "de_text")]
    pub id: String,
    #[serde(default = "default_node_kind", deserialize_with = "de_node_kind")]
    pub kind: String,
    #[serde(deserialize_with = "de_text")]
    pub name: String,
    #[serde(default, deserialize_with = "de_text")]
    pub comment: String,
    /// 标签：自由文本，画布上按标签筛选用（PowerDesigner 的对象分类）。
    /// 存字符串数组而不是固定枚举 —— 各项目对表的分类完全不一样，写死枚举反而不通用。
    #[serde(default, deserialize_with = "de_string_vec")]
    pub tags: Vec<String>,
    /// 主题色（`#rrggbb`）。连线用它着色：一条关联画成它所属表（外键所在那张）的颜色。
    #[serde(default, deserialize_with = "de_text")]
    pub color: String,
    #[serde(default, deserialize_with = "de_f64")]
    pub x: f64,
    #[serde(default, deserialize_with = "de_f64")]
    pub y: f64,
    #[serde(default)]
    pub table: Option<DbTableBody>,
    #[serde(default)]
    pub view: Option<DbViewBody>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DbRelationEnd {
    /// 节点 id
    #[serde(default, deserialize_with = "de_text")]
    pub node: String,
    /// **字段名**（不是字段 id）
    #[serde(default, deserialize_with = "de_text")]
    pub field: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbDesignRelation {
    /// 同上：缺 id 当空，收尾时补。
    #[serde(default, deserialize_with = "de_text")]
    pub id: String,
    #[serde(default, deserialize_with = "de_text")]
    pub name: String,
    #[serde(default)]
    pub from: DbRelationEnd,
    #[serde(default)]
    pub to: DbRelationEnd,
    #[serde(default = "default_relation_kind", deserialize_with = "de_relation_kind")]
    pub kind: String,
    #[serde(default, deserialize_with = "de_text")]
    pub on_delete: String,
    #[serde(default, deserialize_with = "de_text")]
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
    /// 与节点 id 同理：AI 常漏写顶层 id，缺了就当空，由解析侧补一个。
    #[serde(default, deserialize_with = "de_text")]
    pub id: String,
    #[serde(default, deserialize_with = "de_text")]
    pub name: String,
    #[serde(default, deserialize_with = "de_text")]
    pub description: String,
    #[serde(default = "default_dialect", deserialize_with = "de_dialect")]
    pub dialect: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    /// **公共字段**（导出时自动附加到每张表，见 `common_fields.rs`）。
    ///
    /// 存完整定义而不是「勾了哪几个 key」：文件自包含 —— 任何工具 / 任何人都能读懂
    /// 这份设计到底往每张表加了什么，不必知道应用的内置清单。代价是内置清单以后升级
    /// 不会自动反映到已有文件上（那也正是「自包含」想要的确定性）。
    ///
    /// **缺省 = 空数组 = 不附加**，所以旧文件与不配的设计完全不受影响。
    #[serde(default, deserialize_with = "de_vec")]
    pub common_fields: Vec<DbField>,
    /// 设计决策与约束：一句话一条记「为什么这么设计」，AI 每次生成都读它。
    ///
    /// 存成数组而不是一段文本：便于增删、便于侧栏逐条展示与删除。
    /// 缺省空数组 → 旧设计文件零影响。
    ///
    /// **不要指望模型原样带回**：它每轮都要回吐整份文档，很可能把你的决策丢了或改写。
    /// 真正的所有权在前端 —— 保留原文档的这份，只追加模型本轮新增的。
    #[serde(default, deserialize_with = "de_string_vec")]
    pub decisions: Vec<String>,
    #[serde(default, deserialize_with = "de_vec")]
    pub nodes: Vec<DbDesignNode>,
    #[serde(default, deserialize_with = "de_vec")]
    pub relations: Vec<DbDesignRelation>,
    #[serde(default, deserialize_with = "de_text")]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn field(v: serde_json::Value) -> DbField {
        serde_json::from_value(v).expect("字段必须能解析")
    }

    /// 回归（Q：滑雪助手那份产出）：`"default": false` 让整份设计报
    /// 「invalid type: boolean `false`, expected a string」作废 —— boolean 字段配false
    /// 默认值是模型最自然的写法，必须能收成字符串 `"false"`。
    #[test]
    fn default_accepts_boolean_and_number() {
        let f = field(serde_json::json!({
            "name": "need_dynamic_capture", "type": { "base": "boolean" }, "default": false
        }));
        assert_eq!(f.default.as_deref(), Some("false"));
        let f = field(serde_json::json!({
            "name": "sort_order", "type": { "base": "int" }, "default": 0
        }));
        assert_eq!(f.default.as_deref(), Some("0"));
        // null / 缺失都按「无默认值」处理，而不是报错
        let f = field(serde_json::json!({ "name": "a", "type": { "base": "int" }, "default": null }));
        assert_eq!(f.default, None);
        let f = field(serde_json::json!({ "name": "a", "type": { "base": "int" } }));
        assert_eq!(f.default, None);
    }

    /// 其余容易被模型写成别的标量的字段也不能因此作废整份设计。
    #[test]
    fn scalar_fields_tolerate_wrong_json_type() {
        let f = field(serde_json::json!({
            "name": "user_name", "type": { "base": "varchar", "length": "64" },
            "nullable": "false", "pk": true, "unique": "1", "autoIncrement": null
        }));
        assert_eq!(f.r#type.length, Some(64));
        assert!(!f.nullable);
        assert!(f.pk);
        assert!(f.unique, "\"1\" 也是真");
        assert!(!f.auto_increment);

        let f = field(serde_json::json!({
            "name": "tag", "type": { "base": "enum", "values": [1, "a"] }
        }));
        assert_eq!(f.r#type.values, vec!["1".to_string(), "a".to_string()]);

        let n: DbDesignNode = serde_json::from_value(serde_json::json!({
            "name": "t", "x": null, "y": "12.5"
        }))
        .expect("坐标为 null / 字符串不该让解析失败");
        assert_eq!(n.x, 0.0);
        assert_eq!(n.y, 12.5);
    }

    /// 宽容不能反噬：对象 / 数组当默认值毫无意义，按「无默认值」处理，
    /// 不能把一坨 JSON 文本塞进 `DEFAULT ` 后面污染导出的 SQL。
    #[test]
    fn non_scalar_default_is_dropped_not_stringified() {
        let f = field(serde_json::json!({
            "name": "a", "type": { "base": "int" }, "default": {"x": 1}
        }));
        assert_eq!(f.default, None);
    }

    /// `#[serde(default)]` 只管「字段缺失」，**管不了「字段在但值是 null」** ——
/// 而 AI 写 `"comment": null`（想表达「没注释」）远比写空串常见。
    #[test]
    fn explicit_null_does_not_break_parsing() {
        let doc: DbDesignDocument = serde_json::from_value(serde_json::json!({
            "name": null, "dialect": null, "description": null,
            "nodes": [{
                "id": null, "kind": null, "name": "user", "comment": null,
                "color": null, "tags": null, "x": null, "y": null,
                "table": { "fields": [{ "name": "user_id", "type": { "base": null }, "comment": null }], "indexes": null }
            }],
            "relations": null
        }))
        .expect("显式 null 不该让整份设计作废");
        // dialect 为 null 时回落到默认方言，而不是留下空串让校验报「未知方言 」
        assert_eq!(doc.dialect, "mysql");
        assert_eq!(doc.relations.len(), 0);
        let n = &doc.nodes[0];
        assert_eq!(n.kind, "table");
        assert_eq!(n.tags.len(), 0);
        let f = &n.table.as_ref().unwrap().fields[0];
        assert_eq!(f.comment, "");
        assert_eq!(f.r#type.base, "", "类型为 null 时留空，由校验层报「不在白名单」而不是解析崩");
    }

    /// 一个节点写坏了，不该把另外 23 个好表一起判死刑。
    #[test]
    fn broken_node_does_not_take_the_whole_document_down() {
        let doc: DbDesignDocument = serde_json::from_value(serde_json::json!({
            "name": "混了坏节点的产出",
            "nodes": [
                { "name": "user", "table": { "fields": [] } },
                "这不是对象",
                { "name": "order", "table": { "fields": [] } }
            ]
        }))
        .expect("坏元素应被跳过而不是整份失败");
        assert_eq!(doc.nodes.len(), 2);
        assert_eq!(doc.nodes[0].name, "user");
        assert_eq!(doc.nodes[1].name, "order");
    }

    /// 端到端：一份含布尔默认值、数字默认值、null 坐标的产出要能整体解析成设计文件。
    #[test]
    fn document_with_mixed_scalar_defaults_parses() {
        let json = serde_json::json!({
            "name": "示例", "dialect": "mysql",
            "nodes": [{
                "kind": "table", "name": "posture", "x": null, "y": 40,
                "table": { "fields": [
                    { "name": "posture_id", "type": { "base": "bigint" }, "pk": true, "autoIncrement": true },
                    { "name": "need_dynamic_capture", "type": { "base": "boolean" }, "nullable": false, "default": false },
                    { "name": "posture_score", "type": { "base": "decimal", "precision": 5, "scale": 2 }, "default": 0 }
                ] }
            }],
            "relations": []
        });
        let doc: DbDesignDocument =
            serde_json::from_value(json).expect("布尔默认值不该让整份设计作废");
        let node = &doc.nodes[0];
        let fields = &node.table.as_ref().unwrap().fields;
        assert_eq!(fields[1].default.as_deref(), Some("false"));
        assert_eq!(fields[2].default.as_deref(), Some("0"));
        assert_eq!(node.x, 0.0);
        assert_eq!(node.y, 40.0);
    }
}
