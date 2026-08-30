use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 允许从低权限进程（如 Explorer）拖放文件到以管理员身份运行的本程序窗口。
/// Windows UIPI 默认阻止低权限进程向高权限窗口发送 WM_DROPFILES / WM_COPYDATA 消息，
/// 导致管理员模式下无法从外部拖放文件到启动模块的分类。
#[cfg(target_os = "windows")]
/// UIPI 放行：让低权限进程（explorer.exe）能向本进程窗口发送拖放/OLE 相关消息。
/// 覆盖 OLE 拖放（wry 的 IDropTarget）与 DragAcceptFiles 两条路径所需的消息：
/// WM_DROPFILES / WM_COPYDATA / WM_COPYGLOBALDATA / WM_GETOBJECT / WM_QUERYDRAGICON。
/// 同时用进程级 ChangeWindowMessageFilter 与窗口级 ChangeWindowMessageFilterEx
/// 双保险（对每个窗口含 WebView2 子窗口），不依赖窗口标题查找。
fn allow_uipi_drop() {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ChangeWindowMessageFilter, ChangeWindowMessageFilterEx, EnumChildWindows, EnumWindows,
        GetWindowThreadProcessId, MSGFLT_ADD, MSGFLT_ALLOW, WM_COPYDATA, WM_DROPFILES,
        WM_GETOBJECT, WM_QUERYDRAGICON,
    };
    // OLE 拖放（wry 的 IDropTarget）与 DragAcceptFiles 两条路径所需的消息：
    // WM_DROPFILES(0x0233) / WM_COPYDATA(0x004A) / WM_COPYGLOBALDATA(0x0049) /
    // WM_GETOBJECT(0x003D) / WM_QUERYDRAGICON(0x0037)
    const MESSAGES: [u32; 5] = [WM_DROPFILES, WM_COPYDATA, 0x0049, WM_GETOBJECT, WM_QUERYDRAGICON];

    unsafe extern "system" fn allow_on(hwnd: HWND) {
        for &m in &MESSAGES {
            ChangeWindowMessageFilterEx(hwnd, m, MSGFLT_ALLOW, std::ptr::null_mut());
        }
    }
    unsafe extern "system" fn child_cb(hwnd: HWND, _lparam: LPARAM) -> BOOL {
        allow_on(hwnd);
        1
    }
    unsafe extern "system" fn top_cb(hwnd: HWND, _lparam: LPARAM) -> BOOL {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() {
            allow_on(hwnd);
            // WebView2 控件窗口（Chrome_WidgetWin_0 等）是子窗口，递归放行
            EnumChildWindows(hwnd, Some(child_cb), 0);
        }
        1
    }

    unsafe {
        // 进程级：放行后本进程所有窗口都能接收低权限进程（explorer.exe）的消息
        for &m in &MESSAGES {
            ChangeWindowMessageFilter(m, MSGFLT_ADD);
        }
        // 窗口级双保险：对每个顶层窗口及其子窗口逐个放行
        EnumWindows(Some(top_cb), 0);
    }
    crate::exit_log::exit_log(
        "[tray] UIPI: 已放行拖放相关消息（WM_DROPFILES/COPYDATA/COPYGLOBALDATA/GETOBJECT/QUERYDRAGICON，进程级+窗口级）",
    );
}

#[cfg(not(target_os = "windows"))]
fn allow_uipi_drop() {}

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview, WebviewUrl, WebviewWindowBuilder};

const MAIN_WINDOW_LABEL: &str = "main";
const MAIN_WINDOW_TITLE: &str = "Kira 开发助理";
/// 前端「启动」模块的 PageId。主全局热键/托盘恢复/程序启动时打开它。
const LAUNCHER_MODULE: &str = "launcher";
const MAIN_WINDOW_WIDTH: f64 = 1150.0;
const MAIN_WINDOW_HEIGHT: f64 = 780.0;

const TRAY_ID: &str = "main-tray";
const ID_SHOW: &str = "show";
const ID_QUIT: &str = "quit";
const ID_VEX_GREETING: &str = "vex-greeting";
const ID_EMPTY: &str = "__empty";
const ID_SWITCH_PREFIX: &str = "switch::";
const ID_SERVICE_START_PREFIX: &str = "service-start::";
const ID_SERVICE_STOP_PREFIX: &str = "service-stop::";
// ---- Mihomo ----
const ID_MIHOMO_TOGGLE: &str = "mihomo-toggle";

/// 托盘右键菜单是否正处于打开状态。打开期间禁止重建菜单——
/// `tray.set_menu` 会直接关闭正在显示的菜单（表现为“打开几秒后自动消失”）。
static TRAY_MENU_OPEN: AtomicBool = AtomicBool::new(false);

/// 全局保存的 AppHandle，供后台线程（如服务状态快照刷新）触发托盘菜单重建。
static GLOBAL_APP: OnceLock<AppHandle> = OnceLock::new();

/// 正在启动中的服务 id 集合。用于托盘菜单展示「⋯ 启动中」置灰态并防重复启动。
static STARTING_SERVICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn starting_services_contains(id: &str) -> bool {
    STARTING_SERVICES.lock().map(|g| g.iter().any(|s| s == id)).unwrap_or(false)
}

fn starting_services_push(id: String) {
    if let Ok(mut g) = STARTING_SERVICES.lock() {
        if !g.contains(&id) {
            g.push(id);
        }
    }
}

fn starting_services_remove(id: &str) {
    if let Ok(mut g) = STARTING_SERVICES.lock() {
        g.retain(|s| s != id);
    }
}

/// 托盘重建节流。
///
/// 根因：rebuild_tray_menu 每次都会 `set_menu` 完整重建 Win32 原生菜单（HMENU）。
/// mihomo watchdog（每 3 秒一次）与服务状态快照刷新（缓存过期即触发）叠加，会导致
/// 托盘菜单几乎每 3 秒被整体替换一次。长时间运行（几百~几千次重建）后，Win32 菜单
/// 句柄反复创建/销毁使事件回传链失效，表现为「菜单能弹出但点击无响应」（on_menu_event
/// 失效），而 on_tray_icon_event 走独立路径始终正常。
/// 因此对 rebuild 做节流合并：最小间隔内多次请求只真正 set_menu 一次。
static REBUILD_LAST: AtomicU64 = AtomicU64::new(0);
static REBUILD_PENDING: AtomicBool = AtomicBool::new(false);
/// 最小重建间隔（毫秒）。在此间隔内的高频请求会被合并。
const REBUILD_MIN_INTERVAL_MS: u64 = 1500;

/// 后台线程可调用的全局托盘菜单重建入口（无 AppHandle 时静默跳过）。
pub(crate) fn rebuild_tray_menu_global() -> tauri::Result<()> {
    if let Some(app) = GLOBAL_APP.get() {
        rebuild_tray_menu(app)
    } else {
        Ok(())
    }
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let _ = GLOBAL_APP.set(app.clone());
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Kira")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                match (button, button_state) {
                    (MouseButton::Left, MouseButtonState::Up) => {
                        crate::exit_log::exit_log("[tray] 左键点击图标事件");
                        // 简化行为：无论当前状态如何，总是打开并聚焦主窗口
                        show_main_window(tray.app_handle());
                    }
                    // 右键即将弹出菜单：标记打开，避免重建导致菜单闪退
                    (MouseButton::Right, MouseButtonState::Up) => {
                        crate::exit_log::exit_log(&format!(
                            "[tray] 右键点击图标事件（当前 TRAY_MENU_OPEN={}）",
                            TRAY_MENU_OPEN.load(Ordering::SeqCst)
                        ));
                        // 消费待处理的重建请求（节流合并期间积压的），保证弹出的菜单是最新的。
                        // 注意：必须遵守最小重建间隔，否则连续右键会立即重建菜单句柄，
                        // 破坏 Win32 菜单事件链（on_menu_event 失效），这正是当初加节流的原因。
                        if REBUILD_PENDING.load(Ordering::SeqCst) {
                            REBUILD_PENDING.store(false, Ordering::SeqCst);
                            let now = now_ms();
                            let last = REBUILD_LAST.load(Ordering::SeqCst);
                            if now.saturating_sub(last) >= REBUILD_MIN_INTERVAL_MS {
                                let handle = tray.app_handle();
                                if let Some(t) = handle.tray_by_id(TRAY_ID) {
                                    if let Ok(menu) = build_menu(handle) {
                                        let _ = t.set_menu(Some(menu));
                                        REBUILD_LAST.store(now_ms(), Ordering::SeqCst);
                                    }
                                }
                            }
                        }
                        mark_tray_menu_open();
                    }
                    _ => {}
                }
            }
        })
        .on_menu_event(|app, event| {
            crate::exit_log::exit_log(&format!("[tray] on_menu_event 触发 id=\"{}\"", event.id.as_ref()));
            // 选中菜单项即意味着菜单即将关闭，解除“打开中”标记，允许后续重建
            TRAY_MENU_OPEN.store(false, Ordering::SeqCst);
            let id = event.id.as_ref();
            match id {
                ID_SHOW => show_main_window(app),
                ID_QUIT => {
                    crate::exit_log::exit_log("tray 退出菜单点击: 开始退出流程");
                    crate::USER_QUIT_REQUESTED.store(true, Ordering::SeqCst);
                    // 兜底强杀：即使 app.exit(0) 因后台 async 任务（scheduler/watchdog）
                    // 卡在 Tauri 内部优雅关闭流程中，也能在短暂宽限后强制退出进程，
                    // 彻底解决「开启 mihomo 长时间后托盘退出无响应」。
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        crate::exit_log::exit_log("tray 兜底: 500ms 后强制 std::process::exit(0)");
                        std::process::exit(0);
                    });
                    crate::exit_log::exit_log("tray: 调用 app.exit(0)（同步，可能阻塞直到 runtime 关闭）");
                    app.exit(0);
                    // 正常情况下 app.exit(0) 不会返回到此处（runtime 已关闭）；
                    // 若返回说明退出路径异常，记录以便排查。
                    crate::exit_log::exit_log("tray: app.exit(0) 意外返回（异常）");
                }
                other if other.starts_with(ID_SWITCH_PREFIX) => {
                    crate::exit_log::exit_log(&format!("[tray] switch 菜单点击 id=\"{}\"", other));
                    match parse_switch_id(other) {
                        Some((project_id, version)) => {
                            crate::exit_log::exit_log(&format!(
                                "[tray] 解析到 project={} version={}", project_id, version
                            ));
                            match crate::commands::project::versions::project_use_version_inner(
                                project_id,
                                version,
                            ) {
                                Ok(()) => {
                                    crate::exit_log::exit_log(&format!(
                                        "[tray] 切换成功 project={} version={}", project_id, version
                                    ));
                                    let _ = rebuild_tray_menu(app);
                                }
                                Err(e) => {
                                    crate::exit_log::exit_log(&format!(
                                        "[tray] 切换失败 project={} version={} err={}",
                                        project_id, version, e
                                    ));
                                }
                            }
                        }
                        None => {
                            crate::exit_log::exit_log(&format!(
                                "[tray] parse_switch_id 解析失败 id=\"{}\"", other
                            ));
                        }
                    }
                }
                other if other.starts_with(ID_SERVICE_START_PREFIX) => {
                    if let Some(service_id) = other.strip_prefix(ID_SERVICE_START_PREFIX) {
                        // 防重复启动：已在启动中则忽略本次点击
                        if starting_services_contains(service_id) {
                            crate::exit_log::exit_log(&format!(
                                "[tray] 服务 {} 正在启动中，忽略重复点击", service_id
                            ));
                        } else {
                            starting_services_push(service_id.to_string());
                            // 立即重建菜单，把该项显示为「⋯ 启动中」并置灰
                            let _ = rebuild_tray_menu(app);
                            let app2 = app.clone();
                            let sid = service_id.to_string();
                            // 同步启动函数较耗时（内部轮询等待服务就绪），放到后台线程执行，
                            // 避免阻塞托盘菜单事件处理；完成后移除启动中状态并重建菜单。
                            std::thread::spawn(move || {
                                let result = crate::commands::service::start_service_inner(
                                    sid.clone(),
                                    None,
                                );
                                if let Err(error) = &result {
                                    eprintln!("failed to start service {sid}: {error}");
                                    crate::exit_log::exit_log(&format!(
                                        "[tray] 服务 {} 启动失败: {}", sid, error
                                    ));
                                }
                                starting_services_remove(&sid);
                                let _ = rebuild_tray_menu(&app2);
                            });
                        }
                    }
                }
                other if other.starts_with(ID_SERVICE_STOP_PREFIX) => {
                    if let Some(service_id) = other.strip_prefix(ID_SERVICE_STOP_PREFIX) {
                        if let Err(error) = crate::commands::service::stop_service_inner(service_id.to_string()) {
                            eprintln!("failed to stop service {service_id}: {error}");
                        }
                        let _ = rebuild_tray_menu(app);
                    }
                }
                other => handle_extra_menu(app, other),
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

/// 计划提醒红点是否开启（优先级高于状态光环：提醒不能丢）。
static PLAN_BADGE_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 在托盘图标右上角叠加状态光环；color=None 恢复原始图标。
/// 颜色语义（vex 状态灯）：红=提醒，绿=服务运行中，琥珀=忙碌（下载/安装），青=待命。
/// 计划提醒红点（PLAN_BADGE_ON）优先级最高：开启时无论请求什么颜色都保持红色。
pub(crate) fn set_tray_status_badge(app: &AppHandle, color: Option<(u8, u8, u8)>) -> Result<(), String> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return Ok(()) };
    let Some(base) = app.default_window_icon() else { return Ok(()) };
    let mut img = image::RgbaImage::from_raw(base.width(), base.height(), base.rgba().to_vec())
        .ok_or_else(|| "读取默认图标失败".to_string())?;
    // 提醒优先：计划红点开着时，状态光环让位
    let color = if PLAN_BADGE_ON.load(std::sync::atomic::Ordering::SeqCst) {
        Some((255, 59, 48))
    } else {
        color
    };
    if let Some((r, g, b)) = color {
        let dot = ((img.width() as f32) * 0.30).round().max(6.0) as i64;
        let cx = img.width() as i64 - dot / 2 - 2;
        let cy = dot / 2 + 2;
        let rad = dot / 2;
        for y in 0..img.height() {
            for x in 0..img.width() {
                let dx = x as i64 - cx;
                let dy = y as i64 - cy;
                if dx * dx + dy * dy <= rad * rad {
                    img.put_pixel(x, y, image::Rgba([r, g, b, 255]));
                }
            }
        }
    }
    let (w, h) = (img.width(), img.height());
    let icon = tauri::image::Image::new_owned(img.into_raw(), w, h);
    tray.set_icon(Some(icon)).map_err(|e| format!("更新托盘图标失败: {}", e))?;
    Ok(())
}

/// 在托盘图标右上角叠加红点（计划提醒徽标）；show=false 恢复原始图标。
/// 兼容旧调用，语义不变。
pub(crate) fn set_tray_badge(app: &AppHandle, show: bool) -> Result<(), String> {
    PLAN_BADGE_ON.store(show, std::sync::atomic::Ordering::SeqCst);
    set_tray_status_badge(app, if show { Some((255, 59, 48)) } else { None })
}

pub fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    // 菜单打开期间不要重建，否则会关闭正在显示的右键菜单
    if TRAY_MENU_OPEN.load(Ordering::SeqCst) {
        REBUILD_PENDING.store(true, Ordering::SeqCst);
        return Ok(());
    }
    let now = now_ms();
    let last = REBUILD_LAST.load(Ordering::SeqCst);
    if now.saturating_sub(last) < REBUILD_MIN_INTERVAL_MS {
        // 距上次重建过近：合并本次请求，由后续（间隔已过）的调用携带。
        // 关键修复：被节流合并的请求若仅依赖「下次有人再调 rebuild_tray_menu」来
        // 触发，启动阶段可能因时序导致该次重建被永久丢弃（托盘一直停在首次空快照的
        // “待启动”状态）。这里补一个一次性兜底定时器，保证 PENDING 必定在间隔后被刷新。
        if REBUILD_PENDING.load(Ordering::SeqCst) {
            return Ok(()); // 已排队，定时器会处理
        }
        REBUILD_PENDING.store(true, Ordering::SeqCst);
        let app2 = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(REBUILD_MIN_INTERVAL_MS + 50));
            let _ = rebuild_tray_menu(&app2);
        });
        return Ok(());
    }
    // 真正执行重建
    REBUILD_LAST.store(now, Ordering::SeqCst);
    REBUILD_PENDING.store(false, Ordering::SeqCst);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(app)?))?;
    }
    Ok(())
}

/// Mihomo 托盘菜单项的处理（HTTP / RTSP 已从托盘移除，仅保留在设置/主界面）。
fn handle_extra_menu(app: &AppHandle, id: &str) {
    // ---- Mihomo ----
    if id == ID_MIHOMO_TOGGLE {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = match app.try_state::<crate::commands::mihomo::MihomoState>() {
                Some(s) => s.inner().clone(),
                None => return,
            };
            let running = state.child.lock().unwrap().is_some();
            let r = if running {
                crate::commands::mihomo::manager::stop_core(&state);
                Ok(())
            } else {
                crate::commands::mihomo::manager::launch_core(&app, state.clone()).await
            };
            if let Err(e) = r {
                eprintln!("[tray] mihomo 启停失败: {e}");
            }
            // 启停后刷新托盘（更新「运行中/已停止」标题与开关文字）
            let _ = rebuild_tray_menu(&app);
        });
        return;
    }
}

/// 标记托盘菜单已打开，并启动兜底定时器：若用户未点选任何项直接关闭菜单，
/// 超时后自动复位标记，避免菜单重建被永久禁止（此时托盘数据可能短暂滞后）。
fn mark_tray_menu_open() {
    TRAY_MENU_OPEN.store(true, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        TRAY_MENU_OPEN.store(false, Ordering::SeqCst);
        crate::exit_log::exit_log("[tray] 兜底定时器已复位 TRAY_MENU_OPEN=false");
    });
}

#[tauri::command]
pub fn refresh_tray_menu(app: AppHandle) -> Result<(), String> {
    rebuild_tray_menu(&app).map_err(|e| e.to_string())
}

pub(crate) fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let window = match app.get_webview_window(MAIN_WINDOW_LABEL) {
        Some(window) => window,
        None => match create_main_window(app) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("failed to create main window: {error}");
                return;
            }
        },
    };

    // 管理员模式下放行 UIPI 拖放消息，修复无法从外部拖放文件到启动模块的问题
    allow_uipi_drop();

    focus_main_window(&window);
}


/// 显示并聚焦主窗口，且显式聚焦 WebView2 内容。
/// 仅调用 window.set_focus() 只聚焦顶层窗口 HWND，键盘事件仍可能不进入 WebView2，
/// 表现为「输入框光标闪烁但敲不进字」。必须在窗口显示后额外调用 webview.set_focus()。
pub(crate) fn focus_main_window<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    // 管理员模式下放行 UIPI 拖放消息（覆盖页面加载后的首次 show 路径）
    allow_uipi_drop();
    // 默认恢复/唤起时打开「启动」（Launcher）模块（应用启动、托盘恢复、主全局热键）。
    show_and_open_module(window, LAUNCHER_MODULE);
}

/// 始终显示主窗口并切到指定顶级模块。
/// `module` 为前端 PageId（如 "ai"、"mihomo"）；[`LAUNCHER_MODULE`] 表示「启动」模块。
pub(crate) fn show_and_open_module<R: Runtime>(window: &tauri::WebviewWindow<R>, module: &str) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_decorations(false);
    // 1) 激活顶层窗口
    let _ = window.set_focus();
    // 2) 关键：聚焦 WebView2 内容，让键盘事件进入页面
    let webview: &Webview<R> = window.as_ref();
    let _ = webview.set_focus();
    // 3) 通知前端切到目标模块。
    //    "launcher-toggle" 无参 -> 启动模块；"launcher-open-module" 带模块 id -> 对应模块。
    if module == LAUNCHER_MODULE {
        let _ = window.emit("launcher-toggle", ());
    } else {
        let _ = window.emit("launcher-open-module", module.to_string());
    }
}

fn create_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::WebviewWindow<R>> {
    let mut builder = WebviewWindowBuilder::new(
        app,
        MAIN_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title(MAIN_WINDOW_TITLE)
    .inner_size(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT)
    .decorations(false)
    .center();

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone())?;
    }

    builder.build()
}

/// vex 元气问候（托盘失活菜单项）：按时间轮换，托盘每次重建都换一句，让女孩更「活」。
fn vex_greeting() -> String {
    const GREETINGS: &[&str] = &[
        "我在呢，有什么想弄的，随时说一声。",
        "今天也慢慢来，我从旁边陪着。",
        "忙归忙，记得歇一歇。",
        "有需要就喊我，我一直都在。",
    ];
    let secs = now_ms() / 1000;
    let idx = ((secs / 8) as usize) % GREETINGS.len();
    format!("💖 {}", GREETINGS[idx])
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let show_item = MenuItemBuilder::with_id(ID_SHOW, "显示主窗口").build(app)?;
    let mut builder = MenuBuilder::new(app).item(&show_item).separator();
    // vex 的问候：置灰失活项，展示角色存在感
    let vex_item = MenuItemBuilder::with_id(ID_VEX_GREETING, vex_greeting())
        .enabled(false)
        .build(app)?;
    builder = builder.item(&vex_item).separator();

    let config = crate::commands::config::load_config();
    let registry = crate::commands::project::registry::registry();
    let versions_dir = Path::new(&config.versions_dir);
    let links_dir = Path::new(&config.links_dir);
    let mut any_managed = false;

    for def in &registry {
        let id = &def.id;
        let delegation = crate::commands::project::scanner::get_project_delegation(&config, id, def);
        let show_vc = delegation.version_control;
        if !show_vc {
            continue;
        }

        let show_version = config.project_menu_configs.get(id).is_none_or(|c| c.show_version);
        if !show_version {
            continue;
        }

        let mut versions = scan_installed_versions(&versions_dir.join(id));
        if versions.is_empty() {
            continue;
        }
        any_managed = true;

        let active = resolve_active_version(&links_dir.join(id))
            .or_else(|| config.active_versions.get(id).cloned());
        let title = format!(
            "{} ({})",
            def.display_name,
            active.clone().unwrap_or_else(|| "未激活".to_string())
        );
        let mut submenu = SubmenuBuilder::new(app, title);

        for version in versions.drain(..) {
            let label = if Some(&version) == active.as_ref() {
                format!("✓ {}", version)
            } else {
                version.clone()
            };
            let switch_id = format!("{}{}::{}", ID_SWITCH_PREFIX, id, version);
            let item = MenuItemBuilder::with_id(&switch_id, &label).build(app)?;
            submenu = submenu.item(&item);
        }

        let submenu = submenu.build()?;
        builder = builder.item(&submenu);
    }

    if !any_managed {
        let empty = MenuItemBuilder::with_id(ID_EMPTY, "(没有完全托管的项目)")
            .enabled(false)
            .build(app)?;
        builder = builder.item(&empty);
    }

    // 服务平铺到顶层：已托管服务正常显示；未托管但检测到外部进程运行的服务也显示，
    // 让托盘能够反映 MySQL/Redis 等由系统服务或其它程序启动的实例。
    let mut any_service = false;
    // 一次性收集所有服务 id，读取后台刷新的快照，避免在主线程同步执行
    // tasklist/wmic/netstat（长时运行后这些命令变慢会阻塞事件循环，导致托盘无响应）。
    let service_ids: Vec<String> = registry
        .iter()
        .filter(|def| def.category == crate::commands::project::types::ProjectCategory::Service || def.is_service)
        .map(|def| def.id.clone())
        .collect();
    let status_snapshot = crate::commands::service::service_status_snapshot(&service_ids);
    for def in &registry {
        if def.category != crate::commands::project::types::ProjectCategory::Service && !def.is_service {
            continue;
        }
        let status = status_snapshot
            .get(&def.id)
            .cloned()
            .unwrap_or_default();
        let externally_running = status.external || status.status.as_deref() == Some("external_running");
        if !config.managed_items.contains(&def.id) && !externally_running {
            continue;
        }

        let show_service = config.project_menu_configs.get(&def.id).is_none_or(|c| c.show_service);
        if !show_service {
            continue;
        }

        // 从快照取状态；快照未就绪（首次构建/后台刷新中）时用默认值兜底
        let status_text = status.status.as_deref().unwrap_or(if status.running { "running" } else { "stopped" });
        if status_text == "not_installed" {
            continue;
        }
        if !any_service {
            builder = builder.separator();
            any_service = true;
        }

        // 平铺为单个「启动/停止」切换项，不再套子菜单：状态用标题文字表达，
        // 点击即执行对应动作，省去进入子菜单再点击一步。
        let port_text = if status.running {
            status.port.map(|p| format!(" :{}", p)).unwrap_or_default()
        } else {
            String::new()
        };
        // 启动中状态：显示「⋯ 启动中」并置灰禁用，防止重复启动
        let starting = starting_services_contains(&def.id);
        let (item_id, label, enabled) = if starting {
            (
                format!("{}{}", ID_SERVICE_START_PREFIX, def.id),
                format!("⋯ {} · 启动中", def.display_name),
                false,
            )
        } else if externally_running {
            (
                format!("{}{}", ID_SERVICE_START_PREFIX, def.id),
                format!("● {} · 外部运行", def.display_name),
                false,
            )
        } else if status.running {
            (
                format!("{}{}", ID_SERVICE_STOP_PREFIX, def.id),
                format!("■ {} · 停止{}", def.display_name, port_text),
                true,
            )
        } else {
            (
                format!("{}{}", ID_SERVICE_START_PREFIX, def.id),
                format!("▶ {} · 启动", def.display_name),
                true,
            )
        };
        let item = MenuItemBuilder::with_id(item_id, label)
            .enabled(enabled)
            .build(app)?;
        builder = builder.item(&item);
    }

    // ---- Mihomo（可在设置里控制是否显示）----
    let tray_cfg = config.tray_menu.clone();

    if tray_cfg.show_mihomo {
        if let Some(item) = build_mihomo_item(app)? {
            // 若上方已有服务/SDK 项，加分隔线避免挤在一起
            if any_service {
                builder = builder.separator();
            }
            builder = builder.item(&item);
        }
    }

    let quit_item = MenuItemBuilder::with_id(ID_QUIT, "退出 Kira").build(app)?;
    builder = builder.separator().item(&quit_item);

    builder.build()
}

fn build_mihomo_item(app: &AppHandle) -> tauri::Result<Option<tauri::menu::MenuItem<tauri::Wry>>> {
    // 平铺为单个「启动/停止」切换项，与普通服务一致：仅依赖 running 状态，
    // 不依赖订阅/代理/模式等内部动态数据，因此完全不需要周期性刷新托盘菜单
    // （避免 set_menu 反复重建导致 on_menu_event 长时间运行后失效）。
    let state = match app.try_state::<crate::commands::mihomo::MihomoState>() {
        Some(s) => s.inner().clone(),
        None => return Ok(None),
    };
    // 判断逻辑与 launch_core 的幂等检测一致：同时考虑本应用拉起的子进程
    // 与混合端口监听（外部/上次会话遗留进程）。否则自动启动时核心已运行
    // 但 child 为空，托盘会误显示「启动」且无法更新。
    let running = crate::commands::mihomo::manager::is_core_running(&state);

    // 状态并入标题，一步切换启停，不套二级菜单
    let label = if running {
        "■ Mihomo · 停止".to_string()
    } else {
        "▶ Mihomo · 启动".to_string()
    };
    let item = MenuItemBuilder::with_id(ID_MIHOMO_TOGGLE, label).build(app)?;
    Ok(Some(item))
}

fn scan_installed_versions(dir: &Path) -> Vec<String> {
    let mut versions: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_type()
                        .map(|ty| ty.is_dir() || ty.is_symlink())
                        .unwrap_or(false)
                })
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| !name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    versions.sort();
    versions
}

fn resolve_active_version(junction_path: &Path) -> Option<String> {
    if !junction_path.exists() && !junction_path.is_symlink() {
        return None;
    }

    std::fs::canonicalize(junction_path)
        .ok()
        .and_then(|target| target.file_name().map(|name| name.to_string_lossy().to_string()))
}

fn parse_switch_id(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix(ID_SWITCH_PREFIX)?;
    rest.split_once("::")
}
