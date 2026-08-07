// mihomo config model & persistence
// 对齐 clash-party: 两层配置 (AppConfig + ControledMihomoConfig) + Profiles + Overrides
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn default_true() -> bool {
    true
}
fn default_false() -> bool {
    false
}

fn default_tun_name() -> String {
    "AnyVersion".to_string()
}
#[allow(dead_code)]
fn default_mixed_port() -> u16 {
    7890
}
#[allow(dead_code)]
fn default_controller_port() -> u16 {
    9090
}
#[allow(dead_code)]
fn default_proxy_cols() -> u8 {
    6
}
fn default_profile_type() -> String {
    "subscription".into()
}
fn default_override_ext() -> String {
    "yaml".into()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub control_dns: bool,
    pub control_sniff: bool,
    pub use_nameserver_policy: bool,
    pub diff_work_dir: bool,
    pub sys_proxy_enable: bool,
    pub auto_start_core: bool,
    pub auto_set_proxy: bool,
    pub tun_enabled: bool,
    /// TUN 虚拟网卡（wintun 适配器）名称，可在 UI「TUN 网卡名称」自定义；
    /// 适配器名在设备创建时确定，故每次构建运行时配置都强制写入 tun.device
    #[serde(default = "default_tun_name")]
    pub tun_name: String,
    pub mixed_port: u16,
    pub controller_port: u16,
    pub secret: String,
    #[serde(default)]
    pub core_path: Option<String>,
    /// 一级代理（代理页选中的节点/组名，作为二级代理的 dialer-proxy）
    #[serde(default)]
    pub default_proxy: Option<String>,
    /// 二级代理列表（家庭 socks5），可多个，各自可增删
    #[serde(default)]
    pub secondary_proxies: Vec<SecondaryProxy>,
    /// 当前启用的二级代理 id（None=未启用）
    #[serde(default)]
    pub secondary_active_id: Option<String>,
    pub proxy_cols: u8,
    pub proxy_sort_type: String, // "Default" | "Delay" | "Name"
    pub keep_profile_alive: bool,
    pub theme: String,
    pub lang: String,
    pub ipc_port: Option<u16>,
    pub sys_proxy_bypass: String,
    pub substore_enabled: bool,
    pub webdav_url: String,
    pub webdav_user: String,
    pub webdav_pass: String,
    pub webdav_auto_backup: bool,
    pub current_profile: String,
    pub auto_close_proxy: bool,
    /// 其余任意配置键（对齐 clash-party appConfig 的丰富字段：delayTestUrl、
    /// proxyDisplayMode、proxyDisplayOrder、autoCloseConnection、connectionDirection、
    /// logLevel 过滤偏好等），flatten 持久化，patch 深合并后不丢失
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            control_dns: true,
            control_sniff: true,
            use_nameserver_policy: false,
            diff_work_dir: false,
            sys_proxy_enable: false,
            auto_start_core: false,
            auto_set_proxy: false,
            tun_enabled: false,
            tun_name: default_tun_name(),
            mixed_port: 7890,
            controller_port: 9090,
            secret: String::new(),
            core_path: None,
            default_proxy: None,
            secondary_proxies: Vec::new(),
            secondary_active_id: None,
            proxy_cols: 6,
            proxy_sort_type: "Default".into(),
            keep_profile_alive: true,
            theme: "system".into(),
            lang: "zh".into(),
            ipc_port: None,
            sys_proxy_bypass: String::new(),
            substore_enabled: false,
            webdav_url: String::new(),
            webdav_user: String::new(),
            webdav_pass: String::new(),
            webdav_auto_backup: false,
            current_profile: "default".into(),
            auto_close_proxy: false,
            extra: serde_json::Map::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_false")]
    pub auto_update: bool,
    pub updated_at: Option<u64>,
    pub age_secret: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RuleProviderEntry {
    pub id: String,
    pub name: String,
    pub behavior: String,
    pub url: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_false")]
    pub auto_update: bool,
    pub updated_at: Option<u64>,
    pub age_secret: Option<String>,
}

fn default_interval() -> u64 {
    86400
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProfileItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "type", default = "default_profile_type")]
    pub type_: String, // "file" | "subscription"
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub subscriptions: Vec<Subscription>,
    #[serde(default)]
    pub rule_providers: Vec<RuleProviderEntry>,
    #[serde(default)]
    pub custom_rules: Vec<String>,
    #[serde(default = "default_true")]
    pub dns_enabled: bool,
    #[serde(default)]
    pub dns_nameservers: Vec<String>,
    // —— 以下字段对齐 clash-party 编辑信息（订阅元数据）——
    #[serde(default)]
    pub age_secret_key: Option<String>, // Age 解密密钥
    #[serde(default)]
    pub url: Option<String>, // 订阅地址
    #[serde(default)]
    pub auth_token: Option<String>, // 授权令牌
    #[serde(default)]
    pub user_agent: Option<String>, // User Agent
    #[serde(default = "default_false")]
    pub use_proxy: bool, // 使用代理更新
    #[serde(default = "default_false")]
    pub auto_update: bool, // 自动更新
    #[serde(default = "default_interval")]
    pub update_interval: u64, // 更新间隔（秒）
    #[serde(default = "default_update_timeout")]
    pub update_timeout: u64, // 更新超时时间（秒）
    #[serde(default)]
    pub override_ids: Vec<String>, // 覆写（override id 列表）
    #[serde(default)]
    pub subscription_userinfo: Option<SubscriptionUserInfo>, // 订阅流量信息（来自响应头）
    #[serde(default)]
    pub updated_at: Option<u64>,
}

/// 订阅流量信息（解析自响应头 subscription-userinfo: upload=..; download=..; total=..; expire=..）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionUserInfo {
    pub upload: u64,
    pub download: u64,
    pub total: u64,
    pub expire: u64,
}

/// 二级代理（家庭 socks5）节点配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecondaryProxy {
    pub id: String,
    /// 显示名，如「美国1号」
    pub name: String,
    /// IP 或域名
    pub host: String,
    /// 端口
    pub port: u16,
    /// 账号（可选）
    #[serde(default)]
    pub username: Option<String>,
    /// 密码（可选）
    #[serde(default)]
    pub password: Option<String>,
}

impl ProfileItem {
    pub fn is_file(&self) -> bool {
        self.type_ == "file"
    }
}

fn default_update_timeout() -> u64 {
    30
}

impl Default for ProfileItem {
    fn default() -> Self {
        ProfileItem {
            id: "default".into(),
            name: "默认".into(),
            type_: default_profile_type(),
            file_path: None,
            subscriptions: Vec::new(),
            rule_providers: Vec::new(),
            custom_rules: Vec::new(),
            dns_enabled: true,
            dns_nameservers: Vec::new(),
            age_secret_key: None,
            url: None,
            auth_token: None,
            user_agent: None,
            use_proxy: false,
            auto_update: false,
            update_interval: default_interval(),
            update_timeout: default_update_timeout(),
            override_ids: Vec::new(),
            subscription_userinfo: None,
            updated_at: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProfileConfig {
    #[serde(default = "default_profile_current")]
    pub current: String,
    #[serde(default)]
    pub items: Vec<ProfileItem>,
}

fn default_profile_current() -> String {
    "default".into()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OverrideItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "ext", default = "default_override_ext")]
    pub ext: String, // "js" | "yaml"
    #[serde(default = "default_false")]
    pub global: bool,
    /// 对齐 clash-party：local | remote
    #[serde(rename = "type", default = "default_override_type")]
    pub type_: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub updated: Option<u64>,
}

fn default_override_type() -> String {
    "local".into()
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct OverrideConfig {
    #[serde(default)]
    pub items: Vec<OverrideItem>,
}

// ---------- persistence helpers ----------
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let s = match fs::read_to_string(path) {
        Ok(s) => {
            // F2 修复：解析失败时保留 .corrupt.bak 副本，再回退 default
            if serde_json::from_str::<T>(&s).is_err() {
                let _ = fs::copy(path, path.with_extension("corrupt.bak"));
                eprintln!("[mihomo] 配置解析失败，已留存 .corrupt.bak: {:?}", path);
            }
            s
        }
        Err(_) => return None,
    };
    serde_json::from_str(&s).ok()
}
fn write_json<T: Serialize>(path: &Path, v: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(v).unwrap_or_default())
}

pub fn app_config_path(dir: &Path) -> PathBuf {
    dir.join("app.json")
}
pub fn controled_path(dir: &Path) -> PathBuf {
    dir.join("controled.json")
}
pub fn profile_config_path(dir: &Path) -> PathBuf {
    dir.join("profiles.json")
}
pub fn override_config_path(dir: &Path) -> PathBuf {
    dir.join("overrides.json")
}
pub fn profile_content_dir(dir: &Path) -> PathBuf {
    dir.join("profiles")
}
pub fn override_content_dir(dir: &Path) -> PathBuf {
    dir.join("override")
}
pub fn profile_content_path(dir: &Path, id: &str) -> PathBuf {
    profile_content_dir(dir).join(format!("{id}.yaml"))
}
pub fn override_content_path(dir: &Path, id: &str, ext: &str) -> PathBuf {
    override_content_dir(dir).join(format!("{id}.{ext}"))
}
/// 规则覆写目录（rules/<profileId>.yaml，对齐 clash-party rulePath）
pub fn rule_override_dir(dir: &Path) -> PathBuf {
    dir.join("rules")
}
pub fn rule_override_path(dir: &Path, id: &str) -> PathBuf {
    rule_override_dir(dir).join(format!("{id}.yaml"))
}
/// 独立工作目录（diff_work_dir 开启时每个订阅一个目录，对齐 mihomoProfileWorkDir）
pub fn profile_work_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join("profiles-work").join(id)
}
/// 内核实际读取的工作目录与配置文件路径
pub fn core_work_dir(dir: &Path, diff_work_dir: bool, id: &str) -> PathBuf {
    if diff_work_dir {
        profile_work_dir(dir, id)
    } else {
        dir.to_path_buf()
    }
}
pub fn core_work_config_path(dir: &Path, diff_work_dir: bool, id: &str) -> PathBuf {
    core_work_dir(dir, diff_work_dir, id).join("config.yaml")
}

/// 原子写文件：先写临时文件再重命名，避免内核读到半截配置（对齐 atomicWriteFile）
pub fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // F13 修复：tmp 名加 pid + 原子计数器防同毫秒碰撞
    static ATOMIC_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ctr = ATOMIC_CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp = path.with_extension(format!(
        "tmp{}_{}",
        std::process::id(),
        ctr
    ));
    std::fs::write(&tmp, content)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Windows 上目标被占用时回退为直接覆盖写
            let r = std::fs::write(path, content);
            let _ = std::fs::remove_file(&tmp);
            r
        }
    }
}

pub fn load_app_config(dir: &Path) -> AppConfig {
    read_json(&app_config_path(dir)).unwrap_or_default()
}
pub fn save_app_config(dir: &Path, c: &AppConfig) -> std::io::Result<()> {
    write_json(&app_config_path(dir), c)
}
pub fn load_controled(dir: &Path) -> Value {
    read_json(&controled_path(dir)).unwrap_or(Value::Object(Default::default()))
}
pub fn save_controled(dir: &Path, v: &Value) -> std::io::Result<()> {
    write_json(&controled_path(dir), v)
}
pub fn load_profile_config(dir: &Path) -> ProfileConfig {
    read_json(&profile_config_path(dir)).unwrap_or_default()
}
pub fn save_profile_config(dir: &Path, c: &ProfileConfig) -> std::io::Result<()> {
    write_json(&profile_config_path(dir), c)
}
pub fn load_override_config(dir: &Path) -> OverrideConfig {
    read_json(&override_config_path(dir)).unwrap_or_default()
}
pub fn save_override_config(dir: &Path, c: &OverrideConfig) -> std::io::Result<()> {
    write_json(&override_config_path(dir), c)
}

pub fn read_profile_content(dir: &Path, item: &ProfileItem) -> Option<String> {
    if let Some(p) = &item.file_path {
        return fs::read_to_string(p).ok();
    }
    fs::read_to_string(profile_content_path(dir, &item.id)).ok()
}
pub fn write_profile_content(dir: &Path, item: &ProfileItem, content: &str) -> std::io::Result<()> {
    if let Some(p) = &item.file_path {
        return fs::write(p, content);
    }
    let p = profile_content_path(dir, &item.id);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(p, content)
}
pub fn read_override_content(dir: &Path, item: &OverrideItem) -> Option<String> {
    fs::read_to_string(override_content_path(dir, &item.id, &item.ext)).ok()
}
pub fn write_override_content(
    dir: &Path,
    item: &OverrideItem,
    content: &str,
) -> std::io::Result<()> {
    let p = override_content_path(dir, &item.id, &item.ext);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(p, content)
}
