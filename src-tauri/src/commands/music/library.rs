//! 本地曲库：目录扫描（walkdir）+ 标签/时长（lofty）+ 持久化。
//!
//! 与用户确认的口径一致：导入的文件夹写入 `data_dir/music/library.json`，
//! **启动不自动重扫**，只有显式调用 [`refresh_library`] 才重新扫描磁盘。
//!
//! 只读文件头（lofty 的 properties），不整文件解码，几千首也能秒级扫完。

use std::fs;
use std::path::{Path, PathBuf};

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::read_from_path;
use lofty::tag::Accessor;
use serde::{Deserialize, Serialize};

/// 支持的扩展名（与 rodio/symphonia 的解码能力对齐：不含 ape/wma/opus）
pub const AUDIO_EXTS: [&str; 8] = ["mp3", "flac", "wav", "m4a", "mp4", "aac", "ogg", "oga"];

/// 单次扫描的文件数上限（防止误选整个磁盘导致卡死）
const MAX_TRACKS: usize = 30_000;
/// 目录递归深度上限
const MAX_DEPTH: usize = 12;

/// 一首曲目
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MusicTrack {
    /// 绝对路径，同时作为唯一 id
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub size_bytes: u64,
    /// 所属导入目录（用于按文件夹批量移除）
    pub folder: String,
}

/// 曲库（文件夹列表 + 曲目缓存）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MusicLibrary {
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub tracks: Vec<MusicTrack>,
}

/// 曲库目录：`<data_dir>/music`
pub fn music_dir() -> PathBuf {
    crate::commands::config::get_data_dir().join("music")
}

fn library_path() -> PathBuf {
    music_dir().join("library.json")
}

/// 原子写（临时文件 + rename），失败时退回直接写
pub fn write_json_atomic(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let data = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data.as_bytes()).map_err(|e| format!("写入失败: {}", e))?;
    if fs::rename(&tmp, path).is_err() {
        fs::write(path, data.as_bytes()).map_err(|e| format!("写入失败: {}", e))?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

/// 读取曲库（不存在时返回空库）
pub fn load_library() -> MusicLibrary {
    let path = library_path();
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => MusicLibrary::default(),
    }
}

pub fn save_library(library: &MusicLibrary) -> Result<(), String> {
    let value = serde_json::to_value(library).map_err(|e| e.to_string())?;
    write_json_atomic(&library_path(), &value)
}

/// 是否音频文件（按扩展名，忽略大小写）
pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            AUDIO_EXTS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// 读标签与时长；失败时回退文件名（不返回 Err，保证扫库不中断）
fn read_track(path: &Path, folder: &str) -> MusicTrack {
    let path_str = path.to_string_lossy().to_string();
    let size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut track = MusicTrack {
        path: path_str.clone(),
        title: path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.clone()),
        artist: String::new(),
        album: String::new(),
        duration_ms: 0,
        size_bytes,
        folder: folder.to_string(),
    };

    if let Ok(tagged) = read_from_path(path) {
        track.duration_ms = tagged.properties().duration().as_millis() as u64;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(title) = tag.title() {
                let title = title.trim();
                if !title.is_empty() {
                    track.title = title.to_string();
                }
            }
            if let Some(artist) = tag.artist() {
                track.artist = artist.trim().to_string();
            }
            if let Some(album) = tag.album() {
                track.album = album.trim().to_string();
            }
        }
    }
    track
}

/// 扫描一个目录（递归），返回曲目列表
pub fn scan_folder(folder: &str) -> Result<Vec<MusicTrack>, String> {
    let root = Path::new(folder);
    if !root.is_dir() {
        return Err(format!("不是有效的文件夹: {}", folder));
    }
    let mut tracks: Vec<MusicTrack> = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .min_depth(1)
        .max_depth(MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // 跳过回收站/系统目录等噪音
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir()
                && (name == "$RECYCLE.BIN"
                    || name == "System Volume Information"
                    || name.starts_with('.')))
        })
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() || !is_audio_file(entry.path()) {
            continue;
        }
        tracks.push(read_track(entry.path(), folder));
        if tracks.len() >= MAX_TRACKS {
            break;
        }
    }
    tracks.sort_by(|a, b| {
        a.artist
            .to_lowercase()
            .cmp(&b.artist.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(tracks)
}

/// 新增导入文件夹：扫描并合并进曲库（同目录重复导入只刷新其曲目）
pub fn add_folder(library: &mut MusicLibrary, folder: &str) -> Result<usize, String> {
    let normalized = normalize_folder(folder)?;
    let scanned = scan_folder(&normalized)?;
    let added = scanned.len();

    library.folders.retain(|f| !same_path(f, &normalized));
    library.folders.push(normalized.clone());
    library
        .tracks
        .retain(|t| !same_path(&t.folder, &normalized));
    library.tracks.extend(scanned);
    Ok(added)
}

/// 移除导入文件夹（含其曲目）
pub fn remove_folder(library: &mut MusicLibrary, folder: &str) {
    let target = folder.trim_end_matches(['\\', '/']).to_string();
    library.folders.retain(|f| !same_path(f, &target));
    library.tracks.retain(|t| !same_path(&t.folder, &target));
}

/// 手动更新：重扫全部导入文件夹；返回 (新增, 移除) 曲目数
pub fn refresh_library(library: &mut MusicLibrary) -> Result<(usize, usize), String> {
    let folders = library.folders.clone();
    let old_count = library.tracks.len();
    let mut next: Vec<MusicTrack> = Vec::new();
    for folder in &folders {
        match scan_folder(folder) {
            Ok(tracks) => next.extend(tracks),
            Err(err) => {
                // 文件夹被删掉/改名：保留在列表里但标注为空，由用户决定是否移除
                eprintln!("[Music] 扫描 {} 失败: {}", folder, err);
            }
        }
    }
    let new_count = next.len();
    library.tracks = next;
    let added = new_count.saturating_sub(old_count);
    let removed = old_count.saturating_sub(new_count);
    Ok((added, removed))
}

// ─── 单曲文件操作（重命名 / 删除） ───

/// 生成文件名时的最大字符数（Windows 整个路径有 260 字符上限，留出目录与扩展名余量）
const MAX_FILE_NAME_CHARS: usize = 120;

/// 清洗文件名：替换 Windows 非法字符与控制字符，清掉首尾空白/点，并限制长度。
///
/// 冒号换成全角 `：`、路径分隔符换成 `-`，其余非法字符换 `_` —— 尽量保留可读性，
/// 而不是一律下划线（`Artist: Title` → `Artist： Title` 仍能认出来是谁）。
pub fn sanitize_file_name(name: &str) -> String {
    const INVALID: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let mut cleaned = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_control() || INVALID.contains(&ch) {
            cleaned.push(match ch {
                ':' => '：',
                '/' | '\\' => '-',
                _ => '_',
            });
        } else {
            cleaned.push(ch);
        }
    }
    let trimmed = cleaned.trim().trim_end_matches('.');
    let capped: String = trimmed.chars().take(MAX_FILE_NAME_CHARS).collect();
    capped.trim_end_matches([' ', '.']).to_string()
}

/// 用音频标签生成推荐文件名（含扩展名）：`歌手 - 标题.ext`。
///
/// 没有标题标签时返回 None（调用方保留原名）—— 与其猜一个名字，不如不猜。
pub fn suggest_file_name(path: &Path) -> Option<String> {
    let tagged = read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let title = tag
        .title()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let artist = tag.artist().map(|s| s.trim().to_string()).unwrap_or_default();
    let stem = sanitize_file_name(&if artist.is_empty() {
        title
    } else {
        format!("{} - {}", artist, title)
    });
    if stem.is_empty() {
        return None;
    }
    Some(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{}.{}", stem, ext),
        _ => stem,
    })
}

/// 解析重命名目标路径：同目录、保留原扩展名、重名自动追加 ` (1)`、` (2)`…
///
/// 用户可能把扩展名也填了（`歌手 - 标题.mp3`）：先剥掉再按原扩展名拼回，
/// 避免得到 `xxx.mp3.mp3`。
pub fn resolve_rename_target(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("无法确定文件所在目录")?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let cleaned = sanitize_file_name(new_name);
    if cleaned.is_empty() {
        return Err("文件名不能为空".to_string());
    }
    let stem = match cleaned.rsplit_once('.') {
        Some((head, tail)) if !ext.is_empty() && tail.eq_ignore_ascii_case(&ext) => head.to_string(),
        _ => cleaned,
    };
    let stem = stem.trim_end_matches([' ', '.']).to_string();
    if stem.is_empty() {
        return Err("文件名不能为空".to_string());
    }

    let build = |suffix: &str| -> PathBuf {
        let file_name = if ext.is_empty() {
            format!("{}{}", stem, suffix)
        } else {
            format!("{}{}.{}", stem, suffix, ext)
        };
        parent.join(file_name)
    };

    let direct = build("");
    if !direct.exists() || same_path(&direct.to_string_lossy(), &path.to_string_lossy()) {
        return Ok(direct);
    }
    for index in 1..=999 {
        let candidate = build(&format!(" ({})", index));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("同名文件过多，无法生成可用文件名".to_string())
}

/// 重命名曲目文件，返回新路径（名字未变时原样返回）。
pub fn rename_track_file(path: &str, new_name: &str) -> Result<String, String> {
    let source = PathBuf::from(path);
    if !source.is_file() {
        return Err("文件不存在（可能已被移动或删除）".to_string());
    }
    let target = resolve_rename_target(&source, new_name)?;
    if same_path(&target.to_string_lossy(), path) {
        return Ok(path.to_string());
    }
    fs::rename(&source, &target).map_err(|e| format!("重命名失败: {}", e))?;
    Ok(target.to_string_lossy().to_string())
}

/// 删除曲目文件：**移入系统回收站**（可还原），不做永久删除。
pub fn delete_track_file(path: &str) -> Result<(), String> {
    if !Path::new(path).exists() {
        return Ok(()); // 已经不存在 → 视为成功，保持幂等
    }
    trash::delete(path).map_err(|e| format!("移入回收站失败: {}", e))
}

/// 曲目改名后在曲库里同步：重读该文件的标签（无标签时标题取自文件名，必须重读）。
///
/// 找不到对应条目返回 false（例如曲目已不在曲库中）。
pub fn replace_track_path(library: &mut MusicLibrary, old_path: &str, new_path: &str) -> bool {
    let Some(index) = library
        .tracks
        .iter()
        .position(|t| same_path(&t.path, old_path))
    else {
        return false;
    };
    let folder = library.tracks[index].folder.clone();
    library.tracks[index] = read_track(Path::new(new_path), &folder);
    true
}

/// 从曲库移除若干曲目，返回移除条数。
pub fn remove_tracks(library: &mut MusicLibrary, paths: &[String]) -> usize {
    let before = library.tracks.len();
    library
        .tracks
        .retain(|t| !paths.iter().any(|p| same_path(p, &t.path)));
    before - library.tracks.len()
}

/// 取绝对路径并去掉结尾分隔符（统一比较口径）
fn normalize_folder(folder: &str) -> Result<String, String> {
    let trimmed = folder.trim();
    if trimmed.is_empty() {
        return Err("文件夹路径为空".to_string());
    }
    let path = Path::new(trimmed);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    Ok(absolute
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string())
}

/// 路径比较：忽略大小写、混用的正/反斜杠与结尾分隔符
pub fn same_path(a: &str, b: &str) -> bool {
    fn normalize(path: &str) -> String {
        path.trim()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    }
    normalize(a) == normalize(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kira-music-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_is_audio_file_by_extension() {
        assert!(is_audio_file(Path::new("D:/a/b.mp3")));
        assert!(is_audio_file(Path::new("D:/a/b.FLAC")));
        assert!(is_audio_file(Path::new("D:/a/b.m4a")));
        assert!(!is_audio_file(Path::new("D:/a/b.txt")));
        assert!(!is_audio_file(Path::new("D:/a/b.ape")));
        assert!(!is_audio_file(Path::new("D:/a/noext")));
    }

    #[test]
    fn test_scan_folder_filters_and_falls_back_to_filename() {
        let dir = temp_dir("scan");
        fs::write(dir.join("song.mp3"), b"not a real mp3").unwrap();
        fs::write(dir.join("song.mp3"), b"fake").unwrap();
        fs::write(dir.join("readme.txt"), b"text").unwrap();
        fs::create_dir_all(dir.join("album")).unwrap();
        fs::write(dir.join("album").join("track.FLAC"), b"fake").unwrap();
        // 噪音目录应被跳过
        fs::create_dir_all(dir.join("$RECYCLE.BIN")).unwrap();
        fs::write(dir.join("$RECYCLE.BIN").join("junk.mp3"), b"fake").unwrap();

        let folder = dir.to_string_lossy().to_string();
        let tracks = scan_folder(&folder).unwrap();

        assert_eq!(tracks.len(), 2, "应只扫到 2 个音频文件: {tracks:?}");
        // 坏文件读不出标签 → 回退文件名
        assert!(tracks.iter().any(|t| t.title == "track"));
        assert!(tracks.iter().any(|t| t.title == "song"));
        assert!(tracks.iter().all(|t| t.duration_ms == 0));
        assert!(tracks.iter().all(|t| same_path(&t.folder, &folder)));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_scan_folder_rejects_non_directory() {
        assert!(scan_folder("D:/definitely/not/exists/xyz").is_err());
    }

    #[test]
    fn test_add_and_remove_folder_keeps_single_copy() {
        let dir = temp_dir("addremove");
        fs::write(dir.join("a.mp3"), b"fake").unwrap();
        let folder = dir.to_string_lossy().to_string();

        let mut library = MusicLibrary::default();
        assert_eq!(add_folder(&mut library, &folder).unwrap(), 1);
        assert_eq!(library.folders.len(), 1);
        assert_eq!(library.tracks.len(), 1);

        // 重复导入同一目录：不产生重复曲目
        add_folder(&mut library, &folder).unwrap();
        assert_eq!(library.folders.len(), 1);
        assert_eq!(library.tracks.len(), 1);

        remove_folder(&mut library, &folder);
        assert!(library.folders.is_empty());
        assert!(library.tracks.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_refresh_detects_added_and_removed() {
        let dir = temp_dir("refresh");
        fs::write(dir.join("a.mp3"), b"fake").unwrap();
        let folder = dir.to_string_lossy().to_string();

        let mut library = MusicLibrary::default();
        add_folder(&mut library, &folder).unwrap();
        assert_eq!(library.tracks.len(), 1);

        // 新增一个文件 → 刷新后应看到 2 首
        fs::write(dir.join("b.wav"), b"fake").unwrap();
        let (added, removed) = refresh_library(&mut library).unwrap();
        assert_eq!(library.tracks.len(), 2);
        assert_eq!((added, removed), (1, 0));

        // 删掉一个 → 刷新后应回到 1 首
        fs::remove_file(dir.join("a.mp3")).unwrap();
        let (added, removed) = refresh_library(&mut library).unwrap();
        assert_eq!(library.tracks.len(), 1);
        assert_eq!((added, removed), (0, 1));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_same_path_ignores_case_and_trailing_separator() {
        assert!(same_path("D:\\Music\\Rock\\", "d:/music/rock"));
        assert!(!same_path("D:\\Music\\Rock", "D:\\Music\\Jazz"));
    }

    #[test]
    fn test_normalize_folder_rejects_empty() {
        assert!(normalize_folder("   ").is_err());
    }

    /// 非法字符要替换但不能丢可读性；首尾空白/点与超长都要处理。
    #[test]
    fn test_sanitize_file_name() {
        assert_eq!(sanitize_file_name("Artist: Title"), "Artist： Title");
        assert_eq!(sanitize_file_name("a/b\\c"), "a-b-c");
        assert_eq!(sanitize_file_name("what?.mp3"), "what_.mp3");
        assert_eq!(sanitize_file_name("  trailing.  "), "trailing");
        assert_eq!(sanitize_file_name("换行\n字符"), "换行_字符");
        // 中文与常见符号原样保留
        assert_eq!(sanitize_file_name("周杰伦 - 稻香"), "周杰伦 - 稻香");
        // 超长截断到上限之内
        let long = "字".repeat(400);
        assert!(sanitize_file_name(&long).chars().count() <= MAX_FILE_NAME_CHARS);
        assert!(sanitize_file_name("   ").is_empty());
    }

    /// 没有可解析的标签时不给建议（宁可保留原名，也不要猜一个名字）。
    #[test]
    fn test_suggest_file_name_is_none_without_tags() {
        let dir = temp_dir("suggest");
        let file = dir.join("song.mp3");
        fs::write(&file, b"not a real mp3").unwrap();
        assert_eq!(suggest_file_name(&file), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 目标名解析：保留原扩展名、重名自动 `(1)`、用户填了扩展名也不会重复。
    #[test]
    fn test_resolve_rename_target() {
        let dir = temp_dir("rename");
        let source = dir.join("old.mp3");
        fs::write(&source, b"fake").unwrap();

        // 普通改名
        let target = resolve_rename_target(&source, "周杰伦 - 稻香").unwrap();
        assert_eq!(target.file_name().unwrap().to_string_lossy(), "周杰伦 - 稻香.mp3");

        // 用户把扩展名也填了 → 不能出现 .mp3.mp3
        let target = resolve_rename_target(&source, "周杰伦 - 稻香.mp3").unwrap();
        assert_eq!(target.file_name().unwrap().to_string_lossy(), "周杰伦 - 稻香.mp3");

        // 目标已存在 → 追加 (1)
        fs::write(dir.join("taken.mp3"), b"fake").unwrap();
        let target = resolve_rename_target(&source, "taken").unwrap();
        assert_eq!(target.file_name().unwrap().to_string_lossy(), "taken (1).mp3");

        // 名字没变 → 返回原路径（调用方据此判定为空操作）
        let target = resolve_rename_target(&source, "old.mp3").unwrap();
        assert!(same_path(&target.to_string_lossy(), &source.to_string_lossy()));

        // 名字清洗后为空 → 报错
        assert!(resolve_rename_target(&source, "  ").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    /// 真改一次磁盘文件名，并确认曲库条目跟着换（无标签时标题取自文件名）。
    #[test]
    fn test_rename_track_file_and_library_sync() {
        let dir = temp_dir("rename-sync");
        let source = dir.join("old.mp3");
        fs::write(&source, b"fake").unwrap();
        let folder = dir.to_string_lossy().to_string();
        let mut library = MusicLibrary::default();
        add_folder(&mut library, &folder).unwrap();
        let old_path = library.tracks[0].path.clone();

        let new_path = rename_track_file(&old_path, "新名字").unwrap();
        assert!(Path::new(&new_path).is_file());
        assert!(!Path::new(&old_path).exists());
        assert!(new_path.ends_with("新名字.mp3"));

        assert!(replace_track_path(&mut library, &old_path, &new_path));
        assert!(same_path(&library.tracks[0].path, &new_path));
        // 无标签：标题回退为新的文件名
        assert_eq!(library.tracks[0].title, "新名字");
        assert_eq!(library.tracks[0].folder, folder);

        // 文件不存在时报错（而不是静默成功）
        assert!(rename_track_file("D:/definitely/missing/x.mp3", "y").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    /// 删除曲目后曲库里不再包含它；重复删除保持幂等。
    #[test]
    fn test_remove_tracks_from_library_and_delete_is_idempotent() {
        let dir = temp_dir("delete");
        let file = dir.join("a.mp3");
        fs::write(&file, b"fake").unwrap();
        let folder = dir.to_string_lossy().to_string();
        let mut library = MusicLibrary::default();
        add_folder(&mut library, &folder).unwrap();
        assert_eq!(library.tracks.len(), 1);

        let only_path = library.tracks[0].path.clone();
        let removed = remove_tracks(&mut library, &[only_path]);
        assert_eq!(removed, 1);
        assert!(library.tracks.is_empty());
        assert_eq!(remove_tracks(&mut library, &["D:/nope.mp3".to_string()]), 0);

        // 已不存在的文件：删除视为成功（幂等），不会误报失败
        assert!(delete_track_file("D:/definitely/missing/x.mp3").is_ok());

        let _ = fs::remove_dir_all(&dir);
    }
}
