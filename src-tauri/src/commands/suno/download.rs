use super::*;

use crate::commands::foundation::owned_process::{self, Cancellation};
static BATCHES: std::sync::LazyLock<std::sync::Mutex<Vec<Cancellation>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
struct BatchGuard(Cancellation);
impl Drop for BatchGuard {
    fn drop(&mut self) {
        BATCHES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|token| !token.same(&self.0));
    }
}
struct PendingOutput(Option<PathBuf>);
impl Drop for PendingOutput {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
pub fn cancel_all_downloads() {
    for token in BATCHES.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        token.cancel();
    }
}
pub async fn shutdown_module() -> Result<(), String> {
    cancel_all_downloads();
    crate::runtime_tasks::wait_for_idle("suno", std::time::Duration::from_secs(90)).await?;
    owned_process::ensure_released("suno")
}

/// 下载并转码一批歌曲。
#[tauri::command]
pub async fn suno_download_songs(
    app: tauri::AppHandle,
    songs: Vec<DownloadRef>,
    dir: String,
) -> Result<DownloadReport, String> {
    let token = Cancellation::default();
    BATCHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(token.clone());
    let cleanup = BatchGuard(token.clone());
    owned_process::run("suno", token, move || async move {
        let _cleanup = cleanup;
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
            owned_process::ensure_running("suno")?;
            match download_one(&app, song, &target, i + 1, total).await {
                Ok(()) => {
                    succeeded += 1;
                    succeeded_ids.push(song.id.clone());
                    owned_process::ensure_running("suno")?;
                    mark_downloaded(&song.id);
                }
                Err(e) => {
                    eprintln!("[Suno] 下载失败 {}: {}", song.title, e);
                    failed.push(format!("{}：{e}", song.title));
                }
            }
        }
        Ok(DownloadReport {
            succeeded,
            succeeded_ids,
            failed,
        })
    })
    .await
}

/// 下载一首：mp4 落临时文件 → ffmpeg 提音轨转 mp3 → 清理临时文件。
pub(super) async fn download_one(
    app: &tauri::AppHandle,
    song: &DownloadRef,
    dir: &Path,
    current: usize,
    total: usize,
) -> Result<(), String> {
    let scratch = owned_process::ScratchDir::new("suno-download")?;
    let tmp = scratch.path().join("audio.mp4");
    let cover_tmp = scratch.path().join("cover");

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
    let mut pending = PendingOutput(Some(out.clone()));
    let result = transcode_to_mp3(&tmp, cover, &out);
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&cover_tmp);
    if result.is_err() {
        // 转码失败要把占位的 0 字节文件删掉，否则目录里留下残件
        let _ = std::fs::remove_file(&out);
    }
    if result.is_ok() {
        pending.0 = None;
    }
    result
}

pub(super) async fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let client = http_client();
    let resp = client.get(url).send().await.map_err(|e| {
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
pub(super) fn transcode_to_mp3(
    input: &Path,
    cover: Option<&Path>,
    output: &Path,
) -> Result<(), String> {
    let ffmpeg = utils::bin_tool_path("ffmpeg")
        .unwrap_or_else(|| utils::get_bin_dir().join("ffmpeg").join("ffmpeg.exe"));
    if !ffmpeg.is_file() {
        return Err(format!(
            "未找到内置 ffmpeg（{}），请在设置的运行组件里安装",
            ffmpeg.display()
        ));
    }
    eprintln!(
        "[Suno] 转码中: {} -> {}（封面：{}）",
        input.display(),
        output.display(),
        cover
            .map(|c| c.display().to_string())
            .unwrap_or_else(|| "无".to_string())
    );

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
        cmd.arg("-map")
            .arg("1:v")
            .arg("-c:v")
            .arg("copy")
            .arg("-id3v2_version")
            .arg("3")
            .arg("-metadata:s:v")
            .arg("title=Album cover")
            .arg("-metadata:s:v")
            .arg("comment=Cover (front)");
    } else {
        // `-vn` 去掉视频轨（源是 mp4，内嵌视频 + AAC 音频，只提音频转 mp3）
        cmd.arg("-vn");
    }
    cmd.arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("320k")
        .arg(output);

    let status = owned_process::capture("suno", &mut cmd, std::time::Duration::from_secs(30 * 60))
        .map_err(|e| format!("ffmpeg 未完成: {e}"))?
        .status;
    if !status.success() {
        return Err(format!("ffmpeg 转码失败（exit {:?}）", status.code()));
    }
    Ok(())
}

/// 文件名清理：去掉 Windows 非法字符与换行。
pub(super) fn sanitize_filename(title: &str) -> String {
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
pub(super) fn unique_path(path: PathBuf) -> Result<PathBuf, String> {
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

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[test]
    fn pending_output_cleanup_preserves_completed_outputs() {
        let root = owned_process::ScratchDir::new("suno-output-test").unwrap();
        let pending = root.path().join("pending.mp3");
        let finished = root.path().join("finished.mp3");
        std::fs::write(&pending, "partial").unwrap();
        std::fs::write(&finished, "complete").unwrap();
        drop(PendingOutput(Some(pending.clone())));
        let mut complete = PendingOutput(Some(finished.clone()));
        complete.0 = None;
        drop(complete);
        assert!(!pending.exists());
        assert_eq!(std::fs::read_to_string(finished).unwrap(), "complete");
    }
}
