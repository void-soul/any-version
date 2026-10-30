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
use super::store::{normalize_naming, validate};

/// 生成节点 / 关联 id。
///
/// ⚠ 必须带进程内自增序号：只用毫秒时间戳的话，**同一毫秒里建的两张表会拿到同一个 id**
/// （踩过：CREATE INDEX 按 id 找节点，两张表 id 相同 → 索引挂到了第一张表上）。
fn new_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}_{}_{}",
        prefix,
        chrono::Utc::now().timestamp_millis(),
        seq
    )
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
    let mut t = DbLogicalType {
        base: base.to_string(),
        unsigned,
        ..Default::default()
    };
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
    let mut t = DbLogicalType {
        base: base.to_string(),
        ..Default::default()
    };
    if matches!(base, "varchar" | "char") {
        t.length = args
            .first()
            .and_then(|a| a.trim().parse().ok())
            .or(Some(255));
    } else if base == "decimal" {
        t.precision = args
            .first()
            .and_then(|a| a.trim().parse().ok())
            .or(Some(10));
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
    let t = t
        .strip_prefix('`')
        .and_then(|x| x.strip_suffix('`'))
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
    let end = t
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(t.len());
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
    let mut action = first
        .trim_end_matches(';')
        .trim_end_matches(',')
        .to_uppercase();
    if action == "SET" {
        if let Some(second) = words.next() {
            action = format!("SET {}", second.trim_end_matches(';').to_uppercase());
        }
    }
    action
}

// ─────────────────────────── DDL 解析 ───────────────────────────

// ─────────────────────────── SQLite 直连 ───────────────────────────

mod ddl;
pub use ddl::*;
mod sqlite;
pub use sqlite::*;
#[cfg(test)]
mod tests;
