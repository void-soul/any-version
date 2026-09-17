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
