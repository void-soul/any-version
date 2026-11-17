//! FinalShell 连接文件的兼容读写。
//!
//! 实证依据：`D:\Program Files\finalshell\conn\<id>_connect_config.json` 是单行紧凑 JSON，
//! 导出文件与源文件逐字节一致；`password` 是标准 Base64（**带 `=` 填充**，本机样本
//! 恰好都是 24 字节 blob 才没有 `=`）。字段 `conection_type` 是 FinalShell 的官方拼写
//! （少一个 r），101 = RDP。
//! 密码本身按未编辑时字节级透传；加解密能力在 [`super::finalshell_crypto`]。
use super::finalshell_crypto;
use super::models::RdpConnection;
use serde_json::Value;

pub const FINALSHELL_SSH_TYPE: i64 = 100;
pub const FINALSHELL_RDP_TYPE: i64 = 101;

pub use super::finalshell_crypto::is_finalshell_cipher;

fn kind_for(conn_type: i64) -> Option<&'static str> {
    match conn_type {
        FINALSHELL_SSH_TYPE => Some("ssh"),
        FINALSHELL_RDP_TYPE => Some("rdp"),
        _ => None,
    }
}

/// 解析单个 FinalShell 连接文件；非 RDP 类型返回 `Ok(None)`。
/// 返回 (连接, 原始文本)，原始文本用于导出时逐字节写回。
pub fn parse_finalshell(text: &str) -> Result<Option<(RdpConnection, String)>, String> {
    let value: Value = serde_json::from_str(text.trim().trim_start_matches('\u{feff}'))
        .map_err(|e| format!("FinalShell 连接文件不是有效 JSON: {e}"))?;
    let conn_type = value
        .get("conection_type")
        .and_then(Value::as_i64)
        .ok_or_else(|| "FinalShell 连接文件缺少 conection_type".to_string())?;
    let Some(kind) = kind_for(conn_type) else {
        return Ok(None);
    };
    let cipher = str_field(&value, "password");
    if !is_finalshell_cipher(&cipher) {
        return Err("FinalShell 密码密文形态不符（应为标准 Base64，解码后 ≥16 字节）".to_string());
    }
    let fs_id = str_field(&value, "id");
    if fs_id.is_empty() {
        return Err("FinalShell 连接文件缺少 id".to_string());
    }
    let now = chrono::Utc::now().timestamp_millis();
    let port = value
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(if kind == "ssh" { 22 } else { 3389 });
    let mut conn = RdpConnection::new(
        str_field(&value, "name"),
        str_field(&value, "host"),
        port,
        str_field(&value, "user_name"),
    );
    // 用 FinalShell 的 id 派生，保证重复导入落到同一条，且与本地 id 不冲突。
    conn.id = format!("rdp_fs_{fs_id}");
    conn.kind = kind.to_string();
    conn.finalshell_cipher = Some(cipher);
    conn.finalshell_raw = Some(text.to_string());
    conn.group = str_field(&value, "parent_id");
    conn.note = str_field(&value, "description");
    // FinalShell 把「驱动器映射」直接存成 drivestoredirect；不读它就会表现为
    // 「FinalShell 里能看到的磁盘，导入后没了」。
    conn.map_drives = value
        .get("drivestoredirect")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    conn.source = "finalshell".to_string();
    conn.created_at = value.get("create_time").and_then(Value::as_i64).unwrap_or(now);
    Ok(Some((conn, text.to_string())))
}

/// 生成 FinalShell 兼容的单连接 JSON。
/// **未编辑**（含密码未改）时返回原始文本，字节级一致；编辑过则只覆盖可编辑字段与
/// 重新加密后的 `password`，其余键（id / 时间戳 / 全部第三方字段）原样保留。
pub fn export_finalshell(conn: &RdpConnection) -> Result<String, String> {
    let raw = conn
        .finalshell_raw
        .as_deref()
        .ok_or_else(|| "该连接不是从 FinalShell 导入的，无法导出兼容文件".to_string())?;
    let original: Value = serde_json::from_str(raw)
        .map_err(|e| format!("保存的 FinalShell 原始配置损坏: {e}"))?;
    let current_cipher = conn.finalshell_cipher.clone().unwrap_or_default();
    let original_drives = original
        .get("drivestoredirect")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let same = str_field(&original, "name") == conn.name
        && str_field(&original, "host") == conn.host
        && str_field(&original, "user_name") == conn.username
        && original.get("port").and_then(Value::as_u64) == Some(u64::from(conn.port))
        && str_field(&original, "password") == current_cipher
        && original_drives == conn.map_drives;
    if same {
        return Ok(raw.to_string());
    }
    if !current_cipher.is_empty() && !finalshell_crypto::is_finalshell_cipher(&current_cipher) {
        return Err("FinalShell 密码密文形态不合法，未导出".to_string());
    }
    let mut out = original.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.insert("name".into(), Value::String(conn.name.clone()));
        obj.insert("host".into(), Value::String(conn.host.clone()));
        obj.insert("user_name".into(), Value::String(conn.username.clone()));
        obj.insert("port".into(), Value::from(conn.port));
        obj.insert("drivestoredirect".into(), Value::Bool(conn.map_drives));
        if !current_cipher.is_empty() {
            obj.insert("password".into(), Value::String(current_cipher));
        }
        // id / 时间戳 / 其它第三方字段一律不动。
    }
    serde_json::to_string(&out).map_err(|e| format!("生成 FinalShell 兼容文件失败: {e}"))
}

/// 从 Value 读取字符串字段。
pub(super) fn str_field(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}
