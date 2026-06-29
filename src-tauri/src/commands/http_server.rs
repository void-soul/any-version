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

fn url_decode(s: &str) -> String {
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
            decoded.push(b'%');
            decoded.extend(hex);
        } else if b == b'+' {
            decoded.push(b' ');
        } else {
            decoded.push(b);
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
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
    
    html.push_str(&display_path);
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
    
    html.push_str(&format!("<h1>Index of {}</h1><ul>", display_path));
    
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
            html.push_str(&format!("<li><a href=\"{}/\"><span class=\"icon\">📁</span> {}/</a></li>", d, d));
        }
        for f in files {
            html.push_str(&format!("<li><a href=\"{}\"><span class=\"icon\">📄</span> {}</a></li>", f, f));
        }
    }
    
    html.push_str("</ul><footer>Served by AnyVersion HTTP Server</footer></div></body></html>");

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
    let mut buffer = [0; 4096];
    let n = match stream.read(&mut buffer).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request_str = String::from_utf8_lossy(&buffer[..n]);
    let mut lines = request_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return,
    };

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let method = parts[0];
    let raw_path = parts[1];

    if method != "GET" && method != "HEAD" {
        let response = "HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(response.as_bytes()).await;
        return;
    }

    // Strip query parameters
    let path_no_query = raw_path.split('?').next().unwrap_or(raw_path);
    let decoded_path = url_decode(path_no_query);

    // Prevent directory traversal
    let mut safe_path = root_dir.clone();
    for component in decoded_path.split('/') {
        if component == ".." {
            if let Some(parent) = safe_path.parent() {
                if parent.starts_with(&root_dir) {
                    safe_path = parent.to_path_buf();
                }
            }
        } else if !component.is_empty() && component != "." {
            safe_path.push(component);
        }
    }

    if !safe_path.starts_with(&root_dir) {
        let response = "HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(response.as_bytes()).await;
        return;
    }

    if safe_path.is_dir() {
        let index_html = safe_path.join("index.html");
        let index_htm = safe_path.join("index.htm");
        if index_html.is_file() {
            serve_file(&mut stream, &index_html, method == "HEAD").await;
        } else if index_htm.is_file() {
            serve_file(&mut stream, &index_htm, method == "HEAD").await;
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
    state: State<'_, HttpServerState>,
    path: String,
    port: u16,
) -> Result<String, String> {
    let mut servers = state.servers.lock().await;
    if servers.contains_key(&port) {
        return Err(format!("端口 {} 已在服务中", port));
    }

    let root_dir = PathBuf::from(&path);
    if !root_dir.exists() {
        return Err("所选目录不存在".to_string());
    }

    let listener = match TcpListener::bind(format!("0.0.0.0:{}", port)).await {
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

    Ok(format!("http://localhost:{}", port))
}

#[tauri::command]
pub async fn stop_http_server(
    state: State<'_, HttpServerState>,
    port: u16,
) -> Result<(), String> {
    let mut servers = state.servers.lock().await;
    if let Some(info) = servers.remove(&port) {
        let _ = info.stop_tx.send(());
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
