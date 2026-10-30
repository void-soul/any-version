use super::*;

// ─── Postman 导入（v2.1 / v2.0） ───

pub(super) fn parse_postman_item(item: &Value, out: &mut Vec<EndpointDraft>, parent_module: &str) {
    let name = v_str(item, "name");
    if let Some(children) = item.get("item").and_then(|x| x.as_array()) {
        // 文件夹：模块（嵌套文件夹用 / 连接）
        let module = if parent_module.is_empty() {
            name
        } else if name.is_empty() {
            parent_module.to_string()
        } else {
            format!("{}/{}", parent_module, name)
        };
        for child in children {
            parse_postman_item(child, out, &module);
        }
        return;
    }
    let request = item.get("request");
    let Some(request) = request else { return };
    let method = v_str(request, "method").to_uppercase();
    let url_obj = request.get("url");
    let mut url = String::new();
    let mut query_params: Vec<KeyValueItem> = Vec::new();
    if let Some(u) = url_obj {
        // Postman v2.1 允许 `url` 直接写成字符串（`{"url": "https://x/y"}`）。
        // 只认 `url.raw` 的话，这类请求得到的 url 恒为空，于是被下面的
        // `if url.is_empty() { return }` 静默丢弃 —— 导入后少接口且不报错。
        url = match u.as_str() {
            Some(text) => text.to_string(),
            None => v_str(u, "raw"),
        };
        // 查询串刻意**留在 URL 里原样送出**，不拆进 query_params：执行时 `exec.rs`
        // 会把 query_params 追加到 URL 上（`request.query()`），拆出来既会重复一份，
        // 又会把 raw 里已编码的值再编码一次（`%E7%8C%AB` → `%25E7%8C%AB`）。
        if url.is_empty() {
            // 由 protocol/host/path 拼装
            let protocol = v_str(u, "protocol");
            let port = v_str(u, "port");
            let host = u
                .get("host")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                })
                .unwrap_or_default();
            let path = u
                .get("path")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join("/")
                })
                .unwrap_or_default();
            let mut buf = String::new();
            if !protocol.is_empty() {
                buf.push_str(&format!("{}://", protocol));
            }
            buf.push_str(&host);
            if !port.is_empty() {
                buf.push_str(&format!(":{}", port));
            }
            if !path.is_empty() {
                buf.push_str(&format!("/{}", path));
            }
            url = buf;
        }
        if let Some(query) = u.get("query").and_then(|x| x.as_array()) {
            // 有了结构化的 `query`，就把 raw 里的查询串剥掉：执行时 `exec.rs` 会把
            // `query_params` **追加**到 URL 上（`request.query()`），两边都留着会
            // 拼成 `?page=1&page=1`。值取自 `query` 里**已解码**的那份，因此也不存在
            // 二次编码问题（raw 里的 `%E7%8C%AB` 再编一次会变成 `%25E7%8C%AB`）。
            if !query.is_empty() {
                url = strip_query(&url).to_string();
            }
            for q in query {
                let key = v_str(q, "key");
                if key.is_empty() {
                    continue;
                }
                query_params.push(KeyValueItem {
                    key,
                    value: v_str(q, "value"),
                    enabled: kv_enabled(q),
                    description: v_str(q, "description"),
                    from_template: false,
                });
            }
        }
    }
    let mut headers: Vec<KeyValueItem> = Vec::new();
    if let Some(hs) = request.get("header").and_then(|x| x.as_array()) {
        for h in hs {
            let key = v_str(h, "key");
            if key.is_empty() {
                continue;
            }
            headers.push(KeyValueItem {
                key,
                value: v_str(h, "value"),
                enabled: kv_enabled(h),
                description: v_str(h, "description"),
                from_template: false,
            });
        }
    }
    // 认证
    let authorization = parse_postman_auth(request.get("auth"));
    // Cookie
    let mut cookies: Vec<KeyValueItem> = Vec::new();
    if let Some(cs) = request.get("cookie").and_then(|x| x.as_array()) {
        for c in cs {
            let key = v_str(c, "key");
            if !key.is_empty() {
                cookies.push(KeyValueItem {
                    key,
                    value: v_str(c, "value"),
                    enabled: kv_enabled(c),
                    description: v_str(c, "description"),
                    from_template: false,
                });
            }
        }
    }

    let mut body = String::new();
    let mut body_type = "none".to_string();
    let mut body_form: Vec<FormDataItem> = Vec::new();
    let mut body_urlencoded: Vec<KeyValueItem> = Vec::new();
    let mut graphql_query = String::new();
    let mut graphql_variables = String::new();
    if let Some(b) = request.get("body") {
        match v_str(b, "mode").as_str() {
            "raw" => {
                body = v_str(b, "raw");
                let header_says_json = headers.iter().any(|h| {
                    h.key.eq_ignore_ascii_case("content-type") && h.value.contains("json")
                });
                let looks_json =
                    body.trim_start().starts_with('{') || body.trim_start().starts_with('[');
                body_type = if header_says_json
                    || (looks_json && serde_json::from_str::<Value>(&body).is_ok())
                {
                    "json"
                } else {
                    "raw"
                }
                .to_string();
            }
            "urlencoded" => {
                if let Some(list) = b.get("urlencoded").and_then(|x| x.as_array()) {
                    for kv in list {
                        let key = v_str(kv, "key");
                        if !key.is_empty() {
                            body_urlencoded.push(KeyValueItem {
                                key,
                                value: v_str(kv, "value"),
                                enabled: kv_enabled(kv),
                                description: v_str(kv, "description"),
                                from_template: false,
                            });
                        }
                    }
                }
                body_type = "form".to_string();
            }
            "formdata" => {
                if let Some(list) = b.get("formdata").and_then(|x| x.as_array()) {
                    for kv in list {
                        let key = v_str(kv, "key");
                        if key.is_empty() {
                            continue;
                        }
                        let kind = v_str(kv, "type");
                        body_form.push(FormDataItem {
                            key,
                            value: v_str(kv, "value"),
                            enabled: kv_enabled(kv),
                            kind: if kind == "file" {
                                "file".to_string()
                            } else {
                                "text".to_string()
                            },
                            file_path: v_str(kv, "src"),
                            description: v_str(kv, "description"),
                            from_template: false,
                        });
                    }
                }
                body_type = "formdata".to_string();
            }
            "graphql" => {
                graphql_query = v_str(b, "query");
                graphql_variables = v_str(b, "variables");
                body_type = "graphql".to_string();
            }
            "file" => {
                body = v_str(b, "src");
                body_type = "binary".to_string();
            }
            _ => {}
        }
    }
    if url.is_empty() {
        return;
    }
    // 文档：把 Postman 的 description 与 response 示例写入 docs_md
    let mut docs = v_str(request, "description");
    if docs.is_empty() {
        docs = v_str(item, "description");
    }
    if let Some(resp) = item
        .get("response")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
    {
        let resp_name = v_str(resp, "name");
        let resp_code = resp.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
        let resp_body = v_str(resp, "body");
        let mut md = docs;
        if !resp_body.trim().is_empty() {
            md.push_str(&format!(
                "\n\n## 响应示例（{} · HTTP {}）\n\n```json\n{}\n```\n",
                resp_name, resp_code, resp_body
            ));
        }
        docs = md;
    }
    out.push(EndpointDraft {
        module: parent_module.to_string(),
        name: if name.is_empty() {
            format!("{} {}", method, url)
        } else {
            name
        },
        method,
        url,
        headers,
        query_params,
        body,
        body_type,
        body_form,
        body_urlencoded,
        body_graphql_query: graphql_query,
        body_graphql_variables: graphql_variables,
        authorization,
        settings: RequestSettings::default(),
        docs_md: docs,
    });
}

/// 解析 Postman 的 request.auth（v2.1）。
pub(super) fn parse_postman_auth(auth: Option<&Value>) -> Authorization {
    let mut a = Authorization::default();
    let Some(auth) = auth else { return a };
    let t = v_str(auth, "type");
    a.r#type = match t.as_str() {
        "basic" => "basic",
        "bearer" => "bearer",
        "jwt" => "jwt",
        "apikey" => "apiKey",
        _ => "none",
    }
    .to_string();
    let get_val = |name: &str| -> String {
        auth.get(name)
            .and_then(|o| o.as_array())
            .and_then(|arr| arr.iter().find(|x| v_str(x, "key") == name))
            .map(|x| v_str(x, "value"))
            .unwrap_or_default()
    };
    match a.r#type.as_str() {
        "basic" => {
            a.username = get_val("username");
            a.password = get_val("password");
        }
        "bearer" => a.token = get_val("token"),
        "jwt" => a.jwt_token = get_val("token"),
        "apiKey" => {
            a.api_key_name = get_val("key");
            a.api_key_value = get_val("value");
            let in_loc = get_val("in");
            a.api_key_in = if in_loc == "query" { "query" } else { "header" }.to_string();
        }
        _ => {}
    }
    a
}

pub fn parse_postman_collection(json: &str) -> Result<PostmanImport, String> {
    let root: Value =
        serde_json::from_str(json).map_err(|e| format!("Postman JSON 解析失败: {}", e))?;
    let mut variables: Vec<KeyValueItem> = Vec::new();
    if let Some(vars) = root.get("variable").and_then(|x| x.as_array()) {
        for v in vars {
            let key = v_str(v, "key");
            if !key.is_empty() {
                variables.push(KeyValueItem {
                    key,
                    value: v_str(v, "value"),
                    enabled: true,
                    description: v_str(v, "description"),
                    from_template: false,
                });
            }
        }
    }
    let mut out = Vec::new();
    if let Some(items) = root.get("item").and_then(|x| x.as_array()) {
        for item in items {
            parse_postman_item(item, &mut out, "");
        }
    } else if let Some(items) = root.get("items").and_then(|x| x.as_array()) {
        // v2.0 兼容
        for item in items {
            parse_postman_item(item, &mut out, "");
        }
    }
    if out.is_empty() {
        return Err("未在 Postman 数据中找到接口（请确认是 Collection v2.1 格式）".to_string());
    }
    Ok(PostmanImport {
        variables,
        drafts: out,
    })
}

// ─── Postman 导出 ───

pub fn export_postman_collection(
    project_name: &str,
    modules: &[(String, String)], // (module_id, name)
    endpoints: &[crate::commands::api::models::ApiEndpoint],
    variables: &[KeyValueItem],
) -> Result<String, String> {
    let mut items: Vec<Value> = Vec::new();
    for (module_id, module_name) in modules {
        let eps: Vec<&crate::commands::api::models::ApiEndpoint> = endpoints
            .iter()
            .filter(|e| e.module_id.as_deref() == Some(module_id.as_str()))
            .collect();
        if eps.is_empty() {
            continue;
        }
        let item: Vec<Value> = eps.iter().map(|e| endpoint_to_postman_item(e)).collect();
        items.push(serde_json::json!({
            "name": module_name,
            "item": item,
        }));
    }
    // 未分组的接口
    let loose: Vec<&crate::commands::api::models::ApiEndpoint> =
        endpoints.iter().filter(|e| e.module_id.is_none()).collect();
    for e in loose {
        items.push(endpoint_to_postman_item(e));
    }
    let variable: Vec<Value> = variables
        .iter()
        .map(|v| serde_json::json!({ "key": v.key, "value": v.value }))
        .collect();
    let collection = serde_json::json!({
        "info": {
            "name": project_name,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
        },
        "variable": variable,
        "item": items,
    });
    serde_json::to_string_pretty(&collection).map_err(|e| e.to_string())
}

pub(super) fn endpoint_to_postman_item(e: &crate::commands::api::models::ApiEndpoint) -> Value {
    let header: Vec<Value> = e
        .headers
        .iter()
        .filter(|h| h.enabled)
        .map(|h| {
            let mut j = serde_json::json!({ "key": h.key, "value": h.value });
            if !h.description.is_empty() {
                j["description"] = Value::String(h.description.clone());
            }
            j
        })
        .collect();
    let query: Vec<Value> = e
        .query_params
        .iter()
        .filter(|q| q.enabled)
        .map(|q| serde_json::json!({ "key": q.key, "value": q.value }))
        .collect();
    let url = serde_json::json!({
        "raw": e.url,
        "host": [e.url.split('/').next().unwrap_or("")],
        "query": query,
    });
    let mut request = serde_json::json!({
        "method": e.method,
        "header": header,
        "url": url,
    });
    // 认证
    let auth = &e.authorization;
    if auth.r#type != "none" {
        let auth_json = match auth.r#type.as_str() {
            "basic" => serde_json::json!({
                "type": "basic",
                "basic": [
                    { "key": "username", "value": auth.username, "type": "string" },
                    { "key": "password", "value": auth.password, "type": "string" }
                ]
            }),
            "bearer" => serde_json::json!({
                "type": "bearer",
                "bearer": [ { "key": "token", "value": auth.token, "type": "string" } ]
            }),
            "jwt" => serde_json::json!({
                "type": "jwt",
                "jwt": [ { "key": "token", "value": auth.jwt_token, "type": "string" } ]
            }),
            "apiKey" => serde_json::json!({
                "type": "apikey",
                "apikey": [
                    { "key": "key", "value": auth.api_key_name, "type": "string" },
                    { "key": "value", "value": auth.api_key_value, "type": "string" },
                    { "key": "in", "value": auth.api_key_in, "type": "string" }
                ]
            }),
            _ => serde_json::Value::Null,
        };
        if !auth_json.is_null() {
            request["auth"] = auth_json;
        }
    }
    // Cookie
    let cookies: Vec<Value> = e
        .cookies
        .iter()
        .filter(|c| c.enabled)
        .map(|c| serde_json::json!({ "key": c.key, "value": c.value }))
        .collect();
    if !cookies.is_empty() {
        request["cookie"] = Value::Array(cookies);
    }
    match e.body_type.as_str() {
        "json" => {
            request["body"] = serde_json::json!({ "mode": "raw", "raw": e.body, "options": { "raw": { "language": "json" } } });
        }
        "form" => {
            let urlencoded: Vec<Value> = e
                .body_urlencoded
                .iter()
                .filter(|kv| kv.enabled)
                .map(|kv| serde_json::json!({ "key": kv.key, "value": kv.value }))
                .collect();
            request["body"] = serde_json::json!({ "mode": "urlencoded", "urlencoded": urlencoded });
        }
        "formdata" => {
            let formdata: Vec<Value> = e
                .body_form
                .iter()
                .filter(|f| f.enabled)
                .map(|f| {
                    if f.kind == "file" {
                        serde_json::json!({ "key": f.key, "src": f.file_path, "type": "file" })
                    } else {
                        serde_json::json!({ "key": f.key, "value": f.value, "type": "text" })
                    }
                })
                .collect();
            request["body"] = serde_json::json!({ "mode": "formdata", "formdata": formdata });
        }
        "graphql" => {
            request["body"] = serde_json::json!({
                "mode": "graphql",
                "graphql": {
                    "query": e.body_graphql_query,
                    "variables": e.body_graphql_variables,
                }
            });
        }
        "binary" => {
            request["body"] = serde_json::json!({ "mode": "file", "file": { "src": e.body } });
        }
        "raw" => {
            request["body"] = serde_json::json!({ "mode": "raw", "raw": e.body });
        }
        _ => {}
    }
    let mut item = serde_json::json!({
        "name": e.name,
        "request": request,
    });
    if !e.docs_md.is_empty() {
        // Postman 用 description 承载文档
        item["request"]["description"] = Value::String(e.docs_md.clone());
    }
    item
}
