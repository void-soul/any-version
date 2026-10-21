//! AnyVersion 证书管理 — Windows 独立接收端。
//!
//! 独立运行，监听 HTTP 端口，接收 AnyVersion 推送的证书（PEM），
//! 校验 Bearer token 后写入 output_dir，并按配置触发 Nginx 重载 / 命令 / 服务重启。
//!
//! 配置（cert-receiver.toml，按顺序查找）：
//!   1. 命令行 `--config <path>`
//!   2. exe 同目录 cert-receiver.toml
//!   3. 当前工作目录 cert-receiver.toml
//!   4. %ProgramData%\Kira\cert-receiver.toml
//!   5. C:\certs\cert-receiver.toml
//!
//! 示例：
//!   listen_addr = "127.0.0.1:9000"
//!   token = "YOUR_SHARED_TOKEN"
//!   output_dir = "C:\\certs"
//!   layout = "cert"                 # cert | domain | both | flat
//!   api_key = "..."                 # 可选：供 API/Docker 拉取接口使用
//!   reload_mode = "nginx"           # none | command | nginx | service
//!   nginx_prefix = "C:\\nginx"
//!   nginx_conf = "C:\\nginx\\conf\\nginx.conf"

use axum::{
    extract::{Query, State},
    http::{header::AUTHORIZATION, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Deserialize)]
struct Config {
    listen_addr: String,
    token: String,
    output_dir: String,
    /// 落盘布局：cert（按证书 ID）/ domain（按域名）/ both / flat（旧版平铺）
    #[serde(default = "default_layout")]
    layout: String,
    /// API/Docker 拉取接口的共享密钥；为空则关闭拉取接口
    #[serde(default)]
    api_key: String,
    /// none | command | nginx | service
    #[serde(default = "default_reload_mode")]
    reload_mode: String,
    /// reload_mode=command 时执行（cmd /c）
    #[serde(default)]
    reload_command: String,
    /// reload_mode=nginx 时的 Nginx 安装目录（含 nginx.exe）
    #[serde(default)]
    nginx_prefix: String,
    /// reload_mode=nginx 时的配置文件路径
    #[serde(default)]
    nginx_conf: String,
    /// reload_mode=service 时的 Windows 服务名
    #[serde(default)]
    service_name: String,
    /// 兼容旧配置：收到证书后执行的脚本
    #[serde(default)]
    post_script: Option<String>,
}

fn default_layout() -> String {
    "cert".to_string()
}
fn default_reload_mode() -> String {
    "none".to_string()
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

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn find_config_path() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--config") {
        if let Some(p) = args.get(i + 1) {
            return Some(PathBuf::from(p));
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("cert-receiver.toml"));
        }
    }
    candidates.push(PathBuf::from("cert-receiver.toml"));
    if let Ok(program_data) = std::env::var("ProgramData") {
        candidates.push(PathBuf::from(program_data).join("Kira").join("cert-receiver.toml"));
    }
    candidates.push(PathBuf::from("C:\\certs\\cert-receiver.toml"));
    candidates.into_iter().find(|p| p.is_file())
}

fn load_config() -> Result<Config, String> {
    let path = find_config_path().ok_or_else(|| {
        "未找到 cert-receiver.toml。请复制 cert-receiver.toml.example 并填写 token/output_dir。".to_string()
    })?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    let cfg: Config = toml::from_str(&text)
        .map_err(|e| format!("{} 配置格式错误: {}", path.display(), e))?;
    if cfg.token.is_empty() || cfg.token == "changeme" || cfg.token == "YOUR_SHARED_TOKEN" {
        return Err(format!(
            "{} 的 token 未配置。请设置一个强随机 token。",
            path.display()
        ));
    }
    if cfg.output_dir.trim().is_empty() {
        return Err(format!("{} 缺少 output_dir", path.display()));
    }
    Ok(cfg)
}

/// 路径安全名：仅保留字母数字、点、连字符与下划线；通配符 `*` 转 `_`。
fn safe_name(name: &str) -> String {
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
fn pem_first_block(pem: &str) -> String {
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

#[derive(Serialize)]
struct Meta<'a> {
    certificate_id: &'a str,
    domains: &'a [String],
    expired_time: &'a str,
    updated_at_ms: u128,
}

fn write_layout(
    root: &Path,
    dir_name: &str,
    body: &PushBody,
) -> Result<(), (StatusCode, String)> {
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

fn require_token(headers: &axum::http::HeaderMap, cfg: &Config) -> Result<(), (StatusCode, String)> {
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

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "name": "cert-receiver",
        "version": env!("CARGO_PKG_VERSION"),
        "layout": state.config.layout,
        "reload_mode": state.config.reload_mode,
    }))
}

async fn push(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<PushBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    require_token(&headers, &state.config)?;

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

    let root = PathBuf::from(&state.config.output_dir);
    ensure_dir(&root)?;

    let mut wrote = Vec::new();
    match state.config.layout.as_str() {
        "cert" => {
            write_layout(&root, &safe_name(&cert_id), &body)?;
            wrote.push(safe_name(&cert_id));
        }
        "domain" => {
            if !domain.is_empty() {
                write_layout(&root, &safe_name(&domain), &body)?;
                wrote.push(safe_name(&domain));
            }
        }
        "both" => {
            write_layout(&root, &safe_name(&cert_id), &body)?;
            wrote.push(safe_name(&cert_id));
            if !domain.is_empty() && safe_name(&domain) != safe_name(&cert_id) {
                write_layout(&root, &safe_name(&domain), &body)?;
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
            write_layout(&root, &safe_name(&cert_id), &body)?;
            wrote.push(name);
        }
        other => return Err((StatusCode::BAD_REQUEST, format!("未知 layout: {}", other))),
    }

    let reload = run_reload(&state.config).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("证书已写入，但重载失败: {}", e),
        )
    })?;

    Ok(Json(json!({
        "ok": true,
        "certificateId": cert_id,
        "written": wrote,
        "reload": reload,
    })))
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
    State(state): State<AppState>,
    Query(q): Query<ApiQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let cfg = &state.config;
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

    // 与 ohttps 相同：apiId/certificateId/timestamp/apiKey 一起按字母序排序拼接
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
        return Err((StatusCode::UNAUTHORIZED, "签名校验失败".into()));
    }
    let _ = q.api_key; // 请求里可以不带 apiKey，只参与签名

    let root = PathBuf::from(&cfg.output_dir);
    let dir = find_cert_dir(&root, &q.certificate_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("证书 {} 不存在", q.certificate_id)))?;
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

#[tokio::main]
async fn main() {
    let config = load_config().unwrap_or_else(|e| {
        eprintln!("[cert-receiver] 启动失败: {}", e);
        std::process::exit(1);
    });
    if config.listen_addr.starts_with("0.0.0.0") {
        eprintln!("[cert-receiver] ⚠ 监听 0.0.0.0，请注意防火墙与 token 强度");
    }
    let state = AppState {
        config: Arc::new(config.clone()),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/push", post(push))
        .route("/api/open/getCertificate", get(get_certificate))
        .with_state(state);

    let addr: std::net::SocketAddr = config.listen_addr.parse().unwrap_or_else(|_| {
        eprintln!("[cert-receiver] listen_addr 解析失败: {}", config.listen_addr);
        std::process::exit(1);
    });
    println!(
        "[cert-receiver] 监听 {}，输出目录 {}，布局 {}，重载 {}",
        addr, config.output_dir, config.layout, config.reload_mode
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("[cert-receiver] 绑定 {} 失败: {}", addr, e);
            std::process::exit(1);
        });
    axum::serve(listener, app).await.expect("服务异常");
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
