use super::*;

/// 写入一条条目：已存在则按可变字段更新，完全没变则跳过。
///
/// 这是「重复导入不进重复记录」的唯一保证——依赖 `(source, external_id)` 唯一约束，
/// 而不是先查后写（避免并发下两边都查不到）。
pub fn upsert(conn: &Connection, item: &NewFavorite) -> Result<UpsertOutcome, String> {
    // 用户删过的条目不再捞回来：平台上的收藏还在，导入一定会再拉到它，
    // 而本地那条已经删了、唯一键空着，不查碑就会当成新条目重新插入。
    if is_deleted(conn, &item.source, &item.external_id)? {
        return Ok(UpsertOutcome::Deleted);
    }
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

/// 条目 id → (source, title)；正文缓存/README 都要先知道抓谁。
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<(String, String, String)>, String> {
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

/// 排序方式 → 固定 SQL 片段（白名单，绝不把用户输入直接拼进 SQL）。
///
/// 只保留「按收藏时间」一种：收藏模块关心的就是「什么时候收藏的」。
/// 平台没给时间的来源回退到本地首次入库时间（`created_at` 保留作回退，但不再提供
/// 「按入库时间 / 按最近更新」两种排序）。
pub(super) fn order_by_clause(_sort: Option<&str>) -> &'static str {
    "f.is_starred DESC, COALESCE(f.favorited_at, f.created_at) DESC, f.id DESC"
}

/// 按条件列出条目（按收藏时间排序，最多 1000 条）。
pub fn list(conn: &Connection, filter: &ListFilter) -> Result<Vec<FavoriteRow>, String> {
    let limit = if filter.limit == 0 {
        1000
    } else {
        filter.limit
    };
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
        .map(|ids| {
            ids.iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    let sql = format!(
        "SELECT f.id, f.source, f.external_id, f.url, f.title, f.subtitle, f.description, \
                f.status, f.checked_at, f.ai_locked, f.ai_model, f.created_at, f.updated_at, f.favorited_at, f.is_starred \
         FROM favorite f \
         WHERE (?1 IS NULL OR f.source = ?1) \
           AND (?2 IS NULL OR f.status = ?2) \
           AND (?3 IS NULL OR f.title LIKE ?3 OR COALESCE(f.description, '') LIKE ?3) \
           AND (?4 IS NULL OR EXISTS (SELECT 1 FROM favorite_item_category ic \
                                      WHERE ic.favorite_id = f.id AND ic.category_id IN ({category_in}))) \
           AND (?5 IS NULL OR COALESCE(f.favorited_at, f.created_at) >= ?5) \
          AND (?6 = 0 OR f.is_starred = 1) \
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
                since,
                filter.starred_only as i64
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
                    is_starred: row.get::<_, i64>(14)? != 0,
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
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
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

/// 设置 / 取消星标（特别关注）。列表会把它置顶。
pub fn set_starred(conn: &Connection, id: i64, starred: bool) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE favorite SET is_starred = ?2 WHERE id = ?1",
            rusqlite::params![id, starred as i64],
        )
        .map_err(|e| format!("设置星标失败: {e}"))?;
    if changed == 0 {
        return Err(format!("收藏条目不存在: {id}"));
    }
    Ok(())
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

/// 删除本地条目（只删本地，不动平台）+ **立墓碑**。
///
/// 立碑是「删掉的条目不再被导入捞回来」的唯一保证：删之前先把 (source, external_id)
/// 读出来，删完再写进 `favorite_deleted`。碑没立上而条目已删，等于白删——所以
/// 写入失败要连带回滚删除（事务）。
pub fn delete(conn: &mut Connection, id: i64) -> Result<bool, String> {
    let existing: Option<(String, String, String)> = conn
        .query_row(
            "SELECT source, external_id, title FROM favorite WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let tx = conn
        .transaction()
        .map_err(|e| format!("开启删除事务失败: {}", e))?;
    tx.execute(
        "DELETE FROM favorite_item_category WHERE favorite_id = ?1",
        [id],
    )
    .map_err(|e| format!("删除分类关联失败: {}", e))?;
    let removed = tx
        .execute("DELETE FROM favorite WHERE id = ?1", [id])
        .map_err(|e| format!("删除收藏条目失败: {}", e))?;
    if removed > 0 {
        if let Some((source, external_id, title)) = existing {
            tx.execute(
                "INSERT INTO favorite_deleted (source, external_id, title, deleted_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(source, external_id) DO UPDATE SET title = ?3, deleted_at = ?4",
                rusqlite::params![
                    source,
                    tombstone_key(&source, &external_id),
                    title,
                    now_str()
                ],
            )
            .map_err(|e| format!("记录删除墓碑失败: {}", e))?;
        }
    }
    tx.commit().map_err(|e| format!("提交删除失败: {}", e))?;
    Ok(removed > 0)
}
