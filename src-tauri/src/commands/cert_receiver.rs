//! 内嵌证书接收端。
//!
//! 原先是一个独立的 Tauri 小工具（`cert-receiver/`，已标废弃），2026-10-30 集成进 Kira：
//! 证书页多一个「接收端」页签，Kira 进程内直接起 axum 服务。
//!
//! 对外契约**保持不变**（老部署无需改任何脚本）：
//! - `GET  /health`
//! - `POST /push`（`Authorization: Bearer <token>`）
//! - `GET  /api/open/getCertificate`（md5 签名，见 `get_certificate`）
//!
//! 落盘布局 `cert|domain|both|flat`、文件名 `cert.key/fullchain.cer/cert.cer/chain.cer/meta.json`、
//! 重载方式 `none|command|nginx|service` 也全部沿用。
//!
//! 配置从独立的 `cert-receiver.toml` 并入 `config.json` 的 `certReceiver` 段；
//! 首次启动若发现旧 toml 会自动迁移（只读不删，方便回退）。

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

/// 日志事件：逐行推送（前端 `listen("cert-receiver-log")`）
pub const LOG_EVENT: &str = "cert-receiver-log";
/// 状态事件：启动/停止/收到推送时推送完整状态
pub const STATUS_EVENT: &str = "cert-receiver-status";

/// 内存里保留的日志行数（界面首次打开时一次性取走）
const LOG_CAP: usize = 500;

/* ─── 配置 ─── */

/// 接收端配置。字段名与旧 `cert-receiver.toml` 完全一致，迁移时无需改名。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CertReceiverConfig {
    /// 监听地址，形如 `127.0.0.1:9000` / `0.0.0.0:9000`
    pub listen_addr: String,
    pub token: String,
    pub output_dir: String,
    /// cert（按证书 ID）| domain（按域名，通配符丢 `*`）| both | flat
    #[serde(default = "default_layout")]
    pub layout: String,
    /// 拉取接口（/api/open/getCertificate）的签名密钥；为空则关闭该接口
    #[serde(default)]
    pub api_key: String,
    /// none | command | nginx | service
    #[serde(default = "default_reload_mode")]
    pub reload_mode: String,
    #[serde(default)]
    pub reload_command: String,
    #[serde(default)]
    pub nginx_prefix: String,
    #[serde(default)]
    pub nginx_conf: String,
    #[serde(default)]
    pub service_name: String,
    /// 兼容旧配置：收到证书后执行的脚本（优先于 reload_mode）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_script: Option<String>,
    /// 启动 Kira 时自动拉起接收端（`cert` 包默认 true）
    #[serde(default)]
    pub auto_start: bool,
}

fn default_layout() -> String {
    "cert".to_string()
}

fn default_reload_mode() -> String {
    "none".to_string()
}

impl Default for CertReceiverConfig {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:9000".to_string(),
            token: random_hex(16),
            output_dir: "C:\\certs".to_string(),
            layout: default_layout(),
            api_key: random_hex(16),
            reload_mode: default_reload_mode(),
            reload_command: String::new(),
            nginx_prefix: String::new(),
            nginx_conf: String::new(),
            service_name: String::new(),
            post_script: None,
            auto_start: true,
        }
    }
}

impl CertReceiverConfig {
    /// 保存前校验：错误文案直接显示在页面上，所以写成用户能照着改的句子。
    pub fn validate(&self) -> Result<(), String> {
        if self.listen_addr.parse::<SocketAddr>().is_err() {
            return Err(format!(
                "监听地址不合法：{}（应形如 127.0.0.1:9000）",
                self.listen_addr
            ));
        }
        cert_deployment::credentials::validate_token(&self.token)?;
        if self.output_dir.trim().is_empty() {
            return Err("证书输出目录不能为空".into());
        }
        match self.layout.as_str() {
            "cert" | "domain" | "both" | "flat" => {}
            other => return Err(format!("未知落盘布局：{}", other)),
        }
        match self.reload_mode.as_str() {
            "none" | "" | "command" | "nginx" | "service" => {}
            other => return Err(format!("未知重载方式：{}", other)),
        }
        if self.reload_mode == "command" && self.reload_command.trim().is_empty() {
            return Err("重载方式为 command 时必须填写命令".into());
        }
        if self.reload_mode == "service" && self.service_name.trim().is_empty() {
            return Err("重载方式为 service 时必须填写服务名".into());
        }
        Ok(())
    }
}

fn random_hex(n: usize) -> String {
    let mut buf = vec![0u8; n];
    if getrandom::getrandom(&mut buf).is_err() {
        // 极端情况退回时间戳混合，保证不会写出空 token
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (((nanos >> (i % 16)) & 0xff) as u8) ^ (i as u8).wrapping_mul(31);
        }
    }
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 旧配置文件名（迁移用；集成后不再写它）
const LEGACY_CONFIG_FILE: &str = "cert-receiver.toml";

/// 从旧 `cert-receiver.toml` 迁移。找不到或解析失败返回 `None`。
///
/// 用 `toml_edit` 手工取字段而不是引入 `toml` crate：迁移是一次性动作，
/// 不值得为它多一棵依赖树。
pub fn migrate_legacy_config() -> Option<CertReceiverConfig> {
    let path = find_legacy_config()?;
    let text = std::fs::read_to_string(&path).ok()?;
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let get = |key: &str| doc.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());
    let mut cfg = CertReceiverConfig::default();
    if let Some(v) = get("listen_addr") {
        cfg.listen_addr = v;
    }
    if let Some(v) = get("token") {
        cfg.token = v;
    }
    if let Some(v) = get("output_dir") {
        cfg.output_dir = v;
    }
    if let Some(v) = get("layout") {
        cfg.layout = v;
    }
    if let Some(v) = get("api_key") {
        cfg.api_key = v;
    }
    if let Some(v) = get("reload_mode") {
        cfg.reload_mode = v;
    }
    if let Some(v) = get("reload_command") {
        cfg.reload_command = v;
    }
    if let Some(v) = get("nginx_prefix") {
        cfg.nginx_prefix = v;
    }
    if let Some(v) = get("nginx_conf") {
        cfg.nginx_conf = v;
    }
    if let Some(v) = get("service_name") {
        cfg.service_name = v;
    }
    if let Some(v) = get("post_script") {
        cfg.post_script = Some(v);
    }
    Some(cfg)
}

/// 旧配置的查找顺序（与独立版一致）：`--config` → exe 同目录 → 当前目录 → `%ProgramData%\Kira` → `C:\certs`。
fn find_legacy_config() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--config") {
        if let Some(p) = args.get(i + 1) {
            let p = PathBuf::from(p);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(LEGACY_CONFIG_FILE));
        }
    }
    candidates.push(PathBuf::from(LEGACY_CONFIG_FILE));
    if let Ok(program_data) = std::env::var("ProgramData") {
        candidates.push(
            PathBuf::from(program_data)
                .join("Kira")
                .join(LEGACY_CONFIG_FILE),
        );
    }
    candidates.push(PathBuf::from("C:\\certs").join(LEGACY_CONFIG_FILE));
    candidates.into_iter().find(|p| p.is_file())
}

/* ─── 运行时状态 ─── */

#[derive(Default, Clone)]
struct Stats {
    /// 成功写入次数（被 token 挡掉的不算「收到」）
    pushes: u64,
    /// 失败/被拒次数
    fails: u64,
    last_push_at: String,
    last_push_ok: bool,
    last_error: String,
}

struct ServerHandle {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    completion: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
    finished: Arc<AtomicBool>,
    addr: SocketAddr,
}

#[derive(Clone, Serialize)]
pub struct Status {
    pub running: bool,
    pub addr: String,
    pub pushes: u64,
    pub fails: u64,
    pub last_push_at: String,
    pub last_push_ok: bool,
    pub last_error: String,
}

/// 接收端全局状态。用 `OnceLock` 而不是 Tauri `State`：
/// axum handler 与命令层都要访问，全局单例最省事（一个进程只有一个接收端）。
pub struct CertReceiverState {
    lifecycle: Mutex<()>,
    cfg: Mutex<CertReceiverConfig>,
    server: Mutex<Option<ServerHandle>>,
    stats: Mutex<Stats>,
    logs: Mutex<VecDeque<String>>,
    /// 每一代服务的编号：重启时旧任务收尾晚于新任务启动，靠它避免打错日志
    run_seq: AtomicU64,
    app: Mutex<Option<AppHandle>>,
    /// 是否刚从旧 toml 迁移过来（页面提示用）
    migrated: Mutex<bool>,
}

static STATE: OnceLock<Arc<CertReceiverState>> = OnceLock::new();

/// 取全局状态；首次调用时从 `config.json` 载入（含旧 toml 迁移）。
pub fn state() -> Arc<CertReceiverState> {
    STATE
        .get_or_init(|| {
            let (cfg, migrated) = match crate::commands::config::load_config().cert_receiver {
                Some(c) => (c, false),
                None => match migrate_legacy_config() {
                    Some(c) => (c, true),
                    None => (CertReceiverConfig::default(), false),
                },
            };
            Arc::new(CertReceiverState {
                lifecycle: Mutex::new(()),
                cfg: Mutex::new(cfg),
                server: Mutex::new(None),
                stats: Mutex::new(Stats::default()),
                logs: Mutex::new(VecDeque::new()),
                run_seq: AtomicU64::new(0),
                app: Mutex::new(None),
                migrated: Mutex::new(migrated),
            })
        })
        .clone()
}

/// 由 `lib.rs` 的 setup 调用：注入 AppHandle（事件推送需要）+ 把迁移结果落盘。
pub fn init(app: AppHandle) {
    let st = state();
    *st.app.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);

    let migrated = *st.migrated.lock().unwrap_or_else(|e| e.into_inner());
    if migrated {
        let cfg = st.cfg();
        match persist(&cfg) {
            Ok(()) => st.log("已从 cert-receiver.toml 迁移配置（原文件保留，可回退）"),
            Err(e) => st.log(format!("迁移配置写回 config.json 失败：{}", e)),
        }
    }
}

/// 写回 `config.json` 的 `certReceiver` 段。
fn persist(cfg: &CertReceiverConfig) -> Result<(), String> {
    let mut c = crate::commands::config::load_config();
    c.cert_receiver = Some(cfg.clone());
    crate::commands::config::save_config(&c)
}

mod lifecycle;

/* ─── HTTP ─── */

struct AppData {
    cfg: CertReceiverConfig,
    state: Arc<CertReceiverState>,
}

fn deployment_slot() -> &'static Arc<tokio::sync::Semaphore> {
    static SLOT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
}

fn router(data: Arc<AppData>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/push", post(push))
        .route("/api/open/getCertificate", get(get_certificate))
        .with_state(data)
}

use cert_deployment::protocol::CertificatePush as PushBody;

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
use cert_deployment::protocol::pem_first_block;
/// 路径安全名：仅保留字母数字、点、连字符与下划线。
///
/// 通配符 `*` **直接丢掉**（不是换成 `_`）：`*.seemove.cn` → `seemove.cn`，
/// 与 nginx 里 `ssl_certificate ./conf/cert/seemove.cn/fullchain.cer` 这类写法对齐。
pub use cert_deployment::protocol::safe_name;

fn require_token(
    headers: &HeaderMap,
    cfg: &CertReceiverConfig,
) -> Result<(), (StatusCode, String)> {
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let expected = cert_deployment::credentials::bearer_authorization(&cfg.token);
    // 长度先比，再做字节级比较，避免明显的时间侧信道
    if auth.len() != expected.len() || auth.bytes().zip(expected.bytes()).any(|(a, b)| a != b) {
        return Err((StatusCode::UNAUTHORIZED, "invalid token".into()));
    }
    Ok(())
}

async fn health(State(data): State<Arc<AppData>>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "name": "cert-receiver",
        "version": env!("CARGO_PKG_VERSION"),
        "layout": data.cfg.layout,
        "reload_mode": data.cfg.reload_mode,
    }))
}

async fn push(
    State(data): State<Arc<AppData>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PushBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 先验 token 再落盘：顺序反了就等于允许未鉴权的写入
    if let Err(e) = require_token(&headers, &data.cfg) {
        data.state.record_push(
            false,
            format!("{} 推送被拒绝：共享 Token 不匹配", peer.ip()),
        );
        return Err(e);
    }

    let permit = deployment_slot()
        .clone()
        .try_acquire_owned()
        .map_err(|_| (StatusCode::CONFLICT, "已有证书正在部署，请稍后重试".into()))?;
    let certificate_id = body.certificate_id.clone();
    let push_data = data.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        push_inner(&push_data, &body)
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("部署任务异常: {e}"),
        )
    })?;
    match &result {
        Ok((written, _)) => data.state.record_push(
            true,
            format!(
                "{} 推送成功（{}），写入 {} 处",
                peer.ip(),
                certificate_id,
                written.len()
            ),
        ),
        Err((code, msg)) => data.state.record_push(
            false,
            format!("{} 推送失败 [{}]: {}", peer.ip(), code.as_u16(), msg),
        ),
    }
    let (written, reload) = result?;
    Ok(Json(json!({
        "ok": true,
        "certificateId": certificate_id,
        "written": written,
        "reload": reload,
    })))
}

fn push_inner(
    data: &AppData,
    body: &PushBody,
) -> Result<(Vec<String>, String), (StatusCode, String)> {
    let cfg = &data.cfg;
    let options = cert_deployment::protocol::ReceiverOptions {
        output_dir: Path::new(&cfg.output_dir),
        layout: &cfg.layout,
        reload: cert_deployment::protocol::ReloadOptions {
            mode: &cfg.reload_mode,
            command: &cfg.reload_command,
            nginx_prefix: &cfg.nginx_prefix,
            nginx_conf: &cfg.nginx_conf,
            service_name: &cfg.service_name,
            post_script: cfg.post_script.as_deref(),
        },
    };
    cert_deployment::protocol::deploy(&options, body, |line| data.state.log(line)).map_err(|e| {
        (
            if e.invalid_input {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            e.message,
        )
    })
}
#[derive(Debug, Deserialize)]
struct ApiQuery {
    #[serde(rename = "apiId")]
    api_id: String,
    #[serde(rename = "certificateId")]
    certificate_id: String,
    timestamp: String,
    sign: String,
    #[serde(rename = "apiKey", default)]
    api_key: Option<String>,
}

fn lower_md5(s: &str) -> String {
    format!("{:x}", md5::compute(s.as_bytes()))
}

async fn get_certificate(
    State(data): State<Arc<AppData>>,
    Query(q): Query<ApiQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let cfg = &data.cfg;
    if cfg.api_key.trim().is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            "拉取接口未启用（配置 api_key 后可用）".into(),
        ));
    }
    let ts: u128 = q
        .timestamp
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "timestamp 无效".into()))?;
    let now = now_ms();
    let diff = if ts > now { ts - now } else { now - ts };
    if diff > 5 * 60 * 1000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "timestamp 已过期（5 分钟有效）".into(),
        ));
    }

    // 与 ohttps 一致：apiId/apiKey/certificateId/timestamp 按字母序拼接
    let mut params = BTreeMap::new();
    params.insert("apiId".to_string(), q.api_id.clone());
    params.insert("certificateId".to_string(), q.certificate_id.clone());
    params.insert("timestamp".to_string(), q.timestamp.clone());
    params.insert(
        "apiKey".to_string(),
        cert_deployment::credentials::api_key_for_sign(&cfg.api_key).to_string(),
    );
    let string_for_sign = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");
    let expect = lower_md5(&string_for_sign);
    if q.sign.to_lowercase() != expect {
        data.state
            .log(format!("拉取 {} 失败：签名校验不通过", q.certificate_id));
        return Err((StatusCode::UNAUTHORIZED, "签名校验失败".into()));
    }
    let _ = q.api_key; // 请求里可以不带 apiKey，只参与签名

    let _permit = deployment_slot()
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "证书服务正在停止".into()))?;

    let root = PathBuf::from(&cfg.output_dir);
    let id = q.certificate_id.clone();
    let certificate =
        tokio::task::spawn_blocking(move || cert_deployment::reading::read_certificate(&root, &id))
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取证书任务异常: {e}"),
                )
            })?
            .map_err(|e| {
                use cert_deployment::reading::ErrorKind;
                (
                    match e.kind {
                        ErrorKind::NotFound => StatusCode::NOT_FOUND,
                        ErrorKind::Busy => StatusCode::CONFLICT,
                        ErrorKind::Invalid => StatusCode::BAD_REQUEST,
                        ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
                    },
                    e.message,
                )
            })?;
    data.state.log(format!("拉取 {} 成功", q.certificate_id));

    Ok(Json(json!({
        "success": true,
        "msg": "成功",
        "payload": {
            "certKey": certificate.key,
            "fullChainCerts": certificate.full_chain,
            "expiredTime": certificate.expired_time,
        }
    })))
}

/* ─── 前端命令 ─── */

#[derive(Serialize)]
pub struct Bootstrap {
    config: CertReceiverConfig,
    status: Status,
    logs: Vec<String>,
    /// 是否刚从旧 toml 迁移
    migrated: bool,
}

/// 页面首次加载：一次拿全配置 / 状态 / 历史日志 / 迁移标记。
#[tauri::command]
pub fn cert_receiver_bootstrap() -> Bootstrap {
    let st = state();
    let migrated = *st.migrated.lock().unwrap_or_else(|e| e.into_inner());
    Bootstrap {
        config: st.cfg(),
        status: st.status(),
        logs: st.logs_snapshot(),
        migrated,
    }
}

#[tauri::command]
pub fn cert_receiver_status() -> Status {
    state().status()
}

#[tauri::command]
pub async fn cert_receiver_start() -> Result<Status, String> {
    crate::runtime_tasks::require_enabled("cert")?;
    tauri::async_runtime::spawn_blocking(|| {
        let st = state();
        st.start_server()?;
        Ok(st.status())
    })
    .await
    .map_err(|e| format!("接收端启动任务异常: {e}"))?
}

#[tauri::command]
pub async fn cert_receiver_stop() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let st = state();
        st.stop_server()?;
        Ok(st.status())
    })
    .await
    .map_err(|e| format!("接收端停止任务异常: {e}"))?
}

#[tauri::command]
pub async fn cert_receiver_save_config(config: CertReceiverConfig) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = state();
        st.apply_config(config)?;
        Ok(st.status())
    })
    .await
    .map_err(|e| format!("接收端配置任务异常: {e}"))?
}

#[tauri::command]
pub fn cert_receiver_logs() -> Vec<String> {
    state().logs_snapshot()
}

#[tauri::command]
pub fn cert_receiver_clear_logs() -> Vec<String> {
    let st = state();
    st.logs.lock().unwrap_or_else(|e| e.into_inner()).clear();
    Vec::new()
}

/// 已收到的证书（读输出目录里的 meta.json）。
#[derive(Serialize)]
pub struct StoredCert {
    /// 落盘目录名（可能是证书 ID，也可能是域名，取决于 layout）
    pub dir: String,
    pub certificate_id: String,
    pub domains: Vec<String>,
    pub expired_time: String,
    pub updated_at_ms: u64,
}

#[tauri::command]
pub fn cert_receiver_stored_certs() -> Vec<StoredCert> {
    let root = PathBuf::from(state().cfg().output_dir);
    let mut out: Vec<StoredCert> = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let text = match std::fs::read_to_string(p.join("meta.json")) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let certificate_id = v
            .get("certificate_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        if certificate_id.is_empty() {
            continue;
        }
        out.push(StoredCert {
            dir: p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            certificate_id,
            domains: v
                .get("domains")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            expired_time: v
                .get("expired_time")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            updated_at_ms: v.get("updated_at_ms").and_then(|x| x.as_u64()).unwrap_or(0),
        });
    }
    out.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
    out
}

#[derive(Serialize)]
pub struct PullExample {
    pub url: String,
    pub curl: String,
    /// 签名里用的时间戳（5 分钟内有效，过期要重新生成）
    pub timestamp: String,
}

/// 只对 URL 里必须转义的字符做最小处理：这里的 id 一般是字母数字和 -_.
fn url_encode(s: &str) -> String {
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

/// 拉取接口的签名：md5(按字母序拼接 apiId/apiKey/certificateId/timestamp)。
///
/// **必须**与 `get_certificate` 的校验保持同一算法 —— 两边一起改，
/// 否则示例看起来正常、实际一律 401。单测用同一组固定向量交叉验证。
fn pull_sign(api_id: &str, api_key: &str, certificate_id: &str, timestamp: &str) -> String {
    let mut params = BTreeMap::new();
    params.insert("apiId", api_id);
    params.insert(
        "apiKey",
        cert_deployment::credentials::api_key_for_sign(api_key),
    );
    params.insert("certificateId", certificate_id);
    params.insert("timestamp", timestamp);
    let joined = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");
    lower_md5(&joined)
}

/// 生成一条可直接交给容器/脚本的拉取命令。
#[tauri::command]
pub fn cert_receiver_pull_example(
    api_id: String,
    certificate_id: String,
) -> Result<PullExample, String> {
    let cfg = state().cfg();
    if cfg.api_key.trim().is_empty() {
        return Err("接收端还没配 API 密钥：先在上面生成/填写并保存，拉取接口才会启用".into());
    }
    if certificate_id.trim().is_empty() {
        return Err("先选一个已收到的证书（列表为空说明还没推送过）".into());
    }
    let api_id = if api_id.trim().is_empty() {
        "kira-pull".to_string()
    } else {
        api_id.trim().to_string()
    };
    let cert_id = certificate_id.trim().to_string();
    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let sign = pull_sign(&api_id, cfg.api_key.trim(), &cert_id, &ts);

    // 监听在 0.0.0.0 时示例里给 127.0.0.1（0.0.0.0 不是可连接的目标地址）
    let host = if cfg.listen_addr.starts_with("0.0.0.0") {
        cfg.listen_addr.replacen("0.0.0.0", "127.0.0.1", 1)
    } else {
        cfg.listen_addr.clone()
    };
    let url = format!(
        "http://{}/api/open/getCertificate?apiId={}&certificateId={}&timestamp={}&sign={}",
        host,
        url_encode(&api_id),
        url_encode(&cert_id),
        ts,
        sign
    );
    Ok(PullExample {
        curl: format!("curl \"{}\"", url),
        url,
        timestamp: ts,
    })
}

/// 启动时按配置自启（`lib.rs` setup 调用）。失败只写日志，不阻塞启动。
pub fn start_on_boot() {
    let st = state();
    if !st.cfg().auto_start {
        return;
    }
    if let Err(e) = st.start_server() {
        st.log(format!("自动启动失败：{}", e));
    }
}

#[cfg(test)]
mod tests;
