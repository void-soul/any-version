//! OTP 模块（复刻 CloudOTP 的电脑端核心：5 种算法 + token 管理 + 本地持久化）。
//!
//! 忽略 CloudOTP 的云备份 / 多端同步 / 生物识别等特性，仅保留本地单机能力。

mod algo;

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::commands::config::get_data_dir;
use crate::commands::secrets::{decrypt_secret, encrypt_secret};

/// 与 secrets.rs 的 CRED_ENCRYPTION_MARKER 保持一致：密文前缀，用于区分加密/明文行
const ENC_MARKER: &str = "ENC_V2:";

/// OTP 令牌
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpToken {
    pub id: i64,
    pub issuer: String,
    pub account: String,
    pub secret: String,
    pub token_type: String, // "TOTP" | "HOTP" | "MOTP" | "Steam" | "Yandex"
    pub algorithm: String,  // "SHA1" | "SHA256" | "SHA512"
    pub digits: i64,
    pub period: i64,
    pub counter: i64,
    pub pin: String,
    pub pinned: bool,
    pub created_at: i64,
    /// 描述（可选）
    #[serde(default)]
    pub description: String,
    /// 标签（逗号分隔，前端解析为数组）
    #[serde(default)]
    pub tags: String,
    /// 复制次数
    #[serde(default)]
    pub copy_times: i64,
    /// 上次复制时间戳（毫秒）
    #[serde(default)]
    pub last_copy_time: i64,
    /// 自定义图标（emoji 或内置品牌标识）；空 = 自动匹配品牌
    #[serde(default)]
    pub custom_icon: String,
}

impl Default for OtpToken {
    fn default() -> Self {
        Self {
            id: 0,
            issuer: String::new(),
            account: String::new(),
            secret: String::new(),
            token_type: "TOTP".into(),
            algorithm: "SHA1".into(),
            digits: 6,
            period: 30,
            counter: 0,
            pin: String::new(),
            pinned: false,
            created_at: 0,
            description: String::new(),
            tags: String::new(),
            copy_times: 0,
            last_copy_time: 0,
            custom_icon: String::new(),
        }
    }
}

/// 数据库连接（懒初始化单例）
fn conn() -> &'static Mutex<rusqlite::Connection> {
    static DB: OnceLock<Mutex<rusqlite::Connection>> = OnceLock::new();
    DB.get_or_init(|| {
        let dir = get_data_dir().join("otp");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("otp.db");
        let c = rusqlite::Connection::open(path).expect("打开 OTP 数据库失败");
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS otp_tokens (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                issuer TEXT NOT NULL DEFAULT '',
                account TEXT NOT NULL DEFAULT '',
                secret TEXT NOT NULL DEFAULT '',
                token_type TEXT NOT NULL DEFAULT 'TOTP',
                algorithm TEXT NOT NULL DEFAULT 'SHA1',
                digits INTEGER NOT NULL DEFAULT 6,
                period INTEGER NOT NULL DEFAULT 30,
                counter INTEGER NOT NULL DEFAULT 0,
                pin TEXT NOT NULL DEFAULT '',
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL DEFAULT 0,
                description TEXT NOT NULL DEFAULT '',
                tags TEXT NOT NULL DEFAULT '',
                copy_times INTEGER NOT NULL DEFAULT 0,
                last_copy_time INTEGER NOT NULL DEFAULT 0,
                custom_icon TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE IF NOT EXISTS otp_categories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS otp_category_binding (
                token_id INTEGER NOT NULL,
                category_id INTEGER NOT NULL,
                PRIMARY KEY (token_id, category_id)
            );",
        )
        .expect("创建 OTP 表失败");
        // 迁移：旧表补列（TEXT 列 + INTEGER 列分开）
        for col in ["description", "tags", "custom_icon"] {
            let _ = c.execute(
                &format!("ALTER TABLE otp_tokens ADD COLUMN {col} TEXT NOT NULL DEFAULT ''"),
                [],
            );
        }
        for col in ["copy_times", "last_copy_time"] {
            let _ = c.execute(
                &format!("ALTER TABLE otp_tokens ADD COLUMN {col} INTEGER NOT NULL DEFAULT 0"),
                [],
            );
        }
        // 安全迁移：历史明文 secret/pin 就地加密为 ENC_V2（幂等，仅处理未加密行）。
        // OTP 种子等同账户密码，落盘必须加密（AES-256-GCM，机器级主密钥）。
        let plain_rows: Vec<(i64, String, String)> = {
            let mut stmt = c
                .prepare(
                    "SELECT id, secret, pin FROM otp_tokens
                     WHERE (secret != '' AND secret NOT LIKE 'ENC_V2:%')
                        OR (pin != '' AND pin NOT LIKE 'ENC_V2:%')",
                )
                .expect("查询 OTP 明文行失败");
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map(|iter| iter.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        };
        for (id, secret, pin) in plain_rows {
            let new_secret = if !secret.is_empty() && !secret.starts_with(ENC_MARKER) {
                encrypt_secret(&secret).unwrap_or_else(|e| {
                    eprintln!("[otp] secret 加密迁移失败 (id={}): {}", id, e);
                    secret
                })
            } else {
                secret
            };
            let new_pin = if !pin.is_empty() && !pin.starts_with(ENC_MARKER) {
                encrypt_secret(&pin).unwrap_or_else(|e| {
                    eprintln!("[otp] pin 加密迁移失败 (id={}): {}", id, e);
                    pin
                })
            } else {
                pin
            };
            let _ = c.execute(
                "UPDATE otp_tokens SET secret=?1, pin=?2 WHERE id=?3",
                rusqlite::params![new_secret, new_pin, id],
            );
        }
        Mutex::new(c)
    })
}

/// 读取时解密字段（secret/pin）：ENC_V2 前缀走 AES 解密，旧明文透传（迁移前的历史数据）。
fn decrypt_field(value: String) -> String {
    if value.starts_with(ENC_MARKER) {
        match decrypt_secret(&value) {
            Ok(plain) => plain,
            Err(e) => {
                eprintln!("[otp] 字段解密失败（密钥不匹配或数据损坏）: {}", e);
                value
            }
        }
    } else {
        value
    }
}

/// 写入时加密字段（secret/pin）：空值与已加密值原样保留，其余 AES 加密。
fn encrypt_field(value: &str) -> Result<String, String> {
    if value.is_empty() || value.starts_with(ENC_MARKER) {
        Ok(value.to_string())
    } else {
        encrypt_secret(value)
    }
}

fn row_to_token(row: &rusqlite::Row) -> rusqlite::Result<OtpToken> {
    Ok(OtpToken {
        id: row.get(0)?,
        issuer: row.get(1)?,
        account: row.get(2)?,
        secret: decrypt_field(row.get(3)?),
        token_type: row.get(4)?,
        algorithm: row.get(5)?,
        digits: row.get(6)?,
        period: row.get(7)?,
        counter: row.get(8)?,
        pin: decrypt_field(row.get(9)?),
        pinned: row.get::<_, i64>(10)? != 0,
        created_at: row.get(11)?,
        description: row.get(12)?,
        tags: row.get(13)?,
        copy_times: row.get(14)?,
        last_copy_time: row.get(15)?,
        custom_icon: row.get(16)?,
    })
}

/// 统一 SELECT 列清单（与 row_to_token 的列顺序一致）
const TOKEN_COLS: &str = "id, issuer, account, secret, token_type, algorithm, digits, period, counter, pin, pinned, created_at, description, tags, copy_times, last_copy_time, custom_icon";

// —— otpauth URI 解析 ——

fn url_decode(s: &str) -> String {
    let s = s.replace('+', "%20");
    match percent_decode(&s) {
        Some(v) => v,
        None => s,
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let v = u8::from_str_radix(hex, 16).ok()?;
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 解析 otpauth:// URI（单条），失败返回 None。
pub fn parse_otpauth_uri(line: &str) -> Option<OtpToken> {
    let line = line.trim();
    if !line.starts_with("otpauth://") && !line.starts_with("motp://") {
        return None;
    }

    // 分离 scheme://authority/path?query
    let after_scheme = line.splitn(2, "://").nth(1)?;
    let (authority_and_path, query) = match after_scheme.split_once('?') {
        Some((ap, q)) => (ap, q),
        None => (after_scheme, ""),
    };

    // authority = token type（如 totp / hotp / steam / yaotp / motp）
    let (authority, path) = authority_and_path.split_once('/')?;
    let token_type = normalize_token_type(authority)?;

    // path = label，形如 "issuer:account" 或 "account"
    let label = url_decode(path);
    let (issuer_ext, account) = match label.split_once(':') {
        Some((i, a)) => (i.to_string(), a.to_string()),
        None => (String::new(), label.clone()),
    };

    // 解析 query 参数
    let mut params: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for kv in query.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            params.insert(url_decode(k), url_decode(v));
        }
    }

    let secret = params.get("secret")?.clone();
    if secret.is_empty() {
        return None;
    }

    let issuer = params
        .get("issuer")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or(issuer_ext);

    let algorithm = params
        .get("algorithm")
        .map(|s| s.to_uppercase())
        .unwrap_or_else(|| "SHA1".into());

    let digits: i64 = params
        .get("digits")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| default_digits(&token_type));

    let period: i64 = params
        .get("period")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let counter: i64 = params
        .get("counter")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let pin = params.get("pin").cloned().unwrap_or_default();

    Some(OtpToken {
        id: 0,
        issuer,
        account,
        secret,
        token_type,
        algorithm,
        digits,
        period,
        counter,
        pin,
        pinned: false,
        created_at: chrono::Utc::now().timestamp_millis(),
        description: String::new(),
        tags: String::new(),
        copy_times: 0,
        last_copy_time: 0,
        custom_icon: String::new(),
    })
}

fn normalize_token_type(authority: &str) -> Option<String> {
    match authority.to_ascii_lowercase().as_str() {
        "totp" => Some("TOTP".into()),
        "hotp" => Some("HOTP".into()),
        "motp" => Some("MOTP".into()),
        "steam" => Some("Steam".into()),
        "yaotp" | "yandex" => Some("Yandex".into()),
        _ => None,
    }
}

fn default_digits(token_type: &str) -> i64 {
    match token_type {
        "Steam" => 5,
        "Yandex" => 8,
        _ => 6,
    }
}

// —— otpauth-migration:// 解析（Google Authenticator 批量迁移 protobuf） ——

/// 最小 protobuf wire-format 读取器（仅支持 varint + length-delimited，够解析 otpauth-migration）。
struct ProtoReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ProtoReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn read_varint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }

    fn read_len_delimited(&mut self) -> Option<&'a [u8]> {
        let len = self.read_varint()? as usize;
        let end = self.pos.checked_add(len)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
    }

    fn skip_field(&mut self, wire_type: u64) -> bool {
        match wire_type {
            0 => self.read_varint().is_some(),
            1 => {
                // 64-bit
                if self.pos + 8 > self.buf.len() {
                    return false;
                }
                self.pos += 8;
                true
            }
            2 => self.read_len_delimited().is_some(),
            5 => {
                // 32-bit
                if self.pos + 4 > self.buf.len() {
                    return false;
                }
                self.pos += 4;
                true
            }
            _ => false,
        }
    }
}

/// 解析单个 OtpMigrationParameters（嵌套 message）。
fn parse_migration_parameter(buf: &[u8]) -> Option<OtpToken> {
    let mut reader = ProtoReader::new(buf);
    let mut secret: Vec<u8> = Vec::new();
    let mut account = String::new();
    let mut issuer = String::new();
    let mut algorithm = 0u64; // 0=unspecified,1=SHA1,2=SHA256,3=SHA512,4=MD5
    let mut digits = 0u64; // 0=unspecified,1=SIX,2=EIGHT
    let mut token_type = 0u64; // 0=unspecified,1=HOTP,2=TOTP
    let mut counter = 0i64;

    while let Some(key) = reader.read_varint() {
        let field_no = key >> 3;
        let wire_type = key & 0x07;
        match (field_no, wire_type) {
            (1, 2) => secret = reader.read_len_delimited()?.to_vec(),
            (2, 2) => account = String::from_utf8_lossy(reader.read_len_delimited()?).into_owned(),
            (3, 2) => issuer = String::from_utf8_lossy(reader.read_len_delimited()?).into_owned(),
            (4, 0) => algorithm = reader.read_varint()?,
            (5, 0) => digits = reader.read_varint()?,
            (6, 0) => token_type = reader.read_varint()?,
            (7, 0) => counter = reader.read_varint()? as i64,
            _ => {
                if !reader.skip_field(wire_type) {
                    return None;
                }
            }
        }
    }

    if secret.is_empty() {
        return None;
    }

    // secret 是原始字节，需 base32 编码存为字符串
    let secret_b32 = algo::base32_encode(&secret);
    let token_type_str = match token_type {
        1 => "HOTP".to_string(),
        _ => "TOTP".to_string(),
    };
    let algorithm_str = match algorithm {
        2 => "SHA256".to_string(),
        3 => "SHA512".to_string(),
        4 => "MD5".to_string(),
        _ => "SHA1".to_string(),
    };
    let digits_num: i64 = match digits {
        2 => 8,
        _ => 6,
    };

    Some(OtpToken {
        id: 0,
        issuer: if issuer.is_empty() { account.clone() } else { issuer },
        account,
        secret: secret_b32,
        token_type: token_type_str,
        algorithm: algorithm_str,
        digits: digits_num,
        period: 30,
        counter,
        pin: String::new(),
        pinned: false,
        created_at: chrono::Utc::now().timestamp_millis(),
        description: String::new(),
        tags: String::new(),
        copy_times: 0,
        last_copy_time: 0,
        custom_icon: String::new(),
    })
}

/// 解析 otpauth-migration://offline?data=<base64> 批量迁移负载，返回令牌列表。
pub fn parse_otpauth_migration(line: &str) -> Option<Vec<OtpToken>> {
    let line = line.trim();
    if !line.starts_with("otpauth-migration://") {
        return None;
    }
    // 提取 data 参数
    let query = line.split_once('?').map(|(_, q)| q)?;
    let mut data_b64 = String::new();
    for kv in query.split('&') {
        if let Some((k, v)) = kv.split_once('=') {
            if k == "data" {
                data_b64 = url_decode(v);
            }
        }
    }
    if data_b64.is_empty() {
        return None;
    }
    // base64 解码（补 padding）
    let padded = if data_b64.len() % 4 == 0 {
        data_b64
    } else {
        data_b64.clone() + &"=".repeat(4 - data_b64.len() % 4)
    };
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .ok()?;

    // 解析 OtpMigrationPayload（field 1 = repeated otp_parameters）
    let mut reader = ProtoReader::new(&bytes);
    let mut tokens = Vec::new();
    while let Some(key) = reader.read_varint() {
        let field_no = key >> 3;
        let wire_type = key & 0x07;
        if field_no == 1 && wire_type == 2 {
            if let Some(param_buf) = reader.read_len_delimited() {
                if let Some(t) = parse_migration_parameter(param_buf) {
                    tokens.push(t);
                }
            }
        } else if !reader.skip_field(wire_type) {
            break;
        }
    }
    Some(tokens)
}

// —— 命令 ——

/// 列出全部令牌（置顶优先，然后按创建时间倒序）。
#[tauri::command]
pub fn otp_list() -> Result<Vec<OtpToken>, String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    let mut stmt = c
        .prepare(&format!(
            "SELECT {TOKEN_COLS} FROM otp_tokens ORDER BY pinned DESC, created_at DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_token)
        .map_err(|e| e.to_string())?;
    let mut list = Vec::new();
    for r in rows {
        list.push(r.map_err(|e| e.to_string())?);
    }
    Ok(list)
}

/// 新增令牌。
#[tauri::command]
pub fn otp_add(token: OtpToken) -> Result<i64, String> {
    let secret = encrypt_field(&token.secret)?;
    let pin = encrypt_field(&token.pin)?;
    let c = conn().lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    c.execute(
        "INSERT INTO otp_tokens (issuer, account, secret, token_type, algorithm, digits, period, counter, pin, pinned, created_at, description, tags, copy_times, last_copy_time, custom_icon)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            token.issuer,
            token.account,
            secret,
            token.token_type,
            token.algorithm,
            token.digits,
            token.period,
            token.counter,
            pin,
            token.pinned as i64,
            now,
            token.description,
            token.tags,
            token.copy_times,
            token.last_copy_time,
            token.custom_icon,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

/// 更新令牌。
#[tauri::command]
pub fn otp_update(token: OtpToken) -> Result<(), String> {
    let secret = encrypt_field(&token.secret)?;
    let pin = encrypt_field(&token.pin)?;
    let c = conn().lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE otp_tokens SET issuer=?1, account=?2, secret=?3, token_type=?4, algorithm=?5, digits=?6, period=?7, counter=?8, pin=?9, pinned=?10, description=?11, tags=?12, copy_times=?13, last_copy_time=?14, custom_icon=?15 WHERE id=?16",
        rusqlite::params![
            token.issuer,
            token.account,
            secret,
            token.token_type,
            token.algorithm,
            token.digits,
            token.period,
            token.counter,
            pin,
            token.pinned as i64,
            token.description,
            token.tags,
            token.copy_times,
            token.last_copy_time,
            token.custom_icon,
            token.id,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除令牌。
#[tauri::command]
pub fn otp_delete(id: i64) -> Result<(), String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM otp_tokens WHERE id=?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    c.execute(
        "DELETE FROM otp_category_binding WHERE token_id=?1",
        rusqlite::params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 切换置顶。
#[tauri::command]
pub fn otp_toggle_pin(id: i64, pinned: bool) -> Result<(), String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE otp_tokens SET pinned=?1 WHERE id=?2",
        rusqlite::params![pinned as i64, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 复制后累加复制次数并记录时间。
#[tauri::command]
pub fn otp_mark_copied(id: i64) -> Result<(), String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    c.execute(
        "UPDATE otp_tokens SET copy_times = copy_times + 1, last_copy_time = ?1 WHERE id = ?2",
        rusqlite::params![now, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 导入 otpauth URI / otpauth-migration 迁移负载（单条或批量多行）。
#[tauri::command]
pub fn otp_import_uri(text: String) -> Result<Vec<OtpToken>, String> {
    let mut added = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // otpauth-migration:// 批量迁移
        if trimmed.starts_with("otpauth-migration://") {
            if let Some(tokens) = parse_otpauth_migration(trimmed) {
                for token in tokens {
                    let id = otp_add(token.clone())?;
                    let mut t = token;
                    t.id = id;
                    added.push(t);
                }
            }
            continue;
        }
        // 单条 otpauth:// / motp://
        if let Some(token) = parse_otpauth_uri(trimmed) {
            let id = otp_add(token.clone())?;
            let mut t = token;
            t.id = id;
            added.push(t);
        }
    }
    Ok(added)
}

// —— 分类 ——

/// OTP 分类
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtpCategory {
    pub id: i64,
    pub title: String,
    pub created_at: i64,
    /// 该分类下的令牌 id 列表
    pub token_ids: Vec<i64>,
}

/// 列出全部分类（含每个分类绑定的令牌 id）。
#[tauri::command]
pub fn otp_list_categories() -> Result<Vec<OtpCategory>, String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    let mut cats = Vec::new();
    {
        let mut stmt = c
            .prepare("SELECT id, title, created_at FROM otp_categories ORDER BY id ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
        for r in rows {
            let (id, title, created_at) = r.map_err(|e| e.to_string())?;
            cats.push(OtpCategory { id, title, created_at, token_ids: Vec::new() });
        }
    }
    for cat in cats.iter_mut() {
        let mut stmt = c
            .prepare("SELECT token_id FROM otp_category_binding WHERE category_id=?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([cat.id], |r| r.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        for r in rows {
            cat.token_ids.push(r.map_err(|e| e.to_string())?);
        }
    }
    Ok(cats)
}

/// 新增分类，返回新 id。
#[tauri::command]
pub fn otp_add_category(title: String) -> Result<i64, String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp_millis();
    c.execute(
        "INSERT INTO otp_categories (title, created_at) VALUES (?1, ?2)",
        rusqlite::params![title, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(c.last_insert_rowid())
}

/// 重命名分类。
#[tauri::command]
pub fn otp_rename_category(id: i64, title: String) -> Result<(), String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    c.execute(
        "UPDATE otp_categories SET title=?1 WHERE id=?2",
        rusqlite::params![title, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 删除分类（不删除令牌，仅解除绑定）。
#[tauri::command]
pub fn otp_delete_category(id: i64) -> Result<(), String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    c.execute("DELETE FROM otp_categories WHERE id=?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    c.execute(
        "DELETE FROM otp_category_binding WHERE category_id=?1",
        rusqlite::params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置令牌所属分类（整体替换该令牌的分类绑定）。
#[tauri::command]
pub fn otp_set_token_categories(token_id: i64, category_ids: Vec<i64>) -> Result<(), String> {
    let c = conn().lock().map_err(|e| e.to_string())?;
    c.execute(
        "DELETE FROM otp_category_binding WHERE token_id=?1",
        rusqlite::params![token_id],
    )
    .map_err(|e| e.to_string())?;
    for cid in category_ids {
        c.execute(
            "INSERT OR IGNORE INTO otp_category_binding (token_id, category_id) VALUES (?1, ?2)",
            rusqlite::params![token_id, cid],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// —— 品牌图标 ——

/// 内置品牌清单（与 CloudOTP assets/brand 对应，此处为常用站点）。
const BRAND_LOGOS: &[&str] = &[
    "google", "github", "gitlab", "microsoft", "amazon", "aws", "apple", "facebook",
    "twitter", "x", "discord", "slack", "telegram", "steam", "yandex", "dropbox",
    "paypal", "netflix", "spotify", "epicgames", "battlenet", "blizzard", "ubisoft",
    "ea", "riotgames", "nvidia", "adobe", "salesforce", "atlassian", "jira", "confluence",
    "bitbucket", "wordpress", "shopify", "cloudflare", "namecheap", "godaddy", "linode",
    "digitalocean", "hetzner", "ovh", "protonmail", "proton", "tutanota", "outlook",
    "gmail", "yahoo", "hotmail", "mega", "nextcloud", "synology", "qnap", "vaultwarden",
    "bitwarden", "lastpass", "1password", "keeper", "authy", "twilio", "okta", "duo",
    "kraken", "coinbase", "binance", "bybit", "okx", "nordvpn", "surfshark", "tailscale",
    "zerotier", "cloudron", "huggingface", "openai", "anthropic", "claude", "jetbrains",
];

/// 清理品牌名（去空格/下划线/连字符，转小写），与 CloudOTP cleanBrand 一致。
fn clean_brand(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// 最长公共子串长度（与 CloudOTP longestCommonSubstring 一致）。
fn longest_common_substring(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    let mut max_len = 0;
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            if a[i - 1] == b[j - 1] {
                dp[i][j] = dp[i - 1][j - 1] + 1;
                if dp[i][j] > max_len {
                    max_len = dp[i][j];
                }
            }
        }
    }
    max_len
}

/// 根据 issuer 模糊匹配内置品牌（返回品牌名，未匹配返回 None）。
pub fn match_brand(issuer: &str) -> Option<&'static str> {
    let issuer_clean = clean_brand(issuer);
    if issuer_clean.is_empty() {
        return None;
    }
    let mut best: Option<(&'static str, usize)> = None;
    for brand in BRAND_LOGOS {
        let b_clean = clean_brand(brand);
        let lcs = longest_common_substring(&issuer_clean, &b_clean);
        let contains = issuer_clean.contains(&b_clean) || b_clean.contains(&issuer_clean);
        let equal = issuer_clean == b_clean;
        let score = if equal {
            10000
        } else if contains || lcs >= 5 {
            lcs
        } else {
            continue;
        };
        if best.map_or(true, |(_, s)| score > s) {
            best = Some((*brand, score));
        }
    }
    best.map(|(b, _)| b)
}

/// 返回内置品牌图标清单（供前端展示选择）。
#[tauri::command]
pub fn otp_list_brands() -> Result<Vec<String>, String> {
    Ok(BRAND_LOGOS.iter().map(|s| s.to_string()).collect())
}

/// 为令牌匹配品牌图标（未命中返回空字符串）。
#[tauri::command]
pub fn otp_match_brand(issuer: String, token_type: String) -> Result<String, String> {
    if let Some(b) = match_brand(&issuer) {
        return Ok(b.to_string());
    }
    // Steam / Yandex 固定图标
    match token_type.as_str() {
        "Steam" => Ok("steam".to_string()),
        "Yandex" => Ok("yandex".to_string()),
        _ => Ok(String::new()),
    }
}

/// 生成指定令牌的当前验证码。
#[tauri::command]
pub fn otp_generate_code(token: OtpToken) -> Result<String, String> {
    let tt = algo::TokenType::from_str(&token.token_type)
        .ok_or_else(|| format!("未知令牌类型: {}", token.token_type))?;
    let ha = algo::HashAlgo::from_str(&token.algorithm)
        .ok_or_else(|| format!("未知算法: {}", token.algorithm))?;
    let now = chrono::Utc::now().timestamp_millis();
    Ok(algo::generate(
        tt,
        &token.secret,
        &token.pin,
        token.digits as usize,
        token.period as u64,
        token.counter as u64,
        ha,
        now,
    ))
}

/// 计算剩余秒数（倒计时用）。
#[tauri::command]
pub fn otp_remaining_seconds(period: i64) -> Result<i64, String> {
    let now = chrono::Utc::now().timestamp_millis();
    Ok(algo::remaining_seconds(period as u64, now) as i64)
}

/// 从图片文件识别二维码，返回解码出的文本（通常是 otpauth:// URI）。
/// 支持一次识别图中全部二维码，返回第一段有效文本。
#[tauri::command]
pub fn otp_scan_qr(path: String) -> Result<String, String> {
    let img = image::open(&path).map_err(|e| format!("读取图片失败: {e}"))?;
    let gray = img.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(gray);
    let grids = prepared.detect_grids();
    let mut results = Vec::new();
    for grid in grids {
        if let Ok((_meta, content)) = grid.decode() {
            results.push(content);
        }
    }
    results
        .into_iter()
        .next()
        .ok_or_else(|| "未在图片中识别到二维码".to_string())
}
