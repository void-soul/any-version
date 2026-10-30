use super::*;

pub(super) fn ensure_workspace_index_backup(
    target_index_path: &Path,
    backup_root: &Path,
    backup_created: &mut bool,
) -> Result<(), String> {
    if *backup_created {
        return Ok(());
    }
    std::fs::create_dir_all(backup_root).map_err(|e| {
        format!(
            "创建 CodeBuddy CN 会话备份目录失败: path={}, error={}",
            backup_root.display(),
            e
        )
    })?;
    std::fs::copy(target_index_path, backup_root.join("index.json")).map_err(|e| {
        format!(
            "备份 CodeBuddy CN 工作区索引失败: path={}, error={}",
            target_index_path.display(),
            e
        )
    })?;
    *backup_created = true;
    Ok(())
}

pub(crate) fn read_workspace_index(path: &Path) -> Result<Value, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "读取 CodeBuddy CN 工作区索引失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    let value: Value = serde_json::from_str(&content).map_err(|e| {
        format!(
            "解析 CodeBuddy CN 工作区索引失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    if !value.is_object()
        || value
            .get("conversations")
            .map(|v| !v.is_array())
            .unwrap_or(false)
    {
        return Err(format!(
            "CodeBuddy CN 工作区索引结构无效: {}",
            path.display()
        ));
    }
    Ok(value)
}

/// 原子写入工作区索引（冲突裁决更新目标 index.json 用，与读取同一格式）。
pub(super) fn write_workspace_index(path: &Path, index: &Value) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(index)
        .map_err(|e| format!("序列化 CodeBuddy CN 工作区索引失败: {}", e))?;
    store::write_atomic(&path.to_path_buf(), &serialized)
}

pub(super) fn conversations(index: &Value) -> Vec<Value> {
    index
        .get("conversations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn conversation_id(conversation: &Value) -> Option<&str> {
    conversation.get("id").and_then(Value::as_str)
}

pub(crate) fn validate_conversation_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("CodeBuddy CN conversationId 包含不安全的路径字符".to_string());
    }
    Ok(())
}

pub(super) fn conversation_is_newer(source: &Value, target: &Value) -> bool {
    compare_conversation_recency(source, target) == Ordering::Greater
}

pub(super) fn compare_conversation_recency(left: &Value, right: &Value) -> Ordering {
    // None（取不到时间）按"最旧"处理，不再与 0 混为一谈
    conversation_timestamp(left).cmp(&conversation_timestamp(right))
}

/// 会话最后消息时间（epoch 毫秒）。
///
/// 返回 `None` 表示**取不到**（字段缺失 / 类型异常 / RFC3339 解析失败）——
/// 以前用 `unwrap_or_default()` 一律回 0，于是"两侧都缺时间戳"会被判成两边相等、
/// 走"无变化"静默跳过，改动永远同步不过去。现在把"未知"显式区分出来：
/// 未知的一侧无法证明自己没变，一律按"有变化"处理（见 [`decide_sync`]）。
pub(super) fn conversation_timestamp(conversation: &Value) -> Option<i64> {
    let value = conversation.get("lastMessageAt")?;
    if let Some(timestamp) = value.as_i64() {
        return Some(timestamp);
    }
    value
        .as_str()
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|ts| ts.timestamp_millis())
}

/// 指纹里代表"取不到时间戳"的哨兵值（不会与任何数字字符串相等）。
pub(crate) const UNKNOWN_STAMP: &str = "unknown";

/// 会话判定指纹：`时间戳|目录状态`，任一部分变化都算"有变化"。
///
/// 只比时间戳会漏掉「内容变了但客户端没 bump lastMessageAt 」的场景
/// （改标题、删末条消息、上下文裁剪）；目录状态见
/// [`crate::commands::buddy::session_sync::directory_state_fingerprint`]。
pub(super) fn conversation_fingerprint(conversation: &Value, conversation_dir: &Path) -> String {
    let stamp = conversation_timestamp(conversation)
        .map(|ts| ts.to_string())
        .unwrap_or_else(|| UNKNOWN_STAMP.to_string());
    crate::commands::buddy::session_sync::combined_fingerprint_str(
        &stamp,
        crate::commands::buddy::session_sync::directory_state_fingerprint(conversation_dir)
            .as_deref(),
    )
}

pub(crate) fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "拒绝通过符号链接读写 CodeBuddy CN 会话: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "读取 CodeBuddy CN 会话路径属性失败: path={}, error={}",
            path.display(),
            e
        )),
    }
}
