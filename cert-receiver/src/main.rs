//! AnyVersion 证书管理 — Windows 轻量接收端。
//!
//! 独立运行，监听 HTTP 端口，接收 AnyVersion 推送的证书（PEM），
//! 校验 Bearer token 后写入 output_dir，并可执行 post_script 触发重载。
//!
//! 配置（cert-receiver.toml）：
//!   listen_addr = "0.0.0.0:9000"
//!   token = "YOUR_SHARED_TOKEN"
//!   output_dir = "C:\\certs"
//!   post_script = "powershell -File C:\\certs\\reload.ps1"   # 可选

use axum::{
    extract::State,
    http::{header::AUTHORIZATION, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
struct Config {
    listen_addr: String,
    token: String,
    output_dir: String,
    #[serde(default)]
    post_script: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PushBody {
    domain: String,
    cert: String,
    key: String,
    #[serde(default)]
    ca: String,
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
}

fn load_config() -> Result<Config, String> {
    // 优先读同目录 cert-receiver.toml，否则读 C:\certs\cert-receiver.toml
    let candidates = [
        PathBuf::from("cert-receiver.toml"),
        PathBuf::from("C:\\certs\\cert-receiver.toml"),
    ];
    for p in candidates {
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(c) = toml::from_str::<Config>(&s) {
                // 拒绝使用默认 token
                if c.token.is_empty() || c.token == "changeme" || c.token == "YOUR_SHARED_TOKEN" {
                    return Err(format!(
                        "证书接收端 token 未配置（{}）。请在 cert-receiver.toml 中设置强 token。",
                        p.display()
                    ));
                }
                return Ok(c);
            }
        }
    }
    Err(
        "未找到 cert-receiver.toml 或配置无效。请创建配置文件并设置 token。\n\
         示例:\n\
         listen_addr = \"127.0.0.1:9000\"\n\
         token = \"your-random-token-here\"\n\
         output_dir = \"C:\\\\certs\""
            .to_string(),
    )
}

fn safe_name(domain: &str) -> String {
    // 防止路径遍历：仅保留字母数字、点、连字符、星号
    domain
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '*')
        .collect()
}

async fn health() -> &'static str {
    "ok"
}

async fn push(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<PushBody>,
) -> Result<String, (StatusCode, String)> {
    // 校验 token
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let expected = format!("Bearer {}", state.config.token);
    if auth != expected {
        return Err((StatusCode::UNAUTHORIZED, "invalid token".into()));
    }

    let dir = PathBuf::from(&state.config.output_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("创建目录失败: {}", e)));
    }
    let name = safe_name(&body.domain);
    let write = |fname: &str, content: &str| -> Result<(), (StatusCode, String)> {
        let p = dir.join(fname);
        std::fs::write(&p, content).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("写入 {} 失败: {}", p.display(), e),
            )
        })
    };

    write(&format!("{}.crt", name), &body.cert)?;
    write(&format!("{}.key", name), &body.key)?;
    if !body.ca.is_empty() {
        write(&format!("{}.issuer.crt", name), &body.ca)?;
    }

    // 触发 post_script
    if let Some(script) = &state.config.post_script {
        match Command::new("cmd").args(["/c", script]).output() {
            Ok(o) => {
                if !o.status.success() {
                    let msg = format!(
                        "post_script 返回非零: {}",
                        String::from_utf8_lossy(&o.stderr)
                    );
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
                }
            }
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("执行 post_script 失败: {}", e))),
        }
    }

    Ok(format!("received cert for {}", body.domain))
}

#[tokio::main]
async fn main() {
    let config = load_config().unwrap_or_else(|e| {
        eprintln!("[cert-receiver] 启动失败: {}", e);
        std::process::exit(1);
    });
    // 默认监听本地回环，避免暴露到局域网/公网
    let listen_addr = if config.listen_addr.starts_with("0.0.0.0") {
        eprintln!("[cert-receiver] ⚠ 监听 0.0.0.0，建议改为 127.0.0.1 限制本地访问");
        config.listen_addr.clone()
    } else {
        config.listen_addr.clone()
    };
    let state = AppState {
        config: Arc::new(config),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/push", post(push))
        .with_state(state.clone());

    let addr: std::net::SocketAddr = listen_addr
        .parse()
        .expect("listen_addr 解析失败");
    println!("[cert-receiver] 监听 {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.expect("绑定端口失败");
    axum::serve(listener, app).await.expect("服务异常");
}
