//! HTTP 服务本体：接口路径与落盘布局沿用 0.1.x，行为不变，只是多了日志与统计。

use crate::deployment::Deployment;
use crate::reload_command::run as run_command;
use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Deserialize)]
struct PushBody {
    #[serde(default)]
    certificate_id: String,
    #[serde(default)]
    domain: String,
    #[serde(default)]
    domains: Vec<String>,
    cert: String,
    key: String,
    #[serde(default)]
    ca: String,
    #[serde(default)]
    expired_time: String,
}

#[derive(Serialize)]
struct Meta<'a> {
    certificate_id: &'a str,
    domains: &'a [String],
    expired_time: &'a str,
    updated_at_ms: u128,
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 路径安全名：仅保留字母数字、点、连字符与下划线。
///
/// 通配符 `*` **直接丢掉**（不是换成 `_`）：`*.seemove.cn` → `seemove.cn`。
/// 这样通配符证书和裸域名证书落在同一个目录，正好对应 nginx 里
/// `ssl_certificate ./conf/cert/seemove.cn/fullchain.cer;` 这类写法。
pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| *c != '*')
        .map(|c| match c {
            c if c.is_alphanumeric() => c,
            '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    let s = s.trim_matches('.').to_string();
    if s.is_empty() {
        "cert".to_string()
    } else {
        s
    }
}

pub fn pem_first_block(pem: &str) -> String {
    let mut out = String::new();
    let mut in_block = false;
    for line in pem.lines() {
        let t = line.trim();
        if t.starts_with("-----BEGIN") {
            if in_block {
                break;
            }
            in_block = true;
        }
        if !in_block {
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

fn write_layout(
    deployment: &mut Deployment,
    dir_name: &str,
    body: &PushBody,
) -> Result<(), (StatusCode, String)> {
    let meta = Meta {
        certificate_id: &body.certificate_id,
        domains: &body.domains,
        expired_time: &body.expired_time,
        updated_at_ms: now_ms(),
    };
    let text = serde_json::to_string_pretty(&meta)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    deployment
        .directory(
            dir_name,
            vec![
                ("cert.key", body.key.clone()),
                ("fullchain.cer", body.cert.clone()),
                ("cert.cer", pem_first_block(&body.cert)),
                ("chain.cer", body.ca.clone()),
                ("meta.json", text),
            ],
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}

/// 执行外部命令。
///
/// `cwd` 用来把工作目录固定到 nginx 前缀：配置文件里的相对路径
/// （`ssl_certificate "./conf/cert/xxx/fullchain.cer"`）会按**进程工作目录**解析，
/// 不设的话就跟着接收端自己的启动目录飘 —— 「手动 reload 正常、接收端 reload 报
/// 找不到证书」正是这么来的。
fn quoted(s: &str) -> String {
    format!("\"{}\"", s)
}

/// 拼一条「人能看懂、空格也藏不住」的命令行用于日志。
fn cmdline(program: &str, args: &[String]) -> String {
    let mut s = quoted(program);
    for a in args {
        s.push(' ');
        s.push_str(&quoted(a));
    }
    s
}

fn as_refs(v: &[String]) -> Vec<&str> {
    v.iter().map(|s| s.as_str()).collect()
}

fn run_reload(data: &AppData) -> Result<String, String> {
    let cfg = &data.cfg;
    let prefix = cfg.nginx_prefix.trim();
    // nginx 相关的命令都固定在 nginx 前缀下执行（等价于管理员先 cd 再执行）
    let cwd = if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    };

    // 旧配置：post_script 优先兼容
    if let Some(script) = &cfg.post_script {
        if !script.trim().is_empty() {
            data.state
                .log(format!("重载脚本：cmd /c {}", quoted(script)));
            return run_command("cmd", &["/c", script], cwd);
        }
    }
    match cfg.reload_mode.as_str() {
        "none" | "" => Ok("未配置重载".to_string()),
        "command" => {
            let script = cfg.reload_command.trim();
            if script.is_empty() {
                return Err("reload_mode=command 但缺少 reload_command".into());
            }
            data.state
                .log(format!("重载命令：cmd /c {}", quoted(script)));
            run_command("cmd", &["/c", script], cwd)
        }
        "nginx" => {
            let exe = if prefix.is_empty() {
                "nginx.exe".to_string()
            } else {
                Path::new(prefix)
                    .join("nginx.exe")
                    .to_string_lossy()
                    .to_string()
            };
            let mut base: Vec<String> = Vec::new();
            if !prefix.is_empty() {
                base.push("-p".into());
                base.push(prefix.to_string());
            }
            if !cfg.nginx_conf.trim().is_empty() {
                base.push("-c".into());
                base.push(cfg.nginx_conf.trim().to_string());
            }

            // 1) 先校验配置：有问题当场说清楚，别让人以为是「证书推送失败」。
            //    nginx 的相对路径按工作目录解析，这也顺带验证了 cwd 设对了没有。
            let mut test_args = base.clone();
            test_args.push("-t".into());
            data.state
                .log(format!("校验配置：{}", cmdline(&exe, &test_args)));
            if let Err(e) = run_command(&exe, &as_refs(&test_args), cwd) {
                return Err(format!("nginx 配置校验失败（已跳过 reload）：{}", e));
            }

            // 2) 再发 reload
            let mut reload_args = base.clone();
            reload_args.push("-s".into());
            reload_args.push("reload".into());
            data.state
                .log(format!("重载：{}", cmdline(&exe, &reload_args)));
            run_command(&exe, &as_refs(&reload_args), cwd)
        }
        "service" => {
            if cfg.service_name.trim().is_empty() {
                return Err("reload_mode=service 但缺少 service_name".into());
            }
            let cmd = format!(
                "net stop \"{}\" && net start \"{}\"",
                cfg.service_name.trim(),
                cfg.service_name.trim()
            );
            data.state.log(format!("重载服务：{}", cmd));
            run_command("cmd", &["/c", &cmd], None)
        }
        other => Err(format!("未知 reload_mode: {}", other)),
    }
}

fn require_token(headers: &HeaderMap, cfg: &Config) -> Result<(), (StatusCode, String)> {
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let expected = format!("Bearer {}", cfg.token);
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
    let (domain, cert_id) = resolve_target_names(&data.cfg.layout, body)?;
    crate::deployment::validate_identity(&body.cert, &body.key)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    deploy_validated(data, body, domain, cert_id)
}

fn resolve_target_names(
    layout: &str,
    body: &PushBody,
) -> Result<(String, String), (StatusCode, String)> {
    let domain = body
        .domains
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| body.domain.trim())
        .to_string();
    let cert_id = if body.certificate_id.trim().is_empty() {
        domain.clone()
    } else {
        body.certificate_id.trim().to_string()
    };
    if cert_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "certificate_id 与 domain 不能同时为空".into(),
        ));
    }
    if layout == "domain" && domain.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "按域名部署时必须提供非空域名".into(),
        ));
    }
    Ok((domain, cert_id))
}

fn deploy_validated(
    data: &AppData,
    body: &PushBody,
    domain: String,
    cert_id: String,
) -> Result<(Vec<String>, String), (StatusCode, String)> {
    let root = PathBuf::from(&data.cfg.output_dir);
    let mut deployment =
        Deployment::new(&root).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let mut wrote = Vec::new();
    match data.cfg.layout.as_str() {
        "cert" => {
            write_layout(&mut deployment, &safe_name(&cert_id), body)?;
            wrote.push(safe_name(&cert_id));
        }
        "domain" => {
            if !domain.is_empty() {
                write_layout(&mut deployment, &safe_name(&domain), body)?;
                wrote.push(safe_name(&domain));
            }
        }
        "both" => {
            write_layout(&mut deployment, &safe_name(&cert_id), body)?;
            wrote.push(safe_name(&cert_id));
            if !domain.is_empty() && safe_name(&domain) != safe_name(&cert_id) {
                write_layout(&mut deployment, &safe_name(&domain), body)?;
                wrote.push(safe_name(&domain));
            }
        }
        "flat" => {
            let name = safe_name(if !body.domain.is_empty() {
                &body.domain
            } else {
                &cert_id
            });
            deployment
                .file(&format!("{}.crt", name), &body.cert)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
            deployment
                .file(&format!("{}.key", name), &body.key)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
            if !body.ca.is_empty() {
                deployment
                    .file(&format!("{}.issuer.crt", name), &body.ca)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
            }
            // 同时写一份标准布局，便于 Nginx/脚本直接引用
            write_layout(&mut deployment, &safe_name(&cert_id), body)?;
            wrote.push(name);
        }
        other => return Err((StatusCode::BAD_REQUEST, format!("未知 layout: {}", other))),
    }

    if wrote.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "没有可部署的证书目标".into()));
    }
    deployment
        .install()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let reload = match run_reload(data) {
        Ok(msg) => {
            data.state.log(format!("  重载：{}", msg));
            msg
        }
        Err(e) => {
            let rollback = deployment.rollback();
            let recovery = if rollback.is_ok() {
                run_reload(data)
            } else {
                Ok(String::new())
            };
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "重载失败: {e}；{}{}",
                    rollback.err().unwrap_or_else(|| "已恢复原证书文件".into()),
                    recovery
                        .err()
                        .map(|error| format!("；恢复后的重载也失败: {error}"))
                        .unwrap_or_default()
                ),
            ));
        }
    };
    deployment
        .finish()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((wrote, reload))
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

fn find_cert_dir(root: &Path, cert_id: &str) -> Option<PathBuf> {
    let direct = root.join(safe_name(cert_id));
    if direct.join("fullchain.cer").is_file() {
        return Some(direct);
    }
    // layout=domain/flat 时目录名不是证书 ID：扫描 meta.json
    let entries = std::fs::read_dir(root).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let meta = p.join("meta.json");
        if let Ok(text) = std::fs::read_to_string(&meta) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if v.get("certificate_id").and_then(|x| x.as_str()) == Some(cert_id) {
                    return Some(p);
                }
            }
        }
    }
    None
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
    params.insert("apiKey".to_string(), cfg.api_key.clone());
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
    let dir = find_cert_dir(&root, &q.certificate_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("证书 {} 不存在", q.certificate_id),
        )
    })?;
    let key = std::fs::read_to_string(dir.join("cert.key")).unwrap_or_default();
    let cert = std::fs::read_to_string(dir.join("fullchain.cer")).unwrap_or_default();
    let meta: Value = std::fs::read_to_string(dir.join("meta.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let expired = meta
        .get("expired_time")
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    data.state.log(format!("拉取 {} 成功", q.certificate_id));

    Ok(Json(json!({
        "success": true,
        "msg": "成功",
        "payload": {
            "certKey": key,
            "fullChainCerts": cert,
            "expiredTime": expired,
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
