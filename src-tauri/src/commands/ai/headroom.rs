//! Headroom 本地压缩服务的探活与调用约定。
//!
//! Headroom 由「服务」页的 Headroom 服务项托管（pip 包模式启动 `headroom proxy`），
//! 这里只负责：探活（供 AI 设置页显示状态）与压缩请求的地址/路径约定（供请求链路调用）。

use serde::Serialize;
use std::time::Duration;

use super::config::{load_ai_config, save_ai_config_to_file};
use super::models::HeadroomConfig;

/// Headroom 默认端口（须与 node-projects/headroom.json 的 defaultPort 一致，有单测守卫）。
pub const HEADROOM_DEFAULT_PORT: u16 = 8791;

/// 服务基址。
pub fn headroom_base_url(port: u16) -> String {
    format!("http://127.0.0.1:{}", port)
}

/// 探活路径（按顺序尝试）：readyz 就绪探针优先，health 兜底。
pub fn headroom_health_paths() -> [&'static str; 2] {
    ["/readyz", "/health"]
}

/// 压缩旁路端点（compression-only，不发上游生成请求）。
pub fn headroom_compress_path() -> &'static str {
    "/v1/compress"
}

/// 探活结果（Alive 与否都正常返回，错误信息给前端展示）。
#[derive(Debug, Clone, Serialize)]
pub struct HeadroomHealth {
    pub alive: bool,
    pub base_url: String,
    /// 命中的探活路径（存活时）
    pub path: Option<String>,
    /// HTTP 状态码（存活时）
    pub status: Option<u16>,
    /// 失败原因 / 说明
    pub detail: String,
}

/// 读取 Headroom 压缩配置（链路页用，不必加载整个 AI 配置）。
#[tauri::command]
pub fn get_headroom_config() -> HeadroomConfig {
    load_ai_config().headroom
}

/// 保存 Headroom 压缩配置。
#[tauri::command]
pub fn save_headroom_config(config: HeadroomConfig) -> Result<(), String> {
    let mut current = load_ai_config();
    current.headroom = config;
    save_ai_config_to_file(&current)
}

/// 探活：依次尝试 [`headroom_health_paths`]，任一 2xx/3xx 视为存活。
#[tauri::command]
pub async fn check_headroom_health(port: Option<u16>) -> HeadroomHealth {
    let port = port.unwrap_or(HEADROOM_DEFAULT_PORT);
    let base_url = headroom_base_url(port);
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return HeadroomHealth {
                alive: false,
                base_url,
                path: None,
                status: None,
                detail: format!("HTTP 客户端构建失败: {e}"),
            }
        }
    };

    let mut last_detail = String::new();
    for path in headroom_health_paths() {
        let url = format!("{base_url}{path}");
        match client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() || status.is_redirection() {
                    return HeadroomHealth {
                        alive: true,
                        base_url,
                        path: Some(path.to_string()),
                        status: Some(status.as_u16()),
                        detail: format!("服务存活（{}）", url),
                    };
                }
                last_detail = format!("{} 返回 HTTP {}", url, status.as_u16());
            }
            Err(e) => {
                // 连接被拒绝最典型：服务未启动
                last_detail = if e.is_connect() {
                    format!("{} 连接被拒绝——服务未启动（可在「服务」页启动 Headroom）", url)
                } else if e.is_timeout() {
                    format!("{} 连接超时——服务无响应", url)
                } else {
                    format!("{} 请求失败: {}", url, e)
                };
            }
        }
    }
    HeadroomHealth {
        alive: false,
        base_url,
        path: None,
        status: None,
        detail: if last_detail.is_empty() {
            "未探测到可用端点".to_string()
        } else {
            last_detail
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_base_url_and_paths() {
        assert_eq!(headroom_base_url(8791), "http://127.0.0.1:8791");
        assert_eq!(headroom_health_paths()[0], "/readyz");
        assert_eq!(headroom_compress_path(), "/v1/compress");
    }

    /// 默认端口必须与内置服务项一致，否则 AI 开关探活会指向错误端口。
    #[test]
    fn test_default_port_matches_service_def() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../node-projects/headroom.json");
        let raw = std::fs::read_to_string(&path).expect("headroom.json 读取失败");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("headroom.json 解析失败");
        let port = value
            .get("defaultPort")
            .and_then(|v| v.as_u64())
            .expect("defaultPort 缺失") as u16;
        assert_eq!(port, HEADROOM_DEFAULT_PORT, "AI 侧默认端口与服务项不一致");
    }
}
