use std::sync::Arc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::{Mutex, oneshot};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tauri::State;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct RunningServer {
    pub port: u16,
    pub path: String,
}

pub struct ServerInfo {
    pub path: String,
    pub stop_tx: oneshot::Sender<()>,
}

pub struct HttpServerState {
    pub servers: Arc<Mutex<HashMap<u16, ServerInfo>>>,
}

impl Default for HttpServerState {
    fn default() -> Self {
        Self {
            servers: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn url_decode(s: &str) -> Result<String, ()> {
    let mut decoded = Vec::new();
    let mut bytes = s.as_bytes().iter();
    while let Some(&b) = bytes.next() {
        if b == b'%' {
            let mut hex = Vec::new();
            if let Some(&h1) = bytes.next() { hex.push(h1); }
            if let Some(&h2) = bytes.next() { hex.push(h2); }
            if let Ok(hex_str) = std::str::from_utf8(&hex) {
                if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                    decoded.push(val);
                    continue;
                }
            }
            return Err(());
        } else {
            decoded.push(b);
        }
    }
    String::from_utf8(decoded).map_err(|_| ())
}

fn html_escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
        .replace('"', "&quot;").replace('\'', "&#39;")
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(&mut encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// Resolve a URL path inside an already canonical root. Check the actual target,
/// so a junction or symlink cannot turn a valid lexical prefix into an escape.
fn resolve_request_path(root: &Path, raw: &str) -> Result<PathBuf, u16> {
    let decoded = url_decode(raw.split('?').next().unwrap_or(raw)).map_err(|_| 400u16)?;
    if !decoded.starts_with('/') || decoded.contains(['\\', ':', '\0']) {
        return Err(403);
    }
    let mut candidate = root.to_path_buf();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => return Err(403),
            name => candidate.push(name),
        }
    }
    let canonical = candidate.canonicalize().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { 404u16 } else { 403u16 }
    })?;
    if !canonical.starts_with(root) { return Err(403); }
    Ok(canonical)
}

async fn respond_error(stream: &mut TcpStream, status: u16) {
    let reason = match status {
        400 => "Bad Request", 403 => "Forbidden", 404 => "Not Found",
        408 => "Request Timeout", 414 => "URI Too Long", _ => "Internal Server Error",
    };
    let _ = stream.write_all(format!(
        "HTTP/1.1 {status} {reason}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    ).as_bytes()).await;
}

fn get_mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()) {
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("xml") => "application/xml",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn serve_file(stream: &mut TcpStream, file_path: &Path, head_only: bool) {
    let mut file = match tokio::fs::File::open(file_path).await {
        Ok(f) => f,
        Err(_) => {
            let body = "<h1>500 Internal Server Error</h1>";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            return;
        }
    };

    let metadata = match file.metadata().await {
        Ok(m) => m,
        Err(_) => {
            let body = "<h1>500 Internal Server Error</h1>";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            let _ = stream.write_all(response.as_bytes()).await;
            return;
        }
    };

    let file_len = metadata.len();
    let mime = get_mime_type(file_path);

    let headers = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {}\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Connection: close\r\n\r\n",
        mime, file_len
    );

    if stream.write_all(headers.as_bytes()).await.is_err() {
        return;
    }

    if !head_only {
        let mut buffer = [0; 8192];
        loop {
            match file.read(&mut buffer).await {
                Ok(0) => break,
                Ok(n) => {
                    if stream.write_all(&buffer[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }
}

async fn serve_dir_list(stream: &mut TcpStream, dir: &Path, root: &Path, head_only: bool) {
    let mut html = String::new();
    html.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Index of ");
    let display_path = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy().to_string();
    let display_path = if display_path.is_empty() { "/".to_string() } else { format!("/{}", display_path.replace('\\', "/")) };
    
    html.push_str(&html_escape(&display_path));
    html.push_str("</title><style>\
        body { font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, sans-serif; background-color: #0f172a; color: #cbd5e1; padding: 40px; margin: 0; }\
        .container { max-width: 800px; margin: 0 auto; background: rgba(30, 41, 59, 0.5); border: 1px solid rgba(255, 255, 255, 0.05); padding: 30px; border-radius: 16px; box-shadow: 0 4px 30px rgba(0,0,0,0.3); backdrop-filter: blur(10px); }\
        h1 { font-size: 24px; font-weight: 600; color: #f8fafc; margin-top: 0; margin-bottom: 20px; border-bottom: 1px solid rgba(255,255,255,0.08); padding-bottom: 10px; }\
        ul { list-style-type: none; padding: 0; margin: 0; }\
        li { padding: 8px 12px; display: flex; align-items: center; border-radius: 8px; transition: background 0.2s; }\
        li:hover { background: rgba(255,255,255,0.03); }\
        a { text-decoration: none; color: #38bdf8; font-weight: 500; font-size: 14px; flex-grow: 1; display: flex; align-items: center; gap: 8px; }\
        a:hover { color: #7dd3fc; }\
        .icon { font-size: 16px; }\
        footer { margin-top: 30px; border-top: 1px solid rgba(255,255,255,0.08); padding-top: 15px; font-size: 11px; color: #64748b; font-family: monospace; text-align: right; }\
    </style></head><body><div class=\"container\">");
    
    html.push_str(&format!("<h1>Index of {}</h1><ul>", html_escape(&display_path)));
    
    // Add parent link if not root
    if dir != root {
        html.push_str("<li><a href=\"..\"><span class=\"icon\">📁</span> .. (Parent Directory)</a></li>");
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(metadata) = entry.metadata() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if metadata.is_dir() {
                    dirs.push(name);
                } else {
                    files.push(name);
                }
            }
        }
        dirs.sort();
        files.sort();

        for d in dirs {
            html.push_str(&format!("<li><a href=\"{}/\"><span class=\"icon\">📁</span> {}/</a></li>", encode_path_segment(&d), html_escape(&d)));
        }
        for f in files {
            html.push_str(&format!("<li><a href=\"{}\"><span class=\"icon\">📄</span> {}</a></li>", encode_path_segment(&f), html_escape(&f)));
        }
    }
    
    html.push_str("</ul><footer>Served by Kira HTTP Server</footer></div></body></html>");

    let response = if head_only {
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Connection: close\r\n\r\n",
            html.len()
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Connection: close\r\n\r\n\
             {}",
            html.len(),
            html
        )
    };

    let _ = stream.write_all(response.as_bytes()).await;
}

async fn handle_connection(mut stream: TcpStream, root_dir: PathBuf) {
    // TCP reads are fragments, not complete HTTP requests. Bound both the request
    // line and time spent waiting for it to avoid incomplete-path interpretation.
    let mut request = Vec::new();
    let read_line = async {
        let mut buffer = [0; 1024];
        loop {
            let n = stream.read(&mut buffer).await.map_err(|_| 400u16)?;
            if n == 0 { return Err(400); }
            request.extend_from_slice(&buffer[..n]);
            if let Some(end) = request.windows(2).position(|w| w == b"\r\n") {
                if end > 8192 { return Err(414); }
                request.truncate(end);
                return Ok(());
            }
            if request.len() > 8192 { return Err(414); }
        }
    };
    match tokio::time::timeout(std::time::Duration::from_secs(5), read_line).await {
        Ok(Ok(())) => {}
        result => { respond_error(&mut stream, result.ok().and_then(Result::err).unwrap_or(408)).await; return; }
    }
    let request_str = match std::str::from_utf8(&request) {
        Ok(value) => value,
        Err(_) => { respond_error(&mut stream, 400).await; return; }
    };
    let mut lines = request_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return,
    };

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() != 3 || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
        respond_error(&mut stream, 400).await;
        return;
    }

    let method = parts[0];
    let raw_path = parts[1];

    if method != "GET" && method != "HEAD" {
        let response = "HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(response.as_bytes()).await;
        return;
    }

    let safe_path = match resolve_request_path(&root_dir, raw_path) {
        Ok(path) => path,
        Err(status) => { respond_error(&mut stream, status).await; return; }
    };

    if safe_path.is_dir() {
        let index_html = safe_path.join("index.html");
        let index_htm = safe_path.join("index.htm");
        if index_html.is_file() || index_htm.is_file() {
            let index = if index_html.is_file() { index_html } else { index_htm };
            match index.canonicalize() {
                Ok(index) if index.starts_with(&root_dir) => serve_file(&mut stream, &index, method == "HEAD").await,
                _ => respond_error(&mut stream, 403).await,
            }
        } else {
            serve_dir_list(&mut stream, &safe_path, &root_dir, method == "HEAD").await;
        }
    } else if safe_path.is_file() {
        serve_file(&mut stream, &safe_path, method == "HEAD").await;
    } else {
        let response_body = "<h1>404 Not Found</h1>";
        let response = format!(
            "HTTP/1.1 404 Not Found\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Connection: close\r\n\r\n\
             {}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }
}

#[tauri::command]
pub async fn start_http_server(
    app: tauri::AppHandle,
    state: State<'_, HttpServerState>,
    path: String,
    port: u16,
    // 绑定的监听地址：默认仅本地回环 127.0.0.1；仅当用户显式选择“允许局域网访问”时传 "0.0.0.0"。
    // 切勿默认暴露到所有网络接口，否则会把本地目录共享给整个局域网/公网。
    host: Option<String>,
) -> Result<String, String> {
    let mut servers = state.servers.lock().await;
    if servers.contains_key(&port) {
        return Err(format!("端口 {} 已在服务中", port));
    }

    // 启动前端口占用预检（Q-0095）：明确给出占用者，而不是笼统的"绑定端口失败"
    if let Some(owner) = crate::commands::utils::port_conflict_description(port) {
        return Err(format!(
            "端口 {} 已被 {} 占用，请更换端口或结束占用进程后重试",
            port, owner
        ));
    }

    let root_dir = PathBuf::from(&path).canonicalize().map_err(|_| "所选目录不存在".to_string())?;
    if !root_dir.is_dir() { return Err("请选择目录".to_string()); }

    // 默认仅监听本地回环，避免把目录暴露到局域网/公网
    let bind_host = host.filter(|h| h == "0.0.0.0").unwrap_or_else(|| "127.0.0.1".to_string());
    let listener = match TcpListener::bind(format!("{}:{}", bind_host, port)).await {
        Ok(l) => l,
        Err(e) => return Err(format!("绑定端口失败: {}", e)),
    };

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    
    servers.insert(port, ServerInfo {
        path: path.clone(),
        stop_tx,
    });

    let servers_clone = state.servers.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => {
                    break;
                }
                accept_res = listener.accept() => {
                    match accept_res {
                        Ok((stream, _)) => {
                            let root = root_dir.clone();
                            tokio::spawn(async move {
                                handle_connection(stream, root).await;
                            });
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        let mut map = servers_clone.lock().await;
        map.remove(&port);
    });

    // 记住最近一次参数，供托盘「启动上次目录」使用
    crate::commands::config::remember_last_server(
        "http",
        serde_json::json!({ "path": path, "port": port, "host": bind_host }),
    );
    let _ = crate::tray::rebuild_tray_menu(&app);

    Ok(format!("http://localhost:{}", port))
}

/// 供托盘复用：按最近一次记录的参数启动 HTTP 静态服务
pub async fn start_http_server_inner(
    servers: Arc<Mutex<HashMap<u16, ServerInfo>>>,
    path: String,
    port: u16,
    host: Option<String>,
) -> Result<String, String> {
    {
        let map = servers.lock().await;
        if map.contains_key(&port) {
            return Err(format!("端口 {} 已在服务中", port));
        }
    }
    let root_dir = PathBuf::from(&path).canonicalize().map_err(|_| "所选目录不存在".to_string())?;
    if !root_dir.is_dir() { return Err("请选择目录".to_string()); }
    let bind_host = host
        .filter(|h| h == "0.0.0.0")
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let listener = TcpListener::bind(format!("{}:{}", bind_host, port))
        .await
        .map_err(|e| format!("绑定端口失败: {}", e))?;
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    servers.lock().await.insert(
        port,
        ServerInfo {
            path: path.clone(),
            stop_tx,
        },
    );
    let servers_clone = servers.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                accept_res = listener.accept() => {
                    if let Ok((stream, _)) = accept_res {
                        let root = root_dir.clone();
                        tokio::spawn(async move { handle_connection(stream, root).await; });
                    }
                }
            }
        }
        servers_clone.lock().await.remove(&port);
    });
    Ok(format!("http://localhost:{}", port))
}

#[tauri::command]
pub async fn stop_http_server(
    app: tauri::AppHandle,
    state: State<'_, HttpServerState>,
    port: u16,
) -> Result<(), String> {
    let mut servers = state.servers.lock().await;
    if let Some(info) = servers.remove(&port) {
        let _ = info.stop_tx.send(());
        drop(servers);
        let _ = crate::tray::rebuild_tray_menu(&app);
        Ok(())
    } else {
        Err(format!("未找到在端口 {} 上运行的服务", port))
    }
}

#[tauri::command]
pub async fn get_running_http_servers(
    state: State<'_, HttpServerState>,
) -> Result<Vec<RunningServer>, String> {
    let servers = state.servers.lock().await;
    Ok(servers.iter().map(|(&port, info)| RunningServer {
        port,
        path: info.path.clone(),
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("kira-http-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for TestRoot { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }

    #[test]
    fn request_paths_reject_windows_prefixes_and_traversal() {
        let root = TestRoot::new();
        for path in ["/..%5Csecret", "/../secret", "/%2e%2e/secret", "/C:secret", "/%00secret", "\\secret"] {
            assert_eq!(resolve_request_path(&root.0, path), Err(403), "{path}");
        }
        assert_eq!(resolve_request_path(&root.0, "/%ZZ"), Err(400));
        assert_eq!(resolve_request_path(&root.0, "/%FF"), Err(400));
    }

    #[test]
    fn filename_encoding_roundtrips_and_escapes_html() {
        let root = TestRoot::new();
        let name = "a+b#%&中文.txt";
        std::fs::write(root.0.join(name), "contents").unwrap();
        assert_eq!(url_decode("a+b").unwrap(), "a+b");
        let encoded = encode_path_segment(name);
        assert!(encoded.contains("%2B") && encoded.contains("%23") && encoded.contains("%25"));
        assert_eq!(resolve_request_path(&root.0, &format!("/{encoded}")), Ok(root.0.join(name)));
        assert_eq!(html_escape("<&\"'"), "&lt;&amp;&quot;&#39;");
    }

    #[tokio::test]
    async fn fragmented_request_waits_for_complete_line() {
        let root = TestRoot::new();
        std::fs::write(root.0.join("a+b.txt"), "right file").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let path = root.0.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_connection(stream, path).await;
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET /a+").await.unwrap();
        tokio::task::yield_now().await;
        client.write_all(b"b.txt HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        server.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.ends_with("right file"));
    }

    #[cfg(windows)]
    #[test]
    fn junction_target_outside_root_is_rejected() {
        let root = TestRoot::new();
        let outside = TestRoot::new();
        let link = root.0.join("outside");
        // Directory symlinks need developer mode/privilege; junctions do not.
        let result = crate::commands::hidden_cmd::hidden_cmd("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link).arg(&outside.0).output().unwrap();
        assert!(result.status.success());
        assert_eq!(resolve_request_path(&root.0, "/outside"), Err(403));
        std::fs::remove_dir(link).unwrap();
    }
}
