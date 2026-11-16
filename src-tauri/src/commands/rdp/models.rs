use serde::{Deserialize, Serialize};

/// 一条远程连接：`rdp`（Windows 远程桌面）或 `ssh`（Linux 服务器）。
///
/// 密码有两种互不相通的来源，绝不混用：
/// - **自有连接**：`password` = `ENC_V2:` + AES-256-GCM（复用 `secrets.rs`，机器级主密钥）。
/// - **FinalShell 导入**：`finalshell_cipher` 原样保存它的标准 Base64 密文，
///   `finalshell_raw` 保存原始单行 JSON，导出时逐字节写回；解密由 `finalshell_crypto` 提供。
///   100/101 共用同一套 33 键 schema，因此两类连接共用这张表。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RdpConnection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// "rdp" | "ssh"；旧数据缺字段时 serde 回落成 rdp，无需迁移。
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalshell_cipher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalshell_raw: Option<String>,
    /// SSH 认证方式："password"（默认）| "key"（私钥文件）。
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    /// 私钥文件路径（仅 auth_type=key 时读取，由用户通过文件对话框选择）。
    #[serde(default)]
    pub private_key_path: String,
    /// 私钥口令，`ENC_V2:` 加密存储；无口令时为空。
    #[serde(default)]
    pub key_passphrase: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub group: String,
    /// "local"（本模块新建）或 "finalshell"
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_kind() -> String {
    "rdp".to_string()
}

fn default_auth_type() -> String {
    "password".to_string()
}

impl RdpConnection {
    pub fn new(name: String, host: String, port: u16, username: String) -> Self {
        let now = chrono::Utc::now().timestamp_millis();
        Self {
            id: format!("rdp_{}", now),
            name,
            host,
            port,
            kind: default_kind(),
            username,
            password: None,
            finalshell_cipher: None,
            finalshell_raw: None,
            auth_type: default_auth_type(),
            private_key_path: String::new(),
            key_passphrase: String::new(),
            domain: String::new(),
            note: String::new(),
            group: String::new(),
            source: "local".to_string(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub messages: Vec<String>,
}
