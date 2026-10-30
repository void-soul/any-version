use super::*;

// ─── OpenAPI 3.0 导出 ───

/// 从接口 URL 提取 OpenAPI path（剥离协议/域名/查询串，`{{baseUrl}}` 前缀视为服务器地址）。
pub(super) fn url_path_of(url: &str) -> String {
    let no_query = url.split('?').next().unwrap_or(url).trim();
    let rest = no_query.split("://").nth(1).unwrap_or(no_query);
    let path_start = if rest.starts_with("{{") {
        rest.find("}}").map(|i| i + 2).unwrap_or(0)
    } else if rest.starts_with('/') {
        0
    } else {
        rest.find('/').unwrap_or(rest.len())
    };
    let mut path = rest[path_start..].to_string();
    if path.is_empty() {
        path = "/".to_string();
    }
    // Express 风格 :id → OpenAPI {id}
    if path.contains(':') {
        path = path
            .split('/')
            .map(|seg| match seg.strip_prefix(':') {
                Some(name) if !name.is_empty() => format!("{{{}}}", name),
                _ => seg.to_string(),
            })
            .collect::<Vec<_>>()
            .join("/");
    }
    path
}

pub(super) fn sanitize_identifier(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

pub(super) fn openapi_body(ep: &crate::commands::api::models::ApiEndpoint) -> Option<Value> {
    match ep.body_type.as_str() {
        "json" => {
            let example = serde_json::from_str::<Value>(&ep.body)
                .unwrap_or_else(|_| Value::String(ep.body.clone()));
            Some(serde_json::json!({
                "content": { "application/json": { "schema": { "type": "object" }, "example": example } }
            }))
        }
        "raw" => Some(serde_json::json!({
            "content": { "text/plain": { "schema": { "type": "string" }, "example": ep.body } }
        })),
        "form" => {
            let mut props = serde_json::Map::new();
            let mut example = serde_json::Map::new();
            for kv in ep
                .body_urlencoded
                .iter()
                .filter(|kv| kv.enabled && !kv.key.is_empty())
            {
                props.insert(kv.key.clone(), serde_json::json!({ "type": "string" }));
                example.insert(kv.key.clone(), Value::String(kv.value.clone()));
            }
            Some(serde_json::json!({
                "content": {
                    "application/x-www-form-urlencoded": {
                        "schema": { "type": "object", "properties": props },
                        "example": example
                    }
                }
            }))
        }
        "formdata" => {
            let mut props = serde_json::Map::new();
            for item in ep
                .body_form
                .iter()
                .filter(|f| f.enabled && !f.key.is_empty())
            {
                props.insert(
                    item.key.clone(),
                    if item.kind == "file" {
                        serde_json::json!({ "type": "string", "format": "binary" })
                    } else {
                        serde_json::json!({ "type": "string" })
                    },
                );
            }
            Some(serde_json::json!({
                "content": {
                    "multipart/form-data": {
                        "schema": { "type": "object", "properties": props }
                    }
                }
            }))
        }
        "graphql" => {
            let variables =
                serde_json::from_str::<Value>(&ep.body_graphql_variables).unwrap_or(Value::Null);
            Some(serde_json::json!({
                "content": {
                    "application/json": {
                        "schema": { "type": "object" },
                        "example": { "query": ep.body_graphql_query, "variables": variables }
                    }
                }
            }))
        }
        "binary" => Some(serde_json::json!({
            "content": { "application/octet-stream": { "schema": { "type": "string", "format": "binary" } } }
        })),
        _ => None,
    }
}

/// 导出 OpenAPI 3.0.3 文档（JSON 字符串）。
pub fn export_openapi(
    project_name: &str,
    description: &str,
    modules: &[(String, String)], // (module_id, name)
    endpoints: &[crate::commands::api::models::ApiEndpoint],
    env_variables: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let module_name = |id: Option<&str>| -> Option<String> {
        id.and_then(|i| {
            modules
                .iter()
                .find(|(mid, _)| mid == i)
                .map(|(_, n)| n.clone())
        })
    };
    // 这些请求头由 HTTP 客户端自动生成，导出为文档参数没有意义
    const AUTO_HEADERS: &[&str] = &[
        "content-type",
        "content-length",
        "host",
        "user-agent",
        "accept",
        "accept-encoding",
        "connection",
    ];

    let mut paths = serde_json::Map::new();
    let mut schemes = serde_json::Map::new();
    let mut op_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ep in endpoints {
        let path = url_path_of(&ep.url);
        let method = ep.method.to_ascii_lowercase();
        let mut op = serde_json::Map::new();
        op.insert("summary".into(), Value::String(ep.name.clone()));
        let desc = if ep.description.is_empty() {
            ep.docs_md.clone()
        } else if ep.docs_md.is_empty() {
            ep.description.clone()
        } else {
            format!("{}\n\n{}", ep.description, ep.docs_md)
        };
        if !desc.is_empty() {
            op.insert("description".into(), Value::String(desc));
        }
        let mut op_id = sanitize_identifier(&ep.name);
        if op_id.is_empty() {
            op_id = format!("{}{}", method, sanitize_identifier(&path));
        }
        if op_id.is_empty() {
            op_id = "operation".to_string();
        }
        let base_id = op_id.clone();
        let mut seq = 2;
        while !op_ids.insert(op_id.clone()) {
            op_id = format!("{}_{}", base_id, seq);
            seq += 1;
        }
        op.insert("operationId".into(), Value::String(op_id));
        if let Some(tag) = module_name(ep.module_id.as_deref()) {
            op.insert("tags".into(), serde_json::json!([tag]));
        }

        // 参数：query / path / header
        let mut params: Vec<Value> = Vec::new();
        for q in ep
            .query_params
            .iter()
            .filter(|x| x.enabled && !x.key.is_empty())
        {
            params.push(serde_json::json!({
                "name": q.key, "in": "query", "required": false,
                "schema": { "type": "string" }, "example": q.value
            }));
        }
        for p in ep
            .path_params
            .iter()
            .filter(|x| x.enabled && !x.key.is_empty())
        {
            params.push(serde_json::json!({
                "name": p.key, "in": "path", "required": true,
                "schema": { "type": "string" }, "example": p.value
            }));
        }
        for h in ep.headers.iter().filter(|x| {
            x.enabled
                && !x.key.is_empty()
                && !AUTO_HEADERS.contains(&x.key.to_ascii_lowercase().as_str())
        }) {
            params.push(serde_json::json!({
                "name": h.key, "in": "header", "required": false,
                "schema": { "type": "string" }, "example": h.value
            }));
        }
        if !params.is_empty() {
            op.insert("parameters".into(), Value::Array(params));
        }

        if let Some(body) = openapi_body(ep) {
            op.insert("requestBody".into(), body);
        }
        op.insert(
            "responses".into(),
            serde_json::json!({ "200": { "description": "OK" } }),
        );

        // 安全方案（同名方案去重）
        let auth = &ep.authorization;
        match auth.r#type.as_str() {
            "bearer" | "jwt" => {
                schemes
                    .entry("bearerAuth".to_string())
                    .or_insert_with(|| serde_json::json!({ "type": "http", "scheme": "bearer" }));
                op.insert("security".into(), serde_json::json!([{ "bearerAuth": [] }]));
            }
            "basic" => {
                schemes
                    .entry("basicAuth".to_string())
                    .or_insert_with(|| serde_json::json!({ "type": "http", "scheme": "basic" }));
                op.insert("security".into(), serde_json::json!([{ "basicAuth": [] }]));
            }
            "apiKey" if !auth.api_key_name.trim().is_empty() => {
                let name = format!("apiKey_{}", sanitize_identifier(&auth.api_key_name));
                schemes.entry(name.clone()).or_insert_with(|| {
                    serde_json::json!({
                        "type": "apiKey",
                        "in": if auth.api_key_in == "query" { "query" } else { "header" },
                        "name": auth.api_key_name
                    })
                });
                op.insert("security".into(), serde_json::json!([{ name: [] }]));
            }
            _ => {}
        }

        let entry = paths
            .entry(path)
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Value::Object(map) = entry {
            // 同一 path+method 只保留第一条，避免生成非法文档
            if !map.contains_key(&method) {
                map.insert(method, Value::Object(op));
            }
        }
    }

    let mut info = serde_json::json!({ "title": project_name, "version": "1.0.0" });
    if !description.trim().is_empty() {
        info["description"] = Value::String(description.to_string());
    }
    let mut doc = serde_json::json!({
        "openapi": "3.0.3",
        "info": info,
        "paths": Value::Object(paths),
    });
    if let Some(base) = env_variables.get("baseUrl").and_then(|v| v.as_str()) {
        if !base.trim().is_empty() {
            doc["servers"] = serde_json::json!([{ "url": base }]);
        }
    }
    if !schemes.is_empty() {
        doc["components"] = serde_json::json!({ "securitySchemes": Value::Object(schemes) });
    }
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}
