//! HTTP 服务本体：接口路径与落盘布局沿用 0.1.x，行为不变，只是多了日志与统计。

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
use std::process::Command;
use std::sync::Arc;

use crate::config::Config;

pub struct AppData {
    pub cfg: Config,
    pub state: Arc<crate::AppState>,
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

/// 路径安全名：仅保留字母数字、点、连字符与下划线；通配符 `*` 转 `_`。
pub fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            c if c.is_alphanumeric() => c,
            '.' | '-' | '_' => c,
            '*' => '_',
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

fn ensure_dir(p: &Path) -> Result<(), (StatusCode, String)> {
    std::fs::create_dir_all(p)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败 {}: {}", p.display(), e)))
}

fn write_file(p: &Path, content: &str) -> Result<(), (StatusCode, String)> {
    std::fs::write(p, content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("写入 {} 失败: {}", p.display(), e)))
}

/// 取 PEM 中第一段证书（叶子证书），用于 cert.cer。
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

fn write_layout(root: &Path, dir_name: &str, body: &PushBody) -> Result<(), (StatusCode, String)> {
    let dir = root.join(dir_name);
    ensure_dir(&dir)?;
    write_file(&dir.join("cert.key"), &body.key)?;
    write_file(&dir.join("fullchain.cer"), &body.cert)?;
    write_file(&dir.join("cert.cer"), &pem_first_block(&body.cert))?;
    write_file(&dir.join("chain.cer"), &body.ca)?;
    let meta = Meta {
        certificate_id: &body.certificate_id,
        domains: &body.domains,
        expired_time: &body.expired_time,
        updated_at_ms: now_ms(),
    };
    let text = serde_json::to_string_pretty(&meta)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    write_file(&dir.join("meta.json"), &text)?;
    Ok(())
}

fn run_command(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .args(args)
        .output()
        .map_err(|e| format!("启动 {} 失败: {}", program, e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!(
            "{} 返回非零({}): {} {}",
            program, out.status, stdout, stderr
        ));
    }
    Ok(stdout)
}

fn run_reload(cfg: &Config) -> Result<String, String> {
    // 旧配置：post_script 优先兼容
    if let Some(script) = &cfg.post_script {
        if !script.trim().is_empty() {
            return run_command("cmd", &["/c", script]);
        }
    }
    match cfg.reload_mode.as_str() {
        "none" | "" => Ok("未配置重载".to_string()),
        "command" => {
            if cfg.reload_command.trim().is_empty() {
                return Err("reload_mode=command 但缺少 reload_command".into());
            }
            run_command("cmd", &["/c", &cfg.reload_command])
        }
        "nginx" => {
            let exe = if cfg.nginx_prefix.trim().is_empty() {
                PathBuf::from("nginx.exe")
            } else {
                Path::new(cfg.nginx_prefix.trim()).join("nginx.exe")
            };
            let mut args: Vec<String> = Vec::new();
            if !cfg.nginx_prefix.trim().is_empty() {
                args.push("-p".into());
                args.push(cfg.nginx_prefix.trim().into());
            }
            if !cfg.nginx_conf.trim().is_empty() {
                args.push("-c".into());
                args.push(cfg.nginx_conf.trim().into());
            }
            args.push("-s".into());
            args.push("reload".into());
            let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            run_command(&exe.to_string_lossy(), &refs)
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
            run_command("cmd", &["/c", &cmd])
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

    let result = push_inner(&data, &body);
    match &result {
        Ok((written, _)) => data.state.record_push(
            true,
            format!(
                "{} 推送成功（{}），写入 {} 处",
                peer.ip(),
                body.certificate_id,
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
        "certificateId": body.certificate_id,
        "written": written,
        "reload": reload,
    })))
}

fn push_inner(
    data: &AppData,
    body: &PushBody,
) -> Result<(Vec<String>, String), (StatusCode, String)> {
    let domain = body
        .domains
        .first()
        .cloned()
        .unwrap_or_else(|| body.domain.clone());
    let cert_id = if body.certificate_id.trim().is_empty() {
        domain.clone()
    } else {
        body.certificate_id.clone()
    };
    if cert_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "certificate_id 与 domain 不能同时为空".into()));
    }

    let root = PathBuf::from(&data.cfg.output_dir);
    ensure_dir(&root)?;

    let mut wrote = Vec::new();
    match data.cfg.layout.as_str() {
        "cert" => {
            write_layout(&root, &safe_name(&cert_id), body)?;
            wrote.push(safe_name(&cert_id));
        }
        "domain" => {
            if !domain.is_empty() {
                write_layout(&root, &safe_name(&domain), body)?;
                wrote.push(safe_name(&domain));
            }
        }
        "both" => {
            write_layout(&root, &safe_name(&cert_id), body)?;
            wrote.push(safe_name(&cert_id));
            if !domain.is_empty() && safe_name(&domain) != safe_name(&cert_id) {
                write_layout(&root, &safe_name(&domain), body)?;
                wrote.push(safe_name(&domain));
            }
        }
        "flat" => {
            let name = safe_name(if !body.domain.is_empty() { &body.domain } else { &cert_id });
            let p = root.join(format!("{}.crt", name));
            write_file(&p, &body.cert)?;
            write_file(&root.join(format!("{}.key", name)), &body.key)?;
            if !body.ca.is_empty() {
                write_file(&root.join(format!("{}.issuer.crt", name)), &body.ca)?;
            }
            // 同时写一份标准布局，便于 Nginx/脚本直接引用
            write_layout(&root, &safe_name(&cert_id), body)?;
            wrote.push(name);
        }
        other => return Err((StatusCode::BAD_REQUEST, format!("未知 layout: {}", other))),
    }

    let reload = match run_reload(&data.cfg) {
        Ok(msg) => {
            data.state.log(format!("  重载：{}", msg));
            msg
        }
        Err(e) => {
            // 证书已经落盘，重载失败只提示不吞掉（老版本同样返回 500 + 提示）
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("证书已写入，但重载失败: {}", e),
            ));
        }
    };
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
        return Err((StatusCode::NOT_FOUND, "拉取接口未启用（配置 api_key 后可用）".into()));
    }
    let ts: u128 = q
        .timestamp
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "timestamp 无效".into()))?;
    let now = now_ms();
    let diff = if ts > now { ts - now } else { now - ts };
    if diff > 5 * 60 * 1000 {
        return Err((StatusCode::BAD_REQUEST, "timestamp 已过期（5 分钟有效）".into()));
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
        data.state.log(format!("拉取 {} 失败：签名校验不通过", q.certificate_id));
        return Err((StatusCode::UNAUTHORIZED, "签名校验失败".into()));
    }
    let _ = q.api_key; // 请求里可以不带 apiKey，只参与签名

    let root = PathBuf::from(&cfg.output_dir);
    let dir = find_cert_dir(&root, &q.certificate_id).ok_or_else(|| {
        (StatusCode::NOT_FOUND, format!("证书 {} 不存在", q.certificate_id))
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
    fn safe_name_strips_traversal_and_wildcard() {
        assert_eq!(safe_name("*.a.com"), "_.a.com");
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
