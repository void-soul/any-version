use super::*;

// ─── cURL 导入 ───

pub(super) fn kv_item(key: &str, value: &str) -> KeyValueItem {
    KeyValueItem {
        key: key.to_string(),
        value: value.to_string(),
        enabled: true,
        description: String::new(),
        from_template: false,
    }
}

/// curl 参数分词：支持单/双引号、反斜杠转义、`\` 与 Windows `^` 换行续行。
pub(super) fn shell_tokens(input: &str) -> Vec<String> {
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

pub(super) fn next_arg(tokens: &[String], i: &mut usize) -> Option<String> {
    if *i + 1 < tokens.len() {
        *i += 1;
        Some(tokens[*i].clone())
    } else {
        None
    }
}

/// 解析 `k=v&k2=v2` 形式的键值对。
pub(super) fn parse_pairs(raw: &str) -> Vec<KeyValueItem> {
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
pub(super) fn split_url_query(url: &str) -> (String, Vec<KeyValueItem>) {
    match url.split_once('?') {
        Some((base, q)) => (base.to_string(), parse_pairs(q)),
        None => (url.to_string(), Vec::new()),
    }
}

/// 解析 Cookie 字符串（`a=1; b=2`）或 `@文件` 形式。
pub(super) fn parse_cookie_string(raw: &str) -> Vec<KeyValueItem> {
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
pub(super) fn data_value(raw: &str) -> String {
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
        "-m",
        "--max-time",
        "--connect-timeout",
        "-o",
        "--output",
        "--retry",
        "--retry-delay",
        "--cacert",
        "--cert",
        "--key",
        "-x",
        "--proxy",
        "--proxy-user",
        "--resolve",
        "--interface",
        "--limit-rate",
        "-w",
        "--write-out",
        "--keepalive-time",
        "--expect100-timeout",
        "--dns-servers",
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
                        } else if let Some(path) =
                            val.strip_prefix('@').or_else(|| val.strip_prefix('<'))
                        {
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
            "--compressed"
            | "-i"
            | "--include"
            | "-s"
            | "--silent"
            | "-v"
            | "--verbose"
            | "-g"
            | "--globoff"
            | "--no-progress-meter"
            | "--fail"
            | "-f" => {}
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
    if let Some(pos) = headers
        .iter()
        .position(|h| h.key.eq_ignore_ascii_case("authorization"))
    {
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
