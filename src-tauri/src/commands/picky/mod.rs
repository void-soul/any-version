//! Picky 模块：收藏 / 归档页面（与 Flutter 端 picky 保持同一数据接口）。
//!
//! 数据字段（camelCase，与 Flutter 模型 toJson 一致）：
//! - 页面信息：id / title / description / url / imageUrl / faviconUrl / createdAt / updatedAt / metaFetched
//! - 状态：refined（false=收藏中，true=已归档/炼化）
//! - 评论：id / bookmarkId / content / createdAt / updatedAt / parentId（树形，2 级）
//! - 标签：id / name / color / createdAt，书签-标签关联 bookmarkTags: { bookmarkId: [tagId...] }
//!
//! 云同步：S3 兼容存储（SigV4），state.json 结构与 Flutter 端完全一致，可跨端互通。
//! 收藏中的 lingzuCode / aiCopy 等个性化字段不做处理，但会原样保留（extra），
//! 避免与 Flutter 端来回同步时丢失其数据。

mod s3;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::commands::config::get_data_dir;
pub use s3::PickySyncConfig;

// ─── 模型（camelCase，与 Flutter 端一致） ───

/// 与 Flutter 端 `toJson` 的 0/1 整数表示互转：
/// - 反序列化：兼容 `0/1` 整数与 `true/false` 布尔（Flutter 输出 0/1，旧数据可能是布尔）；
/// - 序列化：统一输出 0/1 整数，保证 any-version 上传的数据 Flutter 端 `(x as int?)==1` 可读。
fn de_bool_01<'de, D>(d: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_i64().map(|x| x != 0).unwrap_or(false),
        serde_json::Value::Null => false,
        _ => false,
    })
}

fn ser_bool_01<S>(b: &bool, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_i64(if *b { 1 } else { 0 })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub favicon_url: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default, deserialize_with = "de_bool_01", serialize_with = "ser_bool_01")]
    pub refined: bool,
    #[serde(default, deserialize_with = "de_bool_01", serialize_with = "ser_bool_01")]
    pub meta_fetched: bool,
    /// 保留 Flutter 端的个性化等未知字段（lingzuCode / aiCopy 等），同步时原样带回。
    #[serde(flatten)]
    pub extra: JsonMap<String, JsonValue>,
}

impl Default for Bookmark {
    fn default() -> Self {
        let now = now_iso();
        Self {
            id: uuid_v4(),
            title: String::new(),
            description: None,
            url: None,
            image_url: None,
            favicon_url: None,
            created_at: now.clone(),
            updated_at: now,
            refined: false,
            meta_fetched: false,
            extra: JsonMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    #[serde(default)]
    pub bookmark_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_tag_color")]
    pub color: String,
    #[serde(default)]
    pub created_at: String,
}

fn default_tag_color() -> String {
    "#4FC3F7".to_string()
}

/// 全量状态（前端一次性加载 + 同步用）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PickyState {
    pub bookmarks: Vec<Bookmark>,
    pub comments: Vec<Comment>,
    pub tags: Vec<Tag>,
    pub bookmark_tags: HashMap<String, Vec<String>>,
}

// ─── 工具 ───

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 生成 UUID v4（与 Flutter uuid 包格式一致）。
fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::getrandom(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{}-{}-{}-{}-{}",
        hex(&bytes[0..4]),
        hex(&bytes[4..6]),
        hex(&bytes[6..8]),
        hex(&bytes[8..10]),
        hex(&bytes[10..16])
    )
}

// ─── SQLite 存储 ───

fn db_path() -> std::path::PathBuf {
    get_data_dir().join("picky").join("picky.db")
}

fn open_db() -> Result<rusqlite::Connection, String> {
    let dir = get_data_dir().join("picky");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 Picky 数据目录失败: {}", e))?;
    let conn = rusqlite::Connection::open(db_path())
        .map_err(|e| format!("打开 Picky 数据库失败: {}", e))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS picky_bookmarks (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            description TEXT,
            url TEXT,
            image_url TEXT,
            favicon_url TEXT,
            created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT '',
            refined INTEGER NOT NULL DEFAULT 0,
            meta_fetched INTEGER NOT NULL DEFAULT 0,
            extra TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE IF NOT EXISTS picky_comments (
            id TEXT PRIMARY KEY,
            bookmark_id TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT '',
            parent_id TEXT
        );
        CREATE TABLE IF NOT EXISTS picky_tags (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            color TEXT NOT NULL DEFAULT '#4FC3F7',
            created_at TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS picky_bookmark_tags (
            bookmark_id TEXT NOT NULL,
            tag_id TEXT NOT NULL,
            PRIMARY KEY (bookmark_id, tag_id)
        );
        CREATE TABLE IF NOT EXISTS picky_sync_config (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            endpoint TEXT,
            region TEXT NOT NULL DEFAULT 'us-east-1',
            access_key_id TEXT NOT NULL DEFAULT '',
            secret_enc TEXT NOT NULL DEFAULT '',
            bucket_name TEXT NOT NULL DEFAULT '',
            prefix TEXT,
            enabled INTEGER NOT NULL DEFAULT 0,
            last_sync_at TEXT,
            addressing_style TEXT NOT NULL DEFAULT 'auto',
            tls_verify INTEGER NOT NULL DEFAULT 1,
            timeout_seconds INTEGER NOT NULL DEFAULT 30,
            concurrent_reqs INTEGER NOT NULL DEFAULT 1
        );",
    )
    .map_err(|e| format!("初始化 Picky 表失败: {}", e))?;
    Ok(conn)
}

// ─── Bookmark 读写 ───

fn bookmark_from_row(row: &rusqlite::Row) -> rusqlite::Result<Bookmark> {
    let extra_raw: String = row.get(10)?;
    let extra: JsonMap<String, JsonValue> = serde_json::from_str(&extra_raw).unwrap_or_default();
    Ok(Bookmark {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        url: row.get(3)?,
        image_url: row.get(4)?,
        favicon_url: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        refined: row.get::<_, i64>(8)? != 0,
        meta_fetched: row.get::<_, i64>(9)? != 0,
        extra,
    })
}

/// 按 id 读取单条收藏（用于 refetch 等场景）。
fn get_bookmark(conn: &rusqlite::Connection, id: &str) -> Result<Bookmark, String> {
    conn.query_row(
        "SELECT id,title,description,url,image_url,favicon_url,created_at,updated_at,refined,meta_fetched,extra FROM picky_bookmarks WHERE id=?1",
        [id],
        |row| bookmark_from_row(row),
    )
    .map_err(|e| format!("收藏不存在: {}", e))
}

/// 用 Edge 无头浏览器渲染页面，返回 JS 执行后的完整 DOM 文本。
/// 关键：`--dump-dom` 输出的是渲染完成后的 DOM（含 JS 动态插入的内容）；
/// `--virtual-time-budget` 让浏览器快进虚拟时间并等待异步内容（SPA/懒加载）落定。
/// Edge 不存在或超时返回 None，调用方再决定是否回退普通 HTTP。
async fn render_dom_via_browser(url: &str) -> Option<String> {
    let browser_path = find_browser()?;
    // 独立临时 user-data-dir：避免与用户日常浏览器实例/自动化任务争抢同一 profile 锁
    //（锁被占用时无头进程静默失败，返回空 DOM → 之前被误判为「该站不支持渲染」）。
    let tmp_profile = std::env::temp_dir().join(format!("kira-picky-profile-{}", std::process::id()));
    let args = [
        "--headless",
        "--disable-gpu",
        "--no-sandbox",
        "--disable-extensions",
        // 隐藏自动化特征：无头浏览器默认 UA 带 HeadlessChrome，部分站点据此返回空壳/拦截
        "--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0",
        "--window-size=1280,900",
        "--virtual-time-budget=12000",
        "--dump-dom",
        url,
    ];
    let run = |path: &std::path::PathBuf| {
        tokio::time::timeout(
            std::time::Duration::from_secs(35),
            tokio::process::Command::new(path)
                .args(&args)
                .arg(format!("--user-data-dir={}", tmp_profile.display()))
                .output(),
        )
    };
    let mut output = run(&browser_path).await.ok()?.ok()?;
    // 首次启动可能因冷启动/首建用户目录超时或无输出：有预算再试一次
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        output = run(&browser_path).await.ok()?.ok()?;
    }
    if !output.status.success() {
        return None;
    }
    let body = String::from_utf8_lossy(&output.stdout).to_string();
    // 清理临时 profile（失败不影响结果，下次进程 id 不同会新建）
    let _ = std::fs::remove_dir_all(&tmp_profile);
    if body.trim().is_empty() {
        None
    } else {
        Some(body)
    }
}

/// 定位本机可用的浏览器可执行文件（Windows 自带 Edge；无 Edge 时尝试 Chrome）。
/// 探测顺序：Edge x86 → Edge x64 → Edge 用户级安装 → 注册表 App Paths → Chrome 同序。
fn find_browser() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        let candidates = |name: &str| -> Vec<std::path::PathBuf> {
            let app_rel = format!("Microsoft\\Edge\\Application\\{}.exe", name);
            let chrome_rel = format!("Google\\Chrome\\Application\\{}.exe", name);
            let mut v = Vec::new();
            for var in ["ProgramFiles(x86)", "ProgramFiles", "LocalAppData"] {
                if let Some(root) = std::env::var_os(var) {
                    v.push(std::path::PathBuf::from(&root).join(&app_rel));
                    v.push(std::path::PathBuf::from(root).join(&chrome_rel));
                }
            }
            v
        };
        for p in candidates("msedge") {
            if p.exists() { return Some(p); }
        }
        // 注册表 App Paths 兜底（覆盖自定义安装路径；msedge 与 chrome 都查）
        for exe in ["msedge.exe", "chrome.exe"] {
            if let Some(p) = app_paths_lookup(exe) {
                return Some(p);
            }
        }
        for p in candidates("chrome") {
            if p.exists() { return Some(p); }
        }
        None
    }
    #[cfg(not(windows))]
    {
        for name in ["msedge", "google-chrome", "chromium", "chrome"] {
            if let Ok(out) = std::process::Command::new("which").arg(name).output() {
                if out.status.success() {
                    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !p.is_empty() { return Some(std::path::PathBuf::from(p)); }
                }
            }
        }
        None
    }
}

/// 读取注册表 App Paths 中浏览器的安装路径（HKLM 与 HKCU 都查）。
#[cfg(windows)]
fn app_paths_lookup(exe: &str) -> Option<std::path::PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let key = format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}", exe);
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        if let Ok(k) = RegKey::predef(hive).open_subkey(&key) {
            if let Ok(p) = k.get_value::<String, _>("") {
                let p = std::path::PathBuf::from(p.trim().trim_matches('"'));
                if p.exists() { return Some(p); }
            }
        }
    }
    None
}


/// 从 HTML 里取 `<meta name/property="X" content="...">` 的 content 值。
/// 逐个 `<meta ...>` 标签手写解析属性，不依赖正则回引用（Rust regex 不支持），
/// 因此属性顺序、单双引号、大小写都兼容。
fn meta_content(html: &str, pred: fn(&str) -> bool) -> Option<String> {
    const META: &str = "<meta";
    let lower = html.to_lowercase();
    let mut from = 0usize;
    while let Some(rel) = lower[from..].find(META) {
        let tag_start = from + rel;
        let seg = &html[tag_start..];
        // 找该标签的结束 `>`（不跨上述小写偏移：用原 html 找）
        let tag_end = seg.find('>')?;
        let tag = seg[..tag_end].to_string();
        let tag_lower = tag.to_lowercase();
        // 只处理真正的 meta 标签（magnet etc. 不作为条件，但留有名字可判定）
        let mut content = None;
        let mut name_or_prop: Option<String> = None;
        parse_attrs(&tag, |k, v| {
            match k.as_str() {
                "name" | "property" => name_or_prop = Some(v),
                "content" => content = Some(v),
                _ => {}
            }
        });
        let _ = tag_lower;
        if let Some(k) = name_or_prop {
            if pred(&k) {
                return content;
            }
        }
        from = tag_start + tag_end + 1;
    }
    None
}

/// 手写解析一个标签字符串里的 `k="v"` / `k='v'` / `k=v` 属性，逐个回调。
fn parse_attrs(tag: &str, mut on_attr: impl FnMut(String, String)) {
    let bytes = tag.as_bytes();
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        // 跳过空白
        while i < n && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        // 读 key：字母数字 / '-' / '_' / ':' / '.'
        let key_start = i;
        while i < n {
            let c = bytes[i] as char;
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.') {
                i += 1;
            } else {
                break;
            }
        }
        let key = tag[key_start..i].to_lowercase();
        // 跳过空白到 '='
        while i < n && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= n || bytes[i] != b'=' {
            // 无值属性：继续当作空值（例如仅 rel="icon" 之外的裸属性）
            on_attr(key, String::new());
            continue;
        }
        i += 1; // '='
        while i < n && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let delim = bytes[i];
        if delim == b'"' || delim == b'\'' {
            i += 1;
            let val_start = i;
            while i < n && bytes[i] != delim {
                i += 1;
            }
            let value = tag[val_start..i.min(n)].to_string();
            on_attr(key, value);
            if i < n {
                i += 1; // 结束引号
            }
        } else {
            let val_start = i;
            while i < n && !(bytes[i] as char).is_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            on_attr(key, tag[val_start..i].to_string());
        }
    }
}

/// 解析渲染后 DOM 的元数据：标题 / 描述 / 图片 / favicon。
/// 全部来自浏览器渲染结果（可抓 JS SPA），返回相对路径均已按 base 展开。
fn parse_rendered_meta(dom: &str, url: &str) -> UrlMetadataRich {
    let base = reqwest::Url::parse(url).ok();
    let resolve = |s: &str| -> Option<String> {
        let s = s.trim();
        if s.is_empty() { return None; }
        if let Some(base) = &base {
            if let Ok(u) = base.join(s) { return Some(u.to_string()); }
        }
        Some(s.to_string())
    };

    // title：og:title 优先，否则 <title>
    let title = meta_content(dom, |n| n.eq_ignore_ascii_case("og:title"))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            let re = regex::Regex::new(r"(?is)<title\b[^>]*>(.*?)</title>")
                .ok()?;
            re.captures(dom)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
                .filter(|s| !s.is_empty())
        });

    let description = meta_content(dom, |n| n.eq_ignore_ascii_case("og:description"))
        .filter(|s| !s.trim().is_empty())
        .or_else(|| meta_content(dom, |n| n.eq_ignore_ascii_case("description")))
        .map(|s| s.trim().to_string());

    let image_url = meta_content(dom, |n| n.eq_ignore_ascii_case("og:image"))
        .and_then(|s| resolve(&s));

    // favicon：link[rel~=icon] 的 href，否则默认 /favicon.ico
    let favicon_url = {
        let re = regex::Regex::new(r"(?is)<link\b[^>]*>");
        let mut found = None;
        if let Ok(re) = re {
            for caps in re.captures_iter(dom) {
                let tag_lc = caps.get(0).map(|m| m.as_str()).unwrap_or("").to_lowercase();
                if tag_lc.contains("rel=\"icon\"") || tag_lc.contains("rel='icon'") || tag_lc.contains("rel=\"shortcut icon\"") {
                    if let Ok(hre) = regex::Regex::new(r#"href=["']([^"']+)["']"#) {
                        if let Some(m) = hre.captures(&tag_lc) {
                            found = m.get(1).map(|mm| mm.as_str().to_string());
                            break;
                        }
                    }
                }
            }
        }
        found.and_then(|s| resolve(&s))
            .or_else(|| base.as_ref().and_then(|b| b.join("/favicon.ico").ok().map(|u| u.to_string())))
    };

    UrlMetadataRich { title, description, image_url, favicon_url }
}

#[derive(Default, Clone, Debug)]
struct UrlMetadataRich {
    title: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
    favicon_url: Option<String>,
}

fn insert_bookmark(conn: &rusqlite::Connection, bm: &Bookmark) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO picky_bookmarks
         (id, title, description, url, image_url, favicon_url, created_at, updated_at, refined, meta_fetched, extra)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            bm.id,
            bm.title,
            bm.description,
            bm.url,
            bm.image_url,
            bm.favicon_url,
            bm.created_at,
            bm.updated_at,
            bm.refined as i64,
            bm.meta_fetched as i64,
            serde_json::to_string(&bm.extra).unwrap_or_else(|_| "{}".to_string()),
        ],
    )
    .map_err(|e| format!("写入收藏失败: {}", e))?;
    Ok(())
}

fn bookmark_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_bookmarks WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

fn comment_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_comments WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

fn tag_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_tags WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

fn add_binding_if_absent(conn: &rusqlite::Connection, bookmark_id: &str, tag_id: &str) {
    let _ = conn.execute(
        "INSERT OR IGNORE INTO picky_bookmark_tags (bookmark_id, tag_id) VALUES (?1,?2)",
        rusqlite::params![bookmark_id, tag_id],
    );
}

// ─── 敏感字段加密（复用 commands::secrets，与 cert 模块同一主密钥） ───
use crate::commands::secrets::{decrypt_secret, encrypt_secret};

// ─── 同步配置读写 ───

fn config_from_row(row: &rusqlite::Row) -> rusqlite::Result<PickySyncConfig> {
    let secret_enc: String = row.get(4)?;
    let secret = decrypt_secret(&secret_enc).unwrap_or_default();
    Ok(PickySyncConfig {
        endpoint: row.get(1)?,
        region: row.get(2)?,
        access_key_id: row.get(3)?,
        secret_access_key: secret,
        bucket_name: row.get(5)?,
        prefix: row.get(6)?,
        enabled: row.get::<_, i64>(7)? != 0,
        last_sync_at: row.get(8)?,
        addressing_style: row.get(9)?,
        tls_verify: row.get::<_, i64>(10)? != 0,
        timeout_seconds: row.get(11)?,
        concurrent_reqs: row.get(12)?,
    })
}

fn load_sync_config(conn: &rusqlite::Connection) -> PickySyncConfig {
    conn.query_row("SELECT * FROM picky_sync_config WHERE id=1", [], |row| config_from_row(row))
        .unwrap_or_default()
}

fn save_sync_config_db(conn: &rusqlite::Connection, cfg: &PickySyncConfig) -> Result<(), String> {
    let secret_enc = encrypt_secret(&cfg.secret_access_key)?;
    conn.execute(
        "INSERT OR REPLACE INTO picky_sync_config
         (id, endpoint, region, access_key_id, secret_enc, bucket_name, prefix, enabled, last_sync_at, addressing_style, tls_verify, timeout_seconds, concurrent_reqs)
         VALUES (1,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            cfg.endpoint,
            cfg.region,
            cfg.access_key_id,
            secret_enc,
            cfg.bucket_name,
            cfg.prefix,
            cfg.enabled as i64,
            cfg.last_sync_at,
            cfg.addressing_style,
            cfg.tls_verify as i64,
            cfg.timeout_seconds as i64,
            cfg.concurrent_reqs as i64,
        ],
    )
    .map_err(|e| format!("保存云同步配置失败: {}", e))?;
    Ok(())
}

// ─── 状态序列化（供同步） ───

fn state_to_json(conn: &rusqlite::Connection) -> Result<PickyState, String> {
    let mut bookmarks = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id,title,description,url,image_url,favicon_url,created_at,updated_at,refined,meta_fetched,extra FROM picky_bookmarks")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| bookmark_from_row(row))
            .map_err(|e| e.to_string())?;
        for r in rows {
            bookmarks.push(r.map_err(|e| e.to_string())?);
        }
    }
    let mut comments = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id,bookmark_id,content,created_at,updated_at,parent_id FROM picky_comments")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Comment {
                    id: row.get(0)?,
                    bookmark_id: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    parent_id: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            comments.push(r.map_err(|e| e.to_string())?);
        }
    }
    let mut tags = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id,name,color,created_at FROM picky_tags")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Tag {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            tags.push(r.map_err(|e| e.to_string())?);
        }
    }
    let mut bookmark_tags: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT bookmark_id, tag_id FROM picky_bookmark_tags")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for r in rows {
            let (bid, tid) = r.map_err(|e| e.to_string())?;
            bookmark_tags.entry(bid).or_default().push(tid);
        }
    }
    Ok(PickyState { bookmarks, comments, tags, bookmark_tags })
}

// ─── 云同步合并（合并语义：LWW 按 updatedAt 后写覆盖，绝不丢数据） ───
// 收藏/评论：本地缺失 → 插入；已存在且云端 updatedAt 更新 → 覆盖本地（后写优先）。
// 标签：按 id 只补缺失（避免覆盖本地改名）。标签关联：幂等补充。

#[derive(Debug, Clone, Default)]
struct MergeCounts {
    added_bookmarks: usize,
    updated_bookmarks: usize,
    added_comments: usize,
    updated_comments: usize,
    added_tags: usize,
}

/// 宽松解析 ISO 时间（rfc3339 / 无时区微秒两种格式，兼容 Flutter toIso8601String）。
fn parse_iso_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc())
}

/// 判断 incoming 是否比本地更新（按 updatedAt；解析失败时退化为字符串比较）。
fn incoming_is_newer(incoming: &str, local: &str) -> bool {
    match (parse_iso_utc(incoming), parse_iso_utc(local)) {
        (Some(a), Some(b)) => a > b,
        _ => incoming > local,
    }
}

fn insert_comment(conn: &rusqlite::Connection, c: &Comment) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO picky_comments (id, bookmark_id, content, created_at, updated_at, parent_id)
         VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![c.id, c.bookmark_id, c.content, c.created_at, c.updated_at, c.parent_id],
    )
    .map_err(|e| format!("写入评论失败: {}", e))?;
    Ok(())
}

fn merge_cloud_state(conn: &rusqlite::Connection, state: &JsonValue) -> Result<MergeCounts, String> {
    let mut counts = MergeCounts::default();
    let mut parse_failures = 0usize;

    if let Some(arr) = state.get("bookmarks").and_then(|v| v.as_array()) {
        for item in arr {
            match serde_json::from_value::<Bookmark>(item.clone()) {
                Ok(bm) => {
                    let local_updated: Option<String> = conn
                        .query_row("SELECT updated_at FROM picky_bookmarks WHERE id=?1", [&bm.id], |r| r.get(0))
                        .ok();
                    match local_updated {
                        None => {
                            insert_bookmark(conn, &bm)?;
                            counts.added_bookmarks += 1;
                        }
                        Some(local) if incoming_is_newer(&bm.updated_at, &local) => {
                            insert_bookmark(conn, &bm)?; // INSERT OR REPLACE 全字段覆盖
                            counts.updated_bookmarks += 1;
                        }
                        _ => {} // 本地更新，保留
                    }
                }
                Err(e) => {
                    // 解析失败必须上报并中止同步，绝不允许"云端有数据但本地解析成 0 条"
                    // 再上传空数据把云端覆盖清空（曾经因此丢过数据）。
                    parse_failures += 1;
                    crate::exit_log::exit_log(&format!(
                        "[picky-sync] 解析云端书签失败: {}",
                        e
                    ));
                }
            }
        }
    }
    if parse_failures > 0 {
        return Err(format!(
            "云端快照中有 {} 条书签解析失败，已中止同步以防覆盖云端数据。请检查云端 state.json 格式。",
            parse_failures
        ));
    }
    if let Some(arr) = state.get("comments").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(c) = serde_json::from_value::<Comment>(item.clone()) {
                let local_updated: Option<String> = conn
                    .query_row("SELECT updated_at FROM picky_comments WHERE id=?1", [&c.id], |r| r.get(0))
                    .ok();
                match local_updated {
                    None => {
                        insert_comment(conn, &c)?;
                        counts.added_comments += 1;
                    }
                    Some(local) if incoming_is_newer(&c.updated_at, &local) => {
                        insert_comment(conn, &c)?;
                        counts.updated_comments += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(arr) = state.get("tags").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(t) = serde_json::from_value::<Tag>(item.clone()) {
                if tag_exists(conn, &t.id) {
                    continue;
                }
                conn.execute(
                    "INSERT OR REPLACE INTO picky_tags (id, name, color, created_at) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![t.id, t.name, t.color, t.created_at],
                )
                .map_err(|e| format!("写入标签失败: {}", e))?;
                counts.added_tags += 1;
            }
        }
    }
    if let Some(map) = state.get("bookmarkTags").and_then(|v| v.as_object()) {
        for (bid, val) in map {
            if let Some(ids) = val.as_array() {
                for tid in ids {
                    if let Some(t) = tid.as_str() {
                        add_binding_if_absent(conn, bid, t);
                    }
                }
            }
        }
    }
    Ok(counts)
}

fn format_merge_summary(counts: &MergeCounts, prefix: &str) -> String {
    let mut parts = Vec::new();
    if counts.added_bookmarks > 0 {
        parts.push(format!("新增收藏 {}", counts.added_bookmarks));
    }
    if counts.updated_bookmarks > 0 {
        parts.push(format!("更新收藏 {}", counts.updated_bookmarks));
    }
    if counts.added_comments > 0 {
        parts.push(format!("新增评论 {}", counts.added_comments));
    }
    if counts.updated_comments > 0 {
        parts.push(format!("更新评论 {}", counts.updated_comments));
    }
    if counts.added_tags > 0 {
        parts.push(format!("新增标签 {}", counts.added_tags));
    }
    if parts.is_empty() {
        format!("{}（无变化）", prefix)
    } else {
        format!("{}：{}", prefix, parts.join("，"))
    }
}

// ─── 命令：收藏 ───

/// 一次性加载全量状态（收藏/评论/标签/关联）。
#[tauri::command]
pub fn picky_get_state() -> Result<PickyState, String> {
    let conn = open_db()?;
    state_to_json(&conn)
}

/// 新增收藏。title 为空时用 url 兜底；可传入从网页抓取的元数据（imageUrl/faviconUrl）。
/// 添加后若有 URL，立即在后台走浏览器渲染链路自动补全元数据（JS 页面也能抓到），
/// 用户无需再手动点「刷新元数据」。
#[tauri::command]
pub fn picky_add_bookmark(
    app: tauri::AppHandle,
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
    favicon_url: Option<String>,
) -> Result<Bookmark, String> {
    let conn = open_db()?;
    let mut bm = Bookmark {
        title: title.unwrap_or_default().trim().to_string(),
        url: url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        description: description.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
        image_url: image_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        favicon_url: favicon_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        ..Default::default()
    };
    if bm.title.is_empty() {
        bm.title = bm.url.clone().unwrap_or_else(|| "未命名收藏".to_string());
    }
    let bookmark_id = bm.id.clone();
    insert_bookmark(&conn, &bm)?;
    drop(conn);
    schedule_auto_sync();
    // 后台自动补全元数据：只对「当前标题仍是裸 URL/空描述」的收藏生效（refetch 内部会覆盖）。
    // 用独立任务跑，添加操作本身立即返回，不阻塞 UI。
    if bm.url.is_some() {
        tauri::async_runtime::spawn(async move {
            match picky_refetch_metadata_inner(&bookmark_id).await {
                Ok(_) => { let _ = app.emit("picky-bookmark-updated", &bookmark_id); }
                Err(_) => { /* 自动补全失败静默：用户仍可手动刷新 */ }
            }
        });
    }
    Ok(bm)
}

#[tauri::command]
pub fn picky_update_bookmark(bookmark: Bookmark) -> Result<(), String> {
    let conn = open_db()?;
    if !bookmark_exists(&conn, &bookmark.id) {
        return Err("收藏不存在".to_string());
    }
    let mut bm = bookmark;
    bm.updated_at = now_iso();
    insert_bookmark(&conn, &bm)?;
    schedule_auto_sync();
    Ok(())
}

/// 切换收藏状态（refined：false=收藏中，true=已归档）。
#[tauri::command]
pub fn picky_set_refined(id: String, refined: bool) -> Result<(), String> {
    let conn = open_db()?;
    conn.execute(
        "UPDATE picky_bookmarks SET refined=?1, updated_at=?2 WHERE id=?3",
        rusqlite::params![refined as i64, now_iso(), id],
    )
    .map_err(|e| format!("更新收藏状态失败: {}", e))?;
    schedule_auto_sync();
    Ok(())
}

/// 重新抓取单个收藏页面的完整元数据（标题、描述、图片、Favicon）。
///
/// 真实元数据抓取**必须走浏览器模拟**：很多页面（尤其是 JS 渲染的 SPA / 懒加载页）
/// 用普通 HTTP GET 拿到的 HTML 只有空壳，`<title>`/OG 标签是由脚本动态写入的。
/// 因此这里先调 Edge 无头渲染（`--dump-dom` + 虚拟时间快进），从渲染后的完整 DOM
/// 提取标题/描述/图片/favicon；仅当本机没有 Edge 时，才退回普通 HTTP 抓取兜底。
#[tauri::command]
pub async fn picky_refetch_metadata(id: String) -> Result<Bookmark, String> {
    picky_refetch_metadata_inner(&id).await
}

/// 实际的元数据重抓实现（供命令与新增后的自动补全共用）。
async fn picky_refetch_metadata_inner(id: &str) -> Result<Bookmark, String> {
    let conn = open_db()?;
    let bm = get_bookmark(&conn, id)?;
    let url = bm.url.clone().ok_or_else(|| "该收藏无 URL".to_string())?;

    // 1) 浏览器模拟抓取（首选）：渲染后 DOM 含 JS 动态内容
    let rendered = render_dom_via_browser(&url).await;
    let rich = rendered.as_deref().map(|dom| parse_rendered_meta(dom, &url));

    // 2) 若没有浏览器可用，退回普通 HTTP 抓取的标题/favicon（仅做窄兜底）
    let http_fallback = if rich.is_none() {
        crate::commands::launcher::windows::fetch_url_metadata_with_timeout(
            &url,
            std::time::Duration::from_secs(10),
        )
        .await
        .ok()
    } else {
        None
    };

    let mut updated = bm.clone();
    match &rich {
        Some(r) => {
            if let Some(t) = &r.title {
                updated.title = t.clone();
            }
            if let Some(d) = &r.description {
                updated.description = Some(d.clone());
            }
            if let Some(img) = &r.image_url {
                updated.image_url = Some(img.clone());
            }
            if let Some(f) = &r.favicon_url {
                updated.favicon_url = Some(f.clone());
            }
        }
        None => {
            if let Some(m) = &http_fallback {
                if m.title != url && !m.title.is_empty() {
                    updated.title = m.title.clone();
                }
                if let Some(icon) = m.icon.clone() {
                    updated.favicon_url = Some(icon);
                }
            }
        }
    }
    updated.meta_fetched = true;
    updated.updated_at = now_iso();
    insert_bookmark(&conn, &updated)?;
    drop(conn);
    schedule_auto_sync();
    Ok(updated)
}

#[tauri::command]
pub fn picky_delete_bookmark(id: String) -> Result<(), String> {
    let conn = open_db()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM picky_bookmarks WHERE id=?1", [&id])
        .map_err(|e| format!("删除收藏失败: {}", e))?;
    tx.execute("DELETE FROM picky_comments WHERE bookmark_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM picky_bookmark_tags WHERE bookmark_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    schedule_auto_sync();
    Ok(())
}

// ─── 命令：评论 ───

#[tauri::command]
pub fn picky_add_comment(
    bookmark_id: String,
    content: String,
    parent_id: Option<String>,
) -> Result<Comment, String> {
    let conn = open_db()?;
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("评论内容不能为空".to_string());
    }
    if !bookmark_exists(&conn, &bookmark_id) {
        return Err("收藏不存在".to_string());
    }
    let now = now_iso();
    let c = Comment {
        id: uuid_v4(),
        bookmark_id,
        content,
        created_at: now.clone(),
        updated_at: now,
        parent_id: parent_id.filter(|p| !p.is_empty()),
    };
    conn.execute(
        "INSERT INTO picky_comments (id, bookmark_id, content, created_at, updated_at, parent_id)
         VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![c.id, c.bookmark_id, c.content, c.created_at, c.updated_at, c.parent_id],
    )
    .map_err(|e| format!("写入评论失败: {}", e))?;
    schedule_auto_sync();
    Ok(c)
}

#[tauri::command]
pub fn picky_update_comment(comment: Comment) -> Result<(), String> {
    let conn = open_db()?;
    if !comment_exists(&conn, &comment.id) {
        return Err("评论不存在".to_string());
    }
    conn.execute(
        "UPDATE picky_comments SET content=?1, updated_at=?2 WHERE id=?3",
        rusqlite::params![comment.content, now_iso(), comment.id],
    )
    .map_err(|e| format!("更新评论失败: {}", e))?;
    schedule_auto_sync();
    Ok(())
}

/// 删除评论（连同其子回复）。
#[tauri::command]
pub fn picky_delete_comment(id: String) -> Result<(), String> {
    let conn = open_db()?;
    conn.execute("DELETE FROM picky_comments WHERE id=?1 OR parent_id=?1", [&id])
        .map_err(|e| format!("删除评论失败: {}", e))?;
    schedule_auto_sync();
    Ok(())
}

// ─── 命令：标签 ───

#[tauri::command]
pub fn picky_add_tag(name: String, color: Option<String>) -> Result<Tag, String> {
    let conn = open_db()?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("标签名不能为空".to_string());
    }
    // 同名标签去重
    let existing: Option<String> = conn
        .query_row("SELECT id FROM picky_tags WHERE name=?1", [&name], |r| r.get(0))
        .ok();
    if let Some(id) = existing {
        return Ok(Tag {
            id,
            name,
            color: color.unwrap_or_else(default_tag_color),
            created_at: now_iso(),
        });
    }
    let t = Tag {
        id: uuid_v4(),
        name,
        color: color.unwrap_or_else(default_tag_color),
        created_at: now_iso(),
    };
    conn.execute(
        "INSERT INTO picky_tags (id, name, color, created_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![t.id, t.name, t.color, t.created_at],
    )
    .map_err(|e| format!("写入标签失败: {}", e))?;
    schedule_auto_sync();
    Ok(t)
}

#[tauri::command]
pub fn picky_delete_tag(id: String) -> Result<(), String> {
    let conn = open_db()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM picky_tags WHERE id=?1", [&id])
        .map_err(|e| format!("删除标签失败: {}", e))?;
    tx.execute("DELETE FROM picky_bookmark_tags WHERE tag_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    schedule_auto_sync();
    Ok(())
}

/// 切换书签-标签关联，返回切换后的状态（true=已关联）。
#[tauri::command]
pub fn picky_toggle_bookmark_tag(bookmark_id: String, tag_id: String) -> Result<bool, String> {
    let conn = open_db()?;
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM picky_bookmark_tags WHERE bookmark_id=?1 AND tag_id=?2",
            rusqlite::params![bookmark_id, tag_id],
            |_| Ok(()),
        )
        .is_ok();
    if exists {
        conn.execute(
            "DELETE FROM picky_bookmark_tags WHERE bookmark_id=?1 AND tag_id=?2",
            rusqlite::params![bookmark_id, tag_id],
        )
        .map_err(|e| format!("移除标签失败: {}", e))?;
        schedule_auto_sync();
        Ok(false)
    } else {
        add_binding_if_absent(&conn, &bookmark_id, &tag_id);
        schedule_auto_sync();
        Ok(true)
    }
}

// ─── 命令：云同步配置 ───

#[tauri::command]
pub fn picky_get_sync_config() -> Result<PickySyncConfig, String> {
    let conn = open_db()?;
    Ok(load_sync_config(&conn))
}

#[tauri::command]
pub fn picky_save_sync_config(config: PickySyncConfig) -> Result<(), String> {
    let conn = open_db()?;
    save_sync_config_db(&conn, &config)
}

// ─── 自动同步调度（防抖） ───

/// 防抖窗口：内容变更后合并等待这段时间再同步，避免连续操作触发多次上传。
const AUTO_SYNC_DEBOUNCE: Duration = Duration::from_secs(3);

static LAST_CHANGE: Mutex<Option<Instant>> = Mutex::new(None);
static SYNC_RUNNING: AtomicBool = AtomicBool::new(false);
/// 同步进行中又到达变更时置位，确保同步结束后补一轮（修复漏同步窗口）。
static PENDING_RESYNC: AtomicBool = AtomicBool::new(false);

/// 内容变更后调用：记录变更时间并安排一次防抖同步。
/// 连续变更只触发一次同步（距最后一次变更 3 秒后才执行）。
pub fn schedule_auto_sync() {
    *LAST_CHANGE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(AUTO_SYNC_DEBOUNCE).await;
        // 防抖：睡眠期间有新变更则放弃本次，由最新变更的任务接手
        let should = LAST_CHANGE
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() >= AUTO_SYNC_DEBOUNCE);
        if !should {
            return;
        }
        if SYNC_RUNNING.swap(true, Ordering::SeqCst) {
            // 已有同步在跑：置位待补同步，由运行中的同步结束后接手，
            // 避免本任务提前返回后该变更从此被漏掉。
            PENDING_RESYNC.store(true, Ordering::SeqCst);
            return;
        }
        let sync_started = Instant::now();
        if let Err(e) = picky_sync_now().await {
            crate::exit_log::exit_log(&format!("[picky-sync] 自动同步失败（稍后手动同步）: {}", e));
        }
        SYNC_RUNNING.store(false, Ordering::SeqCst);
        // 同步期间又产生了新变更（防抖任务置位或变更时间晚于同步开始）→ 再调度一轮
        let has_new = PENDING_RESYNC.swap(false, Ordering::SeqCst)
            || LAST_CHANGE
                .lock()
                .unwrap()
                .is_some_and(|t| t > sync_started);
        if has_new {
            schedule_auto_sync();
        }
    });
}

/// 启动/退出时调用：仅当已配置并启用云同步时执行一次同步（静默忽略未配置）。
/// 用于应用启动拉取云端、退出前推送本地。
pub async fn picky_sync_if_enabled() {
    let conn = open_db();
    let cfg = match conn {
        Ok(c) => load_sync_config(&c),
        Err(_) => return,
    };
    if !cfg.is_configured() {
        return;
    }
    let _ = picky_sync_now().await;
}

// ─── 命令：云同步 ───

/// 双向同步（一个按钮内部自动合并）：
/// 1) 先下载云端状态并合并到本地（LWW：本地缺失补入，云端 updatedAt 更新则覆盖）；
/// 2) 再上传合并后的全量状态。
/// 这样无论哪端先/后同步，云端始终是两端数据的并集，绝不互相覆盖丢数据。
/// 返回结果文本。
#[tauri::command]
pub async fn picky_sync_now() -> Result<String, String> {
    crate::exit_log::exit_log("[picky-sync] 开始云同步");
    let conn = open_db()?;
    let cfg = load_sync_config(&conn);
    if !cfg.is_configured() {
        crate::exit_log::exit_log("[picky-sync] 未配置云同步，终止");
        return Err("云同步未配置（需启用并填写 endpoint / AccessKey / SecretKey / Bucket）".to_string());
    }
    crate::exit_log!(
        "[picky-sync] endpoint={:?} bucket={:?} prefix={:?} enabled={}",
        cfg.endpoint, cfg.bucket_name, cfg.prefix, cfg.enabled
    );

    // 1) 先合并云端到本地（不存在则下载并合并，404 视为首次同步）
    let client = s3::PickyS3Client::new(cfg.clone());
    let merged = match client.download_full_state().await? {
        Some(cloud) => {
            let counts = merge_cloud_state(&conn, &cloud)?;
            let s = format_merge_summary(&counts, "已合并云端");
            crate::exit_log!("[picky-sync] 下载并合并云端成功: {}", s);
            s
        }
        None => {
            crate::exit_log::exit_log("[picky-sync] 云端暂无数据（首次同步）");
            "云端暂无数据（首次同步）".to_string()
        }
    };

    // 2) 上传合并后的全量状态
    let state = state_to_json(&conn)?;
    let bookmarks: Vec<JsonValue> = state.bookmarks.iter().map(|b| serde_json::to_value(b).unwrap_or_default()).collect();
    let comments: Vec<JsonValue> = state.comments.iter().map(|c| serde_json::to_value(c).unwrap_or_default()).collect();
    let tags: Vec<JsonValue> = state.tags.iter().map(|t| serde_json::to_value(t).unwrap_or_default()).collect();
    let bm_tags: JsonMap<String, JsonValue> = state
        .bookmark_tags
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::json!(v)))
        .collect();

    crate::exit_log!(
        "[picky-sync] 准备上传: bookmarks={} comments={} tags={}",
        bookmarks.len(),
        comments.len(),
        tags.len()
    );
    let synced_at = client.upload_full_state(&bookmarks, &comments, &tags, &bm_tags).await?;
    crate::exit_log!("[picky-sync] 上传成功，syncedAt={:?}", synced_at);

    // 记录上次同步时间
    let mut updated = cfg.clone();
    updated.last_sync_at = Some(synced_at.clone());
    let conn2 = open_db()?;
    save_sync_config_db(&conn2, &updated)?;

    crate::exit_log::exit_log("[picky-sync] 云同步完成");
    Ok(format!(
        "同步完成：bookmarks={} comments={} tags={} · {} · 同步时间 {}",
        bookmarks.len(),
        comments.len(),
        tags.len(),
        merged,
        synced_at
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试：Flutter 端导出的 state.json 用 0/1 整数表示 refined/metaFetched，
    /// 且带 lingzuCode 等扩展字段。此前 Rust 端 bool 字段解析整数失败，
    /// 导致 8 条书签全部被静默丢弃、上传空数据覆盖云端（真实事故）。
    #[test]
    fn bookmark_parses_flutter_int_bools() {
        let json = serde_json::json!([{
            "id": "e5d94a2f-d53b-4a3e-ab75-4b8e7a6df2ca",
            "title": "测试收藏",
            "description": "描述",
            "url": "https://example.com/a",
            "imageUrl": null,
            "faviconUrl": null,
            "createdAt": "2026-08-22T23:21:55.243827",
            "updatedAt": "2026-08-22T23:22:10.358403",
            "refined": 0,
            "metaFetched": 1,
            "lingzuCode": 8978,
            "lingzuOrigin": "mp.weixin.qq.com",
            "aiCopy": null,
            "aiCopyAt": ""
        }]);
        let arr = json.as_array().unwrap();
        let bm: Bookmark = serde_json::from_value(arr[0].clone()).expect("应能解析 Flutter 整数布尔");
        assert!(!bm.refined);
        assert!(bm.meta_fetched);
        // 扩展字段原样保留
        assert_eq!(bm.extra.get("lingzuCode").and_then(|v| v.as_i64()), Some(8978));
        assert_eq!(bm.extra.get("lingzuOrigin").and_then(|v| v.as_str()), Some("mp.weixin.qq.com"));
    }

    /// 序列化方向：any-version 上传的数据必须输出 0/1 整数（与 Flutter toJson 一致），
    /// 否则 Flutter 端 `(map['refined'] as int?)` 解析失败。
    #[test]
    fn bookmark_serializes_int_bools() {
        let bm = Bookmark {
            id: "abc".to_string(),
            refined: true,
            meta_fetched: false,
            ..Default::default()
        };
        let v = serde_json::to_value(&bm).unwrap();
        assert_eq!(v.get("refined").and_then(|x| x.as_i64()), Some(1));
        assert_eq!(v.get("metaFetched").and_then(|x| x.as_i64()), Some(0));
    }

    /// 兼容旧数据：布尔 true/false 也应能解析。
    #[test]
    fn bookmark_parses_legacy_bools() {
        let json = serde_json::json!({
            "id": "x",
            "title": "t",
            "url": "https://example.com",
            "createdAt": "2026-01-01T00:00:00",
            "updatedAt": "2026-01-01T00:00:00",
            "refined": true,
            "metaFetched": false,
        });
        let bm: Bookmark = serde_json::from_value(json).expect("布尔也应可解析");
        assert!(bm.refined);
        assert!(!bm.meta_fetched);
    }
}
