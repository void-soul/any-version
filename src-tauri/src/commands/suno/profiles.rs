use super::*;

/// 解析 Suno 用户主页，返回歌曲列表。
#[tauri::command]
pub async fn suno_parse_profile(url: String) -> Result<Vec<SunoSong>, String> {
    crate::runtime_tasks::run_if_enabled("suno", async move {
        let trimmed = url.trim();
        if !is_profile_url(trimmed) {
            return Err(
                "只支持 Suno 用户主页（形如 https://suno.com/@用户名），不接受单曲页或其他链接"
                    .to_string(),
            );
        }
        let url = normalize_fetch_url(trimmed);
        eprintln!("[Suno] 用户输入: {trimmed}");
        eprintln!("[Suno] 实际请求: {url}");
        eprintln!(
            "[Suno] 网络路径: {}",
            match system_proxy() {
                Some(p) => format!("系统代理 {p}（失败则回退直连）"),
                None => "直连（未检测到系统代理）".to_string(),
            }
        );

        // 先走带代理的客户端；网络层失败时把代理判定作废并用直连重试一次。
        // 代理是可选加速路径，不是前置条件——固化一个没监听的代理端口会让功能整体不可用。
        let client = http_client();
        let resp = match client
            .get(&url)
            .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[Suno] 经系统代理请求失败（{e}），作废代理判定并改走直连重试");
                invalidate_proxy_cache();
                let fallback = direct_client();
                match fallback
                    .get(&url)
                    .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e2) => {
                        eprintln!("[Suno] 直连也失败: {e2}");
                        return Err(format!("请求主页失败（代理 {}；直连 {}）: {e2}", e, e2));
                    }
                }
            }
        };
        let status = resp.status();
        // 打 `resp.url()` 而不是请求串：重定向后的最终地址才是真正生效的那个
        eprintln!(
            "[Suno] 主页响应 HTTP {status}（协议 {:?}，最终地址 {}）",
            resp.version(),
            resp.url()
        );
        if !status.is_success() {
            return Err(format!("主页返回 HTTP {status}"));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| format!("读取主页失败: {e}"))?;
        eprintln!("[Suno] 主页 HTML 大小 {} 字节", html.len());
        eprintln!("[Suno] 含 content_item: {}", html.contains("content_item"));
        eprintln!("[Suno] 含 media_urls: {}", html.contains("media_urls"));
        eprintln!("[Suno] 含 .m4a: {}", html.contains(".m4a"));
        // 完整 HTML 落到临时文件，便于排查「页面结构变化」
        let debug_path = std::env::temp_dir().join("suno_debug.html");
        if std::fs::write(&debug_path, &html).is_ok() {
            eprintln!("[Suno] 已保存 HTML 到 {}", debug_path.display());
        }
        let head: String = html.chars().take(800).collect();
        eprintln!("[Suno] HTML 开头 800 字符:\n{head}");
        let mut songs = parse_profile_html(&html)?;

        // 主页 SSR 只内嵌前 ~20 首，其余靠翻页接口补齐。
        // 翻页失败不当作错误：宁可少几首，也别让已经解析出来的结果全废掉。
        match extract_target_user_id(&html) {
            Some(uid) => match fetch_all_songs(&client, &uid).await {
                Ok(paged) if !paged.is_empty() => {
                    eprintln!("[Suno] 翻页补齐 {} 首（接口）", paged.len());
                    let mut seen: std::collections::HashSet<String> =
                        songs.iter().map(|s| s.id.clone()).collect();
                    for song in paged {
                        if seen.insert(song.id.clone()) {
                            songs.push(song);
                        }
                    }
                }
                Ok(_) => eprintln!("[Suno] 翻页接口没返回歌曲，沿用 SSR 结果"),
                Err(e) => eprintln!("[Suno] 翻页失败（沿用 SSR 的 {} 首）：{e}", songs.len()),
            },
            None => eprintln!("[Suno] 未从页面提取到 target_user_id，跳过翻页"),
        }

        // 标记已下载（本地缓存）
        let downloaded = load_downloaded();
        for s in songs.iter_mut() {
            s.downloaded = downloaded.contains(&s.id);
        }
        eprintln!(
            "[Suno] 解析到 {} 首，其中已下载 {} 首",
            songs.len(),
            songs.iter().filter(|s| s.downloaded).count()
        );
        Ok(songs)
    })
    .await
}

/// 列出收藏的主页。
#[tauri::command]
pub fn suno_list_profiles() -> Vec<String> {
    load_profiles()
}

/// 收藏一个主页（去重），返回更新后的收藏列表。
#[tauri::command]
pub fn suno_save_profile(url: String) -> Result<Vec<String>, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("主页 URL 不能为空".to_string());
    }
    let normalized = normalize_profile_url(&url);
    let mut profiles = load_profiles();
    if !profiles
        .iter()
        .any(|p| normalize_profile_url(p) == normalized)
    {
        profiles.push(url);
        save_profiles(&profiles)?;
    }
    Ok(profiles)
}

/// 取消收藏一个主页，返回更新后的收藏列表。
#[tauri::command]
pub fn suno_remove_profile(url: String) -> Result<Vec<String>, String> {
    let normalized = normalize_profile_url(&url);
    let mut profiles = load_profiles();
    profiles.retain(|p| normalize_profile_url(p) != normalized);
    save_profiles(&profiles)?;
    Ok(profiles)
}
