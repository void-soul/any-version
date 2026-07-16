pub mod google;
pub mod optimizers;
pub mod server;
pub mod sse;
pub mod transform;
pub mod types;

/// 归一化函数参数的 JSON Schema，确保 `type` 始终为 "object"。
///
/// 某些上游工具定义为 `parameters: null` 或 `{"type": null}`（如部分 Responses 工具），
/// 但严格 OpenAI 兼容供应商（DeepSeek 等）要求 `{"type": "object", "properties": {...}}`，
/// 否则返回 HTTP 400。抄自 cc-switch 9ca1a41f。
pub fn normalize_function_parameters(params: Option<&serde_json::Value>) -> serde_json::Value {
    let mut params = match params {
        Some(serde_json::Value::Object(obj)) => serde_json::Value::Object(obj.clone()),
        _ => serde_json::json!({ "type": "object", "properties": {} }),
    };
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
