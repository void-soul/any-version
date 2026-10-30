use super::*;

// ─── 应用内隐藏 WebView 抓取（Tauri 版「iframe」方案） ───
//
// 为什么不用纯 iframe：跨域页面的 iframe 受浏览器同源策略限制，父文档读不到
// 其 DOM，无法提取 title/OG 标签。Tauri 的等价实现是「隐藏 WebviewWindow」：
// 复用 App 自身的 WebView2 引擎，在后端直接对窗口 evaluate_script 取回数据。
// 相比外部无头 Edge 进程的优势：
// - 无 profile 锁 / 冷启动 / 孤儿进程 / HeadlessChrome UA 被检测等问题
// - 即本机真实 WebView 环境，对反爬/懒加载 SPA 兼容性更好
// 提取 JS 必须 try/catch 后返回 JSON 字符串（Windows 下 eval 异常会被吞，
// 文档明确要求自行捕获并返回字符串）。

pub(super) const FETCH_VIEW_LABEL: &str = "picky-fetch-view";

pub(super) const WEBVIEW_EXTRACT_JS: &str = r#"(function () {
  try {
    function meta(p) {
      var el = document.querySelector('meta[property="' + p + '"]') || document.querySelector('meta[name="' + p + '"]');
      return el ? (el.getAttribute('content') || '').trim() : null;
    }
    function og(names) { for (var i = 0; i < names.length; i++) { var v = meta(names[i]); if (v) return v; } return null; }
    function abs(u) { if (!u) return null; try { return new URL(u, location.href).href; } catch (e) { return null; } }
    var iconEl = document.querySelector('link[rel~="icon"]') || document.querySelector('link[rel~="shortcut icon"]') || document.querySelector('link[rel~="apple-touch-icon"]');
    var imgLink = document.querySelector('link[rel="image_src"]');

    // ── 正文提取（与专用 APP WebView 引擎同一套启发式）──
    function cleanText(s){ return (s||'').replace(/\s+/g,' ').trim(); }
    function extractContent() {
      try {
        var host = (location.hostname||'').toLowerCase();
        var isZhihu = /zhihu\.com$/.test(host);
        var isWechat = /qq\.com$/.test(host);
        var root = null;
        if (isZhihu) {
          root = document.querySelector('.RichText')
              || document.querySelector('.QuestionAnswer-content')
              || document.querySelector('.Post-RichText')
              || document.querySelector('.QuestionHeader-description');
        } else if (isWechat) {
          root = document.querySelector('#js_content');
        }
        if (!root) {
          var sels = ['article','main','[role="main"]',
            '.content','.post-content','.article','.article-content','.article-body',
            '.markdown-body','.post-body','.entry-content','.rich-text','.richtext',
            '.news-content','.article-detail','.read-content','.post-text',
            '.article-text','.text-content','#content','#article',
            '#articleContent','#main-content'];
          for (var i=0;i<sels.length;i++){ var e=document.querySelector(sels[i]); if(e && cleanText(e.innerText)){ root=e; break; } }
        }
        if (!root) {
          var blocks = document.querySelectorAll('div,section,article,main');
          var best=null, bestScore=0;
          for (var j=0;j<blocks.length;j++){
            var b=blocks[j]; var t=cleanText(b.innerText);
            if (t.length < 40) continue;
            var p=b.querySelectorAll('p').length;
            var score=t.length + p*120;
            if (score>bestScore){ bestScore=score; best=b; }
          }
          root=best;
        }
        if (!root) root = document.body;
        if (!root) return '';
        var c = root.cloneNode(true);
        var junk = [];
        if (isZhihu) {
          junk = ['script','style','noscript','iframe','svg','button','[role="button"]',
            '.VoteButton','.ContentItem-actions','.AuthorInfo','.CommentArea',
            '.Sticky','.GlobalSideBar','.Pc-card','.QuestionHeader','.Modal-wrapper',
            '.Topbar','.CornerButtons','.HotspotModal','.KfeCollection','.ListHeader',
            '.Question-main .List','.RelatedReadings','.Promotions'];
        } else if (isWechat) {
          junk = ['script','style','noscript','iframe','svg','button','[role="button"]',
            '.rich_media_tool','.qr_code_pc','.reward_area','.discuss_container',
            '.share_area','.bottom_container','.ct_mpda_wrp','#js_sg_bar',
            '#js_pc_qr_code','.mpda_bottom_container','.recommend_area',
            '.related-article','.comment_input'];
        } else {
          junk = ['script','style','noscript','iframe','svg','nav','header','footer',
            'aside','form','button','[role="button"]','.ad','.ads','.comment',
            '.comments','[aria-hidden="true"]'];
        }
        for (var x=0;x<junk.length;x++){
          var ns = c.querySelectorAll(junk[x]);
          for (var k=0;k<ns.length;k++){ if(ns[k] && ns[k].parentNode) ns[k].parentNode.removeChild(ns[k]); }
        }
        var as = c.querySelectorAll('a');
        for (var m=0;m<as.length;m++){
          var a = as[m];
          var at = cleanText(a.textContent);
          if (at.length <= 10 && /(展开|收起|全文|编辑于|阅读原文|查看|更多|详情|登录|关注|赞同|反对|赞|收藏|分享|评论|举报|投诉)/.test(at)) {
            if (a.parentNode) a.parentNode.removeChild(a);
          } else {
            a.replaceWith(document.createTextNode(a.textContent));
          }
        }
        return (c.innerText || '').trim();
      } catch (e) { return ''; }
    }
    var content = extractContent();

    return JSON.stringify({
      title: (document.title || '').trim() || null,
      description: og(['og:description', 'description', 'twitter:description']),
      image_url: abs(og(['og:image', 'twitter:image']) || (imgLink && imgLink.getAttribute('href'))),
      favicon_url: abs(iconEl && iconEl.getAttribute('href')),
      content: content || null
    });
  } catch (e) {
    return JSON.stringify({ error: String(e) });
  }
})();
"#;

/// 串行化元数据抓取（新增收藏自动补全与手动重抓共用同一隐藏窗口，避免并发建窗）。
pub(super) static WEBVIEW_FETCH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 用应用内隐藏 WebView 抓取页面元数据。
/// 流程：建隐藏窗口 → 轮询 document.readyState 至 complete（≤18s）→ 静置 2.5s
/// （等 SPA 标题/OG 落定）→ eval_with_callback 提取 JSON → 销毁窗口。
/// 总预算约 26s；任一步失败返回 None，由调用方决定是否走无头浏览器/HTTP 兜底。
pub(super) async fn fetch_meta_via_webview(
    app: &tauri::AppHandle,
    url: &str,
) -> Option<UrlMetadataRich> {
    use tokio::sync::oneshot;

    let _guard = WEBVIEW_FETCH_LOCK.lock().await;

    // 每次用全新窗口（destroy 旧窗避免上一页面状态残留；等待其真正销毁再建同名窗）
    if let Some(w) = app.get_webview_window(FETCH_VIEW_LABEL) {
        let _ = w.destroy();
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    let parsed = reqwest::Url::parse(url).ok()?;
    let w = match tauri::WebviewWindowBuilder::new(
        app,
        FETCH_VIEW_LABEL,
        tauri::WebviewUrl::External(parsed),
    )
    .title("picky-fetch")
    .inner_size(1280.0, 900.0)
    .visible(false)
    .decorations(false)
    .skip_taskbar(true)
    .build()
    {
        Ok(w) => w,
        Err(e) => {
            crate::exit_log::exit_log(&format!("[picky-wv] 隐藏窗口创建失败: {e} url={url}"));
            return None;
        }
    };
    let cleanup = |app: &tauri::AppHandle| {
        if let Some(w) = app.get_webview_window(FETCH_VIEW_LABEL) {
            let _ = w.destroy();
        }
    };

    // 1) 等文档 complete（隐藏窗口中 JS 正常执行；readyState 轮询经 eval_with_callback 回传）
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(18);
    let mut ready = false;
    loop {
        let (tx, rx) = oneshot::channel::<String>();
        // eval_with_callback 的回调是 Fn（可能被调用多次），用 Arc<Mutex<Option>> 保证只发一次
        let send = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        let send2 = send.clone();
        if w.eval_with_callback("document.readyState", move |s| {
            if let Some(t) = send2.lock().unwrap().take() {
                let _ = t.send(s);
            }
        })
        .is_err()
        {
            cleanup(app);
            return None;
        }
        match tokio::time::timeout(std::time::Duration::from_secs(3), rx).await {
            Ok(Ok(s)) if s.contains("complete") => {
                ready = true;
                break;
            }
            Ok(Ok(_)) => { /* loading / interactive：继续轮询 */ }
            _ => { /* 回传超时：继续直到 deadline */ }
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    if !ready {
        crate::exit_log::exit_log(&format!("[picky-wv] 等待 complete 超时 url={url}"));
        cleanup(app);
        return None;
    }

    // 2) 静置：SPA 常在 DOM complete 后才写入 title/OG 标签
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;

    // 3) 提取元数据（eval 结果会被再序列化一层：JS 返回字符串 → 回调收到带引号的 JSON 串）
    let (tx, rx) = oneshot::channel::<String>();
    let send = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
    let send2 = send.clone();
    if w.eval_with_callback(WEBVIEW_EXTRACT_JS, move |s| {
        if let Some(t) = send2.lock().unwrap().take() {
            let _ = t.send(s);
        }
    })
    .is_err()
    {
        cleanup(app);
        return None;
    }
    let raw = match tokio::time::timeout(std::time::Duration::from_secs(6), rx).await {
        Ok(Ok(s)) => s,
        _ => {
            crate::exit_log::exit_log(&format!("[picky-wv] 提取回传超时 url={url}"));
            cleanup(app);
            return None;
        }
    };
    cleanup(app);

    let outer: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let inner: serde_json::Value = match outer {
        serde_json::Value::String(s) => match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(_) => return None,
        },
        other => other,
    };
    if inner.get("error").is_some() {
        crate::exit_log::exit_log(&format!(
            "[picky-wv] 页面提取异常: {} url={url}",
            inner.get("error").and_then(|v| v.as_str()).unwrap_or("")
        ));
        return None;
    }
    let meta = UrlMetadataRich {
        title: inner
            .get("title")
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.trim().is_empty()),
        description: inner
            .get("description")
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.trim().is_empty()),
        image_url: inner
            .get("image_url")
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.is_empty()),
        favicon_url: inner
            .get("favicon_url")
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.is_empty()),
        content: inner
            .get("content")
            .and_then(|v| v.as_str())
            .map(String::from)
            .filter(|s| !s.trim().is_empty()),
    };
    if meta.title.is_none()
        && meta.description.is_none()
        && meta.image_url.is_none()
        && meta.content.is_none()
    {
        crate::exit_log::exit_log(&format!("[picky-wv] 未提取到有效元数据 url={url}"));
        return None;
    }
    crate::exit_log::exit_log(&format!(
        "[picky-wv] 成功 title={:?} content_len={} url={url}",
        meta.title
            .as_deref()
            .map(|t| t.chars().take(40).collect::<String>()),
        meta.content.as_ref().map(|c| c.len()).unwrap_or(0)
    ));
    Some(meta)
}
