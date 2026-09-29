//! 音乐插件相关的 Tauri 命令（管理 + 在线搜索）。
//!
//! ⚠️ 与 `music/commands.rs` 同规矩：**全部是 `async fn`**。同步命令在 Tauri v2 里
//! 跑在**主线程**上，而这些命令要起 Node 子进程、等网络（搜索几秒起步），
//! 同步执行会把整个窗口冻住。真正阻塞的部分一律丢进
//! [`tauri::async_runtime::spawn_blocking`]，异步体里只做网络与编排。

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use super::plugin_host::{self, DepsReport};
use super::plugin_registry::{self, PluginEntry, PluginMeta};

/// 探测插件自述（一次 `require`，很快；给足余量防首启 Node 慢）
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// 单次搜索超时。插件慢是常态，但无限等会让界面一直转圈。
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);

/// 依赖安装进度事件名（逐行上报 npm 输出）。
pub const DEPS_LOG_EVENT: &str = "music-plugin-deps-log";

// ─── 数据类型 ───

#[derive(Debug, Serialize)]
pub struct PluginListResult {
    /// 依赖现状（缺 node / 缺包 / 就绪）
    pub deps: DepsReport,
    pub plugins: Vec<PluginEntry>,
    /// 插件根目录（界面「打开目录」用）
    pub dir: String,
}

/// 导入结果。
///
/// 导入前**按内容自动识别**是「单个插件」还是「订阅清单」—— 因为两者的来源形态完全一样
/// （都可能是 URL，也都可以是本地文件），只靠扩展名/按钮分派一定会漏：
/// 例如官方订阅列表本身就是个 `.json` URL。
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginImportResult {
    /// 单个插件
    Single {
        file: String,
        source: String,
        /// 界面显示名（订阅清单里给了名字时用订阅的名字）
        name: String,
        meta: PluginMeta,
        /// 落盘成功但自述探测失败的原因；插件仍可用，可稍后「刷新信息」重试
        probe_error: Option<String>,
    },
    /// 订阅清单（批量导入）
    Subscription {
        plugins: Vec<PluginEntry>,
        imported: usize,
        failures: Vec<PluginFailure>,
    },
}

/// 某个插件出错的统一描述（导入失败 / 搜索失败共用）。
#[derive(Debug, Clone, Serialize)]
pub struct PluginFailure {
    pub file: String,
    pub name: String,
    pub error: String,
}



/// 一条搜索结果。
///
/// `item` 是插件返回的**原始对象**（`id/title/artist/album/duration/artwork…`）原样透传：
/// 它会原封不动地回传给 `getMediaSource`，任何裁剪都可能让插件认不出来。
#[derive(Debug, Serialize)]
pub struct SearchHit {
    /// 来源插件文件名
    pub file: String,
    /// 来源插件显示名
    pub platform: String,
    pub item: Value,
}

#[derive(Debug, Default, Serialize)]
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    /// 已到末页的插件（前端据此停止「加载更多」）
    pub exhausted: Vec<String>,
    /// 失败的插件：**一个插件挂掉不该让整次搜索失败**
    pub failures: Vec<PluginFailure>,
}

// ─── 列表 / 依赖 ───

fn list_result() -> Result<PluginListResult, String> {
    Ok(PluginListResult {
        deps: plugin_host::deps_report(),
        plugins: plugin_registry::list_reconciled()?,
        dir: plugin_host::plugins_root().to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub async fn music_plugin_list() -> Result<PluginListResult, String> {
    list_result()
}

/// 安装 / 补齐插件依赖，逐行把 npm 输出推给前端。
#[tauri::command]
pub async fn music_plugin_install_deps(app: AppHandle) -> Result<PluginListResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut log = |line: &str| {
            eprintln!("[plugin-deps] {line}");
            let _ = app.emit(DEPS_LOG_EVENT, line.to_string());
        };
        plugin_host::install_deps(&mut log)?;
        list_result()
    })
    .await
    .map_err(|e| format!("安装依赖任务失败: {e}"))?
}

// ─── 导入 ───

async fn fetch_text(url: &str) -> Result<String, String> {
    let response = crate::commands::utils::get_http_client()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("服务端返回 {status}"));
    }
    response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))
}

/// 落盘 + 探测自述 + 登记。**已存在同来源的插件时视为更新**（覆盖原文件），
/// 否则同一个插件每导一次就多出一份。
///
/// `name_hint` 来自订阅清单（如「网易」），只对新条目生效。
fn install_plugin(
    source: &str,
    suggested: &str,
    content: &str,
    name_hint: Option<&str>,
) -> Result<PluginImportResult, String> {
    plugin_host::ensure_layout()?;
    let existing = plugin_registry::list_reconciled()?;
    let file = match existing
        .iter()
        .find(|entry| !entry.source.is_empty() && entry.source == source)
    {
        Some(entry) => entry.file.clone(),
        None => {
            let taken: Vec<String> = existing.iter().map(|entry| entry.file.clone()).collect();
            plugin_registry::unique_file_name(suggested, &taken)
        }
    };

    let path = plugin_registry::resolve_script_path(&file)?;
    std::fs::write(&path, content.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;

    // 探测失败**不算导入失败**：文件已经落盘，用户还能在管理页看到它并重试。
    // 用 `load_plugin` 而不是 `call_plugin(path, "load", …)` —— 后者会先载入再
    // 调用一个没有 `path` 的 load，反而把宿主搞坏（见 load_plugin 的注释）。
    let (meta, probe_error) = match plugin_host::load_plugin(&path, PROBE_TIMEOUT) {
        Ok(value) => (PluginMeta::from_bridge(&value), None),
        Err(error) => (PluginMeta::default(), Some(error)),
    };
    let entry = plugin_registry::register_import(&file, source, meta, name_hint)?;
    Ok(PluginImportResult::Single {
        file,
        source: source.to_string(),
        name: entry.display_name(),
        meta: entry.meta,
        probe_error,
    })
}

/// 读取导入来源：http(s) 走网络，其余当本地文件路径。
///
/// 插件与订阅清单都可能是本地文件，所以这一步**不区分**类型 —— 类型由内容判断。
async fn read_source(source: &str) -> Result<String, String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        return fetch_text(source).await;
    }
    let path = std::path::Path::new(source);
    if !path.is_file() {
        return Err(format!("找不到文件：{source}"));
    }
    // 插件/订阅文件都很小（几十 KB），直接读不会造成可感知的阻塞
    std::fs::read_to_string(path).map_err(|e| format!("读取文件失败: {e}"))
}

/// 按内容判断是不是订阅清单；不是就返回 `None`。
///
/// 只认「合法 JSON 且形如订阅」的内容：插件本体是 JS，JSON 解析必然失败，
/// 因此不会误判成订阅。也接受 BOM 与裸数组（社区里两种都有）。
fn as_subscription(content: &str) -> Option<Vec<(String, String)>> {
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }
    let body: Value = serde_json::from_str(trimmed).ok()?;
    parse_subscription(&body).ok()
}

/// 导入插件 / 订阅清单：本地文件或远程地址都行，**按内容自动识别**。
#[tauri::command]
pub async fn music_plugin_import(source: String) -> Result<PluginImportResult, String> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err("请输入插件地址 / 订阅地址，或选择一个插件或订阅文件".to_string());
    }
    let content = read_source(&source).await?;

    // 1) 订阅清单：逐个下载插件本体后再统一落盘。
    //    下载**串行而不是并发**：订阅里动辄几十个插件，并发打过去既容易被限流，
    //    也会让「失败原因」这类提示失去可读性。
    if let Some(entries) = as_subscription(&content) {
        let mut downloaded: Vec<(String, String, String)> = Vec::new();
        let mut failures: Vec<PluginFailure> = Vec::new();
        for (name, plugin_url) in entries {
            match fetch_text(&plugin_url).await {
                Ok(text) => downloaded.push((name, plugin_url, text)),
                Err(error) => failures.push(PluginFailure {
                    file: String::new(),
                    name: if name.trim().is_empty() {
                        plugin_url.clone()
                    } else {
                        name
                    },
                    error,
                }),
            }
        }

        let (imported, failures) = tauri::async_runtime::spawn_blocking(move || {
            let mut imported = 0usize;
            let mut failures = failures;
            for (name, plugin_url, text) in downloaded {
                let suggested = plugin_registry::suggest_script_file_name(&plugin_url);
                let hint = name.trim().to_string();
                let hint_ref = if hint.is_empty() { None } else { Some(hint.as_str()) };
                match install_plugin(&plugin_url, &suggested, &text, hint_ref) {
                    Ok(_) => imported += 1,
                    Err(error) => failures.push(PluginFailure {
                        file: String::new(),
                        name: if hint.is_empty() { plugin_url } else { hint },
                        error,
                    }),
                }
            }
            (imported, failures)
        })
        .await
        .map_err(|e| format!("订阅导入任务失败: {e}"))?;

        return Ok(PluginImportResult::Subscription {
            plugins: plugin_registry::list_reconciled()?,
            imported,
            failures,
        });
    }

    // 2) 单个插件
    let suggested = plugin_registry::suggest_script_file_name(&source);
    let owned_source = source.clone();
    tauri::async_runtime::spawn_blocking(move || {
        install_plugin(&owned_source, &suggested, &content, None)
    })
    .await
    .map_err(|e| format!("导入任务失败: {e}"))?
}

/// 解析订阅清单。MusicFree 官方形态是 `{"plugins":[{"name","url","version"}]}`，
/// 社区里也有裸数组、以及纯 URL 字符串数组，都认。
fn parse_subscription(body: &Value) -> Result<Vec<(String, String)>, String> {
    let list = body
        .get("plugins")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .ok_or_else(|| "订阅内容无法识别（既不是 {plugins:[…]}，也不是数组）".to_string())?;

    let mut out = Vec::new();
    for item in list {
        if let Some(url) = item.as_str() {
            if url.starts_with("http") {
                out.push((String::new(), url.to_string()));
            }
            continue;
        }
        let Some(url) = item.get("url").and_then(Value::as_str) else {
            continue;
        };
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        out.push((name, url.to_string()));
    }
    if out.is_empty() {
        return Err("订阅里没有可用的插件地址".to_string());
    }
    Ok(out)
}

// ─── 管理 ───

#[tauri::command]
pub async fn music_plugin_remove(file: String) -> Result<PluginListResult, String> {
    plugin_registry::remove(&file)?;
    list_result()
}

#[tauri::command]
pub async fn music_plugin_set_enabled(
    file: String,
    enabled: bool,
) -> Result<PluginListResult, String> {
    plugin_registry::set_enabled(&file, enabled)?;
    list_result()
}

#[tauri::command]
pub async fn music_plugin_set_name(file: String, name: String) -> Result<PluginListResult, String> {
    plugin_registry::set_name(&file, &name)?;
    list_result()
}

#[tauri::command]
pub async fn music_plugin_reorder(files: Vec<String>) -> Result<PluginListResult, String> {
    plugin_registry::reorder(&files)?;
    list_result()
}

/// 重新探测自述。`file` 为空时刷新全部。
///
/// 用途：用户自己把 `.js` 丢进插件目录（此时只登记了文件、没有自述），
/// 或插件升级后能力位变了。
#[tauri::command]
pub async fn music_plugin_refresh_meta(file: Option<String>) -> Result<PluginListResult, String> {
    let targets: Vec<String> = match file.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(name) => vec![name.to_string()],
        None => plugin_registry::list_reconciled()?
            .into_iter()
            .map(|entry| entry.file)
            .collect(),
    };

    tauri::async_runtime::spawn_blocking(move || {
        for name in targets {
            let path = plugin_registry::resolve_script_path(&name)?;
            if !path.is_file() {
                continue;
            }
            match plugin_host::load_plugin(&path, PROBE_TIMEOUT) {
                Ok(value) => {
                    plugin_registry::update_meta(&name, PluginMeta::from_bridge(&value))?;
                }
                Err(error) => eprintln!("[plugin] 刷新 {name} 信息失败: {error}"),
            }
        }
        list_result()
    })
    .await
    .map_err(|e| format!("刷新任务失败: {e}"))?
}

/// 在资源管理器里打开插件目录（给了 `file` 就定位到该文件）。
#[tauri::command]
pub async fn music_plugin_open_dir(file: Option<String>) -> Result<String, String> {
    plugin_host::ensure_layout()?;
    let dir = plugin_host::plugins_root();
    let target = match file.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(name) => plugin_registry::resolve_script_path(name)?,
        None => dir.clone(),
    };
    let argument = if target.is_dir() {
        target.to_string_lossy().to_string()
    } else {
        format!("/select,{}", target.to_string_lossy())
    };
    std::process::Command::new("explorer")
        .arg(argument)
        .spawn()
        .map_err(|e| format!("打开目录失败: {e}"))?;
    Ok(dir.to_string_lossy().to_string())
}

// ─── 在线搜索 ───

fn search_blocking(
    keyword: &str,
    files: Option<Vec<String>>,
    page: u32,
) -> Result<SearchOutcome, String> {
    plugin_host::ensure_layout()?;
    let all = plugin_registry::searchable_plugins()?;
    let selected: Vec<PluginEntry> = match files.as_ref() {
        Some(list) if !list.is_empty() => all
            .into_iter()
            .filter(|entry| list.iter().any(|file| file == &entry.file))
            .collect(),
        _ => all,
    };
    if selected.is_empty() {
        return Err("没有可用的插件：请先在插件管理里导入并启用".to_string());
    }

    let mut outcome = SearchOutcome::default();
    for entry in selected {
        let path = plugin_registry::resolve_script_path(&entry.file)?;
        let platform = entry.display_name();
        let params = json!({ "keyword": keyword, "page": page });
        match plugin_host::call_plugin(&path, "search", params, SEARCH_TIMEOUT) {
            Ok(value) => {
                if value.get("isEnd").and_then(Value::as_bool) == Some(true) {
                    outcome.exhausted.push(entry.file.clone());
                }
                for item in value
                    .get("data")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                {
                    outcome.hits.push(SearchHit {
                        file: entry.file.clone(),
                        platform: platform.clone(),
                        item: item.clone(),
                    });
                }
            }
            // 逐个插件记录失败：一个音源挂了，其它音源的结果照样该出来
            Err(error) => outcome.failures.push(PluginFailure {
                file: entry.file.clone(),
                name: platform,
                error,
            }),
        }
    }
    Ok(outcome)
}

/// 在线搜索。`files` 为空表示用全部已启用的插件。
#[tauri::command]
pub async fn music_plugin_search(
    keyword: String,
    files: Option<Vec<String>>,
    page: Option<u32>,
) -> Result<SearchOutcome, String> {
    let keyword = keyword.trim().to_string();
    if keyword.is_empty() {
        return Err("请输入搜索关键词".to_string());
    }
    let page = page.unwrap_or(1).max(1);
    tauri::async_runtime::spawn_blocking(move || search_blocking(&keyword, files, page))
        .await
        .map_err(|e| format!("搜索任务失败: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_accepts_object_array_and_string_array() {
        // MusicFree 官方形态
        let official = json!({
            "desc": "插件列表",
            "plugins": [
                { "name": "Audiomack", "url": "https://x/audiomack/index.js", "version": "0.0.2" }
            ]
        });
        assert_eq!(
            parse_subscription(&official).unwrap(),
            vec![(
                "Audiomack".to_string(),
                "https://x/audiomack/index.js".to_string()
            )]
        );

        // 裸数组（只有 url）
        let bare = json!([{ "url": "https://x/a/index.js" }]);
        assert_eq!(
            parse_subscription(&bare).unwrap(),
            vec![(String::new(), "https://x/a/index.js".to_string())]
        );

        // 纯字符串数组
        let strings = json!(["https://x/a/index.js", "https://x/b/index.js"]);
        assert_eq!(parse_subscription(&strings).unwrap().len(), 2);
    }

    /// 订阅里混进垃圾条目时：能用的照常导入，不能用的安静跳过，
    /// 而不是整个订阅失败。
    #[test]
    fn subscription_skips_unusable_entries() {
        let body = json!({
            "plugins": [
                { "name": "好插件", "url": "https://x/ok/index.js" },
                { "name": "没有地址" },
                { "url": 123 },
                "不是地址"
            ]
        });
        let parsed = parse_subscription(&body).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1, "https://x/ok/index.js");
    }

    #[test]
    fn subscription_without_any_plugin_url_is_an_error() {
        assert!(parse_subscription(&json!({ "plugins": [] })).is_err());
        assert!(parse_subscription(&json!({ "hello": 1 })).is_err());
        assert!(parse_subscription(&json!([])).is_err());
    }

    /// 插件本体与订阅清单的**来源形态完全一样**（都可能是 URL 或本地文件），
    /// 所以只能按内容区分。这条锁住用户给的那种订阅文件能被认出来。
    #[test]
    fn subscription_detection_is_driven_by_content_not_by_extension() {
        let file = r#"{
          "plugins": [
            { "name": "网易", "url": "https://13413.kstore.vip/yuanli/wy.js", "version": "1.2.1" },
            { "name": "qq", "url": "https://13413.kstore.vip/yuanli/qq.js", "version": "1.2.1" }
          ]
        }"#;
        let entries = as_subscription(file).expect("应识别为订阅清单");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "网易");
        assert_eq!(entries[1].1, "https://13413.kstore.vip/yuanli/qq.js");

        // 带 BOM 也要认（用记事本存过的文件就带）
        assert!(as_subscription(&format!("\u{feff}{file}")).is_some());
        // 裸数组也算
        assert!(as_subscription(r#"[{"url":"https://x/a.js"}]"#).is_some());
    }

    /// 插件本体是 JS —— 绝不能因为「看起来像 JSON」就被当成订阅清单。
    #[test]
    fn plugin_source_is_never_mistaken_for_a_subscription() {
        let plugin = r#""use strict";
Object.defineProperty(exports, "__esModule", { value: true });
const axios_1 = require("axios");
exports.search = async () => ({ data: [] });
"#;
        assert!(as_subscription(plugin).is_none());
        // 以 `{` 开头的 JS（块语句）也不能被误判
        assert!(as_subscription("{ const a = 1; }\nmodule.exports = {};").is_none());
        // 合法 JSON 但不是订阅形状 —— 不当作订阅，交给单插件路径报清楚的错
        assert!(as_subscription(r#"{"plugins":[]}"#).is_none());
        assert!(as_subscription(r#"{"hello":1}"#).is_none());
        assert!(as_subscription("").is_none());
        assert!(as_subscription("{ 坏掉的 json").is_none());
    }
}
