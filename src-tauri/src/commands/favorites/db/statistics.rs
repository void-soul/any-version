use super::*;

/// 概览：总数 / 未归类 / 已失效 / 各源条数 / 各分类条数。
pub fn stats(conn: &Connection) -> Result<FavoriteStats, String> {
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM favorite", [], |r| r.get(0))
        .map_err(|e| format!("统计总数失败: {}", e))?;
    let gone: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM favorite WHERE status = 'gone'",
            [],
            |r| r.get(0),
        )
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

/// 条目总数（测试与概览用）。
pub fn count_all(conn: &Connection) -> Result<usize, String> {
    conn.query_row("SELECT COUNT(*) FROM favorite", [], |row| row.get(0))
        .map_err(|e| format!("统计收藏条目失败: {}", e))
}
