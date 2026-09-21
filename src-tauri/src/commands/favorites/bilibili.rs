//! B站收藏导入（**只读**）：收藏夹列表 → 收藏夹内容分页 → 本地条目。
//!
//! 走的是 web 端私有接口，需要 Cookie（SESSDATA）+ WBI 签名（见 [`super::wbi`]）。
//! 逆向依赖，接口随时可能变，因此所有失败都带可操作提示，不静默吞掉。

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::db::NewFavorite;
use super::wbi;

/// 数据源标识（写入 `favorite.source`）
pub const SOURCE: &str = "bilibili";

/// 浏览器 UA：B站对默认 UA 的风控比对其它站更严，这里按浏览器伪装。
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const REFERER: &str = "https://www.bilibili.com/";

/// 每页条数：接口定义域是 1-20，取上限少发请求。
pub const PAGE_SIZE: usize = 20;

/// 一次导入最多翻多少页（防止收藏夹特别大时跑太久）
pub const MAX_PAGES: usize = 200;

/// 登录态与 WBI 口令。
#[derive(Debug, Clone)]
pub struct Session {
    /// 当前登录用户的 mid（未登录时为 None）
    pub mid: Option<i64>,
    pub img_key: String,
    pub sub_key: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 取 WBI 口令与登录态：`GET /x/web-interface/nav`。
///
/// 这个接口**未登录也会返回 wbi_img**（code = -101），所以口令永远拿得到；
/// mid 只有在带有效 Cookie 时才有——拿不到 mid 就要提示用户 Cookie 失效。
pub async fn fetch_session(cookie: &str) -> Result<Session, String> {
    let body = get_json("https://api.bilibili.com/x/web-interface/nav", &[], cookie).await?;
    let wbi_img = body
        .get("data")
        .and_then(|d| d.get("wbi_img"));
    let (img_key, sub_key) = match wbi_img {
        Some(wbi_img) => (
            wbi_img
                .get("img_url")
                .and_then(|v| v.as_str())
                .and_then(wbi::key_from_url),
            wbi_img
                .get("sub_url")
                .and_then(|v| v.as_str())
                .and_then(wbi::key_from_url),
        ),
        None => (None, None),
    };
    let (Some(img_key), Some(sub_key)) = (img_key, sub_key) else {
        return Err("未能从 B站获取 WBI 口令（接口可能已改版）".to_string());
    };
    let mid = body
        .pointer("/data/mid")
        .and_then(|v| v.as_i64())
        .filter(|mid| *mid > 0);
    Ok(Session {
        mid,
        img_key,
        sub_key,
    })
}

/// 取用户创建的所有收藏夹：`GET /x/v3/fav/folder/created/list-all`。
pub async fn fetch_folders(cookie: &str, session: &Session) -> Result<Vec<(i64, String)>, String> {
    let Some(mid) = session.mid else {
        return Err("Cookie 未生效（B站没返回登录用户），请重新粘贴 Cookie".to_string());
    };
    let body = signed_get(
        "https://api.bilibili.com/x/v3/fav/folder/created/list-all",
        &[("up_mid", mid.to_string()), ("type", "2".to_string())],
        cookie,
        session,
    )
    .await?;

    let list = body
        .pointer("/data/list")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut folders = Vec::new();
    for item in list {
        let Some(id) = item.get("id").and_then(|v| v.as_i64()) else {
            continue;
        };
        let title = item
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("未命名收藏夹")
            .to_string();
        folders.push((id, title));
    }
    Ok(folders)
}

/// 取某个收藏夹的一页内容：`GET /x/v3/fav/resource/list`。
/// 返回（条目数组，是否还有下一页）。
pub async fn fetch_folder_page(
    cookie: &str,
    session: &Session,
    folder_id: i64,
    page: usize,
) -> Result<(Vec<Value>, bool), String> {
    let body = signed_get(
        "https://api.bilibili.com/x/v3/fav/resource/list",
        &[
            ("media_id", folder_id.to_string()),
            ("pn", page.to_string()),
            ("ps", PAGE_SIZE.to_string()),
            ("platform", "web".to_string()),
        ],
        cookie,
        session,
    )
    .await?;
    let medias = body
        .pointer("/data/medias")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let has_more = body
        .pointer("/data/has_more")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Ok((medias, has_more))
}

/// 一条收藏内容 → 待落库条目。
///
/// `external_id` 用 **bvid**（视频的稳定标识；avid 也能用但 bvid 才是用户能看到的那个）。
/// `attr != 0` 表示内容已失效（UP 主删除等），直接标成 gone，省得再探测一次。
pub fn media_to_favorite(media: &Value, folder_title: &str) -> Option<NewFavorite> {
    let bvid = media
        .get("bvid")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| media.get("bv_id").and_then(|v| v.as_str()))?;
    let title = media
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let upper = media
        .pointer("/upper/name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let intro = media
        .get("intro")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let attr = media.get("attr").and_then(|v| v.as_i64()).unwrap_or(0);
    Some(NewFavorite {
        source: SOURCE.to_string(),
        external_id: bvid.to_string(),
        url: format!("https://www.bilibili.com/video/{}", bvid),
        title,
        subtitle: Some(if upper.is_empty() {
            folder_title.to_string()
        } else {
            upper.clone()
        }),
        description: intro,
        extra_json: Some(
            json!({
                "folder": folder_title,
                "upper": upper,
                "duration": media.get("duration").and_then(|v| v.as_i64()),
                "fav_time": media.get("fav_time").and_then(|v| v.as_i64()),
                "type": media.get("type").and_then(|v| v.as_i64()),
            })
            .to_string(),
        ),
        initial_status: if attr == 0 { None } else { Some("gone".to_string()) },
    })
}

/// 带 WBI 签名的 GET。
async fn signed_get(
    url: &str,
    params: &[(&str, String)],
    cookie: &str,
    session: &Session,
) -> Result<Value, String> {
    let query = wbi::sign_query(params, &session.img_key, &session.sub_key, now_secs());
    get_json(&format!("{}?{}", url, query), &[], cookie).await
}

/// 发 GET 并解析 JSON；B站的「业务错误」在 body 的 code 里（HTTP 仍是 200），必须单独看。
async fn get_json(url: &str, _params: &[(&str, String)], cookie: &str) -> Result<Value, String> {
    let resp = crate::commands::utils::get_http_client()
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::REFERER, REFERER)
        .header(reqwest::header::COOKIE, cookie)
        .send()
        .await
        .map_err(|e| format!("请求 B站失败: {}", e))?;
    let status = resp.status().as_u16();
    if status == 412 {
        return Err("B站拒绝了请求（412 风控），请稍后再试或减少导入频率".to_string());
    }
    if !(200..300).contains(&status) {
        return Err(format!("B站返回 HTTP {}", status));
    }
    let body = resp
        .json::<Value>()
        .await
        .map_err(|e| format!("解析 B站响应失败: {}", e))?;
    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
    if code != 0 {
        return Err(map_bili_code(code, body.get("message").and_then(|v| v.as_str())));
    }
    Ok(body)
}

/// B站业务错误码 → 可操作提示（`-101 账号未登录` 这种必须说清要重贴 Cookie）。
fn map_bili_code(code: i64, message: Option<&str>) -> String {
    let hint = match code {
        -101 => "（Cookie 已失效，请重新粘贴）",
        -111 => "（csrf 校验失败）",
        -352 | 412 => "（触发风控，请稍后再试）",
        -403 => "（没有访问权限，私有收藏夹需要本人的 Cookie）",
        _ => "",
    };
    format!(
        "B站返回错误 {} {}{}",
        code,
        message.unwrap_or_default(),
        hint
    )
}

#[cfg(test)]
mod tests {
    use super::{map_bili_code, media_to_favorite, SOURCE};
    use serde_json::json;

    fn media(bvid: &str, title: &str, attr: i64) -> serde_json::Value {
        json!({
            "id": 371494037,
            "type": 2,
            "title": title,
            "intro": "简介",
            "duration": 546,
            "upper": { "mid": 1, "name": "UP主" },
            "attr": attr,
            "fav_time": 1598884777,
            "bvid": bvid
        })
    }

    #[test]
    fn maps_media_using_bvid_as_native_id() {
        let fav = media_to_favorite(&media("BV1CZ4y1T7gC", "标题", 0), "我的收藏").unwrap();
        assert_eq!(fav.source, SOURCE);
        assert_eq!(fav.external_id, "BV1CZ4y1T7gC", "必须用 bvid 做去重键");
        assert_eq!(fav.url, "https://www.bilibili.com/video/BV1CZ4y1T7gC");
        assert_eq!(fav.title, "标题");
        assert_eq!(fav.subtitle.as_deref(), Some("UP主"));
        assert_eq!(fav.initial_status, None);
        assert!(fav.extra_json.as_ref().unwrap().contains("我的收藏"));
    }

    /// 已失效的内容（attr != 0）直接标 gone，不用再探测一次。
    #[test]
    fn marks_deleted_media_as_gone() {
        let fav = media_to_favorite(&media("BV1xx", "已删稿", 9), "收藏").unwrap();
        assert_eq!(fav.initial_status.as_deref(), Some("gone"));
    }

    /// 没有 bvid / 标题的异常条目跳过，别让整批导入失败。
    #[test]
    fn malformed_media_is_skipped() {
        assert!(media_to_favorite(&json!({ "title": "x" }), "收藏").is_none());
        assert!(media_to_favorite(&json!({ "bvid": "BV1" }), "收藏").is_none());
    }

    /// -101 必须说清「Cookie 失效」，否则用户只会看到一串数字。
    #[test]
    fn error_codes_are_actionable() {
        let text = map_bili_code(-101, Some("账号未登录"));
        assert!(text.contains("-101"));
        assert!(text.contains("Cookie"));
        assert!(map_bili_code(-403, None).contains("私有"));
    }
}
