//! Buddy 模块 WorkBuddy 会话跨账号合并入口。
//!
//! 移植自 cockpit-tools `modules/workbuddy_session_transfer.rs`：
//! - WorkBuddy 5.x 的会话主存储是 `{config_dir}/workbuddy.db`（按 `user_id` 区分），
//!   因此**无论是否存在旧版扩展会话目录**，都必须执行数据库层 user_id 重映射
//!   （这才是 5.x 真正生效的会话合并）。
//! - 旧版 per-uid 目录（WorkBuddyExtension / CodeBuddyExtension）的 history 合并
//!   复用 codebuddy.rs 的共享引擎；旧版 `codebuddy-sessions.vscdb` 重映射同样复用。

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use super::super::models::{BuddyAccount, BuddyPlatform};
use super::super::store;
use super::codebuddy;
use super::SessionTransferReport;
use super::TransferProgress;
use crate::commands::buddy::emit_switch_progress;

/// 会话备份目录的分类名（`{data_dir}/buddy/session-backup/<本值>/<uid>`）
const BACKUP_PLATFORM_LABEL: &str = "workbuddy";

/// 进程级互斥：同一时刻只允许一次 WorkBuddy 会话合并
static TRANSFER_LOCK: LazyLock<std::sync::Mutex<()>> = LazyLock::new(|| std::sync::Mutex::new(()));

/// 在写入目标账号 auth 文件之前，合并来源账号的本地会话到目标账号。
pub fn transfer_on_switch(
    target: &BuddyAccount,
    progress: TransferProgress,
) -> Result<Option<SessionTransferReport>, String> {
    let uid = target
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "目标 WorkBuddy 账号缺少 uid，无法执行会话合并".to_string())?;

    let current = match buddy_current_account(BuddyPlatform::Workbuddy, &[target.id.clone()]) {
        Some(current) => current,
        None => return Ok(None),
    };
    let source_uid = current
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(source_uid) = source_uid else {
        return Ok(None);
    };
    if source_uid == uid {
        return Ok(None);
    }

    emit_switch_progress(progress.app, progress.platform, progress.account_id, "merging", 0, None);
    let runtime_dir = runtime_dir_for_workbuddy()?;
    let report = transfer_local_sessions(&runtime_dir, source_uid, uid, progress)?;
    Ok(Some(report))
}

fn buddy_current_account(platform: BuddyPlatform, except_ids: &[String]) -> Option<BuddyAccount> {
    let accounts = store::list_accounts(platform);
    let current_id = store::get_current_account_id(platform);
    for account in &accounts {
        if current_id.as_deref() == Some(account.id.as_str()) {
            return Some(account.clone());
        }
    }
    let output = match platform {
        BuddyPlatform::Workbuddy => super::super::workbuddy::resolve_current_account_id(&accounts),
        BuddyPlatform::CodebuddyCn => super::super::codebuddy_cn::resolve_current_account_id(&accounts),
    };
    output.filter(|id| !except_ids.iter().any(|except| except == id))
        .and_then(|id| accounts.iter().find(|account| account.id == id).cloned())
}

fn runtime_dir_for_workbuddy() -> Result<std::path::PathBuf, String> {
    let data_dir = data_dir_for_workbuddy()?;
    let app_dir = data_dir.join("app");
    Ok(if app_dir.is_dir() {
        app_dir
    } else {
        data_dir
    })
}

fn data_dir_for_workbuddy() -> Result<std::path::PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".workbuddy"))
        .ok_or_else(|| "无法确定 WorkBuddy 数据目录".to_string())
}

/// 解析 WorkBuddy 运行时目录：路径以 app 结尾 → (父目录=配置目录, 该目录=Electron 数据目录)；
/// 否则 → (该目录=配置目录, 子目录 app=Electron 数据目录)。复刻 resolve_workbuddy_runtime_dirs。
fn resolve_runtime_dirs(
    runtime_dir: &Path,
) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let raw = runtime_dir.to_string_lossy().trim().to_string();
    if raw.is_empty() {
        return Err("WorkBuddy 实例目录为空".to_string());
    }
    let path = std::path::PathBuf::from(raw);
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("app"))
    {
        let config_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(std::path::PathBuf::from)
            .ok_or_else(|| "无法从 Electron 目录推导 WorkBuddy 配置目录".to_string())?;
        return Ok((config_dir, path));
    }
    Ok((path.clone(), path.join("app")))
}

fn transfer_local_sessions(
    runtime_dir: &Path,
    source_uid: &str,
    target_uid: &str,
    progress: TransferProgress,
) -> Result<SessionTransferReport, String> {
    codebuddy::validate_uid(source_uid)
        .map_err(|e| e.replace("CodeBuddy CN", "WorkBuddy"))?;
    codebuddy::validate_uid(target_uid)
        .map_err(|e| e.replace("CodeBuddy CN", "WorkBuddy"))?;
    if source_uid == target_uid {
        return Ok(SessionTransferReport::default());
    }

    let _guard = TRANSFER_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 本地会话合并正在进行，请稍后重试".to_string())?;

    let (config_dir, electron_data_dir) = resolve_runtime_dirs(runtime_dir)?;
    let backup_root = super::prepare_backup_root(BACKUP_PLATFORM_LABEL, target_uid)?;

    let mut report = SessionTransferReport::default();

    // 1) 旧版 per-uid 扩展目录合并（5.x 通常不存在，仅兼容旧版落盘结构）
    let extension_roots = select_extension_data_roots(source_uid)?;
    if extension_roots.is_empty() {
        eprintln!(
            "[Buddy WB Transfer] 未找到来源账号本地会话目录（uid={}），跳过目录级合并，继续数据库层合并",
            source_uid
        );
    } else {
        for (label, extension_data_dir) in extension_roots {
            let file_report = codebuddy::sync_history_between_accounts(
                &extension_data_dir,
                source_uid,
                target_uid,
                &backup_root.join(label),
                progress,
            )?;
            report.added_conversations += file_report.added_conversations;
            report.replaced_conversations += file_report.replaced_conversations;
            report.scanned_workspaces += file_report.scanned_workspaces;
        }
    }

    // 2) WorkBuddy 5.x 主存储：workbuddy.db 按 user_id 过滤 —— 无条件执行
    report.updated_session_rows += remap_workbuddy_database_user_id(
        &config_dir.join("workbuddy.db"),
        target_uid,
        &backup_root.join("database"),
    )?;

    // 3) 旧版 vscdb（若存在）—— 无条件执行，取第一个命中的
    for legacy_db in [
        electron_data_dir.join("codebuddy-sessions.vscdb"),
        config_dir.join("codebuddy-sessions.vscdb"),
    ] {
        if !legacy_db.is_file() {
            continue;
        }
        report.updated_session_rows += codebuddy::remap_session_vscdb_user_id(
            &legacy_db,
            source_uid,
            target_uid,
            &backup_root.join("legacy-database"),
        )?;
        break;
    }

    eprintln!(
        "[Buddy WB Transfer] 合并完成: source_uid={}, target_uid={}, workspaces={}, added={}, replaced={}, db_rows={}",
        source_uid,
        target_uid,
        report.scanned_workspaces,
        report.added_conversations,
        report.replaced_conversations,
        report.updated_session_rows
    );

    Ok(report)
}

pub(crate) fn select_extension_data_roots(
    source_uid: &str,
) -> Result<Vec<(&'static str, std::path::PathBuf)>, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    #[cfg(target_os = "windows")]
    let base: std::path::PathBuf = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Local"));
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support");
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let base = home.join(".local").join("share");

    let current = base.join("WorkBuddyExtension").join("Data");
    if account_has_history(&current, source_uid)? {
        return Ok(vec![("workbuddy-extension", current)]);
    }
    let legacy = base.join("CodeBuddyExtension").join("Data");
    if account_has_history(&legacy, source_uid)? {
        return Ok(vec![("legacy-codebuddy-extension", legacy)]);
    }
    Ok(Vec::new())
}

fn account_has_history(extension_data_dir: &Path, uid: &str) -> Result<bool, String> {
    let account_root = extension_data_dir.join(uid);
    if !account_root.is_dir() {
        return Ok(false);
    }
    reject_symlink_if_exists(&account_root)?;
    for entry in std::fs::read_dir(&account_root).map_err(|e| {
        format!(
            "读取 WorkBuddy 会话根目录失败: path={}, error={}",
            account_root.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("读取 WorkBuddy 会话目录失败: {}", e))?;
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|e| format!("读取 WorkBuddy 会话目录属性失败: {}", e))?;
        if metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && entry.path().join(uid).join("history").is_dir()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(format!(
            "拒绝通过符号链接读写 WorkBuddy 会话: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "读取 WorkBuddy 会话路径属性失败: path={}, error={}",
            path.display(),
            e
        )),
    }
}

/// WorkBuddy 5.x 主存储重映射：`sessions` 表全部未删除会话的 user_id 改为目标账号。
fn remap_workbuddy_database_user_id(
    db_path: &Path,
    target_uid: &str,
    backup_root: &Path,
) -> Result<usize, String> {
    if !db_path.is_file() {
        return Ok(0);
    }
    reject_symlink_if_exists(db_path)?;
    let mut connection = rusqlite::Connection::open(db_path).map_err(|e| {
        format!(
            "打开 WorkBuddy 会话数据库失败: path={}, error={}",
            db_path.display(),
            e
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置 WorkBuddy 会话数据库超时失败: {}", e))?;

    let has_sessions_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("检查 WorkBuddy 会话数据库结构失败: {}", e))?;
    if !has_sessions_table {
        return Ok(0);
    }

    let update_count: usize = connection
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE user_id != ?1 AND deleted_at IS NULL",
            [target_uid],
            |row| row.get(0),
        )
        .map_err(|e| format!("读取 WorkBuddy 会话索引失败: {}", e))?;
    if update_count == 0 {
        return Ok(0);
    }

    std::fs::create_dir_all(backup_root).map_err(|e| {
        format!(
            "创建 WorkBuddy 会话数据库备份目录失败: path={}, error={}",
            backup_root.display(),
            e
        )
    })?;
    backup_sqlite_database(db_path, backup_root)?;

    let transaction = connection
        .transaction()
        .map_err(|e| format!("开启 WorkBuddy 会话数据库事务失败: {}", e))?;
    let changed = transaction
        .execute(
            "UPDATE sessions SET user_id = ?1 WHERE user_id != ?1 AND deleted_at IS NULL",
            [target_uid],
        )
        .map_err(|e| format!("更新 WorkBuddy 会话 user_id 失败: {}", e))?;
    transaction
        .commit()
        .map_err(|e| format!("提交 WorkBuddy 会话数据库事务失败: {}", e))?;
    Ok(changed)
}

/// 备份 sqlite 主库 + -wal + -shm（WAL 模式下未 checkpoint 的数据在 -wal 里）。
fn backup_sqlite_database(db_path: &Path, backup_root: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let source = if suffix.is_empty() {
            db_path.to_path_buf()
        } else {
            let mut path = db_path.as_os_str().to_os_string();
            path.push(suffix);
            std::path::PathBuf::from(path)
        };
        if !source.is_file() {
            continue;
        }
        reject_symlink_if_exists(&source)?;
        let file_name = source
            .file_name()
            .ok_or_else(|| format!("无法解析 WorkBuddy 数据库文件名: {}", source.display()))?;
        std::fs::copy(&source, backup_root.join(file_name)).map_err(|e| {
            format!(
                "备份 WorkBuddy 会话数据库失败: path={}, error={}",
                source.display(),
                e
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kira-workbuddy-transfer-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            id
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn rejects_unsafe_uids() {
        for uid in ["", "../a", "a/b", "a\\b", "a..b", " a"] {
            assert!(codebuddy::validate_uid(uid).is_err(), "uid should be rejected: {uid}");
        }
        assert!(codebuddy::validate_uid("384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    #[test]
    fn remaps_all_active_sessions_and_preserves_deleted_sessions() {
        let dest = make_temp();
        let db_path = dest.join("workbuddy.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    cwd TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    deleted_at INTEGER
                );
                INSERT INTO sessions VALUES ('a', '/', 'source', 'Done', 1, 2, NULL);
                INSERT INTO sessions VALUES ('b', '/', 'other', 'Done', 1, 2, NULL);
                INSERT INTO sessions VALUES ('c', '/', 'source', 'Done', 1, 2, 3);
                INSERT INTO sessions VALUES ('d', '/', 'target', 'Done', 1, 2, NULL);",
            )
            .unwrap();
            // 制造一个 wal 文件，验证备份包含 wal
            std::fs::write(db_path.with_extension("db-wal"), b"wal-bytes").unwrap();
        }

        let backup_root = dest.join("backup");
        let changed =
            remap_workbuddy_database_user_id(&db_path, "target", &backup_root).unwrap();
        assert_eq!(changed, 2);

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let active_non_target: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE deleted_at IS NULL AND user_id != 'target'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let deleted_user: String = conn
            .query_row("SELECT user_id FROM sessions WHERE id = 'c'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(active_non_target, 0);
        assert_eq!(deleted_user, "source");
        assert!(backup_root.join("workbuddy.db").is_file());
        assert!(backup_root.join("workbuddy.db-wal").is_file());
    }

    #[test]
    fn missing_database_is_a_noop() {
        let dest = make_temp();
        assert_eq!(
            remap_workbuddy_database_user_id(
                &dest.join("missing.db"),
                "target",
                &dest.join("backup")
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn resolve_runtime_dirs_maps_app_leaf_to_parent_config() {
        let dest = make_temp();
        let app_dir = dest.join("app");
        std::fs::create_dir_all(&app_dir).unwrap();
        let (config, electron) = resolve_runtime_dirs(&app_dir).unwrap();
        assert_eq!(config, dest);
        assert_eq!(electron, app_dir);

        let (config, electron) = resolve_runtime_dirs(&dest).unwrap();
        assert_eq!(config, dest);
        assert_eq!(electron, dest.join("app"));
    }
}
