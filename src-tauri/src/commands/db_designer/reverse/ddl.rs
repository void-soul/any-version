use super::*;

/// 从建表语句文本反推设计文档（mysqldump / pg_dump / 任意 .sql）
pub fn reverse_ddl(text: &str, dialect: &str, name: &str) -> Result<DbDesignDocument, String> {
    let clean = strip_comments(text);
    let mut doc = DbDesignDocument {
        id: new_id("dbd"),
        name: name.to_string(),
        description: format!("由 DDL 反推（{}）", dialect),
        dialect: if DIALECTS.contains(&dialect) {
            dialect.to_string()
        } else {
            "mysql".to_string()
        },
        folder_id: None,
        decisions: Vec::new(),
        common_fields: Vec::new(),
        nodes: Vec::new(),
        relations: Vec::new(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    // 表名 → 节点 id（外键按表名找节点，所以这里必须先收齐再建关系）
    let mut by_name: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut pending_fks: Vec<(String, String, String, String, String, String)> = Vec::new();

    for stmt in split_statements(&clean) {
        let lower = stmt.to_lowercase();
        if lower.contains("create table") {
            let Some((table_name, body)) = parse_create_table(&stmt) else {
                continue;
            };
            let id = new_id("t");
            let (fields, indexes) = parse_table_body(&body, &lower);
            // 列级 REFERENCES（内联外键，如 `uid INTEGER REFERENCES users(id)`）也要收
            for (from_field, to_table, to_field) in inline_column_refs(&body) {
                pending_fks.push((
                    table_name.clone(),
                    from_field,
                    to_table,
                    to_field,
                    String::new(),
                    String::new(),
                ));
            }
            by_name.insert(table_name.to_lowercase(), id.clone());
            doc.nodes.push(DbDesignNode {
                id,
                kind: "table".to_string(),
                name: table_name,
                comment: String::new(),
                // 反推出来的位置只是「能看」的兜底；用户点「自动布局」会重排（见前端 layout.ts）
                x: (doc.nodes.len() % 4) as f64 * 280.0,
                y: (doc.nodes.len() / 4) as f64 * 240.0,
                table: Some(DbTableBody { fields, indexes }),
                view: None,
                ..Default::default()
            });
        } else if lower.contains("create view") {
            let Some(v) = parse_create_view(&stmt) else {
                continue;
            };
            let id = new_id("v");
            doc.nodes.push(DbDesignNode {
                id,
                kind: "view".to_string(),
                name: v.0,
                comment: String::new(),
                x: (doc.nodes.len() % 4) as f64 * 280.0,
                y: (doc.nodes.len() / 4) as f64 * 240.0,
                table: None,
                view: Some(DbViewBody { sql: v.1 }),
                ..Default::default()
            });
        } else if lower.contains("create index") || lower.contains("create unique index") {
            let Some((idx_name, table, cols, unique)) = parse_create_index(&stmt) else {
                continue;
            };
            let Some(node_id) = by_name.get(&table.to_lowercase()).cloned() else {
                continue;
            };
            if let Some(node) = doc.nodes.iter_mut().find(|n| n.id == node_id) {
                if let Some(t) = node.table.as_mut() {
                    t.indexes.push(DbIndex {
                        name: idx_name,
                        kind: if unique {
                            "unique".to_string()
                        } else {
                            "index".to_string()
                        },
                        fields: cols,
                    });
                }
            }
        } else if lower.contains("alter table") && lower.contains("foreign key") {
            if let Some(fk) = parse_alter_add_fk(&stmt) {
                pending_fks.push(fk);
            }
        }
    }

    // 表级 FOREIGN KEY（现在表都收齐了，才能把表名解析成节点 id）
    for (child, cols, parent, pcols, on_delete, on_update) in parse_inline_table_fks(&clean) {
        for (i, c) in cols.iter().enumerate() {
            let pc = pcols
                .get(i)
                .cloned()
                .unwrap_or_else(|| pcols.first().cloned().unwrap_or_default());
            pending_fks.push((
                child.clone(),
                c.clone(),
                parent.clone(),
                pc,
                on_delete.clone(),
                on_update.clone(),
            ));
        }
    }

    for (child, from_field, parent, to_field, on_delete, on_update) in pending_fks {
        let (Some(child_id), Some(parent_id)) = (
            by_name.get(&child.to_lowercase()).cloned(),
            by_name.get(&parent.to_lowercase()).cloned(),
        ) else {
            continue; // 外键指向了不在本文件里的表：跳过，不留悬空引用
        };
        if child_id == parent_id || from_field.is_empty() || to_field.is_empty() {
            continue;
        }
        doc.relations.push(DbDesignRelation {
            id: new_id("r"),
            name: String::new(),
            from: DbRelationEnd {
                node: child_id,
                field: from_field,
            },
            to: DbRelationEnd {
                node: parent_id,
                field: to_field,
            },
            kind: "1-n".to_string(),
            on_delete,
            on_update,
            mirror: false,
        });
    }

    // 外来数据不可能天然满足命名规范（真实库里主键就叫 id、字段就叫 name），
    // 而命名规范是 error 级、会阻断后续保存/导出 —— 所以在这里就地规范化一次，
    // 否则用户逆向一个真实的库就会陷入「满屏报错且导不出来」的死锁。
    // 外来数据不可能天然满足命名规范（真实库里主键就叫 id、字段就叫 name），
    // 而命名规范是 error 级、会阻断后续保存/导出 —— 所以在这里就地规范化一次，
    // 否则用户逆向一个真实的库就会陷入「满屏报错且导不出来」的死锁。
    // 规范化改了什么，由前端在载入后展示（返回值里带明细）。
    let _normalized = normalize_naming(&mut doc);

    let report = validate(&doc);
    if !report.is_ok() {
        // 反推出来的文档不该自己就非法；真出现时把问题带回去，别硬塞给用户
        return Err(format!(
            "DDL 解析结果自检未通过：\n- {}",
            report.errors.join("\n- ")
        ));
    }
    Ok(doc)
}

/// `CREATE TABLE [IF NOT EXISTS] <name> (...)` → (表名, 括号内正文)
pub(super) fn parse_create_table(stmt: &str) -> Option<(String, String)> {
    let lower = stmt.to_lowercase();
    let idx = lower.find("create table")? + "create table".len();
    let rest = stmt[idx..].trim_start();
    let rest = {
        let l = rest.to_lowercase();
        if l.starts_with("if not exists") {
            rest["if not exists".len()..].trim_start()
        } else {
            rest
        }
    };
    let open = rest.find('(')?;
    let name = normalize_qualified(&rest[..open]);
    if name.is_empty() {
        return None;
    }
    let body = rest[open + 1..].trim_end().trim_end_matches(';').trim_end();
    let body = match body.rfind(')') {
        Some(i) => &body[..i],
        None => body,
    };
    Some((name, body.to_string()))
}

/// 表体 → (字段, 索引)；同时把表级外键塞进 pending
pub(super) fn parse_table_body(body: &str, _lower_hint: &str) -> (Vec<DbField>, Vec<DbIndex>) {
    let mut fields: Vec<DbField> = Vec::new();
    let mut indexes: Vec<DbIndex> = Vec::new();
    let mut pk_cols: Vec<String> = Vec::new();

    for item in split_top_level(body, ',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let low = item.to_lowercase();
        // `CONSTRAINT x FOREIGN KEY (...) REFERENCES ...` / `CONSTRAINT uk_x UNIQUE (...)`：
        // 去掉 CONSTRAINT 前缀后按索引/外键处理。**不能当列定义** —— 否则会造出
        // 一个名叫 "constraint" 的假字段（踩过：反推结果里凭空多一列）。
        if low.starts_with("constraint") {
            let rest = item["constraint".len()..].trim_start();
            let rest_low = rest.to_lowercase();
            if rest_low.contains("foreign key") || rest_low.contains("references") {
                continue; // 外键由 parse_inline_table_fks 统一处理
            }
            let (name, cols, unique) = parse_index_clause(rest);
            if !cols.is_empty() {
                indexes.push(DbIndex {
                    name: if name.is_empty() {
                        format!("idx_{}_{}", fields.len(), cols.join("_"))
                    } else {
                        name
                    },
                    kind: if unique {
                        "unique".to_string()
                    } else {
                        "index".to_string()
                    },
                    fields: cols,
                });
            }
            continue;
        }
        // 约束行
        if low.starts_with("primary key") {
            if let (Some(o), Some(c)) = (item.find('('), item.rfind(')')) {
                pk_cols = split_top_level(&item[o + 1..c], ',')
                    .iter()
                    .map(|s| unquote_ident(s))
                    .collect();
            }
            continue;
        }
        if low.starts_with("unique") || low.starts_with("key ") || low.starts_with("index ") {
            let (name, cols, unique) = parse_index_clause(item);
            if !cols.is_empty() {
                indexes.push(DbIndex {
                    name: if name.is_empty() {
                        format!("idx_{}_{}", fields.len(), cols.join("_"))
                    } else {
                        name
                    },
                    kind: if unique {
                        "unique".to_string()
                    } else {
                        "index".to_string()
                    },
                    fields: cols,
                });
            }
            continue;
        }
        // 列定义
        if let Some(f) = parse_column(item) {
            fields.push(f);
        }
    }

    for name in pk_cols {
        if let Some(f) = fields.iter_mut().find(|f| f.name == name) {
            f.pk = true;
            f.nullable = false;
        }
    }
    (fields, indexes)
}

/// 索引/主键里的**列名**清洗：`col(10)` 长度后缀、`ASC`/`DESC` 排序方向、反引号。
///
/// ⚠ 踩过：Navicat 导出的索引是 `` KEY `idx_51`(`openid` ASC, `user_status` ASC) USING BTREE ``，
/// 去掉反引号后还剩「`openid` ASC」。而 `unquote_ident` 要求首尾**都是**反引号，
/// 这种「开头是、结尾不是」的串会原样返回 —— 于是索引引用了名为「`openid` ASC」的字段，
/// 校验判为「引用了不存在的字段」，整个逆向 DDL 失败（真实 dump 里 226 处一起报）。
/// 排序方向不是我们要的语义（设计文档里没有 DESC 索引），直接丢掉。
pub(super) fn clean_index_col(raw: &str) -> String {
    let s = raw.trim();
    // `col` DESC / `col` ASC —— 按空白切开，尾部那个排序关键字丢掉
    let s = match s.rsplit_once(char::is_whitespace) {
        Some((head, tail))
            if tail.eq_ignore_ascii_case("asc") || tail.eq_ignore_ascii_case("desc") =>
        {
            head
        }
        _ => s,
    };
    // `col(10)` 之类的长度后缀
    let s = match s.find('(') {
        Some(i) => s[..i].trim(),
        None => s,
    };
    unquote_ident(s)
}

/// `UNIQUE KEY name (a,b)` / `KEY name (a)` / `UNIQUE (a)` → (名, 列, 是否唯一)
pub(super) fn parse_index_clause(item: &str) -> (String, Vec<String>, bool) {
    let mut rest = item.trim().to_string();
    let low = rest.to_lowercase();
    let unique = low.starts_with("unique");
    // 关键字可能叠着出现：`UNIQUE KEY idx (...)` / `KEY INDEX (...)`。
    // 逐个剥掉，但要求关键字后面紧跟空格或左括号 —— 否则表名叫 `indexed_foo`
    // 会被砍成 `ed_foo`（前缀匹配不等于关键字）。
    loop {
        let low = rest.to_lowercase();
        let mut found: Option<&str> = None;
        for k in ["unique", "key", "index"] {
            if !low.starts_with(k) {
                continue;
            }
            // 关键字后面必须是空格或左括号，否则只是前缀撞名（`indexed_foo`）
            let boundary = match rest[k.len()..].chars().next() {
                Some('(') => true,
                Some(c) => c.is_whitespace(),
                None => true,
            };
            if boundary {
                found = Some(k);
                break;
            }
        }
        let Some(kw) = found else { break };
        rest = rest[kw.len()..].trim_start().to_string();
    }
    // 约束名前缀：`CONSTRAINT uk_x UNIQUE (a)` → uk_x
    let mut name = String::new();
    if rest.to_lowercase().starts_with("constraint") {
        rest = rest["constraint".len()..].trim_start().to_string();
        let (n, tail) = match rest.find(char::is_whitespace) {
            Some(i) => (rest[..i].to_string(), rest[i..].trim_start().to_string()),
            None => (rest.clone(), String::new()),
        };
        name = unquote_ident(&n);
        rest = tail;
        if rest.to_lowercase().starts_with("unique") {
            rest = rest["unique".len()..].trim_start().to_string();
        }
    }
    let Some(o) = rest.find('(') else {
        return (name, Vec::new(), unique);
    };
    // `KEY `idx_51` (...)` 的索引名在左括号之前。
    // 踩过：以前只认 `CONSTRAINT x UNIQUE (...)` 那一种形式，Navicat/MySQL dump 里
    // 满地的 `UNIQUE KEY `idx_51` (...)` 全都拿不到名字，被合成成
    // `idx_<序号>_<列名>` —— 逆向再导出，索引名就跟原库对不上了。
    // （`UNIQUE (` / `KEY (` 这种没名字的写法不能把关键字当名字。）
    let head = rest[..o].trim();
    if !head.is_empty() && !matches!(head.to_lowercase().as_str(), "unique" | "key" | "index") {
        name = unquote_ident(head);
    }
    let Some(c) = rest.rfind(')') else {
        return (name, Vec::new(), unique);
    };
    let cols = split_top_level(&rest[o + 1..c], ',')
        .iter()
        .map(|s| clean_index_col(s))
        .filter(|s| !s.is_empty())
        .collect();
    (name, cols, unique)
}

/// 单列定义 → DbField
pub(super) fn parse_column(item: &str) -> Option<DbField> {
    let s = item.trim();
    if s.is_empty() {
        return None;
    }
    // 名字：第一个标识符（引号包裹或裸词）
    let (name, rest) = if let Some(stripped) = s.strip_prefix('`') {
        let end = stripped.find('`')?;
        (
            stripped[..end].to_string(),
            stripped[end + 1..].trim_start(),
        )
    } else if s.starts_with('"') || s.starts_with('[') {
        let close = if s.starts_with('"') { '"' } else { ']' };
        let end = s[1..].find(close)? + 1;
        (s[1..end].to_string(), s[end + 1..].trim_start())
    } else {
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        (s[..end].to_string(), s[end..].trim_start())
    };
    if name.is_empty() {
        return None;
    }
    // 类型：到空格或左括号为止；**带参数的整段都要带上**（varchar(32) / decimal(12,2) / enum('a','b')），
    // 否则 enum 取值、长度、精度会在解析阶段丢掉（踩过：status enum('new','paid') 反推后没取值）。
    let (type_raw, tail) = match rest.find(|c: char| c == '(' || c.is_whitespace()) {
        Some(i) => {
            let head = rest[..i].trim().to_string();
            if rest[i..].starts_with('(') {
                match match_paren(rest, i) {
                    Some(close) => (
                        format!("{}{}", head, &rest[i..=close]),
                        rest[close + 1..].trim_start().to_string(),
                    ),
                    None => (head, rest[i..].trim_start().to_string()),
                }
            } else {
                (head, rest[i..].trim_start().to_string())
            }
        }
        None => (rest.trim().to_string(), String::new()),
    };
    let mut f = DbField {
        name: unquote_ident(&name),
        r#type: mysql_type_to_logical(&type_raw),
        nullable: true,
        default: None,
        comment: String::new(),
        pk: false,
        auto_increment: false,
        unique: false,
    };
    if f.r#type.base == "text" && type_raw.is_empty() {
        // 没写类型的列：SQLite 允许，按 text 处理
        f.r#type.base = "text".to_string();
    }
    // PG 的 serial / bigserial / smallserial：类型名本身就代表自增
    let type_low = type_raw.to_lowercase();
    if type_low.contains("serial") && !type_low.contains("serializ") {
        f.auto_increment = true;
        f.r#type = DbLogicalType {
            base: if type_low.starts_with("big") {
                "bigint".to_string()
            } else if type_low.starts_with("small") {
                "smallint".to_string()
            } else {
                "int".to_string()
            },
            ..Default::default()
        };
    }

    let tail_low = tail.to_lowercase();
    if tail_low.contains("auto_increment") || tail_low.contains("autoincrement") {
        f.auto_increment = true;
        if f.r#type.base == "tinyint" {
            // MySQL 的 tinyint(1) 常被当布尔用
            f.r#type.base = "boolean".to_string();
        }
    }
    if tail_low.contains("not null") {
        f.nullable = false;
    }
    if tail_low.contains("primary key") {
        f.pk = true;
        f.nullable = false;
    }
    if tail_low.contains("unique") {
        f.unique = true;
    }
    if let Some(i) = tail_low.find("default") {
        let after = tail[i + 7..].trim_start();
        // **保留字面量原样（含引号）**：设计文件里存的是「SQL 字面量」而不是
        // 「字符串内容」。早先这里把引号剥了，于是 `DEFAULT 'new'` 存成 `new`，
        // 导出时拼出 `DEFAULT new` —— 语法错误；而校验也无从判断用户是「忘了加引号」
        // 还是「本来就该按字符串处理」。
        let v = if after.starts_with('\'') {
            // 找的是**结尾**那个引号：after[1..] 里再找一次，别拿开引号当结尾
            after[1..]
                .find('\'')
                .map(|e| after[..1 + e + 1].to_string())
        } else if after.starts_with('"') {
            after[1..].find('"').map(|e| after[..1 + e + 1].to_string())
        } else {
            after.split_whitespace().next().map(|x| x.to_string())
        };
        f.default = v;
    }
    if let Some(i) = tail_low.find("comment") {
        let after = tail[i + 7..].trim_start();
        let quote = after.chars().next();
        if let Some(q) = quote.filter(|c| *c == '\'' || *c == '"') {
            if let Some(e) = after[1..].find(q) {
                f.comment = after[1..1 + e].to_string();
            }
        }
    }
    Some(f)
}

pub(super) fn s_is_quoted_type(_s: &str) -> bool {
    false
}

/// 列级 `REFERENCES table(col)` → (列, 表, 列)。
/// 按表体切出的单个列定义来找（不再全句扫 references —— 那样会切错括号）。
pub(super) fn inline_column_refs(body: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for item in split_top_level(body, ',') {
        let low = item.to_lowercase();
        // 只看**列定义**：约束行（CONSTRAINT / FOREIGN KEY / UNIQUE …）里的 references
        // 由 parse_inline_table_fks 处理，在这里再扫一遍会把 first_ident 读成 "CONSTRAINT"
        // 然后造出一条 from 字段叫 CONSTRAINT 的假关联（踩过）。
        if low.starts_with("constraint")
            || low.starts_with("foreign")
            || low.starts_with("primary")
            || low.starts_with("unique")
            || low.starts_with("key")
            || low.starts_with("index")
        {
            continue;
        }
        let Some(ri) = low.find("references") else {
            continue;
        };
        let Some(name) = first_ident(&item) else {
            continue;
        };
        let after = &item[ri + "references".len()..];
        let Some(ro) = after.find('(') else { continue };
        let table = normalize_qualified(&after[..ro]);
        let Some(rc) = match_paren(after, ro) else {
            continue;
        };
        let col = unquote_ident(&after[ro + 1..rc]);
        if !name.is_empty() && !table.is_empty() && !col.is_empty() {
            out.push((name, table, col));
        }
    }
    out
}

/// 表体里的 `FOREIGN KEY (a) REFERENCES t(b)` → (子表, 列, 父表, 父列, onDelete, onUpdate)
pub(super) fn parse_inline_table_fks(
    clean: &str,
) -> Vec<(String, Vec<String>, String, Vec<String>, String, String)> {
    let mut out = Vec::new();
    for stmt in split_statements(clean) {
        let lower = stmt.to_lowercase();
        if !lower.contains("create table") {
            continue;
        }
        let Some((table, body)) = parse_create_table(&stmt) else {
            continue;
        };
        for item in split_top_level(&body, ',') {
            let low = item.to_lowercase();
            if !low.contains("foreign key") {
                continue;
            }
            let Some(fi) = low.find("foreign key") else {
                continue;
            };
            // 从 FOREIGN KEY 之后开始按配对括号取列名：直接在整行里找第一个 '(' 会把
            // 「(uid) REFERENCES users (id)」一起吞进列名（踩过：字段名变成 "uid`) REFERENCES ..."）
            let after = &item[fi + "foreign key".len()..];
            let Some(o) = after.find('(') else { continue };
            let Some(c) = match_paren(after, o) else {
                continue;
            };
            let cols: Vec<String> = split_top_level(&after[o + 1..c], ',')
                .iter()
                .map(|s| unquote_ident(s))
                .filter(|s| !s.is_empty())
                .collect();
            let Some(ri) = after.to_lowercase().find("references") else {
                continue;
            };
            let after2 = &after[ri + "references".len()..];
            let Some(ro) = after2.find('(') else { continue };
            let parent = normalize_qualified(&after2[..ro]);
            let Some(rc) = match_paren(after2, ro) else {
                continue;
            };
            let pcols = split_top_level(&after2[ro + 1..rc], ',')
                .iter()
                .map(|s| unquote_ident(s))
                .filter(|s| !s.is_empty())
                .collect();
            let tail_after = after2[rc + 1..].to_lowercase();
            let on_delete = fk_action_from(&tail_after, "on delete");
            let on_update = fk_action_from(&tail_after, "on update");
            if !cols.is_empty() && !parent.is_empty() {
                out.push((table.clone(), cols, parent, pcols, on_delete, on_update));
            }
        }
    }
    out
}

/// `ALTER TABLE t ADD [CONSTRAINT x] FOREIGN KEY (a) REFERENCES p(b) [ON DELETE ...]`
pub(super) fn parse_alter_add_fk(
    stmt: &str,
) -> Option<(String, String, String, String, String, String)> {
    let lower = stmt.to_lowercase();
    if !lower.contains("alter table") || !lower.contains("foreign key") {
        return None;
    }
    let ti = lower.find("alter table")? + "alter table".len();
    let rest = stmt[ti..].trim_start();
    let child = normalize_qualified(rest.split_whitespace().next()?);
    let Some(fi) = lower.find("foreign key") else {
        return None;
    };
    let after = &stmt[fi + "foreign key".len()..];
    let Some(o) = after.find('(') else {
        return None;
    };
    let Some(c) = match_paren(after, o) else {
        return None;
    };
    let cols: Vec<String> = split_top_level(&after[o + 1..c], ',')
        .iter()
        .map(|s| unquote_ident(s))
        .filter(|s| !s.is_empty())
        .collect();
    let tail = &after[c + 1..];
    let Some(ri) = tail.to_lowercase().find("references") else {
        return None;
    };
    let after2 = &tail[ri + "references".len()..];
    let Some(ro) = after2.find('(') else {
        return None;
    };
    let parent = normalize_qualified(&after2[..ro]);
    let Some(rc) = match_paren(after2, ro) else {
        return None;
    };
    let pcol = unquote_ident(&after2[ro + 1..rc]);
    let tail_low = after2.to_lowercase();
    let on_delete = fk_action_from(&tail_low, "on delete");
    let on_update = fk_action_from(&tail_low, "on update");
    if cols.is_empty() || parent.is_empty() || pcol.is_empty() {
        return None;
    }
    Some((child, cols[0].clone(), parent, pcol, on_delete, on_update))
}

/// `CREATE [UNIQUE] INDEX name ON table (cols)` → (名, 表, 列, 是否唯一)
pub(super) fn parse_create_index(stmt: &str) -> Option<(String, String, Vec<String>, bool)> {
    let lower = stmt.to_lowercase();
    let unique = lower.starts_with("create unique index") || lower.contains("create unique index");
    let ii = lower.find("create ")? + "create ".len();
    let mut rest = stmt[ii..].trim_start().to_string();
    if rest.to_lowercase().starts_with("unique") {
        rest = rest["unique".len()..].trim_start().to_string();
    }
    if rest.to_lowercase().starts_with("index") {
        rest = rest["index".len()..].trim_start().to_string();
    }
    // 可能是 `IF NOT EXISTS idx ON ...`
    if rest.to_lowercase().starts_with("if not exists") {
        rest = rest["if not exists".len()..].trim_start().to_string();
    }
    // 名字与表名之间是 " ON "：idx 指向「ON 前面的空格」，所以名字取 idx 之前、
    // 表名从 idx+4 之后开始（写成 idx+4 再去切名字会把 " ON" 吞进索引名）
    let on_idx = rest.to_lowercase().find(" on ")?;
    let name = unquote_ident(rest[..on_idx].trim());
    let after = rest[on_idx + 4..].trim_start();
    let table = unquote_ident(after.split_whitespace().next()?);
    let Some(o) = after.find('(') else {
        return None;
    };
    let Some(c) = after.rfind(')') else {
        return None;
    };
    let cols = split_top_level(&after[o + 1..c], ',')
        .iter()
        .map(|s| clean_index_col(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<String>>();
    Some((name, table, cols, unique))
}

/// `CREATE VIEW name AS SELECT ...` → (名, SQL)
pub(super) fn parse_create_view(stmt: &str) -> Option<(String, String)> {
    let lower = stmt.to_lowercase();
    let vi = lower.find("create ")?;
    let rest = stmt[vi..].trim_start();
    let rest = {
        // CREATE OR REPLACE VIEW / CREATE ALGORITHM=... VIEW（MySQL dump 常见）
        let l = rest.to_lowercase();
        if l.starts_with("or replace") {
            rest["or replace".len()..].trim_start()
        } else {
            rest
        }
    };
    let low2 = rest.to_lowercase();
    let vi = low2.find("view")? + 4;
    let tail = rest[vi..].trim_start();
    let name_part = tail.split_whitespace().next()?;
    let name = unquote_ident(name_part);
    if name.is_empty() {
        return None;
    }
    let body = tail[name_part.len()..].trim_start();
    // `AS` 可能在开头（"AS SELECT ..."），不一定前面带空格，所以两种形态都要认
    let low3 = body.to_lowercase();
    let as_pos = low3
        .find(" as ")
        .map(|i| i + 4)
        .or_else(|| low3.strip_prefix("as ").map(|_| 3))
        .unwrap_or(0);
    Some((
        name,
        body[as_pos..].trim().trim_end_matches(';').to_string(),
    ))
}
