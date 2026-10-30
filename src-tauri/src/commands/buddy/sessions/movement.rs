use super::*;

/// 改一批会话的所属目录（「转移会话」）的结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BuddySessionMoveReport {
    /// 成功改了目录的会话数
    pub moved: usize,
    /// 搬动的目录数（正文 + 各类辅助目录）
    pub dirs_moved: usize,
    /// 没动成的会话（`id：原因`），界面要原样列出来
    pub skipped: Vec<String>,
    /// 本次备份根目录（要还原就把它整棵放回去）
    pub backup_dir: String,
}

/// 把会话改到另一个项目目录下（**连正文一起搬**）。
///
/// 目录在 CodeBuddy CN 里是**双重绑定**：vscdb 会话记录里的 `cwd`（列表分组用）+
/// 正文所在的 `history/<md5(cwd)>/<id>/`。只改前者，会话会立刻变成「内容缺失」——
/// 删除 / 分叉 / 冲突裁决全都按「cwd 算哈希」找文件，找不到就等于废了。
/// 所以这里两处一起改，并把同步基线、待处理冲突里的工作区维度一起迁过去。
///
/// 备份**整批只准备一次**（`prepare_backup_root` 会清空该 uid 的旧备份，
/// 一条会话调一次会把前一条的备份冲掉），所以本函数接受 ids 数组而不是单个 id。
pub fn move_sessions_to_dir(
    ids: &[String],
    target_dir: &str,
) -> Result<BuddySessionMoveReport, String> {
    use crate::commands::buddy::session_transfer::codebuddy as transfer;

    let target = std::path::Path::new(target_dir.trim());
    if !target.is_dir() {
        return Err(format!("目标目录不存在：{}", target.display()));
    }
    let target_path = transfer::normalize_workspace_path(&target.to_string_lossy());
    let new_workspace = transfer::workspace_hash_for_path(&target_path);

    // 与切号合并互斥（两边动的是同一批目录）
    let _guard = transfer::transfer_lock()
        .lock()
        .map_err(|_| "CodeBuddy CN 会话合并正在进行，请稍后重试".to_string())?;

    let data_dir = crate::commands::buddy::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())?;
    let db_path = data_dir.join("codebuddy-sessions.vscdb");
    if !db_path.is_file() {
        return Err("未找到 CodeBuddy CN 会话库（codebuddy-sessions.vscdb）".to_string());
    }
    transfer::reject_symlink_if_exists(&db_path)?;
    let mut conn =
        Connection::open(&db_path).map_err(|e| format!("打开 CodeBuddy CN 会话库失败: {}", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置数据库超时失败: {}", e))?;

    let uid = current_platform_uid(crate::commands::buddy::models::BuddyPlatform::CodebuddyCn)
        .ok_or_else(|| "未定位到当前 CodeBuddy CN 账号 uid".to_string())?;
    transfer::validate_uid(&uid)?;
    let extension_dir = transfer::codebuddy_extension_data_dir()?;
    let backup_root = crate::commands::buddy::session_transfer::prepare_backup_root(
        transfer::BACKUP_PLATFORM_LABEL,
        &uid,
    )?;

    let mut report = BuddySessionMoveReport {
        backup_dir: backup_root.to_string_lossy().to_string(),
        ..Default::default()
    };
    // (IDE 目录名, 旧工作区哈希, 会话 id)：搬完统一迁基线 / 冲突记录
    let mut remaps: Vec<(String, String, String)> = Vec::new();

    for raw_id in ids {
        let id = raw_id.trim();
        if let Err(e) = transfer::validate_conversation_id(id) {
            report.skipped.push(format!("{}：{}", id, e));
            continue;
        }
        let key = format!("session:{}", id);
        let raw: String = match conn.query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        ) {
            Ok(v) => v,
            Err(_) => {
                report.skipped.push(format!("{}：会话不存在", id));
                continue;
            }
        };
        let mut session: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                report
                    .skipped
                    .push(format!("{}：会话记录解析失败（{}）", id, e));
                continue;
            }
        };
        let old_cwd = session
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if old_cwd.trim().is_empty() {
            report.skipped.push(format!("{}：原会话没有目录信息", id));
            continue;
        }
        let old_workspace = transfer::workspace_hash_for_path(&old_cwd);
        if old_workspace == new_workspace {
            report.skipped.push(format!("{}：已经在这个目录下", id));
            continue;
        }

        match transfer::move_conversation_to_workspace(
            &extension_dir,
            &uid,
            id,
            &old_workspace,
            &new_workspace,
            &backup_root,
        ) {
            Ok(outcome) => {
                if outcome.dirs_moved == 0 {
                    // 正文没找到（多半本来就是「内容缺失」）：索引照改，但如实告诉用户
                    report.skipped.push(format!(
                        "{}：扩展目录里没找到会话正文（目录已改，内容仍缺失）",
                        id
                    ));
                }
                for ide in outcome.ides {
                    remaps.push((ide, old_workspace.clone(), id.to_string()));
                }
                report.dirs_moved += outcome.dirs_moved;
            }
            Err(e) => {
                // 正文没搬成就不改 cwd —— 否则会话会被标成「内容缺失」
                report.skipped.push(format!("{}：{}", id, e));
                continue;
            }
        }

        let Some(object) = session.as_object_mut() else {
            report.skipped.push(format!("{}：会话记录不是对象", id));
            continue;
        };
        object.insert("cwd".to_string(), Value::String(target_path.clone()));
        let serialized = match serde_json::to_string(&session) {
            Ok(s) => s,
            Err(e) => {
                report
                    .skipped
                    .push(format!("{}：序列化会话失败（{}）", id, e));
                continue;
            }
        };
        let tx = conn
            .transaction()
            .map_err(|e| format!("开启数据库事务失败: {}", e))?;
        if let Err(e) = tx.execute(
            "INSERT OR REPLACE INTO ItemTable(key, value) VALUES(?1, ?2)",
            rusqlite::params![key, serialized],
        ) {
            report
                .skipped
                .push(format!("{}：写回会话记录失败（{}）", id, e));
            continue;
        }
        tx.commit()
            .map_err(|e| format!("提交会话记录失败: {}", e))?;
        report.moved += 1;
    }

    remap_sync_state(&uid, &remaps, &new_workspace, &target_path);
    transfer::cache_workspace_path(&new_workspace, &target_path);
    Ok(report)
}

/// 搬完目录后，同步基线 / 待处理冲突里的**工作区维度**要跟着改。
///
/// 不改的后果（都不是数据丢失，但很烦人）：
///   · 基线 key 是 `<IDE>|<工作区哈希>\u{1f}<会话 id>`，换工作区后 key 变了 →
///     下次切号把它当「首次见到」重跑一遍合并；
///   · 待处理冲突里存的是原始哈希，定位不到目录 → 裁决时掉进「两侧都没了」分支被清掉。
pub(super) fn remap_sync_state(
    uid: &str,
    remaps: &[(String, String, String)],
    new_workspace: &str,
    target_path: &str,
) {
    use crate::commands::buddy::session_sync as sync;
    if remaps.is_empty() {
        return;
    }
    let label = crate::commands::buddy::session_transfer::codebuddy::BACKUP_PLATFORM_LABEL;

    // 基线：把旧 key 的指纹挪到新 key 上
    let mut baseline = sync::load_baseline(label, uid);
    let mut changed = false;
    for (ide, old_workspace, id) in remaps {
        let old_key = sync::baseline_key(
            &crate::commands::buddy::session_transfer::codebuddy::workspace_namespace(
                ide,
                old_workspace,
            ),
            id,
        );
        let new_key = sync::baseline_key(
            &crate::commands::buddy::session_transfer::codebuddy::workspace_namespace(
                ide,
                new_workspace,
            ),
            id,
        );
        if let Some(value) = baseline.remove(&old_key) {
            baseline.insert(new_key, value);
            changed = true;
        }
    }
    if changed {
        sync::save_baseline(label, uid, &baseline);
    }

    // 待处理冲突：workspace 换成新哈希（仍存原始哈希，不改成显示用路径）
    let mut conflicts = sync::load_pending_conflicts(label, uid);
    let mut conflicts_changed = false;
    for conflict in conflicts.iter_mut() {
        let hit = remaps.iter().any(|(ide, old_workspace, id)| {
            conflict.ide == *ide && conflict.workspace == *old_workspace && conflict.id == *id
        });
        if hit {
            conflict.workspace = new_workspace.to_string();
            conflict.workspace_path = Some(target_path.to_string());
            conflicts_changed = true;
        }
    }
    if conflicts_changed {
        sync::save_pending_conflicts(label, uid, &conflicts);
    }
}
