//! ffmpeg 转码兜底：解不了的编码，先转成能解的再播。
//!
//! **为什么需要它**：rodio/symphonia 没有 AC-3 / E-AC-3 / DTS 解码器，于是
//! 「MP4 容器 + E-AC-3 音轨」这类文件根本放不出来 —— 它们常常还被写成了
//! `.mp3` / `.flac` 等扩展名（`Yungblud - Abyss.mp3` 就是这种），用户完全无从判断
//! 到底是文件坏了还是播放器不行。ffmpeg 能解这些格式，所以让播放器在**解不开**时
//! 退一步：转成 AAC 落在转码缓存里，再交给同一个播放引擎播。
//!
//! 播放路径依然只有一条（本地文件），所以队列 / 均衡器 / 进度条 / 自动切歌全部照旧。
//! 转码是一次性开销：缓存键 = 路径 + 大小 + 修改时间，源文件换了就重转。
//!
//! 与在线播放的缓存分开存放（`cache/transcoded` vs `cache/online`）；
//! 在线缓存可以随手清（重新下载即可），转码缓存清了就得再跑一次 ffmpeg。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::library;
use crate::commands::hidden_cmd::hidden_cmd;
use crate::commands::utils;

/// 转码超时：正常的 2~5 分钟音频转码只要几秒；卡住（网络盘 / 损坏文件）不能无限等。
const TRANSCODE_TIMEOUT: Duration = Duration::from_secs(120);

/// 转码缓存目录：`<data_dir>/music/cache/transcoded`
pub(crate) fn cache_dir() -> PathBuf {
    library::music_dir().join("cache").join("transcoded")
}

/// FNV-1a 64 —— 只为生成缓存文件名，不用于安全用途（避免为此引入依赖）。
fn hash(text: &str) -> String {
    let mut value: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        value ^= *byte as u64;
        value = value.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{value:016x}")
}

/// 缓存键：**路径 + 大小 + 修改时间** —— 只看路径的话，用户换了个同名文件
/// （比如重新下载后覆盖了旧的）会一直播到旧的转码结果。
fn cache_stem(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // 路径统一小写：Windows 上大小写不同指向同一个文件，不该转两份
    let identity = format!(
        "{}|{}|{}",
        path.to_string_lossy().to_lowercase(),
        meta.len(),
        modified
    );
    Some(hash(&identity))
}

/// 已经转过码则返回缓存文件路径（播放时优先用它）。
pub fn cached(path: &str) -> Option<String> {
    let stem = cache_stem(Path::new(path))?;
    let file = cache_dir().join(format!("{stem}.m4a"));
    if file.is_file() {
        Some(file.to_string_lossy().to_string())
    } else {
        None
    }
}

/// 这个播放器自己解不开这个文件吗？（只探头部，不解码、不转码）
///
/// 后台预转码靠它判断「下一首值不值得先转」—— 直接对每个文件都跑 ffmpeg 的话，
/// 会为几 MB 的普通 mp3 白白起一堆外部进程。
pub fn needs_transcode(path: &str) -> bool {
    if cached(path).is_some() {
        return false;
    }
    // 与 play_locked 完全相同的开流方式：它解不开，播放时就一定会走到转码分支
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let byte_len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut builder = rodio::Decoder::builder().with_data(std::io::BufReader::new(file));
    if byte_len > 0 {
        builder = builder.with_byte_len(byte_len);
    }
    builder.build().is_err()
}

/// 转码缓存占用（字节）。
pub fn cache_bytes() -> u64 {
    dir_size(&cache_dir())
}

/// 清空转码缓存，返回释放的字节数。`playing` 为正在播放的**原文件**（若有）。
///
/// 与在线缓存一起清（设置页的「清理缓存」）：清掉的只是**副本**，
/// 再播那首歌会重新转一次 —— 慢一点，但不会丢任何东西。
///
/// 正在播放的那首要跳过：它的转码副本正被播放器读着，删掉会打断播放，
/// 在 Windows 上更是直接删不掉（文件被占用）—— 强行删只会让「已清理 x MB」
/// 和实际对不上。
pub fn clear_cache(playing: Option<&str>) -> Result<u64, String> {
    let keep = playing.and_then(cached);
    let dir = cache_dir();
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut freed = 0u64;
    for entry in std::fs::read_dir(&dir)
        .map_err(|e| format!("读取转码缓存目录失败: {e}"))?
        .flatten()
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if keep
            .as_deref()
            .map(|k| library::same_path(&path.to_string_lossy(), k))
            .unwrap_or(false)
        {
            continue;
        }
        freed += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        let _ = std::fs::remove_file(&path);
    }
    Ok(freed)
}

/// 目录里所有文件的总大小（目录不存在则为 0）。
fn dir_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_file())
                .filter_map(|entry| entry.metadata().ok())
                .map(|meta| meta.len())
                .sum()
        })
        .unwrap_or(0)
}

/// 这种失败才值得拉起 ffmpeg。
///
/// 只有「解不开」是**编码器缺失**（ffmpeg 能救）；文件不存在、被占用、
/// 输出设备异常之类拉 ffmpeg 也没用，还会把真正的错误掩盖掉。
pub fn worth_retrying(error: &str) -> bool {
    error.starts_with("无法解码")
}

/// 缓存文件名是哈希，播放条得显示**原来的**标题与歌手。
pub fn labels_for(path: &str) -> (String, String) {
    let track = library::load_library()
        .tracks
        .into_iter()
        .find(|t| library::same_path(&t.path, path));
    match track {
        Some(track) => (track.title, track.artist),
        None => (
            Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string()),
            String::new(),
        ),
    }
}

/// 确保 `path` 有一个能播的副本：命中缓存直接返回，否则用 ffmpeg 转一份。
pub fn ensure(path: &str) -> Result<String, String> {
    if let Some(hit) = cached(path) {
        return Ok(hit);
    }
    let source = Path::new(path);
    if !source.is_file() {
        return Err(format!("文件不存在（可能已被移动或删除）: {path}"));
    }
    let stem = cache_stem(source).ok_or_else(|| "读取文件信息失败".to_string())?;
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建转码缓存目录失败: {e}"))?;
    let dest = dir.join(format!("{stem}.m4a"));
    let part = dir.join(format!("{stem}.m4a.part"));

    let ffmpeg = locate_ffmpeg()?;
    // `-map 0:a:0` 只取第一条音轨（源文件常带封面/视频轨）；
    // `-ac 2` 降混到立体声：rodio 输出是立体声，6 声道的 E-AC-3 直接喂进去会解不了；
    // 输出到 `.part` 再改名，避免中断时在缓存里留下半截文件。
    let mut command = hidden_cmd(&ffmpeg);
    command
        .args(["-nostdin", "-y", "-v", "error"])
        .arg("-i")
        .arg(source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "2",
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ])
        .arg(&part);

    match run_with_timeout(&mut command, TRANSCODE_TIMEOUT) {
        Ok(()) => {
            if !part.is_file() {
                return Err("ffmpeg 没有产出文件".to_string());
            }
            // 同卷内改名是原子操作；跨卷时退回拷贝
            if std::fs::rename(&part, &dest).is_err() {
                std::fs::copy(&part, &dest).map_err(|e| format!("保存转码结果失败: {e}"))?;
                let _ = std::fs::remove_file(&part);
            }
            Ok(dest.to_string_lossy().to_string())
        }
        Err(err) => {
            let _ = std::fs::remove_file(&part);
            Err(err)
        }
    }
}

/// ffmpeg 在哪：**只用软件内置的那份**（与 RTSP 服务同一套取法：bin/ffmpeg/ffmpeg.exe）。
///
/// 刻意不查用户 PATH 里的 ffmpeg：外部版本装了哪些编码器不可控（可能没有 AAC），
/// 也可能是被别的软件塞进去的旧版 —— 转码结果不稳定，排查成本还高。
pub(crate) fn locate_ffmpeg() -> Result<PathBuf, String> {
    let path = utils::bin_tool_path("ffmpeg")
        .unwrap_or_else(|| utils::get_bin_dir().join("ffmpeg").join("ffmpeg.exe"));
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "这种编码需要先转码才能播放，但没找到内置的 ffmpeg（{}）；请在设置的运行组件里安装",
            path.display()
        ))
    }
}

/// 跑 ffmpeg 并带超时卡口：卡死的外部进程会把播放彻底堵死，不能无限等。
fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<(), String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 ffmpeg 失败: {e}"))?;

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let mut stderr = String::new();
                    if let Some(mut pipe) = child.stderr.take() {
                        use std::io::Read;
                        let _ = pipe.read_to_string(&mut stderr);
                    }
                    let detail = stderr.trim().lines().last().unwrap_or("").trim();
                    return Err(if detail.is_empty() {
                        format!("ffmpeg 转码失败（退出码 {status}）")
                    } else {
                        format!("ffmpeg 转码失败: {detail}")
                    });
                }
                return Ok(());
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err(format!("ffmpeg 转码超时（超过 {} 秒）", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(format!("等待 ffmpeg 失败: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_depends_on_path_size_and_mtime() {
        let dir = std::env::temp_dir().join(format!("anyver-transcode-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("song.mp3");
        std::fs::write(&file, b"fake audio").unwrap();

        let first = cache_stem(&file).expect("有文件信息");
        assert_eq!(first, cache_stem(&file).expect("同一文件应稳定"), "同一文件必须得到同一个键");

        let other = dir.join("other.mp3");
        std::fs::write(&other, b"fake audio").unwrap();
        assert_ne!(first, cache_stem(&other).expect("有文件信息"), "不同路径不能共用一个缓存");

        // 文件不存在 → 无法生成键（调用方据此放弃）
        assert!(cache_stem(&dir.join("missing.mp3")).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 只有「解不开」才值得转码：文件不存在等情况拉 ffmpeg 只会掩盖真正的错误。
    #[test]
    fn only_decode_failures_are_worth_retrying() {
        assert!(worth_retrying("无法解码该音频文件（实际容器：MP4/M4A…"));
        assert!(!worth_retrying("打开文件失败: 系统找不到指定的路径"));
        assert!(!worth_retrying("创建音频输出失败"));
    }

    /// 后台预转码靠 `needs_transcode` 决定「要不要起 ffmpeg」—— 它一旦对**能播的**
    /// 文件判成 true，后台就会为曲库里每一首都白跑一次外部进程。
    #[test]
    fn needs_transcode_stays_false_for_a_playable_file() {
        let Ok(ffmpeg) = locate_ffmpeg() else {
            return; // 没装内置 ffmpeg，跳过
        };
        let dir = std::env::temp_dir().join(format!("anyver-needs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let playable = dir.join("plain.m4a");
        let generated = hidden_cmd(&ffmpeg)
            .args(["-nostdin", "-y", "-v", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-c:a", "aac", "-ac", "2"])
            .arg(&playable)
            .output();
        if !matches!(generated, Ok(out) if out.status.success()) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // 这个 ffmpeg 构建造不出音频，跳过
        }

        let path = playable.to_string_lossy().to_string();
        assert!(!needs_transcode(&path), "能播的文件不该被判定为需要转码");
        // 文件不存在 / 打不开：不值得转码（播放时会由 play() 给出真正的错误）
        assert!(!needs_transcode("D:/no/such/file.mp3"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真转一次：造一个 6 声道 E-AC-3（symphonia 解不了），确认转码后能拿到 AAC 副本。
    ///
    /// 依赖 ffmpeg，找不到就跳过（与插件宿主那组用例的处理方式一致）。
    #[test]
    fn transcodes_an_unsupported_codec_into_a_playable_copy() {
        let Ok(ffmpeg) = locate_ffmpeg() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("anyver-transcode-run-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let source = dir.join("eac3.mp4");
        let generated = hidden_cmd(&ffmpeg)
            .args(["-nostdin", "-y", "-v", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-c:a", "eac3", "-ac", "6"])
            .arg(&source)
            .output();
        if !matches!(generated, Ok(out) if out.status.success()) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // 这个 ffmpeg 构建没有 eac3 编码器，跳过
        }

        let cached_path = match ensure(&source.to_string_lossy()) {
            Ok(path) => path,
            Err(err) => {
                let _ = std::fs::remove_dir_all(&dir);
                panic!("转码应当成功: {err}");
            }
        };
        assert!(std::path::Path::new(&cached_path).is_file(), "转码结果要落在磁盘上");
        assert_ne!(cached_path, source.to_string_lossy(), "不能覆盖原文件");

        // 转出来的必须是 symphonia 认得的编码（AAC），且已降混成立体声
        let Some(ffprobe) = utils::find_in_path("ffprobe") else {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        let verify = hidden_cmd(&ffprobe)
            .args(["-v", "error", "-show_entries", "stream=codec_name,channels"])
            .args(["-of", "csv=p=0"])
            .arg(&cached_path)
            .output();
        match verify {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
                assert!(text.contains("aac"), "转码结果应是 AAC，实际: {text}");
                assert!(text.contains(",2"), "应降混为立体声，实际: {text}");
            }
            Err(_) => {
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        }

        // 第二次应当直接命中缓存
        let again = ensure(&source.to_string_lossy()).expect("命中缓存");
        assert_eq!(again, cached_path);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
