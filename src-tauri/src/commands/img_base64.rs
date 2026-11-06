use std::path::Path;

use base64::Engine;
use std::io::Read;
/// 与前端 `src/components/imageBase64Data.ts` 一致（AUDIT-033）。
const IMAGE_BASE64_MAX: usize =
    crate::commands::foundation::defaults::sizes::IMAGE_BASE64_MAX as usize;
const MAX_BASE64_CHARS: usize = ((IMAGE_BASE64_MAX + 2) / 3) * 4 + 256;

fn encode_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    if input.len() > MAX_BASE64_CHARS { return Err("图片不能超过16 MiB".into()); }
    let mut clean: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if clean.is_empty() { return Err("Base64不能为空".into()); }
    if !clean.contains('=') { while clean.len() % 4 != 0 { clean.push('='); } }
    let bytes = base64::engine::general_purpose::STANDARD.decode(clean).map_err(|e| format!("Base64格式错误: {e}"))?;
    if bytes.len() > IMAGE_BASE64_MAX { return Err("图片不能超过16 MiB".into()); }
    Ok(bytes)
}

#[tauri::command]
pub fn image_to_base64(file_path: String) -> Result<String, String> {
    let file = std::fs::File::open(&file_path).map_err(|e| format!("读取图片文件失败: {e}"))?;
    let mut bytes = Vec::new();
    file.take(IMAGE_BASE64_MAX as u64 + 1).read_to_end(&mut bytes).map_err(|e| format!("读取图片文件失败: {e}"))?;
    if bytes.len() > IMAGE_BASE64_MAX { return Err("图片不能超过16 MiB".into()); }
    
    let path = Path::new(&file_path);
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or_default().to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        _ => return Err("不支持的图片格式".into()),
    };

    let b64 = encode_base64(&bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}

#[tauri::command]
pub fn save_base64_image(base64_str: String, file_path: String) -> Result<(), String> {
    if base64_str.len() > MAX_BASE64_CHARS { return Err("图片不能超过16 MiB".into()); }
    let clean_base64 = if let Some(pos) = base64_str.find(";base64,") {
        &base64_str[pos + 8..]
    } else {
        &base64_str
    };

    let bytes = decode_base64(clean_base64)?;

    crate::commands::config::atomic_write_file(Path::new(&file_path), &bytes)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_padding_and_preserves_unpadded_compatibility() {
        for invalid in ["T=Q=", "TQ=", "TQ===", "TR==", "A"] { assert!(decode_base64(invalid).is_err(), "{invalid}"); }
        assert_eq!(decode_base64("TQ").unwrap(), b"M");
        assert_eq!(decode_base64(" TQ==\n").unwrap(), b"M");
        assert_eq!(encode_base64(b"Man"), "TWFu");
    }
    #[test]
    fn rejects_oversized_base64_before_decoding() {
        assert!(decode_base64(&"A".repeat(MAX_BASE64_CHARS + 1)).is_err());
    }
}
