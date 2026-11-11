use super::*;

/// 构建订阅下载 client（超时 / 证书校验 / 走本地代理），四条路径共用。
/// 默认校验 TLS；仅当该订阅显式开启「跳过证书校验」时才豁免。
pub(super) fn build_sub_client(
    item: &ProfileItem,
    mixed_port: Option<u16>,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(item.update_timeout.max(5)))
        .danger_accept_invalid_certs(item.skip_verify);
    if item.use_proxy {
        if let Some(port) = mixed_port {
            let proxy_url = format!("http://127.0.0.1:{port}");
            builder = builder.proxy(reqwest::Proxy::all(&proxy_url).map_err(|e| e.to_string())?);
        }
    }
    builder.build().map_err(|e| e.to_string())
}

/// 一次订阅下载：统一请求头，返回 (正文, subscription-userinfo 响应头)。
pub(super) async fn fetch_subscription_text(
    item: &ProfileItem,
    url: &str,
    mixed_port: Option<u16>,
) -> Result<(String, Option<String>), String> {
    let client = build_sub_client(item, mixed_port)?;
    let mut req = client.get(url);
    for (k, v) in subscription_headers(item) {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| format!("订阅下载失败: {e}"))?;
    let userinfo = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);
    let text = resp
        .text()
        .await
        .map_err(|e| format!("订阅内容读取失败: {e}"))?;
    Ok((text, userinfo))
}

#[tauri::command]
pub async fn mihomo_validate_subscription(url: String) -> Result<SubValidation, String> {
    // 与正式导入共用同一套请求头 + 内容处理，避免「校验通过但导入拿到另一份内容」。
    let probe = ProfileItem::default();
    let (text, userinfo) = fetch_subscription_text(&probe, &url, None).await?;
    let (ok, message) = match prepare_subscription_content(&text, None) {
        Ok(_) => (true, "订阅有效".to_string()),
        Err(e) => (false, format!("订阅内容无效: {e}")),
    };
    let info = parse_subinfo(userinfo.as_deref());
    Ok(SubValidation {
        ok,
        message,
        upload: info.as_ref().map(|i| i.upload),
        download: info.as_ref().map(|i| i.download),
        total: info.as_ref().map(|i| i.total),
        expire: info.as_ref().map(|i| i.expire),
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

// ---------- 订阅下载 / 校验 / 落盘的共享层 ----------
// 校验、导入、手动刷新、自动更新四条路径必须共用这一层：过去「导入带默认 UA
// 且写原文」而「刷新不带 UA 且写重建内容」，订阅站按 UA 返回不同格式时就会
// 刷新后内容变样；刷新校验又形同虚设，错误页能直接覆盖好配置且无备份。

/// 订阅下载默认 UA（clash-verge / clash-party 语义）。
pub const DEFAULT_SUB_UA: &str = "clash-verge/v1.7.0";

/// 未显式配置 UA（或只填了空白）时回退到默认 UA。
pub(super) fn effective_user_agent(user_agent: Option<&str>) -> String {
    match user_agent.map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => DEFAULT_SUB_UA.to_string(),
    }
}

/// 统一的订阅请求头，四条下载路径共用。
pub(super) fn subscription_headers(item: &ProfileItem) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        (
            "User-Agent",
            effective_user_agent(item.user_agent.as_deref()),
        ),
        ("Accept-Encoding", "identity".to_string()),
    ];
    if let Some(token) = item
        .auth_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        headers.push(("Authorization", format!("Bearer {token}")));
    }
    headers
}

/// 从 AppConfig 取可用的本地 mixed-port（供「使用代理更新订阅」）。
pub(super) fn mixed_port_of(app_config: &AppConfig) -> Option<u16> {
    let p = app_config.mixed_port;
    (p > 0).then_some(p)
}

/// 校验处理后的订阅正文确实是 clash 配置，拒绝错误页 / 过期提示 / 纯文本。
pub(super) fn validate_processed_content(processed: &str) -> Result<Value, String> {
    let parsed: Value =
        serde_yaml::from_str(processed).map_err(|e| format!("订阅解析失败: {e}"))?;
    if parsed.as_object().is_none() {
        return Err("订阅内容不是有效的 clash 配置（缺少配置节）".to_string());
    }
    let has_sections = parsed.get("proxies").is_some()
        || parsed.get("proxy-providers").is_some()
        || parsed.get("rules").is_some();
    if !has_sections {
        return Err(
            "订阅内容不是有效的 clash 配置（缺少 proxies/proxy-providers/rules）".to_string(),
        );
    }
    if let Some(proxies) = parsed.get("proxies") {
        if proxies.as_array().is_none() {
            return Err("订阅内容中 proxies 必须是数组".to_string());
        }
    }
    Ok(parsed)
}

/// 下载正文 → 嗅探处理（age / base64 / YAML）→ 结构校验。四条路径的唯一入口。
pub(super) fn prepare_subscription_content(
    text: &str,
    age_secret_key: Option<&str>,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("订阅内容为空".to_string());
    }
    let processed =
        crate::commands::mihomo::subparse::process_subscription_content(text, age_secret_key)
            .map_err(|e| format!("订阅内容解析失败: {e}"))?;
    validate_processed_content(&processed)?;
    Ok(processed)
}

/// 校验通过才落盘：失败时现有 profiles/<id>.yaml 保持不动。
pub(super) fn commit_subscription_content(
    dir: &Path,
    item: &ProfileItem,
    text: &str,
) -> Result<String, String> {
    let processed = prepare_subscription_content(text, item.age_secret_key.as_deref())?;
    write_profile_content(dir, item, &processed).map_err(|e| format!("订阅写入失败: {e}"))?;
    Ok(processed)
}

/// 生成不与现有条目冲突的 id。秒级 id 会让同一秒导入的两条订阅共用同一个
/// profiles/<id>.yaml，刷新其中一条时另一条内容也跟着变。
static PROFILE_ID_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) fn unique_profile_id(candidate: &str, existing: &[ProfileItem]) -> String {
    if !existing.iter().any(|i| i.id == candidate) {
        return candidate.to_string();
    }
    let mut n = 2u32;
    loop {
        let next = format!("{candidate}-{n}");
        if !existing.iter().any(|i| i.id == next) {
            return next;
        }
        n += 1;
    }
}

pub(super) fn new_profile_id(existing: &[ProfileItem], prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let ctr = PROFILE_ID_CTR.fetch_add(1, Ordering::SeqCst);
    unique_profile_id(&format!("{prefix}_{millis}_{ctr:x}"), existing)
}

// 导入订阅网址：下载 YAML 并作为「file」型配置存到本地（等价 clash-party 的 remote 导入）
#[tauri::command]
pub async fn mihomo_import_subscription(
    app: AppHandle,
    state: State<'_, MihomoState>,
    url: String,
) -> Result<ProfileItem, String> {
    let probe = ProfileItem::default();
    let (text, userinfo_header) = fetch_subscription_text(&probe, &url, None).await?;
    let id = {
        let cfg = state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        new_profile_id(&cfg.items, "sub")
    };
    // 默认订阅名称：从 URL 提取 host（如 https://sub.example.com/abc -> example.com），
    // 无 host 时退用路径最后一段，再不行用原始 URL。比整串 URL 更友好可读。
    let default_name = extract_sub_name_from_url(&url);
    let item = ProfileItem {
        id,
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
        subscription_userinfo: parse_subinfo(userinfo_header.as_deref()),
        updated_at: Some(now_secs()),
    };
    // 与刷新共用「校验通过才落盘」：错误页不会留下没有内容的空条目，
    // 且落盘内容与刷新完全一致（不再是原始文本 vs 重建文本）。
    commit_subscription_content(&state.data_dir, &item, &text)?;
    {
        let mut cfg = state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        cfg.items.push(item.clone());
        save_profile_config(&state.data_dir, &cfg).ok();
        *state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = cfg;
    }
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
    let id = {
        let cfg = state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        new_profile_id(&cfg.items, "file")
    };
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
        let mut cfg = state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        cfg.items.push(item.clone());
        save_profile_config(&state.data_dir, &cfg).ok();
        *state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = cfg;
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
        .and_then(|c| mixed_port_of(&*c))
}

// 按订阅地址重新拉取订阅内容（使用编辑信息里的 授权令牌/UA/超时），更新流量信息并回写
#[tauri::command]
pub async fn mihomo_update_subscription(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<ProfileItem, String> {
    let item = {
        let cfg = state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cfg.items.iter().find(|i| i.id == id).cloned()
    }
    .ok_or("配置不存在")?;
    let url = item.url.clone().ok_or("该配置没有订阅地址（url）")?;
    let (text, userinfo_header) =
        fetch_subscription_text(&item, &url, get_mixed_port(&state)).await?;
    let mut updated = item.clone();
    updated.subscription_userinfo = parse_subinfo(userinfo_header.as_deref());
    updated.updated_at = Some(now_secs());
    // 与导入共用「校验通过才落盘」：错误页 / 过期提示不会覆盖好配置，
    // 落盘前 write_profile_content 还会保留一份 .bak。
    commit_subscription_content(&state.data_dir, &updated, &text)?;
    {
        let mut cfg = state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
        let cfg = state
            .profile_config
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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

#[cfg(test)]
mod tests;
