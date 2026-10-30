use super::*;

// ---------------------------------------------------------------------------
// Tauri 命令：证书监控 / 通知渠道
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn monitor_list() -> Vec<MonitorTarget> {
    load_json("monitors.json")
}

#[tauri::command]
pub fn monitor_upsert(
    id: Option<String>,
    host: String,
    port: Option<u16>,
    note: Option<String>,
    enabled: Option<bool>,
    notify_days: Option<Vec<i64>>,
) -> Result<MonitorTarget, String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("请填写要监控的域名或 IP".into());
    }
    let thresholds = match notify_days {
        Some(d) => {
            let mut v: Vec<i64> = d.into_iter().filter(|x| *x > 0).collect();
            v.sort_unstable();
            v.dedup();
            if v.is_empty() {
                default_notify_days()
            } else {
                v
            }
        }
        None => default_notify_days(),
    };
    let mut list = load_json::<MonitorTarget>("monitors.json");
    let mid = id.unwrap_or_else(|| gen_id("mon"));
    let target = if let Some(pos) = list.iter().position(|m| m.id == mid) {
        let mut m = list[pos].clone();
        m.host = host;
        m.port = port.unwrap_or(m.port);
        m.note = note.unwrap_or(m.note);
        m.enabled = enabled.unwrap_or(m.enabled);
        m.notify_days = thresholds;
        m
    } else {
        MonitorTarget {
            id: mid.clone(),
            host,
            port: port.unwrap_or(443),
            note: note.unwrap_or_default(),
            enabled: enabled.unwrap_or(true),
            notify_days: thresholds,
            not_after: None,
            days_left: None,
            subject: String::new(),
            issuer: String::new(),
            last_check_at: None,
            last_error: None,
            last_notify_bucket: None,
            last_notified_at: None,
        }
    };
    if let Some(pos) = list.iter().position(|m| m.id == mid) {
        list[pos] = target.clone();
    } else {
        list.push(target.clone());
    }
    save_json("monitors.json", &list);
    Ok(target)
}

#[tauri::command]
pub fn monitor_delete(id: String) {
    let list: Vec<MonitorTarget> = load_json::<MonitorTarget>("monitors.json")
        .into_iter()
        .filter(|m| m.id != id)
        .collect();
    save_json("monitors.json", &list);
}

#[tauri::command]
pub async fn monitor_check(id: String) -> Result<MonitorTarget, String> {
    let mut list = load_json::<MonitorTarget>("monitors.json");
    let channels = load_json::<NotifyChannel>("notify_channels.json");
    let m = list
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or_else(|| "监控目标不存在".to_string())?;
    check_one_monitor(m, &channels, true).await;
    let out = m.clone();
    save_json("monitors.json", &list);
    Ok(out)
}

#[tauri::command]
pub async fn monitor_check_all() -> Vec<MonitorTarget> {
    run_monitor_checks(true).await
}

#[tauri::command]
pub fn notify_channel_list() -> Vec<NotifyChannel> {
    load_json("notify_channels.json")
}

#[tauri::command]
pub fn notify_channel_upsert(
    id: Option<String>,
    channel_type: String,
    name: String,
    config: HashMap<String, String>,
    enabled: Option<bool>,
    note: Option<String>,
) -> Result<NotifyChannel, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("请填写通知渠道名称".into());
    }
    let url = config.get("url").cloned().unwrap_or_default();
    if url.trim().is_empty() {
        return Err("请填写 Webhook 地址".into());
    }
    let mut list = load_json::<NotifyChannel>("notify_channels.json");
    let cid = id.unwrap_or_else(|| gen_id("notify"));
    let channel = if let Some(pos) = list.iter().position(|c| c.id == cid) {
        let mut c = list[pos].clone();
        c.channel_type = channel_type;
        c.name = name;
        c.config = config;
        c.enabled = enabled.unwrap_or(c.enabled);
        c.note = note.unwrap_or(c.note);
        c
    } else {
        NotifyChannel {
            id: cid.clone(),
            channel_type,
            name,
            config,
            enabled: enabled.unwrap_or(true),
            note: note.unwrap_or_default(),
        }
    };
    if let Some(pos) = list.iter().position(|c| c.id == cid) {
        list[pos] = channel.clone();
    } else {
        list.push(channel.clone());
    }
    save_json("notify_channels.json", &list);
    Ok(channel)
}

#[tauri::command]
pub fn notify_channel_delete(id: String) {
    let list: Vec<NotifyChannel> = load_json::<NotifyChannel>("notify_channels.json")
        .into_iter()
        .filter(|c| c.id != id)
        .collect();
    save_json("notify_channels.json", &list);
}

#[tauri::command]
pub async fn notify_channel_test(id: String) -> Result<String, String> {
    let list = load_json::<NotifyChannel>("notify_channels.json");
    let channel = list
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| "通知渠道不存在".to_string())?;
    notify_send(
        channel,
        "AnyVersion 测试通知",
        "这是一条测试消息：证书监控与部署通知已配置成功。",
    )
    .await?;
    Ok("已发送".to_string())
}

// ---------------------------------------------------------------------------
// 测试：有效期解析 / Windows 接收端 URL 归一化
// ---------------------------------------------------------------------------
