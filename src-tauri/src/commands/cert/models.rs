use super::*;

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
    /// local / ssh / linux / windows / api / docker / webhook / cloud providers
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
    pub(super) fn push_log(&mut self, e: SchedulerLogEntry) {
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

pub(super) fn default_true() -> bool {
    true
}
pub(super) fn default_https_port() -> u16 {
    443
}
pub(super) fn default_notify_days() -> Vec<i64> {
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

pub(super) fn default_days() -> i64 {
    30
}
pub(super) fn default_ca() -> String {
    "letsencrypt".to_string()
}
pub(super) fn default_status() -> String {
    "pending".to_string()
}
pub(super) fn default_validation() -> String {
    "dns".to_string()
}
pub(super) fn default_http_port() -> u16 {
    80
}
