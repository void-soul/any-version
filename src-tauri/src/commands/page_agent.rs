//! Page 模块：通过 CDP 驱动一次性 Edge，运行 page-agent 交互任务。
//!
//! 浏览器只负责承载 page-agent 的页面上下文；AI 请求仍使用用户在 AI 模块配置的
//! OpenAI-compatible provider。每次任务使用独立 profile，避免污染用户浏览器登录态。
//! 浏览器以可见窗口启动，遇到登录墙时用户可以直接在该窗口完成登录。

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type WsWriter = futures_util::stream::SplitSink<Ws, Message>;
type WsReader = futures_util::stream::SplitStream<Ws>;

#[derive(Default)]
pub struct PageAgentState {
    active: Mutex<Option<Arc<AtomicBool>>>,
    answer: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageAgentRunResult {
    pub success: bool,
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageAgentRunRequest {
    pub url: String,
    pub task: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    #[serde(default)]
    pub allow_script: bool,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
}

fn default_max_steps() -> u32 { 25 }

fn emit_event(app: &AppHandle, kind: &str, payload: Value) {
    let event = json!({
        "type": kind,
        "payload": payload,
    });
    let _ = app.emit("page-agent-event", event);
}

fn emit_log(app: &AppHandle, line: impl Into<String>) {
    emit_event(app, "log", json!({ "line": line.into() }));
}

fn browser_candidates(name: &str) -> Vec<PathBuf> {
    let mut result = Vec::new();
    #[cfg(windows)]
    {
        let rel = format!("Microsoft\\Edge\\Application\\{}.exe", name);
        let chrome_rel = format!("Google\\Chrome\\Application\\{}.exe", name);
        for var in ["ProgramFiles(x86)", "ProgramFiles", "LocalAppData"] {
            if let Some(root) = std::env::var_os(var) {
                result.push(PathBuf::from(&root).join(&rel));
                result.push(PathBuf::from(root).join(&chrome_rel));
            }
        }
    }
    result
}

fn find_browser() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for p in browser_candidates("msedge") {
            if p.exists() { return Some(p); }
        }
        for p in browser_candidates("chrome") {
            if p.exists() { return Some(p); }
        }
    }
    #[cfg(not(windows))]
    {
        for name in ["msedge", "google-chrome", "chromium", "chrome"] {
            if let Ok(out) = std::process::Command::new("which").arg(name).output() {
                if out.status.success() {
                    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !p.is_empty() { return Some(PathBuf::from(p)); }
                }
            }
        }
    }
    None
}

fn bridge_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = crate::commands::utils::get_resource_dir() {
        candidates.push(dir.join("page-agent-bridge").join("page-agent-bridge.iife.js"));
        candidates.push(dir.join("page-agent-bridge").join("dist").join("page-agent-bridge.iife.js"));
        candidates.push(dir.join("page-agent-bridge.iife.js"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("page-agent-bridge").join("dist").join("page-agent-bridge.iife.js"));
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    candidates.push(repo.join("..").join("page-agent-bridge").join("dist").join("page-agent-bridge.iife.js"));
    candidates.into_iter().find(|p| p.exists())
}

fn hidden_command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        cmd.creation_flags(0x08000000);
    }
    cmd
}

async fn wait_debug_endpoint(port: u16, cancel: &Arc<AtomicBool>) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| format!("创建 CDP 客户端失败: {e}"))?;
    // /json/version 是 Browser websocket；Page/Runtime 命令必须连接到具体 page target。
    let endpoint = format!("http://127.0.0.1:{port}/json/list");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("任务已停止".to_string());
        }
        if let Ok(resp) = client.get(&endpoint).send().await {
            if let Ok(body) = resp.json::<Vec<Value>>().await {
                if let Some(ws) = body.iter()
                    .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
                    .find_map(|target| target.get("webSocketDebuggerUrl").and_then(Value::as_str))
                {
                    return Ok(ws.to_string());
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("浏览器 CDP 启动超时".to_string());
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn forward_binding(app: &AppHandle, params: &Value) -> Option<(String, Value)> {
    if params.get("name").and_then(Value::as_str) != Some("__kiraPageAgentEmit") {
        return None;
    }
    let raw = params.get("payload").and_then(Value::as_str)?;
    let event: Value = serde_json::from_str(raw).ok()?;
    let kind = event.get("type").and_then(Value::as_str)?.to_string();
    let payload = event.get("payload").cloned().unwrap_or_else(|| {
        let mut copy = event.clone();
        if let Some(obj) = copy.as_object_mut() { obj.remove("type"); }
        copy
    });
    emit_event(app, &kind, payload.clone());
    Some((kind, payload))
}

async fn cdp_command(
    writer: &mut WsWriter,
    reader: &mut WsReader,
    next_id: &mut u64,
    method: &str,
    params: Value,
    app: &AppHandle,
    cancel: &Arc<AtomicBool>,
) -> Result<Value, String> {
    let id = *next_id;
    *next_id += 1;
    writer.send(Message::Text(json!({ "id": id, "method": method, "params": params }).to_string().into()))
        .await
        .map_err(|e| format!("CDP 发送失败: {e}"))?;

    loop {
        let message = tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(100)), if cancel.load(Ordering::Relaxed) => {
                return Err("任务已停止".to_string());
            }
            message = reader.next() => message,
        };
        let Some(Ok(message)) = message else { return Err("CDP 连接已断开".to_string()); };
        let Message::Text(text) = message else { continue; };
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("CDP 响应解析失败: {e}"))?;
        if let Some(params) = value.get("params") {
            if value.get("method").and_then(Value::as_str) == Some("Runtime.bindingCalled") {
                let _ = forward_binding(app, params);
            }
        }
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            if let Some(error) = value.get("error") {
                return Err(format!("CDP {} 失败: {}", method, error));
            }
            return Ok(value.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn select_provider(request: &PageAgentRunRequest) -> Result<(String, String, String), String> {
    let config = crate::commands::ai::config::load_ai_config();
    let provider = if let Some(id) = request.provider_id.as_deref().filter(|s| !s.is_empty()) {
        config.providers.iter().find(|p| p.id == id)
    } else {
        config.providers.iter().find(|p| !p.openai_url.trim().is_empty() && !p.api_key.trim().is_empty())
    }.ok_or_else(|| "未找到可用于 Page 的 OpenAI 兼容供应商，请先在 AI 模块配置模型".to_string())?;
    if provider.openai_url.trim().is_empty() {
        return Err("Page 原型当前需要 OpenAI 兼容端点，所选供应商未配置 OpenAI URL".to_string());
    }
    if provider.api_key.trim().is_empty() {
        return Err("所选 AI 供应商没有 API Key".to_string());
    }
    let model = request.model_id.clone()
        .filter(|id| provider.models.iter().any(|m| &m.id == id))
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| "所选供应商没有可用模型".to_string())?;
    Ok((provider.openai_url.trim_end_matches('/').to_string(), provider.api_key.clone(), model))
}

#[tauri::command]
pub async fn page_agent_run(
    app: AppHandle,
    state: tauri::State<'_, PageAgentState>,
    request: PageAgentRunRequest,
) -> Result<PageAgentRunResult, String> {
    let url = request.url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("URL 仅支持 http:// 或 https://".to_string());
    }
    if request.task.trim().is_empty() {
        return Err("请输入页面任务".to_string());
    }
    let (base_url, api_key, model) = select_provider(&request)?;
    let bridge = bridge_path().ok_or_else(|| "找不到 page-agent 桥接脚本，请先构建 page-agent-bridge".to_string())?;
    let browser = find_browser().ok_or_else(|| "未找到 Edge/Chrome 浏览器".to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| format!("分配 CDP 端口失败: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut active = state.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.is_some() {
            return Err("已有 Page 任务正在运行".to_string());
        }
        *active = Some(cancel.clone());
    }
    // 清除上一次任务可能留下的回答，避免新任务误接收旧输入。
    *state.answer.lock().unwrap_or_else(|err| err.into_inner()) = None;

    let profile = std::env::temp_dir().join(format!("kira-page-{}-{}", std::process::id(), port));
    let mut child = match hidden_command(&browser)
        .args([
            "--disable-gpu",
            "--window-size=1440,1000",
            "--disable-extensions",
            "--disable-web-security",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-allow-origins=*",
            &format!("--remote-debugging-port={port}"),
            &format!(            "--user-data-dir={}", profile.display()),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            let mut active = state.active.lock().unwrap_or_else(|err| err.into_inner());
            *active = None;
            return Err(format!("启动浏览器失败: {e}"));
        }
    };

    let result = async {
        emit_event(&app, "status", json!({ "status": "starting" }));
        emit_log(&app, format!("浏览器已启动，目标: {url}"));
        let ws_url = wait_debug_endpoint(port, &cancel).await?;
        let (socket, _) = connect_async(&ws_url).await.map_err(|e| format!("连接 CDP 失败: {e}"))?;
        let (mut writer, mut reader) = socket.split();
        let mut next_id = 1;
        cdp_command(&mut writer, &mut reader, &mut next_id, "Runtime.enable", json!({}), &app, &cancel).await?;
        cdp_command(&mut writer, &mut reader, &mut next_id, "Page.enable", json!({}), &app, &cancel).await?;
        cdp_command(&mut writer, &mut reader, &mut next_id, "Runtime.addBinding", json!({ "name": "__kiraPageAgentEmit" }), &app, &cancel).await?;
        let source = tokio::fs::read_to_string(&bridge).await.map_err(|e| format!("读取 page-agent 桥失败: {e}"))?;
        cdp_command(&mut writer, &mut reader, &mut next_id, "Page.addScriptToEvaluateOnNewDocument", json!({ "source": source }), &app, &cancel).await?;
        cdp_command(&mut writer, &mut reader, &mut next_id, "Page.navigate", json!({ "url": url }), &app, &cancel).await?;

        let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if tokio::time::Instant::now() >= ready_deadline { return Err("页面加载超时，桥接脚本未就绪".to_string()); }
            let probe = cdp_command(&mut writer, &mut reader, &mut next_id, "Runtime.evaluate", json!({ "expression": "typeof window.__kiraPageAgentStart === 'function'", "returnByValue": true }), &app, &cancel).await?;
            if probe.pointer("/result/value").and_then(Value::as_bool) == Some(true) { break; }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }

        let cfg = json!({
            "baseURL": base_url,
            "apiKey": api_key,
            "model": model,
            "maxSteps": request.max_steps.clamp(1, 100),
            "stepDelay": 0.4,
            "language": "zh-CN",
            "allowScript": request.allow_script,
            "instructions": "这是 Kira 的受控页面任务。优先使用页面交互工具完成任务；遇到登录墙时停止并等待用户在浏览器窗口中完成登录。不要泄露 API Key。"
        });
        let call = format!("window.__kiraPageAgentStart({}, {})", serde_json::to_string(&cfg).unwrap_or_default(), serde_json::to_string(&request.task).unwrap_or_default());
        cdp_command(&mut writer, &mut reader, &mut next_id, "Runtime.evaluate", json!({ "expression": call, "awaitPromise": false }), &app, &cancel).await?;
        emit_event(&app, "status", json!({ "status": "running" }));

        loop {
            let message = tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)), if cancel.load(Ordering::Relaxed) => {
                    let _ = cdp_command(&mut writer, &mut reader, &mut next_id, "Runtime.evaluate", json!({ "expression": "window.__kiraPageAgentStop?.()" }), &app, &cancel).await;
                    return Err("任务已停止".to_string());
                }
                message = reader.next() => message,
            };
            let Some(Ok(message)) = message else { return Err("CDP 连接已断开".to_string()); };
            let Message::Text(text) = message else { continue; };
            let value: Value = serde_json::from_str(&text).map_err(|e| format!("CDP 事件解析失败: {e}"))?;
            if value.get("method").and_then(Value::as_str) != Some("Runtime.bindingCalled") { continue; }
            let Some((kind, payload)) = forward_binding(&app, value.get("params").unwrap_or(&Value::Null)) else { continue; };
            match kind.as_str() {
                "result" => {
                    let success = payload.get("success").and_then(Value::as_bool).unwrap_or(false);
                    let data = payload.get("data").and_then(Value::as_str).unwrap_or_default().to_string();
                    return Ok(PageAgentRunResult { success, data });
                }
                "error" => return Err(payload.get("message").and_then(Value::as_str).unwrap_or("Page 任务失败").to_string()),
                "ask_user" => {
                    emit_event(&app, "status", json!({ "status": "waiting_user" }));
                    emit_log(&app, format!("等待用户处理：{}", payload.get("question").and_then(Value::as_str).unwrap_or("请在浏览器中完成操作后回答")));
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            return Err("任务已停止".to_string());
                        }
                        let answer = {
                            state
                                .answer
                                .lock()
                                .unwrap_or_else(|err| err.into_inner())
                                .take()
                        };
                        if let Some(answer) = answer {
                            let expression = format!(
                                "window.__kiraPageAgentAnswer({})",
                                serde_json::to_string(&answer).unwrap_or_else(|_| "\"\"".to_string())
                            );
                            let _ = cdp_command(&mut writer, &mut reader, &mut next_id, "Runtime.evaluate", json!({ "expression": expression, "awaitPromise": false }), &app, &cancel).await?;
                            emit_event(&app, "status", json!({ "status": "running" }));
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                }
                _ => {}
            }
        }
    }.await;

    // The CDP socket halves are dropped when the async task above returns.
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = tokio::fs::remove_dir_all(&profile).await;
    let mut active = state.active.lock().unwrap_or_else(|e| e.into_inner());
    *active = None;
    *state.answer.lock().unwrap_or_else(|err| err.into_inner()) = None;
    result
}

#[tauri::command]
pub fn page_agent_answer(state: tauri::State<'_, PageAgentState>, answer: String) -> Result<(), String> {
    let active = state.active.lock().unwrap_or_else(|err| err.into_inner());
    if active.is_none() {
        return Err("当前没有等待回答的 Page 任务".to_string());
    }
    drop(active);
    *state.answer.lock().unwrap_or_else(|err| err.into_inner()) = Some(answer);
    Ok(())
}

#[tauri::command]
pub fn page_agent_stop(state: tauri::State<'_, PageAgentState>) -> Result<(), String> {
    let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cancel) = active.as_ref() {
        cancel.store(true, Ordering::Relaxed);
        Ok(())
    } else {
        Err("当前没有运行中的 Page 任务".to_string())
    }
}
