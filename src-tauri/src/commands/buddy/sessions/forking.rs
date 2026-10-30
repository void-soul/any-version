use super::*;

// ─── 会话分叉 ───

/// 分叉结果：新会话 + 正文复制情况（正文没拷到要如实告诉前端，避免开出个空会话还以为成功了）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySessionForkReport {
    pub conversation_id: String,
    pub title: String,
    /// 成功复制的正文文件数（WorkBuddy 为 jsonl / artifact-index）
    pub files_copied: usize,
    /// 正文一个都没拷到：新会话只有库记录
    pub content_missing: bool,
    /// 非致命问题（个别文件被占用等），不中断分叉
    pub errors: Vec<String>,
}

/// 分叉会话：复制会话正文 + 用新 id 在会话库里注册一条记录，原会话原样保留。
///
/// - WorkBuddy：`~/.workbuddy/projects/<hash>/<id>.jsonl` 是对话正文（复制时把文件里出现的
///   旧会话 id 全部换成新 id，否则库里两条记录指向同一份 sessionId，打开/续聊会串台）；
///   `artifact-index/<id>.json` 一并复制。**不复制** `workspace/sessions`、`tasks`、
///   `file-history`——那是工作区快照与文件历史，实测单个会话可达 700MB，分叉不该背这个体积。
/// - CodeBuddy CN：会话索引在 vscdb 的 `session:<id>` 键，正文在扩展目录的
///   `<workspace>/<id>/`，两边都要复制一份。
pub fn fork_session(
    platform: &str,
    conversation_id: &str,
    title: Option<String>,
) -> Result<BuddySessionForkReport, String> {
    let src = conversation_id.trim();
    crate::commands::buddy::session_transfer::codebuddy::validate_conversation_id(src)?;
    let new_id = new_uuid_v4()?;
    match platform {
        "workbuddy" => fork_workbuddy_session(src, &new_id, title.as_deref()),
        "codebuddy-cn" | "codebuddy_cn" => {
            fork_codebuddy_cn_session(src, &new_id, title.as_deref())
        }
        other => Err(format!("未知平台: {}", other)),
    }
}

/// 分叉后的默认标题：沿用原标题并加标记（原标题为空时只有标记）。
pub(super) fn fork_title(src_title: &str, title: Option<&str>) -> String {
    let custom = title.map(|t| t.trim()).unwrap_or("");
    if !custom.is_empty() {
        return custom.to_string();
    }
    if src_title.trim().is_empty() {
        "（分叉）".to_string()
    } else {
        format!("{}（分叉）", src_title.trim())
    }
}

pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// UUID v4（本仓库没有 uuid crate，用 getrandom 自己拼）。
pub(super) fn new_uuid_v4() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|e| format!("生成会话 id 失败: {}", e))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

/// 表的列顺序（`SELECT *` 与 `PRAGMA table_info` 同序；写死列名会随版本炸）。
pub(super) fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| format!("读取表结构失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("读取表结构失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows.flatten() {
        out.push(r);
    }
    if out.is_empty() {
        return Err(format!("表 {} 不存在", table));
    }
    Ok(out)
}

pub(super) fn value_as_text(v: &rusqlite::types::Value) -> String {
    match v {
        rusqlite::types::Value::Text(t) => t.clone(),
        rusqlite::types::Value::Integer(n) => n.to_string(),
        rusqlite::types::Value::Real(f) => f.to_string(),
        _ => String::new(),
    }
}

/// WorkBuddy 分叉：复制 sessions 行（换 id/标题/时间）+ 复制正文文件。
pub(super) fn fork_workbuddy_session(
    src_id: &str,
    new_id: &str,
    title: Option<&str>,
) -> Result<BuddySessionForkReport, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    fork_workbuddy_session_at(&home.join(".workbuddy"), src_id, new_id, title)
}

/// 分叉核心（数据根目录参数化，便于用临时目录做端到端测试）。
pub(super) fn fork_workbuddy_session_at(
    data_root: &Path,
    src_id: &str,
    new_id: &str,
    title: Option<&str>,
) -> Result<BuddySessionForkReport, String> {
    let db_path = data_root.join("workbuddy.db");
    if !db_path.is_file() {
        return Err("未找到 WorkBuddy 会话库（~/.workbuddy/workbuddy.db）".to_string());
    }
    crate::commands::buddy::session_transfer::workbuddy::reject_symlink_if_exists(&db_path)?;

    let mut conn =
        Connection::open(&db_path).map_err(|e| format!("打开 WorkBuddy 数据库失败: {}", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置数据库超时失败: {}", e))?;

    // 1) 复制库记录：按列名动态取值（列集合随版本增减，写死 SQL 会炸）
    let columns = table_columns(&conn, "sessions")?;
    let mut values: Vec<rusqlite::types::Value> = Vec::with_capacity(columns.len());
    {
        let mut stmt = conn
            .prepare("SELECT * FROM sessions WHERE id = ?1")
            .map_err(|e| format!("读取源会话失败: {}", e))?;
        let mut rows = stmt
            .query(rusqlite::params![src_id])
            .map_err(|e| format!("查询源会话失败: {}", e))?;
        let row = rows
            .next()
            .map_err(|e| format!("读取源会话失败: {}", e))?
            .ok_or_else(|| format!("会话不存在: {}", src_id))?;
        for i in 0..columns.len() {
            values.push(
                match row
                    .get_ref(i)
                    .map_err(|e| format!("读取会话字段失败: {}", e))?
                {
                    rusqlite::types::ValueRef::Null => rusqlite::types::Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => rusqlite::types::Value::Integer(n),
                    rusqlite::types::ValueRef::Real(f) => rusqlite::types::Value::Real(f),
                    rusqlite::types::ValueRef::Text(t) => {
                        rusqlite::types::Value::Text(String::from_utf8_lossy(t).to_string())
                    }
                    rusqlite::types::ValueRef::Blob(b) => rusqlite::types::Value::Blob(b.to_vec()),
                },
            );
        }
    }

    let src_title = columns
        .iter()
        .position(|c| c == "title")
        .map(|i| value_as_text(&values[i]))
        .unwrap_or_default();
    let new_title = fork_title(&src_title, title);
    let now = now_ms();
    for (i, c) in columns.iter().enumerate() {
        match c.as_str() {
            "id" => values[i] = rusqlite::types::Value::Text(new_id.to_string()),
            "title" => values[i] = rusqlite::types::Value::Text(new_title.clone()),
            "custom_title" => values[i] = rusqlite::types::Value::Text(new_title.clone()),
            // 时间戳刷新，保证新会话排在列表最前
            "created_at" | "updated_at" | "last_activity_at" => {
                values[i] = rusqlite::types::Value::Integer(now)
            }
            // 源会话若被软删，分叉出来的应当是干净的
            "deleted_at" => values[i] = rusqlite::types::Value::Null,
            _ => {}
        }
    }

    let cols_sql = columns
        .iter()
        .map(|c| format!("\"{}\"", c))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=columns.len())
        .map(|i| format!("?{}", i))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO sessions ({}) VALUES ({})",
        cols_sql, placeholders
    );
    {
        let tx = conn
            .transaction()
            .map_err(|e| format!("开启数据库事务失败: {}", e))?;
        tx.execute(&sql, rusqlite::params_from_iter(values))
            .map_err(|e| format!("写入新会话记录失败: {}", e))?;
        tx.commit()
            .map_err(|e| format!("提交新会话记录失败: {}", e))?;
    }

    // 2) 复制正文
    let mut files_copied = 0usize;
    let mut errors: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(data_root.join("projects")) {
        for entry in entries.flatten() {
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }
            let src_file = project_dir.join(format!("{}.jsonl", src_id));
            if !src_file.is_file() {
                continue;
            }
            match copy_text_replacing(
                &src_file,
                &project_dir.join(format!("{}.jsonl", new_id)),
                src_id,
                new_id,
            ) {
                Ok(()) => files_copied += 1,
                Err(e) => errors.push(e),
            }
        }
    }
    let artifact = data_root
        .join("artifact-index")
        .join(format!("{}.json", src_id));
    if artifact.is_file() {
        match copy_text_replacing(
            &artifact,
            &data_root
                .join("artifact-index")
                .join(format!("{}.json", new_id)),
            src_id,
            new_id,
        ) {
            Ok(()) => files_copied += 1,
            Err(e) => errors.push(e),
        }
    }

    Ok(BuddySessionForkReport {
        conversation_id: new_id.to_string(),
        title: new_title,
        files_copied,
        content_missing: files_copied == 0,
        errors,
    })
}

/// 复制文本文件，并把内容里出现的旧会话 id 换成新 id（正文里每条记录都带 sessionId）。
/// 非 UTF-8 时退化为原样复制——宁可少改一个 id，也不要把二进制正文写坏。
pub(super) fn copy_text_replacing(
    src: &Path,
    dst: &Path,
    old_id: &str,
    new_id: &str,
) -> Result<(), String> {
    if let Err(e) =
        crate::commands::buddy::session_transfer::codebuddy::reject_symlink_if_exists(src)
    {
        return Err(e);
    }
    let bytes = std::fs::read(src).map_err(|e| format!("读取正文失败 {}: {}", src.display(), e))?;
    let out = match String::from_utf8(bytes.clone()) {
        Ok(text) => text.replace(old_id, new_id).into_bytes(),
        // 非 UTF-8：原样复制（宁可少改一个 id，也不要把二进制正文写坏）
        Err(e) => {
            eprintln!("[BuddySession] 正文非 UTF-8，原样复制: {}", src.display());
            let _ = e;
            bytes
        }
    };
    std::fs::write(dst, out).map_err(|e| format!("写入正文失败 {}: {}", dst.display(), e))?;
    Ok(())
}
