use super::*;

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
pub(super) fn find_content_item_brace(text: &str, from: usize) -> Option<usize> {
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
pub(super) fn extract_json_object(text: &str, start: usize) -> Option<&str> {
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
pub(super) fn parse_content_item(item: &serde_json::Value) -> Option<SunoSong> {
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
