use super::*;

/// 读各平台凭证的健康状态（用于「Cookie 快过期 / 已失效」提示）。
///
/// 前端在模块打开时调一次：纯本地查询、不联网，所以不会给平台添任何请求。
#[tauri::command]
pub fn fav_credential_status() -> Result<Vec<db::CredentialStatus>, String> {
    db::with_conn(|conn| {
        ["github", "bilibili", zhihu::COOKIE_KEY]
            .iter()
            .map(|source| db::credential_status(conn, source))
            .collect()
    })
}

/// 读条目正文缓存（知乎收藏的内容，或已抓过的 GitHub README）。
///
/// 返回 `None` = 没缓存过，由前端决定要不要去抓（GitHub README 是懒抓的）。
#[tauri::command]
pub fn fav_get_content(id: i64) -> Result<Option<db::CachedContent>, String> {
    db::with_conn(|conn| db::get_content(conn, id))
}

/// 抓某个 GitHub 条目的 README（**优先 `README_ZH.md`**）并缓存。
///
/// 顺序：`README_ZH.md` → GitHub 自己认的默认 README。
/// 先找中文版是因为不少中文项目把中文说明单独放一个文件，默认 README 反而是英文简介。
/// 命中后按文件名写进 `favorite_content.label`，界面上能看出读的是哪一份。
#[tauri::command]
pub async fn fav_github_readme(
    id: i64,
    refresh: Option<bool>,
) -> Result<db::CachedContent, String> {
    crate::runtime_tasks::run_if_enabled("favorites", async move {
        if refresh != Some(true) {
            if let Some(cached) = db::with_conn(|conn| db::get_content(conn, id))? {
                return Ok(cached);
            }
        }

        let (source, full_name, _) = db::with_conn(|conn| db::find_by_id(conn, id))?
            .ok_or_else(|| format!("条目 {} 不存在", id))?;
        if source != github::SOURCE {
            return Err("只有 GitHub 条目支持查看 README".to_string());
        }
        let token =
            db::with_conn(|conn| db::get_credential(conn, github::SOURCE))?.unwrap_or_default();

        for name in github::README_ZH_CANDIDATES {
            if let Some(text) = github::fetch_file_text(&token, &full_name, name).await? {
                return cache_readme(id, &text, Some(name));
            }
        }
        match github::fetch_default_readme(&token, &full_name).await? {
            Some((name, text)) => cache_readme(id, &text, Some(&name)),
            None => Err(format!("{} 没有 README", full_name)),
        }
    })
    .await
}

/// 写入 README 缓存并回读（回读是为了带上统一的 `fetchedAt`）。
pub(super) fn cache_readme(
    id: i64,
    text: &str,
    label: Option<&str>,
) -> Result<db::CachedContent, String> {
    db::with_conn(|conn| db::put_content(conn, id, text, None, label))?;
    db::with_conn(|conn| db::get_content(conn, id))?.ok_or_else(|| "缓存 README 失败".to_string())
}

/// 读取收藏模块自己的 GitHub Token（未配置返回空串；**不读 SDK 模块的配置**）。
#[tauri::command]
pub fn fav_get_github_token() -> Result<String, String> {
    Ok(db::with_conn(|conn| db::get_credential(conn, github::SOURCE))?.unwrap_or_default())
}

/// 保存 / 清除收藏模块自己的 GitHub Token（传空串即清除）。
#[tauri::command]
pub fn fav_set_github_token(token: String) -> Result<(), String> {
    db::with_conn(|conn| db::set_credential(conn, github::SOURCE, &token))
}

/// 设置某平台的 Cookie（B站：含 SESSDATA 的完整 Cookie 串；传空串清除）。
#[tauri::command]
pub fn fav_set_credential(source: String, cookie: String) -> Result<(), String> {
    db::with_conn(|conn| db::set_credential(conn, &source, &cookie))
}

/// 某平台是否已配置 Cookie（**不返回内容**，只回 true/false）。
#[tauri::command]
pub fn fav_has_credential(source: String) -> Result<bool, String> {
    db::with_conn(|conn| db::get_credential(conn, &source).map(|c| c.is_some()))
}

/// 读取某平台已保存的凭证原文（未配置返回空串）。
///
/// 仅供配置弹窗回显（GitHub Token 弹窗一直有这个能力）；只在本机 UI 里展示。
#[tauri::command]
pub fn fav_get_credential(source: String) -> Result<String, String> {
    Ok(db::with_conn(|conn| db::get_credential(conn, &source))?.unwrap_or_default())
}
