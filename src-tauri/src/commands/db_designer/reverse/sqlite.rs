use super::*;

/// 读一个 SQLite 库文件，反推出设计文档（只读打开，绝不写库）
pub fn reverse_sqlite(path: &str, name: &str) -> Result<DbDesignDocument, String> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|e| format!("打开 SQLite 失败 {}: {}", path, e))?;

    let mut doc = DbDesignDocument {
        id: new_id("dbd"),
        name: name.to_string(),
        description: format!("由 SQLite 库反推：{}", path),
        dialect: "sqlite".to_string(),
        folder_id: None,
        decisions: Vec::new(),
        common_fields: Vec::new(),
        nodes: Vec::new(),
        relations: Vec::new(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    let mut stmt = conn
        .prepare(
            "SELECT name, type, COALESCE(sql,'') FROM sqlite_master \
             WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY rowid",
        )
        .map_err(|e| format!("读取 sqlite_master 失败: {}", e))?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("遍历表失败: {}", e))?;
    let objects: Vec<(String, String, String)> = rows.filter_map(|r| r.ok()).collect();

    for (obj_name, obj_type, obj_sql) in &objects {
        if obj_type == "view" {
            let sql = obj_sql
                .rsplit_once(" AS ")
                .map(|(_, b)| b.trim().to_string())
                .unwrap_or_else(|| obj_sql.clone());
            doc.nodes.push(DbDesignNode {
                id: new_id("v"),
                kind: "view".to_string(),
                name: obj_name.clone(),
                comment: String::new(),
                x: (doc.nodes.len() % 4) as f64 * 280.0,
                y: (doc.nodes.len() / 4) as f64 * 240.0,
                table: None,
                view: Some(DbViewBody { sql }),
                ..Default::default()
            });
            continue;
        }

        // 字段
        let mut fields: Vec<DbField> = Vec::new();
        let mut pk_order: Vec<(i64, String)> = Vec::new();
        let mut ti = conn
            .prepare(&format!(
                "PRAGMA table_info(\"{}\")",
                obj_name.replace('"', "\"\"")
            ))
            .map_err(|e| format!("PRAGMA table_info 失败: {}", e))?;
        let cols = ti
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,            // cid
                    r.get::<_, String>(1)?,         // name
                    r.get::<_, String>(2)?,         // type
                    r.get::<_, i64>(3)?,            // notnull
                    r.get::<_, Option<String>>(4)?, // dflt
                    r.get::<_, i64>(5)?,            // pk
                ))
            })
            .map_err(|e| format!("读取字段失败: {}", e))?;
        for c in cols.filter_map(|r| r.ok()) {
            let mut f = DbField {
                name: c.1.clone(),
                r#type: sqlite_type_to_logical(&c.2),
                nullable: c.3 == 0,
                default: c.4.clone(),
                comment: String::new(),
                pk: c.5 > 0,
                auto_increment: false,
                unique: false,
            };
            if f.pk {
                f.nullable = false;
                pk_order.push((c.5, c.1.clone()));
            }
            fields.push(f);
        }
        pk_order.sort_by_key(|(k, _)| *k);

        // INTEGER PRIMARY KEY + 建表语句里写了 AUTOINCREMENT → 自增
        let has_autoincrement = obj_sql.to_lowercase().contains("autoincrement");
        if has_autoincrement && pk_order.len() == 1 {
            if let Some(f) = fields.iter_mut().find(|f| f.pk) {
                // 类型仍用逻辑类型 bigint（导出 SQLite 时才映射成 INTEGER）——
                // 别在反推阶段塞 "integer"，它不在逻辑类型白名单里，校验会拦下
                f.auto_increment = true;
            }
        }

        // 索引（跳过主键索引 origin='pk'）
        let mut indexes: Vec<DbIndex> = Vec::new();
        if let Ok(mut il) = conn.prepare(&format!(
            "PRAGMA index_list(\"{}\")",
            obj_name.replace('"', "\"\"")
        )) {
            let lists = il
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(1)?, // name
                        r.get::<_, i64>(2)?,    // unique
                        r.get::<_, String>(3)?, // origin
                    ))
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                .unwrap_or_default();
            for (iname, uniq, origin) in lists {
                if origin == "pk" {
                    continue;
                }
                let cols = conn
                    .prepare(&format!(
                        "PRAGMA index_info(\"{}\")",
                        iname.replace('"', "\"\"")
                    ))
                    .ok()
                    .and_then(|mut s| {
                        s.query_map([], |r| Ok(r.get::<_, Option<String>>(2)?))
                            .ok()
                            .map(|rows| {
                                rows.filter_map(|r| r.ok().flatten())
                                    .collect::<Vec<String>>()
                            })
                    })
                    .unwrap_or_default();
                if cols.is_empty() {
                    continue;
                }
                indexes.push(DbIndex {
                    name: iname,
                    kind: if uniq == 1 {
                        "unique".to_string()
                    } else {
                        "index".to_string()
                    },
                    fields: cols,
                });
            }
        }

        doc.nodes.push(DbDesignNode {
            id: new_id("t"),
            kind: "table".to_string(),
            name: obj_name.clone(),
            comment: String::new(),
            x: (doc.nodes.len() % 4) as f64 * 280.0,
            y: (doc.nodes.len() / 4) as f64 * 240.0,
            table: Some(DbTableBody { fields, indexes }),
            view: None,
            ..Default::default()
        });
    }

    // 外键（SQLite 的 pragma 能拿全，是四种库里最完整的）
    let mut by_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for n in &doc.nodes {
        by_name.insert(n.name.to_lowercase(), n.id.clone());
    }
    for (obj_name, _, _) in &objects {
        let Some(child_id) = by_name.get(&obj_name.to_lowercase()).cloned() else {
            continue;
        };
        let Ok(mut fl) = conn.prepare(&format!(
            "PRAGMA foreign_key_list(\"{}\")",
            obj_name.replace('"', "\"\"")
        )) else {
            continue;
        };
        let Ok(rows) = fl.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,            // id
                r.get::<_, i64>(1)?,            // seq
                r.get::<_, String>(2)?,         // table
                r.get::<_, String>(3)?,         // from
                r.get::<_, Option<String>>(4)?, // to
                r.get::<_, String>(5)?,         // on_update
                r.get::<_, String>(6)?,         // on_delete
            ))
        }) else {
            continue;
        };
        let mut grouped: std::collections::HashMap<
            i64,
            (String, Vec<(i64, String, String)>, String, String),
        > = std::collections::HashMap::new();
        for f in rows.filter_map(|r| r.ok()) {
            let entry = grouped
                .entry(f.0)
                .or_insert_with(|| (f.2.clone(), Vec::new(), f.5.clone(), f.6.clone()));
            entry
                .1
                .push((f.1, f.3.clone(), f.4.clone().unwrap_or_else(|| f.3.clone())));
        }
        for (_, (parent_table, mut cols, on_update, on_delete)) in grouped {
            let Some(parent_id) = by_name.get(&parent_table.to_lowercase()).cloned() else {
                continue; // 指向库外/未导入的表：跳过，不留悬空引用
            };
            cols.sort_by_key(|(s, _, _)| *s);
            for (_, from_field, to_field) in cols {
                doc.relations.push(DbDesignRelation {
                    id: new_id("r"),
                    name: String::new(),
                    from: DbRelationEnd {
                        node: child_id.clone(),
                        field: from_field,
                    },
                    to: DbRelationEnd {
                        node: parent_id.clone(),
                        field: to_field,
                    },
                    kind: "1-n".to_string(),
                    on_delete: on_delete.clone(),
                    on_update: on_update.clone(),
                    mirror: false,
                });
            }
        }
    }

    // 与 DDL 路径同一个理由：真实 SQLite 库的主键就叫 id、字段就叫 name，
    // 不规范化的话用户逆向完会直接卡在校验上，连导出都做不了
    let _normalized = normalize_naming(&mut doc);
    Ok(doc)
}
