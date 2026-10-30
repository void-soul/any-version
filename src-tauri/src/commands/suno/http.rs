use super::*;

/// 读 Windows 系统代理（mihomo 固定端口模式），返回 `http://127.0.0.1:port`。
///
/// 项目全局的 `utils::get_http_client()` 没有启用 reqwest 的 `system-proxy` feature，
/// 因此默认**不读系统代理**——Suno 这类需要走代理的域名会直接连失败。这里读注册表
/// 把系统代理交给 reqwest。未启用 / 空则返回 None（PAC 模式暂不支持，直连兜底）。
///
/// 结果缓存起来避免每次请求都读注册表，但**可失效**（见 [`invalidate_proxy_cache`]）：
/// 应用启动时代理往往还没就绪（mihomo 未起 / 端口未监听），此时若把地址永久固化，
/// 之后每次请求都会撞上这个死代理，且永远不会重试——表现就是「明明浏览器能打开，
/// 应用里一直报 `error sending request`」。
/// 系统代理判定缓存：外层 `Option` 表示「是否已探测」，内层是探测结果。
pub(super) static PROXY_CACHE: std::sync::Mutex<Option<Option<String>>> =
    std::sync::Mutex::new(None);

pub(super) fn system_proxy() -> Option<String> {
    let mut guard = PROXY_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = guard.as_ref() {
        return cached.clone();
    }
    let proxy = read_system_proxy();
    match &proxy {
        Some(addr) => eprintln!("[Suno] 检测到系统代理: {addr}"),
        None => eprintln!("[Suno] 未检测到系统代理，将直连"),
    }
    *guard = Some(proxy.clone());
    proxy
}

/// 代理判定失效：下一次 [`system_proxy()`] 会重新读注册表。
///
/// 在「走了代理但网络层失败」时调用——代理地址可能已经变了（用户刚起/刚关 mihomo），
/// 固化旧值会让后续请求一直失败。
pub(super) fn invalidate_proxy_cache() {
    *PROXY_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub(super) fn read_system_proxy() -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
            KEY_READ,
        )
        .ok()?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    let server: String = key.get_value("ProxyServer").unwrap_or_default();
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    Some(format!("http://{server}"))
}

/// 不带代理的客户端：作为「代理不可用」时的兜底路径。
///
/// 直连并非永远可行（部分网络环境必须走代理），但代理只是**可选加速路径**——
/// 代理挂了不该让整个功能变成「打不开」，所以两条路径都试。
pub(super) fn direct_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .http1_only()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 构建带系统代理的 HTTP 客户端（首选路径）。
pub(super) fn http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        // Suno 对 HTTP/2（reqwest/hyper 默认协商）与 HTTP/1.1 返回的 SSR 内容不同：
        // 实测 HTTP/1.1 才带歌曲数据，故强制 HTTP/1.1。
        .http1_only()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120));
    if let Some(proxy) = system_proxy() {
        if let Ok(p) = reqwest::Proxy::all(&proxy) {
            builder = builder.proxy(p);
        } else {
            eprintln!("[Suno] 代理地址解析失败，退回直连: {proxy}");
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

/// 把用户粘贴的地址归一化成 reqwest 能直接用的绝对 URL。
///
/// [`is_profile_url`] 故意宽松（允许 `suno.com/@user`、`http://`、`www.` 前缀），
/// 但 reqwest 只接受**绝对 URL** —— 缺协议会直接报 `relative URL without a base`，
/// 校验通过、请求却发不出去。这里统一补 `https://`，再把**真正要发出去的地址**打出来。
pub(super) fn normalize_fetch_url(trimmed: &str) -> String {
    let s = trimmed.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}
