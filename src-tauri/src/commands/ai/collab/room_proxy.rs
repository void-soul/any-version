//! room proxy responsibility.
use super::*;

// ─── 房间级常驻代理 ───

/// 房间+工具级的常驻代理条目
pub(super) struct RoomProxyEntry {
    pub(super) port: u16,
    pub(super) abort_handle: tokio::task::AbortHandle,
}

pub(super) static ROOM_PROXIES: OnceLock<Mutex<HashMap<String, RoomProxyEntry>>> = OnceLock::new();

pub(super) fn room_proxy_key(room_id: &str, tool_id: &str) -> String {
    format!("{}::{}", room_id, tool_id)
}

/// 获取或创建房间级常驻代理。返回 (端口, base_url, api_key)。
/// 首次调用时启动代理 + 写配置文件；后续调用直接复用已运行的代理。
pub(super) async fn ensure_room_proxy(
    room_id: &str,
    tool_id: &str,
    tool_config: &crate::commands::ai_registry::ToolConfig,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    options: &CollabDispatchOptions,
    app_handle: &tauri::AppHandle,
) -> (u16, String, String) {
    let key = room_proxy_key(room_id, tool_id);
    let map = ROOM_PROXIES.get_or_init(|| Mutex::new(HashMap::new()));
    let config = crate::commands::ai::config::load_ai_config();
    let provider = provider_id.and_then(|pid| config.providers.iter().find(|p| p.id == pid));

    // 1. 检查是否已有常驻代理 → 健康检查通过才复用；失联则清除重建
    //（旧逻辑无脑复用，代理任务若已死，子进程连不上 base_url → gemini "fetch failed" 之类）
    let existing_port = { map.lock().get(&key).map(|e| e.port) };
    let mut reused_port: Option<u16> = None;
    if let Some(p0) = existing_port {
        if room_proxy_alive(p0).await {
            eprintln!("[collab] 复用常驻代理: key={}, port={}", key, p0);
            reused_port = Some(p0);
        } else {
            eprintln!(
                "[collab] ⚠ 常驻代理失联(port={})，清除并重建: key={}",
                p0, key
            );
            if let Some(entry) = map.lock().remove(&key) {
                entry.abort_handle.abort();
            }
        }
    }

    let mut port: u16 = reused_port.unwrap_or(0);
    let mut abort_handle: Option<tokio::task::AbortHandle> = None;
    if reused_port.is_none() {
        eprintln!(
            "[collab] 首次创建代理: key={}, tool={}, provider={:?}, model={:?}",
            key, tool_id, provider_id, model_id
        );
        if provider.is_none() {
            eprintln!("[collab] ⚠ 未找到 provider: {:?}", provider_id);
        }

        let req = LaunchAiToolRequest {
            tool_id: tool_id.to_string(),
            model_id: model_id.map(|s| s.to_string()),
            provider_id: provider_id.map(|s| s.to_string()),
            masquerade_model: options.masquerade_model.clone(),
            fallback_model_id: options.fallback_model_id.clone(),
            fallback_provider_id: options.fallback_provider_id.clone(),
            fallback_masquerade_model: options.fallback_masquerade_model.clone(),
            one_m_context: options.one_m_context,
            fallback_one_m_context: options.fallback_one_m_context,
            optimizer_enabled: options.optimizer_enabled,
            rectifier_enabled: options.rectifier_enabled,
            rectifier_thinking_signature: options.rectifier_thinking_signature,
            rectifier_thinking_budget: options.rectifier_thinking_budget,
            rectifier_media_fallback: options.rectifier_media_fallback,
            rectifier_media_heuristic: options.rectifier_media_heuristic,
            rectifier_protocol_mismatch: options.rectifier_protocol_mismatch,
            rectifier_toolcall_dialect: options.rectifier_toolcall_dialect,
            optimizer_cache_injection: options.optimizer_cache_injection,
            optimizer_thinking: options.optimizer_thinking,
            optimizer_deepseek: options.optimizer_deepseek,
            custom_params: options.custom_params.clone(),
            custom_param_values: options.custom_param_values.clone(),
            ..Default::default()
        };

        let (p1, h1, _tok) = start_tool_proxy_with_collab(
            tool_config,
            provider,
            &config,
            &req,
            Some(app_handle.clone()),
            Some(room_id.to_string()),
        )
        .await;
        port = p1;
        abort_handle = h1;
        eprintln!(
            "[collab] 代理启动结果: port={}, has_abort={}",
            port,
            abort_handle.is_some()
        );
    }

    let base_url = if port != 0 {
        format!("http://127.0.0.1:{}", port)
    } else {
        provider
            .map(|p| p.url_for(&tool_config.native_protocol()))
            .unwrap_or_default()
    };
    let api_key = provider.map(|p| p.api_key.clone()).unwrap_or_default();

    // 每次派发都重写工具配置文件：普通启动（AI-工具页）可能在两次派发之间
    // 把共享配置（如 ~/.codex/config.toml）改写指向已关闭的启动代理端口。
    // 即使本地代理未启动（port==0），也写入 provider 上游 URL，避免 stale config
    // 导致工具始终使用旧模型（如 open-code 忽略自定义 env var，只读配置文件 model 字段）。
    if let Some(ref p) = provider {
        if !base_url.is_empty() {
            // 声明模型名 C：伪装优先，否则所选取模型 B
            let claimed_model = options
                .masquerade_model
                .clone()
                .filter(|c| !c.is_empty())
                .or_else(|| model_id.map(|s| s.to_string()));
            // 出站协议：collab 派发的工具均为单协议 JSON runner，native_protocol 即所选协议
            let chosen_protocol = tool_config.native_protocol();
            // 真实上游（走本地代理时 base_url 是 127.0.0.1，展示用的「厂商」要用它）
            let upstream = p.url_for(&chosen_protocol);
            if let Err(e) = crate::commands::ai::launch::write_tool_config_from_spec(
                tool_config,
                model_id,
                claimed_model.as_deref(),
                &base_url,
                &p.api_key,
                &upstream,
                options.fallback_model_id.as_deref(),
                options.fallback_masquerade_model.as_deref(),
                options.one_m_context,
                options.fallback_one_m_context,
                true,
                &options.custom_params,
                &options.custom_param_values,
                options.web_search,
                &chosen_protocol,
            ) {
                eprintln!("[collab] ⚠ 写入工具配置文件失败: {}", e);
            } else {
                eprintln!("[collab] ✓ 工具配置文件已写入（baseUrl → {}）", base_url);
            }
        }
    }

    // 存入全局表（仅新建时；复用路径 abort_handle 为 None 不会重复插入）
    if port != 0 {
        if let Some(h) = abort_handle {
            let mut g = map.lock();
            g.insert(
                key,
                RoomProxyEntry {
                    port,
                    abort_handle: h,
                },
            );
            eprintln!("[collab] 常驻代理已创建 port={}", port);
        }
    }

    (port, base_url, api_key)
}

/// 探测常驻代理是否存活（600ms 超时，直连不走系统代理）
pub(super) async fn room_proxy_alive(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(600))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(&url).send().await {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

/// 停止并移除房间+工具级的常驻代理
pub(super) fn stop_room_proxy(room_id: &str, tool_id: &str) {
    let key = room_proxy_key(room_id, tool_id);
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock();
        if let Some(entry) = g.remove(&key) {
            entry.abort_handle.abort();
            eprintln!("[collab] 已停止代理 port={}", entry.port);
        }
    }
}

/// 停止某房间相关的所有常驻代理
pub(super) fn stop_room_proxies(room_id: &str) {
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock();
        let prefix = format!("{}::", room_id);
        let keys: Vec<String> = g
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for k in keys {
            if let Some(entry) = g.remove(&k) {
                entry.abort_handle.abort();
                eprintln!("[collab] 已停止代理 port={}", entry.port);
            }
        }
    }
}

/// 停止所有常驻代理（应用退出时调用）
pub fn stop_all_room_proxies() {
    crate::exit_log::exit_log("cleanup: stop_all_room_proxies 进入");
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock();
        crate::exit_log::exit_log("cleanup: ROOM_PROXIES 锁已获取");
        let count = g.len();
        for (_, entry) in g.drain() {
            entry.abort_handle.abort();
        }
        if count > 0 {
            eprintln!("[collab] 已停止所有常驻代理（{} 个）", count);
        }
        crate::exit_log::exit_log(&format!("cleanup: 已 abort {} 个常驻代理", count));
    } else {
        crate::exit_log::exit_log("cleanup: ROOM_PROXIES 未初始化，跳过");
    }
}
