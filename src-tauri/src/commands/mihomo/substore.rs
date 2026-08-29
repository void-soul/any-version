//! Sub-Store 集成：下载 / 启动后端(node) / 静态前端服务 / 订阅列表
//! 对齐 clash-party src/main/resolve/server.ts

use crate::commands::mihomo::github::download_with_proxy;
use crate::commands::mihomo::MihomoState;
use crate::commands::hidden_cmd::hidden_cmd;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Mutex;
use tauri::State;

static BACKEND: Mutex<Option<Child>> = Mutex::new(None);
static FRONTEND_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn substore_dir(state: &State<'_, MihomoState>) -> PathBuf {
    state.data_dir.join("substore")
}

fn cfg_u16(state: &State<'_, MihomoState>, key: &str, def: u16) -> u16 {
    state
        .app_config
        .lock()
        .unwrap()
        .extra
        .get(key)
        .and_then(|v| v.as_u64())
        .unwrap_or(def as u64) as u16
}

fn cfg_bool(state: &State<'_, MihomoState>, key: &str, def: bool) -> bool {
    state
        .app_config
        .lock()
        .unwrap()
        .extra
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(def)
}

/// 下载 Sub-Store 后端 bundle 与前端 dist
#[tauri::command]
pub async fn mihomo_substore_download(state: State<'_, MihomoState>) -> Result<String, String> {
    let dir = substore_dir(&state);
    let pref = state
        .app_config
        .lock()
        .unwrap()
        .extra
        .get("githubProxy")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let backend = download_with_proxy(
        "https://github.com/sub-store-org/Sub-Store/releases/latest/download/sub-store.bundle.js",
        &pref,
    )
    .await?;
    std::fs::write(dir.join("sub-store.bundle.js"), &backend).map_err(|e| e.to_string())?;

    let front = download_with_proxy(
        "https://github.com/sub-store-org/Sub-Store-Front-End/releases/latest/download/dist.zip",
        &pref,
    )
    .await?;
    let fdir = dir.join("frontend");
    if fdir.exists() {
        std::fs::remove_dir_all(&fdir).ok();
    }
    std::fs::create_dir_all(&fdir).map_err(|e| e.to_string())?;
    let cursor = std::io::Cursor::new(&front);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("解压失败: {e}"))?;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().replace('\\', "/");
        if name.ends_with('/') || name.contains("..") {
            continue;
        }
        let rel = name.strip_prefix("dist/").unwrap_or(&name).to_string();
        let out = fdir.join(&rel);
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).ok();
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok();
        std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
    }
    Ok(format!("Sub-Store 已下载至 {}", dir.display()))
}

fn node_exe() -> String {
    if let Some(p) = crate::commands::utils::bin_tool_path("node") {
        return p.to_string_lossy().to_string();
    }
    "node".to_string()
}

/// 启动 Sub-Store 后端（node 运行 bundle）
#[tauri::command]
pub fn mihomo_substore_start(state: State<'_, MihomoState>) -> Result<Value, String> {
    let dir = substore_dir(&state);
    let bundle = dir.join("sub-store.bundle.js");
    if !bundle.exists() {
        return Err("尚未下载 Sub-Store，请先点击下载".into());
    }
    mihomo_substore_stop()?;
    let port = cfg_u16(&state, "subStoreBackendPort", 38324);
    let use_proxy = cfg_bool(&state, "useProxyInSubStore", false);
    let mixed = state.app_config.lock().unwrap().mixed_port;
    let data_path = dir.join("data");
    std::fs::create_dir_all(&data_path).ok();

    let mut cmd = hidden_cmd(node_exe());
    cmd.arg(&bundle)
        .current_dir(&dir)
        .env("SUB_STORE_BACKEND_API_PORT", port.to_string())
        .env("SUB_STORE_BACKEND_API_HOST", "127.0.0.1")
        .env("SUB_STORE_DATA_BASE_PATH", &data_path)
        .env("SUB_STORE_BACKEND_CUSTOM_NAME", "vex")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if use_proxy {
        let p = format!("http://127.0.0.1:{mixed}");
        cmd.env("HTTP_PROXY", &p)
            .env("HTTPS_PROXY", &p)
            .env("ALL_PROXY", &p);
    }
    let child = crate::commands::hidden_cmd::spawn_breakaway_fallback(cmd)
        .map_err(|e| format!("启动 Sub-Store 失败（需要 node）: {e}"))?;
    *BACKEND.lock().unwrap() = Some(child);

    // 静态前端服务
    let fport = cfg_u16(&state, "subStoreFrontendPort", 38325);
    let fdir = dir.join("frontend");
    if fdir.exists() && !FRONTEND_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        start_static_server(fdir, fport);
    }
    Ok(json!({
        "backendPort": port,
        "frontendPort": fport,
        "url": format!("http://127.0.0.1:{fport}/?api=http://127.0.0.1:{port}"),
    }))
}

#[tauri::command]
pub fn mihomo_substore_stop() -> Result<(), String> {
    if let Some(mut c) = BACKEND.lock().unwrap().take() {
        let _ = c.kill();
    }
    Ok(())
}

#[tauri::command]
pub fn mihomo_substore_status(state: State<'_, MihomoState>) -> Value {
    let dir = substore_dir(&state);
    let running = BACKEND
        .lock()
        .unwrap()
        .as_mut()
        .map(|c| matches!(c.try_wait(), Ok(None)))
        .unwrap_or(false);
    let bport = cfg_u16(&state, "subStoreBackendPort", 38324);
    let fport = cfg_u16(&state, "subStoreFrontendPort", 38325);
    json!({
        "downloaded": dir.join("sub-store.bundle.js").exists(),
        "frontendReady": dir.join("frontend").join("index.html").exists(),
        "running": running,
        "backendPort": bport,
        "frontendPort": fport,
        "url": format!("http://127.0.0.1:{fport}/?api=http://127.0.0.1:{bport}"),
        "dir": dir.to_string_lossy(),
    })
}

async fn substore_get(port: u16, path: &str) -> Result<Value, String> {
    let c = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .no_proxy()
        .build()
        .unwrap_or_default();
    let v: Value = c
        .get(format!("http://127.0.0.1:{port}{path}"))
        .send()
        .await
        .map_err(|e| format!("Sub-Store 未运行或不可达: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(v.get("data").cloned().unwrap_or(v))
}

/// 单条订阅列表
#[tauri::command]
pub async fn mihomo_substore_subs(state: State<'_, MihomoState>) -> Result<Value, String> {
    let port = cfg_u16(&state, "subStoreBackendPort", 38324);
    substore_get(port, "/api/subs").await
}

/// 组合订阅列表
#[tauri::command]
pub async fn mihomo_substore_collections(state: State<'_, MihomoState>) -> Result<Value, String> {
    let port = cfg_u16(&state, "subStoreBackendPort", 38324);
    substore_get(port, "/api/collections").await
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// 极简静态文件服务（SPA fallback 到 index.html）
fn start_static_server(root: PathBuf, port: u16) {
    std::thread::spawn(move || {
        let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[substore] 前端服务启动失败: {e}");
                FRONTEND_STARTED.store(false, std::sync::atomic::Ordering::SeqCst);
                return;
            }
        };
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let root = root.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .split('?')
                    .next()
                    .unwrap_or("/")
                    .to_string();
                let rel = path.trim_start_matches('/');
                // 防路径穿越：`..` 段或任何逃逸 web 根的请求一律回退到 index.html
                let mut file = if rel.is_empty() {
                    root.join("index.html")
                } else if rel.split(['/', '\\']).any(|seg| seg == "..") {
                    root.join("index.html")
                } else {
                    root.join(rel)
                };
                if !file.exists() || file.is_dir() {
                    file = root.join("index.html");
                }
                // 双重保险：canonicalize 后必须仍在 root 之下
                if let (Ok(canon), Ok(root_canon)) = (file.canonicalize(), root.canonicalize()) {
                    if !canon.starts_with(&root_canon) {
                        file = root.join("index.html");
                    }
                }
                let body = std::fs::read(&file).unwrap_or_default();
                // 去掉 Access-Control-Allow-Origin: *（同源前端无需跨域；通配 ACAO 会让任意网页可读本机文件）
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    content_type(&file),
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            });
        }
    });
}
