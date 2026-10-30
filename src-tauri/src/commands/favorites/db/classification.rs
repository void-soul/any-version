use super::*;

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
pub fn find_favorite_id(
    conn: &Connection,
    source: &str,
    external_id: &str,
) -> Result<Option<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM favorite WHERE source = ?1 AND external_id = ?2")
        .map_err(|e| format!("查询条目失败: {e}"))?;
    let found = stmt
        .query_row(rusqlite::params![source, external_id], |r| {
            r.get::<_, i64>(0)
        })
        .ok();
    Ok(found)
}

/// 追加一条「条目 ↔ 分类」挂载（已存在则忽略，不会重复）。
pub fn link_item_category(
    conn: &Connection,
    favorite_id: i64,
    category_id: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO favorite_item_category (favorite_id, category_id, source) VALUES (?1, ?2, 'bookmark')",
        rusqlite::params![favorite_id, category_id],
    )
    .map_err(|e| format!("挂载分类失败: {e}"))?;
    Ok(())
}

/// 在指定父级下按名字取分类，没有就建（供导入时按浏览器书签目录建树用）。
pub fn ensure_category(
    conn: &Connection,
    parent_id: Option<i64>,
    name: &str,
) -> Result<i64, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM favorite_category WHERE parent_id IS ?1 AND name = ?2")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let found = stmt
        .query_row(rusqlite::params![parent_id, name], |r| r.get::<_, i64>(0))
        .ok();
    drop(stmt);
    match found {
        Some(id) => Ok(id),
        None => create_category(conn, name, parent_id, "topic"),
    }
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
    create_category(conn, name, None, "topic")
}
