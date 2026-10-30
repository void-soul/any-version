use super::*;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Tauri 命令
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 获取远程版本列表
/// - `force = false`：优先读本地磁盘缓存，缓存不存在时才联网
/// - `force = true` ：强制联网刷新，刷新后写入缓存
#[tauri::command]
pub async fn project_list_remote_versions(
    id: String,
    force: bool,
) -> Result<RemoteVersionsResult, String> {
    let def = crate::commands::project::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let exclude_patterns: Vec<regex::Regex> = def
        .remote_versions_config
        .as_ref()
        .and_then(|config| config.get("exclude_version_patterns"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|re_str| regex::Regex::new(re_str).ok())
                .collect()
        })
        .unwrap_or_default();

    let filter_versions = |mut vers: Vec<String>| -> Vec<String> {
        if !exclude_patterns.is_empty() {
            vers.retain(|v| !exclude_patterns.iter().any(|re| re.is_match(v)));
        }
        vers
    };

    // 非强制刷新时优先读缓存
    if !force {
        if let Some(mut cache) = load_version_cache(&id) {
            cache.versions = filter_versions(cache.versions);
            return Ok(RemoteVersionsResult {
                versions: cache.versions,
                updated_at: cache.updated_at,
                from_cache: true,
            });
        }
    }

    // 联网获取
    let mut versions = fetch_remote_versions_inner(&id).await?;
    versions = filter_versions(versions);
    let updated_at = save_version_cache(&id, &versions);

    Ok(RemoteVersionsResult {
        versions,
        updated_at,
        from_cache: false,
    })
}

/// 内部实现：联网获取远程版本列表
pub(super) async fn fetch_remote_versions_inner(id: &str) -> Result<Vec<String>, String> {
    let def = crate::commands::project::registry::find_by_id(id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let config = def
        .remote_versions_config
        .as_ref()
        .ok_or_else(|| format!("未配置远程版本: {}", id))?;

    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    let config_type = config
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("static");

    match config_type {
        "static" => {
            let versions = config
                .get("versions")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            Ok(versions)
        }
        "json_api" => fetch_json_api(&client, config, def.remote_versions_url.as_deref()).await,
        "toml_channel" => {
            fetch_toml_channel(&client, config, def.remote_versions_url.as_deref()).await
        }
        "multi_source" => fetch_multi_source(&client, config).await,
        _ => Err(format!("不支持的远程版本类型: {}", config_type)),
    }
}

pub(super) fn parse_version_to_key(v: &str) -> Vec<u64> {
    let mut parts = Vec::new();
    let mut current = 0u64;
    let mut has_digit = false;
    for c in v.chars() {
        if c.is_ascii_digit() {
            current = current * 10 + (c as u64 - '0' as u64);
            has_digit = true;
        } else if has_digit {
            parts.push(current);
            current = 0;
            has_digit = false;
        }
    }
    if has_digit {
        parts.push(current);
    }
    parts
}

/// 从 Rust 官方 channel manifest（TOML）读取最新 stable 版本号。
/// 适用于官方只提供单条 stable 清单、无完整 JSON 版本列表的场景。
/// 配置项：
/// - url            : channel manifest 地址（默认 channel-rust-stable.toml）
/// - static_versions：可选，追加的历史版本列表（合并进结果，便于安装旧版）
pub(super) async fn fetch_toml_channel(
    client: &reqwest::Client,
    config: &serde_json::Value,
    url_override: Option<&str>,
) -> Result<Vec<String>, String> {
    let url = if let Some(u) = url_override {
        u.to_string()
    } else if let Some(u) = config.get("url").and_then(|v| v.as_str()) {
        u.to_string()
    } else {
        "https://static.rust-lang.org/dist/channel-rust-stable.toml".to_string()
    };

    // 带重试的请求（最多 3 次，指数退避）
    let mut last_error = String::new();
    let mut text = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
        }
        match client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    last_error = format!(
                        "HTTP {} ({}), 可能是网络问题",
                        status.as_u16(),
                        status.canonical_reason().unwrap_or("未知")
                    );
                    continue;
                }
                match resp.text().await {
                    Ok(t) => {
                        text = t;
                        break;
                    }
                    Err(e) => {
                        last_error = format!("读取响应体失败: {}", e);
                        continue;
                    }
                }
            }
            Err(e) => {
                last_error = format!("网络请求失败: {}", e);
                continue;
            }
        }
    }
    if text.is_empty() {
        return Err(format!("获取 channel manifest 失败: {}", last_error));
    }

    let mut versions: Vec<String> = Vec::new();
    match parse_channel_manifest_version(&text) {
        Some(v) => versions.push(v),
        None => {
            // 截取前 200 字符帮助诊断（代理劫持/CDN 异常页时能直接看出返回的不是 TOML）
            let preview: String = text.chars().take(200).collect();
            return Err(format!(
                "无法从 channel manifest 解析 [pkg.rust].version，响应预览: {}",
                preview
            ));
        }
    }

    // 合并配置中的历史版本列表（可选），去重
    if let Some(arr) = config.get("static_versions").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if !versions.iter().any(|x| x == s) {
                    versions.push(s.to_string());
                }
            }
        }
    }

    // 按版本号降序，最新 stable 排在最前
    versions.sort_by(|a, b| {
        let key_a = parse_version_to_key(a);
        let key_b = parse_version_to_key(b);
        key_b.cmp(&key_a)
    });
    Ok(versions)
}

/// 从 channel manifest（TOML）文本解析 `[pkg.rust]` 段的 version。
///
/// 值形如 `"1.98.1 (48a229cea 2026-09-01)"` —— 版本号后可跟括号内的提交信息，
/// 因此不能要求版本号后紧跟引号（旧正则的 bug：闭合引号永远匹配不上，
/// 加上 `[^\[]` 无法越过下一个 `[` 段，导致整体必然失配）。
pub(super) fn parse_channel_manifest_version(text: &str) -> Option<String> {
    let re = regex::Regex::new(
        "(?s)\\[pkg\\.rust][^\\[]*?version\\s*=\\s*\\\"(\\d+\\.\\d+(?:\\.\\d+)*)(?:\\s*\\([^\\\"]*\\))?\\\"",
    )
    .ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// 带 Token 与重试的 GitHub JSON 请求，返回解析后的 JSON。
///
/// **刻意抽出来共用**：拉版本列表（`fetch_json_api`）与解析 release 里的具体 asset
/// （`resolve_github_asset`）是两种取数，但「带 Token + 3 次退避 + GitHub 专属状态码提示」
/// 这段完全一样。复制第二份的话，限流提示的措辞就会在两处漂移。
///
/// Token 来源：SDK 模块设置优先，其次 `GITHUB_TOKEN` / `GH_TOKEN`。
/// 未认证的 GitHub API 只有 60 次/小时，多工具一起刷新很容易撞 403。
pub(crate) async fn get_github_json(
    client: &reqwest::Client,
    url: &str,
) -> Result<serde_json::Value, String> {
    let gh_token = if url.contains("api.github.com") || url.contains("github.com") {
        crate::commands::utils::github_api_token()
    } else {
        None
    };
    let mut last_error = String::new();
    // 重试最多 3 次，指数退避 1s / 2s / 4s
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
        }
        let send_fut = {
            let mut req = client.get(url);
            if let Some(token) = gh_token.as_deref() {
                req = req.bearer_auth(token);
            }
            req
        };
        let resp = match send_fut.send().await {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("网络请求失败: {}", e);
                continue;
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let code = status.as_u16();
            last_error = if gh_token.is_some() || url.contains("github.com") {
                // GitHub 专属的可操作提示（401 令牌失效 / 403·429 限流 / 404 仓库不存在）
                format!(
                    "HTTP {} ({}{})",
                    code,
                    status.canonical_reason().unwrap_or("未知"),
                    crate::commands::utils::github_status_hint(code)
                )
            } else {
                format!(
                    "HTTP {} ({}), 可能是 API 限流或网络问题",
                    code,
                    status.canonical_reason().unwrap_or("未知")
                )
            };
            continue;
        }
        let resp_text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                last_error = format!("读取响应体失败: {}", e);
                continue;
            }
        };
        return match serde_json::from_str::<serde_json::Value>(&resp_text) {
            Ok(v) => Ok(v),
            Err(e) => {
                // 截取前 200 字符帮助诊断
                let preview: String = resp_text.chars().take(200).collect();
                Err(format!("JSON 解析失败: {}，响应预览: {}", e, preview))
            }
        };
    }
    Err(last_error)
}

pub(super) async fn fetch_json_api(
    client: &reqwest::Client,
    config: &serde_json::Value,
    url_override: Option<&str>,
) -> Result<Vec<String>, String> {
    let url = if let Some(u) = url_override {
        u
    } else if let Some(u) = config.get("url").and_then(|v| v.as_str()) {
        u
    } else {
        return Err("缺少 url 配置".to_string());
    };
    let response_type = config
        .get("response_type")
        .and_then(|v| v.as_str())
        .unwrap_or("array");
    let version_field = config
        .get("version_field")
        .and_then(|v| v.as_str())
        .unwrap_or("version");
    let version_transform = config
        .get("version_transform")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let filter_field = config.get("filter_field").and_then(|v| v.as_str());
    let filter_value = config.get("filter_value");
    let filter_contains_not = config.get("filter_contains_not").and_then(|v| v.as_str());
    let sort_by_version = config
        .get("sort_by_version")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reverse = config
        .get("reverse")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let extra_field = config.get("extra_field").and_then(|v| v.as_str());

    let body = get_github_json(client, url).await?;

    let items: Vec<serde_json::Value> = match response_type {
        "array" => body.as_array().cloned().unwrap_or_default(),
        "object_with_array" => {
            let arr_field = config
                .get("array_field")
                .and_then(|v| v.as_str())
                .unwrap_or("versions");
            body.get(arr_field)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        }
        "object_with_nested_array" => {
            let arr_field = config
                .get("array_field")
                .and_then(|v| v.as_str())
                .unwrap_or("releases");
            body.get(arr_field)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        }
        // npm registry 等：versions 是 { "1.0.0": {...}, ... } 这样的字典，取键作版本列表
        "object_keys" => {
            let keys_field = config
                .get("array_field")
                .and_then(|v| v.as_str())
                .unwrap_or("versions");
            body.get(keys_field)
                .and_then(|v| v.as_object())
                .map(|o| {
                    o.keys()
                        .map(|k| serde_json::Value::String(k.clone()))
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => {
            return Err(format!("不支持的 response_type: {}", response_type));
        }
    };

    let mut versions: Vec<String> = Vec::new();
    for item in &items {
        if let Some(ff) = filter_field {
            if let Some(fv) = filter_value {
                let item_val = item.get(ff);
                if let Some(fv_bool) = fv.as_bool() {
                    if item_val.and_then(|v| v.as_bool()).unwrap_or(false) != fv_bool {
                        continue;
                    }
                } else if let Some(fv_str) = fv.as_str() {
                    if item_val.and_then(|v| v.as_str()).unwrap_or("") != fv_str {
                        continue;
                    }
                } else if let Some(fv_num) = fv.as_u64() {
                    if item_val.and_then(|v| v.as_u64()).unwrap_or(0) != fv_num {
                        continue;
                    }
                } else if let Some(fv_num) = fv.as_i64() {
                    if item_val.and_then(|v| v.as_i64()).unwrap_or(0) != fv_num {
                        continue;
                    }
                }
            }
        }

        let raw_version = if response_type == "object_with_array" || response_type == "object_keys"
        {
            // items may be objects (Adoptium API) or plain strings (npm registry keys)
            item.get(version_field)
                .and_then(|v| v.as_str().map(String::from))
                .or_else(|| item.as_str().map(String::from))
                .unwrap_or_default()
        } else {
            item.get(version_field)
                .and_then(|v| {
                    if v.is_string() {
                        v.as_str().map(String::from)
                    } else if v.is_array() {
                        Some(
                            v.as_array()
                                .unwrap()
                                .iter()
                                .map(|n| n.to_string())
                                .collect::<Vec<_>>()
                                .join("."),
                        )
                    } else {
                        Some(v.to_string())
                    }
                })
                .unwrap_or_default()
        };

        if raw_version.is_empty() {
            continue;
        }
        if let Some(fc) = filter_contains_not {
            if raw_version.contains(fc) {
                continue;
            }
        }

        let mut ver = apply_transform(&raw_version, version_transform);

        if let Some(ef) = extra_field {
            if let Some(extra_val) = item.get(ef) {
                let extra_format = config
                    .get("extra_format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if extra_format == "lts_label" {
                    if extra_val.is_boolean() && extra_val.as_bool().unwrap_or(false) {
                        ver = format!("{} (LTS)", ver);
                    } else if extra_val.is_string() {
                        let lts_name = extra_val.as_str().unwrap_or("");
                        if !lts_name.is_empty() && lts_name != "false" {
                            ver = format!("{} (LTS: {})", ver, lts_name);
                        }
                    }
                }
            }
        }
        // version_label: prepend vendor/distribution label for display
        if let Some(label) = config.get("version_label").and_then(|v| v.as_str()) {
            ver = format!("{} {}", label, ver);
        }
        versions.push(ver);
    }

    if sort_by_version {
        versions.sort_by(|a, b| {
            let key_a = parse_version_to_key(a);
            let key_b = parse_version_to_key(b);
            key_a.cmp(&key_b)
        });
    }

    if reverse {
        versions.reverse();
    }
    Ok(versions)
}

pub(super) async fn fetch_multi_source(
    client: &reqwest::Client,
    config: &serde_json::Value,
) -> Result<Vec<String>, String> {
    let mut all_versions: Vec<String> = Vec::new();

    if let Some(statics) = config.get("static_versions").and_then(|v| v.as_array()) {
        for v in statics {
            if let Some(s) = v.as_str() {
                all_versions.push(s.to_string());
            }
        }
    }

    if let Some(sources) = config.get("sources").and_then(|v| v.as_array()) {
        let futures: Vec<_> = sources
            .iter()
            .map(|source| {
                let c = client.clone();
                let source = source.clone();
                async move {
                    let url = if let Some(url_template) =
                        source.get("url_template").and_then(|v| v.as_str())
                    {
                        let versions = source.get("versions").and_then(|v| v.as_array());
                        if let Some(vers) = versions {
                            let mut results = Vec::new();
                            for v in vers {
                                if let Some(ver_str) = v.as_str() {
                                    let next: i32 = ver_str.parse().unwrap_or(0) + 1;
                                    let u = url_template
                                        .replace("{major}", ver_str)
                                        .replace("{next}", &next.to_string());
                                    if let Some(r) = fetch_single_source(&c, &source, &u).await {
                                        results.extend(r);
                                    }
                                }
                            }
                            return results;
                        }
                        return Vec::new();
                    } else if let Some(u) = source.get("url").and_then(|v| v.as_str()) {
                        u.to_string()
                    } else {
                        return Vec::new();
                    };
                    fetch_single_source(&c, &source, &url)
                        .await
                        .unwrap_or_default()
                }
            })
            .collect();

        let results = futures_util::future::join_all(futures).await;
        for mut r in results {
            all_versions.append(&mut r);
        }
    }

    Ok(all_versions)
}

pub(super) async fn fetch_single_source(
    client: &reqwest::Client,
    source: &serde_json::Value,
    url: &str,
) -> Option<Vec<String>> {
    let resp = client.get(url).send().await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;

    let response_type = source
        .get("response_type")
        .and_then(|v| v.as_str())
        .unwrap_or("array");
    let max_per = source
        .get("max_per_source")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;
    let version_transform = source
        .get("version_transform")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let items: Vec<serde_json::Value> = match response_type {
        "array" => body.as_array().cloned().unwrap_or_default(),
        "object_with_array" => {
            let arr_field = source
                .get("array_field")
                .and_then(|v| v.as_str())
                .unwrap_or("releases");
            body.get(arr_field)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default()
        }
        _ => return None,
    };

    let mut versions = Vec::new();
    for item in items.iter().take(max_per) {
        if let Some(ff) = source.get("filter_field").and_then(|v| v.as_str()) {
            if let Some(fc) = source.get("filter_contains").and_then(|v| v.as_str()) {
                let val = item.get(ff).and_then(|v| v.as_str()).unwrap_or("");
                if !val.contains(fc) {
                    continue;
                }
            }
        }
        let raw = if response_type == "object_with_array" {
            item.as_str().map(String::from).unwrap_or_default()
        } else {
            let vf = source
                .get("version_field")
                .and_then(|v| v.as_str())
                .unwrap_or("version");
            item.get(vf)
                .and_then(|v| {
                    if v.is_string() {
                        v.as_str().map(String::from)
                    } else if v.is_array() {
                        let fmt = source
                            .get("version_format")
                            .and_then(|f| f.as_str())
                            .unwrap_or("");
                        if fmt == "join_dots" {
                            Some(
                                v.as_array()
                                    .unwrap()
                                    .iter()
                                    .map(|n| n.to_string())
                                    .collect::<Vec<_>>()
                                    .join("."),
                            )
                        } else {
                            Some(v.to_string())
                        }
                    } else {
                        Some(v.to_string())
                    }
                })
                .unwrap_or_default()
        };
        if !raw.is_empty() {
            versions.push(apply_transform(&raw, version_transform));
        }
    }
    Some(versions)
}
