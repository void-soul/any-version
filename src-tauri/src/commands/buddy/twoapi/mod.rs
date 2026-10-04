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
        Ok(a) => format!("{} · token {} 字符", a.uid, a.access_token.len()),
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
    cfg.timeout_secs = 600;
    cfg.upstream_headers = workbuddy_headers(account);
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
            enterprise_id: "ent-1".into(),
            domain: "www.codebuddy.cn".into(),
            access_token: "tok-abc".into(),
            refresh_token: "ref".into(),
            expires_at_ms: 0,
        }
    }

    #[test]
    fn proxy_config_points_at_workbuddy_backend() {
        let cfg = build_proxy_config(8788, &account());
        assert_eq!(cfg.listen_port, 8788);
        assert_eq!(cfg.upstream_base_url, "https://copilot.tencent.com/v2");
        assert!(cfg.inbound_protocols.contains(&"anthropic".to_string()));
        assert!(cfg.inbound_protocols.contains(&"openai".to_string()));
        assert!(cfg.upstream_api_key.is_empty(), "鉴权由 headers 承担，不填 provider key");
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
    let account = creds.current()?;
    let cfg = build_proxy_config(port, &account);
    set_status(|s| {
        s.phase = "starting".to_string();
        s.port = port;
        s.last_error = None;
        s.account = Some(account.uid.clone());
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
    set_status(|s| s.account = Some(account.uid.clone()));
    eprintln!("[2api] 凭据已同步（uid={}，服务{}）", account.uid, if applied { "已热更新" } else { "未在运行" });
    Ok(applied)
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
        println!("[e2e] messages HTTP {st2}，body 前 160 字 = {:?}", &text2[..std::cmp::min(160, text2.len())]);
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
        println!("[e2e] models HTTP {st3}，body 前 200 字 = {:?}", &text3[..std::cmp::min(200, text3.len())]);

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
