use super::*;

// ---------------------- 项目操作 ----------------------

pub fn list_items_by_classification(cls_id: i64) -> Result<Vec<Item>, String> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open
             FROM launcher_item
             WHERE classification_id = ?1
             ORDER BY sort_order ASC, id ASC"
        ).map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([cls_id], |row| {
                let id: i64 = row.get(0)?;
                let classification_id: i64 = row.get(1)?;
                let name: String = row.get(2)?;
                let item_type: i32 = row.get(3)?;
                let data_str: String = row.get(4)?;
                let shortcut_key: Option<String> = row.get(5)?;
                let global_shortcut_key_int: i32 = row.get(6)?;
                let sort_order: i32 = row.get(7)?;

                let open_number: i64 = row.get(8)?;
                let last_open: i64 = row.get(9)?;
                let mut data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();
                // 启动次数/最近启动时间属于独立数据库列，覆盖旧版本 data 缓存。
                data.open_number = open_number;
                data.last_open = last_open;

                Ok(Item {
                    id,
                    classification_id,
                    name,
                    item_type,
                    data,
                    shortcut_key,
                    global_shortcut_key: global_shortcut_key_int != 0,
                    order: sort_order,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut items = Vec::new();
        for r in rows {
            if let Ok(it) = r {
                items.push(it);
            }
        }
        Ok(items)
    })
}

pub fn list_all_items() -> Result<Vec<Item>, String> {
    with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open
             FROM launcher_item
             ORDER BY sort_order ASC, id ASC"
        ).map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let classification_id: i64 = row.get(1)?;
                let name: String = row.get(2)?;
                let item_type: i32 = row.get(3)?;
                let data_str: String = row.get(4)?;
                let shortcut_key: Option<String> = row.get(5)?;
                let global_shortcut_key_int: i32 = row.get(6)?;
                let sort_order: i32 = row.get(7)?;

                let open_number: i64 = row.get(8)?;
                let last_open: i64 = row.get(9)?;
                let mut data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();
                // 启动次数/最近启动时间属于独立数据库列，覆盖旧版本 data 缓存。
                data.open_number = open_number;
                data.last_open = last_open;

                Ok(Item {
                    id,
                    classification_id,
                    name,
                    item_type,
                    data,
                    shortcut_key,
                    global_shortcut_key: global_shortcut_key_int != 0,
                    order: sort_order,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut items = Vec::new();
        for r in rows {
            if let Ok(it) = r {
                items.push(it);
            }
        }
        Ok(items)
    })
}

/// 按 id 读取单个项目（全局快捷键直接启动时使用）。
pub fn get_item(id: i64) -> Result<Option<Item>, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open
                 FROM launcher_item WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map([id], |row| {
                let id: i64 = row.get(0)?;
                let classification_id: i64 = row.get(1)?;
                let name: String = row.get(2)?;
                let item_type: i32 = row.get(3)?;
                let data_str: String = row.get(4)?;
                let shortcut_key: Option<String> = row.get(5)?;
                let global_shortcut_key_int: i32 = row.get(6)?;
                let sort_order: i32 = row.get(7)?;
                let open_number: i64 = row.get(8)?;
                let last_open: i64 = row.get(9)?;
                let mut data: ItemData = serde_json::from_str(&data_str).unwrap_or_default();
                data.open_number = open_number;
                data.last_open = last_open;
                Ok(Item {
                    id,
                    classification_id,
                    name,
                    item_type,
                    data,
                    shortcut_key,
                    global_shortcut_key: global_shortcut_key_int != 0,
                    order: sort_order,
                })
            })
            .map_err(|e| e.to_string())?;

        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    })
}

/// 所有配置了「全局快捷键」的项目：(id, 快捷键)
pub fn list_items_with_global_hotkey() -> Result<Vec<(i64, String)>, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, shortcut_key FROM launcher_item
                 WHERE global_shortcut_key = 1 AND shortcut_key IS NOT NULL AND TRIM(shortcut_key) <> ''",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows.flatten() {
            out.push(r);
        }
        Ok(out)
    })
}

/// 所有配置了「全局快捷键」的分类：(id, 快捷键)
pub fn list_classifications_with_global_hotkey() -> Result<Vec<(i64, String)>, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, shortcut_key FROM launcher_classification
                 WHERE global_shortcut_key = 1 AND shortcut_key IS NOT NULL AND TRIM(shortcut_key) <> ''",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for r in rows.flatten() {
            out.push(r);
        }
        Ok(out)
    })
}

pub fn save_item(item: &Item) -> Result<i64, String> {
    with_conn(|conn| {
        let now = Local::now().to_rfc3339();
        let mut data = item.data.clone();
        data.open_number = item.data.open_number;
        data.last_open = item.data.last_open;
        let data_json = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());

        if item.id > 0 {
            conn.execute(
                r#"
                UPDATE launcher_item
                SET classification_id = ?1, name = ?2, item_type = ?3, data = ?4,
                    shortcut_key = ?5, global_shortcut_key = ?6, sort_order = ?7,
                    open_number = ?8, last_open = ?9, updated_at = ?10
                WHERE id = ?11
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    item.order,
                    item.data.open_number,
                    item.data.last_open,
                    now,
                    item.id,
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(item.id)
        } else {
            let max_order: i32 = conn.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_item WHERE classification_id = ?1",
                params![item.classification_id],
                |row| row.get(0)
            ).unwrap_or(0);

            conn.execute(
                r#"
                INSERT INTO launcher_item (classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    item.data.open_number,
                    item.data.last_open,
                    now,
                ],
            ).map_err(|e| e.to_string())?;

            let new_id = conn.last_insert_rowid();
            Ok(new_id)
        }
    })
}

pub fn batch_add_items(items: Vec<Item>) -> Result<Vec<i64>, String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let now = Local::now().to_rfc3339();
        let mut inserted_ids = Vec::new();

        for item in items {
            let max_order: i32 = tx.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_item WHERE classification_id = ?1",
                params![item.classification_id],
                |row| row.get(0)
            ).unwrap_or(0);

            let data = item.data.clone();
            let data_json = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());

            tx.execute(
                r#"
                INSERT INTO launcher_item (classification_id, name, item_type, data, shortcut_key, global_shortcut_key, sort_order, open_number, last_open, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
                "#,
                params![
                    item.classification_id,
                    item.name,
                    item.item_type,
                    data_json,
                    item.shortcut_key,
                    if item.global_shortcut_key { 1 } else { 0 },
                    max_order + 1,
                    item.data.open_number,
                    item.data.last_open,
                    now,
                ],
            ).map_err(|e| e.to_string())?;

            inserted_ids.push(tx.last_insert_rowid());
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(inserted_ids)
    })
}

pub fn delete_item(id: i64) -> Result<(), String> {
    with_conn(|conn| {
        conn.execute("DELETE FROM launcher_item WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// 批量删除所有检测结果为「不存在」的项目（data.exists == false），返回删除数量。
/// 未检测过（exists 为 null）或存在（exists == true）的项目不受影响。
pub fn delete_invalid_items() -> Result<usize, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, data FROM launcher_item WHERE data LIKE '%\"exists\":false%'")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let data_str: String = row.get(1)?;
                Ok((id, data_str))
            })
            .map_err(|e| e.to_string())?;

        let mut invalid_ids: Vec<i64> = Vec::new();
        for r in rows {
            if let Ok((id, data_str)) = r {
                // 精确判定：反序列化后 exists 为 false 才删除，避免误伤文本中偶然出现该串的项目
                if let Ok(data) = serde_json::from_str::<ItemData>(&data_str) {
                    if data.exists == Some(false) {
                        invalid_ids.push(id);
                    }
                }
            }
        }
        if invalid_ids.is_empty() {
            return Ok(0);
        }
        drop(stmt);
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for id in &invalid_ids {
            tx.execute("DELETE FROM launcher_item WHERE id = ?1", params![id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(invalid_ids.len())
    })
}

pub fn reorder_items(orders: Vec<(i64, i32)>) -> Result<(), String> {
    with_conn(|conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (id, sort_order) in orders {
            // 不能吞掉错误：序号写失败必须让前端弹出「保存排序失败」，否则会表现为
            // 「拖完看着生效了，重新打开又变回原样」。
            tx.execute(
                "UPDATE launcher_item SET sort_order = ?1 WHERE id = ?2",
                params![sort_order, id],
            )
            .map_err(|e| format!("更新项目排序失败: {}", e))?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// 批量移动子分类：把 source 分类下的所有【直接子分类】整体移动到 target 分类之下。
/// 每个被移动的子分类完整保留其自身的子孙层级（parent_id 不变）与项目（classification_id 不变），
/// 只是把直接子分类的 parent_id 改成 target。返回移动的子分类数量。source 分类自身不移动。
pub fn move_subcategories_to_classification(
    source_id: i64,
    target_id: i64,
) -> Result<usize, String> {
    with_conn(|conn| {
        if source_id == target_id {
            return Err("源分类与目标分类不能相同".to_string());
        }

        // 校验 target 不能是 source 的子孙（否则会把 target 也一并搬走造成循环）
        {
            let mut cursor = Some(target_id);
            let mut guard = 0;
            while let Some(cid) = cursor {
                if cid == source_id {
                    return Err("目标分类不能位于源分类之下".to_string());
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
        }

        // 收集 source 的所有直接子分类 id（按原顺序）
        let child_ids: Vec<i64> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM launcher_classification WHERE parent_id = ?1 ORDER BY sort_order ASC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([source_id], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())?;
            let mut v = Vec::new();
            for r in rows {
                if let Ok(i) = r {
                    v.push(i);
                }
            }
            v
        };

        if child_ids.is_empty() {
            return Ok(0);
        }

        // 目标分类下当前最大 sort_order
        let max_order: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM launcher_classification WHERE parent_id IS ?1",
                params![target_id],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (i, cid) in child_ids.iter().enumerate() {
            tx.execute(
                "UPDATE launcher_classification SET parent_id = ?1, sort_order = ?2 WHERE id = ?3",
                params![target_id, max_order + i as i32, cid],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(child_ids.len())
    })
}

pub fn increment_item_open_count(id: i64) -> Result<(), String> {
    with_conn(|conn| increment_item_open_count_on(conn, id))
}
