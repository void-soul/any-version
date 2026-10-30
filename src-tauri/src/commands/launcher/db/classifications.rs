use super::*;

// ---------------------- 分类操作 ----------------------

pub fn list_classifications() -> Result<Vec<Classification>, String> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.parent_id, c.name, c.classification_type, c.data, c.shortcut_key, c.global_shortcut_key, c.sort_order,
             (SELECT COUNT(1) FROM launcher_item i WHERE i.classification_id = c.id) as item_cnt
             FROM launcher_classification c
             ORDER BY c.sort_order ASC"
        ).map_err(|e| e.to_string())?;

        let mut all_list = Vec::new();
        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let parent_id: Option<i64> = row.get(1)?;
                let name: String = row.get(2)?;
                let classification_type: i32 = row.get(3)?;
                let data_str: String = row.get(4)?;
                let shortcut_key: Option<String> = row.get(5)?;
                let global_shortcut_key_int: i32 = row.get(6)?;
                let sort_order: i32 = row.get(7)?;
                let item_cnt: i64 = row.get(8)?;

                let data: ClassificationData = serde_json::from_str(&data_str).unwrap_or_default();

                Ok(Classification {
                    id,
                    parent_id,
                    name,
                    classification_type,
                    data,
                    shortcut_key,
                    global_shortcut_key: global_shortcut_key_int != 0,
                    order: sort_order,
                    child_list: Some(Vec::new()),
                    item_count: Some(item_cnt as usize),
                })
            })
            .map_err(|e| e.to_string())?;

        for r in rows {
            if let Ok(c) = r {
                all_list.push(c);
            }
        }

        // 返回全部分类（扁平列表），前端按 parentId 自行构建层级树。
        // 注意：不能只返回顶级分类，否则前端 filter(c => c.parentId === cat.id)
        // 永远找不到子分类，导致有子分类的分类显示空白。
        Ok(all_list)
    })
}

pub fn save_classification(cls: &Classification) -> Result<i64, String> {
    with_conn(|conn| {
        let now = Local::now().to_rfc3339();
        let data_json = serde_json::to_string(&cls.data).unwrap_or_else(|_| "{}".to_string());

        if cls.id > 0 {
            conn.execute(
                r#"
                UPDATE launcher_classification
                SET parent_id = ?1, name = ?2, classification_type = ?3, data = ?4,
                    shortcut_key = ?5, global_shortcut_key = ?6, sort_order = ?7, updated_at = ?8
                WHERE id = ?9
                "#,
                params![
                    cls.parent_id,
                    cls.name,
                    cls.classification_type,
                    data_json,
                    cls.shortcut_key,
                    if cls.global_shortcut_key { 1 } else { 0 },
                    cls.order,
                    now,
                    cls.id,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(cls.id)
        } else {
            let max_order: i32 = conn.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_classification WHERE parent_id IS ?1",
                params![cls.parent_id],
                |row| row.get(0)
            ).unwrap_or(0);

            conn.execute(
                r#"
                INSERT INTO launcher_classification (parent_id, name, classification_type, data, shortcut_key, global_shortcut_key, sort_order, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                "#,
                params![
                    cls.parent_id,
                    cls.name,
                    cls.classification_type,
                    data_json,
                    cls.shortcut_key,
                    if cls.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    now,
                ],
            ).map_err(|e| e.to_string())?;

            let new_id = conn.last_insert_rowid();
            Ok(new_id)
        }
    })
}

/// 收集 id 及其**全部**子孙分类（不只是直接子级）。
///
/// 旧实现只查一层子分类，孙级分类会留下 `parent_id` 指向已删除的父节点 ——
/// 它们在树上不可达、里面的项目也跟着看不见删不掉，成了永久孤儿数据。
pub(super) fn descendant_ids_on(conn: &Connection, id: i64) -> Result<Vec<i64>, String> {
    let mut all = vec![id];
    let mut frontier = vec![id];
    let mut guard = 0usize;
    while !frontier.is_empty() && guard < 1000 {
        guard += 1;
        let mut stmt = conn
            .prepare("SELECT id FROM launcher_classification WHERE parent_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([frontier[0]], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        let mut next: Vec<i64> = Vec::new();
        for r in rows {
            if let Ok(cid) = r {
                // 防环兜底：脏数据成环时不重复入列
                if !all.contains(&cid) {
                    all.push(cid);
                    next.push(cid);
                }
            }
        }
        frontier.remove(0);
        frontier.extend(next);
    }
    Ok(all)
}

/// target 是否是 id 的子孙（用于拦截「把分类迁到它自己的子树里」）。
pub(super) fn is_descendant_of(conn: &Connection, id: i64, target: i64) -> Result<bool, String> {
    let mut cursor = Some(target);
    let mut guard = 0;
    while let Some(cid) = cursor {
        if cid == id {
            return Ok(true);
        }
        guard += 1;
        if guard > 1000 {
            return Err("分类层级过深，已中止".to_string());
        }
        cursor = conn
            .query_row(
                "SELECT parent_id FROM launcher_classification WHERE id = ?1",
                params![cid],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(false)
}

/// 删除分类的两种方式。
pub enum DeleteMode {
    /// 连同子分类与所有项目一起删
    Cascade,
    /// 保留内容：把子分类与项目迁到 new_parent_id 之下，只删这个空壳分类
    Reassign { new_parent_id: i64 },
}

/// 删除分类，并**保证不留孤儿**。
///
/// - `Cascade`：整棵子树 + 树下所有项目一起删（旧实现漏了孙级，这里按整棵子树删）；
/// - `Reassign`：先把直接子分类与直属项目迁到新上级，再删分类本身 ——
///   这样内容都能在新位置看到，不会出现「看不见也删不掉」的项目。
pub fn delete_classification(
    id: i64,
    mode: DeleteMode,
) -> Result<DeleteClassificationResult, String> {
    with_conn(|conn| delete_classification_on(conn, id, mode))
}

/// 对给定连接执行删除（与全局连接解耦，单测直接跑这段真实逻辑）。
pub(crate) fn delete_classification_on(
    conn: &Connection,
    id: i64,
    mode: DeleteMode,
) -> Result<DeleteClassificationResult, String> {
    {
        // 生产侧全局连接由 with_conn 的 Mutex 串行化，这里不会再有并发写；
        // 用 unchecked_transaction 是为了让函数收 &Connection（单测也要调它）。
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        match mode {
            DeleteMode::Cascade => {
                let ids = descendant_ids_on(&tx, id)?;
                let mut deleted_items = 0usize;
                for cid in &ids {
                    deleted_items += tx
                        .execute(
                            "DELETE FROM launcher_item WHERE classification_id = ?1",
                            params![cid],
                        )
                        .map_err(|e| e.to_string())?;
                }
                let mut deleted_categories = 0usize;
                for cid in &ids {
                    deleted_categories += tx
                        .execute(
                            "DELETE FROM launcher_classification WHERE id = ?1",
                            params![cid],
                        )
                        .map_err(|e| e.to_string())?;
                }
                tx.commit().map_err(|e| e.to_string())?;
                Ok(DeleteClassificationResult {
                    deleted_categories,
                    deleted_items,
                    ..Default::default()
                })
            }
            DeleteMode::Reassign { new_parent_id } => {
                if new_parent_id == id {
                    return Err("不能把内容迁移到自己之下".to_string());
                }
                if is_descendant_of(&tx, id, new_parent_id)? {
                    return Err("目标分类不能位于待删除分类之下（会把自己也一起搬走）".to_string());
                }
                // 新上级必须真实存在
                let exists: bool = tx
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM launcher_classification WHERE id = ?1",
                        params![new_parent_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if !exists {
                    return Err("目标分类不存在".to_string());
                }

                // ① 直接子分类搬过去（保留各自的孙级关系与顺序）
                let child_ids: Vec<i64> = {
                    let mut stmt = tx
                        .prepare("SELECT id FROM launcher_classification WHERE parent_id = ?1 ORDER BY sort_order ASC")
                        .map_err(|e| e.to_string())?;
                    let rows = stmt
                        .query_map([id], |row| row.get::<_, i64>(0))
                        .map_err(|e| e.to_string())?;
                    let mut v = Vec::new();
                    for r in rows {
                        if let Ok(c) = r {
                            v.push(c);
                        }
                    }
                    v
                };
                let max_order: i32 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_classification WHERE parent_id IS ?1",
                        params![new_parent_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                for (i, cid) in child_ids.iter().enumerate() {
                    tx.execute(
                        "UPDATE launcher_classification SET parent_id = ?1, sort_order = ?2 WHERE id = ?3",
                        params![new_parent_id, max_order + i as i32, cid],
                    )
                    .map_err(|e| e.to_string())?;
                }

                // ② 直属项目也搬过去：否则它们会挂在已删除的分类 id 上成为孤儿
                let moved_items = tx
                    .execute(
                        "UPDATE launcher_item SET classification_id = ?1 WHERE classification_id = ?2",
                        params![new_parent_id, id],
                    )
                    .map_err(|e| e.to_string())?;

                // ③ 删掉已经搬空的分类本身
                let deleted_categories = tx
                    .execute(
                        "DELETE FROM launcher_classification WHERE id = ?1",
                        params![id],
                    )
                    .map_err(|e| e.to_string())?;

                tx.commit().map_err(|e| e.to_string())?;
                Ok(DeleteClassificationResult {
                    deleted_categories,
                    moved_categories: child_ids.len(),
                    moved_items,
                    ..Default::default()
                })
            }
        }
    }
}

pub fn reorder_classifications(orders: Vec<(i64, i32)>) -> Result<(), String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (id, sort_order) in orders {
            tx.execute(
                "UPDATE launcher_classification SET sort_order = ?1 WHERE id = ?2",
                params![sort_order, id],
            )
            .ok();
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}
