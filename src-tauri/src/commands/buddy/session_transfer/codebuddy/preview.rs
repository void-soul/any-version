use super::*;

// ─── 冲突明细：对话内容预览 ───

/// 对话预览里的一条消息（已从双层 JSON 里抽出正文）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictMessage {
    /// user / assistant / tool
    pub role: String,
    pub time: String,
    pub text: String,
}

/// 预览取**最后**多少条（裁决关心的是两边最近聊到哪了）
pub(super) const CONVERSATION_PREVIEW_MAX_MESSAGES: usize = 40;
/// 单条正文截断长度（tool 输出可能巨大）
pub(super) const CONVERSATION_PREVIEW_MAX_CHARS: usize = 1200;

/// 读取冲突会话某一侧（source / target）的对话内容。
///
/// 会话正文布局：`<uid>/<IDE>/<uid>/history/<workspace>/<convId>/index.json`
/// 的 `messages` 数组按序存消息元数据，每条正文在 `messages/<messageId>.json`。
pub(crate) fn read_conflict_messages(
    conversation_id: &str,
    side: &str,
) -> Result<Vec<ConflictMessage>, String> {
    let Some((conflict, _)) = find_pending_conflict(conversation_id)? else {
        return Err(format!("找不到会话 {} 的待处理冲突", conversation_id));
    };
    let uid = match side {
        "source" => &conflict.source_uid,
        "target" => &conflict.target_uid,
        other => return Err(format!("未知的会话侧: {}", other)),
    };
    validate_uid(uid)?;
    let extension_data_dir = codebuddy_extension_data_dir()?;
    let conv_dir = extension_data_dir
        .join(uid)
        .join(&conflict.ide)
        .join(uid)
        .join("history")
        .join(&conflict.workspace)
        .join(&conflict.id);
    if !conv_dir.is_dir() {
        return Err(format!("会话目录不存在: {}", conv_dir.display()));
    }
    reject_symlink_if_exists(&conv_dir)?;
    let index = read_workspace_index(&conv_dir.join("index.json"))?;
    let metas = index
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let start = metas
        .len()
        .saturating_sub(CONVERSATION_PREVIEW_MAX_MESSAGES);
    let mut out = Vec::new();
    for meta in &metas[start..] {
        let Some(id) = meta.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id.contains('/') || id.contains('\\') || id.contains("..") {
            continue;
        }
        let path = conv_dir.join("messages").join(format!("{}.json", id));
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        extract_message_text(&raw, &mut out);
    }
    Ok(out)
}

/// 从消息文件原文抽出 `{role, time, text}`。
///
/// `message` 字段是**内嵌 JSON 字符串**，其 `content` 数组里 `type=text` 的才是
/// 用户可读正文；tool-call 折叠成一行标记，避免工具输出把预览撑爆。
pub(super) fn extract_message_text(raw: &str, out: &mut Vec<ConflictMessage>) {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let time = value
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let message = match value.get("message") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let mut text = String::new();
    if let Ok(inner) = serde_json::from_str::<Value>(&message) {
        if let Some(content) = inner.get("content").and_then(Value::as_array) {
            for part in content {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                    }
                    Some("tool-call") => {
                        let name = part
                            .get("toolName")
                            .and_then(Value::as_str)
                            .unwrap_or("tool");
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&format!("[调用工具: {}]", name));
                    }
                    _ => {}
                }
            }
        }
    }
    // 双层解析失败就用原文兜底，别让一条消息悄悄消失
    if text.is_empty() {
        text = message;
    }
    // tool 输出（role=tool 的原始 JSON）对裁决没帮助，更紧地截断
    let max_chars = if role == "tool" {
        300
    } else {
        CONVERSATION_PREVIEW_MAX_CHARS
    };
    if text.chars().count() > max_chars {
        text = text.chars().take(max_chars).collect::<String>() + "…";
    }
    out.push(ConflictMessage { role, time, text });
}

pub(crate) fn validate_uid(uid: &str) -> Result<(), String> {
    let trimmed = uid.trim();
    if trimmed.is_empty() || trimmed != uid {
        return Err("CodeBuddy CN UID 为空或包含首尾空白".to_string());
    }
    if uid.contains('/') || uid.contains('\\') || uid.contains("..") || uid.contains('\0') {
        return Err("CodeBuddy CN UID 包含不安全的路径字符".to_string());
    }
    let mut components = Path::new(uid).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err("CodeBuddy CN UID 不是安全的单级路径".to_string()),
    }
}
