use super::*;

pub(super) fn load_flat_categories(conn: &Connection) -> Result<Vec<CategoryFlat>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.parent_id, c.name, c.sort_order, c.kind, \
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
                kind: r.get(4)?,
                count: r.get(5)?,
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
                kind: c.kind.clone(),
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
            let children: Vec<i64> = nodes
                .get(id)
                .map(|n| n.children.iter().map(|c| c.id).collect())
                .unwrap_or_default();
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

pub(super) fn category_path_of(
    id: i64,
    parent_of: &std::collections::HashMap<i64, Option<i64>>,
    name_of: &std::collections::HashMap<i64, String>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cur = Some(id);
    while let Some(cid) = cur {
        // 老库/异常数据可能成环：遇到走过的节点就停，别让路径查询死循环
        if !seen.insert(cid) {
            break;
        }
        if let Some(name) = name_of.get(&cid) {
            parts.push(name.clone());
        }
        cur = parent_of.get(&cid).copied().flatten();
    }
    parts.reverse();
    parts.join("/")
}

/// 整棵分类树（只读 ID / 名称 / 上级 ID / 路径）。
///
/// 刻意不查条目数与示例标题：整理类目只改父子关系，模型不需要条目内容。
pub fn category_briefs(conn: &Connection) -> Result<Vec<CategoryBrief>, String> {
    let mut stmt = conn
        .prepare("SELECT id, parent_id, name, kind FROM favorite_category")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CategoryBrief {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                path: String::new(),
            })
        })
        .map_err(|e| format!("查询分类失败: {e}"))?;

    let mut out: Vec<CategoryBrief> = Vec::new();
    let mut parent_of: std::collections::HashMap<i64, Option<i64>> =
        std::collections::HashMap::new();
    let mut name_of: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
    for row in rows {
        let brief = row.map_err(|e| format!("读取分类失败: {e}"))?;
        parent_of.insert(brief.id, brief.parent_id);
        name_of.insert(brief.id, brief.name.clone());
        out.push(brief);
    }
    // 路径依赖父级，先收齐再统一算一遍
    for brief in out.iter_mut() {
        brief.path = category_path_of(brief.id, &parent_of, &name_of);
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// 现有分类的完整路径（只读名称与父级，供 AI 归类时「优先复用已有层级」）。
///
/// 归类每批都要读一次，所以只查名称与父级，不碰条目关联。
pub fn category_paths(conn: &Connection, limit: usize) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id, parent_id, name FROM favorite_category WHERE kind != 'follow'")
        .map_err(|e| format!("查询分类路径失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("查询分类路径失败: {e}"))?;
    let mut parent_of = std::collections::HashMap::new();
    let mut name_of = std::collections::HashMap::new();
    for row in rows {
        let (id, parent_id, name) = row.map_err(|e| format!("读取分类路径失败: {e}"))?;
        parent_of.insert(id, parent_id);
        name_of.insert(id, name);
    }
    let mut out: Vec<String> = name_of
        .keys()
        .filter_map(|id| {
            let path = category_path_of(*id, &parent_of, &name_of);
            (!path.is_empty()).then_some(path)
        })
        .collect();
    out.sort();
    out.dedup();
    out.truncate(limit);
    Ok(out)
}

/// 取某分类的全部子孙 id（含自己），用于「按分类筛选时把子分类也算进来」。
pub fn descendant_ids(conn: &Connection, id: i64) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id, parent_id FROM favorite_category")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?))
        })
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
pub(super) fn sibling_name_taken(
    conn: &Connection,
    parent_id: Option<i64>,
    name: &str,
    except_id: Option<i64>,
) -> Result<bool, String> {
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
pub fn create_category(
    conn: &Connection,
    name: &str,
    parent_id: Option<i64>,
    kind: &str,
) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分类名不能为空".to_string());
    }
    if let Some(pid) = parent_id {
        let exists = conn
            .query_row(
                "SELECT 1 FROM favorite_category WHERE id = ?1",
                [pid],
                |_| Ok(()),
            )
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
        .query_row(
            "SELECT parent_id FROM favorite_category WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
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
pub fn find_child_id(
    conn: &Connection,
    parent_id: Option<i64>,
    name: &str,
) -> Result<Option<i64>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM favorite_category WHERE parent_id IS ?1 AND name = ?2")
        .map_err(|e| format!("查询分类失败: {e}"))?;
    let found = stmt
        .query_row(rusqlite::params![parent_id, name], |r| r.get::<_, i64>(0))
        .ok();
    Ok(found)
}

/// 删除分类（连同所有子孙分类与它们之间的挂载关系）。
///
/// 条目本身**不删**：分类只是标签，删掉分类不该把用户的收藏一起删掉
/// （启动模块删分类会连带删项目，那是「项目归属」语义；这里是「打标签」语义）。
pub fn delete_category(conn: &Connection, id: i64) -> Result<usize, String> {
    let ids = descendant_ids(conn, id)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
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
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {e}"))?;
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
