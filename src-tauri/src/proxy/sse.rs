//! SSE 流式解析工具

/// 从 buffer 中提取一个完整的 SSE 块（以 \n\n 或 \r\n\r\n 分隔）
pub fn take_sse_block(buffer: &str) -> Option<(String, &str)> {
    // 查找块分隔符
    if let Some(pos) = buffer.find("\n\n") {
        let block = &buffer[..pos];
        let remainder = &buffer[pos + 2..];
        return Some((block.to_string(), remainder));
    }
    if let Some(pos) = buffer.find("\r\n\r\n") {
        let block = &buffer[..pos];
        let remainder = &buffer[pos + 4..];
        return Some((block.to_string(), remainder));
    }
    None
}

/// 从 SSE 块中提取 data 字段值
pub fn extract_sse_data(block: &str) -> Option<String> {
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            let data = rest.strip_prefix(" ").unwrap_or(rest);
            if !data.is_empty() && data != "[DONE]" {
                return Some(data.to_string());
            }
            if data == "[DONE]" {
                return Some("[DONE]".to_string());
            }
        }
    }
    None
}

/// 从 SSE 块中提取 event 字段值
pub fn extract_sse_event(block: &str) -> Option<String> {
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            let event = rest.strip_prefix(" ").unwrap_or(rest);
            if !event.is_empty() {
                return Some(event.to_string());
            }
        }
    }
    None
}

/// 安全地将新字节追加到 UTF-8 buffer，处理跨 chunk 的多字节字符
pub fn append_utf8_safe(buffer: &str, new_bytes: &[u8]) -> String {
    let mut combined = buffer.to_string();
    match std::str::from_utf8(new_bytes) {
        Ok(s) => {
            combined.push_str(s);
        }
        Err(e) => {
            // 截取有效的 UTF-8 部分
            let valid_up_to = e.valid_up_to();
            if valid_up_to > 0 {
                combined.push_str(std::str::from_utf8(&new_bytes[..valid_up_to]).unwrap());
            }
            // 剩余的不完整字节留给下一个 chunk 处理
        }
    }
    combined
}
