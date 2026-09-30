//! 在线音源的取流 / 播放 / 下载。
//!
//! **播放只有一条路径：本地文件。** 插件给出的是一对「远程 URL + 请求头」，
//! 这里先把它落到磁盘再交给现有播放引擎，于是队列 / 均衡器 / 切歌推进 / 进度条
//! 全部原样复用 —— 不需要为在线源再造一套播放器，也顺带把「听过的歌」缓存了下来。
//!
//! - 「听」→ 下载到 `music/cache/online/`（可清理；重复播放**完全不联网**）
//! - 「下载」→ 落到用户设置的下载目录，并**自动登记进曲库**（下完即可见、可播、可重命名）

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use super::player::{MusicPlayerState, OnlineOrigin, PendingOnline, PlayerState};
use super::queue::QueueItem;
use super::{library, plugin_host, plugin_registry, settings as music_settings};

/// 取流超时（插件内部可能要先请求一次接口才拿到直链）
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);
/// 下载进度事件名
pub const DOWNLOAD_PROGRESS_EVENT: &str = "music-plugin-download-progress";
/// 进度上报的最小间隔，避免大文件把事件刷爆
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

/// 音频请求的兜底 UA。
///
/// 插件没给 `userAgent` 时用它：音源 CDN 普遍会拦截不认识的 UA，
/// 而 MusicFree 插件里写的请求头基本都是桌面浏览器形态。
const FALLBACK_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36";

/// 允许的音频扩展名（与曲库扫描支持的范围一致）。
const AUDIO_EXTS: [&str; 8] = ["mp3", "flac", "wav", "m4a", "mp4", "aac", "ogg", "opus"];

// ─── 目录 ───

/// 用户设置的下载目录；未设置时为 `data_dir/music/downloads`。
pub fn download_dir() -> PathBuf {
    let configured = music_settings::load_settings().download_dir;
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        library::music_dir().join("downloads")
    } else {
        PathBuf::from(trimmed)
    }
}

/// 在线听歌的缓存目录（可整体清理）。
pub fn cache_dir() -> PathBuf {
    library::music_dir().join("cache").join("online")
}

// ─── 纯函数：命名与解析 ───

/// 把「歌手 - 标题」变成安全文件名（Windows 非法字符与控制字符一律换掉）。
pub fn safe_file_stem(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        return "未命名".to_string();
    }
    // 文件名过长会让某些文件系统直接失败；留足后缀与重名序号的余量
    trimmed.chars().take(120).collect()
}

/// 从直链推断扩展名；不在白名单里就按 mp3 处理。
///
/// 只影响**文件名的可读性**：解码是 symphonia 按内容嗅探的，扩展名写错照样能播。
pub fn extension_for(url: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let ext = without_query
        .rsplit('/')
        .next()
        .and_then(|last| last.rsplit_once('.').map(|(_, ext)| ext))
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if AUDIO_EXTS.contains(&ext.as_str()) {
        ext
    } else {
        "mp3".to_string()
    }
}

/// 缓存文件的**主名**（不含扩展名）。
///
/// 用「插件 + 曲目 id + 音质」做哈希，而不是标题：同一首歌重复播放要命中同一份，
/// 而不同音质必须是不同文件（否则换了音质还在放旧的）。
///
/// 刻意不把扩展名算进来：扩展名要等取流后才知道（见 [`extension_for`]），
/// 而缓存命中判定发生在取流**之前** —— 两者混在一起会导致「永远命中不了」。
pub fn cache_stem(file: &str, item: &Value, quality: &str) -> String {
    use std::hash::{Hash, Hasher};
    let identity = item
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| item.get("title").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    file.hash(&mut hasher);
    identity.hash(&mut hasher);
    quality.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// 缓存主名对应的已有文件（逐个试白名单扩展名）。
///
/// 有了它，「重复播放」可以**完全不联网**（连取流都跳过）—— 直链大多有时效，
/// 能不走网络就不要走。
pub fn find_cached(stem: &str) -> Option<PathBuf> {
    let dir = cache_dir();
    for ext in AUDIO_EXTS {
        let candidate = dir.join(format!("{stem}.{ext}"));
        if std::fs::metadata(&candidate).map(|meta| meta.len() > 0).unwrap_or(false) {
            return Some(candidate);
        }
    }
    None
}

/// 解析插件 `getMediaSource` 的返回值。
///
/// 「插件没给地址」（该音质不可用 / 需要会员）与「返回值结构不对」（插件坏了）
/// 要分开说 —— 这两种提示对用户的意义完全不同。
pub fn parse_resolved_source(file: &str, value: &Value) -> Result<ResolvedSource, String> {
    let unavailable = || format!("{file} 没有返回播放地址（该音质可能不可用或需要登录）");
    if value.is_null() {
        return Err(unavailable());
    }
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if url.is_empty() {
        return Err(unavailable());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("{file} 返回的地址不是 http(s) 链接: {url}"));
    }
    let headers = value
        .get("headers")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Ok(ResolvedSource {
        url,
        headers,
        user_agent: value
            .get("userAgent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
        quality: value
            .get("quality")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

// ─── 数据类型 ───

/// 插件给出的播放来源。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolvedSource {
    pub url: String,
    /// 插件要求的请求头（防盗链等），必须原样带上，否则拿到 403
    pub headers: Vec<(String, String)>,
    pub user_agent: String,
    /// 插件实际给出的音质（可能与我们请求的不同）
    pub quality: String,
}

#[derive(Debug, Serialize)]
pub struct PluginPlayOutcome {
    /// 实际播放的本地文件
    pub path: String,
    /// 是否命中缓存（true = 秒开，没走网络）
    pub from_cache: bool,
    pub title: String,
    pub artist: String,
    /// 命中缓存时为 `None`：那次播放根本没取流（直链有时效，存下来也没意义）
    pub source: Option<ResolvedSource>,
    pub player: PlayerState,
}

#[derive(Debug, Serialize)]
pub struct PluginDownloadOutcome {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub bytes: u64,
    pub source: ResolvedSource,
    /// 下载目录（前端提示「已保存到」）
    pub dir: String,
    /// 登记后的曲库（前端一次调用即可刷新列表）
    pub library: library::MusicLibrary,
}

// ─── 取流 ───

/// 调插件拿直链（阻塞，进 spawn_blocking）。
fn resolve_blocking(file: &str, item: &Value, quality: &str) -> Result<ResolvedSource, String> {
    let path = plugin_registry::resolve_script_path(file)?;
    let params = json!({ "item": item, "quality": quality });
    let value = plugin_host::call_plugin(&path, "mediaSource", params, RESOLVE_TIMEOUT)?;
    parse_resolved_source(file, &value)
}

async fn resolve_source(
    file: String,
    item: Value,
    quality: String,
) -> Result<ResolvedSource, String> {
    tauri::async_runtime::spawn_blocking(move || resolve_blocking(&file, &item, &quality))
        .await
        .map_err(|e| format!("取流任务失败: {e}"))?
}

// ─── 下载 ───

/// 流式下载到 `dest`。
///
/// 先写 `.part` 再改名：中途失败不会在曲库目录里留下半截音频 ——
/// 那种文件会被曲库扫到，用户拿到的是一个能点、点开就报错的条目。
async fn download_to(
    app: &AppHandle,
    source: &ResolvedSource,
    dest: &Path,
    label: &str,
) -> Result<u64, String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    let part = dest.with_extension("part");

    let mut request = crate::commands::utils::get_http_client().get(&source.url);
    for (key, value) in &source.headers {
        request = request.header(key.as_str(), value.as_str());
    }
    request = request.header(
        "User-Agent",
        if source.user_agent.is_empty() {
            FALLBACK_USER_AGENT
        } else {
            source.user_agent.as_str()
        },
    );

    let response = request.send().await.map_err(|e| format!("请求音频失败: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("音源返回 {status}（直链可能已过期，重试即可）"));
    }
    let total = response.content_length().unwrap_or(0);

    let mut file =
        std::fs::File::create(&part).map_err(|e| format!("创建 {} 失败: {e}", part.display()))?;
    let mut stream = response.bytes_stream();
    let mut received: u64 = 0;
    let mut last_emit = Instant::now();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载中断: {e}"))?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| format!("写入失败: {e}"))?;
        received += chunk.len() as u64;
        if last_emit.elapsed() >= PROGRESS_INTERVAL {
            last_emit = Instant::now();
            let _ = app.emit(
                DOWNLOAD_PROGRESS_EVENT,
                json!({ "label": label, "received": received, "total": total }),
            );
        }
    }
    drop(file);

    if received == 0 {
        let _ = std::fs::remove_file(&part);
        return Err("音源返回了空内容（直链可能已过期，重试即可）".to_string());
    }
    // 同卷内改名是原子操作；跨卷时退回拷贝
    if std::fs::rename(&part, dest).is_err() {
        std::fs::copy(&part, dest).map_err(|e| format!("保存文件失败: {e}"))?;
        let _ = std::fs::remove_file(&part);
    }
    let _ = app.emit(
        DOWNLOAD_PROGRESS_EVENT,
        json!({ "label": label, "received": received, "total": received, "done": true }),
    );
    Ok(received)
}

/// 下载目录登记进曲库（下载完自动可见）。
fn register_into_library(dir: &Path) -> Result<library::MusicLibrary, String> {
    let mut current = library::load_library();
    library::add_folder(&mut current, &dir.to_string_lossy())?;
    library::save_library(&current)?;
    Ok(current)
}

/// 从曲目取显示用的 (标题, 歌手, 展示名)。
fn describe(item: &Value) -> (String, String, String) {
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let title = text("title");
    let artist = text("artist");
    let label = if artist.is_empty() {
        title.clone()
    } else {
        format!("{artist} - {title}")
    };
    (title, artist, label)
}

// ─── 命令 ───

/// 只取直链（不下载、不播放）。
#[tauri::command]
pub async fn music_plugin_media_source(
    file: String,
    item: Value,
    quality: Option<String>,
) -> Result<ResolvedSource, String> {
    resolve_source(file, item, quality.unwrap_or_else(|| "standard".to_string())).await
}

/// 在线曲目「落盘」的结果
pub struct Materialized {
    /// 缓存文件（可直接交给播放器）
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    /// 是否命中缓存（命中则没走网络）
    pub from_cache: bool,
    /// 未命中时的直链信息
    pub source: Option<ResolvedSource>,
}

/// 取流 + 落缓存（命中缓存则不联网）。
///
/// 队列里轮到的在线曲目走的也是这里：先落盘，再由播放器接上。
pub async fn materialize(
    app: &AppHandle,
    file: &str,
    item: &Value,
    quality: &str,
) -> Result<Materialized, String> {
    let (title, artist, label) = describe(item);
    let stem = cache_stem(file, item, quality);

    if let Some(cached) = find_cached(&stem) {
        return Ok(Materialized {
            path: cached,
            title,
            artist,
            from_cache: true,
            source: None,
        });
    }

    let source = resolve_source(file.to_string(), item.clone(), quality.to_string()).await?;
    let dest = cache_dir().join(format!("{stem}.{}", extension_for(&source.url)));
    download_to(app, &source, &dest, &label).await?;
    Ok(Materialized {
        path: dest,
        title,
        artist,
        from_cache: false,
        source: Some(source),
    })
}

/// 队列里轮到的在线曲目：取流落盘后接上播放（命令层用，调用方有 async 上下文）。
pub async fn play_pending(
    app: &AppHandle,
    state: &MusicPlayerState,
    pending: &PendingOnline,
) -> Result<PlayerState, String> {
    let seq = pending.seq;
    match materialize(app, &pending.file, &pending.item, &pending.quality).await {
        Ok(m) => {
            let path = m.path.to_string_lossy().to_string();
            state.play_online_resolved(&path, &m.title, &m.artist, seq)
        }
        Err(err) => {
            // 失败也要清标记：不清的话巡查线程会一直以为「有曲目在下载」
            state.abandon_online(seq);
            Err(err)
        }
    }
}

/// 搜索结果里的一首（前端把整份结果传回来建队列）
#[derive(Deserialize)]
pub struct OnlineTrackRef {
    /// 来源插件的脚本文件名
    pub file: String,
    /// 插件返回的曲目对象（原样回传给 `getMediaSource`）
    pub item: Value,
}

/// 在线播放：命中缓存直接播，否则取流 → 落缓存 → 交给播放器。
///
/// 给了 `hits` 时用**整份搜索结果替换播放队列**，并从 `index` 那首开始播 ——
/// 其余曲目在队列里保持「在线」身份，**轮到时才取流**（不会一次性下载几十首）。
#[tauri::command]
pub async fn music_plugin_play(
    app: AppHandle,
    state: State<'_, MusicPlayerState>,
    file: String,
    item: Value,
    quality: Option<String>,
    hits: Option<Vec<OnlineTrackRef>>,
    index: Option<usize>,
) -> Result<PluginPlayOutcome, String> {
    let quality = quality.unwrap_or_else(|| "standard".to_string());
    let materialized = materialize(&app, &file, &item, &quality).await?;
    let path = materialized.path.to_string_lossy().to_string();
    // 记下来源：搜索结果清掉之后，「下载当前曲目」还要靠它重新取流
    let origin = OnlineOrigin {
        platform: super::player::plugin_display_name(&file),
        file: file.clone(),
        item: item.clone(),
        quality: quality.clone(),
    };
    // 缓存文件名是内容哈希，必须把真实歌名交给播放器（否则播放条显示一串哈希）
    let player = state.play_named(&path, &materialized.title, &materialized.artist, Some(origin))?;

    if let Some(hits) = hits {
        let mut items: Vec<QueueItem> = hits
            .into_iter()
            .map(|hit| QueueItem::Online {
                file: hit.file,
                item: hit.item,
                quality: quality.clone(),
            })
            .collect();
        if !items.is_empty() {
            let index = index.unwrap_or(0).min(items.len() - 1);
            // 正在播的这首已经落盘 → 换成路径：回退 / 切模式时不必再取一次流
            items[index] = QueueItem::Path(path.clone());
        }
        state.set_queue_items(items, Some(&path))?;
    }

    Ok(PluginPlayOutcome {
        path,
        from_cache: materialized.from_cache,
        title: materialized.title,
        artist: materialized.artist,
        source: materialized.source,
        player,
    })
}

/// 下载到用户设置的下载目录，并登记进曲库。
#[tauri::command]
pub async fn music_plugin_download(
    app: AppHandle,
    file: String,
    item: Value,
    quality: Option<String>,
) -> Result<PluginDownloadOutcome, String> {
    let quality = quality.unwrap_or_else(|| "standard".to_string());
    download_track(&app, &file, &item, &quality).await
}

/// 下载**正在播放**的那首在线曲目（可选音质，默认沿用当前缓存的音质）。
///
/// 播放器记着这首的来源（插件 + 原始曲目对象），所以搜索结果被清掉之后依然能下 ——
/// 不必让用户回到搜索页再点一次。
#[tauri::command]
pub async fn music_plugin_download_current(
    app: AppHandle,
    state: State<'_, MusicPlayerState>,
    quality: Option<String>,
) -> Result<PluginDownloadOutcome, String> {
    let origin = state
        .current_online_origin()
        .ok_or_else(|| "当前播放的不是在线音源（本地曲目无需下载）".to_string())?;
    let quality = quality.unwrap_or(origin.quality);
    download_track(&app, &origin.file, &origin.item, &quality).await
}

/// 取流 → 落下载目录 → 登记曲库（「下载」与「下载当前曲目」共用）。
async fn download_track(
    app: &AppHandle,
    file: &str,
    item: &Value,
    quality: &str,
) -> Result<PluginDownloadOutcome, String> {
    let (title, artist, label) = describe(item);

    let source = resolve_source(file.to_string(), item.clone(), quality.to_string()).await?;
    let dir = download_dir();
    let dest = dir.join(format!(
        "{}.{}",
        safe_file_stem(&label),
        extension_for(&source.url)
    ));

    let bytes = download_to(app, &source, &dest, &label).await?;
    // 登记要扫目录、读标签，属于阻塞活
    let library = tauri::async_runtime::spawn_blocking({
        let dir = dir.clone();
        move || register_into_library(&dir)
    })
    .await
    .map_err(|e| format!("登记曲库任务失败: {e}"))??;

    Ok(PluginDownloadOutcome {
        path: dest.to_string_lossy().to_string(),
        title,
        artist,
        bytes,
        source,
        dir: dir.to_string_lossy().to_string(),
        library,
    })
}

/// 清理在线播放缓存，返回释放的字节数。
#[tauri::command]
pub async fn music_plugin_clear_cache() -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<u64, String> {
        let dir = cache_dir();
        if !dir.is_dir() {
            return Ok(0);
        }
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("读取缓存目录失败: {e}"))?;
        let mut freed = 0u64;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                freed += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                let _ = std::fs::remove_file(&path);
            }
        }
        eprintln!("[plugin] 已清理在线播放缓存 {freed} 字节");
        Ok(freed)
    })
    .await
    .map_err(|e| format!("清理任务失败: {e}"))?
}

/// 设置下载目录（顺带建出来，免得用户下完才发现路径不存在）。
#[tauri::command]
pub async fn music_plugin_set_download_dir(
    dir: String,
) -> Result<music_settings::MusicSettings, String> {
    let trimmed = dir.trim().to_string();
    if !trimmed.is_empty() {
        std::fs::create_dir_all(&trimmed).map_err(|e| format!("无法使用该目录: {e}"))?;
    }
    let mut current = music_settings::load_settings();
    current.download_dir = trimmed;
    let current = current.sanitized();
    music_settings::save_settings(&current)?;
    Ok(current)
}

/// 当前下载目录与缓存占用（设置页展示用）。
#[tauri::command]
pub async fn music_plugin_storage_info() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<Value, String> {
        let cache = cache_dir();
        let cache_bytes: u64 = std::fs::read_dir(&cache)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.path().is_file())
                    .filter_map(|entry| entry.metadata().ok())
                    .map(|meta| meta.len())
                    .sum()
            })
            .unwrap_or(0);
        Ok(json!({
            "download_dir": download_dir().to_string_lossy(),
            "cache_dir": cache.to_string_lossy(),
            "cache_bytes": cache_bytes,
        }))
    })
    .await
    .map_err(|e| format!("读取存储信息失败: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文件名必须洗掉路径分隔符与 Windows 非法字符，否则会写到别的目录去。
    #[test]
    fn file_stems_are_sanitized() {
        assert_eq!(safe_file_stem("周杰伦 - 稻香"), "周杰伦 - 稻香");
        // 分隔符被换成下划线，前导点被削掉（免得生成近似隐藏文件的名字）
        assert_eq!(safe_file_stem("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_stem(r"AC\DC - Back: in Black?"), "AC_DC - Back_ in Black_");
        assert_eq!(safe_file_stem("   "), "未命名");
        assert_eq!(safe_file_stem("..."), "未命名");
        assert!(safe_file_stem(&"あ".repeat(500)).chars().count() <= 120);
        // 洗完之后不能再有分隔符（Path::join 出去要留在同一层目录里）
        assert!(!safe_file_stem("../../x").contains('/'));
        assert!(!safe_file_stem(r"a\b").contains('\\'));
    }

    /// 扩展名只作可读性用，不在白名单里就退 mp3（解码按内容嗅探）。
    #[test]
    fn extensions_come_from_the_url_path_only() {
        assert_eq!(extension_for("https://x/a/b.flac?token=1"), "flac");
        assert_eq!(extension_for("https://x/a/b.M4A"), "m4a");
        assert_eq!(extension_for("https://x/a/stream?format=flac"), "mp3");
        assert_eq!(extension_for("https://x/a/noext"), "mp3");
        assert_eq!(extension_for(""), "mp3");
    }

    /// 缓存主名要区分音质、区分曲目，但同一组合必须稳定（否则永远命中不了缓存）。
    #[test]
    fn cache_stems_are_stable_and_quality_aware() {
        let item = json!({ "id": "6128587", "title": "Hello Brother" });
        let a = cache_stem("audiomack.js", &item, "standard");
        assert_eq!(a, cache_stem("audiomack.js", &item, "standard"));
        assert_ne!(a, cache_stem("audiomack.js", &item, "high"), "换音质不能复用旧缓存");
        assert_ne!(a, cache_stem("other.js", &item, "standard"), "不同插件不能撞车");
        assert_ne!(
            a,
            cache_stem("audiomack.js", &json!({ "id": "999" }), "standard"),
            "不同曲目不能撞车"
        );
        // 纯主名，不带扩展名（扩展名由取流结果决定）
        assert_eq!(a.len(), 16);
        assert!(!a.contains('.'));
    }

    #[test]
    fn resolved_source_parses_headers_and_ua() {
        let value = json!({
            "url": "https://cdn/x.m4a",
            "headers": { "Referer": "https://site/" },
            "userAgent": "UA/1",
            "quality": "high"
        });
        let source = parse_resolved_source("p.js", &value).unwrap();
        assert_eq!(source.url, "https://cdn/x.m4a");
        assert_eq!(
            source.headers,
            vec![("Referer".to_string(), "https://site/".to_string())]
        );
        assert_eq!(source.user_agent, "UA/1");
        assert_eq!(source.quality, "high");
    }

    /// 插件的「拿不到地址」都要给出人话，且不能被当成成功。
    #[test]
    fn resolved_source_rejects_empty_and_non_http() {
        assert!(parse_resolved_source("p.js", &Value::Null).is_err());
        assert!(parse_resolved_source("p.js", &json!({})).is_err());
        assert!(parse_resolved_source("p.js", &json!({ "url": "   " })).is_err());
        let err = parse_resolved_source("p.js", &json!({ "url": "ftp://x/a.mp3" })).unwrap_err();
        assert!(err.contains("http"), "{err}");
        // 缺 headers / userAgent 时按空处理，而不是报错
        let ok = parse_resolved_source("p.js", &json!({ "url": "https://x/a.mp3" })).unwrap();
        assert!(ok.headers.is_empty());
        assert!(ok.user_agent.is_empty());
    }

    #[test]
    fn describe_builds_label_from_title_and_artist() {
        let (title, artist, label) = describe(&json!({ "title": "稻香", "artist": "周杰伦" }));
        assert_eq!(title, "稻香");
        assert_eq!(artist, "周杰伦");
        assert_eq!(label, "周杰伦 - 稻香");
        // 没有歌手时不该出现「 - 稻香」这种残缺标签
        assert_eq!(describe(&json!({ "title": "稻香" })).2, "稻香");
    }
}
