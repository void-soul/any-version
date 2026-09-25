//! 自定义「写模型」：schema 复杂到没法用「路径 → 值」声明的工具。
//!
//! 复刻自 EchoBird（`E:\pro\other-sdk\ai-tools\EchoBird`）：
//! `tools/*/config.json` 用 `"custom": true` 标记，真正的写入在
//! `src-tauri/src/services/tool_config_manager/<tool>.rs` 里整份生成配置。
//! 我们这里放同一套东西（目前只有 WorkBuddy 一个，后续按需加）。
//!
//! 为什么需要它：WorkBuddy 的模型配置是
//! `{ models: [{ id, name, vendor, url, apiKey, maxInputTokens, maxOutputTokens,
//!   supportsToolCall, supportsImages }], availableModels: [id] }` ——
//! 数组套对象、还要求 URL 是**完整的 /chat/completions**，通用的 `set_json_path` 表达不了。

use std::path::Path;

/// 一次「把模型写进工具配置」的输入（两条调用链共用：启动时写 / 只保存模型）。
pub struct ModelWrite<'a> {
    /// 实际要发给上游的模型 id
    pub model: &'a str,
    /// 声明给工具的模型名（配了伪装就是伪装名，否则等于 model）
    pub claimed: &'a str,
    /// 工具实际请求的 base URL（启动时代理会接管 → 指向 127.0.0.1）
    pub base_url: &'a str,
    pub api_key: &'a str,
    /// 真实上游端点（仅用于展示「厂商」这类元信息；代理模式下 base_url 是本机回环，
    /// 拿它当厂商名会显示成 127.0.0.1）
    pub upstream_url: &'a str,
}

/// WorkBuddy 默认的上下文/输出上限（EchoBird workbuddy.rs 同值）。
/// 它没有「留空让应用自己判断」的写法，字段缺了会被前端当 0 处理。
const WORKBUDDY_MAX_INPUT_TOKENS: u64 = 200_000;
const WORKBUDDY_MAX_OUTPUT_TOKENS: u64 = 8_192;

/// 按写入器名分派。`None` 之外的未知名字直接报错（配置写错要立刻看得见）。
pub fn write_config(writer: &str, path: &Path, m: &ModelWrite<'_>) -> Result<(), String> {
    match writer {
        "workbuddy" => write_workbuddy(path, m),
        other => Err(format!("未知的自定义配置写入器: {other}")),
    }
}

/// 读回「当前写定的模型」（界面回显用）。
pub fn read_model(writer: &str, path: &Path) -> Option<String> {
    match writer {
        "workbuddy" => read_workbuddy(path),
        _ => None,
    }
}

/// 把 base 补成 WorkBuddy 要求的完整 `/chat/completions` 端点。
///
/// 它不做 OpenAI SDK 那种「自己拼路径」的事：给了 `https://x/v1` 它会去请求
/// `https://x/v1` 本身，所以必须写全。规则与项目里其它拼 URL 的地方一致，避免 `/v1/v1`。
pub fn ensure_chat_completions(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return String::new();
    }
    if base.ends_with("/chat/completions") {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        return format!("{base}/chat/completions");
    }
    format!("{base}/v1/chat/completions")
}

/// 从 URL 里取一个可展示的「厂商名」（WorkBuddy 界面会显示它）。
///
/// 回环地址（走本地代理时）看不出上游是谁，统一叫 `local`，别显示成 127.0.0.1。
pub fn vendor_of(url: &str) -> String {
    let rest = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host_port = rest.split('/').next().unwrap_or("");
    // 去掉端口与 userinfo
    let host = host_port.rsplit('@').next().unwrap_or(host_port);
    let host = host.split(':').next().unwrap_or(host).trim();
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() {
        return "custom".to_string();
    }
    if host == "127.0.0.1" || host == "localhost" || host == "::1" || host == "[::1]" {
        return "local".to_string();
    }
    host.to_string()
}

/// WorkBuddy（`~/.workbuddy/models.json`）与 WorkBuddy AI（`~/.workbuddy-ai/models.json`）。
///
/// 语义是**单条覆盖**（「切换模型」，不累积），与 EchoBird 一致；原有条目会被替换掉，
/// 所以替换前数一下旧条目并写进日志，用户至少能在日志里看到发生了什么。
fn write_workbuddy(path: &Path, m: &ModelWrite<'_>) -> Result<(), String> {
    if m.base_url.trim().is_empty() {
        return Err("WorkBuddy 需要上游地址（base URL），请先选择供应商与模型".to_string());
    }
    if m.api_key.trim().is_empty() {
        return Err("WorkBuddy 需要 API Key，请先给供应商填 Key".to_string());
    }
    let model_id = if m.model.trim().is_empty() {
        m.claimed.trim()
    } else {
        m.model.trim()
    };
    if model_id.is_empty() {
        return Err("WorkBuddy 需要一个模型 id".to_string());
    }
    let display = {
        let claimed = m.claimed.trim();
        if claimed.is_empty() {
            model_id
        } else {
            claimed
        }
    };

    if let Some(old) = read_workbuddy_raw(path) {
        let replaced = old
            .get("models")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if replaced > 1 {
            eprintln!("[config_file] WorkBuddy: 覆盖原有 {replaced} 个自定义模型（单条覆盖语义）");
        }
    }

    // 厂商优先取真实上游；没给上游（如「只保存模型」直连模式）就用请求地址的 host
    let vendor = {
        let upstream = vendor_of(m.upstream_url);
        if upstream == "custom" {
            vendor_of(m.base_url)
        } else {
            upstream
        }
    };
    let doc = serde_json::json!({
        "models": [{
            "id": model_id,
            "name": display,
            "vendor": vendor,
            "url": ensure_chat_completions(m.base_url),
            "apiKey": m.api_key,
            "maxInputTokens": WORKBUDDY_MAX_INPUT_TOKENS,
            "maxOutputTokens": WORKBUDDY_MAX_OUTPUT_TOKENS,
            "supportsToolCall": true,
            "supportsImages": true
        }],
        "availableModels": [model_id]
    });

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 WorkBuddy 配置目录失败: {e}（{}）", parent.display()))?;
    }
    // 必须 UTF-8 **不带 BOM**：个别桌面构建解析带 BOM 的 models.json 会直接失败
    // （EchoBird workbuddy.rs 里踩过）。serde_json 写出来天然无 BOM。
    let text = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化 WorkBuddy 配置失败: {e}"))?;
    std::fs::write(path, text)
        .map_err(|e| format!("写入 WorkBuddy 配置失败: {e}（{}）", path.display()))?;
    eprintln!(
        "[config_file] WorkBuddy 模型已写入 {}：{}（{}）",
        path.display(),
        display,
        ensure_chat_completions(m.base_url)
    );
    Ok(())
}

fn read_workbuddy_raw(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()
}

/// 读回第一条模型的 id（WorkBuddy 界面上「当前模型」就是它）。
fn read_workbuddy(path: &Path) -> Option<String> {
    let doc = read_workbuddy_raw(path)?;
    let id = doc
        .get("models")?
        .as_array()?
        .first()?
        .get("id")?
        .as_str()?
        .trim()
        .to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{ensure_chat_completions, read_model, vendor_of, write_config, ModelWrite};
    use std::path::PathBuf;

    fn probe_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-toolcfg-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("models.json")
    }

    /// WorkBuddy 要求完整端点：给的 base 不论带不带 /v1，都要补成 .../chat/completions。
    #[test]
    fn chat_completions_url_is_always_complete() {
        assert_eq!(
            ensure_chat_completions("https://api.x.com"),
            "https://api.x.com/v1/chat/completions"
        );
        assert_eq!(
            ensure_chat_completions("https://api.x.com/v1/"),
            "https://api.x.com/v1/chat/completions"
        );
        // 已经是完整端点 → 不重复补（否则会得到 /chat/completions/chat/completions）
        assert_eq!(
            ensure_chat_completions("https://api.x.com/v1/chat/completions"),
            "https://api.x.com/v1/chat/completions"
        );
        // 网关自带版本段：只补 chat/completions
        assert_eq!(
            ensure_chat_completions("https://gw.x.com/openai/v2"),
            "https://gw.x.com/openai/v2/v1/chat/completions"
        );
        // 走本地代理时 base 就是 127.0.0.1:port
        assert_eq!(
            ensure_chat_completions("http://127.0.0.1:15721"),
            "http://127.0.0.1:15721/v1/chat/completions"
        );
        assert_eq!(ensure_chat_completions(""), "");
    }

    #[test]
    fn vendor_is_a_readable_hostname() {
        assert_eq!(vendor_of("https://api.deepseek.com/v1"), "api.deepseek.com");
        assert_eq!(vendor_of("https://www.x.com:8443/v1"), "x.com");
        assert_eq!(vendor_of("http://127.0.0.1:15721"), "local");
        assert_eq!(vendor_of(""), "custom");
    }

    /// 写入 → 读回：字段齐全、URL 是完整端点、availableModels 与 models[0] 对得上。
    #[test]
    fn workbuddy_write_and_read_round_trip() {
        let path = probe_path("wb");
        write_config(
            "workbuddy",
            &path,
            &ModelWrite {
                model: "deepseek-chat",
                claimed: "gpt-4.1",
                base_url: "http://127.0.0.1:15721",
                api_key: "sk-test",
                upstream_url: "https://api.deepseek.com/v1",
            },
        )
        .expect("写入应成功");

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let first = &doc["models"][0];
        assert_eq!(first["id"], "deepseek-chat");
        // name 用声明名（伪装），id 用真实模型
        assert_eq!(first["name"], "gpt-4.1");
        assert_eq!(first["url"], "http://127.0.0.1:15721/v1/chat/completions");
        assert_eq!(first["apiKey"], "sk-test");
        // 厂商取真实上游，不是回环地址
        assert_eq!(first["vendor"], "api.deepseek.com");
        assert_eq!(first["supportsToolCall"], true);
        assert_eq!(doc["availableModels"][0], "deepseek-chat");

        assert_eq!(
            read_model("workbuddy", &path).as_deref(),
            Some("deepseek-chat")
        );

        // 单条覆盖：再写一次不会累积成两条
        write_config(
            "workbuddy",
            &path,
            &ModelWrite {
                model: "glm-4.6",
                claimed: "",
                base_url: "https://open.bigmodel.cn/api/paas/v4",
                api_key: "sk-2",
                upstream_url: "",
            },
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["models"].as_array().unwrap().len(), 1);
        assert_eq!(read_model("workbuddy", &path).as_deref(), Some("glm-4.6"));
        // claimed 为空 → name 回退成模型 id；vendor 回退成 base 的 host
        assert_eq!(doc["models"][0]["name"], "glm-4.6");
        assert_eq!(doc["models"][0]["vendor"], "open.bigmodel.cn");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 缺 key / base 时明确报错，别写出一份工具读不了的半成品。
    #[test]
    fn workbuddy_requires_url_and_key() {
        let path = probe_path("wb-fail");
        let with = |base_url: &'static str, api_key: &'static str| ModelWrite {
            model: "m",
            claimed: "m",
            base_url,
            api_key,
            upstream_url: "",
        };
        // 缺 base / 缺 key 都要明确报错，不能写出半成品
        assert!(write_config("workbuddy", &path, &with("", "sk")).is_err());
        assert!(write_config("workbuddy", &path, &with("https://x/v1", " ")).is_err());
        // 未知写入器名也要报错（配置写错立刻可见）
        assert!(write_config("nope", &path, &with("https://x/v1", "sk")).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
