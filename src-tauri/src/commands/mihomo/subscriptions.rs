use super::*;

#[tauri::command]
pub async fn mihomo_validate_subscription(url: String) -> Result<SubValidation, String> {
    // 默认校验 TLS 证书（弱证书容错通过，不关闭校验）；个别证书链不完整的订阅站
    // 可在配置里为对应订阅开启「跳过证书校验」豁免。
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("User-Agent", "clash-verge/v1.7.0")
        .header("Accept-Encoding", "identity")
        .send()
        .await
        .map_err(|e| format!("订阅下载失败: {e}"))?;
    let headers = resp.headers().clone();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    // 嗅探并处理内容（age 解密 / base64 节点解码 / YAML），与正式导入保持一致
    let processed = crate::commands::mihomo::subparse::process_subscription_content(&text, None)
        .unwrap_or_else(|_| text.clone());
    let parsed: Value = match serde_yaml::from_str(&processed) {
        Ok(v) => v,
        Err(_) => serde_yaml::from_str(&text).map_err(|e| format!("订阅解析失败: {e}"))?,
    };
    let ok = parsed.get("proxy-providers").is_some()
        || parsed.get("proxies").is_some()
        || parsed.get("rules").is_some();
    let mut upload = None;
    let mut download = None;
    let mut total = None;
    let mut expire = None;
    if let Some(info) = headers
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
    {
        for part in info.split(';') {
            let kv: Vec<&str> = part.split('=').collect();
            if kv.len() == 2 {
                match kv[0].trim() {
                    "upload" => upload = kv[1].trim().parse().ok(),
                    "download" => download = kv[1].trim().parse().ok(),
                    "total" => total = kv[1].trim().parse().ok(),
                    "expire" => expire = kv[1].trim().parse().ok(),
                    _ => {}
                }
            }
        }
    }
    Ok(SubValidation {
        ok,
        message: if ok {
            "订阅有效".into()
        } else {
            "订阅内容缺少 proxies/rules".into()
        },
        upload,
        download,
        total,
        expire,
        suggested_interval: None,
    })
}

// 解析 subscription-userinfo 响应头: upload=..; download=..; total=..; expire=..
pub(super) fn parse_subinfo(header: Option<&str>) -> Option<SubscriptionUserInfo> {
    let s = header?;
    let mut upload = None;
    let mut download = None;
    let mut total = None;
    let mut expire = None;
    for part in s.split(';') {
        let kv: Vec<&str> = part.split('=').collect();
        if kv.len() == 2 {
            match kv[0].trim() {
                "upload" => upload = kv[1].trim().parse().ok(),
                "download" => download = kv[1].trim().parse().ok(),
                "total" => total = kv[1].trim().parse().ok(),
                "expire" => expire = kv[1].trim().parse().ok(),
                _ => {}
            }
        }
    }
    // 订阅商不一定返回全部字段（很多不返回 upload），任一缺失不应让整段失效
    Some(SubscriptionUserInfo {
        upload: upload.unwrap_or(0),
        download: download.unwrap_or(0),
        total: total.unwrap_or(0),
        expire: expire.unwrap_or(0),
    })
}

// 从订阅 URL 推导默认名称：优先 host（example.com），其次路径末段，最后原 URL。
// 不引入额外依赖，用简单字符串解析。
pub(super) fn extract_sub_name_from_url(url: &str) -> String {
    // 去掉协议头
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    // host 为到下一个 '/'、'?'、'#' 之前的部分
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim();
    if !authority.is_empty() {
        // 去掉可能存在的 userinfo@ 与端口
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host);
        if !host.is_empty() {
            return host.to_string();
        }
    }
    // 无 authority 时取路径最后一段非空片段
    if let Some(seg) = after_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|p| !p.is_empty())
        .last()
    {
        return seg.to_string();
    }
    url.to_string()
}

// 导入订阅网址：下载 YAML 并作为「file」型配置存到本地（等价 clash-party 的 remote 导入）
#[tauri::command]
pub async fn mihomo_import_subscription(
    app: AppHandle,
    state: State<'_, MihomoState>,
    url: String,
) -> Result<ProfileItem, String> {
    // F7 修复：加超时
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .header("User-Agent", "clash-verge/v1.7.0")
        .header("Accept-Encoding", "identity")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let userinfo_header = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let parsed_userinfo = parse_subinfo(userinfo_header.as_deref());
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let _: Value = serde_yaml::from_str(&text).map_err(|e| format!("订阅解析失败: {e}"))?;
    let id = format!("sub_{}", now_secs());
    // 默认订阅名称：从 URL 提取 host（如 https://sub.example.com/abc -> example.com），
    // 无 host 时退用路径最后一段，再不行用原始 URL。比整串 URL 更友好可读。
    let default_name = extract_sub_name_from_url(&url);
    let item = ProfileItem {
        id: id.clone(),
        name: default_name,
        type_: "file".into(),
        file_path: None,
        subscriptions: vec![],
        rule_providers: vec![],
        custom_rules: vec![],
        dns_enabled: false,
        dns_nameservers: vec![],
        url: Some(url.clone()),
        age_secret_key: None,
        auth_token: None,
        user_agent: None,
        use_proxy: false,
        auto_update: false,
        skip_verify: false,
        update_interval: 86400,
        update_timeout: 30,
        update_cron: None,
        override_ids: vec![],
        subscription_userinfo: parsed_userinfo,
        updated_at: Some(now_secs()),
    };
    {
        let mut cfg = state.profile_config.lock().unwrap().clone();
        cfg.items.push(item.clone());
        save_profile_config(&state.data_dir, &cfg).ok();
        *state.profile_config.lock().unwrap() = cfg;
    }
    write_profile_content(&state.data_dir, &item, &text).map_err(|e| e.to_string())?;
    emit_state(&app, &state);
    Ok(item)
}

// 导入本地 yml 文件：读取内容并作为「file」型配置存到本地（等价 clash-party 打开 yml）
#[tauri::command]
pub fn mihomo_import_file(
    app: AppHandle,
    state: State<'_, MihomoState>,
    path: String,
) -> Result<ProfileItem, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;
    let _: Value = serde_yaml::from_str(&text).map_err(|e| format!("yaml 解析失败: {e}"))?;
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("导入配置")
        .to_string();
    let id = format!("file_{}", now_secs());
    let item = ProfileItem {
        id: id.clone(),
        name,
        type_: "file".into(),
        file_path: None,
        subscriptions: vec![],
        rule_providers: vec![],
        custom_rules: vec![],
        dns_enabled: false,
        dns_nameservers: vec![],
        age_secret_key: None,
        url: None,
        auth_token: None,
        user_agent: None,
        use_proxy: false,
        auto_update: false,
        skip_verify: false,
        update_interval: 86400,
        update_timeout: 30,
        update_cron: None,
        override_ids: vec![],
        subscription_userinfo: None,
        updated_at: Some(now_secs()),
    };
    {
        let mut cfg = state.profile_config.lock().unwrap().clone();
        cfg.items.push(item.clone());
        save_profile_config(&state.data_dir, &cfg).ok();
        *state.profile_config.lock().unwrap() = cfg;
    }
    write_profile_content(&state.data_dir, &item, &text).map_err(|e| e.to_string())?;
    emit_state(&app, &state);
    Ok(item)
}

/// 获取 mihomo mixed-port（若已配置则返回，供"使用代理更新订阅"走本地代理）。
pub(super) fn get_mixed_port(state: &State<'_, MihomoState>) -> Option<u16> {
    state
        .app_config
        .lock()
        .ok()
        .map(|c| c.mixed_port)
        .filter(|p| *p > 0)
}

// 按订阅地址重新拉取订阅内容（使用编辑信息里的 授权令牌/UA/超时），更新流量信息并回写
#[tauri::command]
pub async fn mihomo_update_subscription(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<ProfileItem, String> {
    let item = {
        let cfg = state.profile_config.lock().unwrap();
        cfg.items.iter().find(|i| i.id == id).cloned()
    }
    .ok_or("配置不存在")?;
    let url = item.url.clone().ok_or("该配置没有订阅地址（url）")?;
    let mut client_builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(item.update_timeout.max(5)))
        // 默认校验 TLS；仅当该订阅显式开启「跳过证书校验」时才豁免，
        // 避免订阅 token / 配置在网络上被中间人替换或窃取。
        .danger_accept_invalid_certs(item.skip_verify);
    // 使用代理更新开关：走本地 mihomo mixed-port，实现"绕墙更新订阅"
    if item.use_proxy {
        if let Some(port) = get_mixed_port(&state) {
            let proxy_url = format!("http://127.0.0.1:{}", port);
            client_builder =
                client_builder.proxy(reqwest::Proxy::all(&proxy_url).map_err(|e| e.to_string())?);
        }
    }
    let client = client_builder.build().map_err(|e| e.to_string())?;
    let mut req = client.get(&url);
    // 对齐 clash-party fetchAndValidateSubscription：强制不压缩，
    // 避免部分订阅站返回 gzip 内容导致 base64/YAML 解析异常。
    req = req.header("Accept-Encoding", "identity");
    if let Some(ua) = &item.user_agent {
        req = req.header("User-Agent", ua.clone());
    }
    if let Some(token) = &item.auth_token {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let userinfo_header = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let text = resp.text().await.map_err(|e| e.to_string())?;
    // 嗅探并处理订阅内容：age 解密 / base64 节点解码 / YAML 校验，转成 mihomo 可加载的 clash YAML
    let processed = crate::commands::mihomo::subparse::process_subscription_content(
        &text,
        item.age_secret_key.as_deref(),
    )
    .map_err(|e| format!("订阅内容解析失败: {e}"))?;
    // 校验解析结果确实是 clash YAML
    let parsed: Value =
        serde_yaml::from_str(&processed).map_err(|e| format!("订阅解析失败: {e}"))?;
    if parsed.get("proxies").is_none() && parsed.get("proxy-providers").is_none() {
        // base64 解码后可能是节点列表但未生成 proxies；若为空则报错
        if text.trim().is_empty() {
            return Err("订阅内容为空".to_string());
        }
    }
    let mut updated = item.clone();
    updated.subscription_userinfo = parse_subinfo(userinfo_header.as_deref());
    updated.updated_at = Some(now_secs());
    write_profile_content(&state.data_dir, &updated, &processed).map_err(|e| e.to_string())?;
    {
        let mut cfg = state.profile_config.lock().unwrap();
        if let Some(it) = cfg.items.iter_mut().find(|i| i.id == id) {
            *it = updated.clone();
        }
        save_profile_config(&state.data_dir, &cfg).ok();
    }
    emit_state(&app, &state);
    Ok(updated)
}

// 查询某个订阅的可用性 + 节点数（用于列表展示「可用/不可用」与「数量」）
#[tauri::command]
pub fn mihomo_get_profile_status(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<Value, String> {
    let item = {
        let cfg = state.profile_config.lock().unwrap();
        cfg.items.iter().find(|i| i.id == id).cloned()
    };
    let content = match item {
        Some(ref it) => read_profile_content(&state.data_dir, it).unwrap_or_default(),
        None => String::new(),
    };
    let available = !content.trim().is_empty();
    let mut node_count = 0u32;
    // 兼容 base64 节点 / age 加密内容：先嗅探处理为 clash YAML 再数节点
    let processed = crate::commands::mihomo::subparse::process_subscription_content(
        &content,
        item.as_ref().and_then(|i| i.age_secret_key.as_deref()),
    )
    .unwrap_or(content.clone());
    if let Ok(v) = serde_yaml::from_str::<Value>(&processed) {
        if let Some(proxies) = v.get("proxies").and_then(|p| p.as_array()) {
            node_count = proxies.len() as u32;
        }
    }
    Ok(json!({ "available": available, "node_count": node_count }))
}
