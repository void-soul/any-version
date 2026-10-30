use super::*;

/// 获取下载 URL 和文件扩展名
pub(super) struct DownloadInfo {
    pub(super) url: String,
    pub(super) file_ext: String,
    pub(super) extract_subdir: Option<String>,
}

/// 从 `assets` 数组里挑出唯一匹配的那一个。**纯函数**，不联网。
///
/// 挑不出来就报错，**绝不随便挑一个**：猜错的后果是下载到一个同名但不对的东西
/// （或者 404），而用户完全看不出哪里错了 —— 这正是本模块 URL 模板那条老路的教训。
pub(crate) fn pick_asset<'a>(
    assets: &'a [serde_json::Value],
    prefix: &str,
    suffix: &str,
) -> Result<(&'a str, &'a str), String> {
    let matched: Vec<&serde_json::Value> = assets
        .iter()
        .filter(|a| {
            a.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.starts_with(prefix) && n.ends_with(suffix))
                .unwrap_or(false)
        })
        .collect();
    match matched.len() {
        0 => Err(format!(
            "该 release 里没有匹配「{}*{}」的资产（现有 {} 个）。\
             资产名里的构建日期之类的东西推不出来，只能逐个列出来核对",
            prefix,
            suffix,
            assets.len()
        )),
        1 => {
            let name = matched[0]
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default();
            let url = matched[0]
                .get("browser_download_url")
                .and_then(|n| n.as_str())
                .ok_or_else(|| format!("资产 {} 缺少 browser_download_url", name))?;
            Ok((name, url))
        }
        n => {
            // 多个匹配说明筛选条件不够窄。把候选列出来，比默默取第一个有用得多。
            let names: Vec<&str> = matched
                .iter()
                .filter_map(|a| a.get("name").and_then(|x| x.as_str()))
                .collect();
            Err(format!(
                "有 {} 个资产同时匹配「{}*{}」，无法判断该下哪个：{}",
                n,
                prefix,
                suffix,
                names.join("、")
            ))
        }
    }
}

/// 配了 `github_asset` 的托管物：打 release 的 assets 拿真实下载 URL。
///
/// 返回 `Ok(None)` 表示这个托管物没配 `github_asset`，调用方应走 URL 模板那条老路。
pub(super) async fn async_resolve_github_asset(
    project_id: &str,
    version: &str,
) -> Result<Option<DownloadInfo>, String> {
    let Some(def) = crate::commands::project::registry::find_by_id(project_id) else {
        return Ok(None);
    };
    let Some(cfg) = def.github_asset.as_ref() else {
        return Ok(None);
    };
    let repo = cfg
        .get("repo")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "github_asset 缺少 repo".to_string())?;
    let tag_template = cfg
        .get("tag_template")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "github_asset 缺少 tag_template".to_string())?;
    let prefix_tpl = cfg
        .get("asset_name_prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let suffix = cfg
        .get("asset_name_suffix")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // 与 get_download_url 的版本清洗保持一致：去 v 前缀、去 -LTS
    let version_clean = version
        .trim_start_matches('v')
        .trim_end_matches("-LTS")
        .split(' ')
        .next()
        .unwrap_or(version);
    let tag = tag_template.replace("{version}", version_clean);
    let prefix = prefix_tpl.replace("{version}", version_clean);

    let url = format!(
        "https://api.github.com/repos/{}/releases/tags/{}",
        repo, tag
    );
    let client = reqwest::Client::builder()
        .user_agent("kira-sdk-manager")
        .build()
        .map_err(|e| format!("HTTP 客户端初始化失败: {}", e))?;
    let body = get_github_json(&client, &url).await?;
    let assets = body
        .get("assets")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    let (name, download_url) = pick_asset(&assets, &prefix, suffix)?;

    // 资产后缀即真实扩展名（如 .zip / .tar.gz）：比配置里写死的 download_file_ext 可靠
    let file_ext = cfg
        .get("file_ext")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            name.rsplit_once('.')
                .map(|(_, e)| e.to_string())
                .unwrap_or_else(|| "zip".to_string())
        });
    let extract_subdir = def
        .extract_subdir
        .as_ref()
        .map(|s| s.replace("{version}", version_clean));
    Ok(Some(DownloadInfo {
        url: download_url.to_string(),
        file_ext,
        extract_subdir,
    }))
}

pub(super) fn get_download_url(project_id: &str, version: &str) -> Result<DownloadInfo, String> {
    let def = crate::commands::project::registry::find_by_id(project_id)
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    // npm 包类型：无二进制归档 URL，由 do_npm_install 处理，这里返回空占位。
    if def.npm_pkg_name.is_some() {
        return Ok(DownloadInfo {
            url: String::new(),
            file_ext: "npm".to_string(),
            extract_subdir: None,
        });
    }

    // 如果版本字符串带有 version_label 前缀，先剥离
    let version_stripped = if let Some(ref cfg) = def.remote_versions_config {
        if let Some(label) = cfg.get("version_label").and_then(|v| v.as_str()) {
            version
                .strip_prefix(&format!("{} ", label))
                .unwrap_or(version)
        } else {
            version
        }
    } else {
        version
    };
    let version_clean = version_stripped
        .trim_start_matches('v')
        .split(' ')
        .next()
        .unwrap_or(version_stripped);
    // Clean version for download URL: strip "-LTS" suffix, encode "+" as "%2B"
    let version_url = version_clean.trim_end_matches("-LTS").replace('+', "%2B");
    let file_ext = def
        .download_file_ext
        .clone()
        .unwrap_or_else(|| "zip".to_string());

    // 1. 优先按版本前缀映射（如 java: adoptium-/microsoft-/oracle-/zulu-）
    if let Some(ref prefix_map) = def.version_prefix_map {
        let major_version = version_clean.split('.').next().unwrap_or("0");
        for (prefix, template) in prefix_map {
            if version_stripped.starts_with(prefix) {
                let ver = version_stripped
                    .trim_start_matches(prefix)
                    .trim_end_matches("-LTS")
                    .replace('+', "%2B");
                let url = template
                    .replace("{ver}", &ver)
                    .replace("{version}", &version_url)
                    .replace("{majorVersion}", major_version);
                let extract_subdir = def.extract_subdir.as_ref().map(|s| {
                    s.replace("{version}", &version_url)
                        .replace("{ver}", &ver)
                        .replace("{majorVersion}", major_version)
                });
                return Ok(DownloadInfo {
                    url,
                    file_ext,
                    extract_subdir,
                });
            }
        }
    }

    // 2. 按版本号前缀映射（如 mysql: 5.7/8.0/8.4）
    if let Some(ref url_prefix_map) = def.version_url_prefix_map {
        let major_version = version_clean.split('.').next().unwrap_or("0");
        for (ver_prefix, template) in url_prefix_map {
            if version_clean.starts_with(ver_prefix) {
                let url = template
                    .replace("{version}", &version_url)
                    .replace("{majorVersion}", major_version);
                let extract_subdir = def.extract_subdir.as_ref().map(|s| {
                    s.replace("{version}", &version_url)
                        .replace("{majorVersion}", major_version)
                });
                return Ok(DownloadInfo {
                    url,
                    file_ext,
                    extract_subdir,
                });
            }
        }
    }

    // 3. 使用通用 download_url_template（手动定义）
    if let Some(ref template) = def.download_url_template {
        let major_version = version_clean.split('.').next().unwrap_or("0");
        let url = template
            .replace("{version}", &version_url)
            .replace("{majorVersion}", major_version);
        let extract_subdir = def.extract_subdir.as_ref().map(|s| {
            s.replace("{version}", &version_url)
                .replace("{majorVersion}", major_version)
        });
        return Ok(DownloadInfo {
            url,
            file_ext,
            extract_subdir,
        });
    }

    Err(format!("未配置下载地址: {}", project_id))
}
