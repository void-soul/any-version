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
    export_sql_with(doc, ExportOptions::default())
}

/// 导出选项：**只作用于这一次导出**，不进设计文件（设计文件描述「表长什么样」，
/// 导出选项描述「这份 SQL 怎么落地」—— 后者每次都可能不同，存进文件反而会让人
/// 拿到文件时看到一份自己不记得设过的开关）。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    /// 是否输出外键约束。关掉时关联只体现在设计文件与画布上，由应用程序保证
    /// （用户场景：不想被外键约束住删数据 —— 删父行时被`ON DELETE` 拦下、
    /// 或为了绕开它被迫留一堆`SET NULL`）。
    ///
    /// 关掉后 n-n 自动生成的中间表**也不带外键**：只留两列 + 复合主键。
    /// 这是「这份 SQL 零外键约束」的预期一致性 —— 半留半不留比全不留更难解释。
    #[serde(default = "default_true")]
    pub foreign_keys: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { foreign_keys: true }
    }
}

/// 带选项的导出。**默认行为与 `export_sql` 完全一致**（带外键），
/// 所以既有测试与既有调用点都不需要改。
pub fn export_sql_with(doc: &DbDesignDocument, opts: ExportOptions) -> Result<String, String> {
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
        "mysql" => Ok(export_relational(doc, Dialect::MySql, opts)),
        "postgres" => Ok(export_relational(doc, Dialect::Postgres, opts)),
        "sqlite" => Ok(export_relational(doc, Dialect::Sqlite, opts)),
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
    // 决策记录：这些是**为什么**，不是表结构本身。排在最前面，
    // 所以“不打开应用也能看懂这份设计”的 Markdown 里，读者看到的不只是表结构，
    // 还有业务约束 —— 比多数据库文档都只写结构、不写动机，使用方看不懂并改掉。
    if !doc.decisions.is_empty() {
        out.push_str("\n## 设计决策与约束\n\n");
        for d in doc.decisions.iter().filter(|d| !d.trim().is_empty()) {
            out.push_str(&format!("- {}\n", d.trim()));
        }
    }
    // 公共字段是**文档级**配置，不属于任何一张表，所以单独列在最前面 ——
    // 不写的话，读者只会以为导出时凭空多了几列。
    if !doc.common_fields.is_empty() {
        out.push_str("\n## 公共字段（导出时自动附加到每张表）\n\n");
        for f in &doc.common_fields {
            out.push_str(&format!(
                "- `{}` {}{}\n",
                f.name,
                render_logical_type(f.r#type.clone()),
                if f.comment.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", f.comment)
                }
            ));
        }
        out.push_str("\n> 表里已有同名字段时以表自己的为准（不重复添加）。\n");
    }
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

fn export_relational(doc: &DbDesignDocument, d: Dialect, opts: ExportOptions) -> String {
    let mut out = String::new();
    out.push_str(&format!("-- {}\n", doc.name));
    if !doc.description.is_empty() {
        out.push_str(&format!("-- {}\n", doc.description));
    }
    out.push_str(&format!(
        "-- 由 Kira 数据库设计器生成 · 方言 {}\n",
        doc.dialect
    ));
    if !opts.foreign_keys {
        // 不写这一行，别人拿到这份 SQL 会以为外键是漏掉了 —— 而它是有意去掉的
        out.push_str("-- 本次导出未包含外键约束：关联关系只体现在设计文件与画布上，由应用程序保证\n");
    }
    out.push('\n');

    let tables: Vec<&DbDesignNode> = doc.nodes.iter().filter(|n| n.kind == "table").collect();

    for t in &tables {
        let body = t.table.as_ref().unwrap();
        // 公共字段：配置里启用、且这张表没有同名的才追加（重名以表自己的为准，不合并）。
        // 追加在**表自身字段之后**（列定义的最后几行）—— 放中间会打乱「主键在前、
        // 外键紧随其后」的阅读顺序，而这几列本来就没人关心它们排第几。
        let (common_add, common_skipped) =
            super::common_fields::common_fields_for(&doc.common_fields, &body.fields);
        // 跳过要留痕，且放在建表语句**之前**：表里本来就有 created_at 的人，
        // 应该知道「这个字段本来会加，是我自己那个赢了」。
        for f in &common_skipped {
            out.push_str(&format!(
                "-- 表 {}：公共字段 {} 因表内已有同名字段而跳过（以表自己的定义为准）\n",
                t.name, f.name
            ));
        }
        out.push_str(&format!("CREATE TABLE {} (\n", quote(d, &t.name)));
        let mut lines: Vec<String> = Vec::new();

        for f in &body.fields {
            let mut line = format!("  {} {}", quote(d, &f.name), column_type_for(d, f));
            let rest = column_def_tail(d, f);
            line.push_str(&rest);
            lines.push(line);
        }

        for f in &common_add {
            let mut line = format!("  {} {}", quote(d, &f.name), column_type_for(d, f));
            line.push_str(&column_def_tail(d, f));
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
            // MySQL 不允许 TEXT/BLOB 进普通 / 唯一索引（要指定前缀长度才行），
            // 生成了也是建表失败，直接跳过并留一行注释说明（fulltext 例外，见 mysql_forbids_key_on）
            if d == Dialect::MySql
                && idx.fields.iter().any(|name| {
                    body.fields
                        .iter()
                        .find(|f| &f.name == name)
                        .map(|f| mysql_forbids_key_on(&f.r#type.base, &idx.kind))
                        .unwrap_or(false)
                })
            {
                out.push_str(&format!(
                    "\n-- 已跳过索引 {}：MySQL 不允许 TEXT/BLOB 列建普通或唯一索引（需要指定前缀长度）\n",
                    idx.name
                ));
                continue;
            }
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
        //
        // `opts.foreign_keys = false` 时整段跳过：用户不要数据库层的引用约束，
        // 由应用程序保证（见 ExportOptions）。
        if opts.foreign_keys {
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
            if opts.foreign_keys {
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
        // 收集成列表再 join：外键那两行是**可选**的（跟着 opts.foreign_keys 走），
        // 手写「前一行结尾带逗号」很容易在去掉外键后留下悬空逗号。
        let mut jl: Vec<String> = vec![
            format!("  {} {} NOT NULL", quote(d, &a_col), a_type),
            format!("  {} {} NOT NULL", quote(d, &b_col), b_type),
            format!("  PRIMARY KEY ({}, {})", quote(d, &a_col), quote(d, &b_col)),
        ];
        if opts.foreign_keys {
            jl.push(format!(
                "  FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {}",
                quote(d, &a_col),
                quote(d, a.map(|n| n.name.as_str()).unwrap_or("a")),
                quote(d, &rel.from.field),
                fk_action(&rel.on_delete)
            ));
            jl.push(format!(
                "  FOREIGN KEY ({}) REFERENCES {} ({}) ON DELETE {}",
                quote(d, &b_col),
                quote(d, b.map(|n| n.name.as_str()).unwrap_or("b")),
                quote(d, &rel.to.field),
                fk_action(&rel.on_delete)
            ));
        }
        out.push_str(&jl.join(",\n"));
        out.push_str("\n);\n\n");
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

/// MySQL 不允许这些类型带**字面量默认值**（报错 1101：
/// `BLOB, TEXT, GEOMETRY or JSON column can't have a default value`）。
/// MySQL 8.0.13+ 虽支持「表达式默认值」，但对这些类型一样禁止。
///
/// 为什么要在导出层处理而不是只报校验错误：这类列的默认值往往是 AI 顺手写的
///（`posture_data json NOT NULL DEFAULT '{}'` 之类），用户改设计的心智成本远高于
/// 「导出时忽略这一个默认值」—— 而留着就会让整份 DDL 执行失败。
fn mysql_forbids_literal_default(base: &str) -> bool {
    matches!(base, "text" | "blob" | "json")
}

/// MySQL 不允许 TEXT/BLOB 列出现在普通 / 唯一索引里（报错 1170：
/// `BLOB/TEXT column used in key specification without a key length`）——
/// 要建必须指定前缀长度（`KEY idx (col(191))`），那已经是「改设计」而不是「导出」了。
///
/// **fulltext 是例外**：全文索引本来就是给文本列建的，MySQL 支持。
fn mysql_forbids_key_on(base: &str, kind: &str) -> bool {
    matches!(base, "text" | "blob") && kind != "fulltext"
}

/// PG 的 json / jsonb 列不接受字符串字面量默认值，必须带类型标注
/// （否则报 `column is of type jsonb but default expression is of type text`）。
/// MySQL / SQLite 的 json 列都是文本存储，写普通字面量即可。
fn pg_json_default_needs_cast(base: &str) -> bool {
    matches!(base, "json")
}

/// 这些类型的字面量在 SQL 里是**字符串**，所以默认值必须带引号。
///
/// `date` / `time` / `datetime` / `timestamp` 也在内：MySQL 与 PG 都接受不带引号的
/// 日期字面量，但那是方言的宽松之处，不是可移植写法（SQLite 干脆不认）。
fn default_is_string_literal(base: &str) -> bool {
    matches!(
        base,
        "char" | "varchar" | "text" | "enum" | "json" | "uuid" | "blob" | "date" | "time"
            | "datetime" | "timestamp"
    )
}

/// 值已经带了引号 / 是括号表达式 / 是 SQL 关键字或函数调用 —— 都不能再套引号。
///
/// 这一步是自动加引号的安全阀：只对「裸词」动手。`CURRENT_TIMESTAMP`、`now()`、
/// `uuid_generate_v4()`、`(col + 1)` 这些一旦被套上引号就彻底变义了，而它们长得
/// 和该加引号的 `enabled` 只差「有没有括号、有没有下划线」—— 所以宁可放过，
/// 不能错改（错改的产物能跑、但语义错了，比报出来难查得多）。
///
/// 这个默认值是否需要加引号。
///
/// **pub 是给 `store::validate` 用的**：校验要报「这个默认值缺引号」，导出要报
/// 「给你补上了」，两处必须同一口径 —— 各写一份必然漂移（而漂移的后果是
/// 「校验说没问题、导出的 SQL 却跑不起来」）。
pub fn default_needs_quoting(base: &str, v: &str) -> bool {
    default_is_string_literal(base) && !default_already_quoted_or_expression(v)
}

fn default_already_quoted_or_expression(v: &str) -> bool {
    if v.starts_with('\'') || v.starts_with('"') || v.starts_with('`') || v.starts_with('(') {
        return true;
    }
    const KEYWORDS: [&str; 14] = [
        "NULL", "TRUE", "FALSE", "UNKNOWN", "CURRENT_TIMESTAMP", "CURRENT_DATE",
        "CURRENT_TIME", "LOCALTIME", "LOCALTIMESTAMP", "CURRENT_USER", "SESSION_USER",
        "USER", "DEFAULT", "NOW",
    ];
    // 纯数字（含负号/小数）不是字符串字面量，别碰
    if v.parse::<f64>().is_ok() {
        return true;
    }
    // 带括号的当函数调用处理（now() / uuid_generate_v4() / CURRENT_TIMESTAMP(3)）
    if v.contains('(') {
        return true;
    }
    KEYWORDS.contains(&v.to_ascii_uppercase().as_str())
}

/// 字符串型默认值补上单引号（内部单引号按 SQL 规则双写）。
fn quote_string_default(v: &str) -> String {
    format!("'{}'", escape_lit(v))
}

/// 字段默认值 → 该方言能跑的 SQL 字面量（空 / 缺省 / 该方言禁止 → `None`，即不写 DEFAULT）。
///
/// 三件事按顺序做：
///
/// 1. **boolean 按方言归一**：`boolean` 只是设计器内部的跨方言抽象，落到各家是不同的类型 ——
///    MySQL 没有 BOOLEAN（`map_type` 导成 `TINYINT(1)`）、SQLite 只有存储类 INTEGER，
///    列是数值型的，默认值就该是 `0` / `1`；只有 PG 真有 BOOLEAN 类型，才写 `FALSE` / `TRUE`。
///    不归一的话，AI 写的 `"default": "false"` 会原样变成 `TINYINT(1) ... DEFAULT false`
///    —— 三家恰好都认这个词所以不会报错，但导出的 DDL 语义与列类型对不上。
/// 2. **字符串型默认值补引号**：写 `"default": "enabled"`（enum 极常见）会原样拼成
///    `DEFAULT enabled`，那是语法错误，**整份建表语句都跑不起来**。这类值在设计文件里
///    存的是「字符串形式的 SQL 字面量」而不是「字符串内容」，漏引号是最高频的写错方式。
/// 3. **PG 的 json/jsonb 补 `::jsonb`**：否则报 default expression is of type text。
fn normalize_default(d: Dialect, f: &DbField) -> Option<String> {
    let raw = f.default.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let base = f.r#type.base.as_str();
    if d == Dialect::MySql && mysql_forbids_literal_default(base) {
        return None;
    }
    if base == "boolean" {
        let b = match raw.to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "1" => true,
            "false" | "f" | "no" | "0" => false,
            // 认不出来的（用户手填的表达式等）原样透传，不擅自改写语义
            _ => return Some(raw.to_string()),
        };
        return Some(match (d, b) {
            (Dialect::Postgres, true) => "TRUE".to_string(),
            (Dialect::Postgres, false) => "FALSE".to_string(),
            (_, true) => "1".to_string(),
            (_, false) => "0".to_string(),
        });
    }
    let quoted = if default_needs_quoting(base, raw) {
        quote_string_default(raw)
    } else {
        raw.to_string()
    };
    if d == Dialect::Postgres && pg_json_default_needs_cast(base) {
        return Some(format!("{quoted}::jsonb"));
    }
    Some(quoted)
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
    if let Some(def) = normalize_default(d, f) {
        s.push_str(&format!(" DEFAULT {}", def));
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
