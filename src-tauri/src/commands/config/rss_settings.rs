use super::*;

/// Read SDK menu display preferences; the SDK itself owns runtime state.
#[tauri::command]
pub fn get_project_menu_config(id: String) -> Result<ProjectMenuConfig, String> {
    let config = load_config();
    Ok(config
        .project_menu_configs
        .get(&id)
        .cloned()
        .unwrap_or_else(|| ProjectMenuConfig {
            show_version: true,
            show_service: true,
        }))
}

#[tauri::command]
pub fn update_project_menu_config(
    app_handle: tauri::AppHandle,
    id: String,
    show_version: bool,
    show_service: bool,
) -> Result<(), String> {
    mutate_config(|config| {
        config.project_menu_configs.insert(
            id,
            ProjectMenuConfig {
                show_version,
                show_service,
            },
        );
        Ok(())
    })?;
    let _ = crate::tray::rebuild_tray_menu(&app_handle);
    Ok(())
}

/// 单个 RSS 订阅源（含自定义名称）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RssSourceDto {
    pub url: String,
    #[serde(default)]
    pub name: String,
    /// rss = XML feed, web = site-specific HTML adapter.
    #[serde(default = "default_rss_source_kind")]
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RssConfig {
    pub sources: Vec<RssSourceDto>,
    pub is_first_launch: bool,
}

/// 内置默认 RSS 源的 kind（兼容旧配置反序列化）
pub(super) fn default_rss_source_kind() -> String {
    "rss".to_string()
}

/// 内置默认 RSS 源的名称映射
pub(super) fn default_rss_source_names() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("https://36kr.com/feed".to_string(), "36氪".to_string());
    m.insert(
        "https://www.ruanyifeng.com/blog/atom.xml".to_string(),
        "阮一峰的网络日志".to_string(),
    );
    m
}

/// 将 config 中的 url 列表与名称映射合并为带名称的源列表
pub(super) fn build_rss_sources(config: &Config) -> Vec<RssSourceDto> {
    config
        .rss_sources
        .iter()
        .map(|url| RssSourceDto {
            url: url.clone(),
            name: config
                .rss_source_names
                .get(url)
                .cloned()
                .unwrap_or_default(),
            kind: "rss".to_string(),
        })
        .collect()
}

#[tauri::command]
pub fn get_rss_config() -> Result<RssConfig, String> {
    let mut config = load_config();
    let is_first_launch = !config.has_run_before;
    if is_first_launch {
        config.has_run_before = true;
        if config.rss_sources.is_empty() {
            config.rss_sources = vec![
                "https://36kr.com/feed".to_string(),
                "https://www.ruanyifeng.com/blog/atom.xml".to_string(),
            ];
        }
        if config.rss_source_names.is_empty() {
            config.rss_source_names = default_rss_source_names();
        }
        save_config(&config)?;
    }
    Ok(RssConfig {
        sources: build_rss_sources(&config),
        is_first_launch,
    })
}

#[tauri::command]
pub fn set_rss_sources(sources: Vec<RssSourceDto>) -> Result<(), String> {
    let mut config = load_config();
    config.rss_sources = sources.iter().map(|s| s.url.clone()).collect();
    config.rss_source_names = sources
        .iter()
        .filter(|s| !s.name.trim().is_empty())
        .map(|s| (s.url.clone(), s.name.trim().to_string()))
        .collect();
    save_config(&config)?;
    Ok(())
}

pub(super) fn http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("构建 HTTP 客户端失败")
        })
        .clone()
}

/// 仅允许 http/https 网址，拒绝 file:// 等本地协议。
pub(super) fn validate_http_url(url: &str) -> Result<(), String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err("仅支持 http/https 网址".to_string())
    }
}

#[tauri::command]
pub async fn fetch_rss_feed(url: String) -> Result<String, String> {
    validate_http_url(&url)?;
    let client = http_client();

    let response = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "application/rss+xml, application/xml, text/xml, text/html;q=0.9, */*;q=0.8")
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP 请求失败: 状态码 {}", status));
    }

    let text = response
        .text()
        .await
        .map_err(|e| format!("读取内容失败: {}", e))?;

    Ok(text)
}

#[tauri::command]
pub fn get_auto_start_services() -> Vec<String> {
    let config = load_config();
    let mut list: Vec<String> = config.auto_start_services.into_iter().collect();
    list.sort();
    list
}

#[tauri::command]
pub fn set_auto_start_service(service_id: String, enabled: bool) -> Result<(), String> {
    mutate_config(|config| {
        if enabled {
            config.auto_start_services.insert(service_id);
        } else {
            config.auto_start_services.remove(&service_id);
        }
        Ok(())
    })
}
