//! forward responsibility.
use super::*;

/// 共享转发流程（OpenAI / Anthropic 入口通用）。
pub(super) async fn forward(
    state: AggState,
    inbound: &'static str,
    headers: &axum::http::HeaderMap,
    mut body: Value,
) -> Response {
    // 会话亲和：算一次会话键，供「候选重排」与「成功后记账」共用
    let conv_key = {
        let pairs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.to_str().unwrap_or("")))
            .collect();
        crate::commands::ai::affinity::conversation_id(&pairs, &body, inbound)
    };
    // 每次请求实时读取配置：链路热更新、重试次数与压缩设置即改即生效
    let config = load_ai_config();
    let retry_count = config.aggregate.retry_count.clamp(1, 5) as usize;
    let context_limit = config.aggregate.context_limit;
    // 首字节 / 空闲超时：每次请求实时读取，改完配置即生效（与 retry_count 一致）
    let first_byte_timeout = config.aggregate.first_byte_timeout_secs;
    let idle_timeout = config.aggregate.idle_timeout_secs;
    let headroom = if config.headroom.enabled {
        Some(HeadroomRuntime {
            base_url: format!("http://127.0.0.1:{}", config.headroom.port),
            timeout_ms: config.headroom.timeout_ms.max(200),
            fail_closed: config.headroom.on_unavailable == "failClosed",
        })
    } else {
        None
    };

    // 链路热更新：实时解析候选，变更时打日志
    let chain = normalize_route_chain(&config.route_chain, &config.providers);
    let mut candidates = build_candidates(&config.providers, &chain, state.port);
    // 热更新后候选数会变（用户刚补了 Key / 删了供应商），状态查询要能看到真实值
    AGG_CANDIDATE_COUNT.store(candidates.len(), Ordering::Relaxed);

    // ── 会话亲和：把上次回答这个对话的候选提到最前（保 prompt cache）──
    {
        let keys: Vec<String> = candidates
            .iter()
            .map(|c| format!("{}::{}", c.provider_id, c.model_id))
            .collect();
        let health = state.health.clone();
        let is_resting = move |k: &str| cooldown_remaining_secs(&health, k).is_some();
        let stick = crate::commands::ai::affinity::get(&conv_key, now_ms());
        let decision =
            crate::commands::ai::affinity::decide(stick.as_ref(), &keys, &is_resting, now_ms());
        if decision == crate::commands::ai::affinity::AffinityDecision::Kept {
            let who = stick.as_ref().map(|s| s.who.clone()).unwrap_or_default();
            crate::commands::ai::affinity::promote(&mut candidates, &who, |c| {
                format!("{}::{}", c.provider_id, c.model_id)
            });
            emit_aggregate_log(
                &state.app,
                "route",
                "info",
                format!("会话亲和：沿用上次回答的候选 {}", who),
            );
        }
    }
    let signature = candidates
        .iter()
        .map(|c| format!("{}::{}::{}", c.provider_id, c.model_id, c.outbound.as_str()))
        .collect::<Vec<_>>()
        .join("|");
    {
        let mut seen = state
            .chain_signature
            .lock()
            .unwrap_or_else(|_| panic!("poisoned"));
        if seen.as_deref() != Some(signature.as_str()) {
            if let Some(prev) = seen.as_ref() {
                emit_aggregate_log(
                    &state.app,
                    "route",
                    "info",
                    format!(
                        "聚合链已热更新：{} → {} 个候选",
                        prev.split('|').count(),
                        candidates.len()
                    ),
                );
            } else {
                emit_aggregate_log(
                    &state.app,
                    "route",
                    "info",
                    format!("聚合链加载完成：{} 个候选", candidates.len()),
                );
            }
            *seen = Some(signature);
        }
    }
    if candidates.is_empty() {
        emit_aggregate_log(
            &state.app,
            "route",
            "error",
            "聚合链为空（或候选缺少端点/API Key），请求失败".to_string(),
        );
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": { "message": "聚合链为空：请到 AI「聚合」页勾选候选（支持热更新，改完直接重试）", "type": "aggregate_chain_empty" }
            })),
        )
            .into_response();
    }

    // 多轮前缀缓存：同一会话复用上轮压缩结果，避免打爆 provider KV 缓存
    let messages = messages_of(&body);
    if !messages.is_empty() {
        let cache = state.prefix_cache.lock().ok().and_then(|g| g.clone());
        let (effective, frozen) = apply_frozen_prefix(&messages, cache.as_ref());
        if frozen > 0 {
            body["messages"] = Value::Array(effective.clone());
            body["config"] = serde_json::json!({ "frozen_message_count": frozen });
            emit_aggregate_log(
                &state.app,
                "context",
                "info",
                format!(
                    "命中前缀缓存：{} 条已冻结（省去重新压缩与缓存未命中）",
                    frozen
                ),
            );
        }
    }

    // 上下文限制：裁剪最早的非 system 消息
    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()).cloned() {
        let (kept, trimmed) = trim_messages_to_limit(messages, context_limit);
        if trimmed > 0 {
            body["messages"] = Value::Array(kept);
            emit_aggregate_log(
                &state.app,
                "context",
                "warn",
                format!(
                    "上下文超限，已裁剪 {} 条最早的消息（上限 {} tokens）",
                    trimmed, context_limit
                ),
            );
        }
    }

    // Headroom 压缩旁路（成功后更新前缀缓存）
    let original_messages = messages_of(&body);
    if let Some(hr) = &headroom {
        match compress_body(hr, &body).await {
            Ok(Some(compressed)) => {
                emit_aggregate_log(
                    &state.app,
                    "compress",
                    "info",
                    "已压缩请求上下文".to_string(),
                );
                body = compressed;
                if let Ok(mut guard) = state.prefix_cache.lock() {
                    *guard = Some(PrefixSnapshot {
                        original: original_messages.clone(),
                        forwarded: messages_of(&body),
                    });
                }
            }
            Ok(None) => {}
            Err(err) => {
                if hr.fail_closed {
                    emit_aggregate_log(
                        &state.app,
                        "compress",
                        "error",
                        format!("压缩失败（策略：直接报错）: {}", err),
                    );
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({
                            "error": { "message": format!("Headroom 压缩服务不可用：{}", err), "type": "compression_unavailable" }
                        })),
                    )
                        .into_response();
                }
                emit_aggregate_log(
                    &state.app,
                    "compress",
                    "warn",
                    format!("压缩失败，跳过压缩继续发送: {}", err),
                );
            }
        }
    }

    let started = std::time::Instant::now();
    let stream_requested = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let mut last_error = String::new();

    // 「聚合挂代理上游」：按请求模型名优先路由（匹配不到按链序，入口名 = 无偏好）
    let entry_name = entry_model(&config.aggregate);
    let requested_model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or_default()
        .to_string();
    let candidates = prioritize_by_model(candidates, &requested_model, &entry_name);

    for (idx, candidate) in candidates.iter().enumerate() {
        // 运行期兜底：万一端口改动后候选变成自引用，直接跳过而不是递归打自己
        if is_self_referential(&candidate.base_url, state.port) {
            emit_aggregate_log(
                &state.app,
                "route",
                "error",
                format!(
                    "跳过自引用候选 {} / {}（指向聚合服务自身，会递归）",
                    candidate.provider_name, candidate.model_id
                ),
            );
            continue;
        }
        let key = format!("{}::{}", candidate.provider_id, candidate.model_id);
        if let Some(remaining) = cooldown_remaining_secs(&state.health, &key) {
            emit_aggregate_log(
                &state.app,
                "route",
                "warn",
                format!(
                    "跳过冷却中的候选 #{} {} / {}（剩余 {}s）",
                    idx + 1,
                    candidate.provider_name,
                    candidate.model_id,
                    remaining
                ),
            );
            continue;
        }

        // 整流配置按候选构造（端点/凭据/出站协议取当前候选，开关取全局）——
        // 与单工具协议代理共用同一套整流实现，避免两条路径行为漂移。
        let proxy_cfg = candidate_proxy_config(candidate, &config);
        let mut attempts = 0usize;
        // 同一候选最多做一次反应式整流重试（cc-switch 同款：每个 provider 的每种
        // 整流各只认领一次，避免上游持续报错时无限重试）
        let mut rectified = false;
        let mut rectified_body: Option<Value> = None;
        loop {
            attempts += 1;
            let result = try_candidate(
                &state,
                candidate,
                inbound,
                &body,
                stream_requested,
                rectified_body.as_ref(),
                &proxy_cfg,
                std::time::Duration::from_secs(first_byte_timeout.max(1)),
                std::time::Duration::from_secs(idle_timeout.max(1)),
            )
            .await;
            match result {
                Ok(response) => {
                    clear_health(&state.health, &key);
                    // 会话亲和记账：**只有成功回答后才记**（抄 magpie `answered` 的时机）。
                    // cache_read 先记 None：此刻响应体还没解析（流式的 usage 在生成器内部），
                    // 拿不到证据就不该阻断下一次的粘连。TODO(真机)：非流式路径可从
                    // `finish_non_stream` 的 usage 回填，让「cache 太小就不粘」这条规则真正生效。
                    crate::commands::ai::affinity::record(&conv_key, &key, None, now_ms());
                    if idx > 0 || attempts > 1 {
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "warn",
                            format!(
                                "已切换到候选 #{} {} / {}（{} 协议，第 {} 次尝试）",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                candidate.outbound.as_str(),
                                attempts
                            ),
                        );
                    }
                    emit_aggregate_log(
                        &state.app,
                        "route",
                        "info",
                        format!(
                            "候选 #{} {} / {} 响应成功（{} ms）",
                            idx + 1,
                            candidate.provider_name,
                            candidate.model_id,
                            started.elapsed().as_millis()
                        ),
                    );
                    return response;
                }
                Err(err) => {
                    last_error = format!("{:?}: {}", err.class, err.message);
                    // 策略类拒绝（内容安全等）：不重试、不切换，直接把错误回给客户端
                    if err.class == FailureClass::Fatal {
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "error",
                            format!(
                                "候选 #{} {} / {} 被策略拒绝，不再切换: {}",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                err.message
                            ),
                        );
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": { "message": err.message, "type": "content_policy" }
                            })),
                        )
                            .into_response();
                    }
                    emit_aggregate_log(
                        &state.app,
                        "route",
                        "warn",
                        format!(
                            "候选 #{} {} / {} 第 {} 次失败 [{:?}]: {}",
                            idx + 1,
                            candidate.provider_name,
                            candidate.model_id,
                            attempts,
                            err.class,
                            err.message
                        ),
                    );
                    // 反应式整流（与单工具协议代理同一套实现）：上游报的是「能修的错」
                    // （thinking 签名 / budget、图片不支持、协议残留字段）就修正后重试一次；
                    // 修不了才往下走。这是抄 cc-switch forwarder 的「报错后整流重试」。
                    if !rectified {
                        if let Some(status) = err.status {
                            if let Some(fixed) = optimizers::try_reactive_rectify(
                                status,
                                &err.error_body,
                                &err.outbound_body,
                                &proxy_cfg,
                                candidate.outbound.as_str(),
                            ) {
                                rectified = true;
                                rectified_body = Some(fixed);
                                emit_aggregate_log(
                                    &state.app,
                                    "rectify",
                                    "info",
                                    format!(
                                        "候选 #{} {} / {} 上报 HTTP {}，已整流修正并重发一次",
                                        idx + 1,
                                        candidate.provider_name,
                                        candidate.model_id,
                                        status
                                    ),
                                );
                                continue;
                            }
                        }
                    }
                    // 换家判定（抄 cc-switch）：请求体层面被拒（400/422）换到哪一家都一样，
                    // 直接把上游错误回给客户端 —— 既不白跑整条链，也不误伤其它候选的冷却。
                    if !should_switch_candidate(err.class, err.status) {
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "error",
                            format!(
                                "候选 #{} {} / {} 的请求被上游拒绝且不属候选问题，不再切换: {}",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                err.message
                            ),
                        );
                        let status_code = err
                            .status
                            .and_then(|s| StatusCode::from_u16(s).ok())
                            .unwrap_or(StatusCode::BAD_GATEWAY);
                        return (
                            status_code,
                            Json(serde_json::json!({
                                "error": {
                                    "message": format!(
                                        "候选 {} / {} 拒绝了该请求（整流后仍失败）: {}",
                                        candidate.provider_name, candidate.model_id, err.message
                                    ),
                                    "type": "upstream_rejected_request"
                                }
                            })),
                        )
                            .into_response();
                    }
                    // 达到该类别的重试预算 → 记冷却并切换到下一个候选
                    if attempts >= retry_budget(err.class).min(retry_count.max(1)) {
                        // 厂商可能明确说了「什么时候恢复」→ 优先等到那个点，而不是固定时长
                        let resets_at = parse_resets_at(&err.error_body, now_ms());
                        let cooldown = record_failure(
                            &state.health,
                            &key,
                            err.class,
                            err.retry_after,
                            resets_at,
                        );
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "warn",
                            format!(
                                "候选 #{} {} / {} 进入冷却 {}s，切换到下一个候选",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                cooldown.as_secs()
                            ),
                        );
                        break;
                    }
                }
            }
        }
    }

    emit_aggregate_log(
        &state.app,
        "route",
        "error",
        format!(
            "聚合链全部候选失败（{} 个）: {}",
            candidates.len(),
            last_error
        ),
    );
    (
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({
            "error": {
                "message": format!(
                    "聚合链 {} 个候选全部失败，最后错误：{}。请检查候选的 API Key / 额度，或到「服务」页查看聚合日志",
                    candidates.len(), last_error
                ),
                "type": "aggregate_chain_exhausted"
            }
        })),
    )
        .into_response()
}
