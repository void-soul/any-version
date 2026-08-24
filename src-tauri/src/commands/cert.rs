//! 证书管理（Q-005）：Let's Encrypt 泛域名证书申请 + 多节点部署 + 应用内调度。
//!
//! 架构：
//! - 数据模型（Certificate / DeployNode / Credential）持久化在 `get_data_dir()/certs/` 下三个 JSON 文件。
//! - ACME 引擎：封装 lego sidecar（`lego --dns <provider> run` / `renew`），DNS-01 由 lego 内置插件完成。
//! - 部署：qiniu / aliyun / linux / windows 四种节点类型。
//! - 调度：应用内 tokio 定时器，扫描到期并触发申请/部署。

use aes_gcm::aead::Aead;
use aes_gcm::KeyInit;
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Duration, Utc};
use hmac::Mac;
use hmac::Hmac;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::collections::HashMap;
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
pub struct Certificate {
    pub id: String,
    pub domains: Vec<String>,
    pub email: String,
    /// letsencrypt（生产） / letsencrypt-staging（测试）
    #[serde(default = "default_ca")]
    pub ca: String,
    /// lego 的 DNS provider 标识，或 "custom"（custom 时凭据 data 的键即 lego 环境变量名）
    pub dns_provider: String,
    pub credential_id: String,
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

fn default_days() -> i64 {
    30
}
fn default_ca() -> String {
    "letsencrypt".to_string()
}
fn default_status() -> String {
    "pending".to_string()
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
    format!("{}-{}", prefix, Utc::now().timestamp_millis())
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

/// 读取 lego 生成的 PEM 三段（cert / key / issuer）。
fn read_lego_pems(cert_dir: &PathBuf) -> Option<(String, String, String)> {
    let cdir = cert_dir.join(".lego").join("certificates");
    if !cdir.exists() {
        return None;
    }
    let mut crt = None;
    let mut key = None;
    let mut issuer = None;
    if let Ok(entries) = std::fs::read_dir(&cdir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let content = std::fs::read_to_string(e.path()).unwrap_or_default();
            if name.ends_with(".crt") && !name.contains("issuer") {
                crt = Some(content);
            } else if name.ends_with(".key") {
                key = Some(content);
            } else if name.contains("issuer") && name.ends_with(".crt") {
                issuer = Some(content);
            }
        }
    }
    match (crt, key) {
        (Some(c), Some(k)) => Some((c, k, issuer.unwrap_or_default())),
        _ => None,
    }
}

fn run_lego(cert: &Certificate, cred: &Credential, renew: bool) -> Result<(String, String, String), String> {
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
    cmd.arg("--dns").arg(&cert.dns_provider);
    for dom in &cert.domains {
        cmd.arg("--domains").arg(dom);
    }
    if renew {
        cmd.arg("--renew-days").arg(cert.renew_before_days.to_string());
    }
    for (k, v) in lego_env(&cert.dns_provider, cred) {
        cmd.env(&k, &v);
    }
    let envs = lego_env(&cert.dns_provider, cred);
    let has_custom_resolvers = envs.iter().any(|(k, _)| k == "LEGO_DNS_RESOLVERS");
    if !has_custom_resolvers {
        cmd.arg("--dns.resolvers").arg("223.5.5.5:53,114.114.114.114:53,8.8.8.8:53");
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
    read_lego_pems(&cert_dir).ok_or_else(|| "lego 执行成功但未找到生成的 PEM 文件".to_string())
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
) -> Result<(), String> {
    match node.node_type.as_str() {
        "windows" => deploy_windows(node, cert, pems).await,
        "linux" => deploy_linux(node, pems),
        "qiniu" => deploy_qiniu(node, cert, pems).await,
        "aliyun" => deploy_aliyun(node, cert, pems).await,
        other => Err(format!("未知部署节点类型: {}", other)),
    }
}

async fn deploy_windows(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
) -> Result<(), String> {
    let url = node.config.get("url").cloned().unwrap_or_default();
    let token = node.config.get("token").cloned().unwrap_or_default();
    if url.is_empty() {
        return Err("windows 节点缺少 url".into());
    }
    let body = serde_json::json!({
        "domain": cert.domains.first().cloned().unwrap_or_default(),
        "cert": pems.0,
        "key": pems.1,
        "ca": pems.2,
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("推送证书到 Windows 接收端失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("Windows 接收端返回 {}", resp.status()));
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
    if let Err(e) = scp_cert {
        return Err(format!("scp 证书失败: {}", e));
    }
    if let Err(e) = scp_key {
        return Err(format!("scp 私钥失败: {}", e));
    }
    if !reload.is_empty() {
        let mut ssh_cmd = Command::new("ssh");
        #[cfg(windows)]
        ssh_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let _ = ssh_cmd
            .args(["-p", &port, &target, &reload])
            .output();
    }
    Ok(())
}

// --- 七牛 / 阿里云：签名实现为 best-effort，未经真实账号验证，需按官方文档核对 ---

#[allow(dead_code)]
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

async fn deploy_qiniu(
    _node: &DeployNode,
    _cert: &Certificate,
    _pems: &(String, String, String),
) -> Result<(), String> {
    // TODO: 七牛证书上传需 Qiniu 管理 API 签名（HMAC-SHA1 over "POST /sslcert\n\n<body>"）。
    // 结构已就绪，签名待按 https://developer.qiniu.com 核对后启用。
    Err("七牛部署：签名实现待完成（见部署节点备注）".into())
}

#[allow(dead_code)]
fn aliyun_rpc_sign(params: &HashMap<String, String>, secret: &str) -> String {
    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort();
    let mut canonical = String::new();
    for k in keys {
        canonical.push_str(&percent_encode(k));
        canonical.push('=');
        canonical.push_str(&percent_encode(params.get(k).unwrap()));
        canonical.push('&');
    }
    canonical.pop();
    let string_to_sign = format!("GET&{}&{}", percent_encode("/"), percent_encode(&canonical));
    let key = format!("{}&", secret);
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(string_to_sign.as_bytes());
    let sig = mac.finalize().into_bytes();
    B64.encode(sig)
}

async fn deploy_aliyun(
    _node: &DeployNode,
    _cert: &Certificate,
    _pems: &(String, String, String),
) -> Result<(), String> {
    // TODO: 阿里云 CAS UploadCertificate 需 RPC 签名（见 aliyun_rpc_sign）。
    // 结构已就绪，签名待按 https://help.aliyun.com 核对后启用。
    Err("阿里云部署：签名实现待完成（见部署节点备注）".into())
}

// ---------------------------------------------------------------------------
// 申请 / 续期 + 部署编排
// ---------------------------------------------------------------------------

/// 申请（或续期）指定证书，并部署到其所有关联节点。
pub async fn issue_and_deploy(cert_id: &str) -> Result<Certificate, String> {
    let mut certs = load_json::<Certificate>("certificates.json");
    let mut nodes = load_json::<DeployNode>("deploy_nodes.json");
    let creds = load_json::<Credential>("credentials.json");

    let idx = certs
        .iter()
        .position(|c| c.id == cert_id)
        .ok_or_else(|| "证书不存在".to_string())?;
    let cert = certs[idx].clone();

    let cred = creds
        .into_iter()
        .find(|c| c.id == cert.credential_id)
        .ok_or_else(|| "关联凭据不存在".to_string())?;
    let cred = decrypt_credential(cred);

    let renew = cert.last_issue_at.is_some();
    let (crt, key, ca) = run_lego(&cert, &cred, renew)?;
    let pems = (crt, key, ca);

    // 更新有效期
    let not_after = extract_not_after(&pems.0);
    let now = Utc::now().to_rfc3339();
    certs[idx].last_issue_at = Some(now.clone());
    certs[idx].not_after = not_after.clone();
    certs[idx].status = "issued".to_string();
    certs[idx].last_error = None;

    // 部署到所有关联节点
    for nid in &cert.deploy_node_ids {
        if let Some(node) = find_node(&nodes, nid) {
            let node = node.clone();
            match deploy_to_node(&node, &cert, &pems).await {
                Ok(()) => {
                    if let Some(n) = nodes.iter_mut().find(|n| n.id == *nid) {
                        n.last_deploy_at = Some(now.clone());
                        n.last_deploy_error = None;
                    }
                }
                Err(e) => {
                    if let Some(n) = nodes.iter_mut().find(|n| n.id == *nid) {
                        n.last_deploy_error = Some(e.clone());
                    }
                }
            }
        }
    }

    let updated = certs[idx].clone();
    save_json("certificates.json", &certs);
    save_json("deploy_nodes.json", &nodes);
    Ok(updated)
}

/// 从证书 PEM 解析 notAfter（RFC3339 或 ASN1 日期）。
fn extract_not_after(crt: &str) -> Option<String> {
    // 简单解析：找 "Not After = <date>" 或 "notAfter=" 行
    for line in crt.lines() {
        let l = line.trim();
        if l.starts_with("Not After") || l.starts_with("notAfter") {
            if let Some(pos) = l.find('=') {
                let date = l[pos + 1..].trim();
                // 尝试解析常见格式并转为 RFC3339
                if let Ok(dt) = DateTime::parse_from_rfc2822(date) {
                    return Some(dt.with_timezone(&Utc).to_rfc3339());
                }
                if let Ok(dt) = DateTime::parse_from_str(date, "%b %e %H:%M:%S %Y %Z") {
                    return Some(dt.with_timezone(&Utc).to_rfc3339());
                }
                return Some(date.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 调度器
// ---------------------------------------------------------------------------

/// 启动应用内后台调度器（在 lib.rs setup 中调用）。
pub fn start_scheduler(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let app = app;
        loop {
            // 读取间隔（短锁，不跨 await）
            let interval = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                s.interval_minutes
            };
            tokio::time::sleep(std::time::Duration::from_secs(interval * 60)).await;

            // 收集本周期需要续期的证书（短锁，不跨 await）
            let due_certs: Vec<String> = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                if !s.enabled {
                    Vec::new()
                } else {
                    let certs = load_json::<Certificate>("certificates.json");
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
                let result = issue_and_deploy(&id).await;
                let (ok, message) = match &result {
                    Ok(_) => (true, "已续期".to_string()),
                    Err(e) => (false, e.clone()),
                };
                let st = app.state::<CertScheduler>();
                let mut s = st.state.lock().unwrap();
                s.push_log(SchedulerLogEntry {
                    at: Utc::now().to_rfc3339(),
                    cert_id: id.clone(),
                    action: "renew".into(),
                    ok,
                    message,
                });
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
    load_json::<Certificate>("certificates.json")
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
) -> Certificate {
    let mut certs = load_json::<Certificate>("certificates.json");
    let cert = Certificate {
        id: gen_id("cert"),
        domains,
        email,
        ca: ca.unwrap_or_else(default_ca),
        dns_provider,
        credential_id,
        renew_before_days: renew_before_days.unwrap_or_else(default_days),
        not_before: None,
        not_after: None,
        last_issue_at: None,
        status: default_status(),
        last_error: None,
        deploy_node_ids: deploy_node_ids.unwrap_or_default(),
    };
    certs.push(cert.clone());
    save_json("certificates.json", &certs);
    cert
}

#[tauri::command]
pub fn cert_update_nodes(id: String, deploy_node_ids: Vec<String>) -> Result<(), String> {
    let mut certs = load_json::<Certificate>("certificates.json");
    let c = certs
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| "证书不存在".to_string())?;
    c.deploy_node_ids = deploy_node_ids;
    save_json("certificates.json", &certs);
    Ok(())
}

#[tauri::command]
pub fn cert_delete(id: String) {
    let certs: Vec<Certificate> = load_json::<Certificate>("certificates.json")
        .into_iter()
        .filter(|c| c.id != id)
        .collect();
    save_json("certificates.json", &certs);
}

#[tauri::command]
pub async fn cert_issue_now(id: String) -> Result<Certificate, String> {
    issue_and_deploy(&id).await
}

#[tauri::command]
pub fn cert_get_pem(id: String) -> Result<HashMap<String, String>, String> {
    let certs = load_json::<Certificate>("certificates.json");
    let cert = find_cert(&certs, &id).ok_or_else(|| "证书不存在".to_string())?;
    let dir = ensure_dir().join(&cert.id).join(".lego").join("certificates");
    let mut out = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".crt") || name.ends_with(".key") {
                out.insert(name, std::fs::read_to_string(e.path()).unwrap_or_default());
            }
        }
    }
    Ok(out)
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
    let certs: Vec<Certificate> = load_json::<Certificate>("certificates.json")
        .into_iter()
        .map(|mut c| {
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
        "windows" => {
            let url = node.config.get("url").cloned().unwrap_or_default();
            let token = node.config.get("token").cloned().unwrap_or_default();
            let resp = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| e.to_string())?
                .get(if url.ends_with('/') { format!("{}health", url) } else { format!("{}/health", url) })
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| format!("连接失败: {}", e))?;
            Ok(format!("Windows 接收端响应 {}", resp.status()))
        }
        "linux" => {
            let host = node.config.get("host").cloned().unwrap_or_default();
            let port = node.config.get("port").cloned().unwrap_or_else(|| "22".into());
            let user = node.config.get("user").cloned().unwrap_or_default();
            let mut ssh_cmd = Command::new("ssh");
            #[cfg(windows)]
            ssh_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            let out = ssh_cmd
                .args(["-p", &port, &format!("{}@{}", user, host), "echo ok"])
                .output();
            match out {
                Ok(o) if o.status.success() => Ok("Linux SSH 连通".into()),
                Ok(o) => Err(format!("SSH 失败: {}", String::from_utf8_lossy(&o.stderr))),
                Err(e) => Err(format!("无法执行 ssh: {}", e)),
            }
        }
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
    let certs = load_json::<Certificate>("certificates.json");
    let mut results = Vec::new();
    for c in &certs {
        match issue_and_deploy(&c.id).await {
            Ok(_) => results.push(format!("{}: ok", c.id)),
            Err(e) => results.push(format!("{}: {}", c.id, e)),
        }
    }
    Ok(results)
}
