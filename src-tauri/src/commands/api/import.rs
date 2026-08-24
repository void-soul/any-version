//! 导入导出：Postman 标准数据、Swagger/OpenAPI、Nest/Nuxt/Spring 框架接口扫描。

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::models::{Authorization, FormDataItem, KeyValueItem};

/// 导入候选（由各解析器产出，统一落库）。
#[derive(Debug, Clone, Default)]
pub struct EndpointDraft {
    /// 模块名（自动创建或匹配已有）
    pub module: String,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<KeyValueItem>,
    pub query_params: Vec<KeyValueItem>,
    pub body: String,
    pub body_type: String,
    pub body_form: Vec<FormDataItem>,
    pub body_urlencoded: Vec<KeyValueItem>,
    pub body_graphql_query: String,
    pub body_graphql_variables: String,
    pub authorization: Authorization,
    pub docs_md: String,
}

/// Postman 导入结果。
#[derive(Debug, Clone)]
pub struct PostmanImport {
    /// 集合级变量（导入为环境变量）
    pub variables: Vec<KeyValueItem>,
    pub drafts: Vec<EndpointDraft>,
}

// ─── 通用工具 ───

fn v_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// 从装饰器后文本提取函数名：取第一个 `(` 前最近的标识符。
fn extract_func_name(after: &str) -> String {
    let Some(paren_idx) = after.find('(') else {
        return String::new();
    };
    let before = &after[..paren_idx];
    let mut name = String::new();
    for ch in before.chars().rev() {
        if ch.is_alphanumeric() || ch == '_' {
            name.insert(0, ch);
        } else if !name.is_empty() {
            break;
        }
    }
    name
}

/// 确保路径以 `/` 开头（Nest/Spring 扫描产物为相对路径时补上）。
fn ensure_leading_slash(url: &str) -> String {
    if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{}", url)
    }
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        if base.is_empty() {
            "/".to_string()
        } else {
            base.to_string()
        }
    } else if base.is_empty() {
        format!("/{}", path)
    } else {
        format!("{}/{}", base, path)
    }
}

/// 目录遍历（跳过常见依赖/构建目录，限深限文件数）。
fn walk_files(root: &Path, depth: usize, max_depth: usize, max_files: &mut usize, out: &mut Vec<PathBuf>) {
    if depth > max_depth || *max_files == 0 {
        return;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *max_files == 0 {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if matches!(
                name.as_str(),
                "node_modules" | ".git" | "dist" | "build" | "target" | ".nuxt" | ".output" | "coverage" | "vendor" | ".next" | ".cache" | ".idea"
            ) {
                continue;
            }
            walk_files(&path, depth + 1, max_depth, max_files, out);
        } else {
            *max_files -= 1;
            out.push(path);
        }
    }
}

fn kv_enabled(v: &Value) -> bool {
    v.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false) == false
}

// ─── Postman 导入（v2.1 / v2.0） ───

fn parse_postman_item(item: &Value, out: &mut Vec<EndpointDraft>, parent_module: &str) {
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
        url = v_str(u, "raw");
        if url.is_empty() {
            // 由 protocol/host/path 拼装
            let protocol = v_str(u, "protocol");
            let port = v_str(u, "port");
            let host = u
                .get("host")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|p| p.as_str()).collect::<Vec<_>>().join("."))
                .unwrap_or_default();
            let path = u
                .get("path")
                .and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|p| p.as_str()).collect::<Vec<_>>().join("/"))
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
                let header_says_json = headers
                    .iter()
                    .any(|h| h.key.eq_ignore_ascii_case("content-type") && h.value.contains("json"));
                let looks_json = body.trim_start().starts_with('{') || body.trim_start().starts_with('[');
                body_type = if header_says_json || (looks_json && serde_json::from_str::<Value>(&body).is_ok()) {
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
                            kind: if kind == "file" { "file".to_string() } else { "text".to_string() },
                            file_path: v_str(kv, "src"),
                            description: v_str(kv, "description"),
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
    if let Some(resp) = item.get("response").and_then(|x| x.as_array()).and_then(|a| a.first()) {
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
        name: if name.is_empty() { format!("{} {}", method, url) } else { name },
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
        docs_md: docs,
    });
}

/// 解析 Postman 的 request.auth（v2.1）。
fn parse_postman_auth(auth: Option<&Value>) -> Authorization {
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
    let root: Value = serde_json::from_str(json).map_err(|e| format!("Postman JSON 解析失败: {}", e))?;
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
    Ok(PostmanImport { variables, drafts: out })
}

// ─── Postman 导出 ───

pub fn export_postman_collection(
    project_name: &str,
    modules: &[(String, String)], // (module_id, name)
    endpoints: &[super::models::ApiEndpoint],
    variables: &[KeyValueItem],
) -> Result<String, String> {
    let mut items: Vec<Value> = Vec::new();
    for (module_id, module_name) in modules {
        let eps: Vec<&super::models::ApiEndpoint> = endpoints
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
    let loose: Vec<&super::models::ApiEndpoint> = endpoints.iter().filter(|e| e.module_id.is_none()).collect();
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

fn endpoint_to_postman_item(e: &super::models::ApiEndpoint) -> Value {
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

// ─── Swagger / OpenAPI 导入 ───

pub async fn fetch_swagger_source(source: &str) -> Result<String, String> {
    if source.starts_with("http://") || source.starts_with("https://") {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(source)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("请求 Swagger 失败: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("请求 Swagger 失败: HTTP {}", resp.status()));
        }
        resp.text().await.map_err(|e| format!("读取 Swagger 失败: {}", e))
    } else {
        std::fs::read_to_string(source).map_err(|e| format!("读取 Swagger 文件失败: {}", e))
    }
}

pub fn parse_swagger(json: &str) -> Result<(String, String, Vec<EndpointDraft>), String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| format!("Swagger JSON 解析失败: {}", e))?;
    let base_url = swagger_base_url(&doc);
    let doc_title = doc
        .get("info")
        .map(|i| v_str(i, "title"))
        .unwrap_or_default();
    let default_module = if doc_title.is_empty() { "Swagger".to_string() } else { doc_title };

    let paths = doc.get("paths").and_then(|p| p.as_object()).ok_or("Swagger 缺少 paths 字段")?;
    let mut drafts: Vec<EndpointDraft> = Vec::new();
    for (path, methods) in paths {
        let Some(methods) = methods.as_object() else { continue };
        for (http_method, op) in methods {
            let http_method = http_method.to_uppercase();
            if !matches!(http_method.as_str(), "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "OPTIONS" | "HEAD") {
                continue;
            }
            let Some(op) = op.as_object() else { continue };
            let summary = op.get("summary").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let operation_id = op.get("operationId").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let name = if !summary.is_empty() { summary } else if !operation_id.is_empty() { operation_id } else { format!("{} {}", http_method, path) };

            let module = op
                .get("tags")
                .and_then(|t| t.as_array())
                .and_then(|t| t.first())
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| default_module.clone());

            let mut query_params: Vec<KeyValueItem> = Vec::new();
            let mut headers: Vec<KeyValueItem> = Vec::new();
            if let Some(params) = op.get("parameters").and_then(|p| p.as_array()) {
                for p in params {
                    let name = v_str(p, "name");
                    if name.is_empty() {
                        continue;
                    }
                    let desc = v_str(p, "description");
                    let example = p.get("schema").and_then(|s| schema_example(s));
                    let value = example.map(|v| v.to_string()).unwrap_or_default();
                    match v_str(p, "in").as_str() {
                        "query" => query_params.push(KeyValueItem { key: name, value, enabled: true, description: desc }),
                        "header" => headers.push(KeyValueItem { key: name, value, enabled: true, description: desc }),
                        _ => {}
                    }
                }
            }

            let mut body = String::new();
            let mut body_type = "none".to_string();
            if let Some(rb) = op.get("requestBody").and_then(|x| x.as_object()) {
                if let Some(content) = rb.get("content").and_then(|c| c.as_object()) {
                    if let Some(json_c) = content.get("application/json").and_then(|j| j.as_object()) {
                        if let Some(example) = json_c.get("example") {
                            body = example.to_string();
                            body_type = "json".to_string();
                        } else if let Some(schema) = json_c.get("schema") {
                            body = schema_example(schema).map(|v| v.to_string()).unwrap_or_default();
                            body_type = "json".to_string();
                        }
                    }
                }
            }
            // Swagger 2.0：body 参数
            if body.is_empty() {
                if let Some(params) = op.get("parameters").and_then(|p| p.as_array()) {
                    for p in params {
                        if v_str(p, "in") == "body" {
                            if let Some(schema) = p.get("schema") {
                                body = schema_example(schema).map(|v| v.to_string()).unwrap_or_default();
                                body_type = "json".to_string();
                            }
                        }
                    }
                }
            }

            let url = join_url(&base_url, path);
            let docs = op.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string();
            drafts.push(EndpointDraft {
                module,
                name,
                method: http_method,
                url,
                headers,
                query_params,
                body,
                body_type,
                docs_md: docs,
                ..Default::default()
            });
        }
    }
    if drafts.is_empty() {
        return Err("Swagger 文档中未解析到任何接口".to_string());
    }
    Ok((default_module, base_url, drafts))
}

fn swagger_base_url(doc: &Value) -> String {
    // OpenAPI 3.x
    if let Some(servers) = doc.get("servers").and_then(|s| s.as_array()) {
        if let Some(first) = servers.first() {
            if let Some(url) = first.get("url").and_then(|u| u.as_str()) {
                return url.to_string();
            }
        }
    }
    // Swagger 2.0：schemes + host + basePath
    let host = v_str(doc, "host");
    let base_path = v_str(doc, "basePath");
    let scheme = doc
        .get("schemes")
        .and_then(|s| s.as_array())
        .and_then(|s| s.first())
        .and_then(|s| s.as_str())
        .unwrap_or("http");
    if !host.is_empty() {
        format!("{}://{}{}", scheme, host, base_path)
    } else {
        base_path
    }
}

/// 从 JSON Schema 生成示例值。
fn schema_example(schema: &Value) -> Option<Value> {
    if let Some(ex) = schema.get("example") {
        return Some(ex.clone());
    }
    if let Some(def) = schema.get("default") {
        return Some(def.clone());
    }
    match schema.get("type").and_then(|t| t.as_str()).unwrap_or("object") {
        "object" => {
            let mut obj = serde_json::Map::new();
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                for (k, v) in props {
                    if let Some(example) = schema_example(v) {
                        obj.insert(k.clone(), example);
                    }
                }
            }
            Some(Value::Object(obj))
        }
        "array" => {
            let item = schema.get("items").and_then(|i| schema_example(i));
            let arr = item.map(|i| vec![i]).unwrap_or_default();
            Some(Value::Array(arr))
        }
        "string" => Some(Value::String("string".to_string())),
        "integer" | "number" => Some(Value::Number(0.into())),
        "boolean" => Some(Value::Bool(false)),
        _ => Some(Value::Null),
    }
}

// ─── 框架扫描：Nest / Nuxt / Spring ───

pub fn scan_framework(dir: &str, framework: &str) -> Result<Vec<EndpointDraft>, String> {
    let root = Path::new(dir);
    if !root.is_dir() {
        return Err("目录不存在或不可读".to_string());
    }
    let mut files = Vec::new();
    let mut max_files = 8000usize;
    walk_files(root, 0, 14, &mut max_files, &mut files);
    if files.is_empty() {
        return Err("目录中没有找到可扫描的文件".to_string());
    }
    let mut drafts: Vec<EndpointDraft> = Vec::new();
    match framework {
        "nest" => {
            for f in files.iter().filter(|f| f.extension().map(|e| e == "ts").unwrap_or(false)) {
                if f.file_name().map(|n| n.to_string_lossy().contains("controller")).unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(f) {
                        drafts.extend(scan_nest_controller(&content, root, f));
                    }
                }
            }
        }
        "nuxt" => {
            for f in files.iter().filter(|f| f.extension().map(|e| e == "ts").unwrap_or(false)) {
                if let Ok(content) = std::fs::read_to_string(f) {
                    drafts.extend(scan_nuxt_api(&content, root, f));
                }
            }
        }
        "spring" => {
            for f in files.iter().filter(|f| f.extension().map(|e| e == "java").unwrap_or(false)) {
                if f.file_name().map(|n| n.to_string_lossy().contains("Controller")).unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(f) {
                        drafts.extend(scan_spring_controller(&content));
                    }
                }
            }
        }
        other => return Err(format!("未知框架类型: {}（支持 nest / nuxt / spring）", other)),
    }
    if drafts.is_empty() {
        return Err(format!("在 {} 中未解析到接口定义", framework));
    }
    // 去重（method + url）
    let mut seen = std::collections::HashSet::new();
    drafts.retain(|d| seen.insert(format!("{} {}", d.method, d.url)));
    Ok(drafts)
}

fn rel_path(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

fn extract_decorator<'a>(content: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("@{}(", name);
    if let Some(idx) = content.find(&marker) {
        let after = &content[idx + marker.len()..];
        let trimmed = after.trim_start();
        if trimmed.starts_with('\'') || trimmed.starts_with('"') {
            let quote = trimmed.chars().next().unwrap();
            let rest = &trimmed[1..];
            if let Some(end) = rest.find(quote) {
                let val = &rest[..end];
                if !val.contains('(') {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// NestJS Controller 扫描。
fn scan_nest_controller(content: &str, root: &Path, file: &Path) -> Vec<EndpointDraft> {
    let mut drafts = Vec::new();
    let prefix = extract_decorator(content, "Controller").unwrap_or("");
    let module = extract_decorator(content, "ApiTags")
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            file.file_name()
                .map(|n| n.to_string_lossy().replace(".controller.ts", ""))
                .unwrap_or_default()
        });
    for method in ["Get", "Post", "Put", "Delete", "Patch"] {
        let marker = format!("@{}(", method);
        let mut search_from = 0;
        while let Some(idx) = content[search_from..].find(&marker) {
            let abs = search_from + idx;
            let after = &content[abs + marker.len()..];
            let path = after.trim_start();
            let path_val = if path.starts_with('\'') || path.starts_with('"') {
                let quote = path.chars().next().unwrap();
                if let Some(end) = path[1..].find(quote) {
                    path[1..1 + end].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let url = ensure_leading_slash(&join_url(prefix, &path_val));
            let after_decorator = &content[abs + marker.len()..];
            let func_name = extract_func_name(after_decorator);
            drafts.push(EndpointDraft {
                module: module.to_string(),
                name: if func_name.is_empty() { format!("{} {}", method.to_uppercase(), url) } else { func_name },
                method: method.to_uppercase(),
                url,
                docs_md: format!("来源: {}", rel_path(root, file)),
                ..Default::default()
            });
            search_from = abs + marker.len() + 1;
        }
    }
    drafts
}

/// Nuxt3 server/api 扫描：路由从文件相对路径推导，method 从后缀推导。
fn scan_nuxt_api(content: &str, root: &Path, file: &Path) -> Vec<EndpointDraft> {
    let _ = content;
    let rel = rel_path(root, file);
    let mut path = rel.trim_start_matches("server/").to_string();
    if path.starts_with("api/") {
        path = format!("/{}", path);
    } else if path.starts_with("routes/") {
        path = format!("/{}", path);
    } else {
        return Vec::new();
    }
    let mut method = "GET".to_string();
    for (suffix, m) in [(".get", "GET"), (".post", "POST"), (".put", "PUT"), (".delete", "DELETE"), (".patch", "PATCH")] {
        if path.ends_with(&format!("{}.ts", suffix)) {
            path = path.trim_end_matches(&format!("{}.ts", suffix)).to_string();
            method = m.to_string();
            break;
        }
    }
    if path.ends_with(".ts") {
        path = path.trim_end_matches(".ts").to_string();
    }
    if path.ends_with("/index") {
        path = path.trim_end_matches("/index").to_string();
    }
    let name = path
        .trim_start_matches('/')
        .split('/')
        .last()
        .unwrap_or("api")
        .to_string();
    Vec::from([EndpointDraft {
        module: "API".to_string(),
        name: format!("{} {}", method, name),
        method,
        url: path,
        docs_md: format!("来源: {}", rel),
        ..Default::default()
    }])
}

/// Spring Controller 扫描。
fn scan_spring_controller(content: &str) -> Vec<EndpointDraft> {
    let mut drafts = Vec::new();
    let class_prefix = extract_decorator(content, "RequestMapping").unwrap_or("");
    let module = content
        .lines()
        .find(|l| l.contains("class") && l.contains("Controller"))
        .map(|l| {
            l.split("class")
                .nth(1)
                .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    for method in ["GetMapping", "PostMapping", "PutMapping", "DeleteMapping", "PatchMapping", "RequestMapping"] {
        let marker = format!("@{}(", method);
        let mut search_from = 0;
        while let Some(idx) = content[search_from..].find(&marker) {
            let abs = search_from + idx;
            let after = &content[abs + marker.len()..];
            if method == "RequestMapping" {
                let head = after.to_uppercase();
                let head = &head[..head.len().min(200)];
                if !head.contains("METHOD") {
                    search_from = abs + marker.len() + 1;
                    continue;
                }
            }
            let trimmed = after.trim_start();
            let path_val = if trimmed.starts_with('\'') || trimmed.starts_with('"') {
                let quote = trimmed.chars().next().unwrap();
                if let Some(end) = trimmed[1..].find(quote) {
                    trimmed[1..1 + end].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let mut effective_method = method.replace("Mapping", "").to_uppercase();
            if effective_method == "REQUEST" {
                effective_method = "GET".to_string();
                let upper = content[abs..].to_uppercase();
                for m in ["GET", "POST", "PUT", "DELETE", "PATCH"] {
                    if upper[..upper.len().min(120)].contains(&format!("REQUESTMETHOD.{}", m)) {
                        effective_method = m.to_string();
                        break;
                    }
                }
            }
            if effective_method == "REQUEST" {
                effective_method = "GET".to_string();
            }
            let url = ensure_leading_slash(&join_url(class_prefix, &path_val));
            let func_name = extract_func_name(after);
            drafts.push(EndpointDraft {
                module: module.clone(),
                name: if func_name.is_empty() { format!("{} {}", effective_method, url) } else { func_name },
                method: effective_method,
                url,
                ..Default::default()
            });
            search_from = abs + marker.len() + 1;
        }
    }
    drafts
}

// ─── 单元测试 ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_postman_minimal() {
        let json = r#"{
            "info": { "name": "Demo" },
            "item": [
                { "name": "用户", "item": [
                    { "name": "获取用户", "request": {
                        "method": "GET",
                        "url": { "raw": "https://api.example.com/users/1", "query": [{ "key": "page", "value": "1" }] },
                        "header": [{ "key": "Authorization", "value": "Bearer {{token}}" }]
                    } }
                ]},
                { "name": "创建用户", "request": {
                    "method": "POST",
                    "url": { "raw": "https://api.example.com/users" },
                    "body": { "mode": "raw", "raw": "{\"name\":\"x\"}" }
                } }
            ]
        }"#;
        let imp = parse_postman_collection(json).unwrap();
        let drafts = imp.drafts;
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].module, "用户");
        assert_eq!(drafts[0].method, "GET");
        assert_eq!(drafts[0].url, "https://api.example.com/users/1");
        assert_eq!(drafts[0].headers[0].value, "Bearer {{token}}");
        assert_eq!(drafts[1].module, "");
        assert_eq!(drafts[1].body_type, "json");
    }

    #[test]
    fn parse_postman_real_collection_features() {
        // 模拟 Amazon 集合：集合变量 + formdata（含文件）+ url 由 host/path 拼装 + response 示例
        let json = r#"{
            "info": { "name": "SP-API" },
            "variable": [
                { "key": "baseUrl", "value": "http://127.0.0.1:5000" },
                { "key": "shopid", "value": "" }
            ],
            "item": [
                { "name": "Orders", "item": [
                    { "name": "getOrders", "request": {
                        "method": "POST",
                        "header": [],
                        "body": { "mode": "formdata", "formdata": [
                            { "key": "shopid", "value": "{{shopid}}", "type": "text" },
                            { "key": "file", "src": "C:\\tmp\\a.txt", "type": "file", "description": "上传文件" }
                        ] },
                        "url": { "raw": "{{baseUrl}}/ordersv0/getOrders.api", "host": ["{{baseUrl}}"], "path": ["ordersv0", "getOrders.api"] }
                    }, "response": [ { "name": "成功", "code": 200, "body": "{\"ok\":true}" } ] }
                ]}
            ]
        }"#;
        let imp = parse_postman_collection(json).unwrap();
        assert_eq!(imp.variables.len(), 2);
        assert_eq!(imp.variables[0].key, "baseUrl");
        let d = &imp.drafts[0];
        assert_eq!(d.body_type, "formdata");
        assert_eq!(d.body_form.len(), 2);
        assert_eq!(d.body_form[1].kind, "file");
        assert_eq!(d.body_form[1].file_path, "C:\\tmp\\a.txt");
        assert!(d.docs_md.contains("响应示例"));
        assert!(d.docs_md.contains("200"));
    }

    #[test]
    fn parse_swagger_v3() {
        let json = r#"{
            "openapi": "3.0.0",
            "info": { "title": "订单服务" },
            "servers": [{ "url": "https://api.example.com/v1" }],
            "paths": {
                "/orders": {
                    "get": {
                        "tags": ["订单"],
                        "summary": "订单列表",
                        "parameters": [{ "name": "status", "in": "query", "schema": { "type": "string" } }]
                    },
                    "post": {
                        "tags": ["订单"],
                        "summary": "创建订单",
                        "requestBody": { "content": { "application/json": { "schema": {
                            "type": "object",
                            "properties": { "amount": { "type": "number" } }
                        } } } }
                    }
                }
            }
        }"#;
        let (_module, base, drafts) = parse_swagger(json).unwrap();
        assert_eq!(base, "https://api.example.com/v1");
        assert_eq!(drafts.len(), 2);
        assert_eq!(drafts[0].module, "订单");
        assert_eq!(drafts[0].url, "https://api.example.com/v1/orders");
        assert_eq!(drafts[0].method, "GET");
        assert!(drafts[1].body.contains("amount"));
        assert_eq!(drafts[1].body_type, "json");
    }

    #[test]
    fn test_scan_nest_controller() {
        let content = r#"
import { Controller, Get, Post } from '@nestjs/common';
@ApiTags('用户')
@Controller('users')
export class UsersController {
  @Get()
  findAll() { return []; }
  @Get(':id')
  findOne(@Param('id') id: string) { return {}; }
  @Post()
  create(@Body() body: any) { return {}; }
}
"#;
        let drafts = scan_nest_controller(content, Path::new("/src"), Path::new("/src/users.controller.ts"));
        assert_eq!(drafts.len(), 3);
        assert!(drafts.iter().any(|d| d.url == "/users" && d.method == "GET" && d.name == "findAll"));
        assert!(drafts.iter().any(|d| d.url == "/users/:id" && d.method == "GET"));
        assert!(drafts.iter().any(|d| d.url == "/users" && d.method == "POST"));
        assert!(drafts.iter().all(|d| d.module == "用户"));
    }

    #[test]
    fn test_scan_spring_controller() {
        let content = r#"
@RestController
@RequestMapping("/api/v1")
public class UserController {
    @GetMapping("/users")
    public List<User> list() { return null; }
    @PostMapping("/users")
    public User create(@RequestBody User u) { return null; }
}
"#;
        let drafts = scan_spring_controller(content);
        assert_eq!(drafts.len(), 2);
        assert!(drafts.iter().any(|d| d.url == "/api/v1/users" && d.method == "GET" && d.name == "list"));
        assert!(drafts.iter().any(|d| d.url == "/api/v1/users" && d.method == "POST"));
    }

    #[test]
    fn scan_nuxt_api_path() {
        let root = Path::new("/proj");
        let file = Path::new("/proj/server/api/orders/get.get.ts");
        let drafts = scan_nuxt_api("export default defineEventHandler(() => {})", root, file);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].url, "/api/orders/get");
        assert_eq!(drafts[0].method, "GET");
    }
}
