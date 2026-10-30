//! 收藏模块本地库：`get_data_dir()/favorites.db`。
//!
//! 所有函数都接受 `&Connection`，便于用内存库做单测；命令层通过 [`with_conn`] 拿全局连接。

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::commands::config::get_data_dir;

/// 全局连接（Mutex 保护）。WAL 模式，读写不互相阻塞。
static DB_CONN: Mutex<Option<Connection>> = Mutex::new(None);

fn db_path() -> PathBuf {
    get_data_dir().join("favorites.db")
}

/// unix 秒 → 本地时间字符串（与 [`now_str`] 同一格式，便于直接按字符串排序与比较）。
pub fn unix_to_local_str(secs: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| {
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
    })
}

/// RFC3339（GitHub 的 `starred_at` 形如 `2021-01-01T00:00:00Z`）→ 本地时间字符串。
pub fn rfc3339_to_local_str(value: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value).ok().map(|dt| {
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
    })
}

/// 打开/初始化全局连接（不持有锁；由调用方在锁内调用）。
fn init_connection() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建数据目录失败: {}", e))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开收藏库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("设置收藏库超时失败: {}", e))?;
    migrate(&conn)?;
    Ok(conn)
}

/// 在全局连接上执行一段逻辑（首次调用自动初始化）。
/// 取连接执行一段逻辑。
///
/// 传 **可变引用**：删除条目要在同一事务里「删行 + 立墓碑」，事务需要 `&mut Connection`。
/// 只读/只写的闭包照样能用（`&mut` 会自动退化成 `&`），调用方无需改。
pub fn with_conn<T>(f: impl FnOnce(&mut Connection) -> Result<T, String>) -> Result<T, String> {
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    if guard.is_none() {
        *guard = Some(init_connection()?);
    }
    let conn = guard.as_mut().expect("连接已初始化");
    f(conn)
}

/// 一条待落库的收藏条目（各源统一形态）。
#[derive(Debug, Clone)]
pub struct NewFavorite {
    pub source: String,
    /// 平台**原生 id**：GitHub 用仓库 id（不用 full_name，改名会变），B站用 bvid，知乎用内容 id
    pub external_id: String,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    /// 各源自定的附加字段（语言 / topics / 分区…），存 JSON 字符串
    pub extra_json: Option<String>,
    /// 平台记录的**收藏时间**（本地时间字符串）。取不到时为 None：
    /// 此时不覆盖库里已有的值，展示与排序回退到 created_at。
    pub favorited_at: Option<String>,
    /// 导入时就能确定失效的（B站内容被 UP 主删除）直接落这个状态，省一次探测
    pub initial_status: Option<String>,
}

/// 送进 AI 归类的最小信息（只带判断分类需要的字段）。
#[derive(Debug, Clone)]
pub struct ClassifyItem {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub language: String,
    pub topics: Vec<String>,
    /// GitHub star 数（热度参考）；视频/文章条目为 None
    pub stars: Option<i64>,
    /// 原始链接。浏览器书签没有语言/topics/简介，域名往往是唯一的分类线索
    /// （`docs.python.org` → Python 文档），所以一并喂给模型。
    pub url: String,
}

/// upsert 的结果：新增 / 有变化已更新 / 完全没变（跨次导入去重的正常结局）/
/// **已被用户删除**（命中墓碑，导入必须跳过，否则删掉的条目会复活）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Added,
    Updated,
    Skipped,
    Deleted,
}

/// 墓碑列表项（设置页展示 + 恢复用）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteDeletedRow {
    pub source: String,
    pub external_id: String,
    pub title: Option<String>,
    pub deleted_at: String,
}

fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 一条凭证的健康状态（供 UI 提示「该换 Cookie 了」）。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub source: String,
    pub configured: bool,
    /// ok | expired | unknown
    pub status: String,
    pub checked_at: Option<String>,
    /// 从 Cookie 里解析出的过期时间（unix 秒）
    pub expires_at: Option<i64>,
    pub updated_at: Option<String>,
}

/// 正文缓存（返回给前端的形态）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedContent {
    pub text: String,
    /// 来源标注：知乎是收藏夹名，GitHub 是 README 文件名
    pub label: Option<String>,
    pub fetched_at: String,
}

/// 列表用的一条收藏（含标签）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteRow {
    pub id: i64,
    pub source: String,
    pub external_id: String,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub checked_at: Option<String>,
    pub ai_locked: bool,
    pub ai_model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// 平台记录的收藏时间；取不到为 None（前端回退显示 created_at）
    pub favorited_at: Option<String>,
    /// 特别关注：列表里置顶，不会被新收藏的涌到后面去
    pub is_starred: bool,
    pub tags: Vec<String>,
}

/// 列表筛选条件（全部可选，不传就是不过滤）。
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub source: Option<String>,
    /// 按分类筛选（**含其所有子分类**）：分类是树，点父级就该看到子级的东西。
    pub category_id: Option<i64>,
    pub status: Option<String>,
    pub keyword: Option<String>,
    /// 排序方式：已收敛为「按收藏时间」，此字段保留仅为兼容前端调用契约。
    pub sort: Option<String>,
    /// 只保留「收藏时间（取不到则入库时间）不早于该时刻」的条目，本地时间字符串
    pub favorited_since: Option<String>,
    /// 只看星标条目（左侧「星标」入口）
    pub starred_only: bool,
    pub limit: usize,
}

// ─── 分类树（多级分类，操作逻辑对齐启动模块的 classification） ───

/// 分类树节点（给前端直接渲染用，`children` 已递归排好序）。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryNode {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort_order: i32,
    /// topic = 主题分类（AI 归类会写）；follow = 关注分组（人工维护，AI 归类不碰）
    pub kind: String,
    /// 直接挂在本分类下的条目数
    pub count: i64,
    /// 含所有子孙分类的条目数（前端侧栏显示这个才有意义：点父级要看全部）
    pub total: i64,
    pub children: Vec<CategoryNode>,
}

/// 扁平分类行（内部用）。
struct CategoryFlat {
    id: i64,
    parent_id: Option<i64>,
    name: String,
    sort_order: i32,
    /// topic / follow
    kind: String,
    count: i64,
}

/// 分类概览（供「整理类目」把整棵树的上下文喂给模型）。
///
/// 只带 ID、名称、上级 ID 与内部路径：整理只需要改父子关系，
/// 条目数 / 示例标题不进 prompt，避免分类一多就把上下文撑爆。
#[derive(Debug, Clone, Default)]
pub struct CategoryBrief {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub kind: String,
    /// 内部用于预览展示的完整路径（如 `编程语言/Rust`），不发模型
    pub path: String,
}

/// 一次「只改上级」的分类移动。
///
/// 刻意**不含**名称/新建/删除字段：整理类目只允许改 `parent_id`，
/// 分类 ID 与条目关联都不动，避免条目指向不存在的分类。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryMove {
    pub id: i64,
    pub parent_id: Option<i64>,
    /// 模型给出的理由（前端应用时可不传）
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteStats {
    pub total: usize,
    pub unclassified: usize,
    pub gone: usize,
    pub by_source: Vec<(String, usize)>,
    /// 兼容旧字段：扁平的「分类名 → 条数」。新前端一律读 `categories`。
    pub tags: Vec<(String, usize)>,
    /// 分类树：侧栏按它渲染层级，条数是**含子分类**的合计。
    pub categories: Vec<CategoryNode>,
}

mod migrations;
pub use migrations::*;
mod deleted;
pub use deleted::*;
mod items;
pub use items::*;
mod classification;
pub use classification::*;
mod categories;
pub use categories::*;
mod credentials;
pub use credentials::*;
mod content;
pub use content::*;
mod statistics;
pub use statistics::*;
#[cfg(test)]
mod tests;
