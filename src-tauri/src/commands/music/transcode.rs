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
fn cache_dir() -> PathBuf {
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

/// ffmpeg 在哪：运行组件目录 → PATH。
fn locate_ffmpeg() -> Result<PathBuf, String> {
    utils::bin_tool_path("ffmpeg")
        .or_else(|| utils::find_bin_executable("ffmpeg"))
        .or_else(|| utils::find_in_path("ffmpeg"))
        .filter(|path| path.is_file())
        .ok_or_else(|| {
            "这种编码需要先转码才能播放，但没找到 ffmpeg（可在设置的运行组件里安装）".to_string()
        })
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
