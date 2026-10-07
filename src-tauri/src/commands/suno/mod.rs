//! Suno 音频下载：解析用户主页（匿名，无需 Cookie）→ 提取歌曲 → 下载 m4a → ffmpeg 转 mp3。
//!
//! 技术要点（已实测）：
//! - Suno 主页是 Next.js SSR，歌曲数据内嵌在 HTML 的 RSC payload 里（JSON 转义形式）。
//! - 真实音频 = `media_urls[0].url`（CloudFront 的 `https://d2lwuy8qc234o3.cloudfront.net/1/clip/<id>.m4a`，
//!   是 m4a 容器 + opus 编码）；匿名时 `audio_url` 是 `.../api/forbidden` 占位，不可用。
//! - 转 mp3 复用 kira 内置 ffmpeg（`utils::bin_tool_path("ffmpeg")`），固定 320k。

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use winreg::enums::*;
use winreg::RegKey;

use crate::commands::utils;

/// 一首歌（解析结果，返回给前端展示）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SunoSong {
    pub id: String,
    pub title: String,
    /// 歌曲视频直链（`video_url`，mp4 内嵌 AAC 音频轨；`media_urls[0]` 的 m4a 是 DRM 加密的不可用）
    pub audio_url: String,
    pub image_url: Option<String>,
    pub play_count: i64,
    /// 是否已下载过（本地缓存记录，供「全选未下载」）
    pub downloaded: bool,
}

/// 前端勾选下载时传的引用（够下载即可）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRef {
    pub id: String,
    pub title: String,
    pub audio_url: String,
}

/// 下载报告
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    pub succeeded: usize,
    pub failed: Vec<String>,
}

/// 逐首下载进度事件（前端 listen `suno-download-progress`）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    /// 当前第几首（整体进度）
    pub current: usize,
    pub total: usize,
    pub title: String,
    /// 当前阶段：`download`（下载中）/ `transcode`（转码中）
    pub stage: String,
}

/// 浏览器 UA：Suno 对非浏览器 UA 可能返回不同的内容，统一伪装成桌面浏览器。
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// ─── 已下载缓存 ───

/// 已下载记录缓存：`<data_dir>/suno/downloaded.json`，存一个 id 数组。
fn downloaded_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("suno").join("downloaded.json")
}

fn load_downloaded() -> std::collections::HashSet<String> {
    let Ok(content) = std::fs::read_to_string(downloaded_path()) else {
        return std::collections::HashSet::new();
    };
    serde_json::from_str::<Vec<String>>(&content)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn mark_downloaded(id: &str) {
    let mut set = load_downloaded();
    if set.contains(id) {
        return;
    }
    set.insert(id.to_string());
    let path = downloaded_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut ids: Vec<String> = set.into_iter().collect();
    ids.sort();
    if let Ok(json) = serde_json::to_string_pretty(&ids) {
        if let Err(e) = std::fs::write(&path, json) {
            eprintln!("[Suno] 写入已下载缓存失败: {e}");
        }
    }
}

// ─── 主页收藏 ───

/// 只认 Suno 用户主页：`https://suno.com/@username`（支持 www 前缀、无协议、http）。
fn is_profile_url(url: &str) -> bool {
    let lower = url.trim().to_lowercase();
    let no_proto = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let (host, rest) = match no_proto.split_once('/') {
        Some((h, r)) => (h, r),
        None => (no_proto, ""),
    };
    let host_ok = host == "suno.com" || host == "www.suno.com";
    let name = rest.trim_start_matches('/').trim_end_matches('/');
    host_ok && name.starts_with('@') && name.len() > 1
}

/// 归一化主页 URL（去尾斜杠、统一小写），用于收藏去重。
fn normalize_profile_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_lowercase()
}

fn profiles_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("suno").join("profiles.json")
}

fn load_profiles() -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(profiles_path()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&content).unwrap_or_default()
}

fn save_profiles(profiles: &[String]) -> Result<(), String> {
    let path = profiles_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(profiles).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写入收藏失败: {e}"))
}

/// 读 Windows 系统代理（mihomo 固定端口模式），返回 `http://127.0.0.1:port`。
///
/// 项目全局的 `utils::get_http_client()` 没有启用 reqwest 的 `system-proxy` feature，
/// 因此默认**不读系统代理**——Suno 这类需要走代理的域名会直接连失败。这里读注册表
/// 把系统代理交给 reqwest。未启用 / 空则返回 None（PAC 模式暂不支持，直连兜底）。
/// 结果用 OnceLock 缓存，代理判定日志只打一次。
fn system_proxy() -> Option<String> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            let proxy = read_system_proxy();
            match &proxy {
                Some(addr) => eprintln!("[Suno] 检测到系统代理: {addr}"),
                None => eprintln!("[Suno] 未检测到系统代理，将直连"),
            }
            proxy
        })
        .clone()
}

fn read_system_proxy() -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
            KEY_READ,
        )
        .ok()?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return None;
    }
    let server: String = key.get_value("ProxyServer").unwrap_or_default();
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    Some(format!("http://{server}"))
}

/// 构建带系统代理的 HTTP 客户端。
fn http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        // Suno 对 HTTP/2（reqwest/hyper 默认协商）与 HTTP/1.1 返回的 SSR 内容不同：
        // 实测 HTTP/1.1 才带歌曲数据，故强制 HTTP/1.1。
        .http1_only()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120));
    if let Some(proxy) = system_proxy() {
        if let Ok(p) = reqwest::Proxy::all(&proxy) {
            builder = builder.proxy(p);
        } else {
            eprintln!("[Suno] 代理地址解析失败，退回直连: {proxy}");
        }
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

/// 解析 Suno 用户主页，返回歌曲列表。
#[tauri::command]
pub async fn suno_parse_profile(url: String) -> Result<Vec<SunoSong>, String> {
    let trimmed = url.trim();
    if !is_profile_url(trimmed) {
        return Err("只支持 Suno 用户主页（形如 https://suno.com/@用户名），不接受单曲页或其他链接".to_string());
    }
    let url = trimmed.to_string();
    eprintln!("[Suno] 解析主页: {url}");
    let client = http_client();
    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
        .send()
        .await
        .map_err(|e| {
            eprintln!("[Suno] 请求主页失败: {e}");
            format!("请求主页失败: {e}")
        })?;
    let status = resp.status();
    eprintln!("[Suno] 主页响应 HTTP {status}（版本 {:?}）", resp.version());
    if !status.is_success() {
        return Err(format!("主页返回 HTTP {status}"));
    }
    let html = resp.text().await.map_err(|e| format!("读取主页失败: {e}"))?;
    eprintln!("[Suno] 主页 HTML 大小 {} 字节", html.len());
    eprintln!("[Suno] 含 content_item: {}", html.contains("content_item"));
    eprintln!("[Suno] 含 media_urls: {}", html.contains("media_urls"));
    eprintln!("[Suno] 含 .m4a: {}", html.contains(".m4a"));
    // 完整 HTML 落到临时文件，便于排查「页面结构变化」
    let debug_path = std::env::temp_dir().join("suno_debug.html");
    if std::fs::write(&debug_path, &html).is_ok() {
        eprintln!("[Suno] 已保存 HTML 到 {}", debug_path.display());
    }
    let head: String = html.chars().take(800).collect();
    eprintln!("[Suno] HTML 开头 800 字符:\n{head}");
    let mut songs = parse_profile_html(&html)?;
    // 标记已下载（本地缓存）
    let downloaded = load_downloaded();
    for s in songs.iter_mut() {
        s.downloaded = downloaded.contains(&s.id);
    }
    eprintln!(
        "[Suno] 解析到 {} 首，其中已下载 {} 首",
        songs.len(),
        songs.iter().filter(|s| s.downloaded).count()
    );
    Ok(songs)
}

/// 列出收藏的主页。
#[tauri::command]
pub fn suno_list_profiles() -> Vec<String> {
    load_profiles()
}

/// 收藏一个主页（去重），返回更新后的收藏列表。
#[tauri::command]
pub fn suno_save_profile(url: String) -> Result<Vec<String>, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("主页 URL 不能为空".to_string());
    }
    let normalized = normalize_profile_url(&url);
    let mut profiles = load_profiles();
    if !profiles.iter().any(|p| normalize_profile_url(p) == normalized) {
        profiles.push(url);
        save_profiles(&profiles)?;
    }
    Ok(profiles)
}

/// 取消收藏一个主页，返回更新后的收藏列表。
#[tauri::command]
pub fn suno_remove_profile(url: String) -> Result<Vec<String>, String> {
    let normalized = normalize_profile_url(&url);
    let mut profiles = load_profiles();
    profiles.retain(|p| normalize_profile_url(p) != normalized);
    save_profiles(&profiles)?;
    Ok(profiles)
}

/// 下载并转码一批歌曲。
#[tauri::command]
pub async fn suno_download_songs(
    app: tauri::AppHandle,
    songs: Vec<DownloadRef>,
    dir: String,
) -> Result<DownloadReport, String> {
    if songs.is_empty() {
        return Err("没有要下载的歌曲".to_string());
    }
    let target = PathBuf::from(&dir);
    std::fs::create_dir_all(&target).map_err(|e| format!("创建目录失败: {e}"))?;

    let total = songs.len();
    let mut succeeded = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for (i, song) in songs.iter().enumerate() {
        match download_one(&app, song, &target, i + 1, total).await {
            Ok(()) => {
                succeeded += 1;
                mark_downloaded(&song.id);
            }
            Err(e) => {
                eprintln!("[Suno] 下载失败 {}: {}", song.title, e);
                failed.push(format!("{}：{e}", song.title));
            }
        }
    }
    Ok(DownloadReport { succeeded, failed })
}

/// 下载一首：mp4 落临时文件 → ffmpeg 提音轨转 mp3 → 清理临时文件。
async fn download_one(
    app: &tauri::AppHandle,
    song: &DownloadRef,
    dir: &Path,
    current: usize,
    total: usize,
) -> Result<(), String> {
    let tmp = std::env::temp_dir().join(format!("suno_{}.mp4", song.id));

    let _ = app.emit(
        "suno-download-progress",
        DownloadProgress {
            current,
            total,
            title: song.title.clone(),
            stage: "download".to_string(),
        },
    );
    download_file(&song.audio_url, &tmp).await?;

    let _ = app.emit(
        "suno-download-progress",
        DownloadProgress {
            current,
            total,
            title: song.title.clone(),
            stage: "transcode".to_string(),
        },
    );
    let mut out = dir.join(format!("{}.mp3", sanitize_filename(&song.title)));
    out = unique_path(out);
    let result = transcode_to_mp3(&tmp, &out);
    let _ = std::fs::remove_file(&tmp);
    result
}

async fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let client = http_client();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| {
            eprintln!("[Suno] 下载失败 {url}: {e}");
            format!("下载失败: {e}")
        })?;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    if !status.is_success() {
        eprintln!("[Suno] 下载返回 HTTP {status}，content-type={content_type}");
        return Err(format!("下载返回 HTTP {status}"));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("读取失败: {e}"))?;
    eprintln!(
        "[Suno] 下载完成 {url}：{} 字节，content-type={content_type}",
        bytes.len()
    );
    std::fs::write(dest, &bytes).map_err(|e| format!("写入临时文件失败: {e}"))?;
    Ok(())
}

/// 用内置 ffmpeg 从 mp4 提音轨转成 320k mp3。
fn transcode_to_mp3(input: &Path, output: &Path) -> Result<(), String> {
    let ffmpeg = utils::bin_tool_path("ffmpeg")
        .unwrap_or_else(|| utils::get_bin_dir().join("ffmpeg").join("ffmpeg.exe"));
    if !ffmpeg.is_file() {
        return Err(format!("未找到内置 ffmpeg（{}），请在设置的运行组件里安装", ffmpeg.display()));
    }
    eprintln!("[Suno] 转码中: {} -> {}", input.display(), output.display());
    // `-vn` 去掉视频轨（源是 mp4，内嵌视频 + AAC 音频，只提音频转 mp3）
    let status = Command::new(&ffmpeg)
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-vn")
        .arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("320k")
        .arg(output)
        .status()
        .map_err(|e| format!("启动 ffmpeg 失败: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg 转码失败（exit {:?}）", status.code()));
    }
    Ok(())
}

/// 文件名清理：去掉 Windows 非法字符与换行。
fn sanitize_filename(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\r' | '\n' | '\t' => '_',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed
    }
}

/// 目标文件已存在时，自动加 `(1)`、`(2)`… 后缀，避免覆盖。
fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new(""));
    let mut n = 1usize;
    loop {
        let candidate = if ext.is_empty() {
            parent.join(format!("{stem} ({n})"))
        } else {
            parent.join(format!("{stem} ({n}).{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

// ─── HTML 解析（RSC payload） ───

/// 从主页 HTML 提取全部歌曲。
///
/// 歌曲数据内嵌在 RSC payload 里，形如 `...\"content_item\":{\"status\":\"complete\",...}`。
/// 策略：先把 `\"` 反转义成 `"`，再用括号配对把每个 `content_item` 的 JSON 对象整段抠出来
/// 交给 `serde_json` 解析——不依赖整页是合法 JSON（RSC 流不是）。
pub fn parse_profile_html(html: &str) -> Result<Vec<SunoSong>, String> {
    // 反转义 JSON 引号：RSC payload 里的字符串是 `\"` 转义的
    let unescaped = html.replace("\\\"", "\"");
    let mut songs: Vec<SunoSong> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut search_from = 0usize;
    while let Some(pos) = find_content_item_brace(&unescaped, search_from) {
        let Some(obj) = extract_json_object(&unescaped, pos) else {
            search_from = pos + 1;
            continue;
        };
        // 只前进一个字符，不跳过整段对象：`content_item` 可能是 feed 容器
        // （内层 `items[]` 里还嵌套每首歌自己的 `content_item`），跳过去会漏掉全部歌曲。
        search_from = pos + 1;

        let Ok(value) = serde_json::from_str::<serde_json::Value>(obj) else {
            continue;
        };
        if let Some(song) = parse_content_item(&value) {
            if seen.insert(song.id.clone()) {
                songs.push(song);
            }
        }
    }

    if songs.is_empty() {
        return Err("没有解析到任何歌曲（页面结构可能已变化，或该主页没有公开歌曲）".to_string());
    }
    Ok(songs)
}

/// 在文本里找 `"content_item":{` 中 `{` 的下标。
fn find_content_item_brace(text: &str, from: usize) -> Option<usize> {
    let tail = &text[from..];
    let marker = "\"content_item\":";
    let rel = tail.find(marker)?;
    let mut idx = from + rel + marker.len();
    // 跳过空白，定位到 `{`
    while idx < text.len() {
        let c = text.as_bytes()[idx] as char;
        if c == '{' {
            return Some(idx);
        }
        if c == ' ' || c == '\n' || c == '\r' || c == '\t' {
            idx += 1;
        } else {
            break;
        }
    }
    None
}

/// 从 `{` 开始提取配对的 JSON 对象子串（处理字符串里的 `{}` 与转义）。
fn extract_json_object(text: &str, start: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for i in start..text.len() {
        let c = bytes[i] as char;
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else {
            match c {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&text[start..=i]);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// 从 `content_item` 对象里提取歌曲字段（`status != complete` 的草稿/生成中跳过）。
fn parse_content_item(item: &serde_json::Value) -> Option<SunoSong> {
    let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "complete" {
        return None;
    }
    let id = item.get("id")?.as_str()?.to_string();
    let title = item
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Untitled")
        .to_string();
    // 注意：`media_urls[0].url` 的 m4a 是加密的（DRM，头部是密文、无 ftyp/moov，ffmpeg 打不开）；
    // `audio_url` 匿名时是 forbidden 占位。真正可用的是 `video_url`（mp4，内嵌 AAC 音频轨），
    // 下载后用 ffmpeg `-vn` 提音轨转 mp3。
    let audio_url = item.get("video_url")?.as_str()?.to_string();
    let image_url = item
        .get("image_large_url")
        .or_else(|| item.get("image_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let play_count = item.get("play_count").and_then(|v| v.as_i64()).unwrap_or(0);
    Some(SunoSong {
        id,
        title,
        audio_url,
        image_url,
        play_count,
        downloaded: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{is_profile_url, normalize_profile_url, parse_profile_html, sanitize_filename, unique_path};

    #[test]
    fn profile_url_only_accepts_user_homepage() {
        assert!(is_profile_url("https://suno.com/@echoingpromoter3561"));
        assert!(is_profile_url("http://suno.com/@user"));
        assert!(is_profile_url("suno.com/@user"));
        assert!(is_profile_url("https://www.suno.com/@user"));
        assert!(is_profile_url("https://suno.com/@user/"));
        assert!(!is_profile_url("https://suno.com/song/abc-123"));
        assert!(!is_profile_url("https://suno.com/@"));
        assert!(!is_profile_url("https://example.com/@user"));
        assert!(!is_profile_url("https://suno.com/"));
        assert_eq!(normalize_profile_url("https://suno.com/@User/"), "https://suno.com/@user");
    }

    #[test]
    fn parses_songs_from_rsc_payload() {
        // 模拟 RSC payload 里的转义片段（两首歌，一首 complete、一首草稿）
        let html = r#"<script>...\"items\":[{\"content_type\":\"clip\",\"content_item\":{\"status\":\"complete\",\"title\":\"我的歌\",\"play_count\":12,\"id\":\"abc-123\",\"entity_type\":\"song_schema\",\"video_url\":\"https://cdn1.suno.ai/abc-123.mp4\",\"media_urls\":[{\"url\":\"https://d2lwuy8qc234o3.cloudfront.net/1/clip/abc-123.m4a\",\"content_type\":\"m4a-opus\"}],\"image_url\":\"https://cdn2.suno.ai/image_abc-123.jpeg\"}},{\"content_type\":\"clip\",\"content_item\":{\"status\":\"streaming\",\"title\":\"生成中\",\"id\":\"draft-1\",\"media_urls\":[{\"url\":\"https://x.m4a\"}]}}]..."#;
        let songs = parse_profile_html(html).unwrap();
        assert_eq!(songs.len(), 1, "草稿应被过滤");
        assert_eq!(songs[0].title, "我的歌");
        assert_eq!(songs[0].id, "abc-123");
        assert_eq!(songs[0].audio_url, "https://cdn1.suno.ai/abc-123.mp4");
        assert_eq!(songs[0].play_count, 12);
        assert!(songs[0].image_url.as_deref().unwrap().contains("image_abc-123"));
    }

    #[test]
    fn dedupes_by_song_id() {
        let html = r#"{\"content_item\":{\"status\":\"complete\",\"title\":\"A\",\"id\":\"same-1\",\"video_url\":\"https://x/same-1.mp4\"}}{\"content_item\":{\"status\":\"complete\",\"title\":\"A\",\"id\":\"same-1\",\"video_url\":\"https://x/same-1.mp4\"}}"#;
        let songs = parse_profile_html(html).unwrap();
        assert_eq!(songs.len(), 1);
    }

    /// 真实结构：外层 `content_item` 是 feed 容器，歌曲嵌套在 `items[].content_item` 里。
    /// 解析必须穿透 feed 容器，把内层歌曲都取出来（否则只抓到 feed 外壳、漏掉全部歌曲）。
    #[test]
    fn parses_songs_nested_in_feed() {
        let html = r#"\"content_item\":{\"feed_id\":\"user_pinned_songs\",\"feed_container_type\":\"synthetic_playlist\",\"feed_title\":\"Pinned\",\"items\":[{\"content_type\":\"clip\",\"content_item\":{\"status\":\"complete\",\"title\":\"我的歌\",\"play_count\":12,\"id\":\"abc-123\",\"video_url\":\"https://cdn1.suno.ai/abc-123.mp4\",\"image_url\":\"https://cdn2.suno.ai/image_abc-123.jpeg\"}},{\"content_type\":\"clip\",\"content_item\":{\"status\":\"complete\",\"title\":\"第二首\",\"id\":\"def-456\",\"video_url\":\"https://cdn1.suno.ai/def-456.mp4\"}}]}"#;
        let songs = parse_profile_html(html).unwrap();
        assert_eq!(songs.len(), 2, "应穿透 feed 容器解析出两首歌");
        assert_eq!(songs[0].title, "我的歌");
        assert_eq!(songs[0].id, "abc-123");
        assert_eq!(songs[1].title, "第二首");
        assert_eq!(songs[1].id, "def-456");
    }

    #[test]
    fn sanitizes_illegal_chars() {
        assert_eq!(sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize_filename("  "), "Untitled");
        assert_eq!(sanitize_filename("标题 123"), "标题 123");
    }

    #[test]
    fn unique_path_adds_suffix() {
        let dir = std::env::temp_dir().join("suno_unique_test");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("x.mp3");
        std::fs::write(&base, b"x").unwrap();
        let p = unique_path(base.clone());
        assert_ne!(p, base);
        assert!(p.to_string_lossy().contains("(1)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
