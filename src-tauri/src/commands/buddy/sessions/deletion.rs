use super::*;

// ─── 删除会话（数据库记录 + 本地文件） ───

/// 删除会话的结果报告。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySessionDeleteReport {
    /// 数据库中删除的记录数（vscdb 键 / sessions 行）
    pub db_deleted: usize,
    /// 删除的 history 会话目录数
    pub history_dirs_removed: usize,
    /// 删除的辅助目录数（check-point 等 5 类）
    pub auxiliary_dirs_removed: usize,
    /// WorkBuddy 5.x 删除的正文项数（`projects/<hash>/<id>.jsonl` 与 `<id>/`、工作区快照、
    /// tasks、file-history、artifact-index）
    pub files_removed: usize,
    /// 非致命错误（如文件被客户端占用），不中断整体删除
    pub errors: Vec<String>,
}

/// 删除指定会话：数据库记录 + 本地会话文件（**正文一起删，不留孤儿**）。
///
/// 顺序对齐 WorkDaddy `/api/sessions/delete`：**先删可重试的文件，最后删数据库记录**。
/// - WorkBuddy 5.x：`projects/<hash>/<id>.jsonl` 与 `<id>/`、`workspace/sessions/<id>`、
///   `tasks/<id>`、`file-history/<id>`、`artifact-index/<id>.json`，外加 `app/sessions.json`
///   里的列表缓存条目（不清缓存，客户端会把刚删的会话"复活"）；
/// - CodeBuddy CN：`<uid>/<IDE>/<uid>/history/<workspace>/<id>/` 与 5 类辅助目录 + index.json 条目。
///
/// 文件删不掉时**保留该会话的数据库记录**（它是重试锚点），错误进 `errors` 由界面展示。
/// 调用方（`buddy_delete_sessions`）会先占切号闸，避免会话同步 worker 把刚删的行写回。
pub fn delete_sessions(
    platform: &str,
    conversation_ids: &[String],
) -> Result<BuddySessionDeleteReport, String> {
    let ids: Vec<String> = conversation_ids
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if ids.is_empty() {
        return Err("未指定要删除的会话".to_string());
    }
    for id in &ids {
        crate::commands::buddy::session_transfer::codebuddy::validate_conversation_id(id)?;
    }
    match platform {
        "workbuddy" => delete_workbuddy_sessions(&ids),
        "codebuddy-cn" | "codebuddy_cn" => delete_codebuddy_sessions(&ids),
        other => Err(format!("未知平台: {}", other)),
    }
}

/// 当前登录账号的 uid（会话文件按 uid 落盘）。
pub(super) fn current_platform_uid(
    platform: crate::commands::buddy::models::BuddyPlatform,
) -> Option<String> {
    let accounts = crate::commands::buddy::store::list_accounts(platform);
    let current_id = crate::commands::buddy::store::get_current_account_id(platform).or_else(
        || match platform {
            crate::commands::buddy::models::BuddyPlatform::Workbuddy => {
                crate::commands::buddy::workbuddy::resolve_current_account_id(platform, &accounts)
            }
            crate::commands::buddy::models::BuddyPlatform::CodebuddyCn => {
                crate::commands::buddy::codebuddy_cn::resolve_current_account_id(&accounts)
            }
        },
    )?;
    accounts
        .iter()
        .find(|a| a.id == current_id)
        .and_then(|a| a.uid.clone())
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
}

pub(super) fn delete_codebuddy_sessions(
    ids: &[String],
) -> Result<BuddySessionDeleteReport, String> {
    let user_data_dir = crate::commands::buddy::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())?;
    let extension_data_dir =
        crate::commands::buddy::session_transfer::codebuddy::codebuddy_extension_data_dir().ok();
    let mut report = BuddySessionDeleteReport::default();
    delete_codebuddy_sessions_at(
        &user_data_dir,
        extension_data_dir.as_deref(),
        ids,
        &mut report,
    );
    Ok(report)
}

/// CN 删除核心（路径参数化以便测试）。
///
/// 顺序对齐 WorkDaddy `daemon.js` `/api/sessions/delete`：**先删可重试的正文文件，最后删数据库键**。
/// 正文没删干净就保留记录 —— 记录是重试锚点，否则会留下"列表里没了、磁盘上还在"的孤儿正文。
///
/// **必须扫描所有账号目录，不能只删当前账号**（2026-10-19 真机实证）：会话合并会把同一条会话复制进
/// 多个账号目录（实测一条 `e:/pro/my/life` 的会话在 14 个账号目录里都有 `history/<hash>/<id>/` 与
/// index.json 条目）。只删当前账号，另外 13 份会在切到那些账号后"复活"。
/// 这就是 WorkDaddy `collectLineageMembersForDelete` 的等价物 —— 我们的同源副本不是 `meta.autoCopy`
/// 链，而是"同一 id 落在多个账号目录"。
pub(super) fn delete_codebuddy_sessions_at(
    user_data_dir: &Path,
    extension_data_dir: Option<&Path>,
    ids: &[String],
    report: &mut BuddySessionDeleteReport,
) {
    let files_ok = match extension_data_dir {
        Some(dir) => delete_session_files_in_extension(dir, ids, report),
        None => {
            // 定位不到扩展数据目录时，一行正文都没删 —— 不能当成删干净了
            report.errors.push(
                "未能定位 CodeBuddy 扩展数据目录，会话正文未删除，已保留数据库记录以便重试"
                    .to_string(),
            );
            false
        }
    };
    if !files_ok {
        report
            .errors
            .push("会话正文未能全部删除，已保留数据库记录以便重试".to_string());
        return;
    }
    let db_path = user_data_dir.join("codebuddy-sessions.vscdb");
    if db_path.is_file() {
        delete_vscdb_session_keys(&db_path, ids, report);
    }
}
