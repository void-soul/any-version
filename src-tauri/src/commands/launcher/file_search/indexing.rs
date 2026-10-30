use super::*;

/// 路径后缀匹配，要求落在目录边界上（`.../temp` 而不是 `.../attempt`）。
pub(super) fn ends_with_path(rel: &str, suffix: &str) -> bool {
    match rel.strip_suffix(suffix) {
        Some(prefix) => prefix.is_empty() || prefix.ends_with('/'),
        None => false,
    }
}

/// 名字含 cache / session 的目录按缓存处理（含 `webpack-cache`、`sessions` 等变体）。
pub(super) fn has_cache_word(name_lower: &str) -> bool {
    name_lower.contains("cache") || name_lower.contains("session")
}

/// GUID（8-4-4-4-12）或 ≥16 位纯十六进制串：缓存键、提交哈希这类不可读标识。
pub(super) fn is_opaque_id(name_lower: &str) -> bool {
    let bytes = name_lower.as_bytes();
    let guid = bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && name_lower
            .chars()
            .filter(|c| *c != '-')
            .all(|c| c.is_ascii_hexdigit());
    guid || (bytes.len() >= 16 && bytes.iter().all(|b| b.is_ascii_hexdigit()))
}

/// 该目录是否需要整棵剪枝（`rel` 为相对盘根的小写路径）。
pub(super) fn is_skipped_dir(rel: &str, name_lower: &str) -> bool {
    if SKIP_DIR_NAMES.iter().any(|s| *s == name_lower) {
        return true;
    }
    if has_cache_word(name_lower) || is_opaque_id(name_lower) {
        return true;
    }
    SKIP_REL_SUFFIXES.iter().any(|s| ends_with_path(rel, s))
}

/// 相对盘根的路径，统一成 `/` 分隔并转小写（.gitignore 规则按大小写不敏感匹配）。
pub(super) fn relative_slash(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/").to_lowercase())
        .unwrap_or_default()
}

pub(super) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(super) fn emit_progress(
    app: &tauri::AppHandle,
    drive: &str,
    scanned: usize,
    inserted: usize,
    current: &str,
    elapsed_ms: i64,
    done: bool,
    canceled: bool,
    error: Option<String>,
) {
    let payload = IndexProgress {
        drive: drive.to_string(),
        scanned,
        inserted,
        current: current.to_string(),
        elapsed_ms,
        done,
        canceled,
        error,
    };
    let _ = app.emit(PROGRESS_EVENT, payload);
}

pub(super) fn flush_batch(
    conn: &mut Connection,
    drive: &str,
    batch: &mut Vec<BatchRow>,
) -> Result<usize, String> {
    if batch.is_empty() {
        return Ok(0);
    }
    let tx = conn
        .transaction()
        .map_err(|e| format!("开启索引事务失败: {}", e))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO fs_entry_tmp
                 (drive, path, name, name_lower, is_dir, size, modified)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )
            .map_err(|e| e.to_string())?;
        for (path, name, name_lower, is_dir, size, modified) in batch.iter() {
            stmt.execute(params![
                drive, path, name, name_lower, is_dir, size, modified
            ])
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit()
        .map_err(|e| format!("提交索引批次失败: {}", e))?;
    let n = batch.len();
    batch.clear();
    Ok(n)
}

pub(super) fn scan_drive(
    app: &tauri::AppHandle,
    drive: &str,
    mode: &str,
    respect_gitignore: bool,
) -> Result<ScanOutcome, String> {
    let mut conn = build_connection()?;
    conn.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
        .map_err(|e| e.to_string())?;
    if mode == "rebuild" {
        conn.execute(
            "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
            params![drive],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_entry WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
    }

    let root = format!("{}\\", drive);
    let root_path = Path::new(&root);
    let started = Instant::now();
    let mut scanned = 0usize;
    let mut inserted = 0usize;
    let mut batch: Vec<BatchRow> = Vec::with_capacity(2048);
    let mut last_emit = Instant::now();
    let mut ignores = IgnoreStack::new();

    let mut walker = walkdir::WalkDir::new(&root).follow_links(false).into_iter();

    while let Some(res) = walker.next() {
        if CANCEL_INDEX.load(Ordering::SeqCst) {
            let _ = flush_batch(&mut conn, drive, &mut batch);
            let _ = conn.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive]);
            return Ok(ScanOutcome::Canceled { scanned });
        }
        let entry = match res {
            Ok(e) => e,
            Err(_) => continue,
        };
        let depth = entry.depth();
        // 离开更深的目录后，丢弃那一层装载的 .gitignore 规则
        ignores.leave_to(depth);
        if depth == 0 {
            if respect_gitignore {
                ignores.enter(0, entry.path(), "");
            }
            continue;
        }
        // 不跟随 junction / 符号链接，避免目录环与重复项。
        if entry.file_type().is_symlink() {
            continue;
        }
        let is_dir = entry.file_type().is_dir();
        let file_name = entry.file_name().to_string_lossy();
        if file_name.is_empty() {
            continue;
        }
        let name_lower = file_name.to_ascii_lowercase();
        // 目录要判缓存/系统路径，文件只在遵循 gitignore 时才需要相对路径
        let rel = if is_dir || respect_gitignore {
            relative_slash(entry.path(), root_path)
        } else {
            String::new()
        };
        if is_dir && is_skipped_dir(&rel, &name_lower) {
            walker.skip_current_dir();
            continue;
        }
        // 纯 GUID / 长哈希文件名（无扩展名的缓存键）同样没有检索价值
        if !is_dir && is_opaque_id(&name_lower) {
            continue;
        }
        if respect_gitignore && ignores.is_ignored(&rel, is_dir) {
            if is_dir {
                walker.skip_current_dir();
            }
            continue;
        }
        if is_dir && respect_gitignore {
            ignores.enter(depth, entry.path(), &rel);
        }

        let name = file_name.to_string();
        let meta = entry.metadata().ok();
        let size = if is_dir {
            0
        } else {
            meta.as_ref().map(|m| m.len() as i64).unwrap_or(0)
        };
        let modified = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let path = entry.path().to_string_lossy().to_string();
        batch.push((
            path,
            name.clone(),
            name.to_lowercase(),
            if is_dir { 1 } else { 0 },
            size,
            modified,
        ));
        scanned += 1;
        if batch.len() >= 2048 {
            inserted += flush_batch(&mut conn, drive, &mut batch)?;
        }

        if last_emit.elapsed() >= Duration::from_millis(300) {
            last_emit = Instant::now();
            emit_progress(
                app,
                drive,
                scanned,
                inserted + batch.len(),
                &entry.path().to_string_lossy(),
                started.elapsed().as_millis() as i64,
                false,
                false,
                None,
            );
        }
    }

    inserted += flush_batch(&mut conn, drive, &mut batch)?;

    // 全文索引的写入比普通行慢得多，单独提示一次，避免进度条停在 99% 看似卡死。
    emit_progress(
        app,
        drive,
        scanned,
        inserted,
        "正在写入全文索引…",
        started.elapsed().as_millis() as i64,
        false,
        false,
        None,
    );

    // 一次性替换：旧索引在扫描期间始终可搜（update 模式）。
    let tx = conn
        .transaction()
        .map_err(|e| format!("替换索引事务失败: {}", e))?;
    tx.execute(
        "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
        params![drive],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM fs_entry WHERE drive = ?1", params![drive])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO fs_entry (drive, path, name, name_lower, is_dir, size, modified)
         SELECT drive, path, name, name_lower, is_dir, size, modified
         FROM fs_entry_tmp WHERE drive = ?1",
        params![drive],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO fs_entry_fts (rowid, name)
         SELECT id, name FROM fs_entry WHERE drive = ?1",
        params![drive],
    )
    .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| format!("提交索引失败: {}", e))?;

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fs_entry WHERE drive = ?1",
            params![drive],
            |row| row.get(0),
        )
        .unwrap_or(inserted as i64);

    set_meta(
        &conn,
        drive,
        "ready",
        count,
        now_secs(),
        started.elapsed().as_millis() as i64,
        respect_gitignore,
    )?;

    Ok(ScanOutcome::Done { scanned, count })
}

#[tauri::command]
pub async fn file_search_list_drives() -> Result<Vec<DriveInfo>, String> {
    let mut meta_map: std::collections::HashMap<String, (String, i64, i64, i64, bool)> =
        std::collections::HashMap::new();
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT drive, status, item_count, indexed_at, elapsed_ms, respect_gitignore
                 FROM fs_meta",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)? != 0,
                ))
            })
            .map_err(|e| e.to_string())?;
        for (drive, status, count, indexed_at, elapsed, respect) in rows.flatten() {
            meta_map.insert(drive, (status, count, indexed_at, elapsed, respect));
        }
        Ok(())
    })?;

    let building = BUILDING_DRIVE.lock().map(|g| g.clone()).unwrap_or(None);

    let mut out: Vec<DriveInfo> = windows_drives()
        .into_iter()
        .map(|(letter, drive_type, total, free)| {
            let (status, item_count, indexed_at, elapsed_ms, respect_gitignore) = meta_map
                .get(&letter)
                .cloned()
                .unwrap_or_else(|| ("idle".to_string(), 0, 0, 0, false));
            let status = if building.as_deref() == Some(letter.as_str()) {
                "building".to_string()
            } else {
                status
            };
            DriveInfo {
                root: format!("{}\\", letter),
                letter,
                drive_type,
                total_bytes: total,
                free_bytes: free,
                status,
                item_count,
                indexed_at,
                elapsed_ms,
                respect_gitignore,
            }
        })
        .collect();
    out.sort_by(|a, b| a.letter.cmp(&b.letter));
    Ok(out)
}

#[tauri::command]
pub async fn file_search_start_index(
    app: tauri::AppHandle,
    drive: String,
    mode: Option<String>,
    respect_gitignore: Option<bool>,
) -> Result<(), String> {
    let drive = normalize_drive(&drive)?;
    let root = format!("{}\\", drive);
    if !Path::new(&root).exists() {
        return Err(format!("盘符不存在: {}", root));
    }
    let mode = mode.unwrap_or_else(|| "update".to_string());
    let respect_gitignore = respect_gitignore.unwrap_or(true);
    {
        let mut guard = BUILDING_DRIVE
            .lock()
            .map_err(|e| format!("索引状态锁错误: {}", e))?;
        if guard.is_some() {
            return Err("已有索引任务正在进行，请先停止".to_string());
        }
        *guard = Some(drive.clone());
    }
    CANCEL_INDEX.store(false, Ordering::SeqCst);
    set_status(&drive, "building");

    let app_for_thread = app.clone();
    let drive_for_thread = drive.clone();
    std::thread::spawn(move || {
        let result = scan_drive(&app_for_thread, &drive_for_thread, &mode, respect_gitignore);
        {
            if let Ok(mut guard) = BUILDING_DRIVE.lock() {
                *guard = None;
            }
        }
        match result {
            Ok(ScanOutcome::Done { scanned, count }) => {
                emit_progress(
                    &app_for_thread,
                    &drive_for_thread,
                    scanned,
                    count as usize,
                    "",
                    0,
                    true,
                    false,
                    None,
                );
            }
            Ok(ScanOutcome::Canceled { scanned }) => {
                restore_status(&drive_for_thread);
                emit_progress(
                    &app_for_thread,
                    &drive_for_thread,
                    scanned,
                    0,
                    "",
                    0,
                    true,
                    true,
                    None,
                );
            }
            Err(err) => {
                cleanup_tmp(&drive_for_thread);
                restore_status(&drive_for_thread);
                emit_progress(
                    &app_for_thread,
                    &drive_for_thread,
                    0,
                    0,
                    "",
                    0,
                    true,
                    false,
                    Some(err),
                );
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn file_search_cancel_index() -> Result<(), String> {
    CANCEL_INDEX.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn file_search_delete_index(drive: String) -> Result<(), String> {
    let drive = normalize_drive(&drive)?;
    with_conn(|conn| {
        conn.execute(
            "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
            params![drive],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_entry WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM fs_meta WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}
