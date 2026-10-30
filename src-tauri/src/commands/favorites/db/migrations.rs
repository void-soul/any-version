use super::*;

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
        -- 删除墓碑：记住「用户删过哪些条目」。
        -- 没有它，本地删掉的条目会在下一次导入时复活——导入按 UNIQUE(source, external_id)
        -- upsert，库里查不到就当新条目插入，而平台上的收藏一直还在。
        -- 只存键 + 标题（供设置页辨认），不存正文。
        CREATE TABLE IF NOT EXISTS favorite_deleted (
            source      TEXT NOT NULL,
            external_id TEXT NOT NULL,
            title       TEXT,
            deleted_at  TEXT NOT NULL,
            PRIMARY KEY (source, external_id)
        );
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
        (
            "status",
            "ALTER TABLE favorite_credential ADD COLUMN status TEXT NOT NULL DEFAULT 'unknown'",
        ),
        (
            "checked_at",
            "ALTER TABLE favorite_credential ADD COLUMN checked_at TEXT",
        ),
        (
            "expires_at",
            "ALTER TABLE favorite_credential ADD COLUMN expires_at INTEGER",
        ),
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
    // 星标：“特别关注”的位置
    if !has_column(conn, "favorite", "is_starred")? {
        conn.execute(
            "ALTER TABLE favorite ADD COLUMN is_starred INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| format!("升级收藏库失败（is_starred）: {}", e))?;
    }
    // 分类的性质：topic = 主题分类（AI 归类会写），follow = 关注分组。
    // 两者必须区分开：关注分组是人工维护的，被「重新归类」清掉或被 AI 自动塞进其他类都会变成灾难恢复的事故
    if !has_column(conn, "favorite_category", "kind")? {
        conn.execute(
            "ALTER TABLE favorite_category ADD COLUMN kind TEXT NOT NULL DEFAULT 'topic'",
            [],
        )
        .map_err(|e| format!("升级收藏库失败（category.kind）: {}", e))?;
    }
    migrate_tags_to_categories(conn)?;
    Ok(())
}

/// 老库升级：把「扁平标签」搬进分类树。
///
/// 幂等：只要分类树里已经有东西就认为搬过了（新库自然是空的，也走不到插入）。
/// 旧标签全部建成**顶层**分类——老数据里没有层级信息，硬猜父子关系只会猜错。
pub(super) fn migrate_tags_to_categories(conn: &Connection) -> Result<(), String> {
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

/// 表里有没有这一列（用于幂等升级老库）。
pub(super) fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
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
