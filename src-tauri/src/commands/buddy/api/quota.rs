//! quota responsibility.
use super::*;

// ─── 用量查询 ───

pub async fn fetch_dosage_notify(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-dosage-notify",
        api_endpoint(platform)
    );

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 dosage notify 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 dosage 响应失败: {}", e))?;

    Ok(body)
}

pub async fn fetch_payment_type(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-payment-type",
        api_endpoint(platform)
    );

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 payment type 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 payment type 响应失败: {}", e))?;

    Ok(body)
}

pub(super) async fn post_user_resource(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    _domain: Option<&str>,
    body: Value,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-user-resource",
        api_endpoint(platform)
    );

    let mut req = client
        .post(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 user resource 失败: {}", e))?;

    let status_code = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status_code == reqwest::StatusCode::UNAUTHORIZED
        || status_code == reqwest::StatusCode::FORBIDDEN
    {
        // 网关层直接拒（响应体常是 HTML）：令牌已被吊销，刷新也救不回来，交给上层置账号状态
        return Err(revoked_error(&format!("HTTP {}", status_code.as_u16())));
    }
    let body: Value = serde_json::from_str(&text).map_err(|e| {
        format!(
            "解析 user resource 响应失败: {} (http={})",
            e,
            status_code.as_u16()
        )
    })?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 user resource 失败 (http={}): {}",
            status_code.as_u16(),
            message
        ));
    }

    if let Some(code) = body.get("code").and_then(|v| v.as_i64()) {
        if code != 0 && code != 200 {
            let message = body
                .get("message")
                .or_else(|| body.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!(
                "请求 user resource 失败 (code={}): {}",
                code, message
            ));
        }
    }

    Ok(body)
}

pub async fn fetch_enterprise_user_usage(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: &str,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-enterprise-user-usage",
        api_endpoint(platform)
    );

    let mut req = client
        .post(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("X-Enterprise-Id", enterprise_id)
        .header("X-Tenant-Id", enterprise_id);
    if let Some(uid) = uid {
        req = req.header("X-User-Id", uid);
    }
    if let Some(domain) = domain {
        req = req.header("X-Domain", domain);
    }

    let resp = req
        .json(&json!({}))
        .send()
        .await
        .map_err(|e| format!("请求 enterprise user usage 失败: {}", e))?;

    let status_code = resp.status();
    let body: Value = resp.json().await.map_err(|e| {
        format!(
            "解析 enterprise user usage 响应失败: {} (http={})",
            e,
            status_code.as_u16()
        )
    })?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 enterprise user usage 失败 (http={}): {}",
            status_code.as_u16(),
            message
        ));
    }

    if let Some(code) = body.get("code").and_then(|v| v.as_i64()) {
        if code != 0 && code != 200 {
            let message = body
                .get("message")
                .or_else(|| body.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!(
                "请求 enterprise user usage 失败 (code={}): {}",
                code, message
            ));
        }
    }

    if enterprise_usage_data(&body)
        .and_then(|data| data.get("limit_num").or_else(|| data.get("limitNum")))
        .and_then(|value| json_f64(Some(value)))
        .is_none()
    {
        return Err("enterprise user usage 响应缺少 limit_num/limitNum".to_string());
    }

    Ok(body)
}

/// 将企业用量响应包装成前端可识别的 get-user-resource 结构
pub fn wrap_enterprise_usage_as_resource(usage_body: &Value) -> Result<Value, String> {
    let data = enterprise_usage_data(usage_body)
        .ok_or_else(|| "enterprise user usage 响应缺少 data".to_string())?;
    let limit_num = data
        .get("limit_num")
        .or_else(|| data.get("limitNum"))
        .and_then(|value| json_f64(Some(value)))
        .ok_or_else(|| "enterprise user usage 响应缺少 limit_num/limitNum".to_string())?;
    let used_num = data
        .get("used_num")
        .or_else(|| data.get("usedNum"))
        .or_else(|| data.get("credit"))
        .and_then(|value| json_f64(Some(value)))
        .ok_or_else(|| "enterprise user usage 响应缺少 credit/used_num".to_string())?;
    let unlimited = limit_num == -1.0;
    let remain = if unlimited {
        -1.0
    } else {
        (limit_num - used_num).max(0.0)
    };
    let cycle_start_time = json_scalar_string(
        data.get("cycle_start_time")
            .or_else(|| data.get("cycleStartTime")),
    );
    let cycle_end_time = json_scalar_string(
        data.get("cycle_end_time")
            .or_else(|| data.get("cycleEndTime")),
    );
    let cycle_reset_time = json_scalar_string(
        data.get("cycle_reset_time")
            .or_else(|| data.get("cycleResetTime")),
    );

    let account = json!({
        "PackageCode": ENTERPRISE_PACKAGE_CODE,
        "PackageName": "企业版",
        "CycleCapacitySizePrecise": limit_num.to_string(),
        "CycleCapacityRemainPrecise": remain.to_string(),
        "CycleCapacityUsedPrecise": used_num.to_string(),
        "CycleCapacitySize": limit_num,
        "CycleCapacityRemain": remain,
        "CycleCapacityUsed": used_num,
        "CapacitySize": limit_num,
        "CapacityRemain": remain,
        "CapacityUsed": used_num,
        "CapacityUnit": "credits",
        "CycleStartTime": cycle_start_time,
        "CycleEndTime": cycle_end_time,
        "CycleResetTime": cycle_reset_time,
        "Unlimited": unlimited,
        "Status": 0
    });

    Ok(json!({
        "code": 0,
        "msg": "OK",
        "data": {
            "Response": {
                "Data": {
                    "Accounts": [account],
                    "TotalCount": 1,
                    "TotalDosage": used_num
                }
            }
        }
    }))
}

pub(super) async fn fetch_user_resource_with_access_token_default(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let payload = post_user_resource(
        platform,
        access_token,
        uid,
        enterprise_id,
        domain,
        build_user_resource_request_body(),
    )
    .await?;
    if user_resource_has_payload(&payload) {
        Ok(payload)
    } else if user_resource_has_shape(&payload) {
        Err("user resource 响应未包含可用资源".to_string())
    } else {
        Err("user resource 响应缺少 resources/Accounts".to_string())
    }
}

pub(super) async fn fetch_quota_resource_for_account(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    if let Some(enterprise_id) = enterprise_id {
        let body =
            fetch_enterprise_user_usage(platform, access_token, uid, enterprise_id, domain).await?;
        wrap_enterprise_usage_as_resource(&body)
    } else {
        fetch_user_resource_with_access_token_default(platform, access_token, uid, None, domain)
            .await
    }
}
