//! 供应商余额 / 余量查询。
//!
//! 仅对官方提供公开余额端点的预设生效（见 [`balance_endpoint`]）；
//! 解析逻辑拆成纯函数便于单测，网络调用集中在 [`query_provider_balance`]。

use serde::Serialize;

/// 余额明细项。`key` 是稳定标识（前端按它做 i18n），`value` 是展示值。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BalanceItem {
    pub key: String,
    pub value: String,
}

/// 查询结果。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProviderBalance {
    pub items: Vec<BalanceItem>,
}

/// 支持余额查询的供应商及其官方端点。
/// DeepSeek:  GET /user/balance（Bearer）
/// SiliconFlow: GET /v1/user/info（Bearer）
/// OpenRouter:  GET /api/v1/auth/key（Bearer）
pub fn balance_endpoint(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "deepseek" => Some("https://api.deepseek.com/user/balance"),
        "siliconflow" => Some("https://api.siliconflow.cn/v1/user/info"),
        "openrouter" => Some("https://openrouter.ai/api/v1/auth/key"),
        _ => None,
    }
}

fn item(key: &str, value: String) -> BalanceItem {
    BalanceItem {
        key: key.to_string(),
        value,
    }
}

/// DeepSeek `GET /user/balance` 解析。
/// `{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00",
///   "granted_balance":"10.00","topped_up_balance":"100.00"}]}`
pub fn parse_deepseek(body: &serde_json::Value) -> Option<ProviderBalance> {
    let info = body.get("balance_infos")?.as_array()?.first()?;
    let currency = info.get("currency").and_then(|v| v.as_str()).unwrap_or("");
    let suffix = if currency.is_empty() {
        String::new()
    } else {
        format!(" {currency}")
    };
    let mut items = Vec::new();
    if let Some(total) = info.get("total_balance").and_then(|v| v.as_str()) {
        items.push(item("total", format!("{total}{suffix}")));
    }
    if let Some(granted) = info.get("granted_balance").and_then(|v| v.as_str()) {
        if granted != "0.00" {
            items.push(item("granted", format!("{granted}{suffix}")));
        }
    }
    if let Some(topped) = info.get("topped_up_balance").and_then(|v| v.as_str()) {
        if topped != "0.00" {
            items.push(item("toppedUp", format!("{topped}{suffix}")));
        }
    }
    if let Some(available) = body.get("is_available").and_then(|v| v.as_bool()) {
        items.push(item("state", if available { "ok" } else { "disabled" }.to_string()));
    }
    if items.is_empty() {
        None
    } else {
        Some(ProviderBalance { items })
    }
}

/// SiliconFlow `GET /v1/user/info` 解析。
/// `{"data":{"balance":"10.50","totalBalance":"12.34","status":"normal",...}}`
pub fn parse_siliconflow(body: &serde_json::Value) -> Option<ProviderBalance> {
    let data = body.get("data")?;
    let mut items = Vec::new();
    if let Some(total) = data.get("totalBalance").and_then(|v| v.as_str()) {
        items.push(item("total", format!("{total} CNY")));
    }
    if let Some(cash) = data.get("balance").and_then(|v| v.as_str()) {
        items.push(item("cash", format!("{cash} CNY")));
    }
    if items.is_empty() {
        None
    } else {
        Some(ProviderBalance { items })
    }
}

/// OpenRouter `GET /api/v1/auth/key` 解析（单位美元）。
/// `{"data":{"label":"...","usage":0.12,"limit":null,...}}`（limit 为 null = 不限）
pub fn parse_openrouter(body: &serde_json::Value) -> Option<ProviderBalance> {
    let data = body.get("data")?;
    let mut items = Vec::new();
    if let Some(usage) = data.get("usage").and_then(|v| v.as_f64()) {
        items.push(item("used", format!("${usage:.2}")));
    }
    match data.get("limit") {
        Some(v) if v.is_null() => items.push(item("limit", "∞".to_string())),
        Some(v) => {
            if let Some(limit) = v.as_f64() {
                items.push(item("limit", format!("${limit:.2}")));
            }
        }
        None => {}
    }
    if items.is_empty() {
        None
    } else {
        Some(ProviderBalance { items })
    }
}

/// 按供应商 id 分发解析。
pub fn parse_balance_response(provider_id: &str, body: &serde_json::Value) -> Option<ProviderBalance> {
    match provider_id {
        "deepseek" => parse_deepseek(body),
        "siliconflow" => parse_siliconflow(body),
        "openrouter" => parse_openrouter(body),
        _ => None,
    }
}

/// 查询供应商余额（仅预设 id 命中 [`balance_endpoint`] 时可用）。
#[tauri::command]
pub async fn query_provider_balance(provider_id: String, api_key: String) -> Result<ProviderBalance, String> {
    let url = balance_endpoint(&provider_id)
        .ok_or_else(|| "该供应商不支持余额查询".to_string())?;
    let key = api_key.trim();
    if key.is_empty() {
        return Err("请先为该供应商配置 API Key".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;
    let resp = client
        .get(url)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| format!("余额查询请求失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        // 401 通常是 key 无效，给出可行动的提示
        if status.as_u16() == 401 {
            return Err("鉴权失败（401），请检查 API Key 是否有效".to_string());
        }
        return Err(format!("余额查询失败 (HTTP {})", status.as_u16()));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("余额响应解析失败: {e}"))?;
    parse_balance_response(&provider_id, &body)
        .ok_or_else(|| "响应格式无法识别，官方接口可能已变更".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_deepseek_full() {
        let body = json!({
            "is_available": true,
            "balance_infos": [{
                "currency": "CNY",
                "total_balance": "110.00",
                "granted_balance": "10.00",
                "topped_up_balance": "100.00"
            }]
        });
        let parsed = parse_deepseek(&body).expect("应解析成功");
        assert_eq!(parsed.items[0], item("total", "110.00 CNY".into()));
        assert_eq!(parsed.items[1], item("granted", "10.00 CNY".into()));
        assert_eq!(parsed.items[2], item("toppedUp", "100.00 CNY".into()));
        assert_eq!(parsed.items[3], item("state", "ok".into()));
    }

    #[test]
    fn test_parse_deepseek_zero_grant_omitted() {
        let body = json!({
            "is_available": false,
            "balance_infos": [{"currency": "CNY", "total_balance": "0.00",
                "granted_balance": "0.00", "topped_up_balance": "0.00"}]
        });
        let parsed = parse_deepseek(&body).expect("应解析成功");
        // 0.00 赠金/充值不展示，只留 total + state
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[1], item("state", "disabled".into()));
    }

    #[test]
    fn test_parse_siliconflow() {
        let body = json!({"data": {"balance": "10.50", "totalBalance": "12.34", "status": "normal"}});
        let parsed = parse_siliconflow(&body).expect("应解析成功");
        assert_eq!(parsed.items[0], item("total", "12.34 CNY".into()));
        assert_eq!(parsed.items[1], item("cash", "10.50 CNY".into()));
    }

    #[test]
    fn test_parse_openrouter_limit_null_means_unlimited() {
        let body = json!({"data": {"usage": 1.255, "limit": null}});
        let parsed = parse_openrouter(&body).expect("应解析成功");
        assert_eq!(parsed.items[0], item("used", "$1.25".into()));
        assert_eq!(parsed.items[1], item("limit", "∞".into()));
    }

    #[test]
    fn test_parse_openrouter_with_limit() {
        let body = json!({"data": {"usage": 0.0, "limit": 20.0}});
        let parsed = parse_openrouter(&body).expect("应解析成功");
        assert_eq!(parsed.items[1], item("limit", "$20.00".into()));
    }

    #[test]
    fn test_parse_garbage_returns_none() {
        assert!(parse_deepseek(&json!({"foo": 1})).is_none());
        assert!(parse_siliconflow(&json!({"data": {}})).is_none());
        assert!(parse_openrouter(&json!({"data": {}})).is_none());
        assert!(parse_balance_response("unknown-provider", &json!({})).is_none());
    }

    #[test]
    fn test_balance_endpoint_only_known_ids() {
        assert!(balance_endpoint("deepseek").is_some());
        assert!(balance_endpoint("siliconflow").is_some());
        assert!(balance_endpoint("openrouter").is_some());
        assert!(balance_endpoint("openai").is_none());
        assert!(balance_endpoint("custom_123").is_none());
    }
}
