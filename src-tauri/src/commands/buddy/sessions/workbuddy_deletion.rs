use super::*;

pub(super) fn delete_workbuddy_sessions(
    ids: &[String],
) -> Result<BuddySessionDeleteReport, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let data_root = home.join(".workbuddy");
    let uid = current_platform_uid(crate::commands::buddy::models::BuddyPlatform::Workbuddy);

    let mut report = BuddySessionDeleteReport::default();
    let mut extension_roots: Vec<PathBuf> = Vec::new();
    if let Some(uid) = uid.as_deref() {
        match crate::commands::buddy::session_transfer::workbuddy::select_extension_data_roots(uid)
        {
            Ok(roots) => extension_roots.extend(roots.into_iter().map(|(_, dir)| dir)),
            Err(e) => report.errors.push(e),
        }
    }
    delete_workbuddy_sessions_at(&data_root, &extension_roots, ids, &mut report);
    Ok(report)
}

/// WorkBuddy 删除核心（路径参数化以便测试）。
///
/// 顺序对齐 WorkDaddy `daemon.js` `/api/sessions/delete`（1.2.64）：
/// 1. **先删可重试的正文**（`projects/<hash>/<id>.jsonl` 与 `<id>/`、`workspace/sessions/<id>`、
///    `tasks/<id>`、`file-history/<id>`、`artifact-index/<id>.json`）；
/// 2. 清 `app/sessions.json` 里的会话缓存条目（不清的话客户端列表会从缓存"复活"刚删的会话）；
/// 3. 旧版 per-uid 扩展目录（5.x 通常不存在）；
/// 4. **最后**才删 `workbuddy.db` 的 sessions 行 —— 它是失败重试的锚点。
///
/// 正文删不掉时该会话的 DB 行**保留**：宁可让用户重试，也不要留下"列表里没了、磁盘上还在"的孤儿正文。
pub(super) fn delete_workbuddy_sessions_at(
    data_root: &Path,
    extension_roots: &[PathBuf],
    ids: &[String],
    report: &mut BuddySessionDeleteReport,
) {
    // 1) 5.x 正文（按 id 精确删）
    let mut file_failed: HashSet<&str> = HashSet::new();
    if data_root.is_dir() {
        for id in ids {
            match delete_workbuddy_session_files(data_root, id) {
                Ok(n) => report.files_removed += n,
                Err(e) => {
                    report.errors.push(format!(
                        "删除会话正文失败（已保留数据库记录以便重试） {}: {}",
                        id, e
                    ));
                    file_failed.insert(id.as_str());
                }
            }
        }
    }
    let deletable: Vec<String> = ids
        .iter()
        .filter(|id| !file_failed.contains(id.as_str()))
        .cloned()
        .collect();

    // 2) 会话缓存条目
    match remove_workbuddy_session_cache(data_root, &deletable) {
        Ok(0) => {}
        Ok(n) => eprintln!("[Buddy Sessions] 已清理会话缓存条目 {}", n),
        Err(e) => {
            report.errors.push(format!(
                "清理会话缓存失败（已保留数据库记录以便重试）: {}",
                e
            ));
            return;
        }
    }

    // 3) 旧版扩展目录（同样扫描该根下所有账号，不只看当前登录账号）
    for root in extension_roots {
        if !delete_session_files_in_extension(root, &deletable, report) {
            report
                .errors
                .push("旧版扩展目录中的会话文件未能全部删除，已保留数据库记录以便重试".to_string());
            return;
        }
    }

    // 4) 最后删 DB 行
    let db_path = data_root.join("workbuddy.db");
    if db_path.is_file() && !deletable.is_empty() {
        delete_workbuddy_db_rows(&db_path, &deletable, report);
    }
}

/// 删除 WorkBuddy 5.x 一个会话的全部正文，返回删除项数。
///
/// 路径集合与 `session_sync::workbuddy_session_fingerprint` 一致（正文指纹算得出来，就删得掉）。
/// 安全边界：父目录必须是**真目录**（符号链接不进）；叶子是符号链接时**只删链接本身**，不跟随。
/// 任一项删不掉返回 `Err`，调用方据此保留 DB 行作为重试锚点。
pub(super) fn delete_workbuddy_session_files(data_root: &Path, id: &str) -> Result<usize, String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err(format!("会话 id 不安全: {}", id));
    }
    let mut removed = 0usize;
    let mut failures: Vec<String> = Vec::new();

    // projects/<项目 hash>/<id>.jsonl 与 <id>/
    let projects = data_root.join("projects");
    if is_managed_dir(&projects) {
        let entries =
            std::fs::read_dir(&projects).map_err(|e| format!("读取 projects 目录失败: {}", e))?;
        for entry in entries.flatten() {
            let project_root = entry.path();
            if !is_managed_dir(&project_root) {
                continue;
            }
            for leaf in [format!("{}.jsonl", id), id.to_string()] {
                match remove_managed_path(&project_root, &leaf) {
                    Ok(true) => removed += 1,
                    Ok(false) => {}
                    Err(e) => failures.push(e),
                }
            }
        }
    }

    for parent in [
        data_root.join("workspace").join("sessions"),
        data_root.join("tasks"),
        data_root.join("file-history"),
    ] {
        if !is_managed_dir(&parent) {
            continue;
        }
        match remove_managed_path(&parent, id) {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(e) => failures.push(e),
        }
    }

    let artifact = data_root.join("artifact-index");
    if is_managed_dir(&artifact) {
        match remove_managed_path(&artifact, &format!("{}.json", id)) {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(e) => failures.push(e),
        }
    }

    if failures.is_empty() {
        Ok(removed)
    } else {
        Err(failures.join("; "))
    }
}

/// 会话缓存条目里标识会话的字段名：客户端不同版本用过不同名字。
/// 只认 `conversationId` 时，字段名一换就「一条都匹配不上」→ 静默返回 0。
pub(super) const CACHE_ID_FIELDS: [&str; 3] = ["conversationId", "sessionId", "id"];

pub(super) fn cache_entry_id(entry: &Value) -> Option<&str> {
    CACHE_ID_FIELDS
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str))
}

/// 从 `<data_root>/app/sessions.json` 移除被删会话的缓存条目，返回移除条数。
///
/// 客户端把会话清单缓存在这里；只删库行不清缓存时，列表会被缓存"复活"。
/// 写回前再读一次原文比对（TOCTOU）—— 客户端可能在这两步之间又写了新缓存，
/// 那就必须报错让用户重试，不能覆盖它刚写的内容。
pub(super) fn remove_workbuddy_session_cache(
    data_root: &Path,
    ids: &[String],
) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let cache_path = data_root.join("app").join("sessions.json");
    let metadata = match std::fs::symlink_metadata(&cache_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("读取会话缓存失败: {}", e)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("会话缓存必须是普通文件".to_string());
    }
    let original =
        std::fs::read_to_string(&cache_path).map_err(|e| format!("读取会话缓存失败: {}", e))?;
    let mut data: Value =
        serde_json::from_str(&original).map_err(|_| "会话缓存无法解析，请稍后重试".to_string())?;
    let entries: Vec<Value> = if let Some(array) = data.as_array() {
        array.clone()
    } else if let Some(array) = data.get("sessions").and_then(Value::as_array) {
        array.clone()
    } else {
        return Err("会话缓存格式不受支持，未改写缓存".to_string());
    };
    let mut unrecognized = 0usize;
    let kept: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            let Some(cid) = cache_entry_id(entry) else {
                unrecognized += 1;
                return true;
            };
            !ids.iter().any(|id| id == cid)
        })
        .cloned()
        .collect();
    // 所有条目都没有可识别的 id 字段 → 缓存格式与预期不符。此时返回 Ok(0) 等于
    // 「缓存里没有这条会话」，上层照常删数据库行，会话下次又会被缓存带回来。
    if !entries.is_empty() && unrecognized == entries.len() {
        return Err(format!(
            "会话缓存格式与预期不符（条目里找不到 {} 任一字段），未改写缓存",
            CACHE_ID_FIELDS.join(" / ")
        ));
    }
    let removed = entries.len() - kept.len();
    if removed == 0 {
        return Ok(0);
    }
    if let Some(array) = data.as_array_mut() {
        *array = kept;
    } else if let Some(object) = data.as_object_mut() {
        object.insert("sessions".to_string(), Value::Array(kept));
    }
    let serialized =
        serde_json::to_string_pretty(&data).map_err(|e| format!("序列化会话缓存失败: {}", e))?;
    let current =
        std::fs::read_to_string(&cache_path).map_err(|e| format!("读取会话缓存失败: {}", e))?;
    if current != original {
        return Err("会话缓存已变化，请重试删除".to_string());
    }
    crate::commands::buddy::store::write_atomic(&cache_path, &serialized)?;
    Ok(removed)
}

/// 目录必须是真目录：存在、是目录、且不是符号链接。
pub(super) fn is_managed_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.is_dir() && !m.file_type().is_symlink())
        .unwrap_or(false)
}

/// 删除 `parent/leaf`：不跟随符号链接（是链接就只删链接）。
/// `Ok(true)` = 删掉了，`Ok(false)` = 本就不存在。
pub(super) fn remove_managed_path(parent: &Path, leaf: &str) -> Result<bool, String> {
    let target = parent.join(leaf);
    let metadata = match std::fs::symlink_metadata(&target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("读取 {} 失败: {}", target.display(), e)),
    };
    let result = if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(&target)
    } else {
        std::fs::remove_dir_all(&target)
    };
    result
        .map(|_| true)
        .map_err(|e| format!("删除 {} 失败: {}", target.display(), e))
}

/// vscdb：删除 `session:<id>` 键（可写事务 + busy_timeout，客户端运行时可并发）。
pub(super) fn delete_vscdb_session_keys(
    db_path: &Path,
    ids: &[String],
    report: &mut BuddySessionDeleteReport,
) {
    if let Err(e) =
        crate::commands::buddy::session_transfer::codebuddy::reject_symlink_if_exists(db_path)
    {
        report.errors.push(e);
        return;
    }
    let mut conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            report.errors.push(format!("打开会话数据库失败: {}", e));
            return;
        }
    };
    if let Err(e) = conn.busy_timeout(Duration::from_secs(5)) {
        report.errors.push(format!("设置数据库超时失败: {}", e));
        return;
    }
    let transaction = match conn.transaction() {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(format!("开启数据库事务失败: {}", e));
            return;
        }
    };
    for id in ids {
        let key = format!("session:{}", id);
        match transaction.execute(
            "DELETE FROM ItemTable WHERE key = ?1",
            rusqlite::params![key],
        ) {
            Ok(n) => report.db_deleted += n,
            Err(e) => report
                .errors
                .push(format!("删除会话记录失败 {}: {}", id, e)),
        }
    }
    if let Err(e) = transaction.commit() {
        report.errors.push(format!("提交数据库删除失败: {}", e));
    }
}

/// workbuddy.db：删除 sessions 表行（5.x 主存储）。
pub(super) fn delete_workbuddy_db_rows(
    db_path: &Path,
    ids: &[String],
    report: &mut BuddySessionDeleteReport,
) {
    if let Err(e) =
        crate::commands::buddy::session_transfer::workbuddy::reject_symlink_if_exists(db_path)
    {
        report.errors.push(e);
        return;
    }
    let mut conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            report
                .errors
                .push(format!("打开 WorkBuddy 数据库失败: {}", e));
            return;
        }
    };
    if let Err(e) = conn.busy_timeout(Duration::from_secs(5)) {
        report.errors.push(format!("设置数据库超时失败: {}", e));
        return;
    }
    let has_sessions_table: bool = match conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions')",
        [],
        |row| row.get(0),
    ) {
        Ok(v) => v,
        Err(e) => {
            report.errors.push(format!("检查数据库结构失败: {}", e));
            return;
        }
    };
    if !has_sessions_table {
        return;
    }
    let transaction = match conn.transaction() {
        Ok(t) => t,
        Err(e) => {
            report.errors.push(format!("开启数据库事务失败: {}", e));
            return;
        }
    };
    for id in ids {
        match transaction.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id]) {
            Ok(n) => report.db_deleted += n,
            Err(e) => report
                .errors
                .push(format!("删除会话记录失败 {}: {}", id, e)),
        }
    }
    if let Err(e) = transaction.commit() {
        report.errors.push(format!("提交数据库删除失败: {}", e));
    }
}
