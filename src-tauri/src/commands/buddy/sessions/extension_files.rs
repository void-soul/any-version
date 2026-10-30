use super::*;

/// 在扩展数据目录 `<uid>/<IDE>/<uid>/` 中查找并删除会话文件：
/// 命中工作区的 index.json 会话条目、会话目录本体与 5 类辅助目录。
///
/// **遍历该目录下所有账号 uid**（不是只删当前登录账号）：会话合并会把同一 id 复制进多个账号目录，
/// 只删一份的话切到别的账号会"复活"（2026-10-19 真机：一条会话在 14 个账号目录里都有副本）。
///
/// 返回 `false` 表示**有文件没能删掉**（被占用、索引写不进去…）——
/// 调用方据此保留数据库记录作为重试锚点，别让会话变成"列表里没了、磁盘上还在"的孤儿。
pub(super) fn delete_session_files_in_extension(
    extension_data_dir: &Path,
    ids: &[String],
    report: &mut BuddySessionDeleteReport,
) -> bool {
    use crate::commands::buddy::session_transfer::codebuddy as transfer;
    let mut ok = true;
    let outer_entries = match std::fs::read_dir(extension_data_dir) {
        Ok(entries) => entries,
        Err(e) => {
            // 读不到目录 ≠ 删干净了：返回 false 让上层保留数据库记录。
            // 反过来说「成功」会留下「列表里没了、磁盘上还在」的孤儿正文。
            report.errors.push(format!(
                "读取 CodeBuddy 扩展数据目录失败（{}），已保留数据库记录以便重试",
                e
            ));
            return false;
        }
    };
    for outer in outer_entries.flatten() {
        let outer_path = outer.path();
        if !is_managed_dir(&outer_path) {
            continue;
        }
        let Some(uid_os) = outer_path.file_name() else {
            continue;
        };
        let uid = uid_os.to_string_lossy().to_string();
        if transfer::validate_uid(&uid).is_err() {
            // 非账号目录（log / tmp / 自定义目录）—— 不是我们的数据，跳过
            continue;
        }
        let ide_entries = match std::fs::read_dir(&outer_path) {
            Ok(entries) => entries,
            Err(e) => {
                report
                    .errors
                    .push(format!("读取账号目录 {} 失败: {}", outer_path.display(), e));
                continue;
            }
        };
        for ide_entry in ide_entries.flatten() {
            let ide_path = ide_entry.path();
            if !is_managed_dir(&ide_path) {
                continue;
            }
            let account_root = ide_path.join(&uid);
            let history_root = account_root.join("history");
            if !is_managed_dir(&history_root) {
                continue;
            }
            let workspace_entries = match std::fs::read_dir(&history_root) {
                Ok(entries) => entries,
                Err(e) => {
                    report.errors.push(format!(
                        "读取会话目录 {} 失败: {}",
                        history_root.display(),
                        e
                    ));
                    continue;
                }
            };
            for workspace_entry in workspace_entries.flatten() {
                let Ok(metadata) = std::fs::symlink_metadata(workspace_entry.path()) else {
                    continue;
                };
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    continue;
                }
                let workspace_path = workspace_entry.path();
                let index_path = workspace_path.join("index.json");
                let Ok(index) = transfer::read_workspace_index(&index_path) else {
                    continue;
                };
                let Some(conversations) = index.get("conversations").and_then(Value::as_array)
                else {
                    continue;
                };
                let hit = ids.iter().any(|id| {
                    conversations
                        .iter()
                        .any(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str()))
                });
                if !hit {
                    continue;
                }

                // index.json：移除被删会话条目；current 指向被删会话时清空
                let mut new_index = index.clone();
                if let Some(array) = new_index
                    .get_mut("conversations")
                    .and_then(Value::as_array_mut)
                {
                    array.retain(|c| {
                        let cid = c.get("id").and_then(Value::as_str).unwrap_or_default();
                        !ids.iter().any(|id| id == cid)
                    });
                }
                if let Some(object) = new_index.as_object_mut() {
                    let current = object
                        .get("current")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if !current.is_empty() && ids.iter().any(|id| id == &current) {
                        object.insert("current".to_string(), Value::String(String::new()));
                    }
                }
                let serialized = match serde_json::to_string_pretty(&new_index)
                    .map_err(|e| format!("序列化工作区索引失败: {}", e))
                {
                    Ok(s) => s,
                    Err(e) => {
                        report.errors.push(e);
                        ok = false;
                        continue;
                    }
                };
                if let Err(e) =
                    crate::commands::buddy::store::write_atomic(&index_path, &serialized)
                {
                    report.errors.push(e);
                    ok = false;
                    continue;
                }

                // 删除会话目录与辅助目录
                for id in ids {
                    let conv_dir = workspace_path.join(id);
                    if conv_dir.is_dir() {
                        match std::fs::remove_dir_all(&conv_dir) {
                            Ok(()) => report.history_dirs_removed += 1,
                            Err(e) => {
                                ok = false;
                                report.errors.push(format!(
                                    "删除会话目录失败 {}: {}",
                                    conv_dir.display(),
                                    e
                                ));
                            }
                        }
                    }
                    for kind in transfer::AUXILIARY_KINDS {
                        let aux_dir = account_root
                            .join(kind)
                            .join(workspace_entry.file_name())
                            .join(id);
                        if aux_dir.is_dir() {
                            match std::fs::remove_dir_all(&aux_dir) {
                                Ok(()) => report.auxiliary_dirs_removed += 1,
                                Err(e) => {
                                    ok = false;
                                    report.errors.push(format!(
                                        "删除辅助目录失败 {}: {}",
                                        aux_dir.display(),
                                        e
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    ok
}
