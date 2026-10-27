//! 证书管理（Q-005）：免费 ACME 证书申请 + 多节点部署 + 应用内调度。
//!
//! 架构：
//! - 数据模型（Certificate / DeployNode / Credential）持久化在 `get_data_dir()/certs/` 下三个 JSON 文件。
//! - 验证方式：DNS-01（lego --dns <provider>）与 HTTP-01（lego --http）；CNAME 委托需公网权威 DNS，暂不支持。
//! - 域名列表 → 部署方式：`Certificate.deploy_links` 保存关联；部署方式本身不保存域名。
//! - 部署：Windows 接收端 / API / Docker / SSH / Webhook / 阿里云 SSL / 腾讯云 SSL / 七牛 SSL。
//! - 调度：应用内 tokio 定时器，扫描到期并触发申请/部署。

use aes_gcm::aead::Aead;
use aes_gcm::KeyInit;
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Duration, TimeZone, Utc};
use hmac::Mac;
use hmac::Hmac;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Command;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::sync::Mutex;
use tauri::Manager;

#[allow(dead_code)]
type HmacSha1 = Hmac<Sha1>;

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub id: String,
    pub name: String,
    /// 凭据类型：alidns / tencentdns / cloudflare / qiniu / aliyun / linux / ssh / windows / custom
    #[serde(rename = "type")]
    pub cred_type: String,
    /// 类型相关的键值（敏感字段目前仅做 base64 轻量混淆，非强加密，详见 encrypt/decrypt）。
    pub data: HashMap<String, String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployNode {
    pub id: String,
    /// qiniu / aliyun / linux / windows
    #[serde(rename = "type")]
    pub node_type: String,
    pub name: String,
    /// 类型相关的配置键值（如 qiniu: access_key/secret_key/cdn_domains；linux: host/port/user/...）
    pub config: HashMap<String, String>,
    /// 提前 N 天部署（相对证书到期日）
    #[serde(default = "default_days")]
    pub deploy_before_days: i64,
    #[serde(default)]
    pub last_deploy_at: Option<String>,
    #[serde(default)]
    pub last_deploy_error: Option<String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployLink {
    /// 关联的部署方式（DeployNode.id）
    pub node_id: String,
    /// 域名列表关联该部署方式时指定的目标域名（CDN/负载均衡等节点使用）；为空时取证书首个域名
    #[serde(default)]
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    pub id: String,
    pub domains: Vec<String>,
    pub email: String,
    /// letsencrypt（生产） / letsencrypt-staging（测试）
    #[serde(default = "default_ca")]
    pub ca: String,
    /// DNS 授权模式使用：lego 的 DNS provider 标识，或 "custom"
    pub dns_provider: String,
    /// DNS 授权模式下关联的凭据；HTTP 验证模式可为空
    #[serde(default)]
    pub credential_id: String,
    /// 验证方式：dns（DNS-01）/ http（HTTP-01）
    #[serde(default = "default_validation")]
    pub validation: String,
    /// HTTP-01 监听端口（默认 80，需要外部可达）
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default = "default_days")]
    pub renew_before_days: i64,
    #[serde(default)]
    pub not_before: Option<String>,
    #[serde(default)]
    pub not_after: Option<String>,
    #[serde(default)]
    pub last_issue_at: Option<String>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub last_error: Option<String>,
    /// 域名列表 → 部署方式关联（新模型）
    #[serde(default)]
    pub deploy_links: Vec<DeployLink>,
    /// 兼容旧数据：仅保存关联的节点 ID
    #[serde(default)]
    pub deploy_node_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulerLogEntry {
    pub at: String,
    pub cert_id: String,
    pub action: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulerState {
    pub enabled: bool,
    pub interval_minutes: u64,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub log: Vec<SchedulerLogEntry>,
}

impl SchedulerState {
    fn push_log(&mut self, e: SchedulerLogEntry) {
        self.log.push(e);
        if self.log.len() > 200 {
            self.log.drain(0..self.log.len() - 200);
        }
    }
}

// 全局调度状态（UI 读取 / 手动触发）
pub struct CertScheduler {
    pub state: Mutex<SchedulerState>,
}

impl Default for CertScheduler {
    fn default() -> Self {
        CertScheduler {
            state: Mutex::new(SchedulerState {
                enabled: true,
                interval_minutes: 360, // 默认每 6 小时扫描
                ..Default::default()
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// 证书监控 + 通知
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}
fn default_https_port() -> u16 {
    443
}
fn default_notify_days() -> Vec<i64> {
    vec![30, 14, 7, 3, 1]
}

/// 外部域名/IP 的证书监控目标（与「申请中的证书」相互独立）：
/// 仅做 TLS 握手读取对端证书有效期，不校验证书链（自签证书同样可监控）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorTarget {
    pub id: String,
    /// 域名或 IP，可写成 `example.com:8443` / `https://example.com`
    pub host: String,
    #[serde(default = "default_https_port")]
    pub port: u16,
    #[serde(default)]
    pub note: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 剩余天数到达这些阈值时提醒（默认 30/14/7/3/1 天）
    #[serde(default = "default_notify_days")]
    pub notify_days: Vec<i64>,
    #[serde(default)]
    pub not_after: Option<String>,
    #[serde(default)]
    pub days_left: Option<i64>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub issuer: String,
    #[serde(default)]
    pub last_check_at: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    /// 上次提醒所命中的阈值（用于避免同一阈值反复提醒；续期后自动重置）
    #[serde(default)]
    pub last_notify_bucket: Option<i64>,
    #[serde(default)]
    pub last_notified_at: Option<String>,
}

/// 通知渠道：钉钉 / 企业微信 / Slack / Discord / Teams / 自定义 Webhook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifyChannel {
    pub id: String,
    #[serde(rename = "type")]
    pub channel_type: String,
    pub name: String,
    /// url（必填）；dingtalk 可选 secret（加签）；webhook 可选 token（Bearer）
    #[serde(default)]
    pub config: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub note: String,
}

fn default_days() -> i64 {
    30
}
fn default_ca() -> String {
    "letsencrypt".to_string()
}
fn default_status() -> String {
    "pending".to_string()
}
fn default_validation() -> String {
    "dns".to_string()
}
fn default_http_port() -> u16 {
    80
}

// ---------------------------------------------------------------------------
// 存储
// ---------------------------------------------------------------------------

fn certs_dir() -> PathBuf {
    let mut p = super::config::get_data_dir();
    p.push("certs");
    p
}

fn ensure_dir() -> PathBuf {
    let d = certs_dir();
    let _ = std::fs::create_dir_all(&d);
    d
}

fn load_json<T: serde::de::DeserializeOwned>(name: &str) -> Vec<T> {
    let p = ensure_dir().join(name);
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(v) = serde_json::from_str::<Vec<T>>(&s) {
            return v;
        }
    }
    Vec::new()
}

fn save_json<T: Serialize>(name: &str, v: &[T]) {
    let p = ensure_dir().join(name);
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let _ = std::fs::write(&p, s);
    }
}

fn gen_id(prefix: &str) -> String {
    // 毫秒时间戳 + 4 字节随机数：凭据/节点 id 被证书外键引用，
    // 同毫秒内连续创建（快速点击/脚本批量）不得碰撞
    let mut buf = [0u8; 4];
    let _ = getrandom::getrandom(&mut buf[..]);
    format!(
        "{}-{:x}-{:02x}{:02x}{:02x}{:02x}",
        prefix,
        Utc::now().timestamp_millis(),
        buf[0], buf[1], buf[2], buf[3]
    )
}

/// 读取域名列表，并把旧版 `deploy_node_ids` 自动迁移为 `deploy_links`。
///
/// 新模型：域名列表负责「关联部署方式」，部署方式本身不再保存域名；
/// 旧数据只有节点 ID，迁移时 target 留空，部署时回退到证书首个域名。
fn load_certs() -> Vec<Certificate> {
    let mut certs = load_json::<Certificate>("certificates.json");
    let mut changed = false;
    for c in certs.iter_mut() {
        if c.deploy_links.is_empty() && !c.deploy_node_ids.is_empty() {
            c.deploy_links = c
                .deploy_node_ids
                .iter()
                .map(|id| DeployLink {
                    node_id: id.clone(),
                    target: String::new(),
                })
                .collect();
            changed = true;
        }
        if c.deploy_node_ids.len() != c.deploy_links.len() {
            c.deploy_node_ids = c.deploy_links.iter().map(|l| l.node_id.clone()).collect();
            changed = true;
        }
    }
    if changed {
        save_json("certificates.json", &certs);
    }
    certs
}

// ---------------------------------------------------------------------------
// 敏感字段加密（AES-256-GCM，带 base64 混淆迁移兼容）
// ---------------------------------------------------------------------------

const CRED_ENCRYPTION_MARKER: &str = "ENC_V2:";

/// 获取或生成机器级加密密钥（32 字节）。
/// 密钥存储在 `~/.any-version/certs/.master_key`，权限由 OS 文件权限保护。
fn get_or_create_master_key() -> Result<[u8; 32], String> {
    let key_path = super::config::get_data_dir().join("certs").join(".master_key");
    if key_path.exists() {
        let encoded = std::fs::read_to_string(&key_path)
            .map_err(|e| format!("读取主密钥失败: {}", e))?;
        let bytes = B64.decode(encoded.trim())
            .map_err(|e| format!("主密钥 base64 解码失败: {}", e))?;
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        eprintln!("[cert] ⚠ 主密钥长度不匹配，重新生成");
    }
    // 生成新密钥
    let mut key = [0u8; 32];
    // 使用操作系统随机数生成器
    getrandom::getrandom(&mut key[..]).map_err(|e| format!("生成随机密钥失败: {}", e))?;
    // 确保父目录存在
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建密钥目录失败: {}", e))?;
    }
    // 写入 base64 编码的密钥
    let encoded = B64.encode(&key);
    std::fs::write(&key_path, encoded).map_err(|e| format!("写入主密钥失败: {}", e))?;
    Ok(key)
}

/// 使用 AES-256-GCM 加密字符串，返回 base64(nonce || ciphertext)。
fn encrypt_secret(plaintext: &str) -> Result<String, String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    let key = get_or_create_master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| format!("创建 AES 密码器失败: {:?}", e))?;
    // 12 字节随机 nonce
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes[..]).map_err(|e| format!("生成 nonce 失败: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes[..]);
    let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("AES 加密失败: {:?}", e))?;
    // 拼接 nonce + ciphertext，然后 base64
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(format!("{}{}", CRED_ENCRYPTION_MARKER, B64.encode(&combined)))
}

/// 解密 AES-256-GCM 加密的字符串。
fn decrypt_secret(ciphertext_b64: &str) -> Result<String, String> {
    if ciphertext_b64.is_empty() {
        return Ok(String::new());
    }
    // 兼容旧版 base64 混淆（无 ENC_V2: 前缀）
    if !ciphertext_b64.starts_with(CRED_ENCRYPTION_MARKER) {
        // 旧版：直接 base64 解码
        return B64.decode(ciphertext_b64)
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .map_err(|e| format!("旧版 base64 解码失败: {}", e));
    }
    let combined_b64 = &ciphertext_b64[CRED_ENCRYPTION_MARKER.len()..];
    let combined = B64.decode(combined_b64)
        .map_err(|e| format!("AES 密文 base64 解码失败: {}", e))?;
    if combined.len() < 13 {
        return Err("AES 密文太短".to_string());
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let key = get_or_create_master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| format!("创建 AES 密码器失败: {:?}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES 解密失败（密钥不匹配或数据损坏）: {:?}", e))?;
    String::from_utf8(plaintext)
        .map_err(|e| format!("AES 解密结果 UTF-8 解码失败: {}", e))
}

// 持久化凭据时对 value 做混淆；读取时还原。
fn encrypt_credential(mut c: Credential) -> Credential {
    for v in c.data.values_mut() {
        if !v.is_empty() {
            if let Ok(encrypted) = encrypt_secret(v) {
                *v = encrypted;
            } else {
                // 加密失败时回退到旧版 base64（保证不丢数据）
                *v = B64.encode(v.as_bytes());
            }
        }
    }
    c
}
fn decrypt_credential(mut c: Credential) -> Credential {
    for v in c.data.values_mut() {
        if !v.is_empty() {
            if let Ok(decrypted) = decrypt_secret(v) {
                *v = decrypted;
            }
            // 解密失败时保留原值（可能是旧版 base64，已兼容）
        }
    }
    c
}

// ---------------------------------------------------------------------------
// lego 集成
// ---------------------------------------------------------------------------

/// 运行时检测真实 CPU 架构（非编译目标）。
fn cpu_arch() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        if let Ok(a) = std::env::var("PROCESSOR_ARCHITECTURE") {
            if a.eq_ignore_ascii_case("ARM64") {
                return "arm64";
            }
        }
    }
    "x64"
}

fn lego_candidates() -> Vec<String> {
    let arch = cpu_arch(); // "x64" / "arm64"
    // 版本化目录命名：lego_v5.3.1_windows_amd64 / windows_arm64
    // 同时兼容 amd64/x64 两种写法
    let arch_keywords: Vec<&str> = if arch == "arm64" {
        vec!["arm64"]
    } else {
        vec!["amd64", "x64"]
    };

    let mut cand = Vec::new();

    let bases = super::utils::get_bin_search_dirs();

    // 在给定目录中查找 lego 候选：版本化子目录、扁平命名
    fn scan(dir: &PathBuf, arch_keywords: &[&str], cand: &mut Vec<String>) {
        if !dir.is_dir() {
            return;
        }
        // 3a. 版本化子目录：lego_*_windows_{arch}/lego.exe
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if let Some(fname) = p.file_name() {
                        let name = fname.to_string_lossy().to_ascii_lowercase();
                        if name.starts_with("lego")
                            && arch_keywords.iter().any(|k| name.contains(k))
                        {
                            let exe = p.join("lego.exe");
                            if exe.is_file() {
                                cand.push(exe.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
        // 3b. 扁平命名：lego-amd64.exe / lego-x64.exe / lego-arm64.exe
        for k in arch_keywords {
            let flat = dir.join(format!("lego-{}.exe", k));
            if flat.is_file() {
                cand.push(flat.to_string_lossy().to_string());
            }
        }
        // 3c. 扁平 lego.exe
        let flat2 = dir.join("lego.exe");
        if flat2.is_file() {
            cand.push(flat2.to_string_lossy().to_string());
        }
    }

    for b in bases {
        // 旧布局：bin/lego_*_windows_{arch}/ 直接在 bin 下
        scan(&b, &arch_keywords, &mut cand);
        // 新布局：bin/lego/lego_*_windows_{arch}/ 多一层 lego 目录
        scan(&b.join("lego"), &arch_keywords, &mut cand);
    }

    // 4. 系统 PATH
    cand.push("lego.exe".to_string());
    cand.push("lego".to_string());

    cand
}

fn find_lego() -> String {
    let mut tried = Vec::new();
    for c in lego_candidates() {
        tried.push(c.clone());
        if std::path::Path::new(&c).exists() {
            eprintln!("[cert] 使用 lego: {}", c);
            return c;
        }
    }
    eprintln!("[cert] 未找到 lego，已尝试候选: {:?}", tried);
    "lego".to_string()
}

fn ca_server(ca: &str) -> Option<String> {
    match ca {
        "letsencrypt-staging" => Some("https://acme-staging-v02.api.letsencrypt.org/directory".to_string()),
        "letsencrypt" | "" => None, // 默认生产
        other => Some(other.to_string()), // 自定义 ACME 目录 URL
    }
}

/// 依据 dns_provider + 凭据 data 组装 lego 所需环境变量。
fn lego_env(dns_provider: &str, cred: &Credential) -> Vec<(String, String)> {
    let d = &cred.data;
    if dns_provider == "custom" {
        // custom：凭据 data 的键即 lego 环境变量名
        return d.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    }
    let mut env: Vec<(String, String)> = Vec::new();
    let get = |k: &str| d.get(k).cloned().unwrap_or_default();
    match dns_provider {
        "alidns" => {
            env.push(("ALICLOUD_ACCESS_KEY".into(), get("access_key")));
            env.push(("ALICLOUD_SECRET_KEY".into(), get("secret_key")));
            if let Some(r) = d.get("region") {
                env.push(("ALICLOUD_REGION".into(), r.clone()));
            }
        }
        "route53" => {
            env.push(("AWS_ACCESS_KEY_ID".into(), get("AWS_ACCESS_KEY_ID")));
            env.push(("AWS_SECRET_ACCESS_KEY".into(), get("AWS_SECRET_ACCESS_KEY")));
            if let Some(r) = d.get("AWS_REGION") {
                env.push(("AWS_REGION".into(), r.clone()));
            }
        }
        "godaddy" => {
            env.push(("GODADDY_API_KEY".into(), get("GODADDY_API_KEY")));
            env.push(("GODADDY_API_SECRET".into(), get("GODADDY_API_SECRET")));
        }
        "cloudflare" => {
            env.push(("CLOUDFLARE_DNS_API_TOKEN".into(), get("api_token")));
        }
        "dnspod" => {
            env.push(("DNSPOD_API_KEY".into(), get("DNSPOD_API_KEY")));
        }
        "huaweicloud" => {
            env.push(("HUAWEICLOUD_ACCESS_KEY_ID".into(), get("HUAWEICLOUD_ACCESS_KEY_ID")));
            env.push(("HUAWEICLOUD_SECRET_ACCESS_KEY".into(), get("HUAWEICLOUD_SECRET_ACCESS_KEY")));
            if let Some(r) = d.get("HUAWEICLOUD_REGION") {
                env.push(("HUAWEICLOUD_REGION".into(), r.clone()));
            }
        }
        "baiducloud" => {
            env.push(("BAIDUCLOUD_ACCESS_KEY_ID".into(), get("BAIDUCLOUD_ACCESS_KEY_ID")));
            env.push(("BAIDUCLOUD_SECRET_ACCESS_KEY".into(), get("BAIDUCLOUD_SECRET_ACCESS_KEY")));
        }
        "dnsla" => {
            env.push(("DNSLA_API_ID".into(), get("DNSLA_API_ID")));
            env.push(("DNSLA_API_SECRET".into(), get("DNSLA_API_SECRET")));
        }
        "westcn" => {
            env.push(("WESTCN_USERNAME".into(), get("WESTCN_USERNAME")));
            env.push(("WESTCN_API_PASSWORD".into(), get("WESTCN_API_PASSWORD")));
        }
        "volcengine" => {
            env.push(("VOLCENGINE_ACCESS_KEY".into(), get("VOLCENGINE_ACCESS_KEY")));
            env.push(("VOLCENGINE_SECRET_KEY".into(), get("VOLCENGINE_SECRET_KEY")));
        }
        "tencentdns" => {
            env.push(("TENCENTCLOUD_SECRET_ID".into(), get("secret_id")));
            env.push(("TENCENTCLOUD_SECRET_KEY".into(), get("secret_key")));
        }
        _ => {
            // 未知 provider：把 data 原样当作环境变量尝试
            for (k, v) in d.iter() {
                env.push((k.clone(), v.clone()));
            }
        }
    }
    env
}

/// lego 数据目录布局：`--path` 指向的是**数据根目录**，
/// 证书位于 `<path>/certificates/`、账号位于 `<path>/accounts/`
/// （实测 lego v5.3.1；v4 同理，只是默认 `--path` 恰为 `~/.lego`）。
/// 历史实现误以为根目录下还有一层 `.lego`（那是默认路径自身的形态），
/// 导致 lego 成功签发后仍读不到 PEM，报「未找到生成的 PEM 文件」。
fn lego_cert_dirs(cert_dir: &PathBuf) -> Vec<PathBuf> {
    let mut dirs = vec![cert_dir.join("certificates")];
    // 兼容历史错误布局（曾按 `<path>/.lego/certificates` 读取）
    dirs.push(cert_dir.join(".lego").join("certificates"));
    // 兜底：任意一级子目录下的 certificates/（防 lego 后续版本再改布局）
    if let Ok(entries) = std::fs::read_dir(cert_dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let sub = p.join("certificates");
                if sub.is_dir() && !dirs.contains(&sub) {
                    dirs.push(sub);
                }
            }
        }
    }
    dirs
}

/// lego 以域名作为证书文件名，非法字符替换为 `_`
/// （实测 `*.seemove.cn` → `_.seemove.cn`）。
fn lego_cert_file_stem(domain: &str) -> String {
    domain
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 在候选文件名中挑选目标：优先精确命中 `want`，否则取排序后的第一个。
/// 排序保证同一目录多次读取结果稳定（`read_dir` 的返回顺序不保证）。
fn pick_name(
    names: &[String],
    want: Option<String>,
    pred: impl Fn(&str) -> bool,
) -> Option<String> {
    let mut cands: Vec<&String> = names.iter().filter(|n| pred(n.as_str())).collect();
    cands.sort();
    if let Some(w) = want {
        if let Some(m) = cands.iter().find(|n| n.as_str() == w) {
            return Some((*m).clone());
        }
    }
    cands.first().map(|n| (*n).clone())
}

/// lego 生成的三个关键文件：内容 + 各自在证书目录中的**原始文件名**
/// （导出 zip 时要按原名附一份，便于与本地证书目录对照）。
struct LegoPems {
    crt_file: String,
    crt: String,
    key_file: String,
    key: String,
    issuer_file: Option<String>,
    issuer: String,
}

/// 读取某个 lego 证书目录下的 PEM 三段（cert / key / issuer）。
/// `prefer_domain` 用于同目录存在多份证书时精确匹配，
/// 否则可能取到别的域名的证书并部署出去。
fn read_pems_in(
    cert_store: &PathBuf,
    prefer_domain: Option<&str>,
) -> Option<LegoPems> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(cert_store) {
        for e in entries.flatten() {
            names.push(e.file_name().to_string_lossy().to_string());
        }
    }
    if names.is_empty() {
        return None;
    }
    let stem = prefer_domain.map(lego_cert_file_stem);
    let crt = pick_name(
        &names,
        stem.as_ref().map(|s| format!("{}.crt", s)),
        |n| n.ends_with(".crt") && !n.contains("issuer"),
    )?;
    let key = pick_name(
        &names,
        stem.as_ref().map(|s| format!("{}.key", s)),
        |n| n.ends_with(".key"),
    )?;
    let issuer = pick_name(
        &names,
        stem.as_ref().map(|s| format!("{}.issuer.crt", s)),
        |n| n.ends_with(".issuer.crt"),
    );

    let read = |n: &String| std::fs::read_to_string(cert_store.join(n)).unwrap_or_default();
    Some(LegoPems {
        crt: read(&crt),
        crt_file: crt,
        key: read(&key),
        key_file: key,
        issuer: issuer.as_ref().map(read).unwrap_or_default(),
        issuer_file: issuer,
    })
}

/// 读取某证书 id 下 lego 生成的 PEM 三段，自动适配 lego 的数据目录布局。
fn read_lego_pems(cert_dir: &PathBuf, prefer_domain: Option<&str>) -> Option<LegoPems> {
    for store in lego_cert_dirs(cert_dir) {
        if let Some(pems) = read_pems_in(&store, prefer_domain) {
            return Some(pems);
        }
    }
    None
}

/// 证书存储目录里现有证书文件的指纹（路径 + 大小 + 修改时间）。
///
/// 用来判断一次 lego 调用到底有没有签发新证书：lego 在「还没到续期时间」时会
/// 跳过续期并以退出码 0 结束（v5.3.1 实测），此时磁盘上一个文件都不会变。
fn cert_store_fingerprint(cert_dir: &PathBuf) -> Vec<(String, u64, u128)> {
    let mut out: Vec<(String, u64, u128)> = Vec::new();
    for store in lego_cert_dirs(cert_dir) {
        let entries = match std::fs::read_dir(&store) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !(name.ends_with(".crt") || name.ends_with(".key")) {
                continue;
            }
            let meta = e.metadata().ok();
            let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let mtime = meta
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            out.push((format!("{}/{}", store.display(), name), len, mtime));
        }
    }
    out.sort();
    out
}

/// 证书目录里是否已有可用证书 —— 决定 lego 走「首次签发」还是「续期」语义。
///
/// 之前用 `last_issue_at.is_some()` 判断，但「跳过续期」也会写 last_issue_at，
/// 跳过一次之后判定就永远偏了；只看磁盘上有没有证书才可靠。
fn has_existing_cert(cert_dir: &PathBuf) -> bool {
    lego_cert_dirs(cert_dir).iter().any(|dir| {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.file_name().to_string_lossy().ends_with(".crt"))
            })
            .unwrap_or(false)
    })
}

/// `force` = 手动「申请」：即使还没到续期时间也要求 lego 重签。
fn run_lego(
    cert: &Certificate,
    cred: Option<&Credential>,
    renew: bool,
    force: bool,
) -> Result<(String, String, String), String> {
    let lego = find_lego();
    let cert_dir = ensure_dir().join(&cert.id);
    let _ = std::fs::create_dir_all(&cert_dir);

    // lego v5：--email/--accept-tos/--key-type/--server/--dns/--domains/--path
    // 均为 `run` 子命令旗标（非全局旗标），必须放在 `run` 之后，否则会被当成
    // 全局旗标解析而报 "flag provided but not defined: -email"。
    // 续期在 v5 中没有独立 `renew` 子命令，重新执行 `run` 即可，天数字段为 --renew-days。
    let mut cmd = Command::new(&lego);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd.arg("run");
    cmd.arg("--email").arg(&cert.email);
    cmd.arg("--accept-tos");
    cmd.arg("--key-type").arg("rsa4096");
    cmd.arg("--path").arg(&cert_dir);
    if let Some(s) = ca_server(&cert.ca) {
        cmd.arg("--server").arg(s);
    }
    match cert.validation.as_str() {
        "http" => {
            if cert.domains.iter().any(|d| d.starts_with("*.")) {
                return Err("HTTP-01 验证不支持泛域名，请改用 DNS 授权".into());
            }
            cmd.arg("--http");
            // lego v5 使用 --http.address（默认 ":80"），不是 --http.port
            cmd.arg("--http.address")
                .arg(format!(":{}", cert.http_port.max(1)));
        }
        "cname" => {
            return Err(
                "免 DNS 授权（CNAME 委托）需要一个公网可达的权威 DNS 服务；桌面版无法单独提供，\
                 请改用 DNS 授权或 HTTP 验证"
                    .into(),
            );
        }
        _ => {
            let cred = cred.ok_or_else(|| "DNS 验证需要关联 DNS 授权凭据".to_string())?;
            cmd.arg("--dns").arg(&cert.dns_provider);
            let envs = lego_env(&cert.dns_provider, cred);
            for (k, v) in &envs {
                cmd.env(k, v);
            }
            if !envs.iter().any(|(k, _)| k == "LEGO_DNS_RESOLVERS") {
                cmd.arg("--dns.resolvers").arg("223.5.5.5:53,114.114.114.114:53,8.8.8.8:53");
            }
            // 不要用「递归解析器能不能查到」当传播判据：
            // 国内权威 DNS（如阿里云 hichina）SOA 默认 TTL 是 600s —— 记录刚写入时，
            // 递归解析器只要有一次查到 NXDOMAIN 就会缓存 10 分钟，而 lego 等传播只等
            // 约 2 分钟（provider 自带超时），于是整个窗口都命中缓存，最后必然报
            // `time limit exceeded: ... NS 223.5.5.5:53 returned NXDOMAIN`。
            // 保留权威 NS 校验（那几台是实时同步的，几秒内就能查到）——真实校验本来就是
            // Let's Encrypt 从多个视角直连权威服务器做的，本地这项只是提前自查。
            cmd.arg("--dns.propagation.disable-rns");
        }
    }
    for dom in &cert.domains {
        cmd.arg("--domains").arg(dom);
    }
    if renew {
        cmd.arg("--renew-days").arg(cert.renew_before_days.to_string());
    }
    if force {
        // lego 在「剩余天数 > --renew-days」时会直接跳过续期，且**退出码仍是 0**
        // （v5.3.1 实测：`INFO Skip renewal: ...` + `EXIT=0`）。后端过去把它当成
        // 签发成功，于是刷新后数据毫无变化 —— 用户看到的就是「点申请没反应」。
        // 手动申请要走真实续期，必须加 --renew-force 绕过到期判断。
        cmd.arg("--renew-force");
    }

    // 打印完整命令行，便于在终端排查
    let cmd_dbg = {
        let mut s = lego.clone();
        for a in cmd.get_args() {
            s.push(' ');
            s.push_str(&a.to_string_lossy());
        }
        s
    };
    eprintln!("[cert] lego 命令: {}{} run",
        if renew { "(renew) " } else { "" }, cmd_dbg);

    let out = cmd.output().map_err(|e| format!(
        "启动 lego 失败: {}（lego 路径={}；请确认 lego 在 bin/ 版本化目录、PATH 或数据目录的 bin/ 中，且架构匹配当前系统）",
        e, lego
    ))?;
    if !out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        // 同时打印到控制台，方便在运行 app 的终端查看完整错误
        eprintln!("[cert] lego 执行失败 ({})\n--- stdout ---\n{}\n--- stderr ---\n{}",
            out.status, stdout, stderr);
        return Err(format!(
            "lego 执行失败 ({}):\n{}\n{}",
            out.status, stdout, stderr
        ));
    }
    eprintln!("[cert] lego 执行成功 ({}): {}",
        if renew { "renew" } else { "issue" }, cert_dir.display());
    let prefer = cert.domains.first().map(|s| s.as_str());
    let stores = lego_cert_dirs(&cert_dir);
    let pems = read_lego_pems(&cert_dir, prefer).ok_or_else(|| {
        let listed: Vec<String> = stores
            .iter()
            .map(|d| {
                let files: Vec<String> = std::fs::read_dir(d)
                    .map(|rd| {
                        rd.flatten()
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                format!("{}（{} 个文件{}）", d.display(), files.len(),
                    if files.is_empty() { String::new() } else { format!(": {}", files.join(", ")) })
            })
            .collect();
        format!("lego 执行成功但未找到生成的 PEM 文件；已查找: {}", listed.join("；"))
    })?;
    Ok((pems.crt, pems.key, pems.issuer))
}

// ---------------------------------------------------------------------------
// 部署
// ---------------------------------------------------------------------------

fn find_cert<'a>(certs: &'a [Certificate], id: &str) -> Option<&'a Certificate> {
    certs.iter().find(|c| c.id == id)
}
fn find_node<'a>(nodes: &'a [DeployNode], id: &str) -> Option<&'a DeployNode> {
    nodes.iter().find(|n| n.id == id)
}

async fn deploy_to_node(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    match node.node_type.as_str() {
        // Windows 接收端 / API / Docker 拉取，均走独立的 cert-receiver /push
        "windows" | "api" | "docker" => deploy_http_push(node, cert, pems, target).await,
        "ssh" => deploy_ssh(node, cert, pems, target),
        // 旧版 linux 节点保留原有「证书路径 + 私钥路径 + 重载命令」语义
        "linux" => deploy_linux(node, pems),
        "webhook" => deploy_webhook(node, cert, pems).await,
        "baota" => deploy_baota(node, cert, pems, target).await,
        "one_panel" => deploy_one_panel(node, cert, pems, target).await,
        "synology" => deploy_synology(node, cert, pems, target).await,
        "upyun" | "upyun_ssl" => deploy_upyun(node, cert, pems, target).await,
        "baidu_cdn" => deploy_baidu_cdn(node, cert, pems, target).await,
        "huawei_cdn" => deploy_huawei_cdn(node, cert, pems, target).await,
        "volcengine_cdn" => deploy_volcengine_cdn(node, cert, pems, target).await,
        "doge_cdn" => deploy_doge_cdn(node, cert, pems, target).await,
        "aliyun_oss" => deploy_aliyun_oss(node, cert, pems, target).await,
        "aliyun_fc" => deploy_aliyun_fc(node, cert, pems, target).await,
        "aliyun" | "aliyun_ssl" => deploy_aliyun_ssl(node, cert, pems, target).await,
        "aliyun_cdn" => deploy_aliyun_cdn(node, cert, pems, target).await,
        "aliyun_dcdn" => deploy_aliyun_dcdn(node, cert, pems, target).await,
        "aliyun_slb" => deploy_aliyun_slb(node, cert, pems, target).await,
        "aliyun_alb" => deploy_aliyun_alb(node, cert, pems, target).await,
        "aliyun_nlb" => deploy_aliyun_nlb(node, cert, pems, target).await,
        "aliyun_api_gateway" => deploy_aliyun_api_gateway(node, cert, pems, target).await,
        "tencent_ssl" => deploy_tencent_ssl(node, cert, pems, target).await,
        "tencent_clb" => deploy_tencent_clb(node, cert, pems, target).await,
        "tencent_cdn" => deploy_tencent_cdn(node, cert, pems, target).await,
        "qiniu" | "qiniu_ssl" => deploy_qiniu_ssl(node, cert, pems, target).await,
        other => Err(format!("未知部署节点类型: {}", other)),
    }
}

/// 把接收端 URL 归一化为「基础地址」（不含 /push 段、无结尾斜杠）。
/// 兼容用户三种输入：`http://h:9000` / `http://h:9000/` / `http://h:9000/push`。
/// 部署与连通性测试必须从同一基础地址派生（/push 与 /health），
/// 否则按占位提示输入完整 /push URL 时，连通测试会打到 `.../push/health`（404）。
fn windows_base_url(url: &str) -> String {
    let mut u = url.trim().to_string();
    while u.ends_with('/') {
        u.pop();
    }
    if let Some(idx) = u.rfind('/') {
        if u[idx..].eq_ignore_ascii_case("/push") {
            u.truncate(idx);
        }
    }
    u
}

fn cfg(node: &DeployNode, key: &str) -> String {
    node.config.get(key).cloned().unwrap_or_default().trim().to_string()
}

/// 通用云厂商 AK/SK（字段名与阿里云/百度/华为/火山一致）。
fn ak_sk(node: &DeployNode, provider: &str) -> Result<(String, String), String> {
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if ak.is_empty() || sk.is_empty() {
        return Err(format!("{} 节点缺少 AccessKey ID / Secret", provider));
    }
    Ok((ak, sk))
}

fn cert_name_for(cert: &Certificate, target: &str, fallback: &str) -> String {
    let base = if !target.trim().is_empty() {
        target.trim()
    } else {
        cert.domains.first().map(|s| s.as_str()).unwrap_or(fallback)
    };
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.push('-');
    name.push_str(&Utc::now().format("%Y%m%d%H%M%S").to_string());
    name.chars().take(63).collect()
}

async fn deploy_http_push(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let base = windows_base_url(&cfg(node, "url"));
    let token = cfg(node, "token");
    if base.is_empty() {
        return Err(format!("{} 节点缺少 url", node.node_type));
    }
    if token.is_empty() {
        return Err("节点缺少共享 Token（需与 cert-receiver 的 token 一致）".into());
    }
    let push_url = format!("{}/push", base);
    let body = serde_json::json!({
        "certificate_id": cert.id,
        "domain": if target.is_empty() { cert.domains.first().cloned().unwrap_or_default() } else { target.to_string() },
        "domains": cert.domains,
        "cert": pems.0,
        "key": pems.1,
        "ca": pems.2,
        "expired_time": cert.not_after.clone().unwrap_or_default(),
        "cert_name": cert_name_for(cert, target, &cert.id),
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&push_url)
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("推送证书到接收端失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("接收端返回 {}: {}", status, text.trim()));
    }
    Ok(())
}

fn deploy_linux(node: &DeployNode, pems: &(String, String, String)) -> Result<(), String> {
    let host = node.config.get("host").cloned().unwrap_or_default();
    let port = node.config.get("port").cloned().unwrap_or_else(|| "22".into());
    let user = node.config.get("user").cloned().unwrap_or_default();
    let cert_path = node.config.get("cert_path").cloned().unwrap_or_default();
    let key_path = node.config.get("key_path").cloned().unwrap_or_default();
    let reload = node.config.get("reload_cmd").cloned().unwrap_or_default();
    if host.is_empty() || cert_path.is_empty() || key_path.is_empty() {
        return Err("linux 节点缺少 host/cert_path/key_path".into());
    }
    // 使用隔离安全子目录，避免私钥在通用临时文件目录下被其它非特权进程读取
    let isolate_dir = std::env::temp_dir().join(format!(".cert_deploy_{}", node.id));
    let _ = std::fs::create_dir_all(&isolate_dir);
    let tmp = isolate_dir.join("cert.pem");
    let tmp_key = isolate_dir.join("key.pem");
    std::fs::write(&tmp, &pems.0).map_err(|e| e.to_string())?;
    std::fs::write(&tmp_key, &pems.1).map_err(|e| e.to_string())?;

    let target = format!("{}@{}", user, host);
    let mut scp_cert_cmd = Command::new("scp");
    #[cfg(windows)]
    scp_cert_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let scp_cert = scp_cert_cmd
        .args(["-P", &port, tmp.to_str().unwrap(), &format!("{}:{}", target, cert_path)])
        .output();
    let mut scp_key_cmd = Command::new("scp");
    #[cfg(windows)]
    scp_key_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let scp_key = scp_key_cmd
        .args(["-P", &port, tmp_key.to_str().unwrap(), &format!("{}:{}", target, key_path)])
        .output();
    let _ = std::fs::remove_dir_all(&isolate_dir);
    match scp_cert {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Err(format!(
                "scp 证书失败 ({}): {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ))
        }
        Err(e) => return Err(format!("scp 证书失败: {}", e)),
    }
    match scp_key {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Err(format!(
                "scp 私钥失败 ({}): {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ))
        }
        Err(e) => return Err(format!("scp 私钥失败: {}", e)),
    }
    if !reload.is_empty() {
        let mut ssh_cmd = Command::new("ssh");
        #[cfg(windows)]
        ssh_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        match ssh_cmd.args(["-p", &port, &target, &reload]).output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return Err(format!(
                    "远程重载命令失败 ({}): {}",
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                ))
            }
            Err(e) => return Err(format!("无法执行 ssh: {}", e)),
        }
    }
    Ok(())
}

fn remote_safe_name(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "cert".into()
    } else {
        out
    }
}

struct SshAuth {
    password: Option<String>,
    key: Option<String>,
}

fn ssh_auth(node: &DeployNode) -> SshAuth {
    let method = cfg(node, "auth_method");
    if method == "password" {
        SshAuth {
            password: Some(cfg(node, "password")),
            key: None,
        }
    } else {
        let key = cfg(node, "private_key");
        SshAuth {
            password: None,
            key: if key.is_empty() { None } else { Some(key) },
        }
    }
}

fn ssh_cmd_args(node: &DeployNode, auth: &SshAuth, remote_cmd: &str) -> Result<(Command, String), String> {
    let host = cfg(node, "host");
    let port = {
        let p = cfg(node, "port");
        if p.is_empty() { "22".to_string() } else { p }
    };
    let user = {
        let u = cfg(node, "user");
        if u.is_empty() { "root".to_string() } else { u }
    };
    if host.is_empty() {
        return Err("ssh 节点缺少 host".into());
    }
    let target = format!("{}@{}", user, host);
    let mut cmd = if let Some(pw) = &auth.password {
        if pw.is_empty() {
            return Err("ssh 密码认证需要填写 password".into());
        }
        if Command::new("sshpass").arg("-V").output().is_err() {
            return Err("密码认证需要系统安装 sshpass；建议改用私钥认证".into());
        }
        let mut c = Command::new("sshpass");
        c.arg("-p").arg(pw).arg("ssh");
        c
    } else {
        Command::new("ssh")
    };
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    if let Some(key) = &auth.key {
        cmd.arg("-i").arg(key);
    }
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-p").arg(port);
    cmd.arg(&target).arg(remote_cmd);
    Ok((cmd, target))
}

fn scp_upload(node: &DeployNode, auth: &SshAuth, local: &str, remote: &str) -> Result<(), String> {
    let host = cfg(node, "host");
    let port = {
        let p = cfg(node, "port");
        if p.is_empty() { "22".to_string() } else { p }
    };
    let user = {
        let u = cfg(node, "user");
        if u.is_empty() { "root".to_string() } else { u }
    };
    let target = format!("{}@{}", user, host);
    let mut cmd = if let Some(pw) = &auth.password {
        let mut c = Command::new("sshpass");
        c.arg("-p").arg(pw).arg("scp");
        c
    } else {
        Command::new("scp")
    };
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    if let Some(key) = &auth.key {
        cmd.arg("-i").arg(key);
    }
    cmd.arg("-P").arg(port);
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg(local).arg(format!("{}:{}", target, remote));
    let out = cmd
        .output()
        .map_err(|e| format!("scp 失败: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "scp 失败 ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// SSH 部署（对齐 ohttps）：前置命令 → 上传 cert.key/fullchain.cer → 后置命令。
fn deploy_ssh(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let auth = ssh_auth(node);
    let cert_dir = {
        let d = cfg(node, "cert_dir");
        if d.is_empty() { "/etc/nginx/certificates".to_string() } else { d }
    };
    let name = remote_safe_name(if target.trim().is_empty() {
        cert.domains.first().map(|s| s.as_str()).unwrap_or(&cert.id)
    } else {
        target
    });
    let remote_dir = format!("{}/{}", cert_dir.trim_end_matches('/'), name);
    let quoted_dir = remote_dir.replace('\'', "'\\''");

    let pre = cfg(node, "pre_command");
    if !pre.is_empty() {
        let (mut cmd, _) = ssh_cmd_args(node, &auth, &pre)?;
        let out = cmd.output().map_err(|e| format!("执行前置命令失败: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "前置命令失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }

    let (mut mkdir_cmd, _) = ssh_cmd_args(node, &auth, &format!("mkdir -p '{}'", quoted_dir))?;
    let mkdir = mkdir_cmd.output().map_err(|e| format!("创建远端目录失败: {}", e))?;
    if !mkdir.status.success() {
        return Err(format!(
            "创建远端目录失败 ({}): {}",
            mkdir.status,
            String::from_utf8_lossy(&mkdir.stderr).trim()
        ));
    }

    let isolate_dir = std::env::temp_dir().join(format!(
        ".cert_deploy_{}_{}",
        node.id,
        Utc::now().timestamp_millis()
    ));
    std::fs::create_dir_all(&isolate_dir).map_err(|e| e.to_string())?;
    let local_cert = isolate_dir.join("fullchain.cer");
    let local_key = isolate_dir.join("cert.key");
    std::fs::write(&local_cert, &pems.0).map_err(|e| e.to_string())?;
    std::fs::write(&local_key, &pems.1).map_err(|e| e.to_string())?;
    let upload = scp_upload(node, &auth, &local_cert.to_string_lossy(), &format!("{}/fullchain.cer", remote_dir))
        .and_then(|_| scp_upload(node, &auth, &local_key.to_string_lossy(), &format!("{}/cert.key", remote_dir)));
    let _ = std::fs::remove_dir_all(&isolate_dir);
    upload?;

    let post = cfg(node, "post_command");
    if !post.is_empty() {
        let (mut cmd, _) = ssh_cmd_args(node, &auth, &post)?;
        let out = cmd.output().map_err(|e| format!("执行后置命令失败: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "后置命令失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
}

async fn deploy_webhook(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
) -> Result<(), String> {
    let url = cfg(node, "url");
    if url.is_empty() {
        return Err("webhook 节点缺少 url".into());
    }
    let token = cfg(node, "token");
    let ts = Utc::now().timestamp_millis();
    let sign = format!("{:x}", md5::compute(format!("{}:{}", ts, token).as_bytes()));
    let body = serde_json::json!({
        "timestamp": ts,
        "payload": {
            "certificateName": cert.id,
            "certificateDomains": cert.domains,
            "certificateCertKey": pems.1,
            "certificateFullchainCerts": pems.0,
            "certificateExpireAt": cert.not_after.clone().unwrap_or_default(),
        },
        "sign": sign,
    });
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用 webhook 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("webhook 返回 {}: {}", status, text.trim()));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if v.get("success").and_then(|x| x.as_bool()) == Some(false) {
            return Err(format!(
                "webhook 返回失败: {}",
                v.get("msg").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    Ok(())
}

// --- 宝塔面板：网站 SSL / 面板 SSL ---

/// 面板地址归一化：补全 scheme、去掉结尾斜杠。
fn http_base_url(raw: &str) -> String {
    let mut u = raw.trim().trim_end_matches('/').to_string();
    if !u.is_empty() && !u.starts_with("http://") && !u.starts_with("https://") {
        u = format!("http://{}", u);
    }
    u
}

fn baota_sign(api_sk: &str) -> (String, String) {
    // 宝塔 API：request_token = md5(request_time + md5(api_sk))
    let time = Utc::now().timestamp().to_string();
    let sk_md5 = format!("{:x}", md5::compute(api_sk.as_bytes()));
    let token = format!("{:x}", md5::compute(format!("{}{}", time, sk_md5).as_bytes()));
    (time, token)
}

async fn baota_post(
    base: &str,
    path_query: &str,
    mut form: Vec<(String, String)>,
    api_sk: &str,
) -> Result<serde_json::Value, String> {
    let (time, token) = baota_sign(api_sk);
    form.push(("request_time".into(), time));
    form.push(("request_token".into(), token));
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}{}", base, path_query))
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("调用宝塔面板失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    serde_json::from_str(&text).map_err(|_| {
        format!(
            "宝塔返回非 JSON ({}): {}",
            status,
            text.chars().take(300).collect::<String>().trim()
        )
    })
}

fn baota_check(v: &serde_json::Value, action: &str) -> Result<(), String> {
    match v.get("status").and_then(|x| x.as_bool()) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "宝塔 {} 失败: {}",
            action,
            v.get("msg").and_then(|x| x.as_str()).unwrap_or("未知错误")
        )),
        // 部分接口不返回 status（直接返回数据），视为成功
        None => Ok(()),
    }
}

fn baota_api_sk(node: &DeployNode) -> String {
    let sk = cfg(node, "api_sk");
    if sk.is_empty() { cfg(node, "api_key") } else { sk }
}

async fn deploy_baota(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let base = http_base_url(&cfg(node, "url"));
    let api_sk = baota_api_sk(node);
    if base.is_empty() || api_sk.is_empty() {
        return Err("宝塔节点缺少面板地址或 API 密钥".into());
    }
    if cfg(node, "scope") == "panel" {
        // 面板 SSL：不同版本字段名不一致，同时提交常见别名（多余字段会被忽略）
        let form = vec![
            ("key".into(), pems.1.clone()),
            ("csr".into(), pems.0.clone()),
            ("cert".into(), pems.0.clone()),
            ("privateKey".into(), pems.1.clone()),
            ("publicKey".into(), pems.0.clone()),
        ];
        let v = baota_post(&base, "/config?action=SetPanelSSL", form, &api_sk).await?;
        return baota_check(&v, "面板 SSL");
    }
    // 网站 SSL：站点名取域名列表关联时填写的目标域名
    let site = if target.trim().is_empty() {
        cert.domains.first().cloned().unwrap_or_default()
    } else {
        target.trim().to_string()
    };
    if site.is_empty() {
        return Err("宝塔网站 SSL 需要在域名列表关联时填写站点域名".into());
    }
    let form = vec![
        ("siteName".into(), site.clone()),
        ("key".into(), pems.1.clone()),
        ("csr".into(), pems.0.clone()),
        ("type".into(), "1".into()),
    ];
    let v = baota_post(&base, "/site?action=SetSSL", form, &api_sk).await?;
    baota_check(&v, "网站 SSL")?;
    // 部分版本需要再写入一次站点 SSL 配置才生效；失败不影响已保存的证书
    let _ = baota_post(
        &base,
        "/site?action=SetSSLConf",
        vec![("siteName".into(), site)],
        &api_sk,
    )
    .await;
    Ok(())
}

async fn baota_test(node: &DeployNode) -> Result<String, String> {
    let base = http_base_url(&cfg(node, "url"));
    let api_sk = baota_api_sk(node);
    if base.is_empty() || api_sk.is_empty() {
        return Err("宝塔节点缺少面板地址或 API 密钥".into());
    }
    let v = baota_post(&base, "/site?action=GetSiteList", vec![], &api_sk).await?;
    baota_check(&v, "API 鉴权")?;
    Ok("宝塔 API 鉴权正常".to_string())
}

// --- 云厂商：阿里云 CAS / 腾讯云 SSL / 七牛 Fusion ---

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 阿里云 RPC 签名（POST + form）。参数需按 key 字典序排列。
fn aliyun_rpc_sign(params: &BTreeMap<String, String>, secret: &str) -> String {
    let canonical = params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let string_to_sign = format!("POST&{}&{}", percent_encode("/"), percent_encode(&canonical));
    let key = format!("{}&", secret);
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(string_to_sign.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

fn aliyun_auth(node: &DeployNode) -> Result<(String, String, String), String> {
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if ak.is_empty() || sk.is_empty() {
        return Err("阿里云节点缺少 AccessKey ID / Secret".into());
    }
    Ok((ak, sk, cfg(node, "region")))
}

/// 阿里云 RPC 通用调用：自动补公共参数、签名，并以 form 方式 POST。
async fn aliyun_rpc(
    host: &str,
    version: &str,
    action: &str,
    params: BTreeMap<String, String>,
    ak: &str,
    sk: &str,
) -> Result<serde_json::Value, String> {
    let mut p = params;
    p.insert("Action".into(), action.into());
    p.insert("Version".into(), version.into());
    p.insert("Format".into(), "JSON".into());
    p.insert("AccessKeyId".into(), ak.into());
    p.insert("SignatureMethod".into(), "HMAC-SHA1".into());
    p.insert("SignatureVersion".into(), "1.0".into());
    p.insert(
        "SignatureNonce".into(),
        format!("{}-{}", Utc::now().timestamp_millis(), std::process::id()),
    );
    p.insert(
        "Timestamp".into(),
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    );
    let signature = aliyun_rpc_sign(&p, sk);
    p.insert("Signature".into(), signature);
    let body = p
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://{}/", host))
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("调用阿里云 {} 失败: {}", action, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("阿里云返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(code) = v.get("Code").and_then(|x| x.as_str()) {
        return Err(format!(
            "阿里云 {} 失败: {} - {}",
            action,
            code,
            v.get("Message").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!("阿里云 {} 返回 {}: {}", action, status, text.trim()));
    }
    Ok(v)
}

async fn aliyun_upload_cert(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<String, String> {
    let (ak, sk, _) = aliyun_auth(node)?;
    let name = cert_name_for(cert, target, &cert.id);
    let mut p = BTreeMap::new();
    p.insert("Name".into(), name.chars().take(63).collect());
    p.insert("Cert".into(), pems.0.clone());
    p.insert("Key".into(), pems.1.clone());
    let v = aliyun_rpc("cas.aliyuncs.com", "2020-04-07", "UploadUserCertificate", p, &ak, &sk).await?;
    v.get("CertId")
        .map(|x| x.to_string().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "阿里云上传证书成功但未返回 CertId".to_string())
}

async fn deploy_aliyun_ssl(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    aliyun_upload_cert(node, cert, pems, target).await.map(|_| ())
}

async fn deploy_aliyun_cdn_like(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
    host: &str,
    version: &str,
    action: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("CDN 部署需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk, _) = aliyun_auth(node)?;
    let name = cert_name_for(cert, target, &cert.id);
    let mut p = BTreeMap::new();
    p.insert("DomainName".into(), target.trim().to_string());
    p.insert("CertName".into(), name.chars().take(63).collect());
    p.insert("CertType".into(), "upload".into());
    p.insert("SSLPub".into(), pems.0.clone());
    p.insert("SSLPri".into(), pems.1.clone());
    p.insert("SSLProtocol".into(), "on".into());
    aliyun_rpc(host, version, action, p, &ak, &sk).await?;
    Ok(())
}

async fn deploy_aliyun_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    deploy_aliyun_cdn_like(
        node,
        cert,
        pems,
        target,
        "cdn.aliyuncs.com",
        "2018-05-10",
        "SetCdnDomainSSLCertificate",
    )
    .await
}

async fn deploy_aliyun_dcdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    deploy_aliyun_cdn_like(
        node,
        cert,
        pems,
        target,
        "dcdn.aliyuncs.com",
        "2018-01-15",
        "SetDcdnDomainSSLCertificate",
    )
    .await
}

fn listener_port(node: &DeployNode) -> Result<u16, String> {
    let s = {
        let p = cfg(node, "listener_port");
        if p.is_empty() { cfg(node, "port") } else { p }
    };
    s.parse::<u16>()
        .map_err(|_| "节点缺少有效的监听端口（listener_port / port）".to_string())
}

fn load_balancer_id(node: &DeployNode) -> Result<String, String> {
    let id = {
        let v = cfg(node, "load_balancer_id");
        if v.is_empty() { cfg(node, "instance_id") } else { v }
    };
    if id.is_empty() {
        Err("节点缺少负载均衡实例 ID".into())
    } else {
        Ok(id)
    }
}

async fn aliyun_listener_id(
    node: &DeployNode,
    host: &str,
    version: &str,
    lb_id: &str,
    port: u16,
    protocol: Option<&str>,
) -> Result<String, String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    let mut p = BTreeMap::new();
    if !region.is_empty() {
        p.insert("RegionId".into(), region);
    }
    p.insert("LoadBalancerIds.1".into(), lb_id.to_string());
    if let Some(proto) = protocol {
        p.insert("ListenerProtocol".into(), proto.to_string());
    }
    let v = aliyun_rpc(host, version, "ListListeners", p, &ak, &sk).await?;
    let listeners = v
        .get("Listeners")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for l in &listeners {
        if l.get("ListenerPort").and_then(|x| x.as_i64()) == Some(port as i64) {
            if let Some(id) = l.get("ListenerId").and_then(|x| x.as_str()) {
                return Ok(id.to_string());
            }
        }
    }
    let ports: Vec<String> = listeners
        .iter()
        .filter_map(|l| l.get("ListenerPort").map(|p| p.to_string()))
        .collect();
    Err(format!(
        "未找到端口 {} 的监听；当前监听端口: {}",
        port,
        if ports.is_empty() { "无".to_string() } else { ports.join(", ") }
    ))
}

async fn deploy_aliyun_slb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    if region.is_empty() {
        return Err("SLB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let cert_id = aliyun_upload_cert(node, cert, pems, target).await?;
    let mut p = BTreeMap::new();
    p.insert("RegionId".into(), region);
    p.insert("LoadBalancerId".into(), lb);
    p.insert("ListenerPort".into(), port.to_string());
    p.insert("ServerCertificateId".into(), cert_id);
    aliyun_rpc(
        "slb.aliyuncs.com",
        "2014-05-15",
        "SetLoadBalancerHTTPSListenerAttribute",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

async fn deploy_aliyun_alb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    if region.is_empty() {
        return Err("ALB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let listener = aliyun_listener_id(node, "alb.aliyuncs.com", "2020-06-16", &lb, port, None).await?;
    let cert_id = aliyun_upload_cert(node, cert, pems, target).await?;
    let mut p = BTreeMap::new();
    p.insert("RegionId".into(), region);
    p.insert("ListenerId".into(), listener);
    p.insert("Certificates.1.CertificateId".into(), cert_id);
    p.insert("Certificates.1.IsDefault".into(), "true".into());
    aliyun_rpc(
        "alb.aliyuncs.com",
        "2020-06-16",
        "UpdateListenerAttribute",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

async fn deploy_aliyun_nlb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    if region.is_empty() {
        return Err("NLB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let listener = aliyun_listener_id(
        node,
        "nlb.aliyuncs.com",
        "2022-04-30",
        &lb,
        port,
        Some("TCPSSL"),
    )
    .await?;
    let cert_id = aliyun_upload_cert(node, cert, pems, target).await?;
    let mut p = BTreeMap::new();
    p.insert("RegionId".into(), region);
    p.insert("ListenerId".into(), listener);
    p.insert("CertificateIds.1".into(), cert_id);
    aliyun_rpc(
        "nlb.aliyuncs.com",
        "2022-04-30",
        "UpdateListenerAttribute",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

async fn deploy_aliyun_api_gateway(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let group_id = cfg(node, "group_id");
    if group_id.is_empty() {
        return Err("API 网关节点缺少 group_id".into());
    }
    if target.trim().is_empty() {
        return Err("API 网关部署需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk, _) = aliyun_auth(node)?;
    let mut p = BTreeMap::new();
    p.insert("GroupId".into(), group_id);
    p.insert("DomainName".into(), target.trim().to_string());
    p.insert(
        "CertificateName".into(),
        cert_name_for(cert, target, &cert.id).chars().take(63).collect(),
    );
    p.insert("CertificateBody".into(), pems.0.clone());
    p.insert("PrivateKey".into(), pems.1.clone());
    aliyun_rpc(
        "apigateway.aliyuncs.com",
        "2016-07-14",
        "SetDomainCertificate",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

fn hmac_sha256(key: &[u8], msg: &str) -> Vec<u8> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC key");
    mac.update(msg.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn tencent_auth(node: &DeployNode) -> Result<(String, String, String), String> {
    let id = cfg(node, "secret_id");
    let key = cfg(node, "secret_key");
    if id.is_empty() || key.is_empty() {
        return Err("腾讯云节点缺少 SecretId / SecretKey".into());
    }
    Ok((id, key, cfg(node, "region")))
}

/// 腾讯云 TC3-HMAC-SHA256 通用调用。
async fn tencent_rpc(
    host: &str,
    service: &str,
    version: &str,
    action: &str,
    region: &str,
    body: serde_json::Value,
    secret_id: &str,
    secret_key: &str,
) -> Result<serde_json::Value, String> {
    let content_type = "application/json; charset=utf-8";
    let now = Utc::now();
    let timestamp = now.timestamp();
    let date = now.format("%Y-%m-%d").to_string();
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let mut headers: Vec<(String, String)> = vec![
        ("content-type".into(), content_type.into()),
        ("host".into(), host.into()),
        ("x-tc-action".into(), action.to_lowercase()),
        ("x-tc-timestamp".into(), timestamp.to_string()),
        ("x-tc-version".into(), version.into()),
    ];
    if !region.is_empty() {
        headers.push(("x-tc-region".into(), region.to_string()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers = headers
        .iter()
        .map(|(k, v)| format!("{}:{}\n", k, v))
        .collect::<String>();
    let signed_headers = headers
        .iter()
        .map(|(k, _)| k.clone())
        .collect::<Vec<_>>()
        .join(";");
    let payload_hash = hex(&Sha256::digest(body_text.as_bytes()));
    let canonical_request = format!(
        "POST\n/\n\n{}\n{}\n{}",
        canonical_headers, signed_headers, payload_hash
    );
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{}\n{}/{}/tc3_request\n{}",
        timestamp,
        date,
        service,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let secret_date = hmac_sha256(format!("TC3{}", secret_key).as_bytes(), &date);
    let secret_service = hmac_sha256(&secret_date, service);
    let secret_signing = hmac_sha256(&secret_service, "tc3_request");
    let signature = hex(&hmac_sha256(&secret_signing, &string_to_sign));
    let auth = format!(
        "TC3-HMAC-SHA256 Credential={}/{}/{}/tc3_request, SignedHeaders={}, Signature={}",
        secret_id, date, service, signed_headers, signature
    );

    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://{}", host))
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .header("X-TC-Action", action)
        .header("X-TC-Timestamp", timestamp.to_string())
        .header("X-TC-Version", version)
        .header(reqwest::header::AUTHORIZATION, auth);
    if !region.is_empty() {
        req = req.header("X-TC-Region", region);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用腾讯云 {} 失败: {}", action, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("腾讯云返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(err) = v.pointer("/Response/Error") {
        return Err(format!(
            "腾讯云 {} 失败: {} - {}",
            action,
            err.get("Code").and_then(|x| x.as_str()).unwrap_or(""),
            err.get("Message").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!("腾讯云 {} 返回 {}: {}", action, status, text.trim()));
    }
    Ok(v.get("Response").cloned().unwrap_or(v))
}

async fn tencent_upload_cert(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<String, String> {
    let (id, key, _) = tencent_auth(node)?;
    let body = serde_json::json!({
        "CertificatePublicKey": pems.0,
        "CertificatePrivateKey": pems.1,
        "Alias": cert_name_for(cert, target, &cert.id),
        "CertificateType": "SVR",
    });
    let v = tencent_rpc(
        "ssl.tencentcloudapi.com",
        "ssl",
        "2019-12-05",
        "UploadCertificate",
        "",
        body,
        &id,
        &key,
    )
    .await?;
    v.get("CertificateId")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "腾讯云上传证书成功但未返回 CertificateId".to_string())
}

async fn deploy_tencent_ssl(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    tencent_upload_cert(node, cert, pems, target).await.map(|_| ())
}

async fn deploy_tencent_clb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (id, key, region) = tencent_auth(node)?;
    if region.is_empty() {
        return Err("腾讯云 CLB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let v = tencent_rpc(
        "clb.tencentcloudapi.com",
        "clb",
        "2017-03-12",
        "DescribeListeners",
        &region,
        serde_json::json!({ "LoadBalancerId": lb }),
        &id,
        &key,
    )
    .await?;
    let listeners = v
        .get("Listeners")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let listener = listeners
        .iter()
        .find(|l| {
            l.get("Port").and_then(|x| x.as_i64()) == Some(port as i64)
                || l.get("ListenerPort").and_then(|x| x.as_i64()) == Some(port as i64)
        })
        .and_then(|l| l.get("ListenerId").and_then(|x| x.as_str()))
        .ok_or_else(|| format!("未找到端口 {} 的 CLB 监听", port))?
        .to_string();
    let cert_id = tencent_upload_cert(node, cert, pems, target).await?;
    tencent_rpc(
        "clb.tencentcloudapi.com",
        "clb",
        "2017-03-12",
        "ModifyListener",
        &region,
        serde_json::json!({
            "LoadBalancerId": lb,
            "ListenerId": listener,
            "Certificate": { "SSLMode": "UNIDIRECTIONAL", "CertId": cert_id }
        }),
        &id,
        &key,
    )
    .await?;
    Ok(())
}

async fn deploy_tencent_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("腾讯云 CDN 部署需要在域名列表关联时填写目标域名".into());
    }
    let (id, key, region) = tencent_auth(node)?;
    let cert_id = tencent_upload_cert(node, cert, pems, target).await?;
    // 先打开 HTTPS，再替换证书
    let _ = tencent_rpc(
        "cdn.tencentcloudapi.com",
        "cdn",
        "2018-03-09",
        "ModifyDomainConfig",
        &region,
        serde_json::json!({
            "Domain": target.trim(),
            "Route": "Https.Switch",
            "Value": "{\"update\":\"on\"}"
        }),
        &id,
        &key,
    )
    .await;
    tencent_rpc(
        "cdn.tencentcloudapi.com",
        "cdn",
        "2018-03-09",
        "ModifyDomainConfig",
        &region,
        serde_json::json!({
            "Domain": target.trim(),
            "Route": "Https.CertInfo.CertId",
            "Value": format!("{{\"update\":\"{}\"}}", cert_id)
        }),
        &id,
        &key,
    )
    .await?;
    Ok(())
}

async fn deploy_qiniu_ssl(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let ak = cfg(node, "access_key");
    let sk = cfg(node, "secret_key");
    if ak.is_empty() || sk.is_empty() {
        return Err("七牛 SSL 节点缺少 AccessKey / SecretKey".into());
    }
    let name = {
        let configured = cfg(node, "cert_name");
        if configured.is_empty() {
            cert_name_for(cert, target, &cert.id)
        } else {
            format!("{}-{}", configured, Utc::now().format("%Y%m%d%H%M%S"))
        }
    };
    let body = serde_json::json!({
        "name": name,
        "pri": pems.1,
        "ca": pems.0,
    });
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let key = format!("{}&", sk);
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(format!("/sslcert\n{}", body_text).as_bytes());
    let sign = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post("https://fusion.qiniuapi.com/sslcert")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::AUTHORIZATION,
            format!("QBox {}:{}", ak, sign),
        )
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用七牛证书服务失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("七牛返回非 JSON ({}): {}", status, text.trim()))?;
    if v.get("certID").and_then(|x| x.as_str()).is_some() {
        return Ok(());
    }
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        return Err(format!(
            "七牛上传证书失败: {} - {}",
            code,
            v.get("error").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!("七牛返回 {}: {}", status, text.trim()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 第三方面板：1Panel / 群晖 DSM / 又拍云
// ---------------------------------------------------------------------------

fn one_panel_base(raw: &str) -> String {
    let mut u = http_base_url(raw);
    if !u.is_empty() && !u.ends_with("/api/v1") {
        while u.ends_with('/') {
            u.pop();
        }
        u.push_str("/api/v1");
    }
    u
}

async fn one_panel_post(
    base: &str,
    path: &str,
    api_key: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    // 1Panel：Timestamp 为 Unix 秒，Token = md5("1panel" + api_key + timestamp)
    let ts = Utc::now().timestamp().to_string();
    let token = format!("{:x}", md5::compute(format!("1panel{}{}", api_key, ts).as_bytes()));
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}{}", base, path))
        .header("1Panel-Token", token)
        .header("1Panel-Timestamp", ts)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用 1Panel 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("1Panel 返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        if code != 200 {
            return Err(format!(
                "1Panel {} 失败: {} - {}",
                path,
                code,
                v.get("message").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    if !status.is_success() {
        return Err(format!("1Panel 返回 {}: {}", status, text.trim()));
    }
    Ok(v)
}

fn json_id_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

async fn deploy_one_panel(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let base = one_panel_base(&cfg(node, "url"));
    let api_key = cfg(node, "api_key");
    if base.is_empty() || api_key.is_empty() {
        return Err("1Panel 节点缺少面板地址或 API 密钥".into());
    }
    if cfg(node, "scope") == "panel" {
        one_panel_post(
            &base,
            "/settings/ssl/update",
            &api_key,
            serde_json::json!({
                "ssl": "enable",
                "sslType": "import-paste",
                "cert": pems.0,
                "key": pems.1,
                "autoRestart": "true"
            }),
        )
        .await?;
        return Ok(());
    }

    let domain = if target.trim().is_empty() {
        cert.domains.first().cloned().unwrap_or_default()
    } else {
        target.trim().to_string()
    };
    if domain.is_empty() {
        return Err("1Panel 网站 SSL 需要在域名列表关联时填写目标域名".into());
    }
    let description = cert_name_for(cert, &domain, &cert.id);

    // 1) 上传证书（paste）；2) 查回刚上传的 SSL 记录 ID
    one_panel_post(
        &base,
        "/websites/ssl/upload",
        &api_key,
        serde_json::json!({
            "type": "paste",
            "description": description,
            "certificate": pems.0,
            "privateKey": pems.1
        }),
    )
    .await?;
    let ssl_list = one_panel_post(
        &base,
        "/websites/ssl/search",
        &api_key,
        serde_json::json!({ "page": 1, "pageSize": 100, "orderBy": "created_at", "order": "desc" }),
    )
    .await?;
    let ssl_items = ssl_list
        .pointer("/data/items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    // 优先按 description 匹配；新版可能改写描述，退回最新一条（列表已按创建时间倒序）
    let ssl_id = ssl_items
        .iter()
        .find(|i| i.get("description").and_then(|d| d.as_str()) == Some(description.as_str()))
        .or_else(|| ssl_items.first())
        .and_then(|i| i.get("id"))
        .map(json_id_str)
        .ok_or_else(|| "1Panel 上传证书后未找到 SSL 记录".to_string())?;

    // 3) 按域名找到网站 ID
    let sites = one_panel_post(
        &base,
        "/websites/search",
        &api_key,
        serde_json::json!({ "page": 1, "pageSize": 10, "name": domain }),
    )
    .await?;
    let site_id = sites
        .pointer("/data/items")
        .and_then(|x| x.as_array())
        .and_then(|items| {
            items
                .iter()
                .find(|i| i.get("name").and_then(|n| n.as_str()) == Some(domain.as_str()))
                .or_else(|| items.first())
        })
        .and_then(|i| i.get("id"))
        .map(json_id_str)
        .ok_or_else(|| format!("1Panel 中未找到网站：{}", domain))?;

    // 4) 绑定证书并开启 HTTPS
    let qid: serde_json::Value = site_id
        .parse::<i64>()
        .map(|n| serde_json::json!(n))
        .unwrap_or_else(|_| serde_json::json!(site_id));
    let qssl: serde_json::Value = ssl_id
        .parse::<i64>()
        .map(|n| serde_json::json!(n))
        .unwrap_or_else(|_| serde_json::json!(ssl_id));
    one_panel_post(
        &base,
        &format!("/websites/{}/https", site_id),
        &api_key,
        serde_json::json!({
            "websiteId": qid,
            "enable": true,
            "type": "existed",
            "websiteSSLId": qssl,
            "httpConfig": "HTTPToHTTPS",
            "SSLProtocol": [],
            "algorithm": ""
        }),
    )
    .await?;
    Ok(())
}

// --- 群晖 DSM ---

fn synology_base(node: &DeployNode) -> String {
    http_base_url(&cfg(node, "url"))
}

async fn synology_login(node: &DeployNode) -> Result<(String, String, String), String> {
    let base = synology_base(node);
    let account = cfg(node, "username");
    let passwd = cfg(node, "password");
    if base.is_empty() || account.is_empty() || passwd.is_empty() {
        return Err("群晖节点缺少地址 / 账号 / 密码".into());
    }
    let mut query = vec![
        ("api", "SYNO.API.Auth".to_string()),
        ("version", "6".to_string()),
        ("method", "login".to_string()),
        ("format", "sid".to_string()),
        ("account", account),
        ("passwd", passwd),
        ("session", "Certificate".to_string()),
        ("enable_syno_token", "yes".to_string()),
    ];
    let otp = cfg(node, "otp_code");
    if !otp.is_empty() {
        query.push(("otp_code", otp));
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .get(format!("{}/webapi/auth.cgi", base))
        .query(&query)
        .send()
        .await
        .map_err(|e| format!("连接群晖失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("群晖返回非 JSON: {}", text.chars().take(200).collect::<String>()))?;
    if v.get("success").and_then(|x| x.as_bool()) != Some(true) {
        let code = v.pointer("/error/code").and_then(|x| x.as_i64()).unwrap_or(-1);
        return Err(format!("群晖登录失败（错误码 {}）", code));
    }
    let sid = v
        .pointer("/data/sid")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    if sid.is_empty() {
        return Err("群晖登录成功但未返回 sid".to_string());
    }
    let token = v
        .pointer("/data/synotoken")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((base, sid, token))
}

async fn synology_cert_id(
    base: &str,
    sid: &str,
    node: &DeployNode,
) -> Result<String, String> {
    let configured = cfg(node, "cert_id");
    if !configured.is_empty() {
        return Ok(configured);
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}/webapi/entry.cgi", base))
        .form(&[
            ("api", "SYNO.Core.Certificate.CRT"),
            ("method", "list"),
            ("version", "1"),
            ("_sid", sid),
        ])
        .send()
        .await
        .map_err(|e| format!("获取群晖证书列表失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let certs = v
        .pointer("/data/certificates")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    // 优先默认证书（系统正在使用的那张），否则第一张
    certs
        .iter()
        .find(|c| c.get("is_default").and_then(|x| x.as_bool()) == Some(true))
        .or_else(|| certs.first())
        .and_then(|c| c.get("id"))
        .map(json_id_str)
        .ok_or_else(|| "群晖中未找到可用的证书，请在节点配置里填写证书 ID".to_string())
}

async fn deploy_synology(
    node: &DeployNode,
    _cert: &Certificate,
    pems: &(String, String, String),
    _target: &str,
) -> Result<(), String> {
    let (base, sid, token) = synology_login(node).await?;
    let cert_id = synology_cert_id(&base, &sid, node).await?;

    // cert 只放叶子证书，中间链放 inter_cert
    let leaf = pem_first_block(&pems.0);
    let chain = pems.0.replacen(leaf.as_str(), "", 1);
    let mut form = reqwest::multipart::Form::new()
        .text("id", cert_id)
        .text("desc", cert_name_for(_cert, "", &_cert.id))
        .text("key", pems.1.clone())
        .text("cert", leaf)
        .text("inter_cert", chain);
    if cfg(node, "as_default") == "true" {
        form = form.text("as_default", "true");
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}/webapi/entry.cgi", base))
        .query(&[
            ("api", "SYNO.Core.Certificate"),
            ("method", "import"),
            ("version", "1"),
            ("_sid", sid.as_str()),
            ("SynoToken", token.as_str()),
        ])
        .header("X-SYNO-TOKEN", token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("导入群晖证书失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("群晖返回非 JSON: {}", text.chars().take(200).collect::<String>()))?;
    if v.get("success").and_then(|x| x.as_bool()) != Some(true) {
        let code = v.pointer("/error/code").and_then(|x| x.as_i64()).unwrap_or(-1);
        return Err(format!("群晖导入证书失败（错误码 {}）", code));
    }
    Ok(())
}

// --- 又拍云（控制台会话） ---

async fn upyun_login(node: &DeployNode) -> Result<String, String> {
    let username = cfg(node, "username");
    let password = cfg(node, "password");
    if username.is_empty() || password.is_empty() {
        return Err("又拍云节点缺少账号 / 密码".into());
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post("https://console.upyun.com/accounts/signin/")
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|e| format!("登录又拍云失败: {}", e))?;
    let status = resp.status();
    let cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("登录又拍云失败 ({}): {}", status, text.trim()));
    }
    if cookies.is_empty() {
        return Err("又拍云登录成功但未返回会话 Cookie（可能需要二次验证）".to_string());
    }
    Ok(cookies.join("; "))
}

async fn upyun_upload_cert(pems: &(String, String, String), cookie: &str) -> Result<String, String> {
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post("https://console.upyun.com/api/https/certificate/")
        .header(reqwest::header::COOKIE, cookie)
        .json(&serde_json::json!({
            "certificate": pems.0,
            "private_key": pems.1
        }))
        .send()
        .await
        .map_err(|e| format!("上传又拍云证书失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("又拍云返回非 JSON: {}", text.chars().take(200).collect::<String>()))?;
    v.pointer("/data/result/certificate_id")
        .map(json_id_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("又拍云上传证书失败: {}", text.chars().take(300).collect::<String>()))
}

async fn deploy_upyun(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let domain = if target.trim().is_empty() {
        cert.domains.first().cloned().unwrap_or_default()
    } else {
        target.trim().to_string()
    };
    if domain.is_empty() {
        return Err("又拍云部署需要在域名列表关联时填写目标域名".into());
    }
    let cookie = upyun_login(node).await?;
    let cert_id = upyun_upload_cert(pems, &cookie).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    // 未开 HTTPS 的域名绑定证书
    let bind = client
        .post("https://console.upyun.com/api/https/certificate/manager")
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "certificate_id": cert_id,
            "domain": domain,
            "https": true,
            "force_https": true
        }))
        .send()
        .await
        .map_err(|e| format!("绑定又拍云证书失败: {}", e))?;
    let status = bind.status();
    let text = bind.text().await.unwrap_or_default();
    if status.is_success() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if v.get("code").and_then(|x| x.as_i64()).unwrap_or(0) == 0 {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }
    // 已启用 HTTPS 的域名使用换证接口重试
    let migrate = client
        .post("https://console.upyun.com/api/https/migrate/domain")
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "crt_id": cert_id, "domain_name": domain }))
        .send()
        .await
        .map_err(|e| format!("又拍云换证失败: {}", e))?;
    let mstatus = migrate.status();
    let mtext = migrate.text().await.unwrap_or_default();
    if !mstatus.is_success() {
        return Err(format!(
            "又拍云绑定证书失败 ({}): {}",
            mstatus,
            mtext.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 更多云厂商：百度智能云 / 华为云 / 火山引擎 / 多吉云 / 阿里云 OSS / 函数计算
// ---------------------------------------------------------------------------

fn hmac_sha1_hex(key: &[u8], msg: &str) -> String {
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key).unwrap();
    mac.update(msg.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

fn sorted_canonical_headers(headers: &[(String, String)]) -> (String, String) {
    let mut hs = headers.to_vec();
    hs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    let canonical = hs
        .iter()
        .map(|(k, v)| format!("{}:{}\n", k.to_lowercase(), v.trim()))
        .collect::<String>();
    let signed = hs
        .iter()
        .map(|(k, _)| k.to_lowercase())
        .collect::<Vec<_>>()
        .join(";");
    (canonical, signed)
}

/// 百度智能云 BCE 签名（bce-auth-v1 / HMAC-SHA256；V1 规范不签 body）。
fn bce_headers(
    method: &str,
    uri: &str,
    query: &str,
    mut headers: Vec<(String, String)>,
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    headers.push(("x-bce-date".into(), ts.clone()));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}",
        method, uri, query, canonical_headers
    );
    let auth_prefix = format!("bce-auth-v1/{}/{}/1800", ak, ts);
    let signing_key = hex(&hmac_sha256(sk.as_bytes(), &auth_prefix));
    let signature = hex(&hmac_sha256(
        signing_key.as_bytes(),
        &format!(
            "{}\n{}",
            auth_prefix,
            hex(&Sha256::digest(canonical_request.as_bytes()))
        ),
    ));
    headers.push((
        "Authorization".into(),
        format!("{}/{}/{}", auth_prefix, signed_headers, signature),
    ));
    headers
}

async fn deploy_baidu_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("百度智能云 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk) = ak_sk(node, "百度智能云")?;
    let domain = target.trim();
    let uri = format!("/v2/{}/certificates", domain);
    let leaf = pem_first_block(&pems.0);
    let chain = pems.0.replacen(leaf.as_str(), "", 1);
    let body = serde_json::json!({
        "httpsEnable": "ON",
        "certificate": {
            "certName": cert_name_for(cert, domain, &cert.id),
            "certServerData": leaf,
            "certPrivateData": pems.1,
            "certLinkData": chain
        }
    });
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let headers = bce_headers(
        "PUT",
        &uri,
        "",
        vec![
            ("host".into(), "cdn.baidubce.com".into()),
            ("content-type".into(), "application/json".into()),
        ],
        &ak,
        &sk,
    );
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .put(format!("https://cdn.baidubce.com{}", uri));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用百度智能云 CDN 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "百度智能云 CDN 返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// 华为云 AK/SK 签名（SDK-HMAC-SHA256）。
fn huawei_headers(
    method: &str,
    uri: &str,
    query: &str,
    mut headers: Vec<(String, String)>,
    body: &[u8],
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    headers.push(("X-Sdk-Date".into(), date.clone()));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        uri,
        query,
        canonical_headers,
        signed_headers,
        hex(&Sha256::digest(body))
    );
    let string_to_sign = format!(
        "SDK-HMAC-SHA256\n{}\n{}",
        date,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let signature = hex(&hmac_sha256(sk.as_bytes(), &string_to_sign));
    headers.push((
        "Authorization".into(),
        format!(
            "SDK-HMAC-SHA256 Access={}, SignedHeaders={}, Signature={}",
            ak, signed_headers, signature
        ),
    ));
    headers
}

async fn deploy_huawei_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("华为云 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk) = ak_sk(node, "华为云")?;
    let domain = target.trim();
    let uri = "/v1.0/cdn/domains/config-https-info";
    let query = "enterprise_project_id=all";
    let body = serde_json::json!({
        "https": {
            "domain_name": domain,
            "https_switch": 1,
            "cert_name": cert_name_for(cert, domain, &cert.id),
            "certificate": pems.0,
            "private_key": pems.1,
            "certificate_type": 0
        }
    });
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let headers = huawei_headers(
        "PUT",
        uri,
        query,
        vec![
            ("content-type".into(), "application/json".into()),
            ("host".into(), "cdn.myhuaweicloud.com".into()),
        ],
        body_text.as_bytes(),
        &ak,
        &sk,
    );
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .put(format!("https://cdn.myhuaweicloud.com{}?{}", uri, query));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用华为云 CDN 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "华为云 CDN 返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// 火山引擎 V4 签名（service=cdn, region=cn-north-1）。
fn volc_headers(
    method: &str,
    uri: &str,
    query: &str,
    mut headers: Vec<(String, String)>,
    body: &[u8],
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let short = &date[..8];
    headers.push(("X-Date".into(), date.clone()));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        uri,
        query,
        canonical_headers,
        signed_headers,
        hex(&Sha256::digest(body))
    );
    let scope = format!("{}/cn-north-1/cdn/request", short);
    let string_to_sign = format!(
        "HMAC-SHA256\n{}\n{}\n{}",
        date,
        scope,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let k_date = hmac_sha256(sk.as_bytes(), short);
    let k_region = hmac_sha256(&k_date, "cn-north-1");
    let k_service = hmac_sha256(&k_region, "cdn");
    let k_signing = hmac_sha256(&k_service, "request");
    let signature = hex(&hmac_sha256(&k_signing, &string_to_sign));
    headers.push((
        "Authorization".into(),
        format!(
            "HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            ak, scope, signed_headers, signature
        ),
    ));
    headers
}

async fn volc_call(
    node: &DeployNode,
    action: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (ak, sk) = ak_sk(node, "火山引擎")?;
    let query = format!("Action={}&Version=2021-03-01", action);
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let headers = volc_headers(
        "POST",
        "/",
        &query,
        vec![
            ("content-type".into(), "application/json".into()),
            ("host".into(), "cdn.volcengineapi.com".into()),
        ],
        body_text.as_bytes(),
        &ak,
        &sk,
    );
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://cdn.volcengineapi.com/?{}", query));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用火山引擎 {} 失败: {}", action, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if let Some(err) = v.pointer("/ResponseMetadata/Error") {
        return Err(format!(
            "火山引擎 {} 失败: {} - {}",
            action,
            err.get("Code").and_then(|x| x.as_str()).unwrap_or(""),
            err.get("Message").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!(
            "火山引擎 {} 返回 {}: {}",
            action,
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(v)
}

async fn deploy_volcengine_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("火山引擎 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let domain = target.trim();
    let uploaded = volc_call(
        node,
        "AddCertificate",
        serde_json::json!({
            "Certificate": pems.0,
            "PrivateKey": pems.1,
            "Source": "volc_cert_center",
            "CertType": "server_cert",
            "Desc": cert_name_for(cert, domain, &cert.id)
        }),
    )
    .await?;
    let cert_id = uploaded
        .pointer("/Result/CertId")
        .map(json_id_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "火山引擎上传证书成功但未返回 CertId".to_string())?;
    volc_call(
        node,
        "UpdateCdnConfig",
        serde_json::json!({
            "Domain": domain,
            "HTTPS": { "Switch": true, "CertInfo": { "CertId": cert_id } }
        }),
    )
    .await?;
    Ok(())
}

/// 多吉云：Authorization: TOKEN {AK}:{hmac_sha1_hex(SK, uri + "\n" + body)}
async fn doge_call(path: &str, body: serde_json::Value, ak: &str, sk: &str) -> Result<serde_json::Value, String> {
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let sign = hmac_sha1_hex(sk.as_bytes(), &format!("{}\n{}", path, body_text));
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://api.dogecloud.com{}", path))
        .header(reqwest::header::AUTHORIZATION, format!("TOKEN {}:{}", ak, sign))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用多吉云失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("多吉云返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        if code != 200 {
            return Err(format!(
                "多吉云 {} 失败: {} - {}",
                path,
                code,
                v.get("msg").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    if !status.is_success() {
        return Err(format!("多吉云返回 {}: {}", status, text.trim()));
    }
    Ok(v)
}

async fn deploy_doge_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("多吉云 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let ak = cfg(node, "access_key");
    let sk = cfg(node, "secret_key");
    if ak.is_empty() || sk.is_empty() {
        return Err("多吉云节点缺少 AccessKey / SecretKey".into());
    }
    let domain = target.trim();
    let uploaded = doge_call(
        "/cdn/cert/upload.json",
        serde_json::json!({
            "note": cert_name_for(cert, domain, &cert.id),
            "cert": pems.0,
            "private": pems.1
        }),
        &ak,
        &sk,
    )
    .await?;
    let cert_id = uploaded
        .pointer("/data/id")
        .map(json_id_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "多吉云上传证书成功但未返回证书 ID".to_string())?;
    doge_call(
        "/cdn/cert/bind.json",
        serde_json::json!({ "id": cert_id, "domain": domain }),
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

async fn deploy_aliyun_oss(
    node: &DeployNode,
    _cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("阿里云 OSS 需要在域名列表关联时填写目标域名".into());
    }
    let bucket = cfg(node, "bucket");
    let endpoint = cfg(node, "endpoint");
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if bucket.is_empty() || endpoint.is_empty() || ak.is_empty() || sk.is_empty() {
        return Err("阿里云 OSS 节点缺少 Bucket / Endpoint / AccessKey".into());
    }
    let host = http_base_url(&endpoint)
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .to_string();
    let date = Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let content_type = "application/xml";
    let canonical_resource = format!("/{}/?cname&comp=add", bucket);
    let string_to_sign = format!("POST\n\n{}\n{}\n{}", content_type, date, canonical_resource);
    let mut mac = <HmacSha1 as Mac>::new_from_slice(format!("{}&", sk).as_bytes()).unwrap();
    mac.update(string_to_sign.as_bytes());
    let signature = B64.encode(mac.finalize().into_bytes());
    let domain = target.trim();
    let body = format!(
        "<BucketCnameConfiguration><Cname><Domain>{}</Domain><CertificateConfiguration><Certificate>{}</Certificate><PrivateKey>{}</PrivateKey><Force>true</Force></CertificateConfiguration></Cname></BucketCnameConfiguration>",
        domain.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
        pems.0,
        pems.1
    );
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://{}.{}/?cname&comp=add", bucket, host))
        .header(reqwest::header::DATE, date)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("OSS {}:{}", ak, signature),
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("调用阿里云 OSS 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "阿里云 OSS 返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// 阿里云 ACS3-HMAC-SHA256 签名（函数计算自定义域名使用）。
fn acs3_headers(
    method: &str,
    path: &str,
    mut headers: Vec<(String, String)>,
    body: &[u8],
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let date = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut nonce = [0u8; 16];
    let _ = getrandom::getrandom(&mut nonce);
    headers.push(("x-acs-date".into(), date));
    headers.push(("x-acs-signature-nonce".into(), hex(&nonce)));
    headers.push((
        "x-acs-content-sha256".into(),
        hex(&Sha256::digest(body)),
    ));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!(
        "{}\n{}\n\n{}\n{}\n{}",
        method,
        path,
        canonical_headers,
        signed_headers,
        hex(&Sha256::digest(body))
    );
    let string_to_sign = format!(
        "ACS3-HMAC-SHA256\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let signature = hex(&hmac_sha256(sk.as_bytes(), &string_to_sign));
    headers.push((
        "Authorization".into(),
        format!(
            "ACS3-HMAC-SHA256 Credential={},SignedHeaders={},Signature={}",
            ak, signed_headers, signature
        ),
    ));
    headers
}

async fn deploy_aliyun_fc(
    node: &DeployNode,
    _cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("阿里云函数计算需要在域名列表关联时填写自定义域名".into());
    }
    let account = cfg(node, "account_id");
    let region = cfg(node, "region");
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if account.is_empty() || region.is_empty() || ak.is_empty() || sk.is_empty() {
        return Err("函数计算节点缺少 Account ID / Region / AccessKey".into());
    }
    let host = format!("{}.{}.fc.aliyuncs.com", account, region);
    let domain = target.trim();
    let path = format!("/2023-03-30/custom-domains/{}", domain);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    // 先读取现有配置，避免 PUT 覆盖路由/TLS 配置
    let headers = acs3_headers(
        "GET",
        &path,
        vec![("host".into(), host.clone())],
        b"",
        &ak,
        &sk,
    );
    let mut get_req = client.get(format!("https://{}{}", host, path));
    for (k, v) in headers {
        get_req = get_req.header(k, v);
    }
    let existing = get_req.send().await.ok();
    let mut cfg_json = match existing {
        Some(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .unwrap_or_else(|_| serde_json::json!({})),
        _ => serde_json::json!({}),
    };
    if !cfg_json.is_object() {
        cfg_json = serde_json::json!({});
    }
    let obj = cfg_json.as_object_mut().unwrap();
    obj.insert("protocol".into(), serde_json::json!("HTTP,HTTPS"));
    obj.insert(
        "certConfig".into(),
        serde_json::json!({
            "certName": cert_name_for(_cert, domain, &_cert.id),
            "certificate": pems.0,
            "privateKey": pems.1
        }),
    );
    let body_text = serde_json::to_string(&cfg_json).map_err(|e| e.to_string())?;
    let headers = acs3_headers(
        "PUT",
        &path,
        vec![
            ("content-type".into(), "application/json".into()),
            ("host".into(), host.clone()),
        ],
        body_text.as_bytes(),
        &ak,
        &sk,
    );
    let mut req = client.put(format!("https://{}{}", host, path));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用函数计算失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "函数计算返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 申请 / 续期 + 部署编排
// ---------------------------------------------------------------------------

/// 一次「申请 / 续期」的结果。
///
/// 为什么不直接回 `Certificate`：lego 在证书还没到续期时间时会**跳过**签发却
/// 仍以退出码 0 结束，只回 Certificate 的话前端无从分辨「真的签了」和「什么都没做」，
/// 表现就是「点了申请没反应」。`renewed == false` 即表示后者。
#[derive(serde::Serialize)]
pub struct IssueResult {
    pub cert: Certificate,
    /// 本次是否真的重新签发了证书（false = 未到续期时间，被 lego 跳过）
    pub renewed: bool,
    /// 证书剩余有效期（天，负数表示已过期）
    pub days_left: Option<i64>,
}

/// 申请（或续期）指定证书，并部署到其所有关联节点。
///
/// `force`：用户手动点「申请」时为 true —— 立刻重签，不看续期阈值；
/// 定时任务传 false，交给 lego 按 `renew_before_days` 自己判断该不该续。
pub async fn issue_and_deploy(cert_id: &str, force: bool) -> Result<IssueResult, String> {
    let mut certs = load_certs();
    let mut nodes = load_json::<DeployNode>("deploy_nodes.json");
    let creds = load_json::<Credential>("credentials.json");

    let idx = certs
        .iter()
        .position(|c| c.id == cert_id)
        .ok_or_else(|| "证书不存在".to_string())?;
    let cert = certs[idx].clone();

    // HTTP 验证不需要 DNS 授权凭据；DNS 验证必须有
    let cred = creds
        .into_iter()
        .find(|c| c.id == cert.credential_id)
        .map(decrypt_credential);

    let cert_dir = ensure_dir().join(&cert.id);
    let fingerprint_before = cert_store_fingerprint(&cert_dir);
    let renew = has_existing_cert(&cert_dir);
    let pems = match run_lego(&cert, cred.as_ref(), renew, force) {
        Ok(v) => v,
        Err(e) => {
            certs[idx].status = "error".to_string();
            certs[idx].last_error = Some(e.clone());
            save_json("certificates.json", &certs);
            return Err(e);
        }
    };
    // lego 跳过续期时退出码仍是 0，只能靠「文件有没有变」判断这次到底签没签
    let renewed = fingerprint_before != cert_store_fingerprint(&cert_dir);

    // 更新有效期（从证书 DER 解析 notBefore / notAfter）
    let (not_before, not_after) = extract_validity(&pems.0);
    let now = Utc::now().to_rfc3339();
    certs[idx].not_before = not_before;
    certs[idx].not_after = not_after;
    if renewed {
        certs[idx].last_issue_at = Some(now.clone());
        certs[idx].status = "issued".to_string();
        certs[idx].last_error = None;
    }
    // 被跳过时不动 last_issue_at / status：证书还是旧的那张，
    // 写成「刚刚签发」会让界面的签发时间失真（这正是一直以来的假象来源）。
    let days_left = certs[idx]
        .not_after
        .as_ref()
        .and_then(|na| DateTime::parse_from_rfc3339(na).ok())
        .map(|d| (d.with_timezone(&Utc) - Utc::now()).num_days());

    // 域名列表 → 部署方式关联；target 用于 CDN / 负载均衡等需要目标域名的节点
    let links: Vec<DeployLink> = if cert.deploy_links.is_empty() {
        cert.deploy_node_ids
            .iter()
            .map(|id| DeployLink {
                node_id: id.clone(),
                target: String::new(),
            })
            .collect()
    } else {
        cert.deploy_links.clone()
    };
    let default_target = cert.domains.first().cloned().unwrap_or_default();
    for link in &links {
        if let Some(node) = find_node(&nodes, &link.node_id) {
            let node = node.clone();
            let target = if link.target.trim().is_empty() {
                default_target.clone()
            } else {
                link.target.trim().to_string()
            };
            match deploy_to_node(&node, &cert, &pems, &target).await {
                Ok(()) => {
                    if let Some(n) = nodes.iter_mut().find(|n| n.id == link.node_id) {
                        n.last_deploy_at = Some(now.clone());
                        n.last_deploy_error = None;
                    }
                }
                Err(e) => {
                    if let Some(n) = nodes.iter_mut().find(|n| n.id == link.node_id) {
                        n.last_deploy_error = Some(e.clone());
                    }
                }
            }
        }
    }

    let updated = certs[idx].clone();
    save_json("certificates.json", &certs);
    save_json("deploy_nodes.json", &nodes);
    eprintln!(
        "[cert] {} {}（剩余 {} 天）",
        cert_id,
        if renewed {
            "已重新签发"
        } else {
            "未到续期时间，已跳过"
        },
        days_left
            .map(|d| d.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    Ok(IssueResult {
        cert: updated,
        renewed,
        days_left,
    })
}

/// 极简 DER TLV 读取器（仅用于解析 X.509 证书有效期）。
struct DerReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> DerReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        DerReader { buf, pos: 0 }
    }

    /// 读取一个 TLV，返回 (tag, value)。
    fn read_tlv(&mut self) -> Option<(u8, &'a [u8])> {
        if self.pos >= self.buf.len() {
            return None;
        }
        let tag = self.buf[self.pos];
        self.pos += 1;
        if self.pos >= self.buf.len() {
            return None;
        }
        let first = self.buf[self.pos];
        self.pos += 1;
        let len = if first & 0x80 == 0 {
            first as usize
        } else {
            let n = (first & 0x7f) as usize;
            if n == 0 || n > 4 || self.pos + n > self.buf.len() {
                return None;
            }
            let mut l = 0usize;
            for _ in 0..n {
                l = (l << 8) | self.buf[self.pos] as usize;
                self.pos += 1;
            }
            l
        };
        if self.pos + len > self.buf.len() {
            return None;
        }
        let value = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Some((tag, value))
    }

    fn read_children(&mut self) -> Vec<(u8, &'a [u8])> {
        let mut out = Vec::new();
        while let Some(tlv) = self.read_tlv() {
            out.push(tlv);
        }
        out
    }
}

/// 解析 DER Time（UTCTime 12 位 / GeneralizedTime 14 位，Z 结尾）。
fn parse_der_time(v: &[u8]) -> Option<DateTime<Utc>> {
    let s = std::str::from_utf8(v).ok()?;
    let s = s.trim_end_matches('Z');
    let s = s.split('.').next().unwrap_or(s); // 去掉小数秒
    let d: Vec<u8> = s.bytes().filter(|b| b.is_ascii_digit()).collect();
    let (year, month, day, hour, min, sec) = match d.len() {
        12 => {
            let yy = (d[0] - b'0') as i32 * 10 + (d[1] - b'0') as i32;
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            (
                year,
                (d[2] - b'0') as u32 * 10 + (d[3] - b'0') as u32,
                (d[4] - b'0') as u32 * 10 + (d[5] - b'0') as u32,
                (d[6] - b'0') as u32 * 10 + (d[7] - b'0') as u32,
                (d[8] - b'0') as u32 * 10 + (d[9] - b'0') as u32,
                (d[10] - b'0') as u32 * 10 + (d[11] - b'0') as u32,
            )
        }
        14 => {
            let year = (d[0] - b'0') as i32 * 1000
                + (d[1] - b'0') as i32 * 100
                + (d[2] - b'0') as i32 * 10
                + (d[3] - b'0') as i32;
            (
                year,
                (d[4] - b'0') as u32 * 10 + (d[5] - b'0') as u32,
                (d[6] - b'0') as u32 * 10 + (d[7] - b'0') as u32,
                (d[8] - b'0') as u32 * 10 + (d[9] - b'0') as u32,
                (d[10] - b'0') as u32 * 10 + (d[11] - b'0') as u32,
                (d[12] - b'0') as u32 * 10 + (d[13] - b'0') as u32,
            )
        }
        _ => return None,
    };
    Utc.with_ymd_and_hms(year, month, day, hour, min, sec).single()
}

/// 从证书 PEM 解析 notBefore / notAfter（RFC3339）。
fn extract_validity(crt: &str) -> (Option<String>, Option<String>) {
    // 提取 base64 主体
    let mut b64 = String::new();
    let mut in_pem = false;
    for line in crt.lines() {
        let l = line.trim();
        if l.starts_with("-----BEGIN") {
            in_pem = true;
            continue;
        }
        if l.starts_with("-----END") {
            break;
        }
        if in_pem {
            b64.push_str(l);
        }
    }
    let der = match B64.decode(b64.trim()) {
        Ok(d) => d,
        Err(_) => return (None, None),
    };
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let mut r = DerReader::new(&der);
    let cert_val = match r.read_tlv() {
        Some((0x30, v)) => v,
        _ => return (None, None),
    };
    // 外层子项的第一个才是 TBSCertificate（其后的 signatureAlgorithm /
    // signatureValue 不是 TBSCertificate 字段）。必须再下降一层，
    // 否则会拿 tbs/sigAlg/sigValue 当字段遍历，validity 永远找不到 →
    // 有效期恒为 None → 调度器因 not_after 缺失永不触发自动续期。
    let tbs = match DerReader::new(cert_val).read_tlv() {
        Some((0x30, v)) => v,
        _ => return (None, None),
    };
    // TBSCertificate ::= SEQUENCE { version?, serialNumber, signature, issuer, validity, ... }
    let mut tr = DerReader::new(tbs);
    let children = tr.read_children();
    let mut seq_idx = 0usize;
    let mut validity: Option<&[u8]> = None;
    for (tag, val) in &children {
        if seq_idx == 0 && *tag == 0xA0 {
            continue; // version [0] EXPLICIT
        }
        if seq_idx == 3 {
            validity = Some(val);
            break;
        }
        seq_idx += 1;
    }
    let validity = match validity {
        Some(v) => v,
        None => return (None, None),
    };
    parse_validity_value(validity)
}

/// 解析 Validity ::= SEQUENCE { notBefore Time, notAfter Time }。
fn parse_validity_value(validity: &[u8]) -> (Option<String>, Option<String>) {
    let mut nb = None;
    let mut na = None;
    for (tag, val) in DerReader::new(validity).read_children() {
        if tag == 0x17 || tag == 0x18 {
            if nb.is_none() {
                nb = parse_der_time(val);
            } else {
                na = parse_der_time(val);
            }
        }
    }
    (nb.map(|d| d.to_rfc3339()), na.map(|d| d.to_rfc3339()))
}

// ---------------------------------------------------------------------------
// 证书详情（查看 / 下载）
// ---------------------------------------------------------------------------

/// subjectAltName 扩展 OID（2.5.29.17）
const SAN_OID: &[u8] = &[0x55, 0x1D, 0x11];
/// commonName 属性 OID（2.5.4.3）
const CN_OID: &[u8] = &[0x55, 0x04, 0x03];

/// 证书详情：解析自 PEM 的展示信息 + 证书目录内的 PEM 原文。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertDetail {
    /// 证书主体 CN
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// 签发者 CN
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// 序列号（大写十六进制）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_after: Option<String>,
    /// 距到期剩余天数（负数表示已过期）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_left: Option<i64>,
    /// subjectAltName 中的域名
    #[serde(default)]
    pub san: Vec<String>,
    /// 证书目录内实际存在的 PEM 文件（原名 → 内容）
    #[serde(default)]
    pub pems: HashMap<String, String>,
}

/// 取出 PEM 中**第一个**块的 base64 主体（去掉换行）。
fn pem_first_body_b64(pem: &str) -> Option<String> {
    let mut body = String::new();
    let mut in_pem = false;
    for line in pem.lines() {
        let l = line.trim();
        if l.starts_with("-----BEGIN") {
            if in_pem {
                break;
            }
            in_pem = true;
            continue;
        }
        if l.starts_with("-----END") {
            break;
        }
        if in_pem {
            body.push_str(l);
        }
    }
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

/// 取出 PEM 中第一个块（含 BEGIN/END 行），用于生成「仅含叶子证书」的 cert.cer。
fn pem_first_block(pem: &str) -> String {
    let mut out = String::new();
    let mut in_pem = false;
    for line in pem.lines() {
        let t = line.trim();
        if t.starts_with("-----BEGIN") {
            if in_pem {
                break;
            }
            in_pem = true;
        }
        if !in_pem {
            continue;
        }
        out.push_str(t);
        out.push('\n');
        if t.starts_with("-----END") {
            break;
        }
    }
    out
}

/// 拼出 fullchain：lego 的 `.crt` 在新版本里**已包含**签发者证书，
/// 检测到未包含时（老版本行为）才把 issuer 追加其后，两种布局都得到完整链。
fn build_fullchain(crt: &str, issuer: &str) -> String {
    if issuer.trim().is_empty() {
        return crt.to_string();
    }
    let flat: String = crt.chars().filter(|c| !c.is_whitespace()).collect();
    match pem_first_body_b64(issuer) {
        Some(b) if flat.contains(&b) => crt.to_string(),
        Some(_) => format!("{}\n{}", crt.trim_end(), issuer.trim_start()),
        None => crt.to_string(),
    }
}

/// 从 X.509 Name（SEQUENCE OF RDN）中取指定 OID 的字符串值（本项目只用 CN）。
fn name_value(name: &[u8], oid: &[u8]) -> Option<String> {
    let (tag, body) = DerReader::new(name).read_tlv()?;
    if tag != 0x30 {
        return None;
    }
    for (t, set) in DerReader::new(body).read_children() {
        if t != 0x31 {
            continue;
        }
        for (t2, seq) in DerReader::new(set).read_children() {
            if t2 != 0x30 {
                continue;
            }
            let mut atv = DerReader::new(seq);
            if let Some((toid, oidv)) = atv.read_tlv() {
                if toid == 0x06 && oidv == oid {
                    if let Some((tval, val)) = atv.read_tlv() {
                        // PrintableString / UTF8String / IA5String
                        if tval == 0x13 || tval == 0x0C || tval == 0x16 {
                            return Some(String::from_utf8_lossy(val).to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// 从 extensions [3] EXPLICIT 的 value 中提取 subjectAltName 的 dNSName 列表。
fn extract_san(extensions: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let seq = match DerReader::new(extensions).read_tlv() {
        Some((0x30, v)) => v,
        _ => return out,
    };
    for (t, ext) in DerReader::new(seq).read_children() {
        if t != 0x30 {
            continue;
        }
        let mut er = DerReader::new(ext);
        let (toid, oid) = match er.read_tlv() {
            Some(x) => x,
            None => continue,
        };
        if toid != 0x06 || oid != SAN_OID {
            continue;
        }
        // Extension ::= SEQUENCE { extnID, critical BOOLEAN DEFAULT FALSE, extnValue OCTET STRING }
        let mut inner = None;
        while let Some((t2, v2)) = er.read_tlv() {
            if t2 == 0x04 {
                inner = Some(v2);
                break;
            }
        }
        let gn = match inner.and_then(|v| DerReader::new(v).read_tlv()) {
            Some((0x30, v)) => v,
            _ => continue,
        };
        for (gt, gv) in DerReader::new(gn).read_children() {
            if gt == 0x82 {
                // dNSName [2] IA5String
                out.push(String::from_utf8_lossy(gv).to_string());
            }
        }
    }
    out
}

/// 解析证书 PEM 得到展示信息（CN / 序列号 / 有效期 / SAN）。
fn parse_cert_detail(crt: &str) -> CertDetail {
    let mut detail = CertDetail::default();
    let der = match pem_first_body_b64(crt).and_then(|b| B64.decode(b).ok()) {
        Some(d) => d,
        None => return detail,
    };
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let cert_val = match DerReader::new(&der).read_tlv() {
        Some((0x30, v)) => v,
        _ => return detail,
    };
    let tbs = match DerReader::new(cert_val).read_tlv() {
        Some((0x30, v)) => v,
        _ => return detail,
    };
    // TBSCertificate ::= SEQUENCE { version?, serialNumber, signature, issuer,
    //                                validity, subject, ..., extensions? }
    let mut fields = DerReader::new(tbs).read_children();
    if matches!(fields.first(), Some((0xA0, _))) {
        fields.remove(0); // version [0] EXPLICIT
    }
    let at = |i: usize| fields.get(i).map(|(_, v)| *v);
    if let Some(s) = at(0) {
        detail.serial = Some(s.iter().map(|b| format!("{:02X}", b)).collect());
    }
    if let Some(v) = at(2) {
        detail.issuer = name_value(v, CN_OID);
    }
    if let Some(v) = at(3) {
        let (nb, na) = parse_validity_value(v);
        detail.not_before = nb;
        detail.not_after = na;
    }
    if let Some(v) = at(4) {
        detail.subject = name_value(v, CN_OID);
    }
    if let Some((_, ext)) = fields.iter().find(|(t, _)| *t == 0xA3) {
        detail.san = extract_san(ext);
    }
    detail
}

/// 收集证书目录内的 PEM 文件（原名 → 内容）：仅 `.crt` / `.key`，跳过 `.json` 等元信息。
fn collect_pem_files(cert_dir: &PathBuf) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for dir in lego_cert_dirs(cert_dir) {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".crt") || name.ends_with(".key") {
                    out.insert(name, std::fs::read_to_string(e.path()).unwrap_or_default());
                }
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// 查看证书：解析出的详情 + 目录内 PEM 原文。
#[tauri::command]
pub fn cert_detail(id: String) -> Result<CertDetail, String> {
    let certs = load_certs();
    let cert = find_cert(&certs, &id).ok_or_else(|| "证书不存在".to_string())?;
    let cert_dir = ensure_dir().join(&cert.id);
    let prefer = cert.domains.first().map(|s| s.as_str());
    let pems = read_lego_pems(&cert_dir, prefer)
        .ok_or_else(|| "未找到证书文件，请先申请证书".to_string())?;

    let mut detail = parse_cert_detail(&pems.crt);
    // 解析失败时退回证书记录里已保存的有效期，界面至少能显示已有信息
    if detail.not_after.is_none() {
        detail.not_before = cert.not_before.clone();
        detail.not_after = cert.not_after.clone();
    }
    if let Some(na) = detail
        .not_after
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
    {
        detail.days_left = Some((na.with_timezone(&Utc) - Utc::now()).num_days());
    }
    detail.pems = collect_pem_files(&cert_dir);
    Ok(detail)
}

/// 打包为 zip（Deflated）。
fn build_zip(files: &[(String, String)]) -> Result<Vec<u8>, String> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in files {
            zip.start_file(name.as_str(), opts)
                .map_err(|e| format!("写入 zip 条目失败 ({}): {}", name, e))?;
            std::io::Write::write_all(&mut zip, content.as_bytes())
                .map_err(|e| format!("写入 zip 内容失败 ({}): {}", name, e))?;
        }
        zip.finish().map_err(|e| format!("生成 zip 失败: {}", e))?;
    }
    Ok(cursor.into_inner())
}

/// 导出证书为 zip：通用命名（可直接照服务商文档部署）+ lego 原生文件（可追溯）。
///
/// 结构（`<域名>/`，通配符 `*.a.com` 按 lego 规则写作 `_.a.com`）：
/// - `cert.key` 私钥
/// - `cert.cer` 域名证书（仅叶子）
/// - `chain.cer` 中间证书
/// - `fullchain.cer` 叶子 + 中间（lego 的 `.crt` 本身已含链）
/// - `lego/<原名>` lego 原生文件
#[tauri::command]
pub fn cert_export_zip(id: String, target_path: String) -> Result<String, String> {
    let certs = load_certs();
    let cert = find_cert(&certs, &id).ok_or_else(|| "证书不存在".to_string())?;
    let cert_dir = ensure_dir().join(&cert.id);
    let prefer = cert.domains.first().map(|s| s.as_str());
    let pems = read_lego_pems(&cert_dir, prefer)
        .ok_or_else(|| "未找到证书文件，请先申请证书".to_string())?;

    let dir_name = cert
        .domains
        .first()
        .map(|d| lego_cert_file_stem(d))
        .unwrap_or_else(|| cert.id.clone());

    let mut files: Vec<(String, String)> = vec![
        (format!("{}/cert.key", dir_name), pems.key.clone()),
        (format!("{}/cert.cer", dir_name), pem_first_block(&pems.crt)),
        (format!("{}/chain.cer", dir_name), pems.issuer.clone()),
        (
            format!("{}/fullchain.cer", dir_name),
            build_fullchain(&pems.crt, &pems.issuer),
        ),
        (
            format!("{}/lego/{}", dir_name, pems.crt_file),
            pems.crt.clone(),
        ),
        (
            format!("{}/lego/{}", dir_name, pems.key_file),
            pems.key.clone(),
        ),
    ];
    if let Some(f) = &pems.issuer_file {
        files.push((format!("{}/lego/{}", dir_name, f), pems.issuer.clone()));
    }

    let data = build_zip(&files)?;
    let target = PathBuf::from(&target_path);
    std::fs::write(&target, &data)
        .map_err(|e| format!("写入 zip 失败: path={}, error={}", target.display(), e))?;
    Ok(target.to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------
// 证书监控：TLS 到期检查
// ---------------------------------------------------------------------------

/// 把 DER 编码的证书转成 PEM（复用 parse_cert_detail 的解析入口）。
fn pem_from_der(der: &[u8]) -> String {
    let b64 = B64.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

/// 解析 `example.com:8443` / `https://example.com` / `[::1]:443` 形式的监控目标。
fn split_host_port(raw: &str, default_port: u16) -> (String, u16) {
    let s = raw
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if let Some(rest) = s.strip_prefix('[') {
        if let Some(idx) = rest.find(']') {
            let host = rest[..idx].to_string();
            let port = rest[idx + 1..]
                .trim_start_matches(':')
                .parse::<u16>()
                .unwrap_or(default_port);
            return (host, port);
        }
    }
    if let Some((h, p)) = s.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.to_string(), port);
        }
    }
    (s.to_string(), default_port)
}

struct TlsProbe {
    subject: String,
    issuer: String,
    not_after: Option<String>,
}

/// 与目标做 TLS 握手并读取对端证书。
/// 不校验证书链与主机名：监控只关心到期时间，自签/过期证书同样应被检测到。
async fn probe_tls(host: &str, port: u16) -> Result<TlsProbe, String> {
    let addr = if host.contains(':') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    };
    let tcp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| format!("连接 {} 超时", addr))?
    .map_err(|e| format!("连接 {} 失败: {}", addr, e))?;

    let builder = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| format!("初始化 TLS 失败: {}", e))?;
    let connector = tokio_native_tls::TlsConnector::from(builder);
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        connector.connect(host, tcp),
    )
    .await
    .map_err(|_| format!("{} TLS 握手超时", addr))?
    .map_err(|e| format!("{} TLS 握手失败: {}", addr, e))?;

    let cert = stream
        .get_ref()
        .peer_certificate()
        .map_err(|e| format!("读取对端证书失败: {}", e))?
        .ok_or_else(|| "对端未提供证书".to_string())?;
    let der = cert.to_der().map_err(|e| format!("证书编码失败: {}", e))?;
    let detail = parse_cert_detail(&pem_from_der(&der));
    Ok(TlsProbe {
        subject: detail.subject.unwrap_or_default(),
        issuer: detail.issuer.unwrap_or_default(),
        not_after: detail.not_after,
    })
}

/// 剩余天数命中的提醒阈值：返回最小的 `>= days_left` 阈值。
/// 已过期返回 Some(0)；高于所有阈值返回 None（不提醒）。
fn notify_bucket(days_left: i64, thresholds: &[i64]) -> Option<i64> {
    if days_left <= 0 {
        return Some(0);
    }
    let mut sorted: Vec<i64> = thresholds.iter().copied().filter(|d| *d > 0).collect();
    sorted.sort_unstable();
    sorted.into_iter().find(|d| days_left <= *d)
}

async fn check_one_monitor(
    m: &mut MonitorTarget,
    channels: &[NotifyChannel],
    notify: bool,
) {
    if !m.enabled {
        return;
    }
    let (host, port) = split_host_port(&m.host, if m.port == 0 { 443 } else { m.port });
    if host.is_empty() {
        m.last_error = Some("监控目标缺少域名".to_string());
        return;
    }
    m.last_check_at = Some(Utc::now().to_rfc3339());

    match probe_tls(&host, port).await {
        Ok(p) => {
            m.last_error = None;
            m.subject = p.subject;
            m.issuer = p.issuer;
            m.not_after = p.not_after.clone();
            let days_left = m
                .not_after
                .as_ref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| (d.with_timezone(&Utc) - Utc::now()).num_days());
            m.days_left = days_left;

            let bucket = days_left.and_then(|d| notify_bucket(d, &m.notify_days));
            if bucket.is_none() {
                // 续期成功（剩余天数回到阈值之外）后重置，便于下一次到期重新提醒
                m.last_notify_bucket = None;
                return;
            }
            if notify && bucket != m.last_notify_bucket {
                let na = m.not_after.clone().unwrap_or_default();
                let msg = match days_left {
                    Some(d) if d >= 0 => format!(
                        "{}:{} 的证书将在 {} 天后到期（{}），请及时续期。",
                        host, port, d, na
                    ),
                    Some(d) => format!(
                        "{}:{} 的证书已过期 {} 天（{}）。",
                        host, port, -d, na
                    ),
                    None => format!("{}:{} 无法解析证书有效期（{}）。", host, port, na),
                };
                notify_all(channels, "证书到期提醒", &msg).await;
                m.last_notify_bucket = bucket;
                m.last_notified_at = Some(Utc::now().to_rfc3339());
            }
        }
        Err(e) => {
            let first = m.last_error.is_none();
            m.last_error = Some(e.clone());
            if notify && first {
                notify_all(
                    channels,
                    "证书监控失败",
                    &format!("{}:{} 检测失败：{}", host, port, e),
                )
                .await;
            }
        }
    }
}

async fn run_monitor_checks(notify: bool) -> Vec<MonitorTarget> {
    let mut monitors = load_json::<MonitorTarget>("monitors.json");
    let channels = load_json::<NotifyChannel>("notify_channels.json");
    for m in monitors.iter_mut() {
        check_one_monitor(m, &channels, notify).await;
    }
    save_json("monitors.json", &monitors);
    monitors
}

// ---------------------------------------------------------------------------
// 通知渠道
// ---------------------------------------------------------------------------

async fn notify_send(
    channel: &NotifyChannel,
    title: &str,
    content: &str,
) -> Result<(), String> {
    let cfg_get = |k: &str| {
        channel
            .config
            .get(k)
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let mut url = cfg_get("url");
    if url.is_empty() {
        return Err(format!("通知渠道「{}」缺少 url", channel.name));
    }

    let body = match channel.channel_type.as_str() {
        "dingtalk" => {
            let secret = cfg_get("secret");
            if !secret.is_empty() {
                // 钉钉加签：timestamp + "\n" + secret 做 HMAC-SHA256，Base64 后再 URL 编码
                let ts = Utc::now().timestamp_millis();
                let sign = B64.encode(hmac_sha256(
                    secret.as_bytes(),
                    &format!("{}\n{}", ts, secret),
                ));
                let sep = if url.contains('?') { '&' } else { '?' };
                url = format!("{}{}timestamp={}&sign={}", url, sep, ts, percent_encode(&sign));
            }
            serde_json::json!({
                "msgtype": "markdown",
                "markdown": { "title": title, "text": format!("### {}\n\n{}", title, content) }
            })
        }
        "wecom" => serde_json::json!({
            "msgtype": "markdown",
            "markdown": { "content": format!("**{}**\n\n{}", title, content) }
        }),
        "slack" => serde_json::json!({ "text": format!("*{}*\n{}", title, content) }),
        "discord" => serde_json::json!({ "content": format!("**{}**\n{}", title, content) }),
        "teams" => serde_json::json!({
            "@type": "MessageCard",
            "@context": "https://schema.org/extensions",
            "summary": title,
            "themeColor": "0078D7",
            "sections": [{ "activityTitle": title, "text": content }]
        }),
        "webhook" => serde_json::json!({
            "event": "cert_notify",
            "title": title,
            "content": content,
            "timestamp": Utc::now().to_rfc3339()
        }),
        other => return Err(format!("未知通知类型: {}", other)),
    };

    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?
        .post(&url)
        .json(&body);
    let token = cfg_get("token");
    if channel.channel_type == "webhook" && !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("发送通知失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("通知接口返回 {}: {}", status, text.trim()));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(code) = v.get("errcode").and_then(|x| x.as_i64()) {
            if code != 0 {
                return Err(format!(
                    "通知接口返回错误: {} - {}",
                    code,
                    v.get("errmsg").and_then(|x| x.as_str()).unwrap_or("")
                ));
            }
        }
    }
    Ok(())
}

async fn notify_all(channels: &[NotifyChannel], title: &str, content: &str) {
    for ch in channels.iter().filter(|c| c.enabled) {
        if let Err(e) = notify_send(ch, title, content).await {
            eprintln!("[cert] 通知「{}」发送失败: {}", ch.name, e);
        }
    }
}

// ---------------------------------------------------------------------------
// 调度器
// ---------------------------------------------------------------------------

/// 启动应用内后台调度器（在 lib.rs setup 中调用）。
/// 节奏：启动 60 秒后先做首次扫描（避免到期证书要等满一个间隔——默认 6 小时——
/// 才被检查；也让调度页尽快有 last_run/next_run 可显示），之后按配置间隔循环。
pub fn start_scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let app = app;
        // 首轮仅延迟 60s（尽快做首次扫描 + 让调度页有 last_run/next_run 可显示）；
        // 之后每次睡眠前读取最新 interval，用户在睡眠期间修改间隔时下一轮立即生效。
        let mut first = true;
        loop {
            // 读取间隔（短锁，不跨 await）
            let interval = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                s.interval_minutes
            };
            let delay_secs = if first { 60u64 } else { interval.saturating_mul(60).max(60) };
            first = false;
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

            // 收集本周期需要续期的证书（短锁，不跨 await）
            let due_certs: Vec<String> = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                if !s.enabled {
                    Vec::new()
                } else {
                    let certs = load_certs();
                    let now = Utc::now();
                    certs
                        .iter()
                        .filter_map(|c| {
                            let due = c.not_after.as_ref().and_then(|na| {
                                DateTime::parse_from_rfc3339(na)
                                    .ok()
                                    .map(|d| now + Duration::days(c.renew_before_days) >= d.with_timezone(&Utc))
                            });
                            if due == Some(true) {
                                Some(c.id.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                }
            };

            for id in due_certs {
                // 定时任务不强制：到没到期由 lego 按 renew_before_days 判断
                let result = issue_and_deploy(&id, false).await;
                let (ok, message) = match &result {
                    Ok(r) if r.renewed => (true, "已续期".to_string()),
                    Ok(r) => (
                        true,
                        format!("未到续期时间，已跳过（剩余 {} 天）", r.days_left.unwrap_or(0)),
                    ),
                    Err(e) => (false, e.clone()),
                };
                {
                    let st = app.state::<CertScheduler>();
                    let mut s = st.state.lock().unwrap();
                    s.push_log(SchedulerLogEntry {
                        at: Utc::now().to_rfc3339(),
                        cert_id: id.clone(),
                        action: "renew".into(),
                        ok,
                        message: message.clone(),
                    });
                }
                // 续期结果通过已启用的通知渠道推送（未配置渠道时无操作）
                let channels = load_json::<NotifyChannel>("notify_channels.json");
                notify_all(
                    &channels,
                    if ok { "证书续期成功" } else { "证书续期失败" },
                    &format!("证书 {}：{}", id, message),
                )
                .await;
            }

            // 到期扫描后顺带检查外部域名证书（同一调度周期）
            let scheduler_enabled = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                s.enabled
            };
            if scheduler_enabled {
                run_monitor_checks(true).await;
            }

            // 更新运行时间（短锁）
            let st = app.state::<CertScheduler>();
            let mut s = st.state.lock().unwrap();
            let now = Utc::now();
            s.last_run_at = Some(now.to_rfc3339());
            s.next_run_at = Some((now + Duration::minutes(interval as i64)).to_rfc3339());
        }
    });
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn cert_list() -> Vec<Certificate> {
    let mut certs = load_certs();
    // 旧数据补解析：已申请但缺有效期的证书，从本地 PEM 读取
    let mut changed = false;
    for c in certs.iter_mut() {
        if c.not_after.is_some() {
            continue;
        }
        let cert_dir = ensure_dir().join(&c.id);
        let mut parsed = false;
        for dir in lego_cert_dirs(&cert_dir) {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".crt") && !name.contains("issuer") {
                        if let Ok(content) = std::fs::read_to_string(e.path()) {
                            let (nb, na) = extract_validity(&content);
                            if na.is_some() {
                                c.not_before = nb;
                                c.not_after = na;
                                if c.status == "pending" {
                                    c.status = "issued".to_string();
                                }
                                changed = true;
                            }
                        }
                        parsed = true;
                        break;
                    }
                }
            }
            if parsed {
                break;
            }
        }
    }
    if changed {
        save_json("certificates.json", &certs);
    }
    certs
}

#[tauri::command]
pub fn cert_create(
    domains: Vec<String>,
    email: String,
    ca: Option<String>,
    dns_provider: String,
    credential_id: String,
    renew_before_days: Option<i64>,
    deploy_node_ids: Option<Vec<String>>,
    validation: Option<String>,
    http_port: Option<u16>,
    deploy_links: Option<Vec<DeployLink>>,
) -> Certificate {
    let mut certs = load_certs();
    let links = deploy_links.unwrap_or_else(|| {
        deploy_node_ids
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|node_id| DeployLink {
                node_id,
                target: String::new(),
            })
            .collect()
    });
    let node_ids = deploy_node_ids
        .unwrap_or_else(|| links.iter().map(|l| l.node_id.clone()).collect());
    let cert = Certificate {
        id: gen_id("cert"),
        domains,
        email,
        ca: ca.unwrap_or_else(default_ca),
        dns_provider,
        credential_id,
        validation: validation.unwrap_or_else(default_validation),
        http_port: http_port.unwrap_or_else(default_http_port),
        renew_before_days: renew_before_days.unwrap_or_else(default_days),
        not_before: None,
        not_after: None,
        last_issue_at: None,
        status: default_status(),
        last_error: None,
        deploy_links: links,
        deploy_node_ids: node_ids,
    };
    certs.push(cert.clone());
    save_json("certificates.json", &certs);
    cert
}

#[tauri::command]
pub fn cert_update(
    id: String,
    domains: Vec<String>,
    email: String,
    ca: String,
    dns_provider: String,
    credential_id: String,
    validation: String,
    http_port: u16,
    renew_before_days: i64,
) -> Result<Certificate, String> {
    let mut certs = load_certs();
    let c = certs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "证书不存在".to_string())?;
    c.domains = domains;
    c.email = email;
    c.ca = ca;
    c.dns_provider = dns_provider;
    c.credential_id = credential_id;
    c.validation = validation;
    c.http_port = http_port.max(1);
    c.renew_before_days = renew_before_days;
    let updated = c.clone();
    save_json("certificates.json", &certs);
    Ok(updated)
}

#[tauri::command]
pub fn cert_update_nodes(id: String, deploy_node_ids: Vec<String>) -> Result<(), String> {
    let mut certs = load_certs();
    let c = certs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "证书不存在".to_string())?;
    let old = c.deploy_links.clone();
    c.deploy_links = deploy_node_ids
        .iter()
        .map(|node_id| DeployLink {
            node_id: node_id.clone(),
            target: old
                .iter()
                .find(|l| &l.node_id == node_id)
                .map(|l| l.target.clone())
                .unwrap_or_default(),
        })
        .collect();
    c.deploy_node_ids = deploy_node_ids;
    save_json("certificates.json", &certs);
    Ok(())
}

#[tauri::command]
pub fn cert_update_links(id: String, deploy_links: Vec<DeployLink>) -> Result<(), String> {
    let mut certs = load_certs();
    let c = certs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "证书不存在".to_string())?;
    c.deploy_node_ids = deploy_links.iter().map(|l| l.node_id.clone()).collect();
    c.deploy_links = deploy_links;
    save_json("certificates.json", &certs);
    Ok(())
}

#[tauri::command]
pub fn cert_delete(id: String) {
    let certs: Vec<Certificate> = load_certs()
        .into_iter()
        .filter(|c| c.id != id)
        .collect();
    save_json("certificates.json", &certs);
}

#[tauri::command]
pub async fn cert_issue_now(id: String) -> Result<IssueResult, String> {
    // 手动点「申请」= 立刻重签，不受续期阈值限制（否则 lego 会静默跳过）
    issue_and_deploy(&id, true).await
}

#[tauri::command]
pub fn cert_get_pem(id: String) -> Result<HashMap<String, String>, String> {
    let certs = load_certs();
    let cert = find_cert(&certs, &id).ok_or_else(|| "证书不存在".to_string())?;
    Ok(collect_pem_files(&ensure_dir().join(&cert.id)))
}

#[tauri::command]
pub fn deploy_node_list() -> Vec<DeployNode> {
    load_json::<DeployNode>("deploy_nodes.json")
}

#[tauri::command]
pub fn deploy_node_upsert(
    id: Option<String>,
    node_type: String,
    name: String,
    config: HashMap<String, String>,
    deploy_before_days: Option<i64>,
    note: Option<String>,
) -> DeployNode {
    let mut nodes = load_json::<DeployNode>("deploy_nodes.json");
    let nid = id.unwrap_or_else(|| gen_id("node"));
    let node = if let Some(pos) = nodes.iter().position(|n| n.id == nid) {
        let mut n = nodes[pos].clone();
        n.node_type = node_type;
        n.name = name;
        n.config = config;
        n.deploy_before_days = deploy_before_days.unwrap_or_else(default_days);
        n.note = note.unwrap_or_default();
        n
    } else {
        DeployNode {
            id: nid.clone(),
            node_type,
            name,
            config,
            deploy_before_days: deploy_before_days.unwrap_or_else(default_days),
            last_deploy_at: None,
            last_deploy_error: None,
            note: note.unwrap_or_default(),
        }
    };
    if let Some(pos) = nodes.iter().position(|n| n.id == nid) {
        nodes[pos] = node.clone();
    } else {
        nodes.push(node.clone());
    }
    save_json("deploy_nodes.json", &nodes);
    node
}

#[tauri::command]
pub fn deploy_node_delete(id: String) {
    let nodes: Vec<DeployNode> = load_json::<DeployNode>("deploy_nodes.json")
        .into_iter()
        .filter(|n| n.id != id)
        .collect();
    save_json("deploy_nodes.json", &nodes);
    // 同时解除证书对它的关联
    let certs: Vec<Certificate> = load_certs()
        .into_iter()
        .map(|mut c| {
            c.deploy_links.retain(|l| l.node_id != id);
            c.deploy_node_ids.retain(|d| d != &id);
            c
        })
        .collect();
    save_json("certificates.json", &certs);
}

#[tauri::command]
pub async fn deploy_node_test(id: String) -> Result<String, String> {
    let nodes = load_json::<DeployNode>("deploy_nodes.json");
    let node = find_node(&nodes, &id).ok_or_else(|| "节点不存在".to_string())?;
    match node.node_type.as_str() {
        "windows" | "api" | "docker" => {
            let url = node.config.get("url").cloned().unwrap_or_default();
            let token = node.config.get("token").cloned().unwrap_or_default();
            let base = windows_base_url(&url);
            if base.is_empty() {
                return Err(format!("{} 节点缺少 url", node.node_type));
            }
            // 与部署同一基础地址派生 /health（兼容用户填写的 /push 后缀）
            let health = format!("{}/health", base);
            let resp = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| e.to_string())?
                .get(&health)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| format!("连接失败: {}", e))?;
            Ok(format!("接收端响应 {}", resp.status()))
        }
        "ssh" | "linux" => {
            let auth = ssh_auth(node);
            let (mut ssh_cmd, target) = ssh_cmd_args(node, &auth, "echo ok")?;
            match ssh_cmd.output() {
                Ok(o) if o.status.success() => Ok(format!("SSH 连通（{}）", target)),
                Ok(o) => Err(format!("SSH 失败: {}", String::from_utf8_lossy(&o.stderr).trim())),
                Err(e) => Err(format!("无法执行 ssh: {}", e)),
            }
        }
        "webhook" => {
            let url = cfg(node, "url");
            if url.is_empty() {
                return Err("webhook 节点缺少 url".into());
            }
            let resp = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?
                .post(&url)
                .json(&serde_json::json!({ "event": "test" }))
                .send()
                .await
                .map_err(|e| format!("连接失败: {}", e))?;
            Ok(format!("Webhook 响应 {}", resp.status()))
        }
        "baota" => baota_test(node).await,
        "aliyun" | "aliyun_ssl" | "tencent_ssl" | "qiniu" | "qiniu_ssl" => Ok(
            "云厂商类型不支持预检，凭据会在实际部署时校验".to_string(),
        ),
        other => Ok(format!("节点类型 {} 暂不支持连通性测试", other)),
    }
}

#[tauri::command]
pub fn credential_list() -> Vec<Credential> {
    load_json::<Credential>("credentials.json")
        .into_iter()
        .map(decrypt_credential)
        .collect()
}

#[tauri::command]
pub fn credential_upsert(
    id: Option<String>,
    name: String,
    cred_type: String,
    data: HashMap<String, String>,
    note: Option<String>,
) -> Credential {
    let mut creds = load_json::<Credential>("credentials.json");
    let cid = id.unwrap_or_else(|| gen_id("cred"));
    let cred = Credential {
        id: cid.clone(),
        name,
        cred_type,
        data,
        note: note.unwrap_or_default(),
    };
    let stored = encrypt_credential(cred.clone());
    if let Some(pos) = creds.iter().position(|c| c.id == cid) {
        creds[pos] = stored;
    } else {
        creds.push(stored);
    }
    save_json("credentials.json", &creds);
    cred // 返回明文版本给前端
}

#[tauri::command]
pub fn credential_delete(id: String) {
    let creds: Vec<Credential> = load_json::<Credential>("credentials.json")
        .into_iter()
        .filter(|c| c.id != id)
        .collect();
    save_json("credentials.json", &creds);
}

#[tauri::command]
pub fn cert_scheduler_status(state: tauri::State<CertScheduler>) -> SchedulerState {
    state.state.lock().unwrap().clone()
}

#[tauri::command]
pub fn cert_scheduler_set(enabled: bool, interval_minutes: Option<u64>, state: tauri::State<CertScheduler>) {
    let mut s = state.state.lock().unwrap();
    s.enabled = enabled;
    if let Some(i) = interval_minutes {
        s.interval_minutes = i.max(1);
    }
}

#[tauri::command]
pub async fn cert_scheduler_run_now() -> Result<Vec<String>, String> {
    let certs = load_certs();
    let now = Utc::now();
    let mut results = Vec::new();
    for c in &certs {
        // 与后台调度同一口径：只有到期（或从未签发、没有有效期）的证书才需要动作。
        // 以前这里无条件全量调用，未到期的也会「跑一遍」，结果全是 lego 静默跳过。
        let due = c
            .not_after
            .as_ref()
            .and_then(|na| DateTime::parse_from_rfc3339(na).ok())
            .map(|d| now + Duration::days(c.renew_before_days) >= d.with_timezone(&Utc))
            .unwrap_or(true);
        if !due {
            results.push(format!("{}: 未到续期时间，已跳过", c.id));
            continue;
        }
        match issue_and_deploy(&c.id, false).await {
            Ok(r) if r.renewed => results.push(format!("{}: ok", c.id)),
            Ok(r) => results.push(format!(
                "{}: 未到续期时间，已跳过（剩余 {} 天）",
                c.id,
                r.days_left.unwrap_or(0)
            )),
            Err(e) => results.push(format!("{}: {}", c.id, e)),
        }
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Tauri 命令：证书监控 / 通知渠道
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn monitor_list() -> Vec<MonitorTarget> {
    load_json("monitors.json")
}

#[tauri::command]
pub fn monitor_upsert(
    id: Option<String>,
    host: String,
    port: Option<u16>,
    note: Option<String>,
    enabled: Option<bool>,
    notify_days: Option<Vec<i64>>,
) -> Result<MonitorTarget, String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("请填写要监控的域名或 IP".into());
    }
    let thresholds = match notify_days {
        Some(d) => {
            let mut v: Vec<i64> = d.into_iter().filter(|x| *x > 0).collect();
            v.sort_unstable();
            v.dedup();
            if v.is_empty() { default_notify_days() } else { v }
        }
        None => default_notify_days(),
    };
    let mut list = load_json::<MonitorTarget>("monitors.json");
    let mid = id.unwrap_or_else(|| gen_id("mon"));
    let target = if let Some(pos) = list.iter().position(|m| m.id == mid) {
        let mut m = list[pos].clone();
        m.host = host;
        m.port = port.unwrap_or(m.port);
        m.note = note.unwrap_or(m.note);
        m.enabled = enabled.unwrap_or(m.enabled);
        m.notify_days = thresholds;
        m
    } else {
        MonitorTarget {
            id: mid.clone(),
            host,
            port: port.unwrap_or(443),
            note: note.unwrap_or_default(),
            enabled: enabled.unwrap_or(true),
            notify_days: thresholds,
            not_after: None,
            days_left: None,
            subject: String::new(),
            issuer: String::new(),
            last_check_at: None,
            last_error: None,
            last_notify_bucket: None,
            last_notified_at: None,
        }
    };
    if let Some(pos) = list.iter().position(|m| m.id == mid) {
        list[pos] = target.clone();
    } else {
        list.push(target.clone());
    }
    save_json("monitors.json", &list);
    Ok(target)
}

#[tauri::command]
pub fn monitor_delete(id: String) {
    let list: Vec<MonitorTarget> = load_json::<MonitorTarget>("monitors.json")
        .into_iter()
        .filter(|m| m.id != id)
        .collect();
    save_json("monitors.json", &list);
}

#[tauri::command]
pub async fn monitor_check(id: String) -> Result<MonitorTarget, String> {
    let mut list = load_json::<MonitorTarget>("monitors.json");
    let channels = load_json::<NotifyChannel>("notify_channels.json");
    let m = list
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or_else(|| "监控目标不存在".to_string())?;
    check_one_monitor(m, &channels, true).await;
    let out = m.clone();
    save_json("monitors.json", &list);
    Ok(out)
}

#[tauri::command]
pub async fn monitor_check_all() -> Vec<MonitorTarget> {
    run_monitor_checks(true).await
}

#[tauri::command]
pub fn notify_channel_list() -> Vec<NotifyChannel> {
    load_json("notify_channels.json")
}

#[tauri::command]
pub fn notify_channel_upsert(
    id: Option<String>,
    channel_type: String,
    name: String,
    config: HashMap<String, String>,
    enabled: Option<bool>,
    note: Option<String>,
) -> Result<NotifyChannel, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("请填写通知渠道名称".into());
    }
    let url = config.get("url").cloned().unwrap_or_default();
    if url.trim().is_empty() {
        return Err("请填写 Webhook 地址".into());
    }
    let mut list = load_json::<NotifyChannel>("notify_channels.json");
    let cid = id.unwrap_or_else(|| gen_id("notify"));
    let channel = if let Some(pos) = list.iter().position(|c| c.id == cid) {
        let mut c = list[pos].clone();
        c.channel_type = channel_type;
        c.name = name;
        c.config = config;
        c.enabled = enabled.unwrap_or(c.enabled);
        c.note = note.unwrap_or(c.note);
        c
    } else {
        NotifyChannel {
            id: cid.clone(),
            channel_type,
            name,
            config,
            enabled: enabled.unwrap_or(true),
            note: note.unwrap_or_default(),
        }
    };
    if let Some(pos) = list.iter().position(|c| c.id == cid) {
        list[pos] = channel.clone();
    } else {
        list.push(channel.clone());
    }
    save_json("notify_channels.json", &list);
    Ok(channel)
}

#[tauri::command]
pub fn notify_channel_delete(id: String) {
    let list: Vec<NotifyChannel> = load_json::<NotifyChannel>("notify_channels.json")
        .into_iter()
        .filter(|c| c.id != id)
        .collect();
    save_json("notify_channels.json", &list);
}

#[tauri::command]
pub async fn notify_channel_test(id: String) -> Result<String, String> {
    let list = load_json::<NotifyChannel>("notify_channels.json");
    let channel = list
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "通知渠道不存在".to_string())?;
    notify_send(
        channel,
        "AnyVersion 测试通知",
        "这是一条测试消息：证书监控与部署通知已配置成功。",
    )
    .await?;
    Ok("已发送".to_string())
}

// ---------------------------------------------------------------------------
// 测试：有效期解析 / Windows 接收端 URL 归一化
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造最小 DER TLV（测试数据用，兼容长格式长度）
    fn der_tlv(tag: u8, value: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        let len = value.len();
        if len < 0x80 {
            out.push(len as u8);
        } else {
            let mut bytes = Vec::new();
            let mut l = len;
            while l > 0 {
                bytes.insert(0, (l & 0xff) as u8);
                l >>= 8;
            }
            out.push(0x80 | bytes.len() as u8);
            out.extend(bytes);
        }
        out.extend(value);
        out
    }

    /// 构造结构完整的 X.509 证书 DER（解析器只走 TLV 结构，签名内容可为假）。
    /// `with_version` = false 模拟 v1 证书（TBSCertificate 无 [0] version 字段）。
    fn test_cert_der(nb: &[u8], na: &[u8], with_version: bool) -> Vec<u8> {
        let mut validity_inner = Vec::new();
        validity_inner.extend(der_tlv(0x17, nb));
        validity_inner.extend(der_tlv(0x17, na));
        let validity = der_tlv(0x30, &validity_inner);
        let serial = der_tlv(0x02, &[0, 1]);
        let sig_alg = der_tlv(0x30, &der_tlv(0x06, &[43, 14, 3, 2, 26]));
        let issuer = der_tlv(0x30, &der_tlv(0x06, &[0x55, 0x04, 0x03, 3]));
        let subject = der_tlv(0x30, &der_tlv(0x06, &[0x55, 0x04, 0x03, 3]));
        let mut tbs_inner = Vec::new();
        if with_version {
            tbs_inner.extend(der_tlv(0xA0, &der_tlv(0x02, &[2]))); // v3
        }
        tbs_inner.extend(&serial);
        tbs_inner.extend(&sig_alg);
        tbs_inner.extend(&issuer);
        tbs_inner.extend(&validity);
        tbs_inner.extend(&subject);
        let tbs = der_tlv(0x30, &tbs_inner);
        // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
        let mut cert_inner = Vec::new();
        cert_inner.extend(&tbs);
        cert_inner.extend(&sig_alg);
        cert_inner.extend(der_tlv(0x03, &[0])); // BIT STRING（空签名）
        der_tlv(0x30, &cert_inner)
    }

    fn der_to_pem(der: &[u8]) -> String {
        let b64 = B64.encode(der);
        let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
        for chunk in b64.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(chunk).unwrap());
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
        pem
    }

    /// 回归：extract_validity 必须从真实 X.509 结构（外层 Certificate → tbsCertificate
    /// → validity）解析出有效期。曾有一版直接读外层子项（tbs/sigAlg/sigValue），
    /// 导致有效期永远解析为 None → 调度器因 not_after 缺失永不触发自动续期。
    #[test]
    fn extract_validity_parses_real_cert_structure() {
        let pem = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
        let (nb, na) = extract_validity(&pem);
        assert_eq!(nb.as_deref(), Some("2025-08-01T12:00:00+00:00"));
        assert_eq!(na.as_deref(), Some("2026-08-01T12:00:00+00:00"));
    }

    /// v1 证书（无 version 字段）同样可解析
    #[test]
    fn extract_validity_without_version_field() {
        let pem = der_to_pem(&test_cert_der(b"240101000000Z", b"241001000000Z", false));
        let (nb, na) = extract_validity(&pem);
        assert_eq!(nb.as_deref(), Some("2024-01-01T00:00:00+00:00"));
        assert_eq!(na.as_deref(), Some("2024-10-01T00:00:00+00:00"));
    }

    /// GeneralizedTime（14 位，2050 年后证书常见）
    #[test]
    fn extract_validity_generalized_time() {
        let pem = der_to_pem(&test_cert_der(b"250101000000Z", b"20801231235959Z", true));
        let (_, na) = extract_validity(&pem);
        assert_eq!(na.as_deref(), Some("2080-12-31T23:59:59+00:00"));
    }

    #[test]
    fn parse_der_time_utc_and_generalized() {
        let d = parse_der_time(b"250801120000Z").unwrap();
        assert_eq!(d.to_rfc3339(), "2025-08-01T12:00:00+00:00");
        let d = parse_der_time(b"20491231235959Z").unwrap();
        assert_eq!(d.to_rfc3339(), "2049-12-31T23:59:59+00:00");
    }

    /// Windows 接收端 URL 归一化：部署（/push）与连通测试（/health）
    /// 必须从同一基础地址派生；兼容用户填写的 /push 后缀与结尾斜杠。
    #[test]
    fn windows_base_url_normalization() {
        assert_eq!(windows_base_url("http://h:9000"), "http://h:9000");
        assert_eq!(windows_base_url("http://h:9000/"), "http://h:9000");
        assert_eq!(windows_base_url("http://h:9000/push"), "http://h:9000");
        assert_eq!(windows_base_url("http://h:9000/push/"), "http://h:9000");
        assert_eq!(windows_base_url("http://h:9000/PUSH"), "http://h:9000");
        assert_eq!(windows_base_url("  http://h:9000/push  "), "http://h:9000");
        assert_eq!(windows_base_url(""), "");
        // 结尾的 /push 段一律剥离（含反向代理子路径场景）：
        // 部署与连通测试从同一 base 派生，保证两者指向同一个接收端
        assert_eq!(windows_base_url("http://h:9000/api/push"), "http://h:9000/api");
    }

    /// 阿里云 RPC 签名：参数按字典序、RFC3986 编码、POST + `/` 参与签名。
    /// 期望值由独立实现计算，防止百分号编码或排序被改坏。
    #[test]
    fn aliyun_rpc_signature_matches_reference() {
        let mut p = BTreeMap::new();
        p.insert("AccessKeyId".to_string(), "ak".to_string());
        p.insert("Action".to_string(), "UploadUserCertificate".to_string());
        p.insert(
            "Cert".to_string(),
            "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----".to_string(),
        );
        p.insert("Format".to_string(), "JSON".to_string());
        p.insert(
            "Key".to_string(),
            "-----BEGIN PRIVATE KEY-----\nBBB\n-----END PRIVATE KEY-----".to_string(),
        );
        p.insert("Name".to_string(), "a.com-2026".to_string());
        p.insert("SignatureMethod".to_string(), "HMAC-SHA1".to_string());
        p.insert("SignatureVersion".to_string(), "1.0".to_string());
        p.insert("SignatureNonce".to_string(), "1234".to_string());
        p.insert("Timestamp".to_string(), "2026-01-01T00:00:00Z".to_string());
        p.insert("Version".to_string(), "2020-04-07".to_string());
        assert_eq!(aliyun_rpc_sign(&p, "sk"), "1Td495sRTleqWu7Jt3Zd65Pvwek=");
    }

    #[test]
    fn remote_name_and_cert_name_are_path_safe() {
        assert_eq!(remote_safe_name("*.a.com"), "_.a.com");
        assert_eq!(remote_safe_name("../../x"), ".._.._x");
        let cert = Certificate {
            id: "cert-1".into(),
            domains: vec!["a.com".into()],
            email: String::new(),
            ca: default_ca(),
            dns_provider: "alidns".into(),
            credential_id: String::new(),
            validation: default_validation(),
            http_port: 80,
            renew_before_days: 30,
            not_before: None,
            not_after: None,
            last_issue_at: None,
            status: default_status(),
            last_error: None,
            deploy_links: Vec::new(),
            deploy_node_ids: Vec::new(),
        };
        let name = cert_name_for(&cert, "", "fallback");
        assert!(name.starts_with("a.com-"));
        assert!(name.len() <= 63);
    }

    fn unique_tmp(prefix: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), n));
        let _ = std::fs::create_dir_all(&p);
        p
    }

    /// lego 的 `--path` 即数据根目录：证书落在 `<path>/certificates/`。
    /// 必须能从此处读到 PEM，否则签发成功也会报「未找到生成的 PEM 文件」。
    #[test]
    fn lego_pems_read_from_data_root_certificates_dir() {
        let base = unique_tmp("anyver_lego_root");
        let store = base.join("certificates");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("_.a.com.crt"), "LEAF").unwrap();
        std::fs::write(store.join("_.a.com.key"), "KEY").unwrap();
        std::fs::write(store.join("_.a.com.issuer.crt"), "ISSUER").unwrap();

        let got = read_lego_pems(&base, Some("*.a.com"))
            .expect("应能从 <path>/certificates 读到 PEM");
        assert_eq!(got.crt, "LEAF");
        assert_eq!(got.key, "KEY");
        assert_eq!(got.issuer, "ISSUER");
        assert_eq!(got.crt_file, "_.a.com.crt");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 同一数据目录存在多份证书时，按域名精确取名，
    /// 不得因 `read_dir` 顺序随机取到别的域名的证书。
    #[test]
    fn lego_pems_prefer_domain_over_other_certificates() {
        let base = unique_tmp("anyver_lego_prefer");
        let store = base.join("certificates");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("a.com.crt"), "CRT_A").unwrap();
        std::fs::write(store.join("a.com.key"), "KEY_A").unwrap();
        std::fs::write(store.join("b.com.crt"), "CRT_B").unwrap();
        std::fs::write(store.join("b.com.key"), "KEY_B").unwrap();

        let got = read_lego_pems(&base, Some("b.com")).unwrap();
        assert_eq!(got.crt, "CRT_B");
        assert_eq!(got.key, "KEY_B");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 兼容历史（错误的）`<path>/.lego/certificates` 布局，避免旧数据读不到。
    #[test]
    fn lego_pems_legacy_dot_lego_layout_still_readable() {
        let base = unique_tmp("anyver_lego_legacy");
        let store = base.join(".lego").join("certificates");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("x.com.crt"), "L").unwrap();
        std::fs::write(store.join("x.com.key"), "K").unwrap();

        let got = read_lego_pems(&base, None).expect("旧布局应仍可读取");
        assert_eq!(got.crt, "L");
        assert_eq!(got.key, "K");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 「跳过续期」必须能被认出来：lego 未到续期时间时仍然以退出码 0 结束（v5.3.1 实测），
    /// 只能靠证书文件有没有变来判断。认不出来的话，后端就会把「什么都没做」记成一次成功签发——
    /// 界面上的表现正是「点了申请没反应」。
    #[test]
    fn lego_skip_detected_by_unchanged_cert_files() {
        let base = unique_tmp("anyver_lego_skip");
        let store = base.join("certificates");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("a.com.crt"), "LEAF").unwrap();
        std::fs::write(store.join("a.com.key"), "KEY").unwrap();

        // 已有证书 → 之后按「续期」语义调用 lego
        assert!(has_existing_cert(&base));

        // 被跳过：文件一个都不变
        let before = cert_store_fingerprint(&base);
        assert_eq!(before, cert_store_fingerprint(&base));

        // 真正重签：cert 文件变化
        std::fs::write(store.join("a.com.crt"), "NEW-LEAF").unwrap();
        assert_ne!(before, cert_store_fingerprint(&base));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 空的证书目录不算「已有证书」，否则首次申请会被当成续期。
    #[test]
    fn empty_cert_dir_has_no_existing_cert() {
        let base = unique_tmp("anyver_lego_empty");
        assert!(!has_existing_cert(&base));
        assert!(cert_store_fingerprint(&base).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 查看证书：有效期与序列号都能从 PEM 解析出来
    #[test]
    fn parse_cert_detail_extracts_validity_and_serial() {
        let pem = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
        let d = parse_cert_detail(&pem);
        assert_eq!(d.not_before.as_deref(), Some("2025-08-01T12:00:00+00:00"));
        assert_eq!(d.not_after.as_deref(), Some("2026-08-01T12:00:00+00:00"));
        assert_eq!(d.serial.as_deref(), Some("0001"));
    }

    /// cert.cer 只取第一段证书（叶子），不含链
    #[test]
    fn pem_first_block_keeps_only_leaf() {
        let leaf = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
        let chain = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", false));
        let both = format!("{}{}", leaf, chain);

        let first = pem_first_block(&both);
        assert_eq!(first.matches("-----BEGIN CERTIFICATE-----").count(), 1);
        assert_eq!(first, leaf);
    }

    /// 新版 lego 的 .crt 已含签发者证书 → fullchain 直接用它，不重复拼接
    #[test]
    fn build_fullchain_skips_duplicate_issuer() {
        let leaf = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
        let chain = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", false));
        let crt = format!("{}{}", leaf, chain);
        let full = build_fullchain(&crt, &chain);
        assert_eq!(full, crt);
        assert_eq!(full.matches("-----BEGIN CERTIFICATE-----").count(), 2);
    }

    /// 老版 lego 的 .crt 只有叶子 → fullchain 追加 issuer，得到叶子 + 中间
    #[test]
    fn build_fullchain_appends_missing_issuer() {
        let leaf = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
        let chain = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", false));
        let full = build_fullchain(&leaf, &chain);
        assert!(full.starts_with(leaf.trim_end()));
        assert!(full.contains(chain.trim_end()));
        assert_eq!(full.matches("-----BEGIN CERTIFICATE-----").count(), 2);
    }

    /// 导出 zip 的条目可被重新读出（验证打包未损坏）
    #[test]
    fn build_zip_roundtrip() {
        use std::io::Read;
        let files = vec![
            ("a.com/cert.key".to_string(), "KEY\n".to_string()),
            ("a.com/cert.cer".to_string(), "CERT\n".to_string()),
        ];
        let data = build_zip(&files).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
        assert_eq!(archive.len(), 2);
        let mut s = String::new();
        archive
            .by_name("a.com/cert.key")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        assert_eq!(s, "KEY\n");
    }

    /// 监控目标输入兼容：裸域名、带 scheme、带端口、IPv6 方括号
    #[test]
    fn split_host_port_parses_common_forms() {
        assert_eq!(split_host_port("example.com", 443), ("example.com".into(), 443));
        assert_eq!(split_host_port("https://example.com/", 443), ("example.com".into(), 443));
        assert_eq!(split_host_port("example.com:8443", 443), ("example.com".into(), 8443));
        assert_eq!(split_host_port("  http://a.com:80  ", 443), ("a.com".into(), 80));
        assert_eq!(split_host_port("[::1]:8443", 443), ("::1".into(), 8443));
        assert_eq!(split_host_port("[::1]", 443), ("::1".into(), 443));
    }

    /// 到期提醒按阈值分桶：同一档只提醒一次，续期后自动回到 None
    #[test]
    fn notify_bucket_crosses_each_threshold_once() {
        let t = [1, 3, 7, 14, 30];
        assert_eq!(notify_bucket(45, &t), None);
        assert_eq!(notify_bucket(30, &t), Some(30));
        assert_eq!(notify_bucket(20, &t), Some(30));
        assert_eq!(notify_bucket(14, &t), Some(14));
        assert_eq!(notify_bucket(2, &t), Some(3));
        assert_eq!(notify_bucket(0, &t), Some(0));
        assert_eq!(notify_bucket(-9, &t), Some(0));
        assert_eq!(notify_bucket(10, &[]), None);
    }

    /// 各云厂商签名共用：请求头必须按小写名称排序，值去掉首尾空白
    #[test]
    fn canonical_headers_are_lowercased_and_sorted() {
        let h = vec![
            ("Host".to_string(), " a.com ".to_string()),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];
        let (canonical, signed) = sorted_canonical_headers(&h);
        assert_eq!(signed, "content-type;host");
        assert_eq!(canonical, "content-type:application/json\nhost:a.com\n");
    }
}
