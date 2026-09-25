//! 浏览器收藏夹导入（自启动模块搬来，落在收藏模块）。
//!
//! 与启动模块那份的关键区别：**书签目录建成多级分类树**。
//! Edge/Chrome 的书签本来就是树，之前压平成「一层分类」把结构丢了；
//! 现在按目录层级建分类，条目挂到它所在的目录上（同名 URL 不重复入库，
//! 但会在它出现的每个目录下都挂一次）。

use std::path::PathBuf;

use serde::Serialize;

use super::db;

/// 导入结果（前端用来回显「导了多少条 / 建了多少目录」）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkImportResult {
    /// 落库的条目数（含已存在但新挂到别的目录的）
    pub imported: usize,
    /// 新建的分类数
    pub folders: usize,
    /// 解析到但跳过的（缺 url / 非法条目）
    pub skipped: usize,
    /// 书签文件路径（回显给用户确认读的是哪个 Profile）
    pub file: String,
}

/// Chromium 的 `date_added`：自 1601-01-01 起的**微秒**数。
fn chrome_time_to_local(raw: &str) -> Option<String> {
    let micros: i64 = raw.parse().ok()?;
    if micros <= 0 {
        return None;
    }
    let unix_secs = micros / 1_000_000 - 11_644_473_600;
    db::unix_to_local_str(unix_secs)
}

/// 找出浏览器书签文件。多个 Profile 时挑**修改时间最新**的那个：
/// 用户实际在用的 Profile 才是有内容的那个。
pub fn locate_bookmarks_file(browser: &str, custom_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = custom_path {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
        return Err(format!("书签文件不存在: {}", pb.display()));
    }
    let local = std::env::var("LOCALAPPDATA").map_err(|_| "读不到 LOCALAPPDATA".to_string())?;
    let (vendor, product) = match browser.to_lowercase().as_str() {
        "edge" => ("Microsoft", "Edge"),
        "chrome" => ("Google", "Chrome"),
        other => return Err(format!("不支持的浏览器: {other}")),
    };
    let base = PathBuf::from(&local).join(vendor).join(product).join("User Data");
    if !base.exists() {
        return Err(format!("没找到 {product} 的用户数据目录（可能没装）"));
    }
    let mut names: Vec<String> = vec!["Default".to_string()];
    if let Ok(entries) = std::fs::read_dir(&base) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("Profile ") {
                names.push(name);
            }
        }
    }
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for name in names {
        let candidate = base.join(name).join("Bookmarks");
        if let Ok(meta) = std::fs::metadata(&candidate) {
            let ts = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if best.as_ref().map(|(_, t)| ts > *t).unwrap_or(true) {
                best = Some((candidate, ts));
            }
        }
    }
    best.map(|(p, _)| p)
        .ok_or_else(|| format!("没找到 {product} 的 Bookmarks 文件"))
}

/// 顶层根目录的中文名（Chromium 的固定三个根）。
fn root_label(key: &str, fallback: &str) -> String {
    match key {
        "bookmark_bar" => "书签栏".to_string(),
        "other" => "其他书签".to_string(),
        "synced" => "移动设备书签".to_string(),
        _ => fallback.to_string(),
    }
}

/// 导入。目录 → 多级分类；条目 → favorite（source = `bookmark`）。
pub fn import(browser: &str, custom_path: Option<&str>) -> Result<BookmarkImportResult, String> {
    let file = locate_bookmarks_file(browser, custom_path)?;
    let raw = std::fs::read_to_string(&file).map_err(|e| format!("读书签文件失败: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("书签文件不是合法 JSON: {e}"))?;
    let roots = json
        .get("roots")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "书签文件里没有 roots".to_string())?;

    // 先把整棵树走成 (分类路径, 条目) 列表，再统一落库：
    // 解析阶段不碰数据库，出错时不会留下半棵树。
    struct WalkOut {
        nodes: Vec<(Vec<String>, db::NewFavorite)>,
    }
    fn walk(node: &serde_json::Value, path: &mut Vec<String>, out: &mut WalkOut, skipped: &mut usize) {
        let Some(children) = node.get("children").and_then(|v| v.as_array()) else {
            return;
        };
        for child in children {
            match child.get("type").and_then(|v| v.as_str()) {
                Some("url") => {
                    let url = child.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
                    if url.is_empty() {
                        *skipped += 1;
                        continue;
                    }
                    let title = child
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(url)
                        .to_string();
                    out.nodes.push((
                        path.clone(),
                        db::NewFavorite {
                            // 用 url 作 external_id：同一个网址在多个目录下只存一条，但会挂在每个目录
                            source: "bookmark".to_string(),
                            external_id: url.to_string(),
                            url: url.to_string(),
                            title,
                            subtitle: None,
                            description: None,
                            extra_json: None,
                            favorited_at: child
                                .get("date_added")
                                .and_then(|v| v.as_str())
                                .and_then(chrome_time_to_local),
                            initial_status: None,
                        },
                    ));
                }
                Some("folder") => {
                    let name = child
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("未命名文件夹")
                        .to_string();
                    path.push(name);
                    walk(child, path, out, skipped);
                    path.pop();
                }
                _ => {
                    *skipped += 1;
                }
            }
        }
    }

    let mut out = WalkOut { nodes: Vec::new() };
    let mut skipped = 0usize;
    // 根节点本身也算一层分类（否则「书签栏/技术/GitHub」会变成「技术/GitHub」）
    for (key, node) in roots {
        let mut path = vec![root_label(key, node.get("name").and_then(|v| v.as_str()).unwrap_or(key))];
        walk(node, &mut path, &mut out, &mut skipped);
    }

    let mut folders = 0usize;
    let imported = db::with_conn(|conn| {
        let mut imported = 0usize;
        for (path, item) in &out.nodes {
            // 目录 → 分类（逐级 ensure，同名复用；新建的才算进 folders 计数）
            let mut parent: Option<i64> = None;
            for name in path {
                let existing = db::find_child_id(conn, parent, name)?;
                let id = match existing {
                    Some(id) => id,
                    None => {
                        folders += 1;
                        db::create_category(conn, name, parent)?
                    }
                };
                parent = Some(id);
            }
            let id = db::upsert(conn, item)
                .map(|_| ())
                .and_then(|_| db::find_favorite_id(conn, &item.source, &item.external_id))?;
            if let (Some(fid), Some(cid)) = (id, parent) {
                db::link_item_category(conn, fid, cid)?;
                imported += 1;
            }
        }
        Ok(imported)
    })?;

    eprintln!(
        "[favorites] 浏览器收藏夹导入完成: browser={} file={} 条目={} 目录={} 跳过={}",
        browser,
        file.display(),
        imported,
        folders,
        skipped
    );
    Ok(BookmarkImportResult {
        imported,
        folders,
        skipped,
        file: file.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{chrome_time_to_local, root_label};

    /// Chromium 的时间基准是 1601-01-01 的微秒；算错会得到一个 1970 年附近的时间。
    #[test]
    fn converts_chromium_timestamp() {
        // 13340000000000000 微秒 ≈ 2023-09-23
        let out = chrome_time_to_local("13340000000000000").expect("应能转换");
        assert!(out.starts_with("2023-"), "转换结果异常: {out}");
        assert!(chrome_time_to_local("0").is_none());
        assert!(chrome_time_to_local("not-a-number").is_none());
    }

    #[test]
    fn maps_known_roots() {
        assert_eq!(root_label("bookmark_bar", "x"), "书签栏");
        assert_eq!(root_label("other", "x"), "其他书签");
        assert_eq!(root_label("synced", "x"), "移动设备书签");
        assert_eq!(root_label("whatever", "原名"), "原名");
    }
}
