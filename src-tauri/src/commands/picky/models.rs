use super::*;

// ─── 模型（camelCase，与 Flutter 端一致） ───

/// 与 Flutter 端 `toJson` 的 0/1 整数表示互转：
/// - 反序列化：兼容 `0/1` 整数与 `true/false` 布尔（Flutter 输出 0/1，旧数据可能是布尔）；
/// - 序列化：统一输出 0/1 整数，保证 any-version 上传的数据 Flutter 端 `(x as int?)==1` 可读。
pub(super) fn de_bool_01<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().map(|x| x != 0).unwrap_or(false),
        serde_json::Value::Null => false,
        _ => false,
    })
}

pub(super) fn ser_bool_01<S>(b: &bool, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_i64(if *b { 1 } else { 0 })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub favicon_url: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(
        default,
        deserialize_with = "de_bool_01",
        serialize_with = "ser_bool_01"
    )]
    pub refined: bool,
    #[serde(
        default,
        deserialize_with = "de_bool_01",
        serialize_with = "ser_bool_01"
    )]
    pub meta_fetched: bool,
    /// 页面正文（纯文本）。PC 端与专用 APP 均通过 S3 同步此字段；
    /// 由抓取链路（WebView/无头浏览器）提取主内容后回填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 保留 Flutter 端的个性化等未知字段（lingzuCode / aiCopy 等），同步时原样带回。
    #[serde(flatten)]
    pub extra: JsonMap<String, JsonValue>,
}

impl Default for Bookmark {
    fn default() -> Self {
        let now = now_iso();
        Self {
            id: uuid_v4(),
            title: String::new(),
            description: None,
            url: None,
            image_url: None,
            favicon_url: None,
            created_at: now.clone(),
            updated_at: now,
            refined: false,
            meta_fetched: false,
            content: None,
            extra: JsonMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    #[serde(default)]
    pub bookmark_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_tag_color")]
    pub color: String,
    #[serde(default)]
    pub created_at: String,
}

pub(super) fn default_tag_color() -> String {
    "#4FC3F7".to_string()
}

/// 全量状态（前端一次性加载 + 同步用）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PickyState {
    pub bookmarks: Vec<Bookmark>,
    pub comments: Vec<Comment>,
    pub tags: Vec<Tag>,
    pub bookmark_tags: HashMap<String, Vec<String>>,
}
