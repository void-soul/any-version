use super::*;

// ============ 托盘快捷操作（供 tray.rs 调用，不经过前端） ============

pub(super) fn tray_state(app: &AppHandle) -> Result<MihomoState, String> {
    app.try_state::<MihomoState>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "mihomo 未初始化".to_string())
}

pub(super) async fn tray_api(
    app: &AppHandle,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<String, String> {
    let state = tray_state(app)?;
    let app_config = state.app_config.lock().unwrap().clone();
    api::mihomo_api_raw(&app_config, method, path, body).await
}

/// 托盘：切换规则/全局/直连
pub async fn tray_set_mode(app: &AppHandle, mode: &str) -> Result<(), String> {
    tray_api(
        app,
        Method::PATCH,
        "/configs",
        Some(serde_json::json!({ "mode": mode })),
    )
    .await?;
    let state = tray_state(app)?;
    emit_state(app, &state);
    Ok(())
}

/// 托盘：切换代理组节点
pub async fn tray_change_proxy(app: &AppHandle, group: &str, node: &str) -> Result<(), String> {
    tray_api(
        app,
        Method::PUT,
        &format!("/proxies/{}", urlencoding(group)),
        Some(serde_json::json!({ "name": node })),
    )
    .await?;
    Ok(())
}

/// 托盘：切换订阅（等价于 mihomo_change_current_profile）
pub async fn tray_change_profile(app: &AppHandle, id: &str) -> Result<(), String> {
    let state = tray_state(app)?;
    if !state
        .profile_config
        .lock()
        .unwrap()
        .items
        .iter()
        .any(|i| i.id == id)
    {
        return Err("profile 不存在".into());
    }
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.current_profile = id.to_string();
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(app, &state);
    if !state.stop_flag.load(Ordering::SeqCst) {
        reload_config(app, Arc::clone(&state)).await?;
    }
    Ok(())
}

/// 托盘：拉取一次「当前模式 + 代理组」快照
pub async fn tray_snapshot(
    app: &AppHandle,
) -> Option<(String, Vec<(String, String, Vec<String>)>)> {
    let mode = tray_api(app, Method::GET, "/configs", None)
        .await
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v.get("mode")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    let proxies = tray_api(app, Method::GET, "/proxies", None)
        .await
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())?;
    let map = proxies.get("proxies")?.as_object()?;
    let mut groups: Vec<(String, String, Vec<String>)> = Vec::new();
    for (name, item) in map {
        let ty = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        // 仅可手动切换的组
        if !matches!(ty, "Selector" | "Fallback" | "URLTest" | "LoadBalance") {
            continue;
        }
        let all: Vec<String> = item
            .get("all")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if all.is_empty() {
            continue;
        }
        let now = item
            .get("now")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        groups.push((name.clone(), now, all));
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    Some((mode, groups))
}

pub(super) fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
