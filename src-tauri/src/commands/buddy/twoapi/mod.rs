//! 2API：把 Buddy 的 WorkBuddy 登录态转成本地 OpenAI / Anthropic 兼容 API。
//!
//! 架构见 `docs/plans/2026-10-03-2api-rust-service-design.md`：
//! 协议转换复用 `crate::proxy`（axum + Anthropic/OpenAI/Responses 转换），
//! 本模块只负责「Buddy 凭据 → 上游鉴权头」与生命周期。
//!
//! **为什么内嵌而不是拉起 Python 服务**：原实现（workbuddy2api）取 at-rest 密钥时
//! 只查默认安装路径，本机 WorkBuddy 装在 `D:\sim-tool\WorkBuddy` → 密钥取不到 →
//! 所有对话请求 500，而 `/docs`、`/health`、`/v1/models` 全 200，极难定位。
//! 内嵌后密钥获取、账号切换、错误呈现都在我们自己的代码里，可测可控。

pub(crate) mod atrest;
pub(crate) mod credentials;
pub(crate) mod upstream;

use crate::proxy::types::{ProxyConfig, UpstreamHeader};

/// 启动前自检的一项结果
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub id: String,
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

/// 自检报告：三项全过才允许启动
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub ok: bool,
    pub checks: Vec<CheckResult>,
    /// 失败项的可读原因（可直接展示给用户）
    pub message: String,
}

/// 自检输入：把"真实采集"与"判定逻辑"分开，便于对判定逻辑写单测
pub struct PreflightInputs {
    /// at-rest 密钥能否拿到
    pub key: Result<String, String>,
    /// 当前登录态能否解析出账号
    pub account: Result<credentials::Account, String>,
    /// 后端模型目录能否拉到（用模型数表示）
    pub catalog: Result<usize, String>,
}

/// 纯判定：任一失败即整体失败，message 汇总所有失败原因
pub fn run_preflight(inputs: &PreflightInputs) -> PreflightReport {
    let mut checks = Vec::new();

    let key_detail = match &inputs.key {
        Ok(secret) => format!("已获取（{} 字符）", secret.len()),
        Err(e) => format!("获取失败：{e}"),
    };
    checks.push(CheckResult {
        id: "key".into(),
        label: "WorkBuddy 密钥".into(),
        ok: inputs.key.is_ok(),
        detail: key_detail,
    });

    let account_detail = match &inputs.account {
        Ok(a) => format!("{} · token {} 字符", a.display_name(), a.access_token.len()),
        Err(e) => format!("解析失败：{e}"),
    };
    checks.push(CheckResult {
        id: "account".into(),
        label: "登录态".into(),
        ok: inputs.account.is_ok(),
        detail: account_detail,
    });

    let catalog_detail = match &inputs.catalog {
        Ok(n) if *n > 0 => format!("后端可达 · {n} 个模型"),
        Ok(_) => "后端可达但模型列表为空".to_string(),
        Err(e) => format!("后端不可达：{e}"),
    };
    checks.push(CheckResult {
        id: "catalog".into(),
        label: "后端".into(),
        ok: matches!(&inputs.catalog, Ok(n) if *n > 0),
        detail: catalog_detail,
    });

    let failures: Vec<String> = checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| format!("{}：{}", c.label, c.detail))
        .collect();
    PreflightReport {
        ok: failures.is_empty(),
        message: if failures.is_empty() {
            String::new()
        } else {
            format!("启动前置检查未通过 —— {}", failures.join("；"))
        },
        checks,
    }
}

/// 组装指向 WorkBuddy 后端的代理配置。
///
/// 协议转换、入站路由、SSE 整形全部复用 `crate::proxy`，这里只换两样：
/// 上游地址与鉴权头。`upstream_api_key` 留空 —— 鉴权由 `upstream_headers` 里的
/// `Authorization` 承担，`proxy` 的 `has_authorization` 会因此跳过 provider key 注入。
pub fn build_proxy_config(port: u16, account: &credentials::Account) -> ProxyConfig {
    use crate::proxy::types::UpstreamHeader;
    let mut cfg = ProxyConfig::default();
    cfg.listen_address = "127.0.0.1".to_string();
    cfg.listen_port = port;
    // 两种入站都注册：Claude Code / Cline 走 anthropic，其余走 openai
    cfg.inbound_protocols = vec!["anthropic".to_string(), "openai".to_string()];
    cfg.outbound_protocol = "openai".to_string();
    cfg.upstream_base_url = format!("{}/v2", upstream::BACKEND_BASE);
    cfg.upstream_api_key = String::new();
    // WorkBuddy 后端的版本号是 /v2，不是 /v1。`resolve_url` 的 include_v1 自动模式
    // 判「结尾不是 /v1 就补 /v1」，会拼出 /v2/v1/chat/completions → 上游 404。
    // 所以显式关掉：让它按 base 直接拼 chat/completions。
    cfg.upstream_include_v1 = Some(false);
    // 模型目录不在 OpenAI 惯例的 /models 上：后端是 /v2/enterprises/personal/models。
    // 这是客户端 UI 用的同一份数据，只有它跟得上新模型（space-bunny 等）。
    cfg.models_path = "/enterprises/personal/models".to_string();
    // 后端目录里混着图像模型（hunyuan-image-alpha 等），不滤的话客户端会枚举到
    // 却调不通（上游对图像模型走 vclm，直接 400）。规则与 preflight 共用 is_chat_model。
    cfg.models_filter_non_chat = true;
    cfg.timeout_secs = 600;
    cfg.upstream_headers = workbuddy_headers(account);
    // WorkBuddy 后端只收流式：非流式请求会被拒（Non-stream chat request is currently
    // not supported）。所以对上游一律发流式，客户端要非流式时代理侧聚合 SSE 再回 JSON。
    cfg.force_upstream_stream = true;
    // 请求日志来源标识：2API 服务（Buddy/2API 面板按它过滤展示）
    cfg.source = "2api".to_string();
    cfg
}

/// WorkBuddy 后端要求的 5 个鉴权头
pub fn workbuddy_headers(account: &credentials::Account) -> Vec<UpstreamHeader> {
    vec![
        UpstreamHeader {
            key: "Authorization".to_string(),
            value: format!("Bearer {}", account.access_token),
        },
        UpstreamHeader {
            key: "X-User-Id".to_string(),
            value: account.uid.clone(),
        },
        UpstreamHeader {
            key: "X-Enterprise-Id".to_string(),
            value: account.enterprise_id.clone(),
        },
        UpstreamHeader {
            key: "X-Tenant-Id".to_string(),
            value: account.enterprise_id.clone(),
        },
        UpstreamHeader {
            key: "X-Domain".to_string(),
            value: account.domain.clone(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> credentials::Account {
        credentials::Account {
            uid: "uid-1".into(),
            nickname: "小明".into(),
            email: "ming@example.com".into(),
            enterprise_id: "ent-1".into(),
            domain: "www.codebuddy.cn".into(),
            access_token: "tok-abc".into(),
            refresh_token: "ref".into(),
            expires_at_ms: 0,
        }
    }

    #[test]
    fn display_name_prefers_nickname_then_email_then_uid() {
        // 面板的「当前账号」给人看：昵称 → 邮箱 → uid（都没有才退 uid）
        let mut a = account();
        assert_eq!(a.display_name(), "小明");
        a.nickname = "   ".into(); // 空白昵称视为没有
        assert_eq!(a.display_name(), "ming@example.com");
        a.email = String::new();
        assert_eq!(a.display_name(), "uid-1", "昵称与邮箱都没有时回落 uid");
    }

    #[test]
    fn proxy_config_points_at_workbuddy_backend() {
        let cfg = build_proxy_config(8788, &account());
        assert_eq!(cfg.listen_port, 8788);
        assert_eq!(cfg.upstream_base_url, "https://copilot.tencent.com/v2");
        assert!(cfg.inbound_protocols.contains(&"anthropic".to_string()));
        assert!(cfg.inbound_protocols.contains(&"openai".to_string()));
        assert!(cfg.upstream_api_key.is_empty(), "鉴权由 headers 承担，不填 provider key");
        assert_eq!(
            cfg.upstream_include_v1,
            Some(false),
            "base 以 /v2 结尾，必须关掉 include_v1，否则会拼出 /v2/v1/chat/completions 而 404"
        );
        // 端到端钉死最终 URL：/v2/chat/completions，而不是 /v2/v1/chat/completions
        let (url, auth_name) = crate::proxy::upstream::resolve_url(
            "openai",
            &cfg.upstream_base_url,
            "hy3",
            true,
            cfg.upstream_include_v1,
        );
        assert_eq!(url, "https://copilot.tencent.com/v2/chat/completions");
        assert_eq!(auth_name, "Authorization");
        assert!(
            cfg.models_filter_non_chat,
            "后端目录混着图像模型，不过滤客户端会枚举到却调不通"
        );
        // 过滤规则只有一份：/v1/models 的归一化与 preflight 的 parse_catalog 必须一致
        let raw = serde_json::json!({"data":{"models":[
            {"id":"hy3"},
            {"id":"hunyuan-image-alpha","tags":["text-to-image"]},
            {"id":"space-bunny"}
        ]}});
        let via_proxy = crate::proxy::convert::normalize_models_response(raw.clone(), cfg.models_filter_non_chat);
        let via_preflight = crate::commands::buddy::twoapi::upstream::parse_catalog(&raw);
        let proxy_ids: Vec<String> = via_proxy["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(proxy_ids, via_preflight, "两条路径的过滤结果必须一致");
        assert_eq!(proxy_ids, vec!["hy3", "space-bunny"]);
        assert!(
            cfg.force_upstream_stream,
            "WorkBuddy 后端只收流式，必须强制对上游发流式（客户端要非流式时代理侧聚合）"
        );
    }

    #[test]
    fn proxy_config_carries_all_five_auth_headers() {
        let cfg = build_proxy_config(8788, &account());
        let get = |name: &str| {
            cfg.upstream_headers
                .iter()
                .find(|h| h.key.eq_ignore_ascii_case(name))
                .map(|h| h.value.clone())
        };
        assert_eq!(get("Authorization").as_deref(), Some("Bearer tok-abc"));
        assert_eq!(get("X-User-Id").as_deref(), Some("uid-1"));
        assert_eq!(get("X-Enterprise-Id").as_deref(), Some("ent-1"));
        assert_eq!(get("X-Tenant-Id").as_deref(), Some("ent-1"));
        assert_eq!(get("X-Domain").as_deref(), Some("www.codebuddy.cn"));
        assert!(
            crate::proxy::headers::has_authorization(&cfg.upstream_headers),
            "显式带 Authorization 时 proxy 不应再注入 provider key"
        );
    }

    #[test]
    fn switching_account_changes_header_values_only() {
        let a = build_proxy_config(8788, &account());
        let mut b_account = account();
        b_account.uid = "uid-2".into();
        b_account.access_token = "tok-xyz".into();
        let b = build_proxy_config(8788, &b_account);
        let uid_of = |c: &ProxyConfig| {
            c.upstream_headers.iter().find(|h| h.key == "X-User-Id").map(|h| h.value.clone())
        };
        assert_eq!(uid_of(&a).as_deref(), Some("uid-1"));
        assert_eq!(uid_of(&b).as_deref(), Some("uid-2"), "切号后鉴权头要跟着换");
        assert_eq!(a.listen_port, b.listen_port, "切号不需要换端口/重启");
    }

    #[test]
    fn preflight_passes_only_when_all_three_checks_pass() {
        let ok = run_preflight(&PreflightInputs {
            key: Ok("secret".into()),
            account: Ok(account()),
            catalog: Ok(31),
        });
        assert!(ok.ok, "三项全过应通过：{}", ok.message);
        assert_eq!(ok.checks.len(), 3);
        assert!(ok.message.is_empty());
    }

    #[test]
    fn preflight_reports_key_failure_with_reason() {
        // 密钥拿不到 → 拒绝启动，并把原因带出来（今天的故障就是这样难以定位）
        let r = run_preflight(&PreflightInputs {
            key: Err("无法获取 WorkBuddy at-rest 密钥（loggerGet）。已试路径：D:\\x".into()),
            account: Ok(account()),
            catalog: Ok(31),
        });
        assert!(!r.ok);
        assert!(r.message.contains("密钥"), "{}", r.message);
        assert!(r.message.contains("已试路径"), "要带已试路径：{}", r.message);
    }

    #[test]
    fn preflight_fails_on_empty_catalog() {
        let r = run_preflight(&PreflightInputs {
            key: Ok("s".into()),
            account: Ok(account()),
            catalog: Ok(0),
        });
        assert!(!r.ok, "模型列表为空也算后端不可用");
        assert!(r.message.contains("后端"), "{}", r.message);
    }

    #[test]
    fn preflight_collects_every_failure_not_just_the_first() {
        let r = run_preflight(&PreflightInputs {
            key: Err("密钥拿不到".into()),
            account: Err("登录态读不了".into()),
            catalog: Err("网络不通".into()),
        });
        assert!(!r.ok);
        assert_eq!(r.checks.iter().filter(|c| !c.ok).count(), 3);
        assert!(r.message.contains("密钥") && r.message.contains("登录态") && r.message.contains("后端"));
    }
}

/// 2API 运行状态（前端面板直接渲染）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoApiStatus {
    /// stopped | starting | running | failed
    pub phase: String,
    pub port: u16,
    pub started_at_ms: Option<u64>,
    pub last_error: Option<String>,
    /// 当前注入的账号 uid（账号联动的可见性）
    pub account: Option<String>,
    pub model_count: usize,
}

impl Default for TwoApiStatus {
    fn default() -> Self {
        Self {
            phase: "stopped".to_string(),
            port: DEFAULT_PORT,
            started_at_ms: None,
            last_error: None,
            account: None,
            model_count: 0,
        }
    }
}

use std::sync::LazyLock;
use std::sync::Mutex;

static STATUS: LazyLock<Mutex<TwoApiStatus>> =
    LazyLock::new(|| Mutex::new(TwoApiStatus::default()));

fn status() -> TwoApiStatus {
    STATUS.lock().map(|s| s.clone()).unwrap_or_else(|_| TwoApiStatus::default())
}

fn set_status(f: impl FnOnce(&mut TwoApiStatus)) {
    if let Ok(mut s) = STATUS.lock() {
        f(&mut s);
    }
}

/// 凭据状态机单例（首次使用时初始化，at-rest 密钥只取一次）
static CREDENTIALS: LazyLock<Mutex<Option<std::sync::Arc<credentials::Credentials>>>> =
    LazyLock::new(|| Mutex::new(None));

fn credentials_instance() -> Result<std::sync::Arc<credentials::Credentials>, String> {
    let mut guard = CREDENTIALS
        .lock()
        .map_err(|_| "凭据锁已中毒".to_string())?;
    if let Some(c) = guard.as_ref() {
        return Ok(c.clone());
    }
    let secret = atrest::fetch_key_payload().and_then(|p| atrest::extract_secret(&p))?;
    let path = credentials::find_auth_file()
        .ok_or_else(|| "未找到 WorkBuddy 登录态文件（workbuddy-desktop.info）".to_string())?;
    let creds = std::sync::Arc::new(credentials::Credentials::new(path, secret));
    *guard = Some(creds.clone());
    Ok(creds)
}

/// 2API 默认监听端口（避开 Free Router 的 8787，与原 Python 服务一致）
pub const DEFAULT_PORT: u16 = 8788;

/// 自启用的服务 id（写进 `config.auto_start_services`）
pub const AUTOSTART_ID: &str = "buddy2api";

// ─── Tauri 命令 ───

/// 当前状态（面板轮询用）
#[tauri::command]
pub fn buddy2api_status() -> TwoApiStatus {
    status()
}

/// 2API 的请求日志（面板轮询展示；只返回来源为 `2api` 的，AI 工具代理日志不混入）。
#[tauri::command]
pub fn buddy2api_request_logs() -> Vec<crate::proxy::server::RequestLogEntry> {
    crate::proxy::server::get_request_logs("2api")
}

/// 清空请求日志缓冲（面板「清空」按钮）。
#[tauri::command]
pub fn buddy2api_clear_request_logs() {
    crate::proxy::server::clear_request_logs();
}

/// 启动前置检查：密钥 / 登录态 / 后端。三项全过才允许启动。
#[tauri::command]
pub async fn buddy2api_preflight() -> Result<PreflightReport, String> {
    let key = atrest::fetch_key_payload().and_then(|p| atrest::extract_secret(&p));
    let account = match credentials_instance() {
        Ok(c) => c.current(),
        Err(e) => Err(e),
    };
    let catalog = match (&account, &key) {
        (Ok(a), Ok(_)) => fetch_catalog_count(a).await,
        _ => Err("密钥或登录态不可用，已跳过后端探测".to_string()),
    };
    // 把最近一次探测到的模型数记进状态，供面板展示
    if let Ok(n) = &catalog {
        set_status(|s| s.model_count = *n);
    }
    Ok(run_preflight(&PreflightInputs { key, account, catalog }))
}

/// 拉后端模型目录并返回条数（自检第三项）
async fn fetch_catalog_count(account: &credentials::Account) -> Result<usize, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let url = format!("{}{}", upstream::BACKEND_BASE, upstream::BACKEND_MODELS_PATH);
    let mut map = HeaderMap::new();
    for h in workbuddy_headers(account) {
        let name: HeaderName = h
            .key
            .parse()
            .map_err(|_| format!("非法请求头名：{}", h.key))?;
        let value: HeaderValue = h
            .value
            .parse()
            .map_err(|_| format!("请求头 {} 的值非法", h.key))?;
        map.insert(name, value);
    }
    let resp = reqwest::Client::new()
        .get(&url)
        .headers(map)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("网络失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("后端返回 HTTP {}", resp.status().as_u16()));
    }
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("响应不是合法 JSON：{e}"))?;
    Ok(upstream::parse_catalog(&payload).len())
}

/// 运行中的服务任务（停止时 abort，socket 随之释放）
static SERVER_TASK: LazyLock<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>> =
    LazyLock::new(|| Mutex::new(None));

/// 启动 2API：**先自检，不过就不监听**。
///
/// 宁可"起不来且给出可读原因"，也不要"起来了但所有对话 500" ——
/// 那正是 2026-10-03 那个故障最难发现的原因（/health 等旁路全绿）。
#[tauri::command]
pub async fn buddy2api_start(port: Option<u16>) -> Result<TwoApiStatus, String> {
    let port = port.unwrap_or_else(configured_port);
    if status().phase == "running" {
        return Ok(status());
    }
    let report = buddy2api_preflight().await?;
    if !report.ok {
        set_status(|s| {
            s.phase = "failed".to_string();
            s.last_error = Some(report.message.clone());
        });
        return Err(report.message);
    }
    let creds = credentials_instance()?;
    // 启动即保证 token 有效：临近过期就顺手刷新并写回（写回受 mtime 保护，
    // 期间若 Buddy 切号则只更新内存、不覆盖用户的切换）
    let account = creds.ensure_fresh().await?;
    let cfg = build_proxy_config(port, &account);
    set_status(|s| {
        s.phase = "starting".to_string();
        s.port = port;
        s.last_error = None;
        s.account = Some(account.display_name());
    });

    let handle = tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::proxy::server::start_proxy_server(cfg).await {
            eprintln!("[2api] 服务退出：{e}");
            set_status(|s| {
                s.phase = "failed".to_string();
                s.last_error = Some(e);
            });
        }
    });
    if let Ok(mut guard) = SERVER_TASK.lock() {
        *guard = Some(handle);
    }
    set_status(|s| {
        s.phase = "running".to_string();
        s.started_at_ms = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        );
    });
    // 巡查只在服务运行期间有意义：它热更新的就是这份运行中的配置。
    // 客户端里切号（不会通知我们）与 token 过期，都靠它兜住。
    start_credentials_watcher(creds, (account.uid.clone(), account.access_token.clone()));
    Ok(status())
}

/// 停止 2API
#[tauri::command]
pub fn buddy2api_stop() -> Result<TwoApiStatus, String> {
    if let Ok(mut guard) = SERVER_TASK.lock() {
        if let Some(handle) = guard.take() {
            handle.abort();
        }
    }
    if let Ok(mut guard) = WATCHER_TASK.lock() {
        if let Some(handle) = guard.take() {
            handle.abort();
        }
    }
    let port = status().port;
    crate::proxy::server::unregister_running(port);
    set_status(|s| {
        s.phase = "stopped".to_string();
        s.started_at_ms = None;
    });
    Ok(status())
}

// ─── 账号联动 ───

/// 把某账号的鉴权头热更新到运行中的服务实例。
///
/// 返回 false 表示服务当前没在跑（尚未启动/已停止）—— 这不算错误，
/// 下次启动时会用最新的凭据组装配置。
pub async fn apply_account_to_running(
    port: u16,
    account: &credentials::Account,
) -> bool {
    crate::proxy::server::update_running_config(port, |cfg| {
        cfg.upstream_headers = workbuddy_headers(account);
    })
    .await
}

/// Buddy 面板切号成功后调用：重读登录态并把新鉴权头热更新进服务。
///
/// 同进程调用，不依赖 mtime 轮询，因此没有"切了但还没生效"的窗口。
/// 返回是否真的更新到了运行中的服务。
pub async fn on_account_switched() -> Result<bool, String> {
    let creds = credentials_instance()?;
    creds.reload_from_buddy()?;
    let account = creds.current()?;
    let port = status().port;
    let applied = apply_account_to_running(port, &account).await;
    set_status(|s| {
        s.account = Some(account.display_name());
        if applied {
            s.last_error = None;
        }
    });
    eprintln!("[2api] 凭据已同步（uid={}，服务{}）", account.uid, if applied { "已热更新" } else { "未在运行" });
    Ok(applied)
}

/// 把「凭据同步失败」记进状态，让面板能看见。
///
/// 只 `eprintln` 等于没报错：服务会继续用旧账号的鉴权头发请求，而旧账号**仍然有效**时
/// 请求一切正常 —— 只是额度和身份记在别人头上，界面上完全看不出来。
pub fn note_sync_error(err: &str) {
    set_status(|s| {
        s.last_error = Some(format!(
            "2API 凭据同步失败：{err}（服务仍在用旧账号的凭据，可尝试重启 2API）"
        ));
    });
}

// ─── 凭据巡查 ───

/// 巡查间隔（ms）。
///
/// 30s 是权衡：最坏情况是切号后半分钟才生效，而 2API 请求本来就走网络，
/// 用户不会盯着这半分钟；再密就是白白 stat 文件。
pub const WATCH_INTERVAL_MS: u64 = 30_000;

/// 巡查任务句柄（停止服务时 abort，避免残留一轮）
static WATCHER_TASK: LazyLock<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>> =
    LazyLock::new(|| Mutex::new(None));

/// 巡查的纯判定：这份凭据是否与上次写进服务的那份不同。
///
/// 抽成纯函数是为了能单测 —— 巡查本体要起异步任务、依赖运行中的服务，测不动。
fn identity_changed(previous: Option<&(String, String)>, uid: &str, token: &str) -> bool {
    match previous {
        None => true,
        Some((u, t)) => u != uid || t != token,
    }
}

/// 起凭据巡查：**用户在 WorkBuddy 客户端里自己切号**这条路径靠它兜住。
///
/// 启动时的鉴权头是**快照**进 `ProxyConfig` 的（见 [`buddy2api_start`]），而客户端切号
/// 不会通知我们，也没有人会来调 [`on_account_switched`] —— 不巡查的话服务会一直用旧
/// 账号发请求。`credentials::Credentials::reload_if_changed` 注释里写的"每个请求前调用"
/// 是**不成立的**：proxy 请求路径根本不碰 `Credentials`，那条路径实际上是死的。
///
/// 顺带解决长期运行的 token 过期：`ensure_fresh` 原先只在启动时调一次，
/// 现在每轮都过一遍（临近过期才真发请求）。
///
/// `initial` = 启动时写进服务的那份凭据，避免第一轮就无谓地重写一次。
fn start_credentials_watcher(
    creds: std::sync::Arc<credentials::Credentials>,
    initial: (String, String),
) {
    let mut applied: Option<(String, String)> = Some(initial);
    let handle = tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(WATCH_INTERVAL_MS)).await;
            // ensure_fresh 内部先走 current()（含 reload_if_changed），所以客户端切号
            // 在这一步被读到；写回仍受 mtime 保护，不会覆盖用户刚做的切换。
            let account = match creds.ensure_fresh().await {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("[2api] 巡查：读取/刷新凭据失败：{e}");
                    continue;
                }
            };
            if !identity_changed(applied.as_ref(), &account.uid, &account.access_token) {
                continue;
            }
            // 每轮重新取端口：改端口会重启服务，写死启动时的端口会打空
            let updated = apply_account_to_running(status().port, &account).await;
            if !updated {
                // 服务没在跑（或重启中间态）：不记进 applied，下一轮再试
                continue;
            }
            set_status(|s| {
                s.account = Some(account.display_name());
                s.last_error = None;
            });
            applied = Some((account.uid.clone(), account.access_token.clone()));
            eprintln!("[2api] 巡查：账号已变，鉴权头热更新（uid={}）", account.uid);
        }
    });
    if let Ok(mut guard) = WATCHER_TASK.lock() {
        *guard = Some(handle);
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[tokio::test]
    async fn updating_an_unregistered_port_is_a_noop() {
        // 服务没在跑时（尚未启动/已停止）不应报错：下次启动会用最新凭据组装配置
        let mut touched = false;
        let ok = crate::proxy::server::update_running_config(59999, |_| touched = true).await;
        assert!(!ok, "未注册的端口应返回 false");
        assert!(!touched, "不应执行修改");
    }

    #[test]
    fn watcher_applies_only_when_uid_or_token_moves() {
        let prev = ("uid-a".to_string(), "tok-1".to_string());
        assert!(identity_changed(None, "uid-a", "tok-1"), "首轮必须写进服务");
        assert!(
            !identity_changed(Some(&prev), "uid-a", "tok-1"),
            "没变就别反复写（每 30s 一次的无谓重写）"
        );
        assert!(
            identity_changed(Some(&prev), "uid-b", "tok-1"),
            "客户端切号 → uid 变了，必须更新"
        );
        assert!(
            identity_changed(Some(&prev), "uid-a", "tok-2"),
            "token 刷新 → 同一个 uid 也要更新（否则长期运行会撞上过期 token）"
        );
    }

    #[test]
    fn sync_failure_reaches_the_panel_not_only_stderr() {
        // 同步失败若只 eprintln，用户永远看不到：旧账号仍有效时请求照样 200，
        // 只是额度记在别人头上
        note_sync_error("读取登录态失败");
        let err = status().last_error.unwrap_or_default();
        assert!(err.contains("读取登录态失败"), "{err}");
        assert!(err.contains("旧账号"), "要说清服务仍在用旧凭据：{err}");
        set_status(|s| s.last_error = None);
    }
}

    /// 端到端：真机起服务、打真请求。
    ///
    /// 单元测试证明不了代理链路（proxy 转换 → 上游 → SSE）真的通。
    /// 需要网络与本机 WorkBuddy，默认不跑：
    /// `cargo test --no-default-features --lib -- --ignored twoapi::e2e`
    #[tokio::test]
    #[ignore = "端到端：需要网络与本机 WorkBuddy"]
    async fn e2e_chat_completions_against_real_backend() {
        let started = buddy2api_start(None).await.expect("启动 2API");
        assert_eq!(started.phase, "running", "启动失败：{:?}", started.last_error);

        // axum 监听是异步的，给它一点时间
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", started.port);
        let payload = serde_json::json!({
            "model": "hy3",
            "messages": [{"role": "user", "content": "1+1=? 只回答数字"}],
            "stream": false
        });
        let resp = client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(90))
            .send()
            .await
            .expect("请求失败");
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        println!("[e2e] 非流式 HTTP {status}");
        assert_eq!(status.as_u16(), 200, "非流式失败：{body}");
        let v: serde_json::Value = serde_json::from_str(&body).expect("响应不是 JSON");
        let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
        println!("[e2e] content = {content:?}");
        assert!(!content.is_empty(), "内容为空：{body}");

        buddy2api_stop().expect("停止 2API");
    }

    /// 端到端：流式 + Anthropic messages + /v1/models
    #[tokio::test]
    #[ignore = "端到端：需要网络与本机 WorkBuddy"]
    async fn e2e_streaming_anthropic_and_models() {
        let started = buddy2api_start(None).await.expect("启动 2API");
        assert_eq!(started.phase, "running", "启动失败：{:?}", started.last_error);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let client = reqwest::Client::new();
        let base = format!("http://127.0.0.1:{}", started.port);

        // 流式
        let resp = client
            .post(format!("{base}/v1/chat/completions"))
            .json(&serde_json::json!({
                "model": "hy3",
                "messages": [{"role": "user", "content": "说：好"}],
                "stream": true
            }))
            .timeout(std::time::Duration::from_secs(90))
            .send()
            .await
            .expect("流式请求失败");
        let st = resp.status();
        let text = resp.text().await.unwrap_or_default();
        println!("[e2e] 流式 HTTP {st}，body 前 120 字 = {:?}", &text[..std::cmp::min(120, text.len())]);
        assert_eq!(st.as_u16(), 200, "流式失败：{text}");
        assert!(text.contains("data:"), "应是 SSE 格式：{text}");

        // Anthropic messages
        let resp2 = client
            .post(format!("{base}/v1/messages"))
            .json(&serde_json::json!({
                "model": "hy3",
                "max_tokens": 64,
                "messages": [{"role": "user", "content": "1+1=?"}]
            }))
            .timeout(std::time::Duration::from_secs(90))
            .send()
            .await
            .expect("messages 请求失败");
        let st2 = resp2.status();
        let text2 = resp2.text().await.unwrap_or_default();
        println!("[e2e] messages HTTP {st2}，body 前 160 字 = {:?}", &text2.chars().take(160).collect::<String>());
        assert_eq!(st2.as_u16(), 200, "messages 失败：{text2}");

        // /v1/models —— 客户端靠它枚举模型
        let resp3 = client
            .get(format!("{base}/v1/models"))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .expect("models 请求失败");
        let st3 = resp3.status();
        let text3 = resp3.text().await.unwrap_or_default();
        println!("[e2e] models HTTP {st3}，body 前 200 字 = {:?}", &text3.chars().take(200).collect::<String>());

        buddy2api_stop().expect("停止 2API");
    }

    /// 端到端：/v1/models 必须列全（客户端靠它枚举模型，含 space-bunny 这类新模型）
    #[tokio::test]
    #[ignore = "端到端：需要网络与本机 WorkBuddy"]
    async fn e2e_models_lists_new_models() {
        let started = buddy2api_start(None).await.expect("启动 2API");
        assert_eq!(started.phase, "running");
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://127.0.0.1:{}/v1/models", started.port))
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
            .expect("models 请求失败");
        assert_eq!(resp.status().as_u16(), 200);
        let v: serde_json::Value = resp.json().await.expect("不是 JSON");
        let ids: Vec<String> = v["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(|x| x.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        println!("[e2e] /v1/models 共 {} 个", ids.len());
        println!("[e2e] 列表 = {}", ids.join(", "));
        assert!(
            ids.iter().any(|i| i == "space-bunny"),
            "必须含 space-bunny（上游已支持、本地 product.json 没有的模型）"
        );
        buddy2api_stop().expect("停止 2API");
    }

// ─── 端口设置 ───

/// 用户配置的监听端口（默认 8788）
pub fn configured_port() -> u16 {
    let p = crate::commands::config::load_config().twoapi_port;
    if p == 0 { DEFAULT_PORT } else { p }
}

/// 把 AI 模块里指向 2API 的供应商 URL 从旧端口改到新端口，返回改了几条。
///
/// 判定口径：供应商 id 是 workbuddy2api，或 URL 里带旧端口 —— 后者覆盖用户
/// 改过 id / 复制过供应商的情况。只改 URL 中的端口段，其余（路径、协议）原样保留。
fn sync_ai_providers(old: u16, new: u16) -> Result<usize, String> {
    if old == new {
        return Ok(0);
    }
    let mut cfg = crate::commands::ai::config::load_ai_config();
    let old_seg = format!(":{old}");
    let new_seg = format!(":{new}");
    let mut changed = 0usize;
    for p in &mut cfg.providers {
        let urls = [&p.openai_url, &p.anthropic_url, &p.google_url];
        let is_ours = p.id == "workbuddy2api" || urls.iter().any(|u| u.contains(&old_seg));
        if !is_ours {
            continue;
        }
        for u in [&mut p.openai_url, &mut p.anthropic_url, &mut p.google_url] {
            if u.contains(&old_seg) {
                *u = u.replace(&old_seg, &new_seg);
                changed += 1;
            }
        }
    }
    if changed > 0 {
        crate::commands::ai::config::save_ai_config_to_file(&cfg)?;
    }
    Ok(changed)
}

/// 修改 2API 端口：保存配置 + 同步 AI 供应商 URL + 若在运行则按新端口重启。
///
/// 端口被占用时用户的常规出路就是改端口；但 AI 模块里那个指向 2API 的供应商
/// 还写着旧地址，不同步的话换一个供应商就报连接被拒 —— 所以这里一并处理。
#[tauri::command]
pub async fn buddy2api_set_port(port: u16) -> Result<TwoApiStatus, String> {
    if port < 1024 {
        return Err(format!("端口 {port} 太小，请用 1024 以上的端口"));
    }
    if port == 8787 {
        return Err("8787 是 Free Router 的默认端口，请换一个".to_string());
    }
    let old = configured_port();
    if old != port {
        let mut cfg = crate::commands::config::load_config();
        cfg.twoapi_port = port;
        crate::commands::config::save_config(&cfg)?;
    }
    let synced = sync_ai_providers(old, port)?;
    set_status(|s| s.port = port);

    // 已在运行：换端口必须重启（监听地址变了，无法热切）
    let was_running = status().phase == "running";
    if was_running {
        let _ = buddy2api_stop();
    }
    if was_running {
        return buddy2api_start(Some(port)).await;
    }
    eprintln!("[2api] 端口 {old} → {port}（已同步 {synced} 条 AI 供应商 URL）");
    Ok(status())
}

#[cfg(test)]
mod direct_probe {
    use super::*;

    /// 二分定位：绕开 proxy 直连上游。若这里 200 而经 proxy 404，问题在转发；
    /// 若这里也 404，问题在凭据或上游。
    #[tokio::test]
    #[ignore = "诊断用：需要网络与本机 WorkBuddy"]
    async fn probe_upstream_directly() {
        let creds = credentials_instance().expect("凭据");
        let acct = creds.current().expect("当前账号");
        eprintln!("[direct] uid={} domain={} token_len={}", acct.uid, acct.domain, acct.access_token.len());
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        for h in workbuddy_headers(&acct) {
            let n: HeaderName = h.key.parse().unwrap();
            let v: HeaderValue = h.value.parse().unwrap();
            headers.insert(n, v);
        }
        for path in ["/v2/chat/completions", "/v2/models"] {
            let method = if path.contains("chat") { "POST" } else { "GET" };
            let mut req = reqwest::Client::new()
                .request(reqwest::Method::from_bytes(method.as_bytes()).unwrap(), format!("{}{}", super::upstream::BACKEND_BASE, path))
                .headers(headers.clone())
                .timeout(std::time::Duration::from_secs(30));
            if method == "POST" {
                req = req.json(&serde_json::json!({"model":"hy3","messages":[{"role":"user","content":"1+1=?"}],"stream":false}));
            }
            match req.send().await {
                Ok(r) => {
                    let s = r.status();
                    let t = r.text().await.unwrap_or_default();
                    eprintln!("[direct] {method} {path} -> {s}  {}", &t[..std::cmp::min(160, t.len())]);
                }
                Err(e) => eprintln!("[direct] {method} {path} 请求失败: {e}"),
            }
            // 上游可能只收流式：单独验证一次
            if method == "POST" {
                let stream_req = reqwest::Client::new()
                    .post(format!("{}/v2/chat/completions", super::upstream::BACKEND_BASE))
                    .headers(headers.clone())
                    .timeout(std::time::Duration::from_secs(30))
                    .json(&serde_json::json!({
                        "model": "hy3",
                        "messages": [{"role": "user", "content": "1+1=?"}],
                        "stream": true
                    }));
                match stream_req.send().await {
                    Ok(r) => {
                        let s = r.status();
                        let t = r.text().await.unwrap_or_default();
                        eprintln!(
                            "[direct] POST /v2/chat/completions (stream=true) -> {s}  {}",
                            &t[..std::cmp::min(200, t.len())]
                        );
                    }
                    Err(e) => eprintln!("[direct] 流式请求失败: {e}"),
                }
            }
        }
    }
}
