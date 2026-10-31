use super::*;

/// 抓取 URL 元数据的完整链路结果（命令返回结构）。
#[derive(serde::Serialize, Clone, Debug, Default)]
pub struct FetchedMeta {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub favicon_url: Option<String>,
    /// 页面正文（纯文本，提取自主内容区）；无法提取时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// 命中来源：webview（应用内隐藏窗口）/ browser（无头渲染）/ http（普通请求）/ none
    pub source: String,
}

impl From<UrlMetadataRich> for FetchedMeta {
    fn from(r: UrlMetadataRich) -> Self {
        FetchedMeta {
            title: r.title,
            description: r.description,
            image_url: r.image_url,
            favicon_url: r.favicon_url,
            content: r.content,
            source: "none".into(),
        }
    }
}

/// 抓取 URL 元数据（完整链路：应用内隐藏 WebView → 无头浏览器渲染 → 普通 HTTP）。
/// 供「新增收藏」表单的抓取按钮使用——原先只走单次 HTTP 请求，JS 渲染页面拿不到
/// 真实标题；现与书签「刷新元数据」共用同一链路。
#[tauri::command]
pub async fn picky_fetch_url_meta(
    app: tauri::AppHandle,
    url: String,
) -> Result<FetchedMeta, String> {
    crate::runtime_tasks::run_if_enabled("picky", async move {
        let mut url = url.trim().to_string();
        if url.is_empty() {
            return Err("URL 不能为空".into());
        }
        if !url.contains("://") {
            url = format!("https://{url}");
        }
        // 1) 应用内隐藏 WebView（首选）
        if let Some(r) = fetch_meta_via_webview(&app, &url).await {
            let mut m: FetchedMeta = r.into();
            m.source = "webview".into();
            return Ok(m);
        }
        // 2) 无头浏览器渲染
        if let Some(dom) = render_dom_via_browser(&url).await {
            let r = parse_rendered_meta(&dom, &url);
            if r.title.is_some() || r.description.is_some() || r.image_url.is_some() {
                let mut m: FetchedMeta = r.into();
                m.source = "browser".into();
                return Ok(m);
            }
        }
        // 3) 普通 HTTP（窄兜底：仅标题/favicon）
        if let Ok(m) = crate::commands::launcher::windows::fetch_url_metadata_with_timeout(
            &url,
            std::time::Duration::from_secs(10),
        )
        .await
        {
            if !m.title.is_empty() {
                return Ok(FetchedMeta {
                    title: Some(m.title),
                    favicon_url: m.icon,
                    source: "http".into(),
                    ..Default::default()
                });
            }
        }
        Err("无法抓取页面元数据（站点可能拦截了请求，或需要登录）".into())
    })
    .await
}

/// 解析渲染后 DOM 的元数据：标题 / 描述 / 图片 / favicon。
/// 全部来自浏览器渲染结果（可抓 JS SPA），返回相对路径均已按 base 展开。
pub(super) fn parse_rendered_meta(dom: &str, url: &str) -> UrlMetadataRich {
    let base = reqwest::Url::parse(url).ok();
    let resolve = |s: &str| -> Option<String> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let Some(base) = &base {
            if let Ok(u) = base.join(s) {
                return Some(u.to_string());
            }
        }
        Some(s.to_string())
    };

    // title：og:title 优先，否则 <title>
    let title = meta_content(dom, |n| n.eq_ignore_ascii_case("og:title"))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let re = regex::Regex::new(r"(?is)<title\b[^>]*>(.*?)</title>").ok()?;
            re.captures(dom)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
                .filter(|s| !s.is_empty())
        });

    let description = meta_content(dom, |n| n.eq_ignore_ascii_case("og:description"))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| meta_content(dom, |n| n.eq_ignore_ascii_case("description")))
        .map(|s| s.trim().to_string());

    let image_url =
        meta_content(dom, |n| n.eq_ignore_ascii_case("og:image")).and_then(|s| resolve(&s));

    // favicon：link[rel~=icon] 的 href，否则默认 /favicon.ico
    let favicon_url = {
        let re = regex::Regex::new(r"(?is)<link\b[^>]*>");
        let mut found = None;
        if let Ok(re) = re {
            for caps in re.captures_iter(dom) {
                let tag_lc = caps.get(0).map(|m| m.as_str()).unwrap_or("").to_lowercase();
                if tag_lc.contains("rel=\"icon\"")
                    || tag_lc.contains("rel='icon'")
                    || tag_lc.contains("rel=\"shortcut icon\"")
                {
                    if let Ok(hre) = regex::Regex::new(r#"href=["']([^"']+)["']"#) {
                        if let Some(m) = hre.captures(&tag_lc) {
                            found = m.get(1).map(|mm| mm.as_str().to_string());
                            break;
                        }
                    }
                }
            }
        }
        found.and_then(|s| resolve(&s)).or_else(|| {
            base.as_ref()
                .and_then(|b| b.join("/favicon.ico").ok().map(|u| u.to_string()))
        })
    };

    // 正文（无头浏览器兜底路径）：从渲染后 DOM 提取主内容纯文本
    let content = extract_content_from_dom(dom).filter(|s| !s.trim().is_empty());

    UrlMetadataRich {
        title,
        description,
        image_url,
        favicon_url,
        content,
    }
}

/// 从渲染后的 HTML DOM 提取正文纯文本（无头浏览器兜底路径用）。
/// 启发式（与 WebView JS 主路径对齐的简化版）：
/// 1) 优先取 `<article>` / `<main>` / `[role=main]` 的内部；
/// 2) 否则剥离 script/style/nav/header/footer/aside/form/按钮等噪声块后取整页文本；
/// 3) 去除 HTML 标签、解码实体、折叠空白。提取不到有意义文本时返回 None。
pub(super) fn extract_content_from_dom(dom: &str) -> Option<String> {
    let lower = dom.to_ascii_lowercase();

    // 1) 定位主内容容器（article / main / role=main）
    let main_inner = || -> Option<String> {
        for pat in [
            r"(?is)<article\b[^>]*>(.*?)</article>",
            r"(?is)<main\b[^>]*>(.*?)</main>",
            r#"(?is)<[^>]+\brole=["']main["'][^>]*>(.*?)</[^>]+>"#,
        ] {
            if let Ok(re) = regex::Regex::new(pat) {
                if let Some(caps) = re.captures(&lower) {
                    if let Some(m) = caps.get(1) {
                        let text = html_to_text(m.as_str());
                        if text.chars().count() >= 40 {
                            return Some(text);
                        }
                    }
                }
            }
        }
        None
    };
    if let Some(text) = main_inner() {
        return Some(text);
    }

    // 2) 剥离噪声块，再取整页文本
    let mut cleaned = dom.to_string();
    for tag in [
        "script", "style", "noscript", "iframe", "svg", "nav", "header", "footer", "aside", "form",
        "button",
    ] {
        if let Ok(re) = regex::Regex::new(&format!(r"(?is)<{tag}\b[^>]*>.*?</{tag}>")) {
            cleaned = re.replace_all(&cleaned, "").to_string();
        }
    }
    let text = html_to_text(&cleaned);
    if text.chars().count() >= 40 {
        Some(text)
    } else {
        None
    }
}

/// 把一段 HTML 片段转为纯文本：去标签、解码常见实体、折叠空白。
pub(super) fn html_to_text(html: &str) -> String {
    let mut s = html.to_string();
    // 块级标签转换行（保留段落结构）
    if let Ok(re) =
        regex::Regex::new(r"(?i)</?(p|div|section|article|li|h[1-6]|br|tr|blockquote|pre)[^>]*>")
    {
        s = re.replace_all(&s, "\n").to_string();
    }
    // 去除所有标签
    if let Ok(re) = regex::Regex::new(r"(?s)<[^>]+>") {
        s = re.replace_all(&s, " ").to_string();
    }
    // 解码常见 HTML 实体
    for (ent, ch) in [
        ("&nbsp;", " "),
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&hellip;", "…"),
    ] {
        s = s.replace(ent, ch);
    }
    // 十进制/十六进制字符实体
    if let Ok(re) = regex::Regex::new(r"&#(\d+);") {
        s = re
            .replace_all(&s, |c: &regex::Captures| {
                let n: u32 = c
                    .get(1)
                    .map(|m| m.as_str().parse().unwrap_or(0))
                    .unwrap_or(0);
                char::from_u32(n)
                    .map(|ch| ch.to_string())
                    .unwrap_or_default()
            })
            .to_string();
    }
    if let Ok(re) = regex::Regex::new(r"&#x([0-9a-fA-F]+);") {
        s = re
            .replace_all(&s, |c: &regex::Captures| {
                let n = u32::from_str_radix(c.get(1).map(|m| m.as_str()).unwrap_or(""), 16)
                    .unwrap_or(0);
                char::from_u32(n)
                    .map(|ch| ch.to_string())
                    .unwrap_or_default()
            })
            .to_string();
    }
    // 折叠空白：行内空白合并为单空格，多余空行合并
    let lines: Vec<String> = s
        .lines()
        .map(|l| {
            let l = l.split_whitespace().collect::<Vec<_>>().join(" ");
            l
        })
        .collect();
    let mut out = String::new();
    let mut prev_empty = false;
    for l in lines {
        if l.is_empty() {
            if !prev_empty {
                out.push('\n');
            }
            prev_empty = true;
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&l);
            prev_empty = false;
        }
    }
    out.trim().to_string()
}

#[derive(Default, Clone, Debug)]
pub(super) struct UrlMetadataRich {
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) image_url: Option<String>,
    pub(super) favicon_url: Option<String>,
    pub(super) content: Option<String>,
}
