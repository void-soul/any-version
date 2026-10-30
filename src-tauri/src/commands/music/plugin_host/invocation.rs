use super::*;

/// 载入插件并返回其自述（`platform` / `version` / 能力…）。
///
/// **必须与 [`call_plugin`] 分开**：`call_plugin(path, "load", …)` 的语义是
/// 「先确保载入，**再调用名为 `load` 的方法**」—— 第二次 load 没有 `path` 参数，
/// 桥会 `require(undefined)` 报「The "id" argument must be of type string」。
/// 真机验证时正是这么踩到的：探测插件信息失败，还连带把宿主状态搞坏。
pub fn load_plugin(plugin_path: &Path, timeout: Duration) -> Result<Value, String> {
    ensure_layout()?;
    let mut guard = host_lock()?;
    let result = {
        let bridge = ensure_bridge(&mut guard)?;
        ensure_loaded(bridge, plugin_path, timeout)
    };
    if result.is_err() {
        *guard = None; // 载入失败多半是宿主状态坏了，丢掉重来
    }
    result
}

/// 该方法需要的能力位。缺了就返回一句人话，而不是等 JS 抛 `TypeError`。
pub(super) fn capability_gap(meta: &Value, method: &str) -> Option<String> {
    let (key, label) = match method {
        "search" => ("hasSearch", "搜索"),
        "mediaSource" => ("hasMediaSource", "获取播放地址"),
        _ => return None,
    };
    if meta.get(key).and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let platform = meta
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("该插件");
    Some(format!("{platform} 不支持{label}"))
}

/// 方法名 → 用户看得懂的动作名（错误信息里用）。
pub(super) fn method_label(method: &str) -> &'static str {
    match method {
        "search" => "搜索",
        "mediaSource" => "获取播放地址",
        "load" => "加载插件",
        _ => "调用插件",
    }
}

/// 从 `Request failed with status code 522` 这类消息里取出状态码。
pub(super) fn status_code(raw: &str) -> Option<u32> {
    const MARKER: &str = "status code ";
    let start = raw.find(MARKER)? + MARKER.len();
    let digits: String = raw[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// 把插件抛出的原始错误翻成「**是什么 + 怎么办**」。
///
/// 插件里抛的多是 axios 的英文原文（如 `Request failed with status code 522`）——
/// 用户既看不懂，也不知道下一步该做什么。这里按常见模式给出中文说明。
///
/// **认不出来的原样返回**：宁可保留原文，也不要编一句可能不准确的话。
/// 这条对 522 尤其重要：它是 Cloudflare 的「源站连接超时」，属于**插件上游**的问题，
/// 我们能做的只有「稍后重试 / 换个音源」，不该让用户以为是我们的播放器坏了。
pub(super) fn friendly_reason(raw: &str) -> String {
    if let Some(code) = status_code(raw) {
        return match code {
            401 | 403 => format!("音源拒绝了这个请求（HTTP {code}），可能需要登录，或该音质不可用"),
            404 => format!("音源接口不存在（HTTP {code}），插件可能已失效或需要更新"),
            429 => format!("请求过于频繁（HTTP {code}），稍等一会儿再试"),
            500..=599 => {
                format!("音源服务器暂时不可用（HTTP {code}），可稍后重试或换一个音源")
            }
            _ => raw.to_string(),
        };
    }
    let lower = raw.to_ascii_lowercase();
    for (pattern, message) in [
        ("timeout of", "音源响应超时，可稍后重试或换一个音源"),
        ("etimedout", "音源响应超时，可稍后重试或换一个音源"),
        (
            "enotfound",
            "连不上音源服务器（域名解析失败），请检查网络或代理",
        ),
        (
            "eai_again",
            "连不上音源服务器（域名解析失败），请检查网络或代理",
        ),
        (
            "getaddrinfo",
            "连不上音源服务器（域名解析失败），请检查网络或代理",
        ),
        ("econnrefused", "连接音源服务器被拒绝，请检查网络或代理"),
        ("econnreset", "音源服务器中断了连接，可稍后重试"),
        ("socket hang up", "音源服务器中断了连接，可稍后重试"),
        ("network error", "网络请求失败，请检查网络或代理"),
    ] {
        if lower.contains(pattern) {
            return message.to_string();
        }
    }
    raw.to_string()
}

/// 调用某插件的方法（自动载入 / 复用宿主进程）。
///
/// 每次调用都带插件路径：与「当前载入的是谁」不一致时会重新 `load`，
/// 调用方不必关心宿主的生命周期。宿主全局只留一个（单进程串行调用）。
pub fn call_plugin(
    plugin_path: &Path,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    ensure_layout()?;
    let mut guard = host_lock()?;
    let mut reset = false;
    let result = {
        let bridge = ensure_bridge(&mut guard)?;
        match ensure_loaded(bridge, plugin_path, timeout) {
            Ok(meta) => match capability_gap(&meta, method) {
                Some(message) => Err(message),
                None => {
                    let call = bridge.call(method, params, timeout);
                    if call.is_err() && bridge.is_dead() {
                        reset = true; // 进程没了：下次调用自动重启，而不是一直报「已退出」
                    }
                    call
                }
            },
            Err(error) => {
                reset = true;
                Err(error)
            }
        }
    };
    if reset {
        *guard = None;
    }
    result
}
