//! managed responsibility.
use super::*;

// ─── 托管安装（GitHub Release → 下载 → 解压 → command 指向可执行文件） ───

/// 托管安装根：`<data_dir>/mcp-servers/<server_id>/<version>/`。
///
/// 与 SDK 的 `sdk/_versions` 分开：MCP 服务器不是语言/CLI，混进 SDK 目录会让
/// 「卸载 SDK 工具」的清理逻辑误伤它，用户的 SDK 列表也会多出看不懂的条目。
pub(super) fn managed_root() -> PathBuf {
    get_data_dir().join("mcp-servers")
}

pub(super) fn managed_version_dir(id: &str, version: &str) -> PathBuf {
    managed_root().join(id).join(version)
}

/// 版本号排序键：把 `0.10.2` 拆成 `[0, 10, 2]`。
/// 直接按字符串比较会把 `0.10.0` 排到 `0.8.0` 后面（"1" < "8"）。
pub(super) fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u64>().unwrap_or(0))
        .collect()
}

pub(super) fn installed_versions(id: &str) -> Vec<String> {
    let dir = managed_root().join(id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();
    out.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    out
}

pub(super) fn find_server(id: &str) -> Result<McpServer, String> {
    load_store()
        .servers
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("MCP 服务器不存在: {}", id))
}

pub(super) fn managed_executable(id: &str, managed: &McpManagedSource, version: &str) -> PathBuf {
    let rel = managed
        .executable
        .trim()
        .replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR);
    managed_version_dir(id, version).join(rel)
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpManagedStatus {
    /// 已安装版本（新 → 旧）
    pub installed_versions: Vec<String>,
    /// 当前 `command` 指向的文件是否真的存在（决定前端是否允许「装到项目」）
    pub executable_exists: bool,
    pub command: String,
}

pub(super) fn status_of(server: &McpServer) -> McpManagedStatus {
    let command = server.command.trim().to_string();
    McpManagedStatus {
        installed_versions: installed_versions(&server.id),
        executable_exists: !command.is_empty() && Path::new(&command).is_file(),
        command,
    }
}

#[tauri::command]
pub fn mcp_managed_status(id: String) -> Result<McpManagedStatus, String> {
    Ok(status_of(&find_server(&id)?))
}

/// `v{version}` → (`v`, ``)；`jadx-{version}-win` → (`jadx-`, `-win`)
pub(super) fn split_template(tpl: &str) -> (&str, &str) {
    match tpl.split_once("{version}") {
        Some((prefix, suffix)) => (prefix, suffix),
        None => (tpl, ""),
    }
}

/// 列出该托管来源可安装的版本（GitHub Releases，新 → 旧）。
#[tauri::command]
pub async fn mcp_list_managed_versions(id: String) -> Result<Vec<String>, String> {
    let server = find_server(&id)?;
    let managed = server
        .managed
        .as_ref()
        .ok_or_else(|| "该服务器没有配置托管安装来源".to_string())?;
    if managed.repo.trim().is_empty() {
        return Err("托管来源缺少仓库（owner/repo）".to_string());
    }
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!(
        "https://api.github.com/repos/{}/releases",
        managed.repo.trim()
    );
    let data = crate::commands::project::versions::get_github_json(&client, &url).await?;
    let arr = data
        .as_array()
        .ok_or_else(|| "GitHub 返回的不是 release 列表".to_string())?;
    let (prefix, suffix) = split_template(&managed.tag_template);
    let mut out: Vec<String> = arr
        .iter()
        .filter_map(|release| {
            let tag = release.get("tag_name")?.as_str()?;
            let rest = tag.strip_prefix(prefix)?;
            let ver = if suffix.is_empty() {
                rest
            } else {
                rest.strip_suffix(suffix)?
            };
            if ver.trim().is_empty() {
                None
            } else {
                Some(ver.to_string())
            }
        })
        .collect();
    out.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    out.dedup();
    Ok(out)
}

/// 安装指定版本：解析 release → 挑资产 → 下载 → 解压 → 落盘 → 改写 `command`。
///
/// 进度通过 `mcp-managed-progress` 事件推送（`{ id, version, downloaded, total, speed }`）。
#[tauri::command]
pub async fn mcp_install_managed(
    app: tauri::AppHandle,
    id: String,
    version: String,
) -> Result<McpManagedStatus, String> {
    use tauri::Emitter;

    let mut store = load_store();
    let idx = store
        .servers
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| format!("MCP 服务器不存在: {}", id))?;
    let managed = store.servers[idx]
        .managed
        .clone()
        .ok_or_else(|| "该服务器没有配置托管安装来源".to_string())?;
    if managed.executable.trim().is_empty() {
        return Err("托管来源缺少可执行文件路径".to_string());
    }

    let version = version.trim().to_string();
    if version.is_empty() {
        return Err("版本号为空".to_string());
    }
    // 版本号会进目录名：挡掉路径穿越（`..`、分隔符）
    if version.contains("..") || version.contains('/') || version.contains('\\') {
        return Err(format!("版本号非法: {}", version));
    }

    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .build()
        .map_err(|e| e.to_string())?;
    let releases = crate::commands::project::versions::get_github_json(
        &client,
        &format!(
            "https://api.github.com/repos/{}/releases",
            managed.repo.trim()
        ),
    )
    .await?;
    let tag = managed.tag_template.replace("{version}", &version);
    let release = releases
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|r| r.get("tag_name").and_then(|t| t.as_str()) == Some(tag.as_str()))
        })
        .ok_or_else(|| format!("未找到 release: {}（{}）", tag, managed.repo))?;
    let assets = release
        .get("assets")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    let (asset_name, asset_url) = crate::commands::project::versions::pick_asset(
        &assets,
        &managed.asset_prefix.replace("{version}", &version),
        &managed.asset_suffix,
    )?;
    if !asset_name.to_ascii_lowercase().ends_with(".zip") {
        return Err(format!(
            "目前只支持 zip 发行包，该版本提供的是: {}",
            asset_name
        ));
    }

    let tmp_root = managed_root().join(".tmp");
    fs::create_dir_all(&tmp_root).map_err(|e| format!("创建临时目录失败: {}", e))?;
    let archive = tmp_root.join(&asset_name);
    let emit_id = id.clone();
    let emit_version = version.clone();
    crate::commands::project::versions::download_with_progress(
        &asset_url,
        &archive,
        move |downloaded, total, speed| {
            let _ = app.emit(
                "mcp-managed-progress",
                serde_json::json!({
                    "id": emit_id,
                    "version": emit_version,
                    "downloaded": downloaded,
                    "total": total,
                    "speed": speed,
                }),
            );
        },
    )
    .await?;

    let extract_dir = tmp_root.join(format!("{}-{}", id, version));
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir).map_err(|e| format!("清理旧解压目录失败: {}", e))?;
    }
    crate::commands::project::versions::unzip_file(&archive, &extract_dir)?;

    let src = if managed.extract_subdir.trim().is_empty() {
        extract_dir.clone()
    } else {
        let sub = managed.extract_subdir.replace("{version}", &version);
        let p = extract_dir.join(&sub);
        if !p.is_dir() {
            return Err(format!("解压后未找到目录: {}", sub));
        }
        p
    };
    let dest = managed_version_dir(&id, &version);
    crate::commands::project::versions::move_extract_to_dest(&src, &dest, false)?;

    let exe = managed_executable(&id, &managed, &version);
    if !exe.is_file() {
        return Err(format!("安装后未找到可执行文件: {}", exe.display()));
    }

    let _ = fs::remove_file(&archive);
    let _ = fs::remove_dir_all(&extract_dir);

    store.servers[idx].command = exe.to_string_lossy().to_string();
    save_store(&store)?;
    Ok(status_of(&store.servers[idx]))
}

/// 卸载托管安装：删掉该服务器的整个安装目录，并清空 `command`。
///
/// 清空 command 是刻意的：留着旧路径会让「装到项目」写出一个指向已删文件的配置，
/// 客户端启动时只会报 ENOENT，看不出是这里的锅。
#[tauri::command]
pub fn mcp_uninstall_managed(id: String) -> Result<McpManagedStatus, String> {
    let mut store = load_store();
    let idx = store
        .servers
        .iter()
        .position(|s| s.id == id)
        .ok_or_else(|| format!("MCP 服务器不存在: {}", id))?;
    if store.servers[idx].managed.is_none() {
        return Err("该服务器不是托管安装，无法卸载".to_string());
    }
    let dir = managed_root().join(&id);
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("删除安装目录失败: {}", e))?;
    }
    store.servers[idx].command = String::new();
    save_store(&store)?;
    Ok(status_of(&store.servers[idx]))
}
