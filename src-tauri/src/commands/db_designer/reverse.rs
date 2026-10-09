// 反向工程：从已有数据库 / 建表语句反推出设计文档（只读，不改库）。
//
// 两条路（用户 2026-10-09 定：不引入 sqlx，零新依赖）：
//   ① SQLite 直连 —— rusqlite 已有，pragma 能拿全字段 / 索引 / 外键；
//   ② DDL 文本解析 —— 解析 mysqldump / pg_dump / 任意 .sql 里的建表语句，
//      对 MySQL / PostgreSQL / SQLite 都成立（不要求连接串）。
//
// 产出的是标准设计文档，用户打开后可以继续在画布上改。
use rusqlite::Connection;

use super::models::*;
use super::store::validate;

/// 生成节点 / 关联 id。
///
/// ⚠ 必须带进程内自增序号：只用毫秒时间戳的话，**同一毫秒里建的两张表会拿到同一个 id**
/// （踩过：CREATE INDEX 按 id 找节点，两张表 id 相同 → 索引挂到了第一张表上）。
fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}_{}_{}", prefix, chrono::Utc::now().timestamp_millis(), seq)
}

// ─────────────────────────── 类型反推 ───────────────────────────

/// MySQL 声明类型 → 逻辑类型
pub fn mysql_type_to_logical(raw: &str) -> DbLogicalType {
    let s = raw.trim().to_lowercase();
    let s = s.trim_start_matches("unsigned ").trim();
    let (base, args) = split_type_and_args(s);
    let unsigned = raw.to_lowercase().contains("unsigned");
    let base = match base {
        "tinyint" => "tinyint",
        "smallint" | "mediumint" => "smallint",
        "int" | "integer" => "int",
        "bigint" => "bigint",
        "bit" | "bool" | "boolean" => "boolean",
        "decimal" | "numeric" => "decimal",
        "float" => "float",
        "double" | "real" | "double precision" => "double",
        "char" => "char",
        "varchar" => "varchar",
        "tinytext" | "text" | "mediumtext" | "longtext" => "text",
        "json" => "json",
        "uuid" | "binary(16)" => "uuid",
        "blob" | "tinyblob" | "mediumblob" | "longblob" => "blob",
        "date" => "date",
        "time" => "time",
        "datetime" | "timestamp" => "timestamp",
        "enum" => "enum",
        _ => "text",
    };
    let mut t = DbLogicalType { base: base.to_string(), unsigned, ..Default::default() };
    if base == "enum" {
        t.values = args
            .iter()
            .map(|a| a.trim().trim_matches('\'').trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
            .collect();
    } else if base == "decimal" {
        t.precision = args.first().and_then(|a| a.trim().parse().ok());
        t.scale = args.get(1).and_then(|a| a.trim().parse().ok());
    } else if matches!(base, "varchar" | "char") {
        t.length = args.first().and_then(|a| a.trim().parse().ok());
    }
    t
}

/// SQLite 声明类型 → 逻辑类型（SQLite 只有 5 类存储类型，声明类型只是约定）
pub fn sqlite_type_to_logical(raw: &str) -> DbLogicalType {
    let s = raw.trim().to_lowercase();
    let (base, args) = split_type_and_args(&s);
    let base = match base {
        "integer" | "int" | "bigint" | "smallint" | "tinyint" => "bigint",
        "real" | "double" | "float" => "double",
        "numeric" | "decimal" => "decimal",
        "blob" => "blob",
        "boolean" | "bool" => "boolean",
        "date" => "date",
        "datetime" | "timestamp" => "timestamp",
        "time" => "time",
        "uuid" => "uuid",
        "json" => "json",
        // 声明里带了长度/精度的按原样尊重（VARCHAR(20) 这类写法很常见）
        "varchar" | "char" => "varchar",
        _ => "text",
    };
    let mut t = DbLogicalType { base: base.to_string(), ..Default::default() };
    if matches!(base, "varchar" | "char") {
        t.length = args.first().and_then(|a| a.trim().parse().ok()).or(Some(255));
    } else if base == "decimal" {
        t.precision = args.first().and_then(|a| a.trim().parse().ok()).or(Some(10));
        t.scale = args.get(1).and_then(|a| a.trim().parse().ok()).or(Some(2));
    }
    t
}

/// `varchar(32) unsigned` → ("varchar", ["32"])，`decimal(12,2)` → ("decimal", ["12","2"])，`enum('a','b')` 同理
fn split_type_and_args(s: &str) -> (&str, Vec<String>) {
    match s.find('(') {
        Some(i) if s.ends_with(')') => {
            let base = s[..i].trim();
            let inner = &s[i + 1..s.len() - 1];
            let args = split_top_level(inner, ',')
                .into_iter()
                .map(|a| a.trim().to_string())
                .collect();
            (base, args)
        }
        _ => (s.trim(), Vec::new()),
    }
}

/// 按顶层分隔符切字符串（忽略括号内与引号内的分隔符）
fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in s.chars() {
        match quote {
            Some(q) => {
                cur.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' | '`' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                '(' => {
                    depth += 1;
                    cur.push(ch);
                }
                ')' => {
                    depth -= 1;
                    cur.push(ch);
                }
                c if c == sep && depth == 0 => {
                    out.push(cur.trim().to_string());
                    cur = String::new();
                }
                _ => cur.push(ch),
            },
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// 去掉注释与语句间空白
fn strip_comments(sql: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // -- 行注释 / # 行注释（MySQL）
        if chars[i] == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if chars[i] == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // /* */ 块注释
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            out.push(' ');
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 按分号切语句（忽略引号内的分号）
fn split_statements(sql: &str) -> Vec<String> {
    split_top_level(sql, ';')
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect()
}

/// 去掉标识符包裹符
fn unquote_ident(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('`').and_then(|x| x.strip_suffix('`'))
        .or_else(|| t.strip_prefix('"').and_then(|x| x.strip_suffix('"')))
        .or_else(|| t.strip_prefix('[').and_then(|x| x.strip_suffix(']')))
        .unwrap_or(t);
    t.to_string()
}

/// 从 `open_byte`（必须是 `(` 的字节位置）找配对的 `)`，返回其字节位置。
/// 括号必须配对才能取参数：`decimal(12,2)`、`FOREIGN KEY (a) REFERENCES t(b)` 都靠它。
fn match_paren(s: &str, open_byte: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut i = open_byte;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                b'\'' | b'"' | b'`' => quote = Some(c),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// 取一段文本里的第一个标识符（列名）：支持 `x` / "x" / [x] / x
fn first_ident(s: &str) -> Option<String> {
    let t = s.trim_start();
    for (open, close) in [('`', '`'), ('"', '"'), ('[', ']')] {
        if let Some(rest) = t.strip_prefix(open) {
            if let Some(end) = rest.find(close) {
                return Some(rest[..end].to_string());
            }
        }
    }
    let end = t.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(t.len());
    if end == 0 {
        None
    } else {
        Some(t[..end].to_string())
    }
}

/// 表名归一：`"public"."team"` / `public.team` / `` `team` `` → `team`
/// （pg_dump 会带 schema 前缀，设计文档里只留表名）
fn normalize_qualified(raw: &str) -> String {
    let s = raw.trim();
    let last = s.rsplit('.').next().unwrap_or(s);
    unquote_ident(last)
}

/// 取 `ON DELETE <动作>` / `ON UPDATE <动作>` 的动作部分。
/// `SET NULL` / `SET DEFAULT` 是两个词，只取一个词会得到没意义的 "SET"。
fn fk_action_from(tail_low: &str, keyword: &str) -> String {
    let Some(i) = tail_low.find(keyword) else {
        return String::new();
    };
    let rest = tail_low[i + keyword.len()..].trim_start();
    let mut words = rest.split_whitespace();
    let Some(first) = words.next() else {
        return String::new();
    };
    let mut action = first.trim_end_matches(';').trim_end_matches(',').to_uppercase();
    if action == "SET" {
        if let Some(second) = words.next() {
            action = format!("SET {}", second.trim_end_matches(';').to_uppercase());
        }
    }
    action
}

// ─────────────────────────── DDL 解析 ───────────────────────────

/// 从建表语句文本反推设计文档（mysqldump / pg_dump / 任意 .sql）
pub fn reverse_ddl(text: &str, dialect: &str, name: &str) -> Result<DbDesignDocument, String> {
    let clean = strip_comments(text);
    let mut doc = DbDesignDocument {
        id: new_id("dbd"),
        name: name.to_string(),
        description: format!("由 DDL 反推（{}）", dialect),
        dialect: if DIALECTS.contains(&dialect) { dialect.to_string() } else { "mysql".to_string() },
        folder_id: None,
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
                pending_fks.push((table_name.clone(), from_field, to_table, to_field, String::new(), String::new()));
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
            let Some(v) = parse_create_view(&stmt) else { continue };
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
                        kind: if unique { "unique".to_string() } else { "index".to_string() },
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
            let pc = pcols.get(i).cloned().unwrap_or_else(|| pcols.first().cloned().unwrap_or_default());
            pending_fks.push((child.clone(), c.clone(), parent.clone(), pc, on_delete.clone(), on_update.clone()));
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
            from: DbRelationEnd { node: child_id, field: from_field },
            to: DbRelationEnd { node: parent_id, field: to_field },
            kind: "1-n".to_string(),
            on_delete,
            on_update,
            mirror: false,
        });
    }

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
fn parse_create_table(stmt: &str) -> Option<(String, String)> {
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
fn parse_table_body(body: &str, _lower_hint: &str) -> (Vec<DbField>, Vec<DbIndex>) {
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
                    kind: if unique { "unique".to_string() } else { "index".to_string() },
                    fields: cols,
                });
            }
            continue;
        }
        // 约束行
        if low.starts_with("primary key") {
            if let (Some(o), Some(c)) = (item.find('('), item.rfind(')')) {
                pk_cols = split_top_level(&item[o + 1..c], ',').iter().map(|s| unquote_ident(s)).collect();
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
                    kind: if unique { "unique".to_string() } else { "index".to_string() },
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
fn clean_index_col(raw: &str) -> String {
    let s = raw.trim();
    // `col` DESC / `col` ASC —— 按空白切开，尾部那个排序关键字丢掉
    let s = match s.rsplit_once(char::is_whitespace) {
        Some((head, tail)) if tail.eq_ignore_ascii_case("asc") || tail.eq_ignore_ascii_case("desc") => head,
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
fn parse_index_clause(item: &str) -> (String, Vec<String>, bool) {
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
fn parse_column(item: &str) -> Option<DbField> {
    let s = item.trim();
    if s.is_empty() {
        return None;
    }
    // 名字：第一个标识符（引号包裹或裸词）
    let (name, rest) = if let Some(stripped) = s.strip_prefix('`') {
        let end = stripped.find('`')?;
        (stripped[..end].to_string(), stripped[end + 1..].trim_start())
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
                Some(close) => (format!("{}{}", head, &rest[i..=close]), rest[close + 1..].trim_start().to_string()),
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
        let v = if after.starts_with('\'') {
            // 找的是**结尾**那个引号：after[1..] 里再找一次，别拿开引号当结尾
            after[1..].find('\'').map(|e| after[1..1 + e].to_string())
        } else {
            after
                .split_whitespace()
                .next()
                .map(|x| x.to_string())
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

fn s_is_quoted_type(_s: &str) -> bool {
    false
}

/// 列级 `REFERENCES table(col)` → (列, 表, 列)。
/// 按表体切出的单个列定义来找（不再全句扫 references —— 那样会切错括号）。
fn inline_column_refs(body: &str) -> Vec<(String, String, String)> {
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
        let Some(ri) = low.find("references") else { continue };
        let Some(name) = first_ident(&item) else { continue };
        let after = &item[ri + "references".len()..];
        let Some(ro) = after.find('(') else { continue };
        let table = normalize_qualified(&after[..ro]);
        let Some(rc) = match_paren(after, ro) else { continue };
        let col = unquote_ident(&after[ro + 1..rc]);
        if !name.is_empty() && !table.is_empty() && !col.is_empty() {
            out.push((name, table, col));
        }
    }
    out
}

/// 表体里的 `FOREIGN KEY (a) REFERENCES t(b)` → (子表, 列, 父表, 父列, onDelete, onUpdate)
fn parse_inline_table_fks(clean: &str) -> Vec<(String, Vec<String>, String, Vec<String>, String, String)> {
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
            let Some(fi) = low.find("foreign key") else { continue };
            // 从 FOREIGN KEY 之后开始按配对括号取列名：直接在整行里找第一个 '(' 会把
            // 「(uid) REFERENCES users (id)」一起吞进列名（踩过：字段名变成 "uid`) REFERENCES ..."）
            let after = &item[fi + "foreign key".len()..];
            let Some(o) = after.find('(') else { continue };
            let Some(c) = match_paren(after, o) else { continue };
            let cols: Vec<String> = split_top_level(&after[o + 1..c], ',')
                .iter()
                .map(|s| unquote_ident(s))
                .filter(|s| !s.is_empty())
                .collect();
            let Some(ri) = after.to_lowercase().find("references") else { continue };
            let after2 = &after[ri + "references".len()..];
            let Some(ro) = after2.find('(') else { continue };
            let parent = normalize_qualified(&after2[..ro]);
            let Some(rc) = match_paren(after2, ro) else { continue };
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
fn parse_alter_add_fk(stmt: &str) -> Option<(String, String, String, String, String, String)> {
    let lower = stmt.to_lowercase();
    if !lower.contains("alter table") || !lower.contains("foreign key") {
        return None;
    }
    let ti = lower.find("alter table")? + "alter table".len();
    let rest = stmt[ti..].trim_start();
    let child = normalize_qualified(rest.split_whitespace().next()?);
    let Some(fi) = lower.find("foreign key") else { return None };
    let after = &stmt[fi + "foreign key".len()..];
    let Some(o) = after.find('(') else { return None };
    let Some(c) = match_paren(after, o) else { return None };
    let cols: Vec<String> = split_top_level(&after[o + 1..c], ',')
        .iter()
        .map(|s| unquote_ident(s))
        .filter(|s| !s.is_empty())
        .collect();
    let tail = &after[c + 1..];
    let Some(ri) = tail.to_lowercase().find("references") else { return None };
    let after2 = &tail[ri + "references".len()..];
    let Some(ro) = after2.find('(') else { return None };
    let parent = normalize_qualified(&after2[..ro]);
    let Some(rc) = match_paren(after2, ro) else { return None };
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
fn parse_create_index(stmt: &str) -> Option<(String, String, Vec<String>, bool)> {
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
    let Some(o) = after.find('(') else { return None };
    let Some(c) = after.rfind(')') else { return None };
    let cols = split_top_level(&after[o + 1..c], ',')
        .iter()
        .map(|s| clean_index_col(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<String>>();
    Some((name, table, cols, unique))
}

/// `CREATE VIEW name AS SELECT ...` → (名, SQL)
fn parse_create_view(stmt: &str) -> Option<(String, String)> {
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
    Some((name, body[as_pos..].trim().trim_end_matches(';').to_string()))
}

// ─────────────────────────── SQLite 直连 ───────────────────────────

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
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })
        .map_err(|e| format!("遍历表失败: {}", e))?;
    let objects: Vec<(String, String, String)> = rows
        .filter_map(|r| r.ok())
        .collect();

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
            .prepare(&format!("PRAGMA table_info(\"{}\")", obj_name.replace('"', "\"\"")))
            .map_err(|e| format!("PRAGMA table_info 失败: {}", e))?;
        let cols = ti
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,  // cid
                    r.get::<_, String>(1)?, // name
                    r.get::<_, String>(2)?, // type
                    r.get::<_, i64>(3)?,  // notnull
                    r.get::<_, Option<String>>(4)?, // dflt
                    r.get::<_, i64>(5)?,  // pk
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
        if let Ok(mut il) = conn.prepare(&format!("PRAGMA index_list(\"{}\")", obj_name.replace('"', "\"\""))) {
            let lists = il
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(1)?,  // name
                        r.get::<_, i64>(2)?,     // unique
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
                    .prepare(&format!("PRAGMA index_info(\"{}\")", iname.replace('"', "\"\"")))
                    .ok()
                    .and_then(|mut s| {
                        s.query_map([], |r| Ok(r.get::<_, Option<String>>(2)?))
                            .ok()
                            .map(|rows| rows.filter_map(|r| r.ok().flatten()).collect::<Vec<String>>())
                    })
                    .unwrap_or_default();
                if cols.is_empty() {
                    continue;
                }
                indexes.push(DbIndex {
                    name: iname,
                    kind: if uniq == 1 { "unique".to_string() } else { "index".to_string() },
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
        let Ok(mut fl) = conn.prepare(&format!("PRAGMA foreign_key_list(\"{}\")", obj_name.replace('"', "\"\""))) else {
            continue;
        };
        let Ok(rows) = fl.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,  // id
                r.get::<_, i64>(1)?,  // seq
                r.get::<_, String>(2)?, // table
                r.get::<_, String>(3)?, // from
                r.get::<_, Option<String>>(4)?, // to
                r.get::<_, String>(5)?, // on_update
                r.get::<_, String>(6)?, // on_delete
            ))
        }) else {
            continue;
        };
        let mut grouped: std::collections::HashMap<i64, (String, Vec<(i64, String, String)>, String, String)> =
            std::collections::HashMap::new();
        for f in rows.filter_map(|r| r.ok()) {
            let entry = grouped
                .entry(f.0)
                .or_insert_with(|| (f.2.clone(), Vec::new(), f.5.clone(), f.6.clone()));
            entry.1.push((f.1, f.3.clone(), f.4.clone().unwrap_or_else(|| f.3.clone())));
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
                    from: DbRelationEnd { node: child_id.clone(), field: from_field },
                    to: DbRelationEnd { node: parent_id.clone(), field: to_field },
                    kind: "1-n".to_string(),
                    on_delete: on_delete.clone(),
                    on_update: on_update.clone(),
                    mirror: false,
                });
            }
        }
    }

    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MYSQL_DUMP: &str = r#"
    -- 订单库导出
    CREATE TABLE `users` (
      `id` BIGINT NOT NULL AUTO_INCREMENT COMMENT '主键',
      `name` VARCHAR(64) NOT NULL COMMENT '昵称',
      `status` enum('new','paid') NOT NULL DEFAULT 'new',
      PRIMARY KEY (`id`),
      UNIQUE KEY `uk_users_name` (`name`)
    ) ENGINE=InnoDB;

    CREATE TABLE `orders` (
      `id` BIGINT NOT NULL AUTO_INCREMENT,
      `no` varchar(32) NOT NULL,
      `uid` BIGINT NOT NULL,
      `amount` DECIMAL(12,2) NOT NULL DEFAULT 0,
      KEY `idx_orders_uid` (`uid`),
      CONSTRAINT `fk_orders_uid` FOREIGN KEY (`uid`) REFERENCES `users` (`id`) ON DELETE CASCADE ON UPDATE CASCADE
    );

    CREATE UNIQUE INDEX `uk_orders_no` ON `orders` (`no`);

    CREATE VIEW `v_paid` AS SELECT * FROM orders WHERE amount > 0;
    "#;

    #[test]
    fn mysql_ddl_becomes_tables_indexes_and_relations() {
        let doc = reverse_ddl(MYSQL_DUMP, "mysql", "订单库").unwrap();
        assert_eq!(doc.nodes.len(), 3, "2 表 + 1 视图: {:?}", doc.nodes);
        let users = doc.nodes.iter().find(|n| n.name == "users").unwrap();
        let fields = users.table.as_ref().unwrap().fields.clone();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].r#type.base, "bigint");
        assert!(fields[0].pk && fields[0].auto_increment && !fields[0].nullable);
        assert_eq!(fields[0].comment, "主键");
        assert_eq!(fields[1].r#type.length, Some(64));
        assert_eq!(fields[2].r#type.base, "enum");
        assert_eq!(fields[2].r#type.values, vec!["new".to_string(), "paid".to_string()]);
        // UNIQUE KEY 进索引表
        let idx = users.table.as_ref().unwrap().indexes.clone();
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].kind, "unique");
        assert_eq!(idx[0].fields, vec!["name".to_string()]);

        let orders = doc.nodes.iter().find(|n| n.name == "orders").unwrap();
        let of = &orders.table.as_ref().unwrap().fields;
        assert_eq!(of[3].r#type.base, "decimal");
        assert_eq!(of[3].r#type.precision, Some(12));
        assert_eq!(of[3].default.as_deref(), Some("0"));
        // CREATE UNIQUE INDEX 也要收进来
        assert!(orders
            .table
            .as_ref()
            .unwrap()
            .indexes
            .iter()
            .any(|i| i.name == "uk_orders_no" && i.kind == "unique"));

        // 外键：从子表指向父表
        assert_eq!(doc.relations.len(), 1);
        let r = &doc.relations[0];
        assert_eq!(doc.node(&r.from.node).unwrap().name, "orders");
        assert_eq!(r.from.field, "uid");
        assert_eq!(doc.node(&r.to.node).unwrap().name, "users");
        assert_eq!(r.to.field, "id");
        assert_eq!(r.on_delete, "CASCADE");

        // 视图
        let v = doc.nodes.iter().find(|n| n.kind == "view").unwrap();
        assert_eq!(v.name, "v_paid");
        assert!(v.view.as_ref().unwrap().sql.contains("SELECT"));
    }

    #[test]
    fn reverse_result_always_passes_validation() {
        let doc = reverse_ddl(MYSQL_DUMP, "mysql", "x").unwrap();
        let report = validate(&doc);
        assert!(report.is_ok(), "{:?}", report.errors);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn postgres_quoted_identifiers_and_alter_table_fk() {
        let ddl = r#"
        CREATE TABLE "public"."team" (
            "id" bigserial PRIMARY KEY,
            "title" varchar(80) NOT NULL
        );
        CREATE TABLE "member" (
            "id" bigint NOT NULL,
            "team_id" bigint NOT NULL
        );
        ALTER TABLE "member" ADD CONSTRAINT "fk_member_team" FOREIGN KEY ("team_id") REFERENCES "public"."team"("id") ON DELETE SET NULL;
        "#;
        let doc = reverse_ddl(ddl, "postgres", "团队").unwrap();
        let names: Vec<String> = doc.nodes.iter().map(|n| n.name.clone()).collect();
        assert!(names.contains(&"team".to_string()), "{:?}", names);
        let team = doc.nodes.iter().find(|n| n.name == "team").unwrap();
        assert!(team.table.as_ref().unwrap().fields[0].auto_increment);
        assert_eq!(doc.relations.len(), 1, "{:?}", doc.relations);
        assert_eq!(doc.relations[0].on_delete, "SET NULL");
    }

    #[test]
    fn navicat_style_index_columns_with_asc_and_using_btree_are_parsed() {
        // Navicat 导出的索引列带排序方向，末尾还跟 USING BTREE：
        //   UNIQUE KEY `idx_51`(`openid` ASC, `user_status` ASC) USING BTREE
        // 踩过的坑：只脱反引号会留下「`openid` ASC」这种列名，校验判为引用不存在字段，
        // 整个逆向 DDL 直接失败（真实 dump 里 226 处一起报）。
        let ddl = r#"
        CREATE TABLE `base_user` (
          `openid` varchar(32) NOT NULL,
          `user_status` char(1) NULL DEFAULT '0',
          PRIMARY KEY (`openid`) USING BTREE,
          UNIQUE KEY `idx_51`(`openid` ASC, `user_status` ASC) USING BTREE
        ) ENGINE = InnoDB CHARACTER SET = utf8mb4;
        CREATE INDEX `idx_52` ON `base_user` (`user_status` DESC);
        "#;
        let doc = reverse_ddl(ddl, "mysql", "t").unwrap();
        let t = doc.nodes[0].table.as_ref().unwrap();
        let idx = t.indexes.iter().find(|i| i.name == "idx_51").expect("idx_51");
        assert_eq!(idx.fields, vec!["openid".to_string(), "user_status".to_string()]);
        assert_eq!(idx.kind, "unique");
        let idx2 = t.indexes.iter().find(|i| i.name == "idx_52").expect("idx_52");
        assert_eq!(idx2.fields, vec!["user_status".to_string()]);
    }

    #[test]
    fn comments_are_stripped_and_quotes_do_not_split_statements() {
        let ddl = r#"
        -- CREATE TABLE fake (x int);
        /* CREATE TABLE fake2 (x int); */
        CREATE TABLE t (
            a VARCHAR(10) DEFAULT 'semi;colon',
            b INT
        );
        "#;
        let doc = reverse_ddl(ddl, "mysql", "t").unwrap();
        assert_eq!(doc.nodes.len(), 1, "注释里的建表不该被解析: {:?}", doc.nodes.len());
        assert_eq!(doc.nodes[0].table.as_ref().unwrap().fields.len(), 2);
        assert_eq!(
            doc.nodes[0].table.as_ref().unwrap().fields[0].default.as_deref(),
            Some("semi;colon")
        );
    }

    #[test]
    fn sqlite_db_is_reversed_with_fields_indexes_and_foreign_keys() {
        let dir = std::env::temp_dir().join(format!("kira-rev-sqlite-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("demo.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE users (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     name VARCHAR(64) NOT NULL,
                     created_at DATETIME
                 );
                 CREATE UNIQUE INDEX uk_users_name ON users(name);
                 CREATE TABLE orders (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     uid INTEGER NOT NULL REFERENCES users(id),
                     amount NUMERIC(12,2)
                 );
                 CREATE VIEW v_orders AS SELECT * FROM orders;",
            )
            .unwrap();
        }

        let doc = reverse_sqlite(&db.to_string_lossy(), "演示库").unwrap();
        assert_eq!(doc.dialect, "sqlite");
        let names: Vec<String> = doc.nodes.iter().map(|n| n.name.clone()).collect();
        assert!(names.contains(&"users".to_string()));
        assert!(names.contains(&"orders".to_string()));
        assert!(names.contains(&"v_orders".to_string()));

        let users = doc.nodes.iter().find(|n| n.name == "users").unwrap();
        let f = users.table.as_ref().unwrap().fields.clone();
        assert!(f[0].auto_increment && f[0].pk);
        assert_eq!(f[1].r#type.length, Some(64));
        assert!(users
            .table
            .as_ref()
            .unwrap()
            .indexes
            .iter()
            .any(|i| i.name == "uk_users_name" && i.kind == "unique"));

        // 内联 REFERENCES 与外键 pragma 都应还原成关联
        assert_eq!(doc.relations.len(), 1, "{:?}", doc.relations);
        assert_eq!(doc.relations[0].from.field, "uid");

        let report = validate(&doc);
        assert!(report.is_ok(), "{:?}", report.errors);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sqlite_reverse_rejects_non_sqlite_files() {
        let dir = std::env::temp_dir().join(format!("kira-rev-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("not.db");
        std::fs::write(&bad, b"this is not a sqlite file").unwrap();
        assert!(reverse_sqlite(&bad.to_string_lossy(), "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}