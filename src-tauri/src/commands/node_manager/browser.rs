use super::*;

// ─── 打开主页面 ───

/// 查询某项目最近捕获的控制台 URL（带凭据主页地址）。
/// 未配置 consoleUrlPattern 或尚未捕获时返回 None。
#[tauri::command]
pub fn npm_console_url(project_id: String) -> Result<Option<String>, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.has_console_url_pattern() {
        return Ok(None);
    }
    Ok(console_url_of(&def.id))
}

/// 在独立的 Kira 窗口中打开服务主页。
///
/// 为什么不用主窗口 iframe：dsh 等服务的鉴权依赖 `SameSite=Strict` 的 HttpOnly
/// Cookie（token URL → 303 → Set-Cookie → /）。iframe 是第三方上下文，WebView2
/// 会拦截该 Cookie，重定向后的 / 永远 401——token 正确也进不去。独立顶级窗口的
/// Cookie 是第一方，可正常完成握手，同时仍属于本应用（而非外部浏览器）。
#[tauri::command]
pub async fn npm_open_window(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    use tauri::Manager;
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    let url = if def.has_console_url_pattern() {
        console_url_of(&def.id).unwrap_or_else(|| def.resolved_web_path())
    } else {
        def.resolved_web_path()
    };
    let label = format!("svc-{}", def.id);
    // 已有窗口：导航到最新地址并聚焦（token 轮换后复用窗口即可）
    if let Some(win) = app.get_webview_window(&label) {
        let js = format!(
            "window.location.replace({});",
            serde_json::to_string(&url).unwrap_or_else(|_| "\"about:blank\"".into())
        );
        let _ = win.eval(&js);
        let _ = win.set_focus();
        return Ok(());
    }
    let parsed: tauri::Url = url.parse().map_err(|e| format!("URL 解析失败: {}", e))?;
    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(parsed))
        .title(format!("{} — Kira", def.display_name))
        .inner_size(1280.0, 820.0)
        .build()
        .map_err(|e| format!("创建窗口失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn npm_open(project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    // 配置了控制台 URL 提取的项目优先用捕获的带凭据地址（token 随每次启动变化）；
    // 尚未捕获时回退普通 webPath（服务自己无鉴权时也能直接打开）。
    let url = if def.has_console_url_pattern() {
        console_url_of(&def.id).unwrap_or_else(|| def.resolved_web_path())
    } else {
        def.resolved_web_path()
    };
    open_url_in_browser(&url);
    Ok(())
}

// ─── 开发者模式打开主页面 ───

/// 返回浏览器的常见安装位置。
/// Windows 桌面应用通常不会把 Edge/Chrome 加入 Kira 进程的 PATH，
/// 仅调用 find_in_path 会导致「开发者模式」看起来完全没有反应。
pub(super) fn browser_candidates(name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = find_in_path(name) {
        out.push(path);
    }

    #[cfg(target_os = "windows")]
    {
        let (relative, exe) = match name.to_ascii_lowercase().as_str() {
            "msedge" | "msedge.exe" => ("Microsoft\\Edge\\Application", "msedge.exe"),
            "chrome" | "chrome.exe" => ("Google\\Chrome\\Application", "chrome.exe"),
            _ => return out,
        };
        for env_name in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(root) = std::env::var(env_name) {
                out.push(PathBuf::from(root).join(relative).join(exe));
            }
        }
    }
    out
}

pub(super) fn find_browser(name: &str) -> Option<PathBuf> {
    browser_candidates(name).into_iter().find(|p| p.is_file())
}

/// 生成独立浏览器实例的启动参数。
/// 使用独立 profile 是关键：如果复用已运行的 Edge/Chrome，浏览器可能把 URL
/// 转发给旧进程并忽略 `--auto-open-devtools-for-tabs`。
pub(super) fn devtools_browser_args(url: &str, profile_dir: &Path) -> Vec<String> {
    vec![
        "--new-window".to_string(),
        "--auto-open-devtools-for-tabs".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        format!("--user-data-dir={}", profile_dir.to_string_lossy()),
        url.to_string(),
    ]
}

/// 在系统浏览器中打开服务首页，并请求自动打开 DevTools。
/// 内嵌 iframe 无法跨域控制浏览器 DevTools，因此开发者模式使用独立浏览器窗口，
/// 同时保留服务管理器里的内嵌首页作为日常查看入口。
#[tauri::command]
pub async fn npm_open_devtools(project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    let url = def.resolved_web_path();

    #[cfg(target_os = "windows")]
    {
        let browser = find_browser("msedge").or_else(|| find_browser("chrome"));
        let path = browser.ok_or_else(|| {
            "未找到 Edge 或 Chrome。请确认浏览器已安装，或将浏览器目录加入 PATH 后重试".to_string()
        })?;
        let profile = std::env::temp_dir()
            .join("any-version-devtools")
            .join(format!("{}-{}", project_id, std::process::id()));
        let args = devtools_browser_args(&url, &profile);
        eprintln!("[node_manager] 开发者模式: {} {:?}", path.display(), args);
        std::process::Command::new(&path)
            .args(&args)
            .spawn()
            .map_err(|e| format!("启动开发者模式浏览器失败（{}）: {}", path.display(), e))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let args = devtools_browser_args(&url, &std::env::temp_dir().join("any-version-devtools"));
        std::process::Command::new("open")
            .args(["-na", "Google Chrome", "--args"])
            .args(&args)
            .spawn()
            .map_err(|e| format!("启动开发者模式浏览器失败: {}", e))?;
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let browser = find_browser("google-chrome")
            .or_else(|| find_browser("chromium"))
            .ok_or_else(|| "未找到 Google Chrome 或 Chromium，无法打开开发者模式".to_string())?;
        let args = devtools_browser_args(&url, &std::env::temp_dir().join("any-version-devtools"));
        std::process::Command::new(browser)
            .args(&args)
            .spawn()
            .map_err(|e| format!("启动开发者模式浏览器失败: {}", e))?;
        return Ok(());
    }
}
