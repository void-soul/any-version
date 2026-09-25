pub mod google;
pub mod headers;
pub mod optimizers;
pub mod server;
pub mod sse;
pub mod transform;
pub mod types;
pub mod upstream;

/// 归一化函数参数的 JSON Schema，确保 `type` 始终为 "object"。
///
/// 某些上游工具定义为 `parameters: null` 或 `{"type": null}`（如部分 Responses 工具），
/// 但严格 OpenAI 兼容供应商（DeepSeek 等）要求 `{"type": "object", "properties": {...}}`，
/// 否则返回 HTTP 400。抄自 cc-switch 9ca1a41f。
pub fn normalize_function_parameters(params: Option<&serde_json::Value>) -> serde_json::Value {
    let mut params = match params {
        Some(serde_json::Value::Object(obj)) => serde_json::Value::Object(obj.clone()),
        _ => return serde_json::json!({ "type": "object", "properties": {} }),
    };
    // 先递归剥掉嵌套的 `type: null`：它只在根节点被下面的分支兜住，
    // 嵌套节点原样透传会触发严格供应商的 400（抄自 CodexPlusPlus a3286e9）。
    strip_null_type(&mut params);
    if let Some(obj) = params.as_object_mut() {
        match obj.get("type").and_then(|v| v.as_str()) {
            Some("object") => {}
            _ => {
                obj.insert("type".to_string(), serde_json::json!("object"));
            }
        }
    }
    params
}

/// 递归剥除 schema 中所有 `"type": null`（含根与嵌套节点）。
///
/// JSON Schema 任何草案都要求 `type` 是字符串，`null` 恒为非法构造，剥掉等价于未声明。
/// 只动 `type` 这一个键：`oneOf`/`anyOf` 等组合器、以及 `default: null` 这类合法 null 保持不变。
fn strip_null_type(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(obj) => {
            if obj.get("type").is_some_and(|v| v.is_null()) {
                obj.remove("type");
            }
            for (_, child) in obj.iter_mut() {
                strip_null_type(child);
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr.iter_mut() {
                strip_null_type(child);
            }
        }
        _ => {}
    }
}

/// 代理端口的环境变量名：手动启动代理时用它覆盖配置里的端口。
///
/// 抄自 CodexPlusPlus 0e9a86a 的 `protocol_proxy_port()` 模式：Windows 上 Hyper-V/WSL 会把
/// 一段动态端口划入系统保留区间，默认端口可能正好落在里面（bind 报 os error 10013），
/// 此时用户无法改配置文件也不想改 UI，用环境变量能一次性把端口挪开。
pub const PROXY_PORT_ENV: &str = "ANYVERSION_PROXY_PORT";

/// 解析实际监听端口：环境变量优先，取值非法（非数字 / 0）时回落传入的配置值。
pub fn resolve_proxy_port(configured: u16) -> u16 {
    std::env::var(PROXY_PORT_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(configured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_function_parameters_strips_nested_type_null_only() {
        // 嵌套节点的 `type: null` 在 JSON Schema 里恒非法（type 必须是字符串），
        // 严格供应商（DeepSeek 等）会以 `got 'type: null'` 拒绝整个请求；
        // 剥掉等价于未声明（抄自 CodexPlusPlus a3286e9）。
        let params = json!({
            "type": "object",
            "properties": {
                "a": {"type": null},
                "b": {"type": "array", "items": {"type": null}},
                "c": {"default": null}
            }
        });
        let out = normalize_function_parameters(Some(&params));
        assert!(out["properties"]["a"].get("type").is_none(), "{out}");
        assert!(out["properties"]["b"]["items"].get("type").is_none(), "{out}");
        // 非 type 的 null 必须原样保留
        assert!(out["properties"]["c"]["default"].is_null(), "{out}");
    }

    #[test]
    fn normalize_function_parameters_defaults_null_root_type_to_object() {
        let out = normalize_function_parameters(Some(&json!({"type": null, "properties": {}})));
        assert_eq!(out["type"], json!("object"));
        // 非对象 / 缺失一律退回最小可用 schema
        let empty = json!({"type": "object", "properties": {}});
        assert_eq!(normalize_function_parameters(None), empty);
        assert_eq!(normalize_function_parameters(Some(&json!(null))), empty);
    }

    #[test]
    fn normalize_function_parameters_preserves_combinators() {
        // 只处理 `type` 这一个键，oneOf/anyOf 等组合器维持原样
        let params = json!({
            "type": "object",
            "properties": {"x": {"oneOf": [{"type": "string"}, {"type": "number"}]}}
        });
        let out = normalize_function_parameters(Some(&params));
        assert_eq!(out["properties"]["x"]["oneOf"][0]["type"], json!("string"));
        assert_eq!(out["properties"]["x"]["oneOf"][1]["type"], json!("number"));
    }
}
