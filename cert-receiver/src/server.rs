//! HTTP 服务本体：接口路径与落盘布局沿用 0.1.x，行为不变，只是多了日志与统计。

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::Config;

pub struct AppData {
    pub cfg: Config,
    pub state: Arc<crate::AppState>,
}

fn deployment_slot() -> &'static Arc<tokio::sync::Semaphore> {
    static SLOT: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(1)))
}

pub fn prepare_storage(config: &Config) -> Result<(), String> {
    let _permit = deployment_slot()
        .clone()
        .try_acquire_owned()
        .map_err(|_| "证书部署仍在进行，请稍后重启服务".to_string())?;
    crate::deployment::recover(Path::new(&config.output_dir))
}

pub fn router(data: Arc<AppData>) -> Router {
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
/// 通配符 `*` **直接丢掉**（不是换成 `_`）：`*.seemove.cn` → `seemove.cn`。
/// 这样通配符证书和裸域名证书落在同一个目录，正好对应 nginx 里
/// `ssl_certificate ./conf/cert/seemove.cn/fullchain.cer;` 这类写法。
pub use cert_deployment::protocol::safe_name;

fn require_token(headers: &HeaderMap, cfg: &Config) -> Result<(), (StatusCode, String)> {
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
#[cfg(test)]
fn resolve_target_names(
    layout: &str,
    body: &PushBody,
) -> Result<(String, String), (StatusCode, String)> {
    cert_deployment::protocol::resolve_target_names(layout, body)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.message))
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

    // 与 ohttps 相同：apiId/apiKey/certificateId/timestamp 一起按字母序排序拼接
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

    // Read the key/certificate/meta from one generation, never across a swap.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_layout_rejects_missing_domain_before_any_deployment() {
        let body: PushBody =
            serde_json::from_value(json!({"certificate_id":"id-only","cert":"","key":""})).unwrap();
        assert_eq!(
            resolve_target_names("domain", &body).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn blank_domain_entries_fall_back_to_valid_domain() {
        let body:PushBody=serde_json::from_value(json!({"certificate_id":"id","domains":[""," "],"domain":" example.com ","cert":"","key":""})).unwrap();
        assert_eq!(
            resolve_target_names("domain", &body).unwrap(),
            ("example.com".into(), "id".into())
        );
    }

    #[test]
    fn safe_name_strips_traversal_and_wildcard() {
        // 通配符直接丢掉：通配符证书和裸域名要落同一个目录，
        // nginx 里才只写一条 ./conf/cert/seemove.cn/...
        assert_eq!(safe_name("*.a.com"), "a.com");
        assert_eq!(safe_name("*.seemove.cn"), "seemove.cn");
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name(""), "cert");
    }

    #[test]
    fn api_sign_matches_ohttps_algorithm() {
        // 与 ohttps 文档示例一致的构造方式（apiKey 参与签名但不放进请求 URL）
        let mut params = BTreeMap::new();
        params.insert("apiId".to_string(), "push-1xlpm40g1105n9gz");
        params.insert("certificateId".to_string(), "cert-n2x740m2qxg8kwyz");
        params.insert("timestamp".to_string(), "1695743251613");
        params.insert("apiKey".to_string(), "89066abd0f21b983331f3aca02590ea3");
        let string_for_sign = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&");
        let sign = lower_md5(&string_for_sign);
        assert_eq!(sign, "c58d69d9e59923f16b7dafa45bef208d");
    }

    #[test]
    fn pem_first_block_keeps_leaf_only() {
        let leaf = "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----\n";
        let chain = "-----BEGIN CERTIFICATE-----\nBBB\n-----END CERTIFICATE-----\n";
        let both = format!("{}{}", leaf, chain);
        assert_eq!(pem_first_block(&both), leaf);
    }
}
