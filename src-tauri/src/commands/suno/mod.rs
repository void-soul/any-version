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
use base64::Engine;
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
    /// 封面直链（可选，有则下载并嵌入 mp3 ID3v2 封面）
    #[serde(default)]
    pub image_url: Option<String>,
}

/// 下载报告
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    pub succeeded: usize,
    /// 成功下载的歌曲 id（前端据此立即标记「已下载」，无需刷新页面）
    pub succeeded_ids: Vec<String>,
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
///
/// 结果缓存起来避免每次请求都读注册表，但**可失效**（见 [`invalidate_proxy_cache`]）：
/// 应用启动时代理往往还没就绪（mihomo 未起 / 端口未监听），此时若把地址永久固化，
/// 之后每次请求都会撞上这个死代理，且永远不会重试——表现就是「明明浏览器能打开，
/// 应用里一直报 `error sending request`」。
/// 系统代理判定缓存：外层 `Option` 表示「是否已探测」，内层是探测结果。
static PROXY_CACHE: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);

fn system_proxy() -> Option<String> {
    let mut guard = PROXY_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = guard.as_ref() {
        return cached.clone();
    }
    let proxy = read_system_proxy();
    match &proxy {
        Some(addr) => eprintln!("[Suno] 检测到系统代理: {addr}"),
        None => eprintln!("[Suno] 未检测到系统代理，将直连"),
    }
    *guard = Some(proxy.clone());
    proxy
}

/// 代理判定失效：下一次 [`system_proxy()`] 会重新读注册表。
///
/// 在「走了代理但网络层失败」时调用——代理地址可能已经变了（用户刚起/刚关 mihomo），
/// 固化旧值会让后续请求一直失败。
fn invalidate_proxy_cache() {
    *PROXY_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
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

/// 不带代理的客户端：作为「代理不可用」时的兜底路径。
///
/// 直连并非永远可行（部分网络环境必须走代理），但代理只是**可选加速路径**——
/// 代理挂了不该让整个功能变成「打不开」，所以两条路径都试。
fn direct_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .http1_only()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 构建带系统代理的 HTTP 客户端（首选路径）。
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

/// 把用户粘贴的地址归一化成 reqwest 能直接用的绝对 URL。
///
/// [`is_profile_url`] 故意宽松（允许 `suno.com/@user`、`http://`、`www.` 前缀），
/// 但 reqwest 只接受**绝对 URL** —— 缺协议会直接报 `relative URL without a base`，
/// 校验通过、请求却发不出去。这里统一补 `https://`，再把**真正要发出去的地址**打出来。
fn normalize_fetch_url(trimmed: &str) -> String {
    let s = trimmed.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else {
        format!("https://{s}")
    }
}

/// 解析 Suno 用户主页，返回歌曲列表。
#[tauri::command]
pub async fn suno_parse_profile(url: String) -> Result<Vec<SunoSong>, String> {
    let trimmed = url.trim();
    if !is_profile_url(trimmed) {
        return Err("只支持 Suno 用户主页（形如 https://suno.com/@用户名），不接受单曲页或其他链接".to_string());
    }
    let url = normalize_fetch_url(trimmed);
    eprintln!("[Suno] 用户输入: {trimmed}");
    eprintln!("[Suno] 实际请求: {url}");
    eprintln!("[Suno] 网络路径: {}", match system_proxy() {
        Some(p) => format!("系统代理 {p}（失败则回退直连）"),
        None => "直连（未检测到系统代理）".to_string(),
    });

    // 先走带代理的客户端；网络层失败时把代理判定作废并用直连重试一次。
    // 代理是可选加速路径，不是前置条件——固化一个没监听的代理端口会让功能整体不可用。
    let client = http_client();
    let resp = match client
        .get(&url)
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[Suno] 经系统代理请求失败（{e}），作废代理判定并改走直连重试");
            invalidate_proxy_cache();
            let fallback = direct_client();
            match fallback
                .get(&url)
                .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
                .send()
                .await
            {
                Ok(r) => r,
                Err(e2) => {
                    eprintln!("[Suno] 直连也失败: {e2}");
                    return Err(format!(
                        "请求主页失败（代理 {}；直连 {}）: {e2}",
                        e, e2
                    ));
                }
            }
        }
    };
    let status = resp.status();
    // 打 `resp.url()` 而不是请求串：重定向后的最终地址才是真正生效的那个
    eprintln!(
        "[Suno] 主页响应 HTTP {status}（协议 {:?}，最终地址 {}）",
        resp.version(),
        resp.url()
    );
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

    // 主页 SSR 只内嵌前 ~20 首，其余靠翻页接口补齐。
    // 翻页失败不当作错误：宁可少几首，也别让已经解析出来的结果全废掉。
    match extract_target_user_id(&html) {
        Some(uid) => match fetch_all_songs(&client, &uid).await {
            Ok(paged) if !paged.is_empty() => {
                eprintln!("[Suno] 翻页补齐 {} 首（接口）", paged.len());
                let mut seen: std::collections::HashSet<String> =
                    songs.iter().map(|s| s.id.clone()).collect();
                for song in paged {
                    if seen.insert(song.id.clone()) {
                        songs.push(song);
                    }
                }
            }
            Ok(_) => eprintln!("[Suno] 翻页接口没返回歌曲，沿用 SSR 结果"),
            Err(e) => eprintln!("[Suno] 翻页失败（沿用 SSR 的 {} 首）：{e}", songs.len()),
        },
        None => eprintln!("[Suno] 未从页面提取到 target_user_id，跳过翻页"),
    }

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
    let mut succeeded_ids: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    for (i, song) in songs.iter().enumerate() {
        match download_one(&app, song, &target, i + 1, total).await {
            Ok(()) => {
                succeeded += 1;
                succeeded_ids.push(song.id.clone());
                mark_downloaded(&song.id);
            }
            Err(e) => {
                eprintln!("[Suno] 下载失败 {}: {}", song.title, e);
                failed.push(format!("{}：{e}", song.title));
            }
        }
    }
    Ok(DownloadReport { succeeded, succeeded_ids, failed })
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
    let cover_tmp = std::env::temp_dir().join(format!("suno_{}_cover", song.id));

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

    // 封面可选：下载失败不阻断音频下载，只是不带封面
    let cover = match &song.image_url {
        Some(url) if !url.is_empty() => match download_file(url, &cover_tmp).await {
            Ok(()) if cover_tmp.is_file() => Some(cover_tmp.as_path()),
            _ => None,
        },
        _ => None,
    };

    let _ = app.emit(
        "suno-download-progress",
        DownloadProgress {
            current,
            total,
            title: song.title.clone(),
            stage: "transcode".to_string(),
        },
    );
    let out = dir.join(format!("{}.mp3", sanitize_filename(&song.title)));
    // 原子占位（避免并发下载同名的 TOCTOU）
    let out = unique_path(out)?;
    let result = transcode_to_mp3(&tmp, cover, &out);
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&cover_tmp);
    if result.is_err() {
        // 转码失败要把占位的 0 字节文件删掉，否则目录里留下残件
        let _ = std::fs::remove_file(&out);
    }
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

/// 用内置 ffmpeg 从 mp4 提音轨转成 320k mp3；`cover` 非空时嵌入 ID3v2 封面。
fn transcode_to_mp3(input: &Path, cover: Option<&Path>, output: &Path) -> Result<(), String> {
    let ffmpeg = utils::bin_tool_path("ffmpeg")
        .unwrap_or_else(|| utils::get_bin_dir().join("ffmpeg").join("ffmpeg.exe"));
    if !ffmpeg.is_file() {
        return Err(format!("未找到内置 ffmpeg（{}），请在设置的运行组件里安装", ffmpeg.display()));
    }
    eprintln!("[Suno] 转码中: {} -> {}（封面：{}）", input.display(), output.display(), cover.map(|c| c.display().to_string()).unwrap_or_else(|| "无".to_string()));

    let mut cmd = Command::new(&ffmpeg);
    // 禁止弹出命令提示符黑框（GUI 应用直接 spawn 控制台程序默认会建一个新窗口）
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    cmd.arg("-y").arg("-i").arg(input);
    if let Some(cover) = cover {
        cmd.arg("-i").arg(cover);
    }
    // 音频轨固定来自第一个输入（mp4）
    cmd.arg("-map").arg("0:a");
    if cover.is_some() {
        // 封面作为第二个输入的视频轨，原格式（jpeg/png）直接搬进 ID3v2 APIC
        cmd.arg("-map").arg("1:v")
            .arg("-c:v").arg("copy")
            .arg("-id3v2_version").arg("3")
            .arg("-metadata:s:v").arg("title=Album cover")
            .arg("-metadata:s:v").arg("comment=Cover (front)");
    } else {
        // `-vn` 去掉视频轨（源是 mp4，内嵌视频 + AAC 音频，只提音频转 mp3）
        cmd.arg("-vn");
    }
    cmd.arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("320k")
        .arg(output);

    let status = cmd
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

/// 目标文件已存在时，自动加 `V2`、`V3`… 序号后缀，避免覆盖。
///
/// 用 `V2` 而不是资源管理器默认的 `xxx (1)`：曲库里同一首歌常有多个版本（重生成 remix），
/// `(1)` 容易被当成文件名的一部分，`V2` 一眼看出是「同一首的第 2 版」。
///
/// 命中的序号从 2 起（原件本身算第 1 版），并**跳过已占用的号**：
/// 删掉 `V2` 后再下一次同名会补回 `V2`，不会跳到 `V4` 留空洞。
/// 原本就以 `V<数字>` 结尾的名字（`DemoV2`）不会被误当成已编号，
/// 因为编号是紧贴扩展名前追加的：`DemoV2.mp3` → `DemoV2V2.mp3`。
///
/// **原子占位**：返回的路径是当场用 `create_new(true)` 建出来的空文件（0 字节），
/// 而不是「查一下不存在就算这个名字归我」。存在性检查与占位分开做是标准的 TOCTOU——
/// 并发下载同一首歌时两边会算出同一个 `V2`，后写的把先写的静默覆盖。
/// 占位之后由调用方写内容（ffmpeg `-y` 直接覆盖这个空文件）；
/// **转码失败时调用方必须删掉占位文件**，否则目录里会留下 0 字节残件。
fn unique_path(path: PathBuf) -> Result<PathBuf, String> {
    use std::io::ErrorKind;

    /// 原子占位：文件已存在返回 `AlreadyExists`，其余错误视为真失败。
    fn try_reserve(candidate: &Path) -> Result<bool, std::io::Error> {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(candidate)
        {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        }
    }

    match try_reserve(&path) {
        Ok(true) => return Ok(path),
        Ok(false) => {}
        Err(e) => return Err(format!("创建文件失败 {}: {e}", path.display())),
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
    let mut n = 2usize;
    loop {
        let candidate = if ext.is_empty() {
            parent.join(format!("{stem}V{n}"))
        } else {
            parent.join(format!("{stem}V{n}.{ext}"))
        };
        match try_reserve(&candidate) {
            Ok(true) => return Ok(candidate),
            // 被占就接着往后找，因此删掉 V2 后仍会补回 V2、不会跳号
            Ok(false) => n += 1,
            Err(e) => return Err(format!("创建文件失败 {}: {e}", candidate.display())),
        }
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

// ─── 分页：把主页的全部歌曲拉完 ───

/// 翻页接口。主页 SSR 只内嵌前 ~20 首，其余要靠它按 cursor 拉。
///
/// 实测要点（都是踩出来的）：
/// - 域名是 `studio-api-prod`（连字符），不是 `studio-api.prod`；
/// - 方法 POST，路径 `/api/unified/feed`（**不是** `/api/feed/v3`，那个匿名一律 401）；
/// - 匿名可用，**不要带 cookie**（前端就是 `credentials: "omit"`）。
const UNIFIED_FEED_URL: &str = "https://studio-api-prod.suno.com/api/unified/feed";
/// 每页条数（接口上限就是 20）。
const PAGE_SIZE: usize = 20;
/// 总量兜底上限：正常主页几百首够用，防止接口返回异常 next_cursor 导致死循环。
const PAGE_MAX_ITEMS: usize = 2000;

/// 从主页 HTML 里取目标用户 id（翻页接口的 `target_user_id`）。
///
/// 必须用 `"v2Data":{"user_id":"` 锚定：页面里有二十多处 `user_id`（歌曲作者、点赞者…），
/// 取第一个会拿到错的用户。实测这个锚点反转义后唯一命中。
fn extract_target_user_id(html: &str) -> Option<String> {
    let unescaped = html.replace("\\\"", "\"");
    let anchor = "\"v2Data\":{\"user_id\":\"";
    let start = unescaped.find(anchor)? + anchor.len();
    let rest = unescaped.get(start..)?;
    let end = rest.find('"')?;
    let id = rest.get(..end)?;
    // 保守校验：只接受 UUID 形态，避免锚点附近结构变化时抓出半截字符串
    (id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4)
        .then(|| id.to_string())
}

/// `browser-token`：base64url(`{"timestamp":<毫秒>}`)，无 padding。
///
/// 这就是浏览器里那串 token 的**全部**内容——不含任何身份信息，纯粹是个时间戳凭证，
/// 所以匿名也能过（实测 200）。别以为它缺了什么登录态而去加 Cookie。
fn browser_token() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let payload = format!("{{\"timestamp\":{millis}}}");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
}

/// `device-id`：随机 UUID v4（本仓库没有 uuid crate，用 getrandom 自己拼）。
fn new_uuid_v4() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|e| format!("生成 device-id 失败: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

/// 拉一页，返回 `(本页歌曲, 下一页游标)`。游标为空串表示到底了。
async fn fetch_feed_page(
    client: &reqwest::Client,
    target_user_id: &str,
    device_id: &str,
    cursor: &str,
) -> Result<(Vec<SunoSong>, String), String> {
    let body = serde_json::json!({
        "feed_id": "user_songs",
        "cursor": cursor,
        "page_size": PAGE_SIZE,
        "request_metadata": { "sort_by": "created_at" },
        "target_user_id": target_user_id,
    });
    let resp = client
        .post(UNIFIED_FEED_URL)
        .header("accept", "*/*")
        // token 要手工拼成 {"token":"..."} 这个 JSON 字符串，不是裸 token
        .header("browser-token", format!("{{\"token\":\"{token}\"}}", token = browser_token()))
        .header("device-id", device_id)
        .header("referer", "https://suno.com/")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("拉取第 {cursor} 页失败: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        // 游标越过末页时接口回 404，属于正常终止而不是错误
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok((Vec::new(), String::new()));
        }
        return Err(format!("拉取第 {cursor} 页返回 HTTP {status}"));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析第 {cursor} 页失败: {e}"))?;

    let next_cursor = json
        .get("feed")
        .and_then(|f| f.get("next_cursor"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // items[].content_item 与主页 SSR 里的完全同构，直接复用同一个解析函数
    let songs = json
        .get("feed")
        .and_then(|f| f.get("items"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("content_item"))
                .filter_map(parse_content_item)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok((songs, next_cursor))
}

/// 逐页拉完整个主页（去重）。`Err` 只在第一页就失败时返回——翻页中途断掉就把已有的交出去。
async fn fetch_all_songs(
    client: &reqwest::Client,
    target_user_id: &str,
) -> Result<Vec<SunoSong>, String> {
    let device_id = new_uuid_v4()?;
    let mut all: Vec<SunoSong> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut cursor = String::new();

    loop {
        let (songs, next) = fetch_feed_page(client, target_user_id, &device_id, &cursor).await?;
        for song in songs {
            if seen.insert(song.id.clone()) {
                all.push(song);
            }
        }
        if next.is_empty() || all.len() >= PAGE_MAX_ITEMS {
            break;
        }
        cursor = next;
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::{
        browser_token, extract_target_user_id, is_profile_url, normalize_fetch_url,
        normalize_profile_url, parse_profile_html, sanitize_filename, unique_path,
    };
    use serde_json::json;

    /// 宽松校验通过后，必须补成绝对 URL —— 否则 reqwest 报 relative URL without a base。
    #[test]
    fn fetch_url_is_always_absolute() {
        assert_eq!(
            normalize_fetch_url("suno.com/@user"),
            "https://suno.com/@user"
        );
        assert_eq!(
            normalize_fetch_url("https://suno.com/@user"),
            "https://suno.com/@user"
        );
        // 已有 http 不改写（保持用户原意）
        assert_eq!(
            normalize_fetch_url("http://suno.com/@user"),
            "http://suno.com/@user"
        );
        // 校验通过的每种写法，归一化后都必须是绝对 URL
        for raw in [
            "https://suno.com/@u",
            "http://suno.com/@u",
            "suno.com/@u",
            "https://www.suno.com/@u",
        ] {
            assert!(is_profile_url(raw), "前提：{raw} 应通过校验");
            let out = normalize_fetch_url(raw);
            assert!(
                out.starts_with("http://") || out.starts_with("https://"),
                "归一化后必须是绝对 URL：{out}"
            );
        }
    }

    /// 分页凭证：`browser-token` 解开就是一个时间戳，没有身份信息。
    #[test]
    fn browser_token_is_base64url_timestamp() {
        use base64::Engine;
        let token = browser_token();
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token.as_bytes())
            .expect("token 必须是合法 base64url");
        let text = String::from_utf8(decoded).expect("解码后应是 UTF-8");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("应是 JSON");
        assert!(
            parsed.get("timestamp").and_then(|v| v.as_i64()).is_some(),
            "token 里应有 timestamp：{text}"
        );
        // 无 padding
        assert!(!token.contains('='), "不该带 padding：{token}");
    }

    /// `target_user_id` 只能取 v2Data 块里的那个，不能被其它 user_id 干扰。
    #[test]
    fn target_user_id_anchors_on_v2data() {
        let html = r#"..."v2Data\":{\"user_id\":\"1498d99f-3aae-4c41-ada4-6642d539a30d\",\"metadata\":{\"handle\":\"echoingpromoter3561"}}"#;
        assert_eq!(
            extract_target_user_id(html).as_deref(),
            Some("1498d99f-3aae-4c41-ada4-6642d539a30d")
        );
        // 前面出现的 song author user_id 不能干扰
        let html2 = r#"\"content_item\":{\"id\":\"x\",\"user_id\":\"00000000-0000-4000-8000-000000000000\"}"v2Data\":{\"user_id\":\"1498d99f-3aae-4c41-ada4-6642d539a30d\"}"#;
        assert_eq!(
            extract_target_user_id(html2).as_deref(),
            Some("1498d99f-3aae-4c41-ada4-6642d539a30d")
        );
        // 抓不到 / 格式异常时返回 None，不硬猜
        assert_eq!(extract_target_user_id("no user here"), None);
        assert_eq!(extract_target_user_id(r#"\"v2Data\":{\"user_id\":\"short\"}"#), None);
    }

    /// 翻页响应里的 content_item 与主页 SSR 同构，能被同一个解析函数吃下。
    #[test]
    fn parses_unified_feed_page_shape() {
        let page = json!({
            "feed": {
                "feed_id": "user_songs",
                "items": [
                    {"content_type": "clip", "content_item": {
                        "status": "complete", "title": "第一首",
                        "id": "clip-1", "video_url": "https://cdn1.suno.ai/clip-1.mp4",
                        "image_large_url": "https://cdn2.suno.ai/i1.jpeg", "play_count": 7
                    }},
                    {"content_type": "clip", "content_item": {
                        "status": "streaming", "title": "生成中", "id": "clip-2",
                        "video_url": "https://cdn1.suno.ai/clip-2.mp4"
                    }}
                ],
                "next_cursor": "20"
            }
        });
        let items = page["feed"]["items"].as_array().unwrap();
        let songs: Vec<_> = items
            .iter()
            .filter_map(|i| i.get("content_item"))
            .filter_map(super::parse_content_item)
            .collect();
        assert_eq!(songs.len(), 1, "草稿应被过滤");
        assert_eq!(songs[0].title, "第一首");
        assert_eq!(songs[0].audio_url, "https://cdn1.suno.ai/clip-1.mp4");
        assert_eq!(page["feed"]["next_cursor"].as_str(), Some("20"));
    }

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

    /// 重名时自动加 `V2`/`V3`… 后缀（不是资源管理器默认的 ` (1)`），且返回的路径是**原子占位**出来的。
    #[test]
    fn unique_path_adds_version_suffix() {
        // 目录名带 pid + 纳秒时间戳：测试二进制可能并行/重复运行同一用例，
        // 固定目录名会让彼此踩文件（表现为随机的 remove_file 失败）。
        let dir = temp_test_dir("suno_unique_test");
        let base = dir.join("我的歌.mp3");
        std::fs::write(&base, b"x").unwrap();

        // 原件 → V2
        let v2 = unique_path(base.clone()).unwrap();
        assert_eq!(v2.file_name().unwrap(), "我的歌V2.mp3");
        assert!(v2.exists(), "占位文件必须当场存在");

        // V2 也占用了 → V3
        std::fs::write(&v2, b"x").unwrap();
        let v3 = unique_path(base.clone()).unwrap();
        assert_eq!(v3.file_name().unwrap(), "我的歌V3.mp3");

        // 删掉 V2 后应补回 V2，不跳号留空洞
        std::fs::remove_file(&v2).unwrap();
        assert_eq!(
            unique_path(base.clone()).unwrap().file_name().unwrap(),
            "我的歌V2.mp3"
        );

        // 不存在的路径直接占位，不加工名字
        let fresh = dir.join("新歌.mp3");
        let got = unique_path(fresh.clone()).unwrap();
        assert_eq!(got, fresh);

        // 本身以 V2 结尾的名字不被误判：`DemoV2` → `DemoV2V2`
        let demo = dir.join("DemoV2.mp3");
        std::fs::write(&demo, b"x").unwrap();
        assert_eq!(
            unique_path(demo).unwrap().file_name().unwrap(),
            "DemoV2V2.mp3"
        );

        // 无扩展名时也带序号
        let noext = dir.join("裸文件");
        std::fs::write(&noext, b"x").unwrap();
        assert_eq!(unique_path(noext).unwrap().file_name().unwrap(), "裸文件V2");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 并发占位必须给出**不同**的名字——这正是改成 `create_new` 要解决的 TOCTOU。
    #[test]
    fn unique_path_is_race_free() {
        let dir = temp_test_dir("suno_race_test");
        let base = dir.join("同首歌.mp3");
        std::fs::write(&base, b"x").unwrap();

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let base = base.clone();
                std::thread::spawn(move || unique_path(base).unwrap())
            })
            .collect();
        let mut got: Vec<std::path::PathBuf> = handles
            .into_iter()
            .map(|h| h.join().expect("占位不应失败"))
            .collect();

        let total = got.len();
        got.sort();
        got.dedup();
        assert_eq!(got.len(), total, "并发占位出现了重复名字：{got:?}");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 建一个本用例独占的临时目录（pid + 纳秒，避免并行测试互踩）。
    fn temp_test_dir(prefix: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), stamp));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
