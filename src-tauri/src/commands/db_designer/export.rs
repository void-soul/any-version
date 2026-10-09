// SQL 导出：设计文档 → 建表语句（MySQL / PostgreSQL / SQLite 三种）。
//
// 三种方言的**唯一**出口在这里（单一真源）：类型映射、标识符引号、注释写法、
// 外键与中间表规则都在本文件，别的模块不要各写一份。
//
// 刻意不做的事：不做迁移版本管理、不连库执行。这里只负责**生成文本**。
use super::models::*;
use super::store::{junction_table_name, validate};

/// 导出整份文档的建表语句（三种关系型方言）。
pub fn export_sql(doc: &DbDesignDocument) -> Result<String, String> {
    let report = validate(doc);
    if !report.is_ok() {
        return Err(format!("设计文档校验未通过：\n- {}", report.errors.join("\n- ")));
    }
    if report
        .warnings
        .iter()
        .any(|w| w.contains("不在白名单") || w.contains("不存在的字段"))
    {
        // 校验里这几类已经进 errors，这里只是防御：万一以后挪了级别也别导出坏 SQL
        return Err("设计文档存在悬空引用，拒绝导出".to_string());
    }

    match doc.dialect.as_str() {
        "mysql" => Ok(export_relational(doc, Dialect::MySql)),
        "postgres" => Ok(export_relational(doc, Dialect::Postgres)),
        "sqlite" => Ok(export_relational(doc, Dialect::Sqlite)),
        other => Err(format!("未知方言 {}（支持：{}）", other, DIALECTS.join(" / "))),
    }
}

/// 结构说明（Markdown）：给人快速浏览设计用，也是「不打开应用也能看懂这份设计」的兜底产物。
pub fn export_outline(doc: &DbDesignDocument) -> String {
    let mut out = format!("# {}\n", doc.name);
    if !doc.description.is_empty() {
        out.push_str(&format!("\n{}\n", doc.description));
    }
    out.push_str(&format!("\n方言：`{}` · 节点 {} 个\n", doc.dialect, doc.nodes.len()));
    for n in &doc.nodes {
        out.push_str(&format!("\n## {} · {}\n", n.name, n.kind));
        if !n.comment.is_empty() {
            out.push_str(&format!("{}\n", n.comment));
        }
        if let Some(t) = &n.table {
            for f in &t.fields {
                let flags = [
                    if f.pk { "PK" } else { "" },
                    if f.auto_increment { "AI" } else { "" },
                    if f.unique { "UQ" } else { "" },
                    if f.nullable { "NULL" } else { "NOT NULL" },
                ]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
                out.push_str(&format!(
                    "- `{}` {} {}{}\n",
                    f.name,
                    render_logical_type(f.r#type.clone()),
                    flags,
                    if f.comment.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", f.comment)
                    }
                ));
            }
        }
        if let Some(v) = &n.view {
            if !v.sql.trim().is_empty() {
                out.push_str(&format!("\n```sql\n{}\n```\n", v.sql.trim()));
            }
        }
    }
    if !doc.relations.is_empty() {
        out.push_str("\n## 关联\n");
        for r in &doc.relations {
            let a = doc.node(&r.from.node).map(|n| n.name.as_str()).unwrap_or("?");
            let b = doc.node(&r.to.node).map(|n| n.name.as_str()).unwrap_or("?");
            out.push_str(&format!(
                "- {}：{}.{} → {}.{}（{}）\n",
                if r.name.is_empty() { r.kind.as_str() } else { r.name.as_str() },
                a, r.from.field, b, r.to.field, r.kind
            ));
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Dialect {
    MySql,
    Postgres,
    Sqlite,
}

fn quote(d: Dialect, ident: &str) -> String {
    match d {
        Dialect::MySql => format!("`{}`", ident.replace('`', "``")),
        _ => format!("\"{}\"", ident.replace('"', "\"\"")),
    }
}

fn escape_lit(s: &str) -> String {
    s.replace('\'', "''")
}

/// 逻辑类型 → 方言具体类型
fn map_type(d: Dialect, t: &DbLogicalType) -> String {
    let base = t.base.as_str();
    match d {
        Dialect::MySql => match base {
            "int" | "bigint" | "smallint" | "tinyint" => {
                let name = match base {
                    "int" => "INT",
                    "bigint" => "BIGINT",
                    "smallint" => "SMALLINT",
                    _ => "TINYINT",
                };
                if t.unsigned {
                    format!("{} UNSIGNED", name)
                } else {
                    name.to_string()
                }
            }
            "decimal" => format!(
                "DECIMAL({},{})",
                t.precision.unwrap_or(10),
                t.scale.unwrap_or(2)
            ),
            "float" => "FLOAT".to_string(),
            "double" => "DOUBLE".to_string(),
            "char" => format!("CHAR({})", t.length.unwrap_or(1)),
            "varchar" => format!("VARCHAR({})", t.length.unwrap_or(255)),
            "text" => "TEXT".to_string(),
            "date" => "DATE".to_string(),
            "time" => "TIME".to_string(),
            "datetime" => "DATETIME".to_string(),
            "timestamp" => "TIMESTAMP".to_string(),
            "boolean" => "TINYINT(1)".to_string(),
            "json" => "JSON".to_string(),
            "uuid" => "CHAR(36)".to_string(),
            "blob" => "BLOB".to_string(),
            "enum" => format!(
                "ENUM({})",
                t.values
                    .iter()
                    .map(|v| format!("'{}'", escape_lit(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => other.to_uppercase(),
        },
        Dialect::Postgres => match base {
            "int" => "INTEGER".to_string(),
            "bigint" => "BIGINT".to_string(),
            "smallint" => "SMALLINT".to_string(),
            "tinyint" => "SMALLINT".to_string(),
            "decimal" => format!("NUMERIC({},{})", t.precision.unwrap_or(10), t.scale.unwrap_or(2)),
            "float" => "REAL".to_string(),
            "double" => "DOUBLE PRECISION".to_string(),
            "char" => format!("CHAR({})", t.length.unwrap_or(1)),
            "varchar" => format!("VARCHAR({})", t.length.unwrap_or(255)),
            "text" => "TEXT".to_string(),
            "date" => "DATE".to_string(),
            "time" => "TIME".to_string(),
            "datetime" | "timestamp" => "TIMESTAMP".to_string(),
            "boolean" => "BOOLEAN".to_string(),
            "json" => "JSONB".to_string(),
            "uuid" => "UUID".to_string(),
            "blob" => "BYTEA".to_string(),
            // PG 无原生 enum 字面量列类型（自定义类型要先建），退化成 TEXT + CHECK
            "enum" => "TEXT".to_string(),
            other => other.to_uppercase(),
        },
        Dialect::Sqlite => match base {
            // SQLite 只有 INTEGER/REAL/TEXT/BLOB/NUMERIC 五类存储类型
            "int" | "bigint" | "smallint" | "tinyint" | "boolean" => "INTEGER".to_string(),
            "decimal" => "NUMERIC".to_string(),
            "float" | "double" => "REAL".to_string(),
            "char" | "varchar" | "text" | "date" | "time" | "datetime" | "timestamp" => {
                "TEXT".to_string()
            }
            "json" | "uuid" | "enum" | "blob" => "TEXT".to_string(),
            other => other.to_uppercase(),
        },
    }
}

fn render_logical_type(t: DbLogicalType) -> String {
    match t.base.as_str() {
        "varchar" | "char" => format!("{}({})", t.base, t.length.unwrap_or(255)),
        "decimal" => format!("decimal({},{})", t.precision.unwrap_or(10), t.scale.unwrap_or(2)),
        "enum" => format!("enum({})", t.values.join("|")),
        other => other.to_string(),
    }
}

/// PG 的自增列要把类型换成 SERIAL / BIGSERIAL（写在类型位，不是约束位）
fn column_type_for(d: Dialect, f: &DbField) -> String {
    if d == Dialect::Postgres && f.auto_increment {
        return match f.r#type.base.as_str() {
            "bigint" => "BIGSERIAL".to_string(),
            "smallint" => "SMALLSERIAL".to_string(),
            _ => "SERIAL".to_string(),
        };
    }
    map_type(d, &f.r#type)
}

fn export_relational(doc: &DbDesignDocument, d: Dialect) -> String {
    let mut out = String::new();
    out.push_str(&format!("-- {}\n", doc.name));
    if !doc.description.is_empty() {
        out.push_str(&format!("-- {}\n", doc.description));
    }
    out.push_str(&format!(
        "-- 由 Kira 数据库设计器生成 · 方言 {}\n\n",
        doc.dialect
    ));

    let tables: Vec<&DbDesignNode> = doc.nodes.iter().filter(|n| n.kind == "table").collect();

    for t in &tables {
        let body = t.table.as_ref().unwrap();
        out.push_str(&format!("CREATE TABLE {} (\n", quote(d, &t.name)));
        let mut lines: Vec<String> = Vec::new();

        for f in &body.fields {
            let mut line = format!("  {} {}", quote(d, &f.name), column_type_for(d, f));
            let rest = column_def_tail(d, f);
            line.push_str(&rest);
            lines.push(line);
        }

        let pks: Vec<&DbField> = body.fields.iter().filter(|f| f.pk).collect();
        // SQLite 单列自增主键已经内联，跳过独立的 PRIMARY KEY 行
        let pk_inline =
            d == Dialect::Sqlite && pks.len() == 1 && pks[0].auto_increment;
        if !pks.is_empty() && !pk_inline {
            lines.push(format!(
                "  PRIMARY KEY ({})",
                pks.iter()
                    .map(|f| quote(d, &f.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // unique 字段 → 独立唯一索引（不写成行内 UNIQUE，避免与索引表重复）
        for f in body.fields.iter().filter(|f| f.unique) {
            lines.push(format!(
                "  CONSTRAINT {} UNIQUE ({})",
                quote(d, &format!("uk_{}_{}", t.name, f.name)),
                quote(d, &f.name)
            ));
        }

        // 索引（primary 类型由主键行承担，这里跳过）
        for idx in body.indexes.iter().filter(|i| i.kind != "primary") {
            let kw = match (d, idx.kind.as_str()) {
                (Dialect::MySql, "fulltext") => "FULLTEXT KEY".to_string(),
                (Dialect::MySql, "unique") => "UNIQUE KEY".to_string(),
                (Dialect::MySql, _) => "KEY".to_string(),
                (_, "unique") => "UNIQUE".to_string(),
                (_, _) => "INDEX".to_string(),
            };
            let cols = idx
                .fields
                .iter()
                .map(|f| quote(d, f))
                .collect::<Vec<_>>()
                .join(", ");
            if d == Dialect::Postgres || d == Dialect::Sqlite {
                // PG / SQLite 的索引必须在建表语句之外
                out.push_str(&format!("\nCREATE {} {} ON {} ({});", kw_for_create(d, &idx.kind), quote(d, &idx.name), quote(d, &t.name), cols));
                continue;
            }
            lines.push(format!("  {} {} ({})", kw, quote(d, &idx.name), cols));
        }

        // 外键：MySQL / SQLite 写在建表语句内，PG 用 ALTER（便于命名与后续增删）。
        // 方向约定：**外键建在 from 端**（from 是「多 / 子」端，引用 to 这个「一 / 父」端）。
        // 只挂在 from 上，否则父子表会各建一条外键（曾经两边都建，users 也被挂上 FK）。
        for rel in doc
            .relations
            .iter()
            .filter(|r| r.kind != "n-n" && r.from.node == t.id)
        {
            let Some(target) = rel_to_table(doc, rel, &t.id) else {
                continue;
            };
            let fk = fk_clause(d, t, rel, target);
            if d == Dialect::Postgres {
                continue;
            }
            lines.push(format!("  {}", fk));
        }

        out.push_str(&lines.join(",\n"));
        out.push_str("\n)");
        if d == Dialect::MySql && !t.comment.is_empty() {
            out.push_str(&format!(" COMMENT='{}'", escape_lit(&t.comment)));
        }
        out.push_str(";\n\n");

        // PG：注释与外键放在建表之后
        if d == Dialect::Postgres {
            if !t.comment.is_empty() {
                out.push_str(&format!(
                    "COMMENT ON TABLE {} IS '{}';\n",
                    quote(d, &t.name),
                    escape_lit(&t.comment)
                ));
            }
            for f in &body.fields {
                if !f.comment.is_empty() {
                    out.push_str(&format!(
                        "COMMENT ON COLUMN {}.{} IS '{}';\n",
                        quote(d, &t.name),
                        quote(d, &f.name),
                        escape_lit(&f.comment)
                    ));
                }
            }
            for rel in doc
                .relations
                .iter()
                .filter(|r| r.kind != "n-n" && r.from.node == t.id)
            {
                let Some(target) = rel_to_table(doc, rel, &t.id) else {
                    continue;
                };
                out.push_str(&format!(
                    "ALTER TABLE {} ADD {};\n",
                    quote(d, &t.name),
                    fk_clause(d, t, rel, target)
                ));
            }
            out.push('\n');
        }

        // SQLite 没有注释语法，降级成 SQL 注释，别让信息丢掉
        if d == Dialect::Sqlite {
            if !t.comment.is_empty() {
                out.push_str(&format!("-- 表 {}：{}\n", t.name, t.comment));
            }
            for f in &body.fields {
                if !f.comment.is_empty() {
                    out.push_str(&format!("-- 列 {}.{}：{}\n", t.name, f.name, f.comment));
                }
            }
            out.push('\n');
        }
    }

    // n-n：自动生成中间表（人类不用手画）
    for rel in doc.relations.iter().filter(|r| r.kind == "n-n") {
        let jt = junction_table_name(doc, rel);
        let a = doc.node(&rel.from.node);
        let b = doc.node(&rel.to.node);
        let (a_type, b_type) = (
            a.and_then(|n| n.table.as_ref())
                .and_then(|t| t.fields.iter().find(|f| f.name == rel.from.field))
                .map(|f| map_type(d, &f.r#type))
                .unwrap_or_else(|| map_type(d, &DbLogicalType { base: "bigint".into(), ..Default::default() })),
            b.and_then(|n| n.table.as_ref())
                .and_then(|t| t.fields.iter().find(|f| f.name == rel.to.field))
                .map(|f| map_type(d, &f.r#type))
                .unwrap_or_else(|| map_type(d, &DbLogicalType { base: "bigint".into(), ..Default::default() })),
        );
        let a_col = format!("{}_{}", a.map(|n| n.name.as_str()).unwrap_or("a"), rel.from.field);
        let b_col = format!("{}_{}", b.map(|n| n.name.as_str()).unwrap_or("b"), rel.to.field);
        out.push_str(&format!("CREATE TABLE {} (\n", quote(d, &jt)));
        out.push_str(&format!("  {} {} NOT NULL,\n", quote(d, &a_col), a_type));
        out.push_str(&format!("  {} {} NOT NULL,\n", quote(d, &b_col), b_type));
        out.push_str(&format!("  PRIMARY KEY ({}, {}),\n", quote(d, &a_col), quote(d, &b_col)));
        out.push_str(&format!(
            "  FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {},\n",
            quote(d, &a_col),
            quote(d, a.map(|n| n.name.as_str()).unwrap_or("a")),
            quote(d, &rel.from.field),
            fk_action(&rel.on_delete)
        ));
        out.push_str(&format!(
            "  FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {}\n);\n\n",
            quote(d, &b_col),
            quote(d, b.map(|n| n.name.as_str()).unwrap_or("b")),
            quote(d, &rel.to.field),
            fk_action(&rel.on_delete)
        ));
    }

    // 视图
    for v in doc.nodes.iter().filter(|n| n.kind == "view") {
        let sql = v.view.as_ref().map(|x| x.sql.trim()).unwrap_or("");
        if sql.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "CREATE VIEW {} AS\n{};\n\n",
            quote(d, &v.name),
            sql.trim_end_matches(';')
        ));
    }

    out
}

fn kw_for_create(d: Dialect, kind: &str) -> String {
    match (d, kind) {
        (_, "unique") => "UNIQUE INDEX".to_string(),
        (_, "fulltext") => "INDEX".to_string(),
        _ => "INDEX".to_string(),
    }
}

fn fk_action(a: &str) -> &str {
    if a.trim().is_empty() {
        "RESTRICT"
    } else {
        a.trim()
    }
}

/// 关联里「另一端」的表（当前节点是 from 时取 to，反之亦然）
fn rel_to_table<'a>(
    doc: &'a DbDesignDocument,
    rel: &'a DbDesignRelation,
    node_id: &str,
) -> Option<&'a DbDesignNode> {
    if rel.from.node == node_id {
        doc.node(&rel.to.node)
    } else if rel.to.node == node_id {
        doc.node(&rel.from.node)
    } else {
        None
    }
}

fn fk_clause(d: Dialect, table: &DbDesignNode, rel: &DbDesignRelation, target: &DbDesignNode) -> String {
    let (local_field, remote_field) = if rel.from.node == table.id {
        (&rel.from.field, &rel.to.field)
    } else {
        (&rel.to.field, &rel.from.field)
    };
    let name = format!("fk_{}_{}", table.name, local_field);
    let base = format!(
        "FOREIGN KEY ({}) REFERENCES {} ({})",
        quote(d, local_field),
        quote(d, &target.name),
        quote(d, remote_field)
    );
    // SQLite 不支持 ALTER TABLE ADD CONSTRAINT，外键只能在建表语句内
    if d == Dialect::Sqlite {
        format!(
            "{} ON DELETE {} ON UPDATE {}",
            base,
            fk_action(&rel.on_delete),
            fk_action(&rel.on_update)
        )
    } else {
        format!(
            "CONSTRAINT {} {} ON DELETE {} ON UPDATE {}",
            quote(d, &name),
            base,
            fk_action(&rel.on_delete),
            fk_action(&rel.on_update)
        )
    }
}

/// 字段行的「类型之后」部分（column_def 拆出来的另一半，避免重复构造类型位）
fn column_def_tail(d: Dialect, f: &DbField) -> String {
    let mut s = String::new();
    if !(d == Dialect::Sqlite && f.pk && f.auto_increment) {
        s.push_str(if f.nullable { " NULL" } else { " NOT NULL" });
        if d == Dialect::MySql && f.auto_increment {
            s.push_str(" AUTO_INCREMENT");
        }
    } else {
        s.push_str(" PRIMARY KEY AUTOINCREMENT");
    }
    if let Some(def) = &f.default {
        if !def.trim().is_empty() {
            s.push_str(&format!(" DEFAULT {}", def.trim()));
        }
    }
    if f.r#type.base == "enum" && d != Dialect::MySql && !f.r#type.values.is_empty() {
        let vals = f
            .r#type
            .values
            .iter()
            .map(|v| format!("'{}'", escape_lit(v)))
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!(" CHECK ({} IN ({}))", quote(d, &f.name), vals));
    }
    if d == Dialect::MySql && !f.comment.is_empty() {
        s.push_str(&format!(" COMMENT '{}'", escape_lit(&f.comment)));
    }
    s
}
