//! model configuration responsibility.
use super::*;

/// 根据工具 config.json 中的 configFile 字段，自动写入工具配置文件。
/// 不注入任何环境变量，全部参数（模型 / baseUrl / apiKey / 别名）都写进配置文件，
/// 便于对照各工具官方文档逐项核对。
///
/// - `model_id`：实际模型 B（供应商模型）。
/// - `claimed_model`：声明模型名 C（工具以为自己调用的模型；伪装时 = masquerade_model，
///   否则 = B）。配置文件中的 `model` 字段写入 C，由本地代理按 masquerade 映射 C → B。
pub(crate) fn write_tool_config_from_spec(
    tool_config: &ToolConfig,
    model_id: Option<&str>,
    claimed_model: Option<&str>,
    base_url: &str,
    api_key: &str,
    // 真实上游端点。仅用于写「厂商」这类**展示用**元信息：走本地代理时 `base_url`
    // 是 127.0.0.1，直接拿它当厂商名会显示成本机回环。留空表示直连（base_url 即上游）。
    upstream_url: &str,
    fallback_model_id: Option<&str>,
    fallback_masquerade_model: Option<&str>,
    one_m_context: bool,
    fallback_one_m_context: bool,
    proxy_mode: bool,
    custom_params: &[ModelCustomParam],
    custom_param_values: &HashMap<String, String>,
    web_search: bool,
    chosen_protocol: &str,
) -> Result<(), String> {
    // write_tool_config_generic 内部会检查 config_file 是否存在，无 configFile 时直接返回 Ok(())
    write_tool_config_generic(
        tool_config,
        model_id,
        claimed_model,
        base_url,
        api_key,
        upstream_url,
        fallback_model_id,
        fallback_masquerade_model,
        one_m_context,
        fallback_one_m_context,
        proxy_mode,
        custom_params,
        custom_param_values,
        web_search,
        chosen_protocol,
    )
}

/// 从 config_file.write 映射中提取 env.* 前缀的键，构建环境变量 HashMap。
/// 值模板匹配：apiKey → api_key, baseUrl → base_url, model/modelName → model, 其他 → 字面值。
/// 空值不注入。供 launch_ai_tool 和 collab dispatch_to_tool 共用。
pub(crate) fn build_env_vars(
    tool_config: &ToolConfig,
    api_key: &str,
    base_url: &str,
    model: &str,
) -> HashMap<String, String> {
    let mut envs = HashMap::new();
    if let Some(ref cf) = tool_config.config_file {
        if let Some(ref write_map) = cf.write {
            for (path, value_template) in write_map {
                if path.starts_with("env.") {
                    let env_key = &path[4..];
                    let env_value = match value_template.as_str() {
                        "apiKey" => api_key.to_string(),
                        "baseUrl" => base_url.to_string(),
                        "model" | "modelName" => model.to_string(),
                        other => other.to_string(),
                    };
                    if !env_value.is_empty() {
                        envs.insert(env_key.to_string(), env_value);
                    }
                }
            }
        }
    }
    envs
}

/// 根据 modelFormat 配置格式化模型名
pub(crate) fn format_model_name(raw: &str, tool_config: &ToolConfig) -> String {
    if raw.is_empty() {
        return String::new();
    }
    if let Some(ref fmt) = tool_config.model_format {
        let prefix = fmt.prefix.as_deref().unwrap_or("");
        if fmt.extract_last {
            let name = raw.split('/').next_back().unwrap_or(raw);
            format!("{}{}", prefix, name)
        } else {
            format!("{}{}", prefix, raw)
        }
    } else {
        raw.to_string()
    }
}

/// 格式化模型名，并按需追加 [1m]（1M 上下文后缀，仅 Anthropic 协议工具需要）
pub(super) fn format_model_name_with_ctx(
    raw: &str,
    tool_config: &ToolConfig,
    one_m: bool,
) -> String {
    let mut s = format_model_name(raw, tool_config);
    if one_m && !s.contains("[1m]") {
        s = format!("{}[1m]", s);
    }
    s
}

/// 通用工具配置文件写入：根据 config.json 的 configFile.write 映射写入。
/// 支持 json / jsonc（serde_json）与 toml（行式）两种格式。
///
/// `model` 字段写入**声明模型名 C**（claimed_model，回退到实际模型 B），交由本地代理
/// 按 masquerade 映射 C → B 转发到上游。代理模式下不再跳过模型字段——工具必须以 C
/// 发起请求，代理才能正确改写。模型伪装（C → B 的具体映射）由启动时代理动态持有，
/// 这里不再写 ANTHROPIC_DEFAULT_* 之类的别名环境变量。
/// 模型名可能含 "."（如 LongCat-2.0 / gpt-4.1），而配置路径用 "." 作层级分隔符。
/// 写路径前将模型名里的 "." 转义为此占位符，set_json_path 落盘时再还原，
/// 避免 "provider.anyversion.models.LongCat-2.0.name" 被误拆成 LongCat-2 → 0 → name。
pub(super) const MODEL_NAME_DOT_ESCAPE: &str = "__DOT__";

/// 账本里存的值形态：标量取字符串形态（布尔/数字统一成字面量文本），
/// 数组/对象存 JSON 文本。**只用于「和后来读到的值比对」，不是配置内容。**
/// 测试用：按文件扩展名 + 声明默认值解析写入格式。
#[cfg(test)]
pub(crate) fn launch_write_format_for_test(p: &std::path::Path, default: &str) -> WriteFormat {
    write_format_for(p, default)
}

pub(crate) fn ledger_value_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}
