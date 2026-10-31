//! AnyVersion 证书管理 — Windows 独立接收端（托盘小工具）。
//!
//! 与 0.1.x 的区别只在「怎么用」：原来是一个没有界面的控制台服务，配置要手改 toml、
//! 缺配置直接 `exit(1)` 闪退；现在是常驻托盘的小窗口，端口/token/证书目录都在界面上配，
//! 日志实时可见，关窗口只隐藏到托盘。
//!
//! 接口与落盘布局完全没变（`/health`、`/push`、`/api/open/getCertificate`），
//! 配置仍是同一份 `cert-receiver.toml`，命令行 `--config <path>` 依旧有效。

// release 下不要弹黑色控制台窗口（GUI 程序的正常形态）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod lifecycle;
mod server;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

use config::Config;
use server::AppData;

/// 日志逐行推送（前端 listen 同名事件）
pub const LOG_EVENT: &str = "receiver-log";
/// 服务状态推送（运行中/地址/统计）
pub const STATUS_EVENT: &str = "receiver-status";

const LOG_CAP: usize = 500;

#[derive(Default, Clone)]
struct Stats {
    /// 成功写入的次数（被 token 挡掉的不算「收到」）
    pushes: u64,
    /// 失败/被拒的次数
    fails: u64,
    last_push_at: String,
    last_push_ok: bool,
    last_error: String,
}

struct ServerHandle {
    shutdown: Option<oneshot::Sender<()>>,
    completion: Arc<Mutex<std::sync::mpsc::Receiver<()>>>,
    finished: Arc<AtomicBool>,
    addr: SocketAddr,
}

pub struct AppState {
    app: AppHandle,
    cfg_path: Mutex<PathBuf>,
    lifecycle: Mutex<()>,
    cfg: Mutex<Config>,
    server: Mutex<Option<ServerHandle>>,
    stats: Mutex<Stats>,
    logs: Mutex<VecDeque<String>>,
    /// 每一代服务的编号：重启时旧任务收尾晚于新任务启动，靠它避免打错日志
    run_seq: AtomicU64,
}

#[derive(Serialize, Clone)]
pub struct Status {
    running: bool,
    addr: String,
    pushes: u64,
    fails: u64,
    last_push_at: String,
    last_push_ok: bool,
    last_error: String,
}

impl AppState {
    fn new(app: AppHandle, cfg_path: PathBuf, cfg: Config) -> Self {
        Self {
            app,
            cfg_path: Mutex::new(cfg_path),
            lifecycle: Mutex::new(()),
            cfg: Mutex::new(cfg),
            server: Mutex::new(None),
            stats: Mutex::new(Stats::default()),
            logs: Mutex::new(VecDeque::new()),
            run_seq: AtomicU64::new(0),
        }
    }

    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 写一行日志：内存环形缓冲（供界面首次加载）+ 实时事件推送。
    pub fn log(&self, line: impl Into<String>) {
        let line = format!(
            "[{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            line.into()
        );
        {
            let mut logs = self.logs.lock().unwrap_or_else(|e| e.into_inner());
            logs.push_back(line.clone());
            while logs.len() > LOG_CAP {
                logs.pop_front();
            }
        }
        let _ = self.app.emit(LOG_EVENT, line);
    }

    /// 一次推送的结论：更新统计 + 写日志 + 通知界面。
    pub fn record_push(&self, ok: bool, line: String) {
        {
            let mut s = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            if ok {
                s.pushes += 1;
            } else {
                s.fails += 1;
                s.last_error = line.clone();
            }
            s.last_push_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
            s.last_push_ok = ok;
        }
        self.log(line);
        self.emit_status();
    }

    fn status(&self) -> Status {
        let s = self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let addr = self
            .server
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|h| h.addr.to_string())
            .unwrap_or_else(|| self.cfg().listen_addr);
        Status {
            running: self
                .server
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|h| !h.finished.load(Ordering::SeqCst)),
            addr,
            pushes: s.pushes,
            fails: s.fails,
            last_push_at: s.last_push_at,
            last_push_ok: s.last_push_ok,
            last_error: s.last_error,
        }
    }

    fn emit_status(&self) {
        let _ = self.app.emit(STATUS_EVENT, self.status());
    }
}

/* ─── 前端命令 ─── */

#[derive(Serialize)]
struct Bootstrap {
    config: Config,
    status: Status,
    cfg_path: String,
    logs: Vec<String>,
}

#[tauri::command]
fn bootstrap(state: State<'_, Arc<AppState>>) -> Bootstrap {
    Bootstrap {
        config: state.cfg(),
        status: state.status(),
        cfg_path: state
            .cfg_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .display()
            .to_string(),
        logs: state
            .logs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect(),
    }
}

/// 保存配置：校验 → 写盘 → 若服务在跑则用新配置重启（端口/token 改了必须重启才生效）。
#[tauri::command]
async fn save_config(state: State<'_, Arc<AppState>>, config: Config) -> Result<Status, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        state.apply_config(config)?;
        Ok(state.status())
    })
    .await
    .map_err(|e| format!("配置任务异常: {e}"))?
}

#[tauri::command]
async fn set_running(state: State<'_, Arc<AppState>>, running: bool) -> Result<Status, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if running {
            state.start_server()?;
        } else {
            state.stop_server()?;
        }
        Ok(state.status())
    })
    .await
    .map_err(|e| format!("启停任务异常: {e}"))?
}

/// 扫描输出目录里已收到的证书（读每份 meta.json）。
///
/// 用户需要知道「到底收到了哪些、什么时候到期」，也才有东西可选来生成拉取示例。
#[derive(Serialize)]
struct StoredCert {
    /// 落盘目录名（可能就是证书 ID，也可能是域名，取决于 layout）
    dir: String,
    certificate_id: String,
    domains: Vec<String>,
    expired_time: String,
    updated_at_ms: u64,
}

#[tauri::command]
fn list_stored_certs(state: State<'_, Arc<AppState>>) -> Vec<StoredCert> {
    let root = PathBuf::from(state.cfg().output_dir);
    let mut out: Vec<StoredCert> = Vec::new();
    let entries = match std::fs::read_dir(&root) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let meta_path = p.join("meta.json");
        let text = match std::fs::read_to_string(&meta_path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let v: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let certificate_id = v
            .get("certificate_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        if certificate_id.is_empty() {
            continue;
        }
        out.push(StoredCert {
            dir: p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
            certificate_id,
            domains: v
                .get("domains")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            expired_time: v
                .get("expired_time")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            updated_at_ms: v.get("updated_at_ms").and_then(|x| x.as_u64()).unwrap_or(0),
        });
    }
    out.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
    out
}

#[derive(Serialize)]
struct PullExample {
    url: String,
    curl: String,
    /// 签名里用的时间戳（5 分钟内有效，过期要重新生成）
    timestamp: String,
    /// 是否监听在 0.0.0.0：是的话示例里的主机是 127.0.0.1，客户端要换成真实地址
    listen_is_any: bool,
    /// 本机可用 IPv4（出口网卡），让用户知道该填哪个地址
    hosts: Vec<String>,
}

/// 本机出口 IPv4。UDP `connect` 只是让系统选路由，不会真的发包。
fn local_ipv4() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("223.5.5.5:53").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                let ip = addr.ip().to_string();
                if ip != "0.0.0.0" && ip != "127.0.0.1" {
                    out.push(ip);
                }
            }
        }
    }
    out
}

/// 拉取接口的签名：md5(按字母序拼接 apiId/apiKey/certificateId/timestamp)。
///
/// **必须**与 `server.rs` 里 `get_certificate` 的校验保持同一算法 —— 两边一起改，
/// 否则示例看起来正常、实际一律 401。单测用同一组固定向量交叉验证。
fn pull_sign(api_id: &str, api_key: &str, certificate_id: &str, timestamp: &str) -> String {
    let mut params = std::collections::BTreeMap::new();
    params.insert("apiId", api_id);
    params.insert(
        "apiKey",
        cert_deployment::credentials::api_key_for_sign(api_key),
    );
    params.insert("certificateId", certificate_id);
    params.insert("timestamp", timestamp);
    let joined = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");
    format!("{:x}", md5::compute(joined.as_bytes()))
}

/// 只对 URL 里必须转义的字符做最小处理：这里的 id 一般是字母数字和 -_.
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 生成一条可直接交给容器/脚本的拉取命令。
///
/// 签名算法与接收端校验逻辑（`server.rs` 的 `get_certificate`）必须一致：
/// md5(按字母序拼接 apiId/apiKey/certificateId/timestamp)。放后端算是因为
/// 浏览器没有内置 md5，前端自己实现容易和校验端写岔。
#[tauri::command]
fn pull_example(
    state: State<'_, Arc<AppState>>,
    api_id: String,
    certificate_id: String,
) -> Result<PullExample, String> {
    let cfg = state.cfg();
    if cfg.api_key.trim().is_empty() {
        return Err("接收端还没配 API 密钥：先在上面生成/填写并保存，拉取接口才会启用".into());
    }
    if certificate_id.trim().is_empty() {
        return Err("先选一个已收到的证书（列表为空说明还没推送过）".into());
    }
    let api_id = if api_id.trim().is_empty() {
        "kira-pull".to_string()
    } else {
        api_id.trim().to_string()
    };
    let cert_id = certificate_id.trim().to_string();
    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let sign = pull_sign(&api_id, cfg.api_key.trim(), &cert_id, &ts);

    // 监听在 0.0.0.0 时，示例里给 127.0.0.1（0.0.0.0 不是可连接的目标地址），
    // 并由界面提示用户换成服务器实际地址。
    let listen_is_any = cfg.listen_addr.starts_with("0.0.0.0");
    let host = if listen_is_any {
        cfg.listen_addr.replacen("0.0.0.0", "127.0.0.1", 1)
    } else {
        cfg.listen_addr.clone()
    };
    let url = format!(
        "http://{}/api/open/getCertificate?apiId={}&certificateId={}&timestamp={}&sign={}",
        host,
        url_encode(&api_id),
        url_encode(&cert_id),
        ts,
        sign
    );
    Ok(PullExample {
        curl: format!("curl \"{}\"", url),
        url,
        timestamp: ts,
        listen_is_any,
        hosts: local_ipv4(),
    })
}

#[tauri::command]
fn clear_logs(state: State<'_, Arc<AppState>>) -> Vec<String> {
    state.logs.lock().unwrap_or_else(|e| e.into_inner()).clear();
    Vec::new()
}

/// 原生「选择文件夹」对话框（rfd）。返回 None = 用户取消。
#[tauri::command]
fn pick_dir(current: Option<String>) -> Option<String> {
    let mut dialog = rfd::FileDialog::new().set_title("选择证书输出目录");
    if let Some(dir) = current.filter(|d| !d.trim().is_empty()) {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            dialog = dialog.set_directory(p);
        }
    }
    dialog
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn open_dir(state: State<'_, Arc<AppState>>, path: Option<String>) -> Result<(), String> {
    let target = path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| state.cfg().output_dir);
    if target.trim().is_empty() {
        return Err("证书输出目录为空".into());
    }
    let p = PathBuf::from(&target);
    if !p.is_dir() {
        // 目录还没建出来时给个明确提示，而不是让 explorer 自己弹「找不到」
        return Err(format!("目录不存在：{}", target));
    }
    open_path(&target)
}

/// 原生「选择文件」对话框（rfd）：目前用于 reload_mode=nginx 的配置文件。
#[tauri::command]
fn pick_file(current: Option<String>) -> Option<String> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("选择 Nginx 配置文件")
        .add_filter("Nginx 配置", &["conf"]);
    if let Some(dir) = current
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .filter(|d| d.is_dir())
    {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn open_path(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打开 {} 失败：{}", path, e))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("仅支持 Windows".into())
    }
}

/* ─── 窗口 / 托盘 ─── */

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show", "打开窗口", true, None::<&str>)?;
    let open_dir = MenuItem::with_id(app, "open_dir", "打开证书目录", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &open_dir, &quit])?;

    TrayIconBuilder::with_id("main")
        .tooltip("证书接收端")
        .icon(tauri::include_image!("icons/32x32.png"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "open_dir" => {
                let target = app
                    .try_state::<Arc<AppState>>()
                    .map(|s| s.cfg().output_dir)
                    .unwrap_or_default();
                if let Err(e) = open_dir_quiet(&target) {
                    if let Some(st) = app.try_state::<Arc<AppState>>() {
                        st.log(e);
                    }
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn open_dir_quiet(target: &str) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("证书输出目录为空".into());
    }
    let p = PathBuf::from(target);
    if !p.is_dir() {
        return Err(format!("目录不存在：{}", target));
    }
    open_path(target)
}

/* ─── 启动 ─── */

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            save_config,
            set_running,
            clear_logs,
            list_stored_certs,
            pull_example,
            pick_dir,
            pick_file,
            open_dir,
            quit_app
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // 配置：按既有顺序找；一个都没有就在 exe 同目录生成一份带随机 token 的，
            // 这样「双击就能用」，不用先手动复制 example。
            let (cfg, path, note) = match config::find_config_path() {
                Some(p) => match config::load(&p) {
                    Ok(c) => (c, p.clone(), None),
                    Err(e) => (Config::fresh(), p.clone(), Some(e)),
                },
                None => {
                    let p = config::default_config_path();
                    let c = Config::fresh();
                    let saved = config::save(&p, &c).err();
                    (
                        c,
                        p,
                        Some(saved.unwrap_or_else(|| {
                            "首次运行：已生成配置文件，请确认 Token 与证书目录".to_string()
                        })),
                    )
                }
            };

            let state = Arc::new(AppState::new(handle, path.clone(), cfg));
            app.manage(state.clone());

            state.log(format!("配置文件：{}", path.display()));
            if let Some(note) = note {
                state.log(note);
            }

            build_tray(app)?;

            // 配置合法就自动把服务拉起来：托盘工具的预期是「开着就能收」
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = state.start_server() {
                    state.log(format!("自动启动失败：{e}"));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关窗口 = 收进托盘；真要退出走托盘菜单 / 界面上的退出按钮
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                if let Some(st) = window.app_handle().try_state::<Arc<AppState>>() {
                    st.log("窗口已收进托盘（托盘菜单可退出）");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("cert-receiver 启动失败");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 `server::tests::api_sign_matches_ohttps_algorithm` 用同一组向量：
    /// 拉取示例算出的签名必须能被接收端校验通过，否则示例给了也白给。
    #[test]
    fn pull_sign_matches_server_algorithm() {
        let sign = pull_sign(
            "push-1xlpm40g1105n9gz",
            "89066abd0f21b983331f3aca02590ea3",
            "cert-n2x740m2qxg8kwyz",
            "1695743251613",
        );
        assert_eq!(sign, "c58d69d9e59923f16b7dafa45bef208d");
    }

    #[test]
    fn url_encode_escapes_only_when_needed() {
        assert_eq!(url_encode("cert-abc_1.2~"), "cert-abc_1.2~");
        assert_eq!(url_encode("a b/c"), "a%20b%2Fc");
    }
    #[test]
    fn pull_sign_normalizes_api_key_like_request_verification() {
        assert_eq!(
            pull_sign("api", "  isolation-key\r\n", "cert", "123"),
            pull_sign(
                "api",
                cert_deployment::credentials::api_key_for_sign("  isolation-key\r\n"),
                "cert",
                "123"
            )
        );
    }
}
