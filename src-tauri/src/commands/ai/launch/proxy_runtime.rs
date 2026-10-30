//! proxy runtime responsibility.
use super::*;

/// 选择出站协议：若供应商支持工具「原生协议」，则同协议直连（不转换）；
/// 否则取供应商首个支持的协议（由代理做协议转换）。
/// 供应商未配置任何协议端点 URL 时返回 None。
pub(super) fn pick_outbound_protocol(native: &str, provider: &AiProvider) -> Option<String> {
    if provider.supported_protocols().contains(&native.to_string()) {
        return Some(native.to_string());
    }
    if !provider.openai_url.is_empty() {
        return Some("openai".to_string());
    }
    if !provider.anthropic_url.is_empty() {
        return Some("anthropic".to_string());
    }
    if !provider.google_url.is_empty() {
        return Some("google".to_string());
    }
    None
}

/// 上游端点是否指向**本地聚合服务**（127.0.0.1:<聚合端口>）。
///
/// 是的话代理不再自己做「整流重试 + 协议回退」——聚合自己已经会失败切换、冷却与协议转换，
/// 代理再叠一层只会把重试次数放大（并重复触发聚合的冷却）。端口匹配要求后面紧跟 `/`/`?`/结尾，
/// 避免把 `:15721` 误匹配到 `:157210`。
pub(super) fn is_aggregate_upstream(base_url: &str, aggregate_port: u16) -> bool {
    if aggregate_port == 0 {
        return false;
    }
    let base = base_url.trim().to_ascii_lowercase();
    if !(base.starts_with("http://127.0.0.1")
        || base.starts_with("http://localhost")
        || base.starts_with("https://127.0.0.1"))
    {
        return false;
    }
    let marker = format!(":{aggregate_port}");
    match base.find(&marker) {
        Some(idx) => {
            let after = &base[idx + marker.len()..];
            after.is_empty() || after.starts_with('/') || after.starts_with('?')
        }
        None => false,
    }
}

/// 供应商端点「要不要补 `/v1`」开关（按出站协议取对应字段）。`None` = 自动。
///
/// 以前只有聚合认这个开关，协议转换代理（工具启动路径）不认：同一个 `openai_include_v1=false`
/// 的供应商，走聚合能连上、走工具启动就拼错 URL 404。这里补上，两边统一。
pub(super) fn provider_include_v1(provider: &AiProvider, outbound: &str) -> Option<bool> {
    match outbound {
        "anthropic" => provider.anthropic_include_v1,
        "openai" => provider.openai_include_v1,
        _ => None,
    }
}

/// 生成本地代理的每次启动随机鉴权 token（32 字节加密随机 → hex）。
/// 工具通过 env / 配置文件携带该 token 访问本地代理，真实上游 key 不再暴露给工具进程。
pub(super) fn fresh_proxy_token() -> String {
    let mut buf = [0u8; 32];
    match getrandom::getrandom(&mut buf) {
        Ok(()) => buf.iter().map(|b| format!("{:02x}", b)).collect(),
        Err(_) => {
            // OS RNG 不可用（几乎不发生）：回退时间戳+进程号组合
            let ns = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!("tok_{}_{}", ns, std::process::id())
        }
    }
}

/// 为指定工具启动一个本地代理（按需、空闲端口、后台 spawn）。
/// CLI 工具与 GUI/桌面应用共用：返回 (监听端口, abort_handle, 本次启动的鉴权 token)；
/// 若未配置 Provider/Key、无可用出站协议或绑定失败，则返回 (0, None, "")（调用方应回退普通逻辑）。
pub(crate) async fn start_tool_proxy(
    tool_config: &ToolConfig,
    provider: Option<&AiProvider>,
    config: &AiConfig,
    req: &LaunchAiToolRequest,
) -> (u16, Option<tokio::task::AbortHandle>, String) {
    start_tool_proxy_with_collab(tool_config, provider, config, req, None, None).await
}

/// 带协作上下文的代理启动（collab 调用时传入 app_handle / room_id）
pub(crate) async fn start_tool_proxy_with_collab(
    tool_config: &ToolConfig,
    provider: Option<&AiProvider>,
    config: &AiConfig,
    req: &LaunchAiToolRequest,
    app_handle: Option<tauri::AppHandle>,
    collab_room_id: Option<String>,
) -> (u16, Option<tokio::task::AbortHandle>, String) {
    if !crate::runtime_tasks::is_enabled("ai") {
        return (0, None, String::new());
    }
    let inbound_protocols = tool_config.inbound_protocols();
    let primary_inbound = tool_config.native_protocol();

    eprintln!(
        "\n[proxy] inbound_protocols={:?}, primary={}",
        inbound_protocols, primary_inbound
    );

    // 根据供应商已配置的协议 URL 选择出站协议：优先工具原生协议（同协议直连），
    // 否则取供应商首个支持的协议（由代理做协议转换）。
    let chosen_outbound = provider
        .as_ref()
        .and_then(|p| pick_outbound_protocol(&primary_inbound, p))
        .unwrap_or_default();

    let mut proxy_port: u16 = 0;
    let mut abort_handle: Option<tokio::task::AbortHandle> = None;
    // 独立启动路径（非协同）启用每次启动的随机鉴权 token；协同房间代理保持原有
    // 总线 token 校验流程，不额外引入 per-start token 以免破坏已存活的委派工具。
    let auth_token = if collab_room_id.is_none() {
        fresh_proxy_token()
    } else {
        String::new()
    };
    // 明确的「不启动」诊断，避免静默 (0, None) 让调用方/用户无从排查
    if provider.is_none() {
        eprintln!("[proxy] ✗ 代理未启动: 未传入 provider（工具未绑定供应商或未选模型）");
        return (0, None, String::new());
    }
    let p = provider.unwrap();
    if p.api_key.is_empty() {
        eprintln!(
            "[proxy] ✗ 代理未启动: provider '{}' 的 api_key 为空（请在设置里配置密钥）",
            p.id
        );
        return (0, None, String::new());
    }
    if chosen_outbound.is_empty() {
        eprintln!(
            "[proxy] ✗ 代理未启动: 无可用出站协议 (inbound={}, provider '{}' 未配置匹配的协议 URL)",
            primary_inbound, p.id
        );
        return (0, None, String::new());
    }
    if let Some(p) = provider {
        if !p.api_key.is_empty() && !chosen_outbound.is_empty() {
            let outbound_protocol = chosen_outbound.clone();
            let upstream_base_url = p.url_for(&outbound_protocol);
            let proxy_settings = &registry().terminals().proxy_settings;
            let timeout = proxy_settings.timeout_seconds as u64;

            let conversion_mode =
                crate::proxy::types::derive_conversion_mode(&primary_inbound, &outbound_protocol);

            // 模型伪装：声明名 C → 实际模型 B；masquerade_model 为空表示不伪装。
            let target_model = req.model_id.clone().unwrap_or_default();
            let mut model_aliases: HashMap<String, String> = HashMap::new();
            // 声明名 C 的唯一来源：Claude Desktop 与 ChatGPT 桌面端需要伪装（它们的选择器
            // 只认自家官方模型名），其余工具原样。**必须与写工具配置那条路径调同一个
            // 函数** —— 否则会出现「配置写 A、这里注册 B」，工具发的名字无人认领 → 400。
            let effective_claimed =
                crate::commands::ai::tool_config_custom::effective_claimed_model(
                    &tool_config.id,
                    &target_model,
                    req.masquerade_model.as_deref().unwrap_or(""),
                );
            let c_norm = effective_claimed
                .replace("[1m]", "")
                .replace("[1M]", "")
                .trim()
                .to_string();
            if !c_norm.is_empty() && c_norm != target_model {
                model_aliases.insert(c_norm, target_model.clone());
            }

            // fallback/小模型伪装映射：声明名 C_small → 实际模型 B_small。
            if let Some(ref fb) = req.fallback_model_id {
                if !fb.is_empty() {
                    let claimed_requested = match &req.fallback_masquerade_model {
                        Some(c) if !c.is_empty() => c.clone(),
                        _ => format_model_name(fb, &tool_config),
                    };
                    let claimed_norm = claimed_requested
                        .replace("[1m]", "")
                        .replace("[1M]", "")
                        .trim()
                        .to_string();
                    if !claimed_norm.is_empty() {
                        model_aliases.insert(claimed_norm, fb.clone());
                    }
                }
            }

            // 补登：工具配置文件实际写入的模型名带 modelFormat 前缀（如 anyversion/gpt-4o），
            // 而下面的 routes/aliases 仅按无前缀真实 id 建表。若不补，代理收到带前缀的声明名
            // → 查不到路由 → 回落上游并带错模型名 → 404 / 上游忽略未知模型用默认（"官方模型"）。
            let claimed_cfg = req
                .masquerade_model
                .clone()
                .filter(|c| !c.is_empty())
                .or_else(|| req.model_id.clone())
                .unwrap_or_default();
            let claimed_fmt = format_model_name(&claimed_cfg, &tool_config);
            if !claimed_fmt.is_empty() && claimed_fmt != target_model {
                model_aliases.insert(claimed_fmt.clone(), target_model.clone());
            }

            // 跨供应商路由：按实际模型名 → 其所属供应商的端点+key。
            // 自定义请求头跟随各自供应商，保证「大模型 / 辅助模型」分属不同网关时都能带上
            // 自己需要的头（抄自 CodexPlusPlus ea0ac5d）。
            let main_headers = crate::proxy::headers::normalize(&p.custom_headers);
            let main_include_v1 = provider_include_v1(p, &chosen_outbound);
            let mut model_routes: HashMap<String, ModelRoute> = HashMap::new();
            // 用户在聚合链上对该「供应商 + 模型」显式声明的图片能力（None = 交给注册表）。
            // 声明同时供单工具代理与聚合服务使用，两条路径的判定必须一致。
            let chain = crate::commands::ai::route::normalize_route_chain(
                &config.route_chain,
                &config.providers,
            );
            let declared_main = req
                .model_id
                .as_deref()
                .map(|mid| crate::commands::ai::route::declared_image_support(&chain, &p.id, mid))
                .unwrap_or(None);
            if let Some(ref mid) = req.model_id {
                if !mid.is_empty() {
                    model_routes.insert(
                        mid.clone(),
                        ModelRoute {
                            base_url: p.url_for(&chosen_outbound),
                            api_key: p.api_key.clone(),
                            headers: main_headers.clone(),
                            include_v1: main_include_v1,
                            supports_image: declared_main,
                        },
                    );
                }
            }
            if let Some(ref fb) = req.fallback_model_id {
                if !fb.is_empty() {
                    if let Some(fp) = req
                        .fallback_provider_id
                        .as_ref()
                        .and_then(|pid| config.providers.iter().find(|pr| &pr.id == pid))
                    {
                        model_routes.insert(
                            fb.clone(),
                            ModelRoute {
                                base_url: fp.url_for(&chosen_outbound),
                                api_key: fp.api_key.clone(),
                                headers: crate::proxy::headers::normalize(&fp.custom_headers),
                                include_v1: provider_include_v1(fp, &chosen_outbound),
                                supports_image: crate::commands::ai::route::declared_image_support(
                                    &chain, &fp.id, fb,
                                ),
                            },
                        );
                    }
                }
            }

            // 补登带前缀的 claimed 模型名到路由表（C → 供应商端点），与上方 aliases 配对，
            // 使代理能识别工具发出的 anyversion/xxx 并正确路由。
            if !claimed_fmt.is_empty() && claimed_fmt != target_model {
                model_routes.insert(
                    claimed_fmt.clone(),
                    ModelRoute {
                        base_url: p.url_for(&chosen_outbound),
                        api_key: p.api_key.clone(),
                        headers: main_headers.clone(),
                        include_v1: main_include_v1,
                        // 带前缀的名字就是实际模型名的别名，声明跟着模型走
                        supports_image: declared_main,
                    },
                );
            }

            // 优化器 / 整流器：工具支持时可由启动请求开关覆盖，否则继承全局配置
            let optimizer_on = tool_config.supports_optimizer
                && req.optimizer_enabled.unwrap_or(true)
                && config.optimizer.enabled;
            let rectifier_on = tool_config.supports_rectifier
                && req.rectifier_enabled.unwrap_or(true)
                && config.rectifier.enabled;

            // 协议回退端点：持有"另一协议"的 URL+key，供 anthropic 出站遇
            // 401/404（上游实为 OpenAI 兼容）时自动以 openai 出站（a2o）重发。
            let (fallback_base_url, fallback_api_key) = match chosen_outbound.as_str() {
                "anthropic" => (
                    if p.openai_url.is_empty() {
                        String::new()
                    } else {
                        p.url_for("openai")
                    },
                    p.api_key.clone(),
                ),
                "openai" => (
                    if p.anthropic_url.is_empty() {
                        String::new()
                    } else {
                        p.url_for("anthropic")
                    },
                    p.api_key.clone(),
                ),
                _ => (String::new(), String::new()),
            };

            // 绑定空闲端口（OS 分配，避免冲突）
            match crate::proxy::server::bind_free_port(&proxy_settings.listen_address) {
                Ok((port, listener)) => {
                    proxy_port = port;
                    let listen_addr = proxy_settings.listen_address.clone();
                    let proxy_config = crate::proxy::types::ProxyConfig {
                        listen_address: listen_addr,
                        listen_port: port,
                        source: "ai".to_string(),
                        auth_token: auth_token.clone(),
                        inbound_protocols: inbound_protocols.clone(),
                        outbound_protocol: outbound_protocol.clone(),
                        conversion_mode,
                        upstream_api_key: p.api_key.clone(),
                        upstream_base_url: upstream_base_url.clone(),
                        fallback_base_url: fallback_base_url.clone(),
                        fallback_api_key: fallback_api_key.clone(),
                        // 渠道校验兜底是 WorkBuddy 后端专用能力，通用 AI 代理不启用
                        channel_check_alt_base_url: None,
                        // 截断流检测对通用代理同样有意义（默认开）
                        stream_truncation_guard: true,
                        // 纯思考轮次把思考当正文下发（默认开，与 ProxyConfig::default 一致）
                        reasoning_as_content: true,
                        // 会话头族是 WorkBuddy 后端专用能力，通用 AI 代理不启用
                        workbuddy_session_headers: false,
                        model_routes,
                        upstream_headers: main_headers,
                        upstream_include_v1: provider_include_v1(p, &chosen_outbound),
                        upstream_is_aggregate: is_aggregate_upstream(
                            &upstream_base_url,
                            config.aggregate.port,
                        ),
                        models_path: "/models".to_string(),
                        models_filter_non_chat: false,
                        force_upstream_stream: false,
                        target_model,
                        timeout_secs: timeout,
                        model_aliases,
                        default_model: req.model_id.clone(),
                        tool_id: req.tool_id.clone(),
                        provider_id: p.id.clone(),
                        rectifier_enabled: rectifier_on,
                        rectifier_thinking_signature: req
                            .rectifier_thinking_signature
                            .unwrap_or(config.rectifier.thinking_signature),
                        rectifier_thinking_budget: req
                            .rectifier_thinking_budget
                            .unwrap_or(config.rectifier.thinking_budget),
                        rectifier_media_fallback: req
                            .rectifier_media_fallback
                            .unwrap_or(config.rectifier.media_fallback),
                        rectifier_media_heuristic: req
                            .rectifier_media_heuristic
                            .unwrap_or(config.rectifier.media_heuristic),
                        rectifier_protocol_mismatch: req
                            .rectifier_protocol_mismatch
                            .unwrap_or(config.rectifier.protocol_mismatch),
                        rectifier_tool_enum_type: true,
                        rectifier_toolcall_dialect: req
                            .rectifier_toolcall_dialect
                            .unwrap_or(config.rectifier.toolcall_dialect),
                        optimizer_enabled: optimizer_on,
                        optimizer_cache_injection: req
                            .optimizer_cache_injection
                            .unwrap_or(config.optimizer.cache_injection),
                        optimizer_thinking: req
                            .optimizer_thinking
                            .unwrap_or(config.optimizer.thinking_optimizer),
                        optimizer_deepseek: req
                            .optimizer_deepseek
                            .unwrap_or(config.optimizer.deepseek_normalize),
                        app_handle: app_handle.clone(),
                        collab_room_id: collab_room_id.clone(),
                    };
                    eprintln!(
                        "[proxy] ✓ 启动代理 -> 127.0.0.1:{}  ({} -> {})",
                        port, primary_inbound, outbound_protocol
                    );
                    let handle = tokio::spawn(async move {
                        if let Err(e) =
                            crate::proxy::server::serve_proxy(proxy_config, listener).await
                        {
                            eprintln!("[proxy] 代理错误: {}", e);
                        }
                    });
                    abort_handle = Some(handle.abort_handle());
                    // 必须等到代理真正可服务再返回端口，否则子进程连上未就绪端口会
                    // 触发 undici "fetch failed"（即 dead port）。未就绪则中止并返回 0。
                    let ready = wait_for_proxy_ready(&proxy_settings.listen_address, port).await;
                    if !ready {
                        eprintln!("[proxy] ⚠ 代理就绪检查失败, 中止: port={}", port);
                        handle.abort();
                        return (0, None, String::new());
                    }
                    if !crate::runtime_tasks::track_task(
                        "ai",
                        &format!("tool-proxy-{port}"),
                        handle,
                    ) {
                        return (0, None, String::new());
                    }
                }
                Err(e) => {
                    eprintln!("[proxy] ✗ 绑定空闲端口失败: {}", e);
                }
            }
        }
    }
    (proxy_port, abort_handle, auth_token)
}
