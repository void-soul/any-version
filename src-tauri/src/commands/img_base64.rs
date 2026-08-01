use std::path::Path;

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_base64(bytes: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };

        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);

        let c0 = (n >> 18) & 63;
        let c1 = (n >> 12) & 63;
        let c2 = (n >> 6) & 63;
        let c3 = n & 63;

        result.push(BASE64_CHARS[c0 as usize] as char);
        result.push(BASE64_CHARS[c1 as usize] as char);
        if i + 1 < bytes.len() {
            result.push(BASE64_CHARS[c2 as usize] as char);
        } else {
            result.push('=');
        }
        if i + 2 < bytes.len() {
            result.push(BASE64_CHARS[c3 as usize] as char);
        } else {
            result.push('=');
        }
        i += 3;
    }
    result
}

fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = 0u32;
    let mut count = 0;
    for c in s.chars() {
        if c.is_whitespace() || c == '=' {
            continue;
        }
        let val = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => return Err(format!("无效的 Base64 字符: {}", c)),
        };
        buffer = (buffer << 6) | val;
        count += 1;
        if count == 4 {
            bytes.push((buffer >> 16) as u8);
            bytes.push((buffer >> 8) as u8);
            bytes.push(buffer as u8);
            buffer = 0;
            count = 0;
        }
    }
    if count == 2 {
        bytes.push((buffer >> 4) as u8);
    } else if count == 3 {
        bytes.push((buffer >> 10) as u8);
        bytes.push((buffer >> 2) as u8);
    } else if count != 0 {
        return Err("Base64 字符串长度不正确".to_string());
    }
    Ok(bytes)
}

#[tauri::command]
pub fn image_to_base64(file_path: String) -> Result<String, String> {
    let bytes = std::fs::read(&file_path)
        .map_err(|e| format!("读取图片文件失败: {}", e))?;
    
    let path = Path::new(&file_path);
    let mime = match path.extension().and_then(|s| s.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        _ => "image/png",
    };

    let b64 = encode_base64(&bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}

#[tauri::command]
pub fn save_base64_image(base64_str: String, file_path: String) -> Result<(), String> {
    let clean_base64 = if let Some(pos) = base64_str.find(";base64,") {
        &base64_str[pos + 8..]
    } else {
        &base64_str
    };

    let bytes = decode_base64(clean_base64)?;

    std::fs::write(file_path, bytes)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    Ok(())
}
