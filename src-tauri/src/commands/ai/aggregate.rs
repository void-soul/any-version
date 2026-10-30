//! 聚合服务：本地 HTTP 聚合代理（按聚合链顺序请求，可启停 + 日志回传）。
//!
//! - 对外单一模型入口：`kiro-proxy`（聚合 = 多个候选对内的单一入口）
//! - 入口协议：OpenAI `/v1/chat/completions` + Anthropic `/v1/messages`
//! - 候选出口协议：按供应商配置的 `openai_url` / `anthropic_url` / `google_url` 决定
//!   （openai → anthropic → google 优先级），协议转换复用 `proxy::transform` / `proxy::google`
//! - 链路顺序：按 `route_chain` 依次尝试；失败按类别决定重试/切换，并进入冷却
//! - 上下文限制：请求超过 `AggregateConfig.context_limit` 时裁剪最早的非 system 消息
//! - 压缩：Headroom 开启时先走 `/v1/compress`，并用 `frozen_message_count` 保持多轮前缀缓存
//! - 链路热更新：每次请求实时读取配置，改完链路无需重启服务

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use super::config::{load_ai_config, save_ai_config_to_file};
use super::models::{AggregateConfig, AiConfig, AiProvider, RouteCandidate};
use super::route::normalize_route_chain;
use super::usage::{log_usage_db_timed, log_usage_entry, log_usage_failure, UsageEntry};
use crate::proxy::optimizers;
use crate::proxy::types::{ModelRoute, ProxyConfig};

/// 聚合服务对外只暴露一个模型（对内才按链分发）——这也是「聚合」的含义。
pub const AGGREGATE_MODEL_ID: &str = "kiro-proxy";

// ─── 失败分类与切换策略（纯逻辑，便于单测） ───

pub use crate::proxy::failure::{classify_failure, FailureClass};
// ─── 配置读写 ───

/// 读取聚合服务配置。
#[tauri::command]
pub fn get_aggregate_config() -> AggregateConfig {
    load_ai_config().aggregate
}

/// 保存聚合服务配置（运行中不允许改端口，需先停止）。
#[tauri::command]
pub fn save_aggregate_config(config: AggregateConfig) -> Result<AggregateConfig, String> {
    let mut sanitized = config.clone();
    sanitized.retry_count = sanitized.retry_count.clamp(1, 5);
    // 入口模型名：trim，留空回落默认 kiro-proxy
    sanitized.entry_model = entry_model(&sanitized);
    let mut current = load_ai_config();
    if running_port().is_some() && current.aggregate.port != sanitized.port {
        return Err("聚合服务运行中不能修改端口，请先停止服务".to_string());
    }
    current.aggregate = sanitized.clone();
    save_ai_config_to_file(&current)?;
    Ok(sanitized)
}

// ─── 启停 / 状态 ───

/// 启动聚合服务。链路支持热更新：启动后修改聚合链无需重启。
#[tauri::command]
pub async fn start_aggregate_service(app: tauri::AppHandle) -> Result<AggregateStatus, String> {
    crate::runtime_tasks::require_enabled("ai")?;
    let config = load_ai_config();
    let port = config.aggregate.port;
    if let Some(p) = running_port() {
        emit_aggregate_log(
            &app,
            "start",
            "warn",
            format!("聚合服务已在端口 {} 运行", p),
        );
        return Ok(AggregateStatus {
            running: true,
            port: p,
            candidate_count: 0,
            detail: "已在运行".to_string(),
        });
    }
    let chain = normalize_route_chain(&config.route_chain, &config.providers);
    let candidates = build_candidates(&config.providers, &chain, port);
    if let Some(owner) = crate::commands::utils::port_conflict_description(port) {
        return Err(format!("端口 {} 已被占用：{}，请更换端口", port, owner));
    }

    let state = AggState {
        app: app.clone(),
        port,
        health: Arc::new(Mutex::new(HashMap::new())),
        prefix_cache: Arc::new(Mutex::new(None)),
        chain_signature: Arc::new(Mutex::new(None)),
    };

    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        // 与协议转换代理的路由保持一致：同时挂「带 /v1」与「不带 /v1」两种前缀，
        // 否则把 baseUrl 配成不含 /v1 的工具（opencode 系自己补 /chat/completions）会 404。
        .route("/v1/chat/completions", post(chat_completions))
        .route("/chat/completions", post(chat_completions))
        .route("/v1/messages", post(messages))
        .route("/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/messages/count_tokens", post(count_tokens))
        .route("/v1/models", get(list_models))
        .route("/models", get(list_models))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("监听 127.0.0.1:{} 失败: {}", port, e))?;

    let handle = crate::runtime_tasks::spawn_owned("ai", "aggregate", async move {
        if let Err(e) = axum::serve(listener, router).await {
            eprintln!("[aggregate] 服务异常退出: {}", e);
        }
    })
    .ok_or("聚合服务已在运行或模块已禁用")?;
    *AGGREGATE.get_or_init(|| Mutex::new(None)).lock().unwrap() =
        Some(AggregateEntry { port, handle });

    // 记下真实候选数，供状态查询返回（以前恒为 0）
    AGG_CANDIDATE_COUNT.store(candidates.len(), Ordering::Relaxed);

    if candidates.is_empty() {
        emit_aggregate_log(
            &app,
            "start",
            "warn",
            "聚合链为空，服务已启动但请求会失败；请在聚合页勾选候选（支持热更新，无需重启）"
                .to_string(),
        );
    } else {
        emit_aggregate_log(
            &app,
            "start",
            "info",
            format!(
                "聚合服务已启动：http://127.0.0.1:{}（对外模型 {}；{} 个候选；上下文上限 {} tokens）",
                port,
                entry_model(&config.aggregate),
                candidates.len(),
                config.aggregate.context_limit
            ),
        );
    }
    Ok(AggregateStatus {
        running: true,
        port,
        candidate_count: candidates.len(),
        detail: "已启动".to_string(),
    })
}

/// 停止聚合服务。
#[tauri::command]
pub fn stop_aggregate_service(app: tauri::AppHandle) -> Result<AggregateStatus, String> {
    let entry = AGGREGATE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "聚合服务状态锁损坏".to_string())?
        .take();
    let Some(entry) = entry else {
        emit_aggregate_log(&app, "stop", "warn", "聚合服务未在运行".to_string());
        return Ok(AggregateStatus {
            running: false,
            port: 0,
            candidate_count: 0,
            detail: "未运行".to_string(),
        });
    };
    entry.handle.abort();
    emit_aggregate_log(
        &app,
        "stop",
        "info",
        format!("聚合服务已停止（端口 {}）", entry.port),
    );
    Ok(AggregateStatus {
        running: false,
        port: entry.port,
        candidate_count: 0,
        detail: "已停止".to_string(),
    })
}

/// 聚合服务状态（含端口健康检查）。
#[tauri::command]
pub async fn get_aggregate_status() -> AggregateStatus {
    let Some(port) = running_port() else {
        return AggregateStatus {
            running: false,
            port: 0,
            candidate_count: 0,
            detail: "未运行".to_string(),
        };
    };
    let alive = reqwest::get(format!("http://127.0.0.1:{}/health", port))
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    AggregateStatus {
        running: alive,
        port,
        candidate_count: AGG_CANDIDATE_COUNT.load(Ordering::Relaxed),
        detail: if alive {
            "运行中".to_string()
        } else {
            "服务无响应".to_string()
        },
    }
}

// ─── 处理函数 ───

/// 对外暴露的入口模型名：取配置（可配置，默认 kiro-proxy），空值回落默认。
pub(crate) fn entry_model(config: &AggregateConfig) -> String {
    let name = config.entry_model.trim();
    if name.is_empty() {
        AGGREGATE_MODEL_ID.to_string()
    } else {
        name.to_string()
    }
}

/// 对外只暴露一个模型（聚合 = 多个候选对内的单一入口），内部按链分发到具体模型。
async fn list_models() -> Json<Value> {
    let name = entry_model(&load_ai_config().aggregate);
    Json(serde_json::json!({
        "object": "list",
        "data": [{
            "id": name,
            "object": "model",
            "owned_by": "aggregate",
        }]
    }))
}

async fn chat_completions(
    State(state): State<AggState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    forward(state, "openai", &headers, body).await
}

async fn messages(
    State(state): State<AggState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    forward(state, "anthropic", &headers, body).await
}

/// `/v1/messages/count_tokens`：Anthropic 入站时的 token 估算（Claude Code 会先探测这个端点）。
/// 与协议转换代理同款估算（按 UTF-16 长度 / 4），够用即可——真正计费以真实上游 usage 为准。
async fn count_tokens(Json(body): Json<Value>) -> Json<Value> {
    let text_len = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().map(|m| m.to_string()).collect::<String>().len())
        .unwrap_or(0);
    Json(serde_json::json!({ "input_tokens": (text_len / 4).max(1) }))
}

/// 请求体中实际携带的 messages 数组（两种入口协议字段相同）。
fn messages_of(body: &Value) -> Vec<Value> {
    body.get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default()
}

mod health_policy;
pub use health_policy::*;

mod candidates;
pub use candidates::*;

mod context;
pub use context::*;

mod forward;
pub use forward::*;

mod upstream_response;
pub use upstream_response::*;

#[cfg(test)]
mod tests;
