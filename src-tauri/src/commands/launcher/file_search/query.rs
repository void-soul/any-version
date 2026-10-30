use super::*;

pub(super) fn map_hit(row: &rusqlite::Row<'_>) -> rusqlite::Result<HitRow> {
    Ok((
        row.get(0)?,
        FileHit {
            drive: row.get(1)?,
            path: row.get(2)?,
            name: row.get(3)?,
            is_dir: row.get::<_, i64>(4)? != 0,
            size: row.get(5)?,
            modified: row.get(6)?,
        },
    ))
}

/// 结果类型过滤：文件夹 / 普通文件，以及扩展名白名单（空 = 不限）。
///
/// 必须下推到 SQL：前端过滤会被 100 条上限先截断，反而搜不到目标类型。
pub(super) fn push_type_filter(
    values: &mut Vec<Value>,
    conds: &mut Vec<String>,
    alias: &str,
    kind: &str,
    exts: &[String],
) {
    let col = |name: &str| {
        if alias.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", alias, name)
        }
    };
    match kind {
        "dir" => conds.push(format!("{} = 1", col("is_dir"))),
        "file" => conds.push(format!("{} = 0", col("is_dir"))),
        _ => {}
    }
    if !exts.is_empty() {
        let mut ors = Vec::new();
        for ext in exts {
            values.push(Value::Text(format!("%.{}", escape_like(ext))));
            ors.push(format!(
                "{} LIKE ?{} ESCAPE '\\'",
                col("name_lower"),
                values.len()
            ));
        }
        conds.push(format!("({})", ors.join(" OR ")));
    }
}

/// 首词是否形如盘符：`e`、`e:`、`e:\`。
pub(super) fn token_drive(tok: &str) -> Option<String> {
    let t = tok.trim_end_matches(|c| c == '\\' || c == '/');
    let bytes = t.as_bytes();
    let letter = match bytes.len() {
        1 if bytes[0].is_ascii_alphabetic() => bytes[0],
        2 if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' => bytes[0],
        _ => return None,
    };
    Some(format!("{}:", (letter as char).to_ascii_uppercase()))
}

/// 取命名目录作为 `parent` 的直接子目录（大小写不敏感）。
pub(super) fn find_child_dir(
    conn: &Connection,
    drive: &str,
    parent: &str,
    name: &str,
) -> Result<Option<String>, String> {
    let base = parent.trim_end_matches('\\');
    let pattern = format!("{}\\\\%", escape_like(base));
    let len = base.chars().count() + 1 + name.chars().count();
    conn.query_row(
        "SELECT path FROM fs_entry
         WHERE drive = ?1 AND is_dir = 1 AND name_lower = ?2
           AND path LIKE ?3 ESCAPE '\\' AND length(path) = ?4
         LIMIT 1",
        params![drive, name, pattern, len as i64],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("解析搜索目录失败: {}", e))
}

/// 把原始输入拆成盘符 + 目录前缀 + 关键词。
///
/// 只有拿到确定的盘符时才解析目录：否则 `pro` 既可能是目录名也可能是关键词，
/// 贸然吞掉会让普通搜索搜不到东西。目录解析至少留一个词作关键词。
pub(super) fn resolve_scope(conn: &Connection, raw: &str, drives: &[String]) -> QueryScope {
    let parts: Vec<String> = raw
        .split_whitespace()
        .take(8)
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if parts.is_empty() {
        return QueryScope::default();
    }
    let (typed, start) = match token_drive(&parts[0]) {
        Some(d) => (Some(d), 1),
        None => (None, 0),
    };
    let drive = typed.clone().or_else(|| {
        if drives.len() == 1 {
            Some(drives[0].clone())
        } else {
            None
        }
    });
    let Some(drive) = drive else {
        return QueryScope {
            drive: None,
            path_prefix: None,
            tokens: parts,
        };
    };

    let mut prefix = format!("{}\\", drive);
    let mut consumed = 0usize;
    // 至少留一个词当关键词；`e pro gld` 最后一个词仍是关键词
    while start + consumed + 1 < parts.len() {
        let token = &parts[start + consumed];
        match find_child_dir(conn, &drive, &prefix, token) {
            Ok(Some(path)) => {
                prefix = path;
                consumed += 1;
            }
            Ok(None) => break,
            // 索引读取异常不阻塞搜索，按普通关键词处理
            Err(_) => break,
        }
    }
    let tokens = parts[start + consumed..].to_vec();
    let path_prefix = if consumed > 0 {
        Some(prefix.trim_end_matches('\\').to_string())
    } else {
        None
    };
    QueryScope {
        drive: Some(drive),
        path_prefix,
        tokens,
    }
}

pub(super) fn search_impl(
    query: &str,
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
) -> Result<Vec<FileHit>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 500);

    with_conn(|conn| {
        let scope = resolve_scope(conn, q, drives);
        if scope.tokens.is_empty() {
            return Ok(Vec::new());
        }
        // 输入里写了盘符就以它为准（优先于下拉选择）
        let scoped_drives: Vec<String> = match &scope.drive {
            Some(d) => vec![d.clone()],
            None => drives.to_vec(),
        };
        // 排序用的「整体词」不包含盘符/目录，否则永远命不中前缀
        let q_lower = scope.tokens.join(" ");
        search_with_conn(
            conn,
            &q_lower,
            &scope.tokens,
            limit,
            &scoped_drives,
            kind,
            exts,
            scope.path_prefix.as_deref(),
        )
    })
}

pub(super) fn search_with_conn(
    conn: &Connection,
    q: &str,
    tokens: &[String],
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
) -> Result<Vec<FileHit>, String> {
    // trigram 只索引 ≥3 字符的片段，短词无法命中全文索引，交给 LIKE。
    if tokens.iter().all(|t| t.chars().count() >= 3) {
        match fts_search(conn, q, tokens, limit, drives, kind, exts, path_prefix) {
            Ok(hits) => return Ok(hits),
            // 全文索引异常（缺失/损坏）时退回 LIKE：先保证搜得到，再谈搜得快。
            Err(err) => crate::exit_log!("[文件搜索] 全文检索失败，回退 LIKE: {}", err),
        }
    }
    like_search(conn, q, tokens, limit, drives, kind, exts, path_prefix)
}

/// 目录范围下推到 SQL：只要该前缀下（含更深层级）的条目。
pub(super) fn push_path_filter(
    values: &mut Vec<Value>,
    conds: &mut Vec<String>,
    alias: &str,
    path_prefix: Option<&str>,
) {
    let Some(prefix) = path_prefix else {
        return;
    };
    let base = prefix.trim_end_matches('\\');
    if base.is_empty() {
        return;
    }
    let col = |name: &str| {
        if alias.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", alias, name)
        }
    };
    values.push(Value::Text(format!("{}\\\\%", escape_like(base))));
    conds.push(format!(
        "{} LIKE ?{} ESCAPE '\\'",
        col("path"),
        values.len()
    ));
}

/// FTS5 查询表达式：每个词用双引号包成短语，词间 AND（即「同时包含」）。
pub(super) fn fts_expr(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// 走 trigram 全文索引：中缀、前缀、完全匹配都由索引定位，避免全表扫描。
pub(super) fn fts_search(
    conn: &Connection,
    q: &str,
    tokens: &[String],
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
) -> Result<Vec<FileHit>, String> {
    let mut values: Vec<Value> = vec![Value::Text(fts_expr(tokens))];
    let mut conds: Vec<String> = vec!["fs_entry_fts MATCH ?1".to_string()];

    if !drives.is_empty() {
        let mut placeholders = Vec::new();
        for d in drives {
            values.push(Value::Text(d.clone()));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("e.drive IN ({})", placeholders.join(",")));
    }
    push_path_filter(&mut values, &mut conds, "e", path_prefix);
    push_type_filter(&mut values, &mut conds, "e", kind, exts);

    let q_lower = q.to_lowercase();
    values.push(Value::Text(q_lower.clone()));
    let exact_idx = values.len();
    values.push(Value::Text(format!("{}%", escape_like(&q_lower))));
    let prefix_idx = values.len();
    values.push(Value::Integer(limit as i64));
    let limit_idx = values.len();

    let sql = format!(
        "SELECT e.id, e.drive, e.path, e.name, e.is_dir, e.size, e.modified
         FROM fs_entry_fts JOIN fs_entry e ON e.id = fs_entry_fts.rowid
         WHERE {conds}
         ORDER BY CASE
             WHEN e.name_lower = ?{exact} THEN 0
             WHEN e.name_lower LIKE ?{prefix} ESCAPE '\\' THEN 1
             ELSE 2 END,
         length(e.name), e.name COLLATE NOCASE
         LIMIT ?{limit}",
        conds = conds.join(" AND "),
        exact = exact_idx,
        prefix = prefix_idx,
        limit = limit_idx,
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("准备全文检索语句失败: {}", e))?;
    let rows = stmt
        .query_map(params_from_iter(values), map_hit)
        .map_err(|e| format!("执行全文检索失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map(|rows| rows.into_iter().map(|(_, hit)| hit).collect())
        .map_err(|e| format!("读取全文检索结果失败: {}", e))
}

/// 短词（<3 字符）退化路径：前缀命中走 `idx_fs_name`，再用中缀扫描补足剩余名额。
pub(super) fn like_search(
    conn: &Connection,
    q: &str,
    tokens: &[String],
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
) -> Result<Vec<FileHit>, String> {
    let mut rows = prefix_search(conn, q, limit, drives, kind, exts, path_prefix)?;
    if rows.len() < limit {
        let exclude: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
        rows.extend(infix_search(
            conn,
            tokens,
            limit - rows.len(),
            drives,
            kind,
            exts,
            path_prefix,
            &exclude,
        )?);
    }
    Ok(rows.into_iter().map(|(_, hit)| hit).collect())
}

/// 第一段：完全同名 + 前缀命中（两段都能命中索引）。
pub(super) fn prefix_search(
    conn: &Connection,
    q: &str,
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
) -> Result<Vec<HitRow>, String> {
    let q_lower = q.to_lowercase();
    let mut values: Vec<Value> = Vec::new();
    values.push(Value::Text(q_lower.clone()));
    let exact_idx = values.len();
    values.push(Value::Text(format!("{}%", escape_like(&q_lower))));
    let prefix_idx = values.len();

    let mut conds: Vec<String> = vec![format!(
        "(name_lower = ?{exact} OR name_lower LIKE ?{prefix} ESCAPE '\\')",
        exact = exact_idx,
        prefix = prefix_idx
    )];
    if !drives.is_empty() {
        let mut placeholders = Vec::new();
        for d in drives {
            values.push(Value::Text(d.clone()));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("drive IN ({})", placeholders.join(",")));
    }
    push_path_filter(&mut values, &mut conds, "", path_prefix);
    push_type_filter(&mut values, &mut conds, "", kind, exts);
    values.push(Value::Integer(limit as i64));
    let limit_idx = values.len();

    let sql = format!(
        "SELECT id, drive, path, name, is_dir, size, modified FROM fs_entry
         WHERE {conds}
         ORDER BY CASE WHEN name_lower = ?{exact} THEN 0 ELSE 1 END,
         length(name), name COLLATE NOCASE
         LIMIT ?{limit}",
        conds = conds.join(" AND "),
        exact = exact_idx,
        limit = limit_idx,
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("准备搜索语句失败: {}", e))?;
    let rows = stmt
        .query_map(params_from_iter(values), map_hit)
        .map_err(|e| format!("执行搜索失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取搜索结果失败: {}", e))
}

/// 第二段：多词中缀匹配，排除第一段已返回的 rowid，避免重复。
pub(super) fn infix_search(
    conn: &Connection,
    tokens: &[String],
    limit: usize,
    drives: &[String],
    kind: &str,
    exts: &[String],
    path_prefix: Option<&str>,
    exclude_ids: &[i64],
) -> Result<Vec<HitRow>, String> {
    let mut values: Vec<Value> = Vec::new();
    let mut conds: Vec<String> = Vec::new();

    for token in tokens {
        values.push(Value::Text(format!("%{}%", escape_like(token))));
        conds.push(format!("name_lower LIKE ?{} ESCAPE '\\'", values.len()));
    }
    if !drives.is_empty() {
        let mut placeholders = Vec::new();
        for d in drives {
            values.push(Value::Text(d.clone()));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("drive IN ({})", placeholders.join(",")));
    }
    push_path_filter(&mut values, &mut conds, "", path_prefix);
    push_type_filter(&mut values, &mut conds, "", kind, exts);
    if !exclude_ids.is_empty() {
        let mut placeholders = Vec::new();
        for id in exclude_ids {
            values.push(Value::Integer(*id));
            placeholders.push(format!("?{}", values.len()));
        }
        conds.push(format!("id NOT IN ({})", placeholders.join(",")));
    }
    values.push(Value::Integer(limit as i64));
    let limit_idx = values.len();

    let sql = format!(
        "SELECT id, drive, path, name, is_dir, size, modified FROM fs_entry
         WHERE {conds}
         ORDER BY length(name), name COLLATE NOCASE
         LIMIT ?{limit}",
        conds = conds.join(" AND "),
        limit = limit_idx,
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("准备搜索语句失败: {}", e))?;
    let rows = stmt
        .query_map(params_from_iter(values), map_hit)
        .map_err(|e| format!("执行搜索失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取搜索结果失败: {}", e))
}

#[tauri::command]
pub async fn file_search_query(
    query: String,
    limit: Option<usize>,
    drives: Option<Vec<String>>,
    kind: Option<String>,
    exts: Option<Vec<String>>,
) -> Result<Vec<FileHit>, String> {
    let normalized: Vec<String> = drives
        .unwrap_or_default()
        .iter()
        .filter_map(|d| normalize_drive(d).ok())
        .collect();
    let limit = limit.unwrap_or(100);
    let kind = match kind.as_deref() {
        Some("dir") => "dir",
        Some("file") => "file",
        _ => "all",
    }
    .to_string();
    let exts: Vec<String> = exts
        .unwrap_or_default()
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 16 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .take(24)
        .collect();
    tauri::async_runtime::spawn_blocking(move || {
        search_impl(&query, limit, &normalized, &kind, &exts)
    })
    .await
    .map_err(|e| format!("搜索线程失败: {}", e))?
}
