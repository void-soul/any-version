//! mihomo 内核变体检视与外部面板下载
//!
//! 内核不再由程序联网安装：稳定版 / alpha 预览版 / smart 智能版三个二进制
//! 已随程序预置在内核目录（bin/mihomo）中，这里只负责列出它们的状态与版本。

use crate::commands::hidden_cmd::hidden_cmd;
use crate::commands::mihomo::config::*;
use crate::commands::mihomo::manager::{core_dir, core_file_name};
use crate::commands::mihomo::MihomoState;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tauri::State;

const GITHUB_PROXIES: [&str; 4] = [
    "https://gh-proxy.org",
    "https://ghfast.top",
    "https://down.clashparty.org",
    "https://download.mihomo.party",
];

fn build_download_urls(url: &str, proxy_pref: &str) -> Vec<String> {
    if proxy_pref == "direct" {
        return vec![url.to_string()];
    }
    if !proxy_pref.is_empty() && proxy_pref != "auto" {
        return vec![format!("{}/{}", proxy_pref.trim_end_matches('/'), url)];
    }
    let mut v: Vec<String> = GITHUB_PROXIES
        .iter()
        .map(|p| format!("{}/{}", p, url))
        .collect();
    v.push(url.to_string());
    v
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_default()
}

pub async fn download_with_proxy(url: &str, proxy_pref: &str) -> Result<Vec<u8>, String> {
    let mut last = String::from("下载失败");
    for candidate in build_download_urls(url, proxy_pref) {
        match client()
            .get(&candidate)
            .header("User-Agent", "any-version")
            .send()
            .await
        {
            Ok(resp) => {
                if !resp.status().is_success() {
                    last = format!("{} -> HTTP {}", candidate, resp.status());
                    continue;
                }
                match resp.bytes().await {
                    Ok(b) if b.len() > 1024 => return Ok(b.to_vec()),
                    Ok(_) => last = format!("{} -> 响应过小", candidate),
                    Err(e) => last = format!("{} -> {}", candidate, e),
                }
            }
            Err(e) => last = format!("{} -> {}", candidate, e),
        }
    }
    Err(last)
}

fn exe_version(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let out = hidden_cmd(path).arg("-v").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let s = if s.trim().is_empty() {
        String::from_utf8_lossy(&out.stderr).to_string()
    } else {
        s
    };
    let first = s.lines().next()?.trim().to_string();
    if first.is_empty() {
        None
    } else {
        Some(first)
    }
}

/// 列出各内核变体的就绪状态与版本（内核由程序预置，不联网安装）
#[tauri::command]
pub fn mihomo_core_variants(state: State<'_, MihomoState>) -> Value {
    let app_config = state.app_config.lock().unwrap().clone();
    let dir = core_dir(&app_config);
    let mut arr = Vec::new();
    for v in ["mihomo", "mihomo-alpha", "mihomo-smart"] {
        let p = dir.join(core_file_name(v));
        arr.push(serde_json::json!({
            "id": v,
            "path": p.to_string_lossy(),
            "installed": p.exists(),
            "version": exe_version(&p),
        }));
    }
    serde_json::json!({ "dir": dir.to_string_lossy(), "items": arr })
}

/// 下载外部控制面板（zashboard）到 workdir/ui
#[tauri::command]
pub async fn mihomo_download_ui(state: State<'_, MihomoState>) -> Result<String, String> {
    let (pref, url, dir) = {
        let app = state.app_config.lock().unwrap();
        let pref = app
            .extra
            .get("githubProxy")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ctl = state.controled_config.lock().unwrap();
        let url = ctl
            .get("external-ui-url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://github.com/Zephyruso/zashboard/releases/latest/download/dist.zip")
            .to_string();
        (pref, url, state.data_dir.join("ui"))
    };
    let data = download_with_proxy(&url, &pref).await?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cursor = std::io::Cursor::new(&data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("解压失败: {e}"))?;
    // zashboard dist.zip 内含顶层 dist/ 目录，需要剥离
    let mut strip = true;
    for i in 0..archive.len() {
        let f = archive.by_index(i).map_err(|e| e.to_string())?;
        let n = f.name().to_string();
        if !n.starts_with("dist/") && !n.starts_with("dist\\") {
            strip = false;
            break;
        }
    }
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().replace('\\', "/");
        if name.ends_with('/') {
            continue;
        }
        let rel = if strip {
            name.splitn(2, '/').nth(1).unwrap_or(&name).to_string()
        } else {
            name.clone()
        };
        if rel.is_empty() || rel.contains("..") {
            continue;
        }
        let out = dir.join(&rel);
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).ok();
        }
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).ok();
        std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
    }
    Ok(format!("面板已下载至 {}", dir.display()))
}

/// 供 mod.rs 复用：AppConfig 里读取 core 变体
pub fn selected_core(app: &AppConfig) -> String {
    app.extra
        .get("core")
        .and_then(|v| v.as_str())
        .unwrap_or("mihomo")
        .to_string()
}
