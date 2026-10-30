use axum::response::IntoResponse;
// dispatch responsibility.
use super::{
    apply_masquerade, build_aliases, build_upstream_request, build_upstream_url, convert,
    convert_request, emit_proxy_event, get, google, json, log_proxy, log_proxy_json_full,
    optimizers, process_response, proxy_debug_enabled, push_request_log, upstream_error, HeaderMap,
    Instant, Json, ProxyState, Response, SessionIds, StatusCode, UpstreamErrorKind, Value,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  统一处理管线
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 请求处理统一入口。
/// - `inbound`：工具说话的协议（"anthropic" | "openai" | "google"）
/// - `claimed_model`：声明名 C（anthropic/openai 来自 body.model；google 来自 URL path）
/// - `google_is_stream`：仅 google 入站有意义
/// - `body`：P_in 形态请求体
pub(super) async fn process_request(
    state: &ProxyState,
    headers: &HeaderMap,
    inbound: &str,
    claimed_model: String,
    google_is_stream: bool,
    mut body: Value,
) -> Response {
    {
        let mut stats = state.stats.write().await;
        stats.total_requests += 1;
    }
    let start = Instant::now();

    let config = state.config.read().await.clone();
    let outbound = config.outbound_protocol.clone();
    let aliases = build_aliases(&config);
    let log_source = config.source.clone();

    log_proxy(&format!(
        "← IN   [{}] model={}  → OUT [{}]  (转换: {})",
        inbound, claimed_model, outbound, config.conversion_mode
    ));
    push_request_log(
        &log_source,
        format!(
            "IN [{}] model={} → OUT [{}]",
            inbound, claimed_model, outbound
        ),
    );

    // ─── 详细请求日志 ───
    let is_stream = if inbound == "google" {
        google_is_stream
    } else {
        body.get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    let msg_count = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let first_msg_preview = body
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|a| a.last())
        .and_then(|m| {
            m.get("content").and_then(|c| match c {
                Value::String(s) => Some(s.as_str()),
                Value::Array(parts) => parts
                    .iter()
                    .find_map(|p| p.get("text").and_then(|t| t.as_str())),
                _ => None,
            })
        })
        .unwrap_or("");
    let preview: String = first_msg_preview.chars().take(80).collect();
    log_proxy(&format!(
        "  请求详情: messages={}, stream={}, preview=\"{}\"",
        msg_count, is_stream, preview
    ));

    // ─── emit collab:proxy-request ───
    emit_proxy_event(
        &state,
        "collab:proxy-request",
        json!({
            "model": claimed_model,
            "messages": msg_count,
            "stream": is_stream,
        }),
    );

    // 会话头族（WorkBuddy 专用）：**必须在 body 被改写之前**算 ——
    // 轮级锚点是「入站原始 messages 里最后一条 user 消息」，伪装/转换后就不再是它了。
    // 会话级与轮级全轮恒定（含整流、协议回退、渠道校验换路径），消息级与 trace 每次出站新生成。
    let session = if config.workbuddy_session_headers {
        Some(SessionIds::from_request(&body, headers))
    } else {
        None
    };

    // ② 模型伪装：C → B（替换 body.model，google 入站不写 body）
    let actual_model = apply_masquerade(inbound, &mut body, &claimed_model, aliases.as_ref());

    // ③ 协议转换：P_in → P_out（已在 body 写入 B）
    let mut out_body = convert_request(inbound, &outbound, &body, &actual_model);

    // ③.5 角色归一化：出站 OpenAI 时把 `developer` 改成 `system`。
    // Codex 的 Responses 请求把系统提示拆成多条 `role: "developer"` 塞在 input 里，
    // 转换时会被原样搬过来；只认老角色的第三方网关（实测 WorkBuddy）会回 400
    // `11128 Illegal API invocation from an unapproved channel`，报文像是渠道问题，
    // 实际是角色不认。放在优化器之前，整流器看到的也是归一化后的角色。
    if outbound == "openai" {
        crate::proxy::convert::normalize_openai_message_roles(&mut out_body);
        crate::proxy::convert::collapse_system_messages_to_head(&mut out_body);
        crate::proxy::convert::normalize_image_data_urls_for_model(&mut out_body, &actual_model);
    }

    // ④ 优化器（P_out 形态）
    optimizers::apply_optimizers(&mut out_body, &outbound, &config);

    // ⑤ 预防式整流（P_out 形态）
    optimizers::apply_preventive_rectifiers(&mut out_body, &outbound, &config);

    // ⑥ 转发
    // 出站为 OpenAI / Anthropic 时，确保请求体带 stream 字段：
    // Google 入站（流式靠 URL 判定）以及跨协议转换后的 body 可能未写入 `stream`，
    // 而 OpenAI / Anthropic 上游依赖 body 中的 stream 字段决定是否流式返回。
    //
    // `upstream_stream` 与 `is_stream` 分开：前者是「发给上游要不要流式」，
    // 后者是「客户端要不要流式」。配置了 force_upstream_stream 的上游只收流式，
    // 此时即使客户端要非流式也强发流式，响应再由 process_response 聚合回去。
    let upstream_stream = is_stream || config.force_upstream_stream;
    if upstream_stream && (outbound == "openai" || outbound == "anthropic") {
        if let Some(o) = out_body.as_object_mut() {
            o.insert("stream".into(), json!(true));
        }
    }
    let (upstream_url, auth_name, route_api_key, route_headers) =
        build_upstream_url(&config, &outbound, &actual_model, upstream_stream);
    if upstream_url.is_empty() {
        let mut stats = state.stats.write().await;
        stats.failed_requests += 1;
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"message": "未配置对应出站协议的上游 URL"}})),
        )
            .into_response();
    }

    log_proxy(&format!("→ OUT  POST {} auth={}", upstream_url, auth_name));
    log_proxy(&format!(
        "  出站体: model={}, stream={}",
        actual_model, is_stream
    ));
    if proxy_debug_enabled() {
        log_proxy_json_full("  入站完整请求体(P_in)", &body);
        log_proxy_json_full("  出站完整请求体(P_out)", &out_body);
    }

    let req = build_upstream_request(
        &state.client,
        headers,
        &upstream_url,
        &auth_name,
        &route_api_key,
        &route_headers,
        &out_body,
        session.as_ref(),
    );
    let upstream_resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let mut stats = state.stats.write().await;
            stats.failed_requests += 1;
            log_proxy(&format!(
                "✗ OUT  POST {} 请求失败: {}  ({}ms)",
                upstream_url,
                e,
                start.elapsed().as_millis()
            ));
            push_request_log(&log_source, format!("✗ 上游请求失败: {e}"));
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": format!("上游请求失败: {}", e)}})),
            )
                .into_response();
        }
    };

    let mut status = upstream_resp.status();
    log_proxy(&format!(
        "← UPSTREAM {}  ({}ms)",
        status.as_u16(),
        start.elapsed().as_millis()
    ));
    push_request_log(
        &log_source,
        format!(
            "← 上游 {} ({}ms)",
            status.as_u16(),
            start.elapsed().as_millis()
        ),
    );

    // ─── emit collab:proxy-response-start ───
    emit_proxy_event(
        &state,
        "collab:proxy-response-start",
        json!({
            "status": status.as_u16(),
            "elapsed_ms": start.elapsed().as_millis(),
        }),
    );

    if !status.is_success() {
        let mut error_body = upstream_resp.text().await.unwrap_or_default();
        let mut kind = upstream_error::classify(status.as_u16(), &error_body);
        let mut err_label = upstream_error::label(kind, status.as_u16(), &error_body);
        log_proxy(&format!(
            "✗ UPSTREAM {} [{}] error: {}",
            status.as_u16(),
            err_label,
            error_body.chars().take(300).collect::<String>()
        ));
        push_request_log(
            &log_source,
            format!("✗ 上游 {} {}", status.as_u16(), err_label),
        );

        // ─── emit collab:proxy-error ───
        emit_proxy_event(
            &state,
            "collab:proxy-error",
            json!({
                "status": status.as_u16(),
                "error": error_body.chars().take(500).collect::<String>(),
            }),
        );

        // ⑦-0 渠道校验：换路径重发。**只换 URL**，body 与认证一律不动
        //（实测校验对象是 URL + 会话头族，往 body 塞 conversationId 无效）。
        // 只有 2API 填了 `channel_check_alt_base_url` 才启用 —— 通用代理与聚合不受影响。
        // 换路径失败就接着走下面的整流/协议回退，不新增硬失败点。
        if kind == UpstreamErrorKind::ChannelCheck && !config.upstream_is_aggregate {
            if let Some(alt) = config
                .channel_check_alt_base_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let (alt_url, alt_auth) = crate::proxy::upstream::resolve_url(
                    &outbound,
                    alt,
                    &actual_model,
                    upstream_stream,
                    Some(false),
                );
                if !alt_url.is_empty() && alt_url != upstream_url {
                    log_proxy(&format!(
                        "↻ 渠道校验 → 换路径: POST {} auth={}",
                        alt_url, alt_auth
                    ));
                    push_request_log(&log_source, format!("↻ 渠道校验命中，换路径重试 {alt_url}"));
                    let alt_req = build_upstream_request(
                        &state.client,
                        headers,
                        &alt_url,
                        alt_auth,
                        &route_api_key,
                        &route_headers,
                        &out_body,
                        session.as_ref(),
                    );
                    match alt_req.send().await {
                        Ok(alt_resp) if alt_resp.status().is_success() => {
                            log_proxy(&format!(
                                "✓ 换路径成功  ({}ms)",
                                start.elapsed().as_millis()
                            ));
                            push_request_log(&log_source, "✓ 渠道校验兜底成功".to_string());
                            return process_response(
                                state,
                                alt_resp,
                                &config,
                                inbound,
                                &outbound,
                                &claimed_model,
                                &actual_model,
                                is_stream,
                                upstream_stream,
                            )
                            .await;
                        }
                        Ok(alt_resp) => {
                            status = alt_resp.status();
                            error_body = alt_resp.text().await.unwrap_or_default();
                            kind = upstream_error::classify(status.as_u16(), &error_body);
                            // 回退路径上的 11102 只说明「这条路没这个模型」，不是模型真不存在：
                            // 既不能对外宣称模型不可用，也不值得再整流/再回退。
                            if kind == UpstreamErrorKind::ModelMissing {
                                kind = UpstreamErrorKind::FallbackModelMissing;
                            }
                            err_label = upstream_error::label(kind, status.as_u16(), &error_body);
                            log_proxy(&format!(
                                "✗ 换路径也失败: {} [{}]",
                                status.as_u16(),
                                err_label
                            ));
                            push_request_log(
                                &log_source,
                                format!("✗ 换路径失败 {} {}", status.as_u16(), err_label),
                            );
                        }
                        Err(e) => {
                            log_proxy(&format!("✗ 换路径请求失败: {e}"));
                            push_request_log(&log_source, format!("✗ 换路径请求失败: {e}"));
                        }
                    }
                }
            }
        }

        // ⑦ 反应式整流：修正后重试一次
        // 上游是聚合时跳过：聚合自己会按失败类别重试/切换/冷却，代理再整流重试是重复放大。
        // 业务码已定性为「不是字段问题」时同样跳过（401/11148/渠道校验…重发只是多打一次上游）。
        if !config.upstream_is_aggregate && kind.worth_rectify() {
            if let Some(rectified) = optimizers::try_reactive_rectify(
                status.as_u16(),
                &error_body,
                &out_body,
                &config,
                &outbound,
            ) {
                let retry_req = build_upstream_request(
                    &state.client,
                    headers,
                    &upstream_url,
                    &auth_name,
                    &route_api_key,
                    &route_headers,
                    &rectified,
                    session.as_ref(),
                );
                if let Ok(retry_resp) = retry_req.send().await {
                    if retry_resp.status().is_success() {
                        log_proxy(&format!(
                            "↻ retry succeeded after rectify  ({}ms)",
                            start.elapsed().as_millis()
                        ));
                        return process_response(
                            state,
                            retry_resp,
                            &config,
                            inbound,
                            &outbound,
                            &claimed_model,
                            &actual_model,
                            is_stream,
                            upstream_stream,
                        )
                        .await;
                    }
                }
            }
        }

        // ⑦½ 协议回退整流（参考 cc-switch 的 with_fallback 哲学）：
        // anthropic 出站遇 401/404 且配置了 openai 回退端点时，说明该供应商的
        // anthropic_url 实际并不兼容 Anthropic 协议（仅 OpenAI 兼容，如
        // longcat/sensenova）。用"另一协议"端点以 openai 出站（a2o 转换）重发一次。
        // 上游是聚合时同样跳过：聚合自己会做协议转换与切换。
        if outbound == "anthropic"
            && (status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND)
            && !config.fallback_base_url.is_empty()
            && !config.upstream_is_aggregate
        {
            let mut fb_config = config.clone();
            fb_config.outbound_protocol = "openai".to_string();
            // 清空按模型查的 anthropic 路由，强制走 fallback_base_url（openai 端点），
            // 否则 build_upstream_url 会命中 model_routes 里仍为 anthropic 的端点。
            fb_config.model_routes.clear();
            fb_config.upstream_base_url = config.fallback_base_url.clone();
            fb_config.upstream_api_key = config.fallback_api_key.clone();
            // 用原始（已伪装但未转换的）body 重新做 a2o 转换
            let mut fb_body = convert_request(inbound, "openai", &body, &actual_model);
            optimizers::apply_optimizers(&mut fb_body, "openai", &fb_config);
            optimizers::apply_preventive_rectifiers(&mut fb_body, "openai", &fb_config);
            if is_stream && (fb_config.outbound_protocol == "openai") {
                if let Some(o) = fb_body.as_object_mut() {
                    o.insert("stream".into(), json!(true));
                }
            }
            let (fb_url, fb_auth, fb_key, fb_headers) =
                build_upstream_url(&fb_config, "openai", &actual_model, is_stream);
            if !fb_url.is_empty() {
                log_proxy(&format!(
                    "↻ protocol fallback anthropic→openai: POST {} auth={}",
                    fb_url, fb_auth
                ));
                let fb_req = build_upstream_request(
                    &state.client,
                    headers,
                    &fb_url,
                    &fb_auth,
                    &fb_key,
                    &fb_headers,
                    &fb_body,
                    session.as_ref(),
                );
                if let Ok(fb_resp) = fb_req.send().await {
                    if fb_resp.status().is_success() {
                        log_proxy(&format!(
                            "↻ fallback succeeded  ({}ms)",
                            start.elapsed().as_millis()
                        ));
                        return process_response(
                            state,
                            fb_resp,
                            &fb_config,
                            inbound,
                            "openai",
                            &claimed_model,
                            &actual_model,
                            is_stream,
                            fb_config.force_upstream_stream,
                        )
                        .await;
                    }
                    log_proxy(&format!(
                        "↻ fallback also failed: {}",
                        fb_resp.status().as_u16()
                    ));
                }
            }
        }

        {
            let mut stats = state.stats.write().await;
            stats.failed_requests += 1;
        }
        // 失败落账（整流/回退都失败后才到这里）：按模型的成功率靠它做分母，
        // 也顺带把失败类别记下来，便于在用量统计里看出「哪个供应商在报什么错」。
        // 业务码优先：11102 → model_unavailable、14018 → billing、11128 → transient…
        // 业务码不明时才退回按状态码判（保持 AI 模块的既有统计口径）。
        let failure_class = upstream_error::to_failure_class(kind, status.as_u16(), &error_body);
        crate::proxy::hooks::current().record_failure(
            &config.tool_id,
            &actual_model,
            &config.provider_id,
            failure_class.as_str(),
        );
        return (
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(json!({
                "error": { "message": format!("上游返回错误 ({}): {}", status, error_body) }
            })),
        )
            .into_response();
    }

    process_response(
        state,
        upstream_resp,
        &config,
        inbound,
        &outbound,
        &claimed_model,
        &actual_model,
        is_stream,
        upstream_stream,
    )
    .await
}
