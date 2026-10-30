use super::*;

// ---- 升级 ----
#[tauri::command]
pub async fn mihomo_upgrade(
    app: AppHandle,
    state: State<'_, MihomoState>,
) -> Result<String, String> {
    let (_, url) = get_latest_mihomo_release().await?;
    // F1 修复：仅允许官方源（github.com/MetaCubeX），并做 SHA256 校验
    if !url.starts_with("https://github.com/MetaCubeX") {
        return Err(format!("拒绝非官方升级源: {url}"));
    }
    // 仅官方 release 提供固定 SHA256 摘要；此处用发布清单中的校验（无法静态获取时退化为比对域名）
    let expected = std::env::var("MIHOMO_RELEASE_SHA256").unwrap_or_default();
    let data = if expected.is_empty() {
        download_bytes(&url).await?
    } else {
        download_bytes_verified(&url, &expected).await?
    };
    let dest = state.data_dir.join("bin");
    unzip_to(&data, &dest)?;
    let mut found: Option<PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(&dest) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "exe").unwrap_or(false)
                && p.file_name()
                    .map(|n| n.to_string_lossy().contains("mihomo"))
                    .unwrap_or(false)
            {
                found = Some(p);
                break;
            }
        }
    }
    if let Some(src) = found {
        let target = dest.join("mihomo.exe");
        stop_core(&state);
        let _ = std::fs::remove_file(&target);
        std::fs::copy(&src, &target).map_err(|e| e.to_string())?;
    } else {
        return Err("解压后未找到 mihomo 可执行文件".into());
    }
    refresh_core_version(&state);
    emit_state(&app, &state);
    Ok("核心已升级".into())
}
#[tauri::command]
pub async fn mihomo_upgrade_geo(state: State<'_, MihomoState>) -> Result<String, String> {
    let base = "https://github.com/MetaCubeX/meta-rules-dat/releases/latest/download";
    for (name, out) in [
        ("geoip.metadb", "geoip.metadb"),
        ("geosite.dat", "geosite.dat"),
        ("geoip.dat", "geoip.dat"),
    ] {
        let url = format!("{}/{}", base, name);
        if let Ok(d) = download_bytes(&url).await {
            std::fs::write(state.data_dir.join(out), d).ok();
        }
    }
    Ok("Geo 数据已更新".into())
}
#[tauri::command]
pub fn mihomo_upgrade_ui() -> Value {
    serde_json::json!({ "url": "http://127.0.0.1:9090/ui", "message": "在浏览器打开外部面板（如 zashboard/yacd）" })
}

// ---- SubStore（最小化集成）----
#[tauri::command]
pub fn mihomo_open_substore(state: State<'_, MihomoState>) -> Value {
    serde_json::json!({ "enabled": state.app_config.lock().unwrap().substore_enabled, "message": "SubStore 需在 clash-party 中独立部署，此处仅作配置开关" })
}

// ---- 防火墙 / UWP 回环 / 覆写执行日志 ----

/// 重置 Windows 防火墙规则：为 mihomo 内核放行（TUN 模式常见故障排查项）
#[tauri::command]
pub fn mihomo_setup_firewall(state: State<'_, MihomoState>) -> Result<String, String> {
    let core = resolve_core_path(&state.app_config.lock().unwrap().clone());
    if !core.exists() {
        return Err("未找到 mihomo 内核，无法配置防火墙".into());
    }
    let core_str = core.to_string_lossy().to_string();
    let rule = "AnyVersion Mihomo";
    // 先删除旧规则（忽略失败）
    let _ = hidden_cmd("netsh")
        .args([
            "advfirewall",
            "firewall",
            "delete",
            "rule",
            &format!("name={}", rule),
        ])
        .output();
    let mut errs: Vec<String> = Vec::new();
    for dir in ["in", "out"] {
        let out = hidden_cmd("netsh")
            .args([
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={}", rule),
                &format!("dir={}", dir),
                "action=allow",
                &format!("program={}", core_str),
                "enable=yes",
                "profile=any",
            ])
            .output()
            .map_err(|e| format!("执行 netsh 失败: {e}"))?;
        if !out.status.success() {
            errs.push(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
    }
    if errs.is_empty() {
        Ok("防火墙规则已重置".into())
    } else {
        Err(format!(
            "部分规则设置失败（可能需要管理员权限）: {}",
            errs.join("; ")
        ))
    }
}

/// 打开 UWP 回环豁免工具（Windows 应用/微软商店应用无法走代理时使用）
#[tauri::command]
pub fn mihomo_open_uwp_tool() -> Result<String, String> {
    // 单一路径策略：仅从 data_dir/bin 查找 enableLoopback 工具，不从程序根目录读取。
    let enable_loopback = crate::commands::utils::get_bin_dir().join("enableLoopback.exe");
    if enable_loopback.exists() {
        hidden_cmd(&enable_loopback)
            .spawn()
            .map_err(|e| format!("启动 UWP 工具失败: {e}"))?;
        return Ok("已打开 UWP 回环豁免工具".into());
    }
    // 回退：打开系统自带的 CheckNetIsolation（命令行方式列出 UWP 应用）
    hidden_cmd("cmd")
        .args(["/c", "start", "", "CheckNetIsolation.exe"])
        .spawn()
        .map_err(|e| format!("未找到 enableLoopback.exe，且启动 CheckNetIsolation 失败: {e}"))?;
    Ok("未找到 enableLoopback.exe，已改为打开系统 CheckNetIsolation".into())
}
