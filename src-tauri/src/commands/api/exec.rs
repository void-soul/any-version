//! 请求执行器 + 单元测试运行器。所有 HTTP 请求由 Rust 后端发起。

use super::models::*;
use super::render::{apply_path_params, render_kv, render_template};

const BODY_LIMIT: usize = 2 * 1024 * 1024;

/// 构造 reqwest::Request（含变量渲染）。每次调用都会重新渲染随机变量，
/// 供压测引擎逐请求构造使用；共享的 Client 由调用方提供。
pub fn build_request(client: &reqwest::Client, input: &SendRequestInput) -> Result<reqwest::Request, String> {
    let variables = &input.variables;

    // 渲染 URL（路径参数 + 模板变量）
    let path_rendered = apply_path_params(&input.url, &render_kv(&input.path_params, variables));
    let url_rendered = render_template(&path_rendered, variables);

    // 渲染查询参数
    let mut query: Vec<(String, String)> = render_kv(&input.query_params, variables)
        .into_iter()
        .filter(|kv| kv.enabled && !kv.key.is_empty())
        .map(|kv| (kv.key, kv.value))
        .collect();

    // 渲染请求头
    let mut headers = reqwest::header::HeaderMap::new();
    let rendered_headers = render_kv(&input.headers, variables);
    let has_content_type = rendered_headers.iter().any(|kv| kv.enabled && kv.key.eq_ignore_ascii_case("content-type"));
    for kv in rendered_headers.iter().filter(|kv| kv.enabled && !kv.key.is_empty()) {
        if let Ok(name) = reqwest::header::HeaderName::from_bytes(kv.key.as_bytes()) {
            if let Ok(val) = reqwest::header::HeaderValue::from_str(&kv.value) {
                headers.insert(name, val);
            }
        }
    }

    // 认证
    apply_authorization(&input.authorization, variables, &mut headers, &mut query);

    // Cookie（独立设置）
    let cookies: Vec<String> = render_kv(&input.cookies, variables)
        .into_iter()
        .filter(|kv| kv.enabled && !kv.key.is_empty())
        .map(|kv| format!("{}={}", kv.key, kv.value))
        .collect();
    if !cookies.is_empty() {
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&cookies.join("; ")) {
            headers.insert(reqwest::header::COOKIE, val);
        }
    }

    let method = reqwest::Method::from_bytes(input.method.to_uppercase().as_bytes())
        .unwrap_or(reqwest::Method::GET);

    let mut request = client.request(method.clone(), &url_rendered);
    if !query.is_empty() {
        request = request.query(&query);
    }
    request = request.headers(headers.clone());

    // 组装 body
    match input.body_type.as_str() {
        "json" => {
            let body_rendered = render_template(&input.body, variables);
            if !body_rendered.trim().is_empty() {
                request = request
                    .header("content-type", "application/json")
                    .body(body_rendered);
            }
        }
        "form" => {
            // x-www-form-urlencoded
            let pairs: Vec<(String, String)> = if !input.body_urlencoded.is_empty() {
                render_kv(&input.body_urlencoded, variables)
                    .into_iter()
                    .filter(|kv| kv.enabled && !kv.key.is_empty())
                    .map(|kv| (kv.key, kv.value))
                    .collect()
            } else {
                // 兼容旧版：body 每行 key=value
                let body_rendered = render_template(&input.body, variables);
                let mut form = std::collections::BTreeMap::new();
                for line in body_rendered.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    if let Some((k, v)) = line.split_once('=') {
                        form.insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
                form.into_iter().collect()
            };
            if !pairs.is_empty() {
                request = request
                    .header("content-type", "application/x-www-form-urlencoded")
                    .form(&pairs);
            }
        }
        "formdata" => {
            // multipart/form-data（支持 text 与 file），手动拼装 body 避免额外依赖
            let boundary = format!("----anyversion{}", rand_boundary());
            let mut body: Vec<u8> = Vec::new();
            for item in &input.body_form {
                if !item.enabled || item.key.trim().is_empty() {
                    continue;
                }
                let key = render_template(&item.key, variables);
                if item.kind == "file" && !item.file_path.is_empty() {
                    let path = render_template(&item.file_path, variables);
                    let fname = std::path::Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "file".to_string());
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            body.extend_from_slice(
                                format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n", boundary, key, fname).as_bytes(),
                            );
                            body.extend_from_slice(&bytes);
                            body.extend_from_slice(b"\r\n");
                        }
                        Err(_) => {
                            // 文件不存在时按文本值发送，避免请求失败
                            body.extend_from_slice(
                                format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n", boundary, key, render_template(&item.value, variables)).as_bytes(),
                            );
                        }
                    }
                } else {
                    body.extend_from_slice(
                        format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n", boundary, key, render_template(&item.value, variables)).as_bytes(),
                    );
                }
            }
            body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
            request = request
                .header("content-type", format!("multipart/form-data; boundary={}", boundary))
                .body(body);
        }
        "graphql" => {
            let query_rendered = render_template(&input.body_graphql_query, variables);
            let vars_rendered = render_template(&input.body_graphql_variables, variables);
            if !query_rendered.trim().is_empty() {
                let mut obj = serde_json::Map::new();
                obj.insert("query".to_string(), serde_json::Value::String(query_rendered));
                let vars: serde_json::Value = serde_json::from_str(&vars_rendered).unwrap_or(serde_json::Value::Null);
                obj.insert("variables".to_string(), vars);
                let payload = serde_json::Value::Object(obj).to_string();
                request = request
                    .header("content-type", "application/json")
                    .body(payload);
            }
        }
        "binary" => {
            let path = render_template(&input.body, variables);
            if !path.trim().is_empty() {
                if let Ok(bytes) = std::fs::read(path.trim()) {
                    if !has_content_type {
                        request = request.header("content-type", "application/octet-stream");
                    }
                    request = request.body(bytes);
                }
            }
        }
        _ => {
            // raw
            let body_rendered = render_template(&input.body, variables);
            if !body_rendered.is_empty() {
                if !has_content_type {
                    request = request.header("content-type", "text/plain");
                }
                request = request.body(body_rendered);
            }
        }
    }
    request.build().map_err(|e| format!("构造请求失败: {}", e))
}

/// 应用认证（Basic / Bearer / JWT / API Key）。
fn apply_authorization(
    auth: &Authorization,
    variables: &serde_json::Map<String, serde_json::Value>,
    headers: &mut reqwest::header::HeaderMap,
    query: &mut Vec<(String, String)>,
) {
    match auth.r#type.as_str() {
        "basic" => {
            let user = render_template(&auth.username, variables);
            let pass = render_template(&auth.password, variables);
            let token = base64_standard(&format!("{}:{}", user, pass));
            if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Basic {}", token)) {
                headers.insert(reqwest::header::AUTHORIZATION, val);
            }
        }
        "bearer" => {
            let token = render_template(&auth.token, variables);
            if !token.is_empty() {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
                    headers.insert(reqwest::header::AUTHORIZATION, val);
                }
            }
        }
        "jwt" => {
            let token = render_template(&auth.jwt_token, variables);
            if !token.is_empty() {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)) {
                    headers.insert(reqwest::header::AUTHORIZATION, val);
                }
            }
        }
        "apiKey" => {
            let name = render_template(&auth.api_key_name, variables);
            let value = render_template(&auth.api_key_value, variables);
            if name.is_empty() {
                return;
            }
            if auth.api_key_in == "query" {
                query.push((name, value));
            } else if let Ok(hn) = reqwest::header::HeaderName::from_bytes(name.as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(&value) {
                    headers.insert(hn, val);
                }
            }
        }
        _ => {}
    }
}

fn base64_standard(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// 随机 multipart 边界（避免与内容冲突）。
fn rand_boundary() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}", std::process::id(), n)
}

/// URL 长度上限（防止超长 URL 占用内存 / 攻击面）。
const URL_MAX_LEN: usize = 10 * 1024;

/// 组装并执行单个 HTTP 请求（含变量渲染 + 认证 + 设置）。
pub async fn execute_request(input: &SendRequestInput) -> Result<SendRequestOutput, String> {
    if input.url.len() > URL_MAX_LEN {
        return Err(format!("URL 过长（{} 字节），上限 {} 字节", input.url.len(), URL_MAX_LEN));
    }
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(input.timeout_ms.max(100) as u64));

    let s = &input.settings;
    if !s.verify_ssl {
        builder = builder.danger_accept_invalid_certs(true);
    }
    // follow_original_method / remove_referer_on_redirect 为尽力而为：
    // reqwest 高层 API 不直接暴露，跟随重定向时保持默认（跟随方法、携带 Referer）。
    if s.follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::limited(10));
    } else {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    if s.http_version == "http1" {
        builder = builder.http1_only();
    } else if s.http_version == "http2" {
        builder = builder.http2_prior_knowledge();
    }

    let client = builder
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {}", e))?;
    let request = build_request(&client, input)?;

    let started = std::time::Instant::now();
    let response = client.execute(request).await.map_err(|e| format!("请求失败: {}", e))?;
    let status = response.status();
    let status_text = status.canonical_reason().unwrap_or("").to_string();
    let headers_out: Vec<KeyValueItem> = response
        .headers()
        .iter()
        .take(60)
        .map(|(name, value)| KeyValueItem {
            key: name.to_string(),
            value: value.to_str().unwrap_or("<binary>").to_string(),
            enabled: true,
            description: String::new(),
        })
        .collect();
    let size_bytes = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let body_bytes = response.bytes().await.map_err(|e| format!("读取响应失败: {}", e))?;
    let body_truncated = body_bytes.len() > BODY_LIMIT;
    let body = String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(BODY_LIMIT)]).to_string();
    let time_ms = started.elapsed().as_millis();

    Ok(SendRequestOutput {
        ok: status.is_success(),
        status: status.as_u16(),
        status_text,
        headers: headers_out,
        body,
        body_truncated,
        time_ms,
        size_bytes,
    })
}

// ─── JSON 路径取值（支持 a.b[0].c） ───

pub fn json_get<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() || path == "$" {
        return Some(value);
    }
    let mut current = value;
    // 解析 a.b[0].c 形式的路径
    let mut token = String::new();
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                if !token.is_empty() {
                    current = descend(current, &token)?;
                    token.clear();
                }
            }
            '[' => {
                if !token.is_empty() {
                    current = descend(current, &token)?;
                    token.clear();
                }
                let mut idx = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc == ']' {
                        chars.next();
                        break;
                    }
                    idx.push(nc);
                    chars.next();
                }
                if let Ok(i) = idx.parse::<usize>() {
                    current = current.as_array()?.get(i)?;
                } else if !idx.is_empty() {
                    current = descend(current, &idx)?;
                }
            }
            _ => token.push(c),
        }
    }
    if !token.is_empty() {
        current = descend(current, &token)?;
    }
    Some(current)
}

fn descend<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map.get(key),
        serde_json::Value::Array(arr) => {
            // 支持通配 * 取第一个
            if key == "*" {
                arr.first()
            } else {
                key.parse::<usize>().ok().and_then(|i| arr.get(i))
            }
        }
        _ => None,
    }
}

// ─── 单元测试 ───

/// 对一次响应结果执行断言列表，返回逐条结果。
pub fn evaluate_assertions(
    assertions: &[UnitTestAssertion],
    output: &SendRequestOutput,
) -> Vec<AssertionResult> {
    let json: Option<serde_json::Value> = serde_json::from_str(&output.body).ok();
    let mut results = Vec::with_capacity(assertions.len());
    for a in assertions {
        let (pass, actual) = evaluate_one(a, output, json.as_ref());
        results.push(AssertionResult {
            pass,
            assertion: describe_assertion(a),
            actual,
        });
    }
    results
}

fn describe_assertion(a: &UnitTestAssertion) -> String {
    let expected = a
        .expected
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_default();
    match a.r#type.as_str() {
        "status_eq" => format!("状态码 == {}", expected),
        "status_lt" => format!("状态码 < {}", expected),
        "status_gt" => format!("状态码 > {}", expected),
        "body_contains" => format!("Body 包含 \"{}\"", expected),
        "body_not_contains" => format!("Body 不包含 \"{}\"", expected),
        "json_path" => format!("JSON {} {} {}", a.path.as_deref().unwrap_or("$"), a.op.as_deref().unwrap_or("eq"), expected),
        "time_lt_ms" => format!("耗时 < {}ms", expected),
        other => format!("未知断言 {}", other),
    }
}

fn evaluate_one(
    a: &UnitTestAssertion,
    output: &SendRequestOutput,
    json: Option<&serde_json::Value>,
) -> (bool, String) {
    let expected = a.expected.as_ref();
    match a.r#type.as_str() {
        "status_eq" => {
            let want = expected.and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            (output.status == want, output.status.to_string())
        }
        "status_lt" => {
            let want = expected.and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            (output.status < want, output.status.to_string())
        }
        "status_gt" => {
            let want = expected.and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            (output.status > want, output.status.to_string())
        }
        "body_contains" => {
            let want = expected.and_then(|v| v.as_str()).unwrap_or("");
            (output.body.contains(want), format!("contains={}", want))
        }
        "body_not_contains" => {
            let want = expected.and_then(|v| v.as_str()).unwrap_or("");
            (!output.body.contains(want), format!("not_contains={}", want))
        }
        "time_lt_ms" => {
            let want = expected.and_then(|v| v.as_u64()).unwrap_or(0);
            (output.time_ms < want as u128, format!("{}ms", output.time_ms))
        }
        "json_path" => {
            let path = a.path.as_deref().unwrap_or("$");
            let got = json.and_then(|j| json_get(j, path));
            let op = a.op.as_deref().unwrap_or("eq");
            match (got, expected) {
                (Some(g), Some(want)) => {
                    let pass = match op {
                        "eq" => g == want,
                        "ne" => g != want,
                        "contains" => g
                            .as_str()
                            .map(|s| s.contains(want.as_str().unwrap_or("")))
                            .unwrap_or(false),
                        "gt" => numeric_cmp(g, want) > 0,
                        "lt" => numeric_cmp(g, want) < 0,
                        _ => false,
                    };
                    (pass, g.to_string())
                }
                (None, _) => (false, format!("路径 {} 不存在", path)),
                (Some(g), None) => (false, g.to_string()),
            }
        }
        _ => (false, "未知断言".to_string()),
    }
}

fn numeric_cmp(a: &serde_json::Value, b: &serde_json::Value) -> i64 {
    let an = a.as_f64().unwrap_or(0.0);
    let bn = b.as_f64().unwrap_or(0.0);
    if an > bn {
        1
    } else if an < bn {
        -1
    } else {
        0
    }
}

/// 从接口定义构造请求输入（附带环境变量）。
pub fn endpoint_to_input(ep: &ApiEndpoint, variables: &serde_json::Map<String, serde_json::Value>) -> SendRequestInput {
    SendRequestInput {
        method: ep.method.clone(),
        url: ep.url.clone(),
        headers: ep.headers.clone(),
        query_params: ep.query_params.clone(),
        path_params: ep.path_params.clone(),
        body: ep.body.clone(),
        body_type: ep.body_type.clone(),
        body_form: ep.body_form.clone(),
        body_urlencoded: ep.body_urlencoded.clone(),
        body_graphql_query: ep.body_graphql_query.clone(),
        body_graphql_variables: ep.body_graphql_variables.clone(),
        authorization: ep.authorization.clone(),
        cookies: ep.cookies.clone(),
        settings: ep.settings.clone(),
        timeout_ms: ep.timeout_ms,
        variables: variables.clone(),
    }
}
