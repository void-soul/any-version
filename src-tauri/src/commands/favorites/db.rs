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

/// 建表（幂等）。三张表：条目、标签（多标签）、导入进度。
pub fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS favorite (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            source       TEXT    NOT NULL,
            external_id  TEXT    NOT NULL,
            url          TEXT    NOT NULL,
            title        TEXT    NOT NULL,
            subtitle     TEXT,
            description  TEXT,
            extra_json   TEXT,
            status       TEXT    NOT NULL DEFAULT 'ok',
            checked_at   TEXT,
            ai_locked    INTEGER NOT NULL DEFAULT 0,
            ai_model     TEXT,
            ai_at        TEXT,
            created_at   TEXT    NOT NULL,
            updated_at   TEXT    NOT NULL,
            -- 平台记录的**收藏时间**（GitHub starred_at / B站 fav_time / 知乎 created）。
            -- 老库与取不到时间的来源为 NULL，展示与排序回退到 created_at（本地首次入库时间）。
            favorited_at TEXT,
            UNIQUE(source, external_id)
        );
        CREATE INDEX IF NOT EXISTS idx_favorite_source       ON favorite(source);
        CREATE INDEX IF NOT EXISTS idx_favorite_status       ON favorite(status);
        CREATE INDEX IF NOT EXISTS idx_favorite_ai_locked    ON favorite(ai_locked);

        -- 多标签：一个条目可以同时属于多个分类，不设「主分类」。
        -- 历史遗留：分类改成树（favorite_category）后由 favorite_item_category 接管，
        -- 这张表只用于一次性迁移，之后不再写入。
        CREATE TABLE IF NOT EXISTS favorite_tag (
            favorite_id INTEGER NOT NULL,
            tag         TEXT    NOT NULL,
            PRIMARY KEY (favorite_id, tag)
        );
        CREATE INDEX IF NOT EXISTS idx_favorite_tag_tag ON favorite_tag(tag);

        -- 多级分类树（操作逻辑对齐启动模块的 launcher_classification）
        CREATE TABLE IF NOT EXISTS favorite_category (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            parent_id  INTEGER,
            name       TEXT    NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT    NOT NULL,
            UNIQUE(parent_id, name)
        );
        CREATE INDEX IF NOT EXISTS idx_fav_category_parent ON favorite_category(parent_id);

        -- 条目 ↔ 分类（多对多）
        -- source 记录这条关联是「谁」建立的：ai（AI 归类）/ bookmark（浏览器书签目录）/ manual（人工）。
        -- 「重新归类」只清 source='ai' 的关联，书签目录与人工选择都保留。
        CREATE TABLE IF NOT EXISTS favorite_item_category (
            favorite_id INTEGER NOT NULL,
            category_id INTEGER NOT NULL,
            source      TEXT    NOT NULL DEFAULT 'manual',
            PRIMARY KEY (favorite_id, category_id)
        );
        CREATE INDEX IF NOT EXISTS idx_fav_item_category_cat ON favorite_item_category(category_id);

        CREATE TABLE IF NOT EXISTS favorite_import_state (
            source      TEXT PRIMARY KEY,
            cursor      TEXT,
            last_run_at TEXT,
            total       INTEGER NOT NULL DEFAULT 0
        );

        -- 各平台 Cookie（B站 SESSDATA / 知乎 z_c0）。**只存不打印**，
        -- 日志与错误信息里一律不出现它的内容。
        CREATE TABLE IF NOT EXISTS favorite_credential (
            source      TEXT PRIMARY KEY,
            cookie      TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            -- ok | expired | unknown：最近一次用它的结果，用于主动提醒「该换 Cookie 了」
            status      TEXT NOT NULL DEFAULT 'unknown',
            checked_at  TEXT,
            -- 从 Cookie 里能解析出的过期时间（unix 秒）；解析不出为 NULL
            expires_at  INTEGER
        );

        -- 条目正文缓存：知乎收藏的 content（HTML+纯文本）、GitHub 的 README。
        -- 单独一张表而不是塞进 favorite.extra_json：正文动辄几十 KB，
        -- 列表查询只要元数据，混在一起会让每次列表都拖着大字段走。
        CREATE TABLE IF NOT EXISTS favorite_content (
            favorite_id INTEGER PRIMARY KEY,
            -- 展示用的纯文本（已剥标签）
            text        TEXT NOT NULL,
            -- 原始 HTML（目前只在需要时留档，前端一律渲染 text，避免注入远端 HTML）
            html        TEXT,
            -- 来源标注，如 `README_CN.md` / 收藏夹名
            label       TEXT,
            fetched_at  TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("初始化收藏库失败: {}", e))?;

    // 老库补列：`CREATE TABLE IF NOT EXISTS` 对已存在的表不会加新列。
    for (column, ddl) in [
        ("status", "ALTER TABLE favorite_credential ADD COLUMN status TEXT NOT NULL DEFAULT 'unknown'"),
        ("checked_at", "ALTER TABLE favorite_credential ADD COLUMN checked_at TEXT"),
        ("expires_at", "ALTER TABLE favorite_credential ADD COLUMN expires_at INTEGER"),
    ] {
        if !has_column(conn, "favorite_credential", column)? {
            conn.execute(ddl, [])
                .map_err(|e| format!("升级收藏库失败（{}）: {}", column, e))?;
        }
    }
    // favorite 表同样要补「收藏时间」列（见建表注释）
    if !has_column(conn, "favorite", "favorited_at")? {
        conn.execute("ALTER TABLE favorite ADD COLUMN favorited_at TEXT", [])
            .map_err(|e| format!("升级收藏库失败（favorited_at）: {}", e))?;
    }
    // 索引必须等补列之后再建：老库上表里还没有这一列时，建索引会直接报错
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_favorite_favorited ON favorite(favorited_at)",
        [],
    )
    .map_err(|e| format!("初始化收藏库失败: {}", e))?;
    // favorite_item_category 补「来源」列：区分 AI / 书签目录 / 人工，「重新归类」只清 AI 那部分。
    if !has_column(conn, "favorite_item_category", "source")? {
        conn.execute(
            "ALTER TABLE favorite_item_category ADD COLUMN source TEXT NOT NULL DEFAULT 'manual'",
            [],
        )
        .map_err(|e| format!("升级收藏库失败（item_category.source）: {}", e))?;
        // 回填来源：老数据没记来源，按条目当前状态推断——
        // 人工锁定 → manual；AI 归类过（有 ai_model）→ ai；其余（书签目录、还没 AI 归类）→ bookmark。
        // 局限：书签条目若同时被旧版 AI 归类过，其书签目录关联也会被推成 ai，
        // 重新归类时会被一并清掉，属于可接受的旧数据边缘情况（书签目录可重新导入恢复）。
        conn.execute(
            "UPDATE favorite_item_category SET source = CASE \
                WHEN EXISTS (SELECT 1 FROM favorite f WHERE f.id = favorite_id AND f.ai_locked = 1) THEN 'manual' \
                WHEN EXISTS (SELECT 1 FROM favorite f WHERE f.id = favorite_id AND f.ai_model IS NOT NULL) THEN 'ai' \
                ELSE 'bookmark' END",
            [],
        )
        .map_err(|e| format!("回填分类来源失败: {}", e))?;
    }
    migrate_tags_to_categories(conn)?;
    Ok(())
}

/// 老库升级：把「扁平标签」搬进分类树。
///
/// 幂等：只要分类树里已经有东西就认为搬过了（新库自然是空的，也走不到插入）。
/// 旧标签全部建成**顶层**分类——老数据里没有层级信息，硬猜父子关系只会猜错。
fn migrate_tags_to_categories(conn: &Connection) -> Result<(), String> {
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM favorite_category", [], |r| r.get(0))
        .unwrap_or(0);
    if existing > 0 {
        return Ok(());
    }
    let Ok(mut stmt) = conn.prepare("SELECT favorite_id, tag FROM favorite_tag") else {
        return Ok(());
    };
    let rows = match stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    let mut pairs: Vec<(i64, String)> = Vec::new();
    for row in rows {
        if let Ok(p) = row {
            pairs.push(p);
        }
    }
    if pairs.is_empty() {
        return Ok(());
    }
    for (favorite_id, tag) in pairs {
        let category_id = resolve_category_by_name(conn, &tag)?;
        conn.execute(
            "INSERT OR IGNORE INTO favorite_item_category (favorite_id, category_id) VALUES (?1, ?2)",
            rusqlite::params![favorite_id, category_id],
        )
        .map_err(|e| format!("迁移旧分类标签失败: {e}"))?;
    }
    eprintln!("[favorites] 旧标签已迁移为分类树");
    Ok(())
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
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        })
}

/// 表里有没有这一列（用于幂等升级老库）。
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| format!("读取表结构失败: {}", e))?;
    let mut rows = statement
        .query([])
        .map_err(|e| format!("读取表结构失败: {}", e))?;
    while let Some(row) = rows.next().map_err(|e| format!("读取表结构失败: {}", e))? {
        let name: String = row.get(1).map_err(|e| format!("读取表结构失败: {}", e))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
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
pub fn with_conn<T>(f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    if guard.is_none() {
        *guard = Some(init_connection()?);
    }
    let conn = guard.as_ref().expect("连接已初始化");
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

/// upsert 的结果：新增 / 有变化已更新 / 完全没变（跨次导入去重的正常结局）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Added,
    Updated,
    Skipped,
}

fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 写入一条条目：已存在则按可变字段更新，完全没变则跳过。
///
/// 这是「重复导入不进重复记录」的唯一保证——依赖 `(source, external_id)` 唯一约束，
/// 而不是先查后写（避免并发下两边都查不到）。
pub fn upsert(conn: &Connection, item: &NewFavorite) -> Result<UpsertOutcome, String> {
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO favorite \
             (source, external_id, url, title, subtitle, description, extra_json, status, favorited_at, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(?8, 'ok'), ?9, ?10, ?10)",
            rusqlite::params![
                item.source,
                item.external_id,
                item.url,
                item.title,
                item.subtitle,
                item.description,
                item.extra_json,
                item.initial_status,
                item.favorited_at,
                now_str(),
            ],
        )
        .map_err(|e| format!("写入收藏条目失败: {}", e))?;
    if inserted > 0 {
        return Ok(UpsertOutcome::Added);
    }

    let current: (String, String, String, String, String, String) = conn
        .query_row(
            "SELECT url, title, COALESCE(subtitle, ''), COALESCE(description, ''), \
                    COALESCE(extra_json, ''), COALESCE(favorited_at, '') \
             FROM favorite WHERE source = ?1 AND external_id = ?2",
            rusqlite::params![item.source, item.external_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|e| format!("读取已存在收藏条目失败: {}", e))?;

    // 收藏时间也参与「有没有变化」的比较：老库（或旧版本导入）里该列为空时，
    // 这次带上了平台时间就算一次有效更新，从而把历史数据补齐。
    let incoming = (
        item.url.clone(),
        item.title.clone(),
        item.subtitle.clone().unwrap_or_default(),
        item.description.clone().unwrap_or_default(),
        item.extra_json.clone().unwrap_or_default(),
        item.favorited_at.clone().unwrap_or_default(),
    );
    if current == incoming {
        return Ok(UpsertOutcome::Skipped);
    }

    conn.execute(
        "UPDATE favorite SET url = ?1, title = ?2, subtitle = ?3, description = ?4, \
         extra_json = ?5, favorited_at = COALESCE(?9, favorited_at), updated_at = ?6 \
         WHERE source = ?7 AND external_id = ?8",
        rusqlite::params![
            incoming.0,
            incoming.1,
            item.subtitle,
            item.description,
            item.extra_json,
            now_str(),
            item.source,
            item.external_id,
            // 本次没取到时间就保留库里已有的，别把知道的覆盖成 NULL
            item.favorited_at,
        ],
    )
    .map_err(|e| format!("更新收藏条目失败: {}", e))?;
    Ok(UpsertOutcome::Updated)
}

/// 「重新归类」准备：把待重归条目（未锁定、未失效、且 AI 归类过）的 AI 标记清空，
/// 让它们重新进入「未归类」批次（`ai_model IS NULL`），由正常的归类循环重新处理。
///
/// 旧的 AI 分类关联不在这里清——由 [`apply_tags`] 在逐批写入时按 `source='ai'` 清掉，
/// 这样「停止」中断时，还没处理到的条目仍保留旧分类，下次重归再接着清。
/// 返回被重置的条数。
pub fn reset_classification(conn: &Connection) -> Result<usize, String> {
    conn.execute(
        "UPDATE favorite SET ai_model = NULL, ai_at = NULL \
         WHERE ai_locked = 0 AND status != 'gone' AND ai_model IS NOT NULL",
        [],
    )
    .map_err(|e| format!("重置归类标记失败: {e}"))
}

/// 清理所有「空」分类（既没有条目关联、也没有子分类）。
///
/// 「重新归类」会清掉旧的 AI 关联：被 AI 用过、现在不再有内容的分类（含历史遗留的）
/// 会残留成空节点，侧栏看着乱。这里把这类分类**连同跟着变空的祖先**一起收掉，
/// 反复扫描直到没有可删的为止；只要某层还有条目或子分类就停，所以有内容的分类不受影响。
///
/// 注意：这会把「用户手动创建但还没归类任何条目」的空分类也一并收掉——重新归类
/// 本就是把分类树重新整理的语义，空分类一律清掉才符合「别留乱」的预期。
/// 返回删除的分类数。
pub fn prune_all_empty_categories(conn: &Connection) -> Result<usize, String> {
    let mut removed = 0usize;
    loop {
        let found = conn.query_row(
            "SELECT c.id FROM favorite_category c \
             WHERE NOT EXISTS (SELECT 1 FROM favorite_item_category ic WHERE ic.category_id = c.id) \
               AND NOT EXISTS (SELECT 1 FROM favorite_category child WHERE child.parent_id = c.id) \
             LIMIT 1",
            [],
            |r| r.get::<_, i64>(0),
        );
        match found {
            Ok(id) => {
                conn.execute("DELETE FROM favorite_category WHERE id = ?1", [id])
                    .map_err(|e| format!("删除空分类失败: {e}"))?;
                removed += 1;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => break,
            Err(e) => return Err(format!("查找空分类失败: {e}")),
        }
    }
    Ok(removed)
}

/// 取一批「待归类」条目：**还没被 AI 归类过** 且 **未被人工锁定** 且 **未失效**。
///
/// 判据是 `ai_model IS NULL`（而不是「没有任何分类」）：浏览器书签导入时**自带**
/// 目录分类（书签栏/技术/GitHub），如果用「无分类」判据，这批条目永远进不了归类批次，
/// 等于在收藏库里当二等公民。AI 归类是在已有分类之上**追加**主题分类（多标签语义）。
///
/// - 人工改过分类的条目（`ai_locked = 1`）永不再进入归类批次——用户的选择优先于模型；
/// - 已失效（`gone`）的条目同样跳过：给一个打不开的仓库归类没有意义，还白花 token。
///
/// 「重新归类」在开工前先调 [`reset_classification`] 把已归类条目的 `ai_model` 清空，
/// 于是它们也会被这里选中——重归复用同一条「逐批取未归类」的进度推进，不会死循环。
pub fn select_unclassified(conn: &Connection, limit: usize) -> Result<Vec<ClassifyItem>, String> {
    let mut statement = conn
        .prepare(
            "SELECT f.id, f.title, COALESCE(f.description, ''), COALESCE(f.extra_json, ''), f.url \
             FROM favorite f \
             WHERE f.ai_locked = 0 \
               AND f.status != 'gone' \
               AND f.ai_model IS NULL \
             ORDER BY f.id LIMIT ?1",
        )
        .map_err(|e| format!("查询待归类条目失败: {}", e))?;
    let rows = statement
        .query_map([limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| format!("查询待归类条目失败: {}", e))?;

    let mut items = Vec::new();
    for row in rows {
        let (id, title, description, extra_json, url) =
            row.map_err(|e| format!("读取待归类条目失败: {}", e))?;
        let extra: serde_json::Value =
            serde_json::from_str(&extra_json).unwrap_or(serde_json::Value::Null);
        let language = extra
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let topics = extra
            .get("topics")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|t| t.as_str())
                    .map(|t| t.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let stars = extra
            .get("stars")
            .and_then(|v| v.as_i64())
            .filter(|count| *count > 0);
        items.push(ClassifyItem {
            id,
            title,
            description,
            language,
            topics,
            stars,
            url,
        });
    }
    Ok(items)
}

/// 写入一批归类结果（多标签：一个条目可落多个分类），并记录所用模型。
///
/// `groups` 的每组是 `(分类路径段, 条目下标列表)`：路径逐级 `ensure_category` 建树，
/// 条目挂到叶子分类。`reclassify = true` 时先把本批条目旧的 `ai` 来源关联清掉，
/// 书签目录与人工选择（`bookmark` / `manual`）保留。
pub fn apply_tags(
    conn: &Connection,
    items: &[ClassifyItem],
    groups: &[(Vec<String>, Vec<usize>)],
    model: &str,
    reclassify: bool,
) -> Result<usize, String> {
    let written = now_str();
    let mut count = 0usize;

    // 重新归类：清掉旧的 AI 分类（只清 ai 来源，书签目录与人工选择不动）
    if reclassify {
        for item in items {
            conn.execute(
                "DELETE FROM favorite_item_category WHERE favorite_id = ?1 AND source = 'ai'",
                [item.id],
            )
            .map_err(|e| format!("清除旧 AI 分类失败: {}", e))?;
        }
    }

    for (path, indices) in groups {
        // 模型给的是「/」分隔的多级分类名：逐级 ensure（同名复用），条目挂到叶子
        let mut parent: Option<i64> = None;
        let mut category_id: i64 = 0;
        for seg in path {
            category_id = ensure_category(conn, parent, seg)?;
            parent = Some(category_id);
        }
        for index in indices {
            let Some(item) = items.get(*index) else {
                continue;
            };
            conn.execute(
                "INSERT OR IGNORE INTO favorite_item_category (favorite_id, category_id, source) VALUES (?1, ?2, 'ai')",
                rusqlite::params![item.id, category_id],
            )
            .map_err(|e| format!("写入分类失败: {}", e))?;
            count += 1;
        }
    }
    for item in items {
        conn.execute(
            "UPDATE favorite SET ai_model = ?1, ai_at = ?2 WHERE id = ?3",
            rusqlite::params![model, written, item.id],
        )
        .map_err(|e| format!("记录归类模型失败: {}", e))?;
    }
    Ok(count)
}

/// 读取某平台的 Cookie（没配过返回 None）。
pub fn get_credential(conn: &Connection, source: &str) -> Result<Option<String>, String> {
    let mut statement = conn
        .prepare("SELECT cookie FROM favorite_credential WHERE source = ?1")
        .map_err(|e| format!("读取凭证失败: {}", e))?;
    match statement.query_row([source], |row| row.get::<_, String>(0)) {
        Ok(cookie) => Ok(Some(cookie)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("读取凭证失败: {}", e)),
    }
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

/// 写入 / 清除某平台的 Cookie（传空串即清除）。
///
/// 写入时顺手清掉旧的健康状态：新 Cookie 还没验证过，不该继承上一次的 `expired`。
pub fn set_credential(conn: &Connection, source: &str, cookie: &str) -> Result<(), String> {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        conn.execute("DELETE FROM favorite_credential WHERE source = ?1", [source])
            .map_err(|e| format!("清除凭证失败: {}", e))?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO favorite_credential (source, cookie, updated_at, status, checked_at, expires_at) \
         VALUES (?1, ?2, ?3, 'unknown', NULL, ?4) \
         ON CONFLICT(source) DO UPDATE SET cookie = ?2, updated_at = ?3, \
         status = 'unknown', checked_at = NULL, expires_at = ?4",
        rusqlite::params![
            source,
            trimmed,
            now_str(),
            crate::commands::favorites::cookie_expiry::parse_cookie_expiry(source, trimmed)
        ],
    )
    .map_err(|e| format!("保存凭证失败: {}", e))?;
    Ok(())
}

/// 记一次使用结果（`ok` / `expired`），并刷新 `checked_at`。
pub fn mark_credential_status(conn: &Connection, source: &str, status: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE favorite_credential SET status = ?1, checked_at = ?2 WHERE source = ?3",
        rusqlite::params![status, now_str(), source],
    )
    .map_err(|e| format!("更新凭证状态失败: {}", e))?;
    Ok(())
}

/// 读某平台的凭证健康状态（没配过时 `configured = false`）。
pub fn credential_status(conn: &Connection, source: &str) -> Result<CredentialStatus, String> {
    let mut statement = conn
        .prepare(
            "SELECT status, checked_at, expires_at, updated_at FROM favorite_credential \
             WHERE source = ?1",
        )
        .map_err(|e| format!("读取凭证状态失败: {}", e))?;
    match statement.query_row([source], |row| {
        Ok(CredentialStatus {
            source: source.to_string(),
            configured: true,
            status: row.get(0)?,
            checked_at: row.get(1)?,
            expires_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    }) {
        Ok(status) => Ok(status),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(CredentialStatus {
            source: source.to_string(),
            configured: false,
            status: "unknown".to_string(),
            ..CredentialStatus::default()
        }),
        Err(e) => Err(format!("读取凭证状态失败: {}", e)),
    }
}

/// 写一条正文缓存（同一个条目重复写就覆盖：内容变了要跟新）。
pub fn put_content(
    conn: &Connection,
    favorite_id: i64,
    text: &str,
    html: Option<&str>,
    label: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO favorite_content (favorite_id, text, html, label, fetched_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(favorite_id) DO UPDATE SET text = ?2, html = ?3, label = ?4, fetched_at = ?5",
        rusqlite::params![favorite_id, text, html, label, now_str()],
    )
    .map_err(|e| format!("缓存正文失败: {}", e))?;
    Ok(())
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

/// 读正文缓存。
pub fn get_content(
    conn: &Connection,
    favorite_id: i64,
) -> Result<Option<CachedContent>, String> {
    let mut statement = conn
        .prepare("SELECT text, label, fetched_at FROM favorite_content WHERE favorite_id = ?1")
        .map_err(|e| format!("读取正文缓存失败: {}", e))?;
    match statement.query_row([favorite_id], |row| {
        Ok(CachedContent {
            text: row.get(0)?,
            label: row.get(1)?,
            fetched_at: row.get(2)?,
        })
    }) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("读取正文缓存失败: {}", e)),
    }
}

/// 条目 id → (source, title)；正文缓存/README 都要先知道抓谁。
pub fn find_by_id(
    conn: &Connection,
    id: i64,
) -> Result<Option<(String, String, String)>, String> {
    let mut statement = conn
        .prepare("SELECT source, title, COALESCE(subtitle, '') FROM favorite WHERE id = ?1")
        .map_err(|e| format!("查询条目失败: {}", e))?;
    match statement.query_row([id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("查询条目失败: {}", e)),
    }
}

/// 待探测条目：`(id, source, title, url)`。`all = false` 时跳过已探测过的。
///
/// 不限来源：GitHub 走 API（能识别改名），其余（浏览器书签 / B站 / 知乎…）
/// 走 HTTP 探测——导入来的条目应当一视同仁地能查失效。
pub fn select_for_check_multi(
    conn: &Connection,
    sources: &[String],
    all: bool,
    limit: usize,
) -> Result<Vec<(i64, String, String, String)>, String> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let list = sources
        .iter()
        .map(|s| format!("'{}'", s.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let sql = if all {
        format!("SELECT id, source, title, url FROM favorite WHERE source IN ({list}) ORDER BY id LIMIT ?1")
    } else {
        format!(
            "SELECT id, source, title, url FROM favorite \
             WHERE source IN ({list}) AND checked_at IS NULL ORDER BY id LIMIT ?1"
        )
    };
    let mut statement = conn
        .prepare(&sql)
        .map_err(|e| format!("查询待探测条目失败: {}", e))?;
    let rows = statement
        .query_map(rusqlite::params![limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| format!("查询待探测条目失败: {}", e))?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| format!("读取待探测条目失败: {}", e))?);
    }
    Ok(items)
}

/// 库里出现过的所有来源（探测时按它分派：GitHub 走 API，其余走 HTTP）。
pub fn all_sources(conn: &Connection) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare("SELECT DISTINCT source FROM favorite ORDER BY source")
        .map_err(|e| format!("查询来源失败: {}", e))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("查询来源失败: {}", e))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("读取来源失败: {}", e))?);
    }
    Ok(out)
}

/// 落探测结论；改名的顺手把 URL 指到新地址。
pub fn apply_status(
    conn: &Connection,
    id: i64,
    status: &str,
    new_url: Option<&str>,
) -> Result<(), String> {
    match new_url {
        Some(url) => conn.execute(
            "UPDATE favorite SET status = ?1, url = ?2, checked_at = ?3 WHERE id = ?4",
            rusqlite::params![status, url, now_str(), id],
        ),
        None => conn.execute(
            "UPDATE favorite SET status = ?1, checked_at = ?2 WHERE id = ?3",
            rusqlite::params![status, now_str(), id],
        ),
    }
    .map_err(|e| format!("更新探测状态失败: {}", e))?;
    Ok(())
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
    /// 排序方式：`favorited`（按收藏时间）/ `created`（按入库时间）/ 其它/缺省 = 按最近更新
    pub sort: Option<String>,
    /// 只保留「收藏时间（取不到则入库时间）不早于该时刻」的条目，本地时间字符串
    pub favorited_since: Option<String>,
    pub limit: usize,
}

/// 排序方式 → 固定 SQL 片段（白名单，绝不把用户输入直接拼进 SQL）。
fn order_by_clause(sort: Option<&str>) -> &'static str {
    match sort {
        // 收藏时间优先；平台没给时间的来源回退到本地首次入库时间
        Some("favorited") => "COALESCE(f.favorited_at, f.created_at) DESC, f.id DESC",
        Some("created") => "f.created_at DESC, f.id DESC",
        // 默认：最近更新在前
        _ => "f.updated_at DESC, f.id DESC",
    }
}

/// 按条件列出条目（默认按最近更新排序，最多 1000 条）。
pub fn list(conn: &Connection, filter: &ListFilter) -> Result<Vec<FavoriteRow>, String> {
    let limit = if filter.limit == 0 { 1000 } else { filter.limit };
    let keyword = filter
        .keyword
        .as_deref()
        .map(|k| format!("%{}%", k.trim()))
        .filter(|k| k != "%%");

    let since = filter
        .favorited_since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    // 分类筛选要带上子分类：递归展开后拼成 IN 列表（全是自己库里的整数 id，无注入风险）
    let category_ids: Option<Vec<i64>> = match filter.category_id {
        Some(id) => Some(descendant_ids(conn, id)?),
        None => None,
    };
    // 没按分类筛时也要拼出合法 SQL：`IN ()` 在 SQLite 里合法且恒为假，配合 `?4 IS NULL` 短路
    let category_in = category_ids
        .as_ref()
        .map(|ids| ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","))
        .unwrap_or_default();

    let sql = format!(
        "SELECT f.id, f.source, f.external_id, f.url, f.title, f.subtitle, f.description, \
                f.status, f.checked_at, f.ai_locked, f.ai_model, f.created_at, f.updated_at, f.favorited_at \
         FROM favorite f \
         WHERE (?1 IS NULL OR f.source = ?1) \
           AND (?2 IS NULL OR f.status = ?2) \
           AND (?3 IS NULL OR f.title LIKE ?3 OR COALESCE(f.description, '') LIKE ?3) \
           AND (?4 IS NULL OR EXISTS (SELECT 1 FROM favorite_item_category ic \
                                      WHERE ic.favorite_id = f.id AND ic.category_id IN ({category_in}))) \
           AND (?5 IS NULL OR COALESCE(f.favorited_at, f.created_at) >= ?5) \
         ORDER BY {} LIMIT {}",
        order_by_clause(filter.sort.as_deref()),
        limit
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|e| format!("查询收藏列表失败: {}", e))?;
    let rows = statement
        .query_map(
            rusqlite::params![
                filter.source,
                filter.status,
                keyword,
                filter.category_id,
                since
            ],
            |row| {
                Ok(FavoriteRow {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    external_id: row.get(2)?,
                    url: row.get(3)?,
                    title: row.get(4)?,
                    subtitle: row.get(5)?,
                    description: row.get(6)?,
                    status: row.get(7)?,
                    checked_at: row.get(8)?,
                    ai_locked: row.get::<_, i64>(9)? != 0,
                    ai_model: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    favorited_at: row.get(13)?,
                    tags: Vec::new(),
                })
            },
        )
        .map_err(|e| format!("查询收藏列表失败: {}", e))?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| format!("读取收藏条目失败: {}", e))?);
    }
    // 分类名单独查一次再挂回去：条目量上千时比每行一个子查询便宜得多
    let mut tag_map: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    {
        let mut tag_statement = conn
            .prepare(
                "SELECT ic.favorite_id, c.name \
                 FROM favorite_item_category ic JOIN favorite_category c ON c.id = ic.category_id \
                 ORDER BY c.name",
            )
            .map_err(|e| format!("查询分类失败: {}", e))?;
        let tag_rows = tag_statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| format!("读取分类失败: {}", e))?;
        for row in tag_rows {
            let (id, tag) = row.map_err(|e| format!("读取分类失败: {}", e))?;
            tag_map.entry(id).or_default().push(tag);
        }
    }
    for item in items.iter_mut() {
        item.tags = tag_map.remove(&item.id).unwrap_or_default();
    }
    Ok(items)
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
    count: i64,
}

fn load_flat_categories(conn: &Connection) -> Result<Vec<CategoryFlat>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.parent_id, c.name, c.sort_order, \
                    (SELECT COUNT(*) FROM favorite_item_category ic WHERE ic.category_id = c.id) \
             FROM favorite_category c \
             ORDER BY c.parent_id IS NOT NULL, c.sort_order ASC, c.name ASC",
        )
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CategoryFlat {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                name: r.get(2)?,
                sort_order: r.get(3)?,
                count: r.get(4)?,
            })
        })
        .map_err(|e| format!("读取分类失败: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("读取分类失败: {e}"))?);
    }
    Ok(out)
}

/// 列出整棵分类树（含条目数）。
pub fn list_category_tree(conn: &Connection) -> Result<Vec<CategoryNode>, String> {
    let flat = load_flat_categories(conn)?;
    let mut nodes: std::collections::HashMap<i64, CategoryNode> = std::collections::HashMap::new();
    for c in &flat {
        nodes.insert(
            c.id,
            CategoryNode {
                id: c.id,
                parent_id: c.parent_id,
                name: c.name.clone(),
                sort_order: c.sort_order,
                count: c.count,
                total: c.count,
                children: Vec::new(),
            },
        );
    }
    // 父节点可能排在子节点之后（sort_order 是「同级内」的序号），先收集再挂
    let mut roots: Vec<i64> = Vec::new();
    for c in &flat {
        match c.parent_id {
            Some(pid) if nodes.contains_key(&pid) => {}
            _ => roots.push(c.id),
        }
    }
    for c in &flat {
        if let Some(pid) = c.parent_id {
            if let Some(child) = nodes.get(&c.id).cloned() {
                if let Some(parent) = nodes.get_mut(&pid) {
                    parent.children.push(child);
                }
            }
        }
    }
    // 自底向上累加 total：深层级先算，父级再叠
    fn sum_total(ids: &[i64], nodes: &mut std::collections::HashMap<i64, CategoryNode>) -> i64 {
        let mut acc = 0i64;
        for id in ids {
            let children: Vec<i64> = nodes.get(id).map(|n| n.children.iter().map(|c| c.id).collect()).unwrap_or_default();
            let sub = sum_total(&children, nodes);
            if let Some(n) = nodes.get_mut(id) {
                n.total = n.count + sub;
                acc += n.total;
            }
        }
        acc
    }
    sum_total(&roots, &mut nodes);

    let mut out: Vec<CategoryNode> = roots
        .into_iter()
        .filter_map(|id| nodes.remove(&id))
        .collect();
    out.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
    Ok(out)
}

/// 取某分类的全部子孙 id（含自己），用于「按分类筛选时把子分类也算进来」。
pub fn descendant_ids(conn: &Connection, id: i64) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id, parent_id FROM favorite_category")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)))
        .map_err(|e| format!("读取分类失败: {e}"))?;
    let mut pairs: Vec<(i64, Option<i64>)> = Vec::new();
    for r in rows {
        pairs.push(r.map_err(|e| format!("读取分类失败: {e}"))?);
    }
    let mut out = vec![id];
    let mut changed = true;
    while changed {
        changed = false;
        for (cid, pid) in &pairs {
            if pid == &Some(id) || out.contains(&pid.unwrap_or(-1)) {
                if !out.contains(cid) {
                    out.push(*cid);
                    changed = true;
                }
            }
        }
    }
    Ok(out)
}

/// 同级重名检查（SQLite 的 UNIQUE 对 `parent_id IS NULL` 不去重，只能自己判）。
fn sibling_name_taken(conn: &Connection, parent_id: Option<i64>, name: &str, except_id: Option<i64>) -> Result<bool, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM favorite_category WHERE parent_id IS ?1 AND name = ?2")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![parent_id, name], |r| r.get::<_, i64>(0))
        .map_err(|e| format!("读取分类失败: {e}"))?;
    for r in rows {
        let id = r.map_err(|e| format!("读取分类失败: {e}"))?;
        if except_id != Some(id) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 新建分类（同名同级拒绝），返回新 id。
pub fn create_category(conn: &Connection, name: &str, parent_id: Option<i64>) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分类名不能为空".to_string());
    }
    if let Some(pid) = parent_id {
        let exists = conn
            .query_row("SELECT 1 FROM favorite_category WHERE id = ?1", [pid], |_| Ok(()))
            .is_ok();
        if !exists {
            return Err(format!("父分类不存在: {pid}"));
        }
    }
    if sibling_name_taken(conn, parent_id, name, None)? {
        return Err(format!("同级下已有同名分类: {name}"));
    }
    let next: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM favorite_category WHERE parent_id IS ?1",
            [parent_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("读取排序号失败: {e}"))?;
    conn.execute(
        "INSERT INTO favorite_category (parent_id, name, sort_order, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![parent_id, name, next, now_str()],
    )
    .map_err(|e| format!("新建分类失败: {e}"))?;
    Ok(conn.last_insert_rowid())
}

/// 重命名分类。
pub fn rename_category(conn: &Connection, id: i64, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分类名不能为空".to_string());
    }
    let parent_id: Option<i64> = conn
        .query_row("SELECT parent_id FROM favorite_category WHERE id = ?1", [id], |r| r.get(0))
        .map_err(|_| format!("分类不存在: {id}"))?;
    if sibling_name_taken(conn, parent_id, name, Some(id))? {
        return Err(format!("同级下已有同名分类: {name}"));
    }
    conn.execute(
        "UPDATE favorite_category SET name = ?1 WHERE id = ?2",
        rusqlite::params![name, id],
    )
    .map_err(|e| format!("重命名分类失败: {e}"))?;
    Ok(())
}

/// 移动分类（换父级）：目标不能是自己或自己的子孙，否则会成环。
pub fn move_category(conn: &Connection, id: i64, new_parent: Option<i64>) -> Result<(), String> {
    if let Some(pid) = new_parent {
        if pid == id {
            return Err("不能把分类移到自己下面".to_string());
        }
        if descendant_ids(conn, id)?.contains(&pid) {
            return Err("不能把分类移到自己的子分类下面（会成环）".to_string());
        }
    }
    conn.execute(
        "UPDATE favorite_category SET parent_id = ?1 WHERE id = ?2",
        rusqlite::params![new_parent, id],
    )
    .map_err(|e| format!("移动分类失败: {e}"))?;
    Ok(())
}

/// 在指定父级下查同名分类的 id（只查不建）。
pub fn find_child_id(conn: &Connection, parent_id: Option<i64>, name: &str) -> Result<Option<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM favorite_category WHERE parent_id IS ?1 AND name = ?2")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let found = stmt
        .query_row(rusqlite::params![parent_id, name], |r| r.get::<_, i64>(0))
        .ok();
    Ok(found)
}

/// 人工设置分类（界面选择器走这条）：顺手 `ai_locked = 1`。
///
/// 归类判据是「`ai_model IS NULL`」，如果不锁定，用户手选的分类会在下一轮 AI 归类里
/// 被模型再叠一层——那不是用户要的「我改过了」。
pub fn set_item_categories_manual(
    conn: &Connection,
    favorite_id: i64,
    ids: &[i64],
) -> Result<(), String> {
    set_item_categories(conn, favorite_id, ids)?;
    conn.execute(
        "UPDATE favorite SET ai_locked = 1, updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now_str(), favorite_id],
    )
    .map_err(|e| format!("锁定条目失败: {}", e))?;
    Ok(())
}

/// 按 (source, external_id) 查条目 id（导入后要给条目挂分类，需要这个 id）。
pub fn find_favorite_id(conn: &Connection, source: &str, external_id: &str) -> Result<Option<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM favorite WHERE source = ?1 AND external_id = ?2")
        .map_err(|e| format!("查询条目失败: {e}"))?;
    let found = stmt
        .query_row(rusqlite::params![source, external_id], |r| r.get::<_, i64>(0))
        .ok();
    Ok(found)
}

/// 追加一条「条目 ↔ 分类」挂载（已存在则忽略，不会重复）。
pub fn link_item_category(conn: &Connection, favorite_id: i64, category_id: i64) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO favorite_item_category (favorite_id, category_id, source) VALUES (?1, ?2, 'bookmark')",
        rusqlite::params![favorite_id, category_id],
    )
    .map_err(|e| format!("挂载分类失败: {e}"))?;
    Ok(())
}

/// 在指定父级下按名字取分类，没有就建（供导入时按浏览器书签目录建树用）。
pub fn ensure_category(conn: &Connection, parent_id: Option<i64>, name: &str) -> Result<i64, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM favorite_category WHERE parent_id IS ?1 AND name = ?2")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let found = stmt
        .query_row(rusqlite::params![parent_id, name], |r| r.get::<_, i64>(0))
        .ok();
    drop(stmt);
    match found {
        Some(id) => Ok(id),
        None => create_category(conn, name, parent_id),
    }
}

/// 删除分类（连同所有子孙分类与它们之间的挂载关系）。
///
/// 条目本身**不删**：分类只是标签，删掉分类不该把用户的收藏一起删掉
/// （启动模块删分类会连带删项目，那是「项目归属」语义；这里是「打标签」语义）。
pub fn delete_category(conn: &Connection, id: i64) -> Result<usize, String> {
    let ids = descendant_ids(conn, id)?;
    let tx = conn.unchecked_transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    for cid in &ids {
        tx.execute(
            "DELETE FROM favorite_item_category WHERE category_id = ?1",
            [cid],
        )
        .map_err(|e| format!("清理分类关联失败: {e}"))?;
        tx.execute("DELETE FROM favorite_category WHERE id = ?1", [cid])
            .map_err(|e| format!("删除分类失败: {e}"))?;
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(ids.len())
}

/// 同级排序：`[(id, sort_order)]` 批量写回。
pub fn reorder_categories(conn: &Connection, orders: &[(i64, i32)]) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("开启事务失败: {e}"))?;
    for (id, order) in orders {
        tx.execute(
            "UPDATE favorite_category SET sort_order = ?1 WHERE id = ?2",
            rusqlite::params![order, id],
        )
        .ok();
    }
    tx.commit().map_err(|e| format!("提交事务失败: {e}"))?;
    Ok(())
}

/// 全量替换某条目的分类。
pub fn set_item_categories(conn: &Connection, favorite_id: i64, ids: &[i64]) -> Result<(), String> {
    conn.execute(
        "DELETE FROM favorite_item_category WHERE favorite_id = ?1",
        [favorite_id],
    )
    .map_err(|e| format!("清除旧分类失败: {e}"))?;
    for cid in ids {
        conn.execute(
            "INSERT OR IGNORE INTO favorite_item_category (favorite_id, category_id, source) VALUES (?1, ?2, 'manual')",
            rusqlite::params![favorite_id, cid],
        )
        .map_err(|e| format!("写入分类失败: {e}"))?;
    }
    Ok(())
}

/// 按名字查分类（**只查不建**）。
///
/// 检索类入口用它：模型传来的名字可能是瞎猜的，不能在查询时顺手建出垃圾分类。
pub fn find_category_by_name(conn: &Connection, name: &str) -> Result<Option<i64>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    match conn.query_row(
        "SELECT id FROM favorite_category WHERE name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    ) {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("查询分类失败: {e}")),
    }
}

/// 按名字取分类（没有就在**顶层**建一个），供 AI 归类 / 旧的按名标签入口使用。
pub fn resolve_category_by_name(conn: &Connection, name: &str) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分类名不能为空".to_string());
    }
    if let Ok(id) = conn.query_row(
        "SELECT id FROM favorite_category WHERE name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    ) {
        return Ok(id);
    }
    create_category(conn, name, None)
}

/// 人工设置标签（全量替换），并锁定：后续 AI 归类不再碰它。
///
/// 名字走「顶层分类」：旧调用方传的是纯名字（逗号分隔），这里按需建分类。
pub fn set_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<(), String> {
    let mut ids = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        ids.push(resolve_category_by_name(conn, trimmed)?);
    }
    set_item_categories(conn, id, &ids)?;
    conn.execute(
        "UPDATE favorite SET ai_locked = 1, updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now_str(), id],
    )
    .map_err(|e| format!("锁定条目失败: {}", e))?;
    Ok(())
}

/// 删除本地条目（只删本地，不动平台）。
pub fn delete(conn: &Connection, id: i64) -> Result<bool, String> {
    conn.execute(
        "DELETE FROM favorite_item_category WHERE favorite_id = ?1",
        [id],
    )
    .map_err(|e| format!("删除分类关联失败: {}", e))?;
    let removed = conn
        .execute("DELETE FROM favorite WHERE id = ?1", [id])
        .map_err(|e| format!("删除收藏条目失败: {}", e))?;
    Ok(removed > 0)
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

/// 概览：总数 / 未归类 / 已失效 / 各源条数 / 各分类条数。
pub fn stats(conn: &Connection) -> Result<FavoriteStats, String> {
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM favorite", [], |r| r.get(0))
        .map_err(|e| format!("统计总数失败: {}", e))?;
    let gone: usize = conn
        .query_row("SELECT COUNT(*) FROM favorite WHERE status = 'gone'", [], |r| r.get(0))
        .map_err(|e| format!("统计失效数失败: {}", e))?;
    let unclassified = select_unclassified(conn, 1_000_000)?.len();

    let mut by_source: Vec<(String, usize)> = Vec::new();
    {
        let mut statement = conn
            .prepare("SELECT source, COUNT(*) FROM favorite GROUP BY source ORDER BY COUNT(*) DESC")
            .map_err(|e| format!("统计来源失败: {}", e))?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)))
            .map_err(|e| format!("统计来源失败: {}", e))?;
        for row in rows {
            by_source.push(row.map_err(|e| format!("读取来源统计失败: {}", e))?);
        }
    }
    let mut tags: Vec<(String, usize)> = Vec::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT c.name, COUNT(ic.favorite_id) \
                 FROM favorite_category c \
                 LEFT JOIN favorite_item_category ic ON ic.category_id = c.id \
                 GROUP BY c.id ORDER BY COUNT(ic.favorite_id) DESC, c.name",
            )
            .map_err(|e| format!("统计分类失败: {}", e))?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)))
            .map_err(|e| format!("统计分类失败: {}", e))?;
        for row in rows {
            tags.push(row.map_err(|e| format!("读取分类统计失败: {}", e))?);
        }
    }
    let categories = list_category_tree(conn)?;

    Ok(FavoriteStats {
        total,
        unclassified,
        gone,
        by_source,
        tags,
        categories,
    })
}

/// 读取某键的断点游标（键形如 `zhihu:<收藏夹id>`）。
///
/// 为什么需要：知乎用户数据接口按自然日配额（默认 100 次/天、未实名 10 次/天），
/// 每次翻页消耗一次。如果每次导入都从第 0 页重来，已导入的页照样烧额度，
/// 长收藏夹永远导不完——断点让每天的配额全部花在新内容上。
pub fn get_import_cursor(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    let mut statement = conn
        .prepare("SELECT cursor FROM favorite_import_state WHERE source = ?1")
        .map_err(|e| format!("读取导入断点失败: {}", e))?;
    match statement.query_row([key], |row| row.get::<_, Option<String>>(0)) {
        Ok(cursor) => Ok(cursor.filter(|c| !c.is_empty())),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("读取导入断点失败: {}", e)),
    }
}

/// 写入 / 清除断点游标（`None` = 该键已导完）。
pub fn set_import_cursor(conn: &Connection, key: &str, cursor: Option<&str>) -> Result<(), String> {
    conn.execute(
        "INSERT INTO favorite_import_state (source, cursor, last_run_at, total) \
         VALUES (?1, ?2, ?3, 0) \
         ON CONFLICT(source) DO UPDATE SET cursor = ?2, last_run_at = ?3",
        rusqlite::params![key, cursor, now_str()],
    )
    .map_err(|e| format!("保存导入断点失败: {}", e))?;
    Ok(())
}

/// 记录一次导入的收尾状态（供 UI 展示"上次导入"与续跑判断）。
pub fn mark_imported(conn: &Connection, source: &str, total: usize) -> Result<(), String> {
    conn.execute(
        "INSERT INTO favorite_import_state (source, cursor, last_run_at, total) \
         VALUES (?1, NULL, ?2, ?3) \
         ON CONFLICT(source) DO UPDATE SET last_run_at = ?2, total = ?3",
        rusqlite::params![source, now_str(), total as i64],
    )
    .map_err(|e| format!("记录导入状态失败: {}", e))?;
    Ok(())
}

/// 写入并返回条目 id（正文缓存要按 id 挂，而 upsert 本身不返回 id）。
pub fn upsert_with_id(
    conn: &Connection,
    item: &NewFavorite,
) -> Result<(UpsertOutcome, i64), String> {
    let outcome = upsert(conn, item)?;
    let id = conn
        .query_row(
            "SELECT id FROM favorite WHERE source = ?1 AND external_id = ?2",
            rusqlite::params![item.source, item.external_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| format!("读取条目 id 失败: {}", e))?;
    Ok((outcome, id))
}

/// 条目总数（测试与概览用）。
pub fn count_all(conn: &Connection) -> Result<usize, String> {
    conn.query_row("SELECT COUNT(*) FROM favorite", [], |row| row.get(0))
        .map_err(|e| format!("统计收藏条目失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::{
        apply_status, apply_tags, count_all, create_category, delete, delete_category,
        find_category_by_name, get_credential, get_import_cursor, link_item_category, list,
        list_category_tree, migrate, move_category, prune_all_empty_categories, rename_category,
        reset_classification, select_unclassified, set_credential, set_import_cursor,
        set_item_categories, set_item_categories_manual, set_tags, stats, upsert, CategoryNode,
        FavoriteRow, FavoriteStats, ListFilter, NewFavorite, UpsertOutcome,
    };

    /// 前后端的字段契约：这两个结构按 camelCase 序列化，前端读的是 `bySource` / `aiLocked`。
    /// 一旦 rename_all 被去掉或字段改名，前端只会静默拿到 undefined（来源下拉框只剩
    /// 「全部来源」、锁定徽标不显示），所以在这里钉死。
    #[test]
    fn list_and_stats_serialize_as_camel_case() {
        let overview = FavoriteStats {
            total: 3,
            unclassified: 1,
            gone: 0,
            by_source: vec![("github".to_string(), 3)],
            tags: vec![("CLI".to_string(), 2)],
            categories: vec![],
        };
        let value = serde_json::to_value(&overview).unwrap();
        assert!(value.get("by_source").is_none());
        assert_eq!(value["bySource"][0][0], "github");
        assert_eq!(value["bySource"][0][1], 3);

        let row = FavoriteRow {
            id: 1,
            source: "github".to_string(),
            external_id: "42".to_string(),
            url: "https://github.com/a/b".to_string(),
            title: "a/b".to_string(),
            subtitle: None,
            description: None,
            status: "ok".to_string(),
            checked_at: None,
            ai_locked: true,
            ai_model: Some("m".to_string()),
            created_at: "2026-01-01".to_string(),
            updated_at: "2026-01-01".to_string(),
            favorited_at: None,
            tags: vec![],
        };
        let value = serde_json::to_value(&row).unwrap();
        assert!(value.get("ai_locked").is_none());
        assert_eq!(value["aiLocked"], true);
        assert_eq!(value["externalId"], "42");
        // 收藏时间字段也要能被前端读到（列表里要显示、排序要用）
        assert!(value.get("favoritedAt").is_some());
    }

    fn sample(external_id: &str, title: &str) -> NewFavorite {
        NewFavorite {
            source: "github".to_string(),
            external_id: external_id.to_string(),
            url: format!("https://github.com/{}", title),
            title: title.to_string(),
            subtitle: Some(title.to_string()),
            description: Some("desc".to_string()),
            extra_json: Some("{}".to_string()),
            favorited_at: None,
            initial_status: None,
        }
    }

    #[test]
    fn list_sorts_and_filters_by_favorited_time() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let mut old = sample("1", "old");
        old.favorited_at = Some("2020-05-01T10:00:00".to_string());
        let mut recent = sample("2", "recent");
        recent.favorited_at = Some("2026-05-01T10:00:00".to_string());
        upsert(&conn, &old).unwrap();
        upsert(&conn, &recent).unwrap();

        // 按收藏时间排序：最近收藏的在前（而不是按入库/更新时间）
        let sorted = list(
            &conn,
            &ListFilter {
                sort: Some("favorited".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(sorted[0].external_id, "2");
        assert_eq!(sorted[0].favorited_at.as_deref(), Some("2026-05-01T10:00:00"));
        assert_eq!(sorted[1].external_id, "1");

        // 时间过滤：只留收藏时间不早于 2026 的
        let filtered = list(
            &conn,
            &ListFilter {
                favorited_since: Some("2026-01-01T00:00:00".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].external_id, "2");

        // 没有平台收藏时间的条目回退用入库时间判断，不会被时间过滤直接漏掉
        let no_time = sample("3", "no-time");
        upsert(&conn, &no_time).unwrap();
        let all = list(
            &conn,
            &ListFilter {
                favorited_since: Some("2000-01-01T00:00:00".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn reimport_backfills_favorited_time_without_touching_known_values() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        // 老库（或升级前导入）没有收藏时间：再次导入带上时间时应算一次更新并补齐
        let mut item = sample("9", "backfill");
        assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);
        item.favorited_at = Some("2021-03-04T05:06:07".to_string());
        assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Updated);
        let rows = list(&conn, &ListFilter::default()).unwrap();
        assert_eq!(rows[0].favorited_at.as_deref(), Some("2021-03-04T05:06:07"));

        // 本次没取到时间（如平台接口不再返回）时保留库里已有的值，不覆盖成 NULL
        let mut without_time = sample("9", "backfill");
        without_time.description = Some("changed".to_string());
        assert_eq!(upsert(&conn, &without_time).unwrap(), UpsertOutcome::Updated);
        let rows = list(&conn, &ListFilter::default()).unwrap();
        assert_eq!(rows[0].favorited_at.as_deref(), Some("2021-03-04T05:06:07"));
    }

    #[test]
    fn schema_creates_all_three_tables() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' \
                 AND name IN ('favorite','favorite_tag','favorite_import_state')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    /// 建表必须幂等：命令层每次调用都会走到这里。
    #[test]
    fn migrate_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
    }

    /// 去重靠的是 (source, external_id) 唯一约束，缺了它整个模块的前提就没了。
    #[test]
    fn duplicate_source_and_external_id_is_rejected() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let insert = |external_id: &str| -> rusqlite::Result<usize> {
            conn.execute(
                "INSERT INTO favorite (source, external_id, url, title, created_at, updated_at) \
                 VALUES ('github', ?1, 'https://x', 't', 'now', 'now')",
                [external_id],
            )
        };
        assert!(insert("42").is_ok());
        let err = insert("42").unwrap_err();
        assert!(
            matches!(err, rusqlite::Error::SqliteFailure(_, _)),
            "重复 (source, external_id) 必须被唯一约束拦下，实际: {:?}",
            err
        );
        // 不同 source 的同 id 不算冲突
        conn.execute(
            "INSERT INTO favorite (source, external_id, url, title, created_at, updated_at) \
             VALUES ('zhihu', '42', 'https://x', 't', 'now', 'now')",
            [],
        )
        .unwrap();
    }

    /// 第二次导入同一批数据必须「一个都不新增」——这是用户对重复导入的核心要求。
    #[test]
    fn second_import_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let item = sample("1", "o/r");
        assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);
        assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Skipped);
        assert_eq!(count_all(&conn).unwrap(), 1);
    }

    /// 内容变了（仓库改名 / 描述更新）要更新，而不是无视。
    #[test]
    fn changed_fields_are_updated_in_place() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        upsert(&conn, &sample("1", "o/r")).unwrap();
        let mut renamed = sample("1", "o/r");
        renamed.title = "o/renamed".to_string();
        assert_eq!(upsert(&conn, &renamed).unwrap(), UpsertOutcome::Updated);
        assert_eq!(count_all(&conn).unwrap(), 1, "更新不能变成新增一行");
        let title: String = conn
            .query_row("SELECT title FROM favorite WHERE external_id = '1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "o/renamed");
    }

    /// GitHub 用仓库 id 做 external_id：仓库改名（full_name 变了）仍是同一条记录。
    #[test]
    fn renaming_full_name_does_not_duplicate() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = sample("42", "old/name");
        let mut renamed = sample("42", "old/name");
        renamed.title = "new/name".to_string();
        renamed.url = "https://github.com/new/name".to_string();
        upsert(&conn, &first).unwrap();
        upsert(&conn, &renamed).unwrap();
        assert_eq!(count_all(&conn).unwrap(), 1, "改名不能导进重复条目");
    }

    /// 不同平台的 id 撞车不算同一条（外部 id 只在同 source 内唯一）。
    #[test]
    fn same_external_id_across_sources_stays_separate() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut zhihu = sample("42", "x/y");
        zhihu.source = "zhihu".to_string();
        upsert(&conn, &sample("42", "x/y")).unwrap();
        upsert(&conn, &zhihu).unwrap();
        assert_eq!(count_all(&conn).unwrap(), 2);
    }

    fn insert_raw(conn: &rusqlite::Connection, id: i64, external_id: &str, extra: &str) {
        conn.execute(
            "INSERT INTO favorite (id, source, external_id, url, title, extra_json, created_at, updated_at) \
             VALUES (?1, 'github', ?2, 'https://x', ?2, ?3, 'now', 'now')",
            rusqlite::params![id, external_id, extra],
        )
        .unwrap();
    }

    /// 断点游标：按键读写、可清除；跨天续传靠它把配额花在新内容上。
    #[test]
    fn import_cursor_roundtrip_and_clear() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let key = "zhihu:123456789";
        assert_eq!(get_import_cursor(&conn, key).unwrap(), None);

        set_import_cursor(&conn, key, Some("40")).unwrap();
        assert_eq!(get_import_cursor(&conn, key).unwrap().as_deref(), Some("40"));

        // 不同键互不影响（B站/其它收藏夹各有各的断点）
        set_import_cursor(&conn, "zhihu:999", Some("20")).unwrap();
        assert_eq!(get_import_cursor(&conn, key).unwrap().as_deref(), Some("40"));

        // 导完后清除：下次导入从头检查是否有新内容
        set_import_cursor(&conn, key, None).unwrap();
        assert_eq!(get_import_cursor(&conn, key).unwrap(), None);
        // 清除不能动别的键
        assert_eq!(get_import_cursor(&conn, "zhihu:999").unwrap().as_deref(), Some("20"));
    }

    /// 凭证按 source 隔离：收藏模块的 GitHub Token 与 B站 Cookie 互不影响，
    /// 也就不会出现「换了 B站 Cookie 把 GitHub Token 顶掉」这种事。
    #[test]
    fn credentials_are_stored_per_source() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert_eq!(get_credential(&conn, "github").unwrap(), None);

        set_credential(&conn, "github", "ghp_xxx").unwrap();
        set_credential(&conn, "bilibili", "SESSDATA=yyy").unwrap();
        assert_eq!(get_credential(&conn, "github").unwrap().as_deref(), Some("ghp_xxx"));
        assert_eq!(
            get_credential(&conn, "bilibili").unwrap().as_deref(),
            Some("SESSDATA=yyy")
        );

        // 覆盖同一个 source 不影响另一个；空串即清除
        set_credential(&conn, "github", "ghp_new").unwrap();
        assert_eq!(get_credential(&conn, "bilibili").unwrap().as_deref(), Some("SESSDATA=yyy"));
        set_credential(&conn, "github", "").unwrap();
        assert_eq!(get_credential(&conn, "github").unwrap(), None);
        assert_eq!(get_credential(&conn, "bilibili").unwrap().is_some(), true);
    }

    /// 已失效的条目不再送归类（给它分类没意义，还白花 token）。
    #[test]
    fn select_unclassified_skips_gone_items() {
        let conn = seeded();
        apply_status(&conn, 2, "gone", None).unwrap();
        let pending = select_unclassified(&conn, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, 1);
    }

    /// 多级分类：父子层级、按层级筛选（父级要能看到子级的东西）、成环一律拒绝。
    #[test]
    fn category_tree_supports_nesting_and_filtering() {
        let conn = seeded();
        let root = create_category(&conn, "技术", None).unwrap();
        let child = create_category(&conn, "编译器", Some(root)).unwrap();
        let grand = create_category(&conn, "LLVM", Some(child)).unwrap();

        // 同级重名要拦住（父级不同则可以同名）
        assert!(create_category(&conn, "技术", None).is_err());
        assert!(create_category(&conn, "编译器", Some(grand)).is_ok());

        set_item_categories(&conn, 1, &[grand]).unwrap();
        set_item_categories(&conn, 2, &[]).unwrap();

        // 点父分类要能把子孙分类下的条目一起筛出来
        assert_eq!(
            list(&conn, &ListFilter { category_id: Some(root), ..Default::default() })
                .unwrap()
                .len(),
            1,
            "按根分类筛选应包含子分类里的条目"
        );
        assert_eq!(
            list(&conn, &ListFilter { category_id: Some(grand), ..Default::default() })
                .unwrap()
                .len(),
            1
        );

        // 不能移到自己 / 自己子孙下面（成环）
        assert!(move_category(&conn, root, Some(root)).is_err());
        assert!(move_category(&conn, root, Some(grand)).is_err());
        assert!(move_category(&conn, grand, None).is_ok());

        // 删分类只解开关联，不删条目
        // grand 已被移到顶层，所以这次删的是 root + child 两级
        let removed = delete_category(&conn, root).unwrap();
        assert_eq!(removed, 2, "子孙分类要一起删");
        assert!(find_category_by_name(&conn, "LLVM").unwrap().is_some(), "移走的分类不受影响");
        assert_eq!(count_all(&conn).unwrap(), 2, "条目不能跟着分类一起没");
    }

    /// 分类树的条目数：自己挂的 + 所有子孙的（侧栏显示这个才有意义）。
    #[test]
    fn category_tree_counts_include_descendants() {
        let conn = seeded();
        let root = create_category(&conn, "A", None).unwrap();
        let child = create_category(&conn, "B", Some(root)).unwrap();
        set_item_categories(&conn, 1, &[root]).unwrap();
        set_item_categories(&conn, 2, &[child]).unwrap();

        let tree = list_category_tree(&conn).unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].count, 1);
        assert_eq!(tree[0].total, 2, "父级要算上子分类的条目");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].total, 1);

        rename_category(&conn, child, "B2").unwrap();
        let tree = list_category_tree(&conn).unwrap();
        assert_eq!(tree[0].children[0].name, "B2");
        let _: CategoryNode = tree[0].clone();
    }

    /// 旧库升级：扁平标签要变成顶层分类，条目关联不能丢。
    #[test]
    fn legacy_tags_are_migrated_into_category_tree() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // 先建一张只有旧标签表的库，再补插旧数据，最后跑新版 migrate
        conn.execute_batch(
            "CREATE TABLE favorite_tag (favorite_id INTEGER NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (favorite_id, tag));",
        )
        .unwrap();
        conn.execute("INSERT INTO favorite_tag VALUES (1, 'CLI')", []).unwrap();
        conn.execute("INSERT INTO favorite_tag VALUES (1, 'Web')", []).unwrap();
        migrate(&conn).unwrap();

        assert_eq!(find_category_by_name(&conn, "CLI").unwrap(), Some(1));
        let linked: i64 = conn
            .query_row("SELECT COUNT(*) FROM favorite_item_category WHERE favorite_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(linked, 2, "两个旧标签都要变成分类关联");
        // 再迁移一次不能重复建
        migrate(&conn).unwrap();
        assert_eq!(list_category_tree(&conn).unwrap().len(), 2);
    }

    /// 归类候选 = 「AI 还没归类过」且未人工锁定、未失效。
    ///
    /// 关键回归：**只有分类（浏览器书签带来的目录）但没有 ai_model 的条目仍然要进候选** ——
    /// 否则浏览器收藏在收藏库里永远是二等公民，AI 归类永远碰不到它们。
    #[test]
    fn select_unclassified_skips_locked_classified_and_gone() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", "{}");
        insert_raw(&conn, 2, "2", "{}");
        insert_raw(&conn, 3, "3", "{}");
        insert_raw(&conn, 4, "4", "{}");

        let pending = select_unclassified(&conn, 10).unwrap();
        assert_eq!(pending.len(), 4);

        // 只有目录分类（书签导入形态）、没有 ai_model → 仍要归类
        let cli = create_category(&conn, "CLI", None).unwrap();
        set_item_categories(&conn, 1, &[cli]).unwrap();
        // 人工改分类（界面选择器）会顺手锁定 → 永远不再归类
        set_item_categories_manual(&conn, 2, &[cli]).unwrap();
        // AI 归类过（ai_model 非空）→ 不再重复归类
        conn.execute("UPDATE favorite SET ai_model = 'gpt-x' WHERE id = 3", []).unwrap();
        // 已失效 → 跳过
        apply_status(&conn, 4, "gone", None).unwrap();

        let pending = select_unclassified(&conn, 10).unwrap();
        assert_eq!(pending.len(), 1, "只剩「有目录分类但没 AI 归类过」的那条");
        assert_eq!(pending[0].id, 1);
    }

    /// 多标签：一个条目落两个分类要写两行；写完之后它就不再是「待归类」。
    #[test]
    fn apply_tags_writes_every_label_and_clears_pending() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", "{}");
        let items = select_unclassified(&conn, 10).unwrap();
        let groups = vec![
            (vec!["LLM".to_string()], vec![0usize]),
            (vec!["运维".to_string()], vec![0usize]),
        ];
        let written = apply_tags(&conn, &items, &groups, "gpt-x", false).unwrap();
        assert_eq!(written, 2, "多标签每条都要写");

        let tags: Vec<String> = conn
            .prepare(
                "SELECT c.name FROM favorite_item_category ic \
                 JOIN favorite_category c ON c.id = ic.category_id \
                 WHERE ic.favorite_id = 1 ORDER BY c.name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(tags, vec!["LLM".to_string(), "运维".to_string()]);
        assert!(select_unclassified(&conn, 10).unwrap().is_empty());

        let model: String = conn
            .query_row("SELECT ai_model FROM favorite WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(model, "gpt-x");
    }

    /// 多级分类名要建成真正的父子层级：条目挂到叶子，父分类是叶子分类的 parent。
    #[test]
    fn apply_tags_builds_multi_level_tree() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", "{}");
        let items = select_unclassified(&conn, 10).unwrap();

        let groups = vec![(vec!["编程语言".to_string(), "Rust".to_string()], vec![0usize])];
        let written = apply_tags(&conn, &items, &groups, "gpt-x", false).unwrap();
        assert_eq!(written, 1);

        let root_id = find_category_by_name(&conn, "编程语言").unwrap().unwrap();
        let leaf_id = find_category_by_name(&conn, "Rust").unwrap().unwrap();
        let parent: Option<i64> = conn
            .query_row("SELECT parent_id FROM favorite_category WHERE id = ?1", [leaf_id], |r| r.get(0))
            .unwrap();
        assert_eq!(parent, Some(root_id), "叶子分类应挂在父分类下");

        // 条目挂在叶子分类上，且来源是 ai
        let (linked, source): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MAX(source), '') FROM favorite_item_category \
                 WHERE favorite_id = 1 AND category_id = ?1",
                [leaf_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(linked, 1);
        assert_eq!(source, "ai");
    }

    /// 重新归类只清 AI 加的关联，书签目录与人工选择都保留。
    #[test]
    fn reclassify_clears_only_ai_links() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", "{}");

        // 书签目录分类（bookmark）
        let bm = create_category(&conn, "书签栏", None).unwrap();
        link_item_category(&conn, 1, bm).unwrap();

        // 第一次 AI 归类（ai）→ 旧分类
        let items = select_unclassified(&conn, 10).unwrap();
        apply_tags(
            &conn,
            &items,
            &[(vec!["旧分类".to_string()], vec![0usize])],
            "gpt-x",
            false,
        )
        .unwrap();
        let old_id = find_category_by_name(&conn, "旧分类").unwrap().unwrap();

        // 重新归类：先清掉 AI 标记，已归类过的条目重新进「未归类」批次
        let reset = reset_classification(&conn).unwrap();
        assert_eq!(reset, 1, "只有 AI 归类过的那条被重置");
        let items = select_unclassified(&conn, 10).unwrap();
        assert_eq!(items.len(), 1, "重置后已归类的条目也要进重归批次");
        apply_tags(
            &conn,
            &items,
            &[(vec!["新分类".to_string()], vec![0usize])],
            "gpt-x",
            true,
        )
        .unwrap();

        let link_count = |cat_id: i64| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM favorite_item_category WHERE favorite_id = 1 AND category_id = ?1",
                [cat_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(link_count(old_id), 0, "旧的 AI 分类要被清掉");
        assert_eq!(link_count(bm), 1, "书签目录分类要保留");
        assert_eq!(link_count(find_category_by_name(&conn, "新分类").unwrap().unwrap()), 1);
    }

    /// 重新归类后的空分类清理：只删「既无关联又无子分类」的，且级联向上收掉变空的祖先。
    #[test]
    fn prune_all_empty_categories_removes_only_empty() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        // 有内容的分类（保留）
        let used = create_category(&conn, "有内容", None).unwrap();
        link_item_category(&conn, 1, used).unwrap();

        // 空叶子（删）
        let _ = create_category(&conn, "空叶子", None).unwrap();

        // 空父 + 空子：父子都空 → 级联删
        let empty_parent = create_category(&conn, "空父", None).unwrap();
        let _ = create_category(&conn, "空子", Some(empty_parent)).unwrap();

        // 父分类下有「有内容」的子分类：父不删
        let parent = create_category(&conn, "父", None).unwrap();
        let used_child = create_category(&conn, "有内容子", Some(parent)).unwrap();
        link_item_category(&conn, 1, used_child).unwrap();

        let removed = prune_all_empty_categories(&conn).unwrap();
        assert_eq!(removed, 3, "空叶子 + 空父 + 空子 三个被删");

        assert!(find_category_by_name(&conn, "有内容").unwrap().is_some());
        assert!(find_category_by_name(&conn, "父").unwrap().is_some());
        assert!(find_category_by_name(&conn, "有内容子").unwrap().is_some());
        assert!(find_category_by_name(&conn, "空叶子").unwrap().is_none());
        assert!(find_category_by_name(&conn, "空父").unwrap().is_none());
        assert!(find_category_by_name(&conn, "空子").unwrap().is_none());
    }

    /// 归类要能读到语言与 topics（这是判断分类最有用的两个信号）。
    #[test]
    fn select_unclassified_reads_language_and_topics() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(
            &conn,
            1,
            "o/r",
            r#"{"language":"Rust","topics":["cli","rust"]}"#,
        );
        let items = select_unclassified(&conn, 10).unwrap();
        assert_eq!(items[0].language, "Rust");
        assert_eq!(items[0].topics, vec!["cli".to_string(), "rust".to_string()]);
    }

    fn seeded() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", r#"{"language":"Rust"}"#);
        insert_raw(&conn, 2, "2", r#"{"language":"Go"}"#);
        conn.execute("UPDATE favorite SET title = 'o/cli', description = 'a fast cli' WHERE id = 1", [])
            .unwrap();
        conn.execute("UPDATE favorite SET title = 'o/web', description = 'web framework' WHERE id = 2", [])
            .unwrap();
        conn
    }

    /// 人工改标签 = 全量替换 + 锁定（用户的选择优先于模型）。
    #[test]
    fn set_tags_replaces_and_locks() {
        let conn = seeded();
        set_tags(&conn, 1, &["CLI".to_string(), "工具".to_string()]).unwrap();
        let row = &list(&conn, &ListFilter::default()).unwrap()
            .into_iter()
            .find(|r| r.id == 1)
            .unwrap();
        assert_eq!(row.tags, vec!["CLI".to_string(), "工具".to_string()]);
        assert!(row.ai_locked);
        assert!(select_unclassified(&conn, 10).unwrap().iter().all(|i| i.id != 1));

        // 再设一次：旧标签被替换而不是叠加
        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        let row = &list(&conn, &ListFilter::default()).unwrap()
            .into_iter()
            .find(|r| r.id == 1)
            .unwrap();
        assert_eq!(row.tags, vec!["CLI".to_string()]);
    }

    /// 删本地条目要连标签一起删（不留孤儿行）。
    #[test]
    fn delete_removes_tags_too() {
        let conn = seeded();
        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        assert!(delete(&conn, 1).unwrap());
        assert!(!delete(&conn, 1).unwrap(), "删第二次应返回 false");
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM favorite_tag", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
        assert_eq!(count_all(&conn).unwrap(), 1);
    }

    #[test]
    fn list_filters_by_source_tag_and_keyword() {
        let conn = seeded();
        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        set_tags(&conn, 2, &["Web".to_string()]).unwrap();

        assert_eq!(list(&conn, &ListFilter::default()).unwrap().len(), 2);
        let cli_id = find_category_by_name(&conn, "CLI").unwrap().expect("CLI 分类已建");
        assert_eq!(
            list(&conn, &ListFilter { category_id: Some(cli_id), ..Default::default() })
                .unwrap()
                .len(),
            1
        );
        // 关键字同时匹配标题与描述
        assert_eq!(
            list(&conn, &ListFilter { keyword: Some("cli".into()), ..Default::default() })
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list(&conn, &ListFilter { keyword: Some("framework".into()), ..Default::default() })
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list(&conn, &ListFilter { source: Some("zhihu".into()), ..Default::default() })
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn stats_counts_unclassified_and_gone() {
        let conn = seeded();
        let before = stats(&conn).unwrap();
        assert_eq!(before.total, 2);
        assert_eq!(before.unclassified, 2);
        assert_eq!(before.gone, 0);

        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        apply_status(&conn, 2, "gone", None).unwrap();
        let after = stats(&conn).unwrap();
        assert_eq!(after.unclassified, 0, "已归类的 + 已失效的都不再送归类");
        assert_eq!(after.gone, 1);
        assert_eq!(after.tags, vec![("CLI".to_string(), 1)]);
    }

    /// 多标签：同一条目可挂多个标签，但完全相同的 (条目, 标签) 只能有一条。
    #[test]
    fn tags_allow_multiple_labels_per_item() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO favorite (id, source, external_id, url, title, created_at, updated_at) \
             VALUES (1, 'github', '42', 'https://x', 't', 'now', 'now')",
            [],
        )
        .unwrap();
        let tag = |t: &str| {
            conn.execute("INSERT INTO favorite_tag (favorite_id, tag) VALUES (1, ?1)", [t])
        };
        assert!(tag("LLM").is_ok());
        assert!(tag("运维").is_ok(), "多标签必须允许同一条目挂第二个分类");
        assert!(tag("LLM").is_err(), "同一标签不能重复挂");
    }
}
