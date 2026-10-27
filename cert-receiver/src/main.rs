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
mod server;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
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
    shutdown: oneshot::Sender<()>,
    addr: SocketAddr,
}

pub struct AppState {
    app: AppHandle,
    cfg_path: Mutex<PathBuf>,
    cfg: Mutex<Config>,
    server: Mutex<Option<ServerHandle>>,
    stats: Mutex<Stats>,
    logs: Mutex<VecDeque<String>>,
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
            cfg: Mutex::new(cfg),
            server: Mutex::new(None),
            stats: Mutex::new(Stats::default()),
            logs: Mutex::new(VecDeque::new()),
        }
    }

    fn cfg(&self) -> Config {
        self.cfg.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 写一行日志：内存环形缓冲（供界面首次加载）+ 实时事件推送。
    pub fn log(&self, line: impl Into<String>) {
        let line = format!("[{}] {}", chrono::Local::now().format("%H:%M:%S"), line.into());
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
                .is_some(),
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

    /// 启动服务。先同步 `bind`，端口被占用能立刻把错误回给界面，而不是扔进后台任务里。
    fn start_server(self: &Arc<Self>) -> Result<(), String> {
        if self.server.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            return Ok(());
        }
        let cfg = self.cfg();
        if let Err(e) = cfg.validate() {
            self.log(format!("未启动：{}", e));
            return Err(e);
        }
        let addr: SocketAddr = cfg
            .listen_addr
            .parse()
            .map_err(|_| format!("监听地址不合法：{}", cfg.listen_addr))?;

        let std_listener = std::net::TcpListener::bind(addr)
            .map_err(|e| format!("绑定 {} 失败：{}", addr, e))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败：{}", e))?;

        let data = Arc::new(AppData {
            cfg: cfg.clone(),
            state: self.clone(),
        });
        let (tx, rx) = oneshot::channel::<()>();
        let log_state = self.clone();
        tauri::async_runtime::spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(std_listener) {
                Ok(l) => l,
                Err(e) => {
                    log_state.log(format!("监听器转换失败：{}", e));
                    return;
                }
            };
            log_state.log(format!("服务已就绪，监听 {}", addr));
            let app = server::router(data);
            let served = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
            if let Err(e) = served {
                log_state.log(format!("服务异常退出：{}", e));
            }
            log_state.log("服务已停止");
            log_state.emit_status();
        });

        *self.server.lock().unwrap_or_else(|e| e.into_inner()) = Some(ServerHandle { shutdown: tx, addr });
        self.log(format!(
            "服务已启动：{}（布局 {}，重载 {}）",
            addr, cfg.layout, cfg.reload_mode
        ));
        self.emit_status();
        Ok(())
    }

    fn stop_server(&self) {
        let handle = self.server.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = handle {
            let _ = h.shutdown.send(());
            self.log("已请求停止服务");
        }
        self.emit_status();
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
fn save_config(
    state: State<'_, Arc<AppState>>,
    config: Config,
) -> Result<Status, String> {
    config.validate()?;
    let path = state
        .cfg_path
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    config::save(&path, &config)?;
    {
        let mut cur = state.cfg.lock().unwrap_or_else(|e| e.into_inner());
        *cur = config.clone();
    }
    state.log(format!("配置已保存到 {}", path.display()));

    let was_running = state
        .server
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();
    if was_running {
        state.stop_server();
        // 给旧监听器一点时间释放端口，否则同端口重启会 bind 失败
        std::thread::sleep(std::time::Duration::from_millis(150));
        state.start_server()?;
    } else {
        state.emit_status();
    }
    Ok(state.status())
}

#[tauri::command]
fn set_running(state: State<'_, Arc<AppState>>, running: bool) -> Result<Status, String> {
    if running {
        state.start_server()?;
    } else {
        state.stop_server();
    }
    Ok(state.status())
}

#[tauri::command]
fn clear_logs(state: State<'_, Arc<AppState>>) -> Vec<String> {
    state
        .logs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
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
            match state.start_server() {
                Ok(()) => {}
                Err(e) => state.log(format!("自动启动失败：{}", e)),
            }
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
