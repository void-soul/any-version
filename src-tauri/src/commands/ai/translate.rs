//! 划词翻译：复用 AI 配置中的供应商/模型，直接以 OpenAI 兼容协议请求翻译。
//!
//! 与外部 AI 工具（opencode/claude/codex）无关，仅通过 `AiConfig` 中用户已配置的
//! 供应商（含 api_key、openai_url、模型列表）发起一次非流式 chat/completions 请求。
//!
//! 同时提供「划词翻译」链路：模拟 Ctrl+C 读取前台窗口选中文本 → 翻译 →
//! 在独立悬浮窗(translate-popup)中展示结果（带钉住/关闭）。

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
use super::config::load_ai_config;

/// 划词翻译的独立配置（provider + model + 目标语言）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TranslateConfig {
    /// 选中的供应商 id；为空 = 默认取第一个可用供应商
    pub provider_id: Option<String>,
    /// 选中的模型 id（该供应商下的一个模型）
    pub model_id: Option<String>,
    /// 划词翻译默认目标语言（如 "中文"、"English"）；为空 = 中文
    pub target_lang: Option<String>,
}

// 最近一次划词翻译结果（供悬浮窗在窗口复用/聚焦时同步，避免事件丢失导致不更新）
static LAST_TRANSLATE_RESULT: std::sync::Mutex<Option<serde_json::Value>> =
    std::sync::Mutex::new(None);
static POPUP_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TRANSLATE_REQUEST_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 保存最近结果快照，且保证版本号单调递增：
/// trigger_selection_translate 与 translate_text 都会写入快照，后写的
/// requestId 必须不小于已有快照，避免版本号回退导致悬浮窗轮询误判。
fn set_last_translate_result(v: serde_json::Value) {
    let new_seq = v.get("requestId").and_then(|r| r.as_u64());
    if let Ok(mut g) = LAST_TRANSLATE_RESULT.lock() {
        let keep_old = match (&*g, new_seq) {
            (Some(old), Some(new_id)) => old
                .get("requestId")
                .and_then(|r| r.as_u64())
                .is_some_and(|old_id| old_id > new_id),
            _ => false,
        };
        if !keep_old {
            *g = Some(v);
        }
    }
}

fn get_last_translate_result_value() -> Option<serde_json::Value> {
    LAST_TRANSLATE_RESULT.lock().ok().and_then(|g| g.clone())
}

/// 前端悬浮窗打开/聚焦时调用，拉取最近一次划词翻译结果（原文/译文/目标语言），
/// 用于窗口复用或事件时序下兜底同步，确保悬浮窗始终显示最新结果。
#[tauri::command]
pub fn get_last_translate_result() -> Option<serde_json::Value> {
    get_last_translate_result_value()
}

/// 悬浮窗页面完成事件监听注册后调用，后端随后才发送新的翻译事件。
#[tauri::command]
pub fn translate_popup_ready() {
    POPUP_READY.store(true, std::sync::atomic::Ordering::Release);
}

// ─── 翻译历史（跨窗口共享 + 持久化） ───
// 所有翻译（面板手动 / 划词热键 / 悬浮窗手动）都经过 translate_text，
// 因此在这里统一记录历史，翻译模块面板与悬浮窗共享同一份历史。

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TranslateHistoryEntry {
    #[serde(default)]
    pub id: String,
    pub source: String,
    pub result: String,
    pub target: String,
    pub provider: String,
    pub model: String,
    pub ts: u64,
    #[serde(default)]
    pub pinned: bool,
}

static HISTORY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static HISTORY_ID_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn history_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("translate_history.json")
}

fn new_entry_id(ts: u64) -> String {
    let n = HISTORY_ID_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{}", ts, n)
}

fn persist_history(list: &[TranslateHistoryEntry]) {
    if let Ok(data) = serde_json::to_string_pretty(list) {
        let _ = crate::commands::config::atomic_write_file(&history_path(), data.as_bytes());
    }
}

fn load_history() -> Vec<TranslateHistoryEntry> {
    let path = history_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(mut list) = serde_json::from_str::<Vec<TranslateHistoryEntry>>(&data) {
                // 兼容旧数据（无 id 字段）：补齐唯一 id 并回写
                let mut changed = false;
                for e in list.iter_mut() {
                    if e.id.is_empty() {
                        e.id = new_entry_id(e.ts);
                        changed = true;
                    }
                }
                if changed {
                    persist_history(&list);
                }
                return list;
            }
        }
    }
    Vec::new()
}

fn sort_history(list: &mut [TranslateHistoryEntry]) {
    // 置顶优先，其次按时间倒序
    list.sort_by(|a, b| b.pinned.cmp(&a.pinned).then_with(|| b.ts.cmp(&a.ts)));
}

const HISTORY_CAP: usize = 100;

fn add_history_entry(entry: TranslateHistoryEntry) {
    let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_history();
    list.insert(0, entry);
    sort_history(&mut list);
    list.truncate(HISTORY_CAP);
    persist_history(&list);
}

/// 读取翻译历史（面板与悬浮窗共用），置顶优先、按时间倒序。
#[tauri::command]
pub fn translate_history_list() -> Vec<TranslateHistoryEntry> {
    let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_history();
    sort_history(&mut list);
    list
}

/// 删除一条翻译历史。
#[tauri::command]
pub fn translate_history_delete(id: String) -> Result<(), String> {
    let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_history();
    let before = list.len();
    list.retain(|e| e.id != id);
    if list.len() == before {
        return Err(format!("未找到历史记录: {}", id));
    }
    persist_history(&list);
    Ok(())
}

/// 置顶/取消置顶一条翻译历史（置顶条目始终排在最前）。
#[tauri::command]
pub fn translate_history_pin(id: String) -> Result<(), String> {
    let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_history();
    let mut found = false;
    for e in list.iter_mut() {
        if e.id == id {
            e.pinned = !e.pinned;
            found = true;
            break;
        }
    }
    if !found {
        return Err(format!("未找到历史记录: {}", id));
    }
    sort_history(&mut list);
    persist_history(&list);
    Ok(())
}

/// 清空翻译历史（置顶条目保留，与剪贴板模块行为一致）。
#[tauri::command]
pub fn translate_history_clear() {
    let _guard = HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_history();
    list.retain(|e| e.pinned);
    if list.is_empty() {
        let _ = std::fs::remove_file(history_path());
    } else {
        persist_history(&list);
    }
}

// ─── 读写划词翻译配置 ───

fn translate_config_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("translate_config.json")
}

pub(crate) fn load_translate_config() -> TranslateConfig {
    let path = translate_config_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<TranslateConfig>(&data) {
                return cfg;
            }
        }
    }
    TranslateConfig::default()
}

#[tauri::command]
pub fn get_translate_config() -> TranslateConfig {
    load_translate_config()
}

#[tauri::command]
pub fn save_translate_config(config: TranslateConfig) -> Result<(), String> {
    let data = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&translate_config_path(), data.as_bytes())
}

/// 从 AiConfig 解析出一个可用的 (provider, model_id)，按优先级：
/// 1. 显式传入的 provider_id + model_id
/// 2. 保存的划词翻译配置
/// 3. 任意第一个有 api_key 且配置了 OpenAI 端点的供应商 + 其 active_model_id
fn resolve_translation_target(
    cfg: &crate::commands::ai::models::AiConfig,
    provider_id: &Option<String>,
    model_id: &Option<String>,
) -> Result<(crate::commands::ai::models::AiProvider, String), String> {
    let saved = load_translate_config();
    let pid = provider_id.clone().or(saved.provider_id);
    let mid = model_id.clone().or(saved.model_id);

    let provider = if let Some(pid) = &pid {
        cfg.providers
            .iter()
            .find(|p| &p.id == pid)
            .cloned()
            .ok_or_else(|| format!("未找到供应商: {}", pid))?
    } else {
        cfg.providers
            .iter()
            .find(|p| !p.api_key.is_empty() && !p.openai_url.is_empty())
            .cloned()
            .ok_or_else(|| "没有配置了 OpenAI 端点和 API Key 的供应商".to_string())?
    };

    if provider.openai_url.is_empty() {
        return Err(format!("供应商「{}」未配置 OpenAI 兼容端点", provider.name));
    }
    if provider.api_key.is_empty() {
        return Err(format!("供应商「{}」未配置 API Key", provider.name));
    }

    // 模型：显式 model_id > 该供应商 active_model_id > 该供应商第一个模型 > 报错
    let model = mid
        .clone()
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| format!("供应商「{}」未配置任何模型", provider.name))?;

    Ok((provider, model))
}

/// 翻译文本。`target_lang` 为目标语言描述（如 "中文"、"English"、"日文"）；
/// 为空则让模型自动判断源语言并翻译成中文。
/// 成功后自动写入翻译历史（面板 / 划词悬浮窗统一入口）。
#[tauri::command]
pub async fn translate_text(
    app: tauri::AppHandle,
    text: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    target_lang: Option<String>,
) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("待翻译文本为空".to_string());
    }

    // 版本标记：前端点击翻译也是一次“新的翻译请求”，必须递增版本号并保存快照。
    // 悬浮窗通过轮询 get_last_translate_result + requestId 比较版本，
    // 只有后端版本号领先时才会覆盖页面上的原文/译文（用户手动翻译同样推进版本）。
    let request_id = TRANSLATE_REQUEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

    let cfg = load_ai_config();
    let (provider, model) = resolve_translation_target(&cfg, &provider_id, &model_id)?;

    let target = target_lang.unwrap_or_else(|| "中文".to_string());
    let system_prompt = format!(
        "你是一个专业的翻译助手。请将用户提供的文本翻译成{}。\
         只输出翻译结果，不要添加任何解释、引号、前后缀或多余内容。\
         若源语言与目标语言相同，原样返回输入文本。",
        target
    );

    let base_url = provider.openai_url.trim_end_matches('/').to_string();
    let url = if base_url.ends_with("/v1") {
        format!("{}/chat/completions", base_url)
    } else {
        format!("{}/v1/chat/completions", base_url)
    };

    let body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": text }
        ],
        "stream": false,
        "temperature": 0.3,
    });

    // 诊断：记录实际使用的供应商、模型、请求地址与参数
    crate::exit_log!(
        "[翻译] provider_id={} provider_name={} model={}",
        provider.id,
        provider.name,
        model
    );
    crate::exit_log!("[翻译] 请求地址: {}", url);
    crate::exit_log!(
        "[翻译] 请求体(截断): {}",
        serde_json::to_string(&body).unwrap_or_default().chars().take(300).collect::<String>()
    );

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("翻译请求失败: {}", e))?;

    let status = resp.status();
    let value: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;

    if !status.is_success() {
        let msg = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        crate::exit_log!(
            "[翻译] 接口错误 ({}): {} 完整响应: {}",
            status.as_u16(),
            msg,
            serde_json::to_string(&value).unwrap_or_default()
        );
        return Err(format!(
            "翻译接口返回错误 ({}): {}（供应商: {}，模型: {}，地址: {}）",
            status.as_u16(),
            msg,
            provider.name,
            model,
            url
        ));
    }

    let content = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if content.is_empty() {
        return Err("翻译结果为空".to_string());
    }

    // 记录翻译历史（面板 / 悬浮窗共用），并通知前端刷新
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    add_history_entry(TranslateHistoryEntry {
        id: new_entry_id(ts),
        source: text.clone(),
        result: content.clone(),
        target: target.clone(),
        provider: provider.name,
        model: model.clone(),
        ts,
        pinned: false,
    });
    let _ = app.emit("translate-history-changed", ());

    // 保存最近结果快照（含版本号），供悬浮窗轮询同步；
    // 与 trigger_selection_translate 的 ok_payload 结构保持一致。
    let ok_payload = serde_json::json!({
        "loading": false, "source": text, "result": content, "target": target, "requestId": request_id
    });
    set_last_translate_result(ok_payload);

    Ok(content)
}

// ─── 划词翻译：读取前台窗口选中文本 ───

/// CF_UNICODETEXT 标准值（Windows 常量，未在 windows-sys 直接暴露为常量）
const CF_UNICODETEXT: u32 = 13;

/// 模拟 Ctrl+C，把当前前台窗口的选中文本复制到剪贴板。
/// 若前台窗口无选中文本，剪贴板可能保持不变；由调用方读取后判断是否为空。
fn simulate_copy_selection() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    unsafe {
        let mut inputs: Vec<INPUT> = Vec::with_capacity(4);
        // Ctrl down
        let mut ctrl: INPUT = std::mem::zeroed();
        ctrl.r#type = INPUT_KEYBOARD;
        ctrl.Anonymous.ki.wVk = VK_CONTROL as u16;
        inputs.push(ctrl);
        // C down
        let mut c_down: INPUT = std::mem::zeroed();
        c_down.r#type = INPUT_KEYBOARD;
        c_down.Anonymous.ki.wVk = b'C' as u16;
        inputs.push(c_down);
        // C up
        let mut c_up: INPUT = std::mem::zeroed();
        c_up.r#type = INPUT_KEYBOARD;
        c_up.Anonymous.ki.wVk = b'C' as u16;
        c_up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
        inputs.push(c_up);
        // Ctrl up
        let mut ctrl_up: INPUT = std::mem::zeroed();
        ctrl_up.r#type = INPUT_KEYBOARD;
        ctrl_up.Anonymous.ki.wVk = VK_CONTROL as u16;
        ctrl_up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
        inputs.push(ctrl_up);

        SendInput(inputs.len() as u32, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

/// 读取剪贴板纯文本（需在 OpenClipboard 之后调用）。此处自带重试，避免被占用。
fn read_clipboard_text() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    unsafe {
        let mut opened = false;
        for _ in 0..20 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            let _ = CloseClipboard();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !opened {
            return None;
        }
        let h = GetClipboardData(CF_UNICODETEXT);
        if h.is_null() {
            let _ = CloseClipboard();
            return None;
        }
        let ptr = GlobalLock(h);
        if ptr.is_null() {
            let _ = CloseClipboard();
            return None;
        }
        let s = read_wide_string(ptr as *const u16);
        let _ = GlobalUnlock(h);
        let _ = CloseClipboard();
        s
    }
}

fn read_wide_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
    }
    if len == 0 {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    let s = String::from_utf16_lossy(slice).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// ─── 划词翻译悬浮窗 ───

const POPUP_LABEL: &str = "translate-popup";

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
    ))
}

fn emit_translate_payload(app: &tauri::AppHandle, payload: &serde_json::Value) {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        if let Err(e) = win.emit("translate-result", payload.clone()) {
            crate::exit_log!("[划词翻译] 悬浮窗事件发送失败: {}", e);
        }
    }
    if let Err(e) = app.emit("translate-result", payload.clone()) {
        crate::exit_log!("[划词翻译] 应用级事件发送失败: {}", e);
    }
}

/// 确保悬浮窗存在并返回其句柄（存在则复用）。
fn ensure_translate_popup(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        return Ok(win);
    }
    POPUP_READY.store(false, std::sync::atomic::Ordering::Release);
    let win = tauri::WebviewWindowBuilder::new(
        app,
        POPUP_LABEL,
        tauri::WebviewUrl::App("index.html?popup=translate".into()),
    )
    .title("翻译")
    .inner_size(400.0, 480.0)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .shadow(false)
    .build()
    .map_err(|e| format!("创建悬浮窗失败: {}", e))?;
    Ok(win)
}

/// 划词翻译主流程：读取前台窗口选中文本 → 翻译 → 显示到悬浮窗。
/// 返回 (是否成功, 提示信息)。由前端触发（模块热键或面板按钮）。
#[tauri::command]
pub async fn trigger_selection_translate(
    app: tauri::AppHandle,
    target_lang: Option<String>,
) -> Result<(), String> {
    // 1. 先记住/模拟复制前台窗口选中文本
    crate::exit_log!("[划词翻译] 开始，模拟 Ctrl+C");
    simulate_copy_selection();
    // 稍等剪贴板更新
    std::thread::sleep(std::time::Duration::from_millis(120));
    let text = match read_clipboard_text() {
        Some(t) => {
            crate::exit_log!("[划词翻译] 读取到选中文本 {} 字符: {:?}", t.chars().count(), t.chars().take(30).collect::<String>());
            t
        }
        None => {
            crate::exit_log!("[划词翻译] 读取剪贴板为空（模拟 Ctrl+C 可能未复制到文本）");
            return Err("未能读取到选中的文本（剪贴板为空）。请先在任意程序中选中文字，再触发划词翻译。".to_string());
        }
    };

    // 2. 立即创建/显示悬浮窗（先显示，翻译期间前端展示 loading）
    crate::exit_log!("[划词翻译] 创建/复用悬浮窗");
    if let Err(e) = ensure_translate_popup(&app) {
        crate::exit_log!("[划词翻译] 创建悬浮窗失败: {}", e);
        return Err(e);
    }
    // 首次创建窗口时等待前端完成事件注册，避免 loading/结果事件在页面初始化前丢失。
    for _ in 0..200 {
        if POPUP_READY.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // 3. 目标语言：显式传入 > 文本语言自动判断 > 划词翻译配置 > 默认中文。
    // 快捷键通常用于把选中的中文翻译成英文，因此中文文本默认目标设为 English。
    let target = target_lang
        .filter(|s| !s.trim().is_empty())
        .or_else(|| contains_cjk(&text).then(|| "English".to_string()))
        .or_else(|| load_translate_config().target_lang.filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "中文".to_string());
    let request_id = TRANSLATE_REQUEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

    // 窗口操作（show/set_focus）与事件推送必须在 Tauri 主线程执行，
    // 否则在 async spawn 线程里 set_focus 可能失效，前端 onFocusChanged 不触发，
    // 导致"聚焦时同步最新结果"失效 → 悬浮窗原文不更新。
    let show_payload = serde_json::json!({ "loading": true, "source": text, "target": target, "requestId": request_id });
    set_last_translate_result(show_payload.clone());
    let app_for_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        let win = app_for_main.get_webview_window(POPUP_LABEL);
        if let Some(w) = win.as_ref() {
            let _ = w.show();
            let _ = w.set_focus();
            // 仅 window.set_focus() 只激活顶层窗口 HWND，WebView2 内容不会获得焦点，
            // 前端就收不到 tauri://focus / tauri://blur，导致「失焦自动关闭（钉住判断）」失效。
            // 需额外聚焦 WebView2（与主窗口 show_and_open_module 一致）。
            let webview: &tauri::Webview<tauri::Wry> = w.as_ref();
            let _ = webview.set_focus();
        }
    });
    emit_translate_payload(&app, &show_payload);
    crate::exit_log!("[划词翻译] 悬浮窗已显示（loading），开始翻译，目标语言: {}", target);

    // 4. 翻译；无论成败都回推结果到悬浮窗（失败也展示错误信息）
    let translated = match translate_text(app.clone(), text.clone(), None, None, Some(target.clone())).await {
        Ok(t) => {
            crate::exit_log!("[划词翻译] 翻译成功: {}", t.chars().take(30).collect::<String>());
            t
        }
        Err(e) => {
            crate::exit_log!("[划词翻译] 翻译失败: {}", e);
            let err_payload = serde_json::json!({
                "loading": false, "source": text, "result": format!("翻译失败: {}", e), "target": target, "requestId": request_id, "error": true
            });
            set_last_translate_result(err_payload.clone());
            emit_translate_payload(&app, &err_payload);
            return Err(e);
        }
    };
    let ok_payload = serde_json::json!({
        "loading": false, "source": text, "result": translated, "target": target, "requestId": request_id
    });
    set_last_translate_result(ok_payload.clone());
    // 最终结果直接发送，不依赖主线程队列；主线程只负责窗口显示/聚焦。
    emit_translate_payload(&app, &ok_payload);
    crate::exit_log!("[划词翻译] 结果已推送");
    Ok(())
}

/// 关闭/隐藏悬浮窗（前端钉住关闭时调用）。
#[tauri::command]
pub fn hide_translate_popup(app: tauri::AppHandle) {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        let _ = win.hide();
    }
}

/// 直接通过悬浮窗展示指定翻译（供前端 TranslatePanel 复用，避免重复建窗）。
#[tauri::command]
pub async fn show_translate_result(app: tauri::AppHandle, source: String, result: String, target: String) -> Result<(), String> {
    ensure_translate_popup(&app)?;
    let payload = serde_json::json!({ "source": source, "result": result, "target": target });
    // 与 trigger_selection_translate 保持一致：保存最近结果供前端拉取，
    // 窗口操作放主线程执行，并额外聚焦 WebView2（否则收不到 focus/blur 事件）。
    set_last_translate_result(payload.clone());
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(POPUP_LABEL) {
            let _ = w.show();
            let _ = w.set_focus();
            let webview: &tauri::Webview<tauri::Wry> = w.as_ref();
            let _ = webview.set_focus();
        }
    });
    emit_translate_payload(&app, &payload);
    Ok(())
}
