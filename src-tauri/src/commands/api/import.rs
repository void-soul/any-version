//! 导入导出：Postman 标准数据、Swagger/OpenAPI、Nest/Nuxt/Spring 框架接口扫描。

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::models::{Authorization, FormDataItem, KeyValueItem, RequestSettings};

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
    /// 请求设置（cURL 的 -k/--http1.1 等会映射到这里）
    pub settings: RequestSettings,
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

/// 剥掉 URL 里的查询串（`?` 及其后所有内容），只在 `url.query` 数组非空时用。
fn strip_query(url: &str) -> &str {
    match url.split_once('?') {
        Some((base, _)) => base,
        None => url,
    }
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
                            kind: if kind == "file" { "file".to_string() } else { "text".to_string() },
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
        settings: RequestSettings::default(),
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
                        "query" => query_params.push(KeyValueItem { key: name, value, enabled: true, description: desc, from_template: false }),
                        "header" => headers.push(KeyValueItem { key: name, value, enabled: true, description: desc, from_template: false }),
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

// ─── cURL 导入 ───

fn kv_item(key: &str, value: &str) -> KeyValueItem {
    KeyValueItem {
        key: key.to_string(),
        value: value.to_string(),
        enabled: true,
        description: String::new(),
        from_template: false,
    }
}

/// curl 参数分词：支持单/双引号、反斜杠转义、`\` 与 Windows `^` 换行续行。
fn shell_tokens(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let (mut in_single, mut in_double) = (false, false);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // 续行符：反斜杠或 ^ 后跟换行
        if !in_single && !in_double && (c == '\\' || c == '^') {
            let mut j = i + 1;
            if j < chars.len() && chars[j] == '\r' {
                j += 1;
            }
            if j < chars.len() && chars[j] == '\n' {
                i = j + 1;
                continue;
            }
        }
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '"' | '\\' | '$' | '`') {
                current.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_double = false;
            } else {
                current.push(c);
            }
            i += 1;
            continue;
        }
        match c {
            // ANSI-C quoting：$'...' / $"..."（浏览器复制 cURL 时常见）
            '$' if i + 1 < chars.len() && chars[i + 1] == '\'' => {
                in_single = true;
                has_token = true;
                i += 2;
            }
            '$' if i + 1 < chars.len() && chars[i + 1] == '"' => {
                in_double = true;
                has_token = true;
                i += 2;
            }
            '\'' => {
                in_single = true;
                has_token = true;
                i += 1;
            }
            '"' => {
                in_double = true;
                has_token = true;
                i += 1;
            }
            '\\' if i + 1 < chars.len() => {
                current.push(chars[i + 1]);
                has_token = true;
                i += 2;
            }
            c if c.is_whitespace() => {
                if has_token {
                    tokens.push(current.clone());
                    current.clear();
                    has_token = false;
                }
                i += 1;
            }
            _ => {
                current.push(c);
                has_token = true;
                i += 1;
            }
        }
    }
    if has_token {
        tokens.push(current);
    }
    tokens
}

fn next_arg(tokens: &[String], i: &mut usize) -> Option<String> {
    if *i + 1 < tokens.len() {
        *i += 1;
        Some(tokens[*i].clone())
    } else {
        None
    }
}

/// 解析 `k=v&k2=v2` 形式的键值对。
fn parse_pairs(raw: &str) -> Vec<KeyValueItem> {
    raw.split('&')
        .filter(|s| !s.trim().is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            kv_item(k.trim(), v)
        })
        .filter(|kv| !kv.key.is_empty())
        .collect()
}

/// 拆分 URL 中的查询串（curl 的 URL 常常自带 query）。
fn split_url_query(url: &str) -> (String, Vec<KeyValueItem>) {
    match url.split_once('?') {
        Some((base, q)) => (base.to_string(), parse_pairs(q)),
        None => (url.to_string(), Vec::new()),
    }
}

/// 解析 Cookie 字符串（`a=1; b=2`）或 `@文件` 形式。
fn parse_cookie_string(raw: &str) -> Vec<KeyValueItem> {
    let content = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path.trim()).unwrap_or_default()
    } else {
        raw.to_string()
    };
    content
        .split(|c| c == ';' || c == '\n')
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            let k = k.trim();
            if k.is_empty() {
                None
            } else {
                Some(kv_item(k, v.trim()))
            }
        })
        .collect()
}

/// `-d @file` / `--data-binary @file`：读取文件内容（读不到时按字面值处理）。
fn data_value(raw: &str) -> String {
    if let Some(path) = raw.strip_prefix('@') {
        if let Ok(text) = std::fs::read_to_string(path.trim()) {
            return text;
        }
    }
    raw.to_string()
}

/// 解析单条 cURL 命令为接口草稿（常用参数：-X/-H/-d/-F/-u/-b/-k/-L/--url/--http1.1/--http2）。
pub fn parse_curl_command(input: &str) -> Result<EndpointDraft, String> {
    /// 未单独处理、但会吃一个值的参数：跳过其值，避免把值误当作 URL。
    const FLAGS_WITH_VALUE: &[&str] = &[
        "-m", "--max-time", "--connect-timeout", "-o", "--output", "--retry",
        "--retry-delay", "--cacert", "--cert", "--key", "-x", "--proxy",
        "--proxy-user", "--resolve", "--interface", "--limit-rate", "-w", "--write-out",
        "--keepalive-time", "--expect100-timeout", "--dns-servers",
    ];
    let command = input
        .trim()
        .trim_start_matches(|c: char| c == '$' || c == '>' || c.is_whitespace());
    if command.is_empty() {
        return Err("请输入 curl 命令".to_string());
    }
    if !command.to_ascii_lowercase().starts_with("curl") {
        return Err("只支持 cURL 命令（需以 curl 开头）".to_string());
    }
    let tokens = shell_tokens(command);
    if tokens.is_empty() {
        return Err("cURL 命令为空".to_string());
    }

    let mut url = String::new();
    let mut method: Option<String> = None;
    let mut headers: Vec<KeyValueItem> = Vec::new();
    let mut cookies: Vec<KeyValueItem> = Vec::new();
    let mut data: Vec<String> = Vec::new();
    let mut urlencoded: Vec<KeyValueItem> = Vec::new();
    let mut form: Vec<FormDataItem> = Vec::new();
    let mut auth = Authorization::default();
    let mut verify_ssl = true;
    let mut http_version = "auto".to_string();
    let mut follow_redirects = true;
    let mut data_as_query = false;
    let mut json_hint = false;
    let mut upload_file: Option<String> = None;

    let mut i = 1;
    while i < tokens.len() {
        let t = tokens[i].clone();
        match t.as_str() {
            "-X" | "--request" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    method = Some(v.to_uppercase());
                }
            }
            "-H" | "--header" => {
                if let Some(raw) = next_arg(&tokens, &mut i) {
                    if let Some((k, v)) = raw.split_once(':') {
                        let key = k.trim();
                        let value = v.trim();
                        if !key.is_empty() {
                            if key.eq_ignore_ascii_case("cookie") {
                                cookies.extend(parse_cookie_string(value));
                            } else {
                                headers.push(kv_item(key, value));
                            }
                        }
                    }
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-ascii" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    data.push(data_value(&v));
                }
            }
            "--json" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    data.push(data_value(&v));
                    json_hint = true;
                }
            }
            "--data-urlencode" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    if let Some((k, val)) = v.split_once('=') {
                        urlencoded.push(kv_item(k.trim(), val));
                    } else if let Some((k, file)) = v.split_once('@') {
                        let content = std::fs::read_to_string(file.trim()).unwrap_or_default();
                        urlencoded.push(kv_item(k.trim(), &content));
                    } else {
                        urlencoded.push(kv_item(v.trim(), ""));
                    }
                }
            }
            "-F" | "--form" | "--form-string" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    if let Some((k, val)) = v.split_once('=') {
                        let key = k.trim().to_string();
                        if key.is_empty() {
                            // 忽略空字段名
                        } else if let Some(path) = val.strip_prefix('@').or_else(|| val.strip_prefix('<')) {
                            form.push(FormDataItem {
                                key,
                                value: String::new(),
                                enabled: true,
                                kind: "file".to_string(),
                                file_path: path.trim().to_string(),
                                description: String::new(),
                                from_template: false,
                            });
                        } else {
                            form.push(FormDataItem {
                                key,
                                value: val.to_string(),
                                enabled: true,
                                kind: "text".to_string(),
                                file_path: String::new(),
                                description: String::new(),
                                from_template: false,
                            });
                        }
                    }
                }
            }
            "-u" | "--user" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    let (user, pass) = v.split_once(':').unwrap_or((v.as_str(), ""));
                    auth = Authorization {
                        r#type: "basic".to_string(),
                        username: user.to_string(),
                        password: pass.to_string(),
                        ..Default::default()
                    };
                }
            }
            "-b" | "--cookie" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    cookies.extend(parse_cookie_string(&v));
                }
            }
            "-A" | "--user-agent" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    headers.push(kv_item("User-Agent", &v));
                }
            }
            "-e" | "--referer" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    headers.push(kv_item("Referer", &v));
                }
            }
            "-T" | "--upload-file" => {
                upload_file = next_arg(&tokens, &mut i);
            }
            "--url" => {
                if let Some(v) = next_arg(&tokens, &mut i) {
                    url = v;
                }
            }
            "-G" | "--get" => data_as_query = true,
            "-k" | "--insecure" => verify_ssl = false,
            "--http1.1" => http_version = "http1".to_string(),
            "--http2" | "--http2-prior-knowledge" => http_version = "http2".to_string(),
            "-L" | "--location" => follow_redirects = true,
            // 无参数开关（忽略）
            "--compressed" | "-i" | "--include" | "-s" | "--silent" | "-v" | "--verbose"
            | "-g" | "--globoff" | "--no-progress-meter" | "--fail" | "-f" => {}
            other => {
                if other.starts_with('-') {
                    if FLAGS_WITH_VALUE.contains(&other) {
                        let _ = next_arg(&tokens, &mut i);
                    }
                    // 其它未知选项：忽略（避免把选项值误当作 URL）
                } else if url.is_empty() {
                    url = other.to_string();
                }
            }
        }
        i += 1;
    }

    if url.trim().is_empty() {
        return Err("cURL 命令中未找到 URL".to_string());
    }
    // 补全协议（curl 默认 http）
    let mut url = url.trim().to_string();
    if !url.contains("://") && !url.starts_with("{{") {
        url = format!("http://{}", url);
    }
    let (base_url, mut query_params) = split_url_query(&url);

    // -G：把 -d 数据挪到查询参数
    if data_as_query {
        query_params.extend(urlencoded.drain(..));
        for d in data.drain(..) {
            query_params.extend(parse_pairs(&d));
        }
    }

    // 认证头 → 认证配置（避免与 Authorization 头重复）
    if let Some(pos) = headers.iter().position(|h| h.key.eq_ignore_ascii_case("authorization")) {
        let value = headers[pos].value.clone();
        let lower = value.to_ascii_lowercase();
        if let Some(token) = lower.strip_prefix("bearer ") {
            let start = value.len() - token.len();
            auth = Authorization {
                r#type: "bearer".to_string(),
                token: value[start..].trim().to_string(),
                ..Default::default()
            };
            headers.remove(pos);
        } else if lower.starts_with("basic ") {
            use base64::Engine;
            let encoded = value[6..].trim();
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(encoded) {
                let pair = String::from_utf8_lossy(&decoded).to_string();
                let (user, pass) = pair.split_once(':').unwrap_or((pair.as_str(), ""));
                auth = Authorization {
                    r#type: "basic".to_string(),
                    username: user.to_string(),
                    password: pass.to_string(),
                    ..Default::default()
                };
                headers.remove(pos);
            }
        }
    }

    let content_type = headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase())
        .unwrap_or_default();

    let mut body = String::new();
    let body_form = form;
    let mut body_urlencoded = urlencoded;
    let body_type: &str = if let Some(path) = upload_file {
        body = path;
        "binary"
    } else if !body_form.is_empty() {
        "formdata"
    } else if !body_urlencoded.is_empty() {
        "form"
    } else if !data.is_empty() {
        let joined = data.join("&");
        if json_hint || content_type.contains("json") {
            body = joined;
            "json"
        } else if content_type.contains("x-www-form-urlencoded") {
            body_urlencoded = parse_pairs(&joined);
            "form"
        } else {
            body = joined;
            "raw"
        }
    } else {
        "none"
    };

    let method = method.unwrap_or_else(|| {
        if matches!(body_type, "formdata" | "form" | "json" | "raw" | "binary") {
            "POST".to_string()
        } else {
            "GET".to_string()
        }
    });

    let path = url_path_of(&base_url);
    let name = if path == "/" {
        format!("{} {}", method, base_url)
    } else {
        format!("{} {}", method, path)
    };

    Ok(EndpointDraft {
        module: String::new(),
        name,
        method,
        url: base_url,
        headers,
        query_params,
        body,
        body_type: body_type.to_string(),
        body_form,
        body_urlencoded,
        body_graphql_query: String::new(),
        body_graphql_variables: String::new(),
        authorization: auth,
        settings: RequestSettings {
            http_version,
            verify_ssl,
            follow_redirects,
            ..Default::default()
        },
        docs_md: format!("```bash\n{}\n```", command),
    })
}

// ─── OpenAPI 3.0 导出 ───

/// 从接口 URL 提取 OpenAPI path（剥离协议/域名/查询串，`{{baseUrl}}` 前缀视为服务器地址）。
fn url_path_of(url: &str) -> String {
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

fn sanitize_identifier(s: &str) -> String {
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

fn openapi_body(ep: &super::models::ApiEndpoint) -> Option<Value> {
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
            for kv in ep.body_urlencoded.iter().filter(|kv| kv.enabled && !kv.key.is_empty()) {
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
            for item in ep.body_form.iter().filter(|f| f.enabled && !f.key.is_empty()) {
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
            let variables = serde_json::from_str::<Value>(&ep.body_graphql_variables)
                .unwrap_or(Value::Null);
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
    endpoints: &[super::models::ApiEndpoint],
    env_variables: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let module_name = |id: Option<&str>| -> Option<String> {
        id.and_then(|i| modules.iter().find(|(mid, _)| mid == i).map(|(_, n)| n.clone()))
    };
    // 这些请求头由 HTTP 客户端自动生成，导出为文档参数没有意义
    const AUTO_HEADERS: &[&str] = &[
        "content-type", "content-length", "host", "user-agent",
        "accept", "accept-encoding", "connection",
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
        for q in ep.query_params.iter().filter(|x| x.enabled && !x.key.is_empty()) {
            params.push(serde_json::json!({
                "name": q.key, "in": "query", "required": false,
                "schema": { "type": "string" }, "example": q.value
            }));
        }
        for p in ep.path_params.iter().filter(|x| x.enabled && !x.key.is_empty()) {
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
                schemes.entry("bearerAuth".to_string()).or_insert_with(|| {
                    serde_json::json!({ "type": "http", "scheme": "bearer" })
                });
                op.insert("security".into(), serde_json::json!([{ "bearerAuth": [] }]));
            }
            "basic" => {
                schemes.entry("basicAuth".to_string()).or_insert_with(|| {
                    serde_json::json!({ "type": "http", "scheme": "basic" })
                });
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

    /// Postman v2.1 允许 `request.url` 直接写成字符串（`{"url": "https://..."}`）。
    /// 只认 `url.raw` 的话，这类请求的 url 恒为空 → 被后面的 `if url.is_empty() { return }`
    /// **静默丢弃**（导入后少接口，且不报错）。手写集合里这种写法很常见。
    #[test]
    fn postman_url_as_plain_string_is_imported() {
        let json = r#"{
            "info": { "name": "手写集合" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": "https://api.example.com/users?page=1"
                } },
                { "name": "详情", "request": {
                    "method": "POST",
                    "url": "https://api.example.com/users/1",
                    "body": { "mode": "raw", "raw": "{\"name\":\"x\"}" }
                } }
            ]
        }"#;
        let drafts = parse_postman_collection(json).unwrap().drafts;
        assert_eq!(drafts.len(), 2, "字符串形式的 url 不该被丢弃");
        assert_eq!(drafts[0].url, "https://api.example.com/users?page=1");
        assert_eq!(drafts[1].url, "https://api.example.com/users/1");
        assert_eq!(drafts[1].body_type, "json");
        // 查询串留在 URL 里原样送出，不进 query_params（见 parse_postman_item 的说明）
        assert!(drafts[0].query_params.is_empty());
    }

    /// Postman 导出的常规形态：`url.raw` 里带着查询串，`url.query` 数组里又列了一遍。
    /// 两边都留着的话，执行时 `exec.rs` 把 `query_params` **追加**到 URL 上
    /// （`request.query()`），实际发出去的是 `?page=1&page=1`。
    /// 既然有结构化的 query，就以它为准，把 raw 里的那一份剥掉。
    #[test]
    fn postman_raw_query_is_not_duplicated_by_the_query_array() {
        let json = r#"{
            "info": { "name": "带查询" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": {
                        "raw": "https://api.example.com/users?page=1&size=20",
                        "query": [
                            { "key": "page", "value": "1" },
                            { "key": "size", "value": "20" }
                        ]
                    }
                } }
            ]
        }"#;
        let drafts = parse_postman_collection(json).unwrap().drafts;
        assert_eq!(
            drafts[0].url, "https://api.example.com/users",
            "raw 里的查询串要剥掉，否则会和 query_params 重复: {}", drafts[0].url
        );
        assert_eq!(drafts[0].query_params.len(), 2);
        assert_eq!(drafts[0].query_params[0].key, "page");
        assert_eq!(drafts[0].query_params[0].value, "1");
    }

    /// 没有 query 数组（或它是空的）时，查询串必须**留在 URL 里** —— 剥掉就丢了。
    #[test]
    fn postman_raw_query_is_kept_when_there_is_no_query_array() {
        let json = r#"{
            "info": { "name": "只有 raw" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": { "raw": "https://api.example.com/users?page=1" }
                } },
                { "name": "空数组", "request": {
                    "method": "GET",
                    "url": { "raw": "https://api.example.com/users?page=1", "query": [] }
                } },
                { "name": "字符串形式", "request": {
                    "method": "GET",
                    "url": "https://api.example.com/users?page=1"
                } }
            ]
        }"#;
        let drafts = parse_postman_collection(json).unwrap().drafts;
        assert_eq!(drafts.len(), 3);
        for (index, draft) in drafts.iter().enumerate() {
            assert_eq!(
                draft.url, "https://api.example.com/users?page=1",
                "第 {index} 个请求的查询串不该被剥掉"
            );
            assert!(draft.query_params.is_empty());
        }
    }

    /// `disabled` 的查询参数不该发出去（与 Postman 一致）：剥掉 raw 后它由
    /// `query_params` 的 enabled 标记控制，所以必须原样保留在列表里。
    #[test]
    fn postman_disabled_query_param_is_not_sent() {
        let json = r#"{
            "info": { "name": "禁用" },
            "item": [
                { "name": "列表", "request": {
                    "method": "GET",
                    "url": {
                        "raw": "https://api.example.com/users?page=1&debug=1",
                        "query": [
                            { "key": "page", "value": "1" },
                            { "key": "debug", "value": "1", "disabled": true }
                        ]
                    }
                } }
            ]
        }"#;
        let drafts = parse_postman_collection(json).unwrap().drafts;
        assert_eq!(drafts[0].url, "https://api.example.com/users");
        assert_eq!(drafts[0].query_params.len(), 2);
        assert!(!drafts[0].query_params[1].enabled, "禁用的参数要保留标记");
    }

    /// 退化输入不能 panic：`url` 缺失 / 为 null / 空串 / 空对象时该请求被跳过，
    /// 且**不影响同一集合里其它正常接口的导入**。
    #[test]
    fn postman_missing_or_null_url_is_skipped_without_panic() {
        let json = r#"{
            "info": { "name": "空" },
            "item": [
                { "name": "无 url", "request": { "method": "GET" } },
                { "name": "null url", "request": { "method": "GET", "url": null } },
                { "name": "空串", "request": { "method": "GET", "url": "" } },
                { "name": "空对象", "request": { "method": "GET", "url": {} } },
                { "name": "有效", "request": { "method": "GET", "url": { "raw": "https://api.example.com/ok" } } }
            ]
        }"#;
        let drafts = parse_postman_collection(json).unwrap().drafts;
        assert_eq!(drafts.len(), 1, "拿不到 url 的请求应被跳过，且不影响其它接口");
        assert_eq!(drafts[0].url, "https://api.example.com/ok");
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
    fn parse_curl_json_request() {
        let cmd = r#"curl 'https://api.example.com/users?page=2&size=10' \
  -X POST \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer abc123' \
  -d '{"name":"alice"}' -k"#;
        let d = parse_curl_command(cmd).unwrap();
        assert_eq!(d.method, "POST");
        assert_eq!(d.url, "https://api.example.com/users");
        assert_eq!(d.query_params.len(), 2);
        assert_eq!(d.query_params[0].key, "page");
        assert_eq!(d.body_type, "json");
        assert_eq!(d.body, r#"{"name":"alice"}"#);
        assert_eq!(d.authorization.r#type, "bearer");
        assert_eq!(d.authorization.token, "abc123");
        assert!(!d.headers.iter().any(|h| h.key.eq_ignore_ascii_case("authorization")));
        assert!(d.docs_md.contains("curl"));
        // 当前草稿没有承载 verify_ssl，由命令层使用默认设置；这里只验证解析不丢失关键信息
        assert_eq!(d.name, "POST /users");
    }

    #[test]
    fn parse_curl_form_and_basic_auth() {
        let cmd = r#"curl -X PUT http://127.0.0.1:8080/api/upload -F "file=@C:\tmp\a.png" -F "name=test" -u user:pass -b 'sid=1; theme=dark'"#;
        let d = parse_curl_command(cmd).unwrap();
        assert_eq!(d.method, "PUT");
        assert_eq!(d.url, "http://127.0.0.1:8080/api/upload");
        assert_eq!(d.body_type, "formdata");
        assert_eq!(d.body_form.len(), 2);
        assert_eq!(d.body_form[0].kind, "file");
        assert_eq!(d.body_form[0].file_path, r"C:\tmp\a.png");
        assert_eq!(d.authorization.r#type, "basic");
        assert_eq!(d.authorization.username, "user");
        assert_eq!(d.query_params.len(), 0);
        assert_eq!(d.body_form[1].value, "test");
    }

    #[test]
    fn parse_curl_skips_options_with_values() {
        // 未知但吃值的选项（--max-time 10）不能把 10 当成 URL
        let cmd = r#"curl --max-time 10 -m 5 --compressed 'https://api.example.com/ping?x=1'"#;
        let d = parse_curl_command(cmd).unwrap();
        assert_eq!(d.url, "https://api.example.com/ping");
        assert_eq!(d.method, "GET");
        assert_eq!(d.query_params[0].value, "1");
    }

    #[test]
    fn export_openapi_basic_shape() {
        let ep = super::super::models::ApiEndpoint {
            id: "ep1".into(),
            project_id: "p1".into(),
            module_id: Some("m1".into()),
            name: "获取用户".into(),
            method: "GET".into(),
            url: "{{baseUrl}}/users/{id}?verbose=1".into(),
            headers: vec![],
            query_params: vec![
                KeyValueItem { key: "page".into(), value: "1".into(), enabled: true, description: String::new(), from_template: false },
            ],
            path_params: vec![],
            body: String::new(),
            body_type: "none".into(),
            body_form: vec![],
            body_urlencoded: vec![],
            body_graphql_query: String::new(),
            body_graphql_variables: String::new(),
            authorization: Authorization {
                r#type: "bearer".into(),
                token: "{{token}}".into(),
                ..Default::default()
            },
            cookies: vec![],
            settings: Default::default(),
            response_comment: String::new(),
            is_favorite: false,
            extract_rules: vec![],
            description: "用户详情".into(),
            docs_md: String::new(),
            timeout_ms: 15000,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let mut vars = serde_json::Map::new();
        vars.insert("baseUrl".into(), Value::String("https://api.example.com".into()));
        let json = export_openapi(
            "Demo",
            "示例项目",
            &[("m1".into(), "用户".into())],
            std::slice::from_ref(&ep),
            &vars,
        )
        .unwrap();
        let doc: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(doc["openapi"], "3.0.3");
        assert_eq!(doc["paths"]["/users/{id}"]["get"]["summary"], "获取用户");
        assert_eq!(doc["paths"]["/users/{id}"]["get"]["tags"][0], "用户");
        assert_eq!(doc["servers"][0]["url"], "https://api.example.com");
        assert!(doc["components"]["securitySchemes"]["bearerAuth"].is_object());
        assert_eq!(doc["paths"]["/users/{id}"]["get"]["parameters"][0]["name"], "page");
    }
}
