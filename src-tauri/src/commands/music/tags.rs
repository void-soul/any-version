//! 把在线音源给的元信息写进下载下来的音频文件。
//!
//! 下载得到的文件默认**没有任何标签**（只有一个文件名），于是曲库只能显示文件名，
//! 作者 / 专辑 / 封面全是空的。而插件返回的曲目对象里通常带着这些信息，
//! 所以落盘时一并写进去 —— 曲库扫描（`library::read_track`）自然就显示出来了。
//!
//! 写标签只影响展示，失败不能影响下载本身：调用方应当只记日志。

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::read_from_path;
use lofty::tag::{Accessor, Tag, TagExt};

/// 从插件曲目对象里取到的元信息
pub struct TrackMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// 封面图（已经下载好的字节 + 按魔数判断出的图片类型）
pub struct CoverArt {
    pub bytes: Vec<u8>,
    pub mime: Option<MimeType>,
}

/// 按魔数判断图片类型。
///
/// 认不出时返回 None：`Picture::new_unchecked` 允许不带 MIME，播放器一般也能正常显示；
/// 反过来**猜错**更糟（声明成 image/jpeg 实际是 png，部分播放器直接不显示）。
pub fn sniff_image_mime(bytes: &[u8]) -> Option<MimeType> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(MimeType::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(MimeType::Jpeg);
    }
    if bytes.starts_with(b"GIF8") {
        return Some(MimeType::Gif);
    }
    if bytes.starts_with(b"BM") {
        return Some(MimeType::Bmp);
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return Some(MimeType::Tiff);
    }
    None
}

/// 写入标题 / 歌手 / 专辑 / 封面。
///
/// 文件格式本身不支持写标签时（例如裸 WAV）返回 Err，调用方记录即可。
pub fn write_tags(path: &Path, meta: &TrackMeta, cover: Option<CoverArt>) -> Result<(), String> {
    let mut tagged = read_from_path(path).map_err(|e| format!("读取音频文件失败: {e}"))?;

    let tag = match tagged.primary_tag_mut() {
        Some(tag) => tag,
        None => {
            // 文件里还没有标签：按它的格式补一个空的，再写
            let tag_type = tagged.file_type().primary_tag_type();
            tagged.insert_tag(Tag::new(tag_type));
            tagged
                .primary_tag_mut()
                .ok_or_else(|| "该格式不支持写入标签".to_string())?
        }
    };

    if !meta.title.trim().is_empty() {
        tag.set_title(meta.title.trim().to_string());
    }
    if !meta.artist.trim().is_empty() {
        tag.set_artist(meta.artist.trim().to_string());
    }
    if !meta.album.trim().is_empty() {
        tag.set_album(meta.album.trim().to_string());
    }
    if let Some(cover) = cover {
        tag.push_picture(Picture::new_unchecked(
            PictureType::CoverFront,
            cover.mime,
            None,
            cover.bytes,
        ));
    }

    tag.save_to_path(path, WriteOptions::default())
        .map_err(|e| format!("写入标签失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_common_image_formats() {
        assert_eq!(
            sniff_image_mime(b"\x89PNG\r\n\x1a\nrest"),
            Some(MimeType::Png),
            "png 魔数要认出来"
        );
        assert_eq!(sniff_image_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(MimeType::Jpeg));
        assert_eq!(sniff_image_mime(b"GIF89a"), Some(MimeType::Gif));
        assert_eq!(sniff_image_mime(b"BM...."), Some(MimeType::Bmp));
    }

    /// 认不出就返回 None，绝不猜测：猜错会让播放器干脆不显示封面。
    #[test]
    fn unknown_bytes_are_not_guessed() {
        assert_eq!(sniff_image_mime(b"not an image at all"), None);
        assert_eq!(sniff_image_mime(b""), None);
    }

    /// 真写真读：用内置 ffmpeg 造一个 mp3，写进去再用 lofty 读回来。
    ///
    /// 只验证「写了能读回」—— 这正是下载后曲库能显示作者/专辑的前提。
    /// 没有 ffmpeg 就跳过（与 transcode 那组用例的处理一致）。
    #[test]
    fn written_tags_read_back() {
        use crate::commands::hidden_cmd::hidden_cmd;

        let ffmpeg = match super::super::transcode::locate_ffmpeg() {
            Ok(path) => path,
            Err(_) => return,
        };
        let dir = std::env::temp_dir().join(format!("anyver-tags-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("sample.mp3");
        let generated = hidden_cmd(&ffmpeg)
            .args(["-nostdin", "-y", "-v", "error"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .args(["-c:a", "libmp3lame", "-q:a", "9"])
            .arg(&file)
            .output();
        if !matches!(generated, Ok(out) if out.status.success()) {
            let _ = std::fs::remove_dir_all(&dir);
            return; // 这个 ffmpeg 没有 mp3 编码器
        }

        let meta = TrackMeta {
            title: "稻香".to_string(),
            artist: "周杰伦".to_string(),
            album: "魔杰座".to_string(),
        };
        // 1x1 的 png，只为验证封面能写进去（体积与内容无关）
        let png: Vec<u8> = vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0,
        ];
        let result = write_tags(
            &file,
            &meta,
            Some(CoverArt {
                mime: sniff_image_mime(&png),
                bytes: png,
            }),
        );
        assert!(result.is_ok(), "写入标签应成功: {result:?}");

        let tagged = read_from_path(&file).expect("重新读取文件");
        let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
        let tag = tag.expect("文件里应有标签");
        assert_eq!(tag.title().as_deref(), Some("稻香"));
        assert_eq!(tag.artist().as_deref(), Some("周杰伦"));
        assert_eq!(tag.album().as_deref(), Some("魔杰座"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
