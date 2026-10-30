use super::*;

/// 用 Edge 无头浏览器渲染页面，返回 JS 执行后的完整 DOM 文本。
/// 关键：`--dump-dom` 输出的是渲染完成后的 DOM（含 JS 动态插入的内容）；
/// `--virtual-time-budget` 让浏览器快进虚拟时间并等待异步内容（SPA/懒加载）落定。
/// Edge 不存在或超时返回 None，调用方再决定是否回退普通 HTTP。
pub(super) async fn render_dom_via_browser(url: &str) -> Option<String> {
    let browser_path = match find_browser() {
        Some(p) => p,
        None => {
            crate::exit_log::exit_log("[picky-render] 未找到可用浏览器（Edge/Chrome），跳过渲染");
            return None;
        }
    };
    // 每次渲染用全新的临时 user-data-dir（进程 id + 纳秒时间戳）：
    // profile 锁被占用时无头进程会「静默失败」（退出码 0、0 字节输出、无 stderr），
    // 复用同一目录在并发抓取（新增收藏自动补全元数据）或上次渲染超时残留孤儿进程时必然踩锁。
    let tmp_profile = std::env::temp_dir().join(format!(
        "kira-picky-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let args = [
        "--headless",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-extensions",
        // 隐藏自动化特征：无头浏览器默认 UA 带 HeadlessChrome，部分站点据此返回空壳/拦截
        "--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0",
        "--window-size=1280,900",
        "--virtual-time-budget=12000",
        "--dump-dom",
        url,
    ];
    let run = |path: &std::path::PathBuf| {
        tokio::time::timeout(
            std::time::Duration::from_secs(35),
            tokio::process::Command::new(path)
                .args(&args)
                // 超时/取消时终止浏览器进程，避免孤儿无头浏览器占住 profile 影响后续抓取
                .kill_on_drop(true)
                .arg(format!("--user-data-dir={}", tmp_profile.display()))
                .output(),
        )
    };
    let mut output = match run(&browser_path).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            crate::exit_log::exit_log(&format!("[picky-render] spawn失败: {e} url={url}"));
            return None;
        }
        Err(_) => {
            crate::exit_log::exit_log(&format!("[picky-render] 超时(35s) url={url}"));
            return None;
        }
    };
    // 首次启动可能因冷启动/首建用户目录超时或无输出：有预算再试一次
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        output = match run(&browser_path).await {
            Ok(Ok(o)) => o,
            _ => {
                crate::exit_log::exit_log(&format!("[picky-render] 重试失败 url={url}"));
                return None;
            }
        };
    }
    if !output.status.success() {
        crate::exit_log::exit_log(&format!(
            "[picky-render] 非零退出 code={:?} stderr={} url={url}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(200)
                .collect::<String>()
        ));
        return None;
    }
    let body = String::from_utf8_lossy(&output.stdout).to_string();
    // 清理临时 profile（失败不影响结果；目录含纳秒时间戳，残留不占锁）
    let _ = std::fs::remove_dir_all(&tmp_profile);
    if body.trim().is_empty() {
        crate::exit_log::exit_log(&format!("[picky-render] DOM为空 url={url}"));
        None
    } else {
        crate::exit_log::exit_log(&format!(
            "[picky-render] 成功 {} 字节 url={url}",
            body.len()
        ));
        Some(body)
    }
}

/// 定位本机可用的浏览器可执行文件（Windows 自带 Edge；无 Edge 时尝试 Chrome）。
/// 探测顺序：Edge x86 → Edge x64 → Edge 用户级安装 → 注册表 App Paths → Chrome 同序。
pub(super) fn find_browser() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        let candidates = |name: &str| -> Vec<std::path::PathBuf> {
            let app_rel = format!("Microsoft\\Edge\\Application\\{}.exe", name);
            let chrome_rel = format!("Google\\Chrome\\Application\\{}.exe", name);
            let mut v = Vec::new();
            for var in ["ProgramFiles(x86)", "ProgramFiles", "LocalAppData"] {
                if let Some(root) = std::env::var_os(var) {
                    v.push(std::path::PathBuf::from(&root).join(&app_rel));
                    v.push(std::path::PathBuf::from(root).join(&chrome_rel));
                }
            }
            v
        };
        for p in candidates("msedge") {
            if p.exists() {
                return Some(p);
            }
        }
        // 注册表 App Paths 兜底（覆盖自定义安装路径；msedge 与 chrome 都查）
        for exe in ["msedge.exe", "chrome.exe"] {
            if let Some(p) = app_paths_lookup(exe) {
                return Some(p);
            }
        }
        for p in candidates("chrome") {
            if p.exists() {
                return Some(p);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        for name in ["msedge", "google-chrome", "chromium", "chrome"] {
            if let Ok(out) = std::process::Command::new("which").arg(name).output() {
                if out.status.success() {
                    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !p.is_empty() {
                        return Some(std::path::PathBuf::from(p));
                    }
                }
            }
        }
        None
    }
}

/// 读取注册表 App Paths 中浏览器的安装路径（HKLM 与 HKCU 都查）。
#[cfg(windows)]
pub(super) fn app_paths_lookup(exe: &str) -> Option<std::path::PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let key = format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}",
        exe
    );
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(k) = RegKey::predef(hive).open_subkey(&key) {
            if let Ok(p) = k.get_value::<String, _>("") {
                let p = std::path::PathBuf::from(p.trim().trim_matches('"'));
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 从 HTML 里取 `<meta name/property="X" content="...">` 的 content 值。
/// 逐个 `<meta ...>` 标签手写解析属性，不依赖正则回引用（Rust regex 不支持），
/// 因此属性顺序、单双引号、大小写都兼容。
pub(super) fn meta_content(html: &str, pred: fn(&str) -> bool) -> Option<String> {
    const META: &str = "<meta";
    let lower = html.to_lowercase();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find(META) {
        let tag_start = from + rel;
        let seg = &html[tag_start..];
        // 找该标签的结束 `>`（不跨上述小写偏移：用原 html 找）
        let tag_end = seg.find('>')?;
        let tag = seg[..tag_end].to_string();
        let tag_lower = tag.to_lowercase();
        // 只处理真正的 meta 标签（magnet etc. 不作为条件，但留有名字可判定）
        let mut content = None;
        let mut name_or_prop: Option<String> = None;
        parse_attrs(&tag, |k, v| match k.as_str() {
            "name" | "property" => name_or_prop = Some(v),
            "content" => content = Some(v),
            _ => {}
        });
        let _ = tag_lower;
        if let Some(k) = name_or_prop {
            if pred(&k) {
                return content;
            }
        }
        from = tag_start + tag_end + 1;
    }
    None
}

/// 手写解析一个标签字符串里的 `k="v"` / `k='v'` / `k=v` 属性，逐个回调。
pub(super) fn parse_attrs(tag: &str, mut on_attr: impl FnMut(String, String)) {
    let bytes = tag.as_bytes();
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        // 跳过空白
        while i < n && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        // 读 key：字母数字 / '-' / '_' / ':' / '.'
        let key_start = i;
        while i < n {
            let c = bytes[i] as char;
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.') {
                i += 1;
            } else {
                break;
            }
        }
        let key = tag[key_start..i].to_lowercase();
        // 跳过空白到 '='
        while i < n && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= n || bytes[i] != b'=' {
            // 无值属性：继续当作空值（例如仅 rel="icon" 之外的裸属性）
            on_attr(key, String::new());
            continue;
        }
        i += 1; // '='
        while i < n && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let delim = bytes[i];
        if delim == b'"' || delim == b'\'' {
            i += 1;
            let val_start = i;
            while i < n && bytes[i] != delim {
                i += 1;
            }
            let value = tag[val_start..i.min(n)].to_string();
            on_attr(key, value);
            if i < n {
                i += 1; // 结束引号
            }
        } else {
            let val_start = i;
            while i < n && !(bytes[i] as char).is_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            on_attr(key, tag[val_start..i].to_string());
        }
    }
}
