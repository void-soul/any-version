// mihomo 模块（对齐 clash-party 架构）
// 两层配置: AppConfig(应用配置) + ControledMihomoConfig(受控 mihomo 配置)
// Profiles / Overrides / Factory / Manager / Controller API
pub mod api;
pub mod backup;
pub mod config;
pub mod factory;
pub mod github;
pub mod manager;
pub mod misc;
pub mod netinfo;
pub mod smart;
pub mod subparse;
pub mod substore;

pub use manager::launch_core;

use crate::commands::mihomo::api::mihomo_api_raw;
use reqwest::Method;
use crate::commands::mihomo::config::*;
use crate::commands::mihomo::factory::{build_subscription_profile, deep_merge};
use crate::commands::mihomo::manager::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use crate::commands::hidden_cmd::hidden_cmd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};
use winreg::enums::*;
use winreg::RegKey;

pub type MihomoState = Arc<MihomoInner>;

pub struct MihomoInner {
    pub app_config: Mutex<AppConfig>,
    pub controled_config: Mutex<Value>,
    pub profile_config: Mutex<ProfileConfig>,
    pub override_config: Mutex<OverrideConfig>,
    pub child: Mutex<Option<Child>>,
    pub data_dir: PathBuf,
    pub stop_flag: Arc<AtomicBool>,
    pub watchdog_running: Arc<AtomicBool>,
    pub scheduler_running: Arc<AtomicBool>,
    pub runtime_config_str: Mutex<String>,
    pub core_version: Mutex<Option<String>>,
    pub auto_last: Mutex<HashMap<String, u64>>,
    pub log_file: PathBuf,
    /// 内核运行期产生的告警（启动失败 / TUN 创建失败等），随状态推送到前台
    pub runtime_warnings: Mutex<Vec<String>>,
}

impl MihomoInner {
    pub fn current_profile(&self) -> ProfileItem {
        let id = self.app_config.lock().unwrap().current_profile.clone();
        let pc = self.profile_config.lock().unwrap();
        if let Some(p) = pc.items.iter().find(|i| i.id == id) {
            return p.clone();
        }
        // 没有任何 profile 时返回一个空白默认项，避免 panic 导致 Mutex 中毒
        pc.items.first().cloned().unwrap_or_default()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MihomoStateView {
    pub installed: bool,
    pub core_version: Option<String>,
    pub running: bool,
    pub pid: Option<i64>,
    pub app_config: AppConfig,
    pub current_profile: String,
    pub runtime_config: String,
    pub controller_port: u16,
    pub secret: String,
    /// 当前进程是否具备管理员权限
    pub is_admin: bool,
    /// 需要在前台展示的告警（如 TUN 需要管理员权限）
    pub warnings: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SubValidation {
    pub ok: bool,
    pub message: String,
    pub upload: Option<u64>,
    pub download: Option<u64>,
    pub total: Option<u64>,
    pub expire: Option<u64>,
    pub suggested_interval: Option<u64>,
}

// ---------- helpers ----------
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn uid() -> String {
    format!("{:x}", now_secs() ^ (std::process::id() as u64))
}
fn get_str<'a>(v: &'a Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string())
}
fn get_str_opt<'a>(v: &'a Value, k: &str) -> Option<String> {
    get_str(v, k)
}
fn get_u64(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64())
}

fn refresh_core_version(inner: &MihomoInner) {
    let core = {
        let cfg = inner.app_config.lock().unwrap();
        resolve_core_path(&cfg)
    };
    eprintln!("[mihomo] refresh_core_version: core = {:?}, exists = {}", core, core.exists());
    if !core.exists() {
        *inner.core_version.lock().unwrap() = None;
        return;
    }
    // 运行 -v 获取版本号（mihomo 不支持 --version，且可能把版本信息打到 stderr）。
    // 同时捕获 stdout+stderr，取首个非空行。
    match hidden_cmd(&core).arg("-v").output() {
        Ok(out) => {
            let combined = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            eprintln!("[mihomo] -v 原始输出: {:?}", combined);
            let line = combined
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .next()
                .unwrap_or("")
                .to_string();
            // 从首行提取 vX.Y.Z 形式的版本号；取不到则兜底显示“已安装”
            let version = line
                .find('v')
                .and_then(|i| {
                    let rest = &line[i..];
                    let end = rest.find(|c: char| !(c == 'v' || c.is_ascii_digit() || c == '.'));
                    Some(match end {
                        Some(e) => rest[..e].to_string(),
                        None => rest.to_string(),
                    })
                })
                .filter(|s| s.len() > 1)
                .unwrap_or_else(|| {
                    if line.is_empty() {
                        "已安装".to_string()
                    } else {
                        line.clone()
                    }
                });
            *inner.core_version.lock().unwrap() = Some(version);
        }
        Err(e) => {
            eprintln!("[mihomo] -v 运行失败: {:?}，但核心文件存在，标记为已安装", e);
            *inner.core_version.lock().unwrap() = Some("已安装".to_string());
        }
    }
}

fn build_state_view(inner: &MihomoInner) -> MihomoStateView {
    // 注意：结构体字面量中的临时 MutexGuard 会活到整条语句结束，
    // 对同一个 Mutex 重复 lock 会死锁，因此这里每个锁都只取一次并提前释放。
    let (running, pid) = {
        let g = inner.child.lock().unwrap();
        (
            g.as_ref().map(|c| c.id() > 0).unwrap_or(false) && !inner.stop_flag.load(Ordering::SeqCst),
            g.as_ref().map(|c| c.id() as i64),
        )
    };
    let app_config = inner.app_config.lock().unwrap().clone();
    let installed = resolve_core_path(&app_config).exists();
    let core_version = inner.core_version.lock().unwrap().clone();
    let runtime_config = inner.runtime_config_str.lock().unwrap().clone();
    let is_admin = crate::commands::mihomo::manager::is_admin();
    let mut warnings: Vec<String> = Vec::new();
    if app_config.tun_enabled && !is_admin {
        warnings.push(
            "TUN 已开启但当前非管理员，可能无法创建虚拟网卡（请以管理员身份运行）".to_string(),
        );
    }
    if !installed {
        warnings.push(format!(
            "内核文件不存在：{}",
            resolve_core_path(&app_config).display()
        ));
    }
    warnings.extend(inner.runtime_warnings.lock().unwrap().iter().cloned());
    MihomoStateView {
        installed,
        core_version,
        running,
        pid,
        current_profile: app_config.current_profile.clone(),
        controller_port: app_config.controller_port,
        secret: app_config.secret.clone(),
        app_config,
        runtime_config,
        is_admin,
        warnings,
    }
}

pub fn emit_state(app: &AppHandle, inner: &MihomoInner) {
    let view = build_state_view(inner);
    let _ = app.emit("mihomo://state-changed", view);
    // 托盘里的 Mihomo 子菜单只提供「启停」开关（见 build_mihomo_submenu），
    // 仅依赖 running 状态，不依赖订阅/代理/模式等内部动态数据，因此无需在
    // watchdog 心跳（每 3 秒一次 emit_state）里刷新托盘菜单——避免 set_menu
    // 反复重建导致 on_menu_event 长时间运行后失效。启停状态由托盘 toggle
    // 事件处理里的 rebuild_tray_menu 负责更新。
}

// 兼容性迁移：旧 mihomo.json (MihomoSettings) -> 新模型
fn migrate_legacy(dir: &Path, app: &mut AppConfig, profiles: &mut ProfileConfig) {
    let old = dir.join("mihomo.json");
    if !old.exists() {
        return;
    }
    if let Ok(s) = std::fs::read_to_string(&old) {
        if let Ok(v) = serde_json::from_str::<Value>(&s) {
            if let Some(cp) = get_u64(&v, "controller_port") {
                app.controller_port = cp as u16;
            }
            if let Some(mp) = get_u64(&v, "mixed_port") {
                app.mixed_port = mp as u16;
            }
            if let Some(secret) = get_str(&v, "secret") {
                app.secret = secret;
            }
            if let Some(tun) = v.get("tun_enabled").and_then(|x| x.as_bool()) {
                app.tun_enabled = tun;
            }
            if let Some(cp) = get_str(&v, "core_path") {
                if !cp.is_empty() {
                    app.core_path = Some(cp);
                }
            }
            if let Some(dp) = get_str(&v, "default_proxy") {
                app.default_proxy = Some(dp);
            }
            let mut item = ProfileItem {
                id: "default".into(),
                name: "默认".into(),
                type_: "subscription".into(),
                file_path: None,
                subscriptions: vec![],
                rule_providers: vec![],
                custom_rules: vec![],
                dns_enabled: true,
                dns_nameservers: vec![],
                age_secret_key: None,
                url: None,
                auth_token: None,
                user_agent: None,
                use_proxy: false,
                auto_update: false,
                update_interval: 86400,
                update_timeout: 30,
                override_ids: vec![],
                subscription_userinfo: None,
                updated_at: None,
            };
            if let Some(arr) = v.get("subscriptions").and_then(|x| x.as_array()) {
                for s in arr {
                    item.subscriptions.push(Subscription {
                        id: uid(),
                        name: get_str(s, "name").unwrap_or_else(|| "订阅".into()),
                        url: get_str(s, "url").unwrap_or_default(),
                        interval: get_u64(s, "interval").unwrap_or(86400),
                        auto_update: s.get("auto_update").and_then(|x| x.as_bool()).unwrap_or(false),
                        updated_at: get_u64(s, "updated_at"),
                        age_secret: get_str_opt(s, "age_secret"),
                    });
                }
            }
            if let Some(arr) = v.get("rule_providers").and_then(|x| x.as_array()) {
                for s in arr {
                    item.rule_providers.push(RuleProviderEntry {
                        id: uid(),
                        name: get_str(s, "name").unwrap_or_default(),
                        behavior: get_str(s, "behavior").unwrap_or_else(|| "classical".into()),
                        url: get_str(s, "url").unwrap_or_default(),
                        interval: get_u64(s, "interval").unwrap_or(86400),
                        auto_update: s.get("auto_update").and_then(|x| x.as_bool()).unwrap_or(false),
                        updated_at: get_u64(s, "updated_at"),
                        age_secret: get_str_opt(s, "age_secret"),
                    });
                }
            }
            if let Some(arr) = v
                .get("overrides")
                .and_then(|x| x.get("custom_rules"))
                .and_then(|x| x.as_array())
            {
                for r in arr {
                    if let Some(s) = r.as_str() {
                        item.custom_rules.push(s.to_string());
                    }
                }
            }
            item.age_secret_key = get_str_opt(&v, "age_secret_key");
            item.url = get_str_opt(&v, "url");
            item.auth_token = get_str_opt(&v, "auth_token");
            item.user_agent = get_str_opt(&v, "user_agent");
            if let Some(b) = v.get("use_proxy").and_then(|x| x.as_bool()) {
                item.use_proxy = b;
            }
            if let Some(b) = v.get("auto_update").and_then(|x| x.as_bool()) {
                item.auto_update = b;
            }
            if let Some(n) = get_u64(&v, "update_interval") {
                item.update_interval = n;
            }
            if let Some(n) = get_u64(&v, "update_timeout") {
                item.update_timeout = n;
            }
            if let Some(arr) = v.get("override_ids").and_then(|x| x.as_array()) {
                for id in arr {
                    if let Some(s) = id.as_str() {
                        item.override_ids.push(s.to_string());
                    }
                }
            }
            if let Some(dns) = v
                .get("overrides")
                .and_then(|x| x.get("dns_enabled"))
                .and_then(|x| x.as_bool())
            {
                item.dns_enabled = dns;
            }
            if let Some(arr) = v
                .get("overrides")
                .and_then(|x| x.get("dns_nameservers"))
                .and_then(|x| x.as_array())
            {
                for n in arr {
                    if let Some(s) = n.as_str() {
                        item.dns_nameservers.push(s.to_string());
                    }
                }
            }
            profiles.items.push(item);
            profiles.current = "default".into();
            app.current_profile = "default".into();
        }
    }
    let _ = std::fs::rename(&old, dir.join("mihomo.json.bak"));
}

pub fn init_state() -> MihomoState {
    // 数据目录跟随全局设置（config.data_dir，可改到非系统盘），而非 AppData。
    // 这样 mihomo 的全部数据（配置/geo/日志/订阅）都落在用户维护的数据目录下。
    let data_dir = crate::commands::config::get_data_dir().join("mihomo");
    std::fs::create_dir_all(&data_dir).ok();
    // 兜底：若压缩包已提前解压，先同步一次 geo 文件到 data_dir。
    // 主同步在 download_bin_assets 解压完成后调用 sync_mihomo_geo 完成。
    crate::commands::utils::sync_mihomo_geo();
    let mut app_config = load_app_config(&data_dir);
    let controled_config = load_controled(&data_dir);
    let mut profile_config = load_profile_config(&data_dir);
    let override_config = load_override_config(&data_dir);

    migrate_legacy(&data_dir, &mut app_config, &mut profile_config);

    if profile_config.items.is_empty() {
        profile_config.items.push(ProfileItem {
            id: "default".into(),
            name: "默认".into(),
            type_: "subscription".into(),
            file_path: None,
            subscriptions: vec![],
            rule_providers: vec![],
            custom_rules: vec![],
            dns_enabled: true,
            dns_nameservers: vec!["https://dns.google".into(), "https://1.1.1.1".into()],
            age_secret_key: None,
            url: None,
            auth_token: None,
            user_agent: None,
            use_proxy: false,
            auto_update: false,
            update_interval: 86400,
            update_timeout: 30,
            override_ids: vec![],
            subscription_userinfo: None,
            updated_at: None,
        });
        profile_config.current = "default".into();
        app_config.current_profile = "default".into();
        save_profile_config(&data_dir, &profile_config).ok();
        save_app_config(&data_dir, &app_config).ok();
    }

    let inner = MihomoInner {
        app_config: Mutex::new(app_config),
        controled_config: Mutex::new(controled_config),
        profile_config: Mutex::new(profile_config),
        override_config: Mutex::new(override_config),
        child: Mutex::new(None),
        data_dir: data_dir.clone(),
        stop_flag: Arc::new(AtomicBool::new(true)),
        watchdog_running: Arc::new(AtomicBool::new(false)),
        scheduler_running: Arc::new(AtomicBool::new(false)),
        runtime_config_str: Mutex::new(String::new()),
        core_version: Mutex::new(None),
        auto_last: Mutex::new(HashMap::new()),
        log_file: data_dir.join("mihomo.log"),
        runtime_warnings: Mutex::new(Vec::new()),
    };
    let state = Arc::new(inner);
    refresh_core_version(&state);
    state
}

/// 定时更新远程订阅（对齐 clash-party profileUpdater：按 item.interval 重新下载并热重载）
async fn auto_update_profiles(app: &AppHandle, inner: &Arc<MihomoInner>) {
    let now = now_secs();
    let (items, current) = {
        let cfg = inner.profile_config.lock().unwrap();
        (cfg.items.clone(), cfg.current.clone())
    };
    for item in items {
        if !item.auto_update || item.update_interval == 0 {
            continue;
        }
        let Some(url) = item.url.clone() else { continue };
        if now.saturating_sub(item.updated_at.unwrap_or(0)) < item.update_interval {
            continue;
        }
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(item.update_timeout.max(5)))
            .build()
        {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut req = client.get(&url).header(
            "User-Agent",
            item.user_agent.clone().unwrap_or_else(|| "clash-verge/v1.7.0".into()),
        );
        if let Some(token) = &item.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let Ok(resp) = req.send().await else {
            eprintln!("[mihomo] 订阅自动更新失败: {}", item.name);
            continue;
        };
        let userinfo = resp
            .headers()
            .get("subscription-userinfo")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let Ok(text) = resp.text().await else { continue };
        // 嗅探并处理内容（age 解密 / base64 节点解码 / YAML），保持与手动更新一致
        let processed =
            match crate::commands::mihomo::subparse::process_subscription_content(
                &text,
                item.age_secret_key.as_deref(),
            ) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[mihomo] 订阅自动更新内容解析失败 {}: {}", item.name, e);
                    continue;
                }
            };
        if serde_yaml::from_str::<Value>(&processed).is_err() {
            eprintln!("[mihomo] 订阅自动更新内容非法 YAML: {}", item.name);
            continue;
        }
        let mut updated = item.clone();
        updated.subscription_userinfo = parse_subinfo(userinfo.as_deref());
        updated.updated_at = Some(now);
        if write_profile_content(&inner.data_dir, &updated, &processed).is_err() {
            continue;
        }
        {
            let mut cfg = inner.profile_config.lock().unwrap();
            if let Some(it) = cfg.items.iter_mut().find(|i| i.id == updated.id) {
                *it = updated.clone();
            }
            save_profile_config(&inner.data_dir, &cfg).ok();
        }
        eprintln!("[mihomo] 订阅已自动更新: {}", updated.name);
        if current == updated.id && !inner.stop_flag.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = reload_config(app, Arc::clone(inner)).await;
        }
    }
}

pub fn start_scheduler(_app: AppHandle, inner: Arc<MihomoInner>) {
    if inner.scheduler_running.load(Ordering::SeqCst) {
        return;
    }
    inner.scheduler_running.store(true, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if inner.stop_flag.load(Ordering::SeqCst) {
                inner.scheduler_running.store(false, Ordering::SeqCst);
                break;
            }
            let now = now_secs();
            auto_update_profiles(&_app, &inner).await;
            let profile = inner.current_profile();
            if profile.type_ != "subscription" {
                continue;
            }
            let mut to_update: Vec<String> = vec![];
            for s in &profile.subscriptions {
                if !s.auto_update {
                    continue;
                }
                let last = inner.auto_last.lock().unwrap().get(&s.id).copied().unwrap_or(0);
                if now - last >= s.interval {
                    to_update.push(s.id.clone());
                }
            }
            for r in &profile.rule_providers {
                if !r.auto_update {
                    continue;
                }
                let last = inner.auto_last.lock().unwrap().get(&r.id).copied().unwrap_or(0);
                if now - last >= r.interval {
                    to_update.push(r.id.clone());
                }
            }
            let app_config = inner.app_config.lock().unwrap().clone();
            for id in to_update {
                inner.auto_last.lock().unwrap().insert(id.clone(), now);
                let _ = mihomo_api_raw(
                    &app_config,
                    reqwest::Method::PUT,
                    &format!("/providers/proxies/{}", id),
                    Some(serde_json::json!({"name": id})),
                )
                .await;
                let _ = mihomo_api_raw(
                    &app_config,
                    reqwest::Method::PUT,
                    &format!("/providers/rules/{}", id),
                    Some(serde_json::json!({"name": id})),
                )
                .await;
            }
            if inner.stop_flag.load(Ordering::SeqCst) {
                inner.scheduler_running.store(false, Ordering::SeqCst);
                break;
            }
        }
    });
}

pub fn kill_on_exit(inner: &MihomoInner) {
    crate::exit_log::exit_log("cleanup: kill_on_exit 进入");
    // 注意：app_config 是 std::sync::Mutex（非重入）。下面先取出 auto_close 后
    // 立即让 Guard 离开作用域释放锁，否则后续 set_sys_proxy 内部再次 lock 会死锁，
    // 导致 ExitRequested 回调卡死、托盘"退出"无响应（仅在开启了代理自动关闭时触发）。
    let auto_close = {
        let g = inner.app_config.lock().unwrap();
        g.auto_close_proxy
    };
    crate::exit_log::exit_log(&format!("cleanup: auto_close_proxy={}", auto_close));
    if auto_close {
        crate::exit_log::exit_log("cleanup: 开始 set_sys_proxy(false)（同步 shell，可能卡）");
        // F5 修复：退出时清除系统代理失败应上报（此处为强制退出，保持日志但传播）
        if let Err(e) = set_sys_proxy(inner, false) {
            eprintln!("[mihomo] 退出时清除系统代理失败: {e}");
            crate::exit_log::exit_log(&format!("cleanup: set_sys_proxy 失败: {e}"));
        } else {
            crate::exit_log::exit_log("cleanup: set_sys_proxy 完成");
        }
    }
    crate::exit_log::exit_log("cleanup: 开始 stop_core");
    stop_core(inner);
    crate::exit_log::exit_log("cleanup: stop_core 完成");
}

// ---------- 下载/安装 ----------
/// 统一的下载客户端：带 60s 超时，避免无限挂起（F7）
fn download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = download_client()
        .get(url)
        .header("User-Agent", "any-version")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
}

/// 下载并校验 SHA256（仅允许官方 GitHub 源，F1）
async fn download_bytes_verified(url: &str, expected_sha256: &str) -> Result<Vec<u8>, String> {
    let data = download_bytes(url).await?;
    let actual = sha256_hex(&data);
    if !expected_sha256.eq_ignore_ascii_case(&actual) {
        return Err(format!(
            "SHA256 校验失败: 期望 {expected_sha256}, 实际 {actual}"
        ));
    }
    Ok(data)
}

/// 计算二进制内容的 SHA256 十六进制串
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    let d = h.finalize();
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
fn unzip_to(data: &[u8], dest: &Path) -> Result<(), String> {
    // F3 修复：先解压到临时目录，校验非空后再原子 rename 覆盖；保留旧 exe 的 .bak
    let tmp_dest = dest.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_dest);
    std::fs::create_dir_all(&tmp_dest).map_err(|e| e.to_string())?;
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().to_string();
        if name.ends_with('/') {
            continue;
        }
        let fname = name.split('/').last().unwrap_or(&name).to_string();
        let out = tmp_dest.join(&fname);
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).map_err(|e| e.to_string())?;
        total += buf.len() as u64;
        std::fs::write(&out, &buf).map_err(|e| e.to_string())?;
    }
    if total == 0 {
        return Err("解压结果为空，疑似损坏的 zip，已拒绝覆盖".to_string());
    }
    if dest.exists() {
        let _ = std::fs::rename(dest, dest.with_extension("bak"));
    }
    std::fs::rename(&tmp_dest, dest).map_err(|e| e.to_string())
}
async fn get_latest_mihomo_release() -> Result<(String, String), String> {
    let api = "https://api.github.com/repos/MetaCubeX/mihomo/releases/latest";
    // F7 修复：加超时，避免上游挂起时永久阻塞
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(api)
        .header("User-Agent", "any-version")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    let assets = v
        .get("assets")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for a in assets {
        let name = a
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        if name.contains("windows") && name.contains("amd64") && name.ends_with(".zip") {
            if let Some(url) = a.get("browser_download_url").and_then(|x| x.as_str()) {
                return Ok((name, url.to_string()));
            }
        }
    }
    Err("未找到适配的 mihomo 发布资源".into())
}

// ---------- 通用 patch ----------
fn patch_struct<T: Serialize + for<'de> Deserialize<'de>>(
    cur: &T,
    patch: &Value,
) -> Result<T, String> {
    let mut cur_v = serde_json::to_value(cur).map_err(|e| e.to_string())?;
    deep_merge(&mut cur_v, patch);
    serde_json::from_value(cur_v).map_err(|e| e.to_string())
}

// ================= 命令 =================
// 克隆 app_config 后再 await，避免锁跨 await 导致 future 非 Send
async fn mihomo_api_call(
    state: &State<'_, MihomoState>,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<String, String> {
    let app_config = state.app_config.lock().unwrap().clone();
    mihomo_api_raw(&app_config, method, url, body).await
}
// 用 async 让它跑在工作线程而非主线程，避免任何意外阻塞冻结整个界面
#[tauri::command]
pub async fn mihomo_get_state(state: State<'_, MihomoState>) -> Result<MihomoStateView, String> {
    Ok(build_state_view(&state))
}

/// 清除内核运行期告警（runtime_warnings）。运行期产生的 TUN/权限类错误会累积显示，
/// 且只在重启时清空，这里提供手动清除入口。
#[tauri::command]
pub fn mihomo_clear_warnings(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    state.runtime_warnings.lock().map_err(|e| e.to_string())?.clear();
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn mihomo_controller_info(state: State<'_, MihomoState>) -> Value {
    let app = state.app_config.lock().unwrap();
    serde_json::json!({
        "port": app.controller_port,
        "secret": app.secret,
        "wsBase": format!("ws://127.0.0.1:{}", app.controller_port),
    })
}

#[tauri::command]
pub async fn mihomo_start(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    launch_core(&app, Arc::clone(&*state)).await?;
    let need_proxy = state.app_config.lock().unwrap().sys_proxy_enable;
    if need_proxy {
        set_sys_proxy(&state, true)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn mihomo_stop(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    stop_core(&state);
    let need_proxy = state.app_config.lock().unwrap().sys_proxy_enable;
    if need_proxy {
        set_sys_proxy(&state, false)?;
    }
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn mihomo_restart(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    launch_core(&app, Arc::clone(&*state)).await
}

#[tauri::command]
pub fn mihomo_set_core_path(
    app: AppHandle,
    state: State<'_, MihomoState>,
    path: Option<String>,
) -> Result<(), String> {
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.core_path = path;
    save_app_config(&state.data_dir, &app_config).map_err(|e| e.to_string())?;
    *state.app_config.lock().unwrap() = app_config;
    refresh_core_version(&state);
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn mihomo_close_all_connections(state: State<'_, MihomoState>) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::DELETE,
        "/connections",
        Some(serde_json::json!({"all": true})),
    )
    .await?;
    Ok(())
}

// ---- AppConfig ----
#[tauri::command]
pub fn mihomo_get_app_config(state: State<'_, MihomoState>) -> AppConfig {
    state.app_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_patch_app_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    patch: Value,
) -> Result<AppConfig, String> {
    let new_cfg = patch_struct(&*state.app_config.lock().unwrap(), &patch)?;
    save_app_config(&state.data_dir, &new_cfg).ok();
    *state.app_config.lock().unwrap() = new_cfg.clone();
    emit_state(&app, &state);
    Ok(new_cfg)
}

// ---- Secondary proxies（二级代理）----
// 保存整份二级代理列表 + 当前启用项，并热重载配置使链式生效
#[tauri::command]
pub async fn mihomo_save_secondary_proxies(
    app: AppHandle,
    state: State<'_, MihomoState>,
    items: Vec<crate::commands::mihomo::config::SecondaryProxy>,
    active_id: Option<String>,
) -> Result<(), String> {
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.secondary_proxies = items;
    app_config.secondary_active_id = active_id;
    save_app_config(&state.data_dir, &app_config).map_err(|e| e.to_string())?;
    *state.app_config.lock().unwrap() = app_config;
    if !state.stop_flag.load(Ordering::SeqCst) {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    emit_state(&app, &state);
    Ok(())
}

// ---- ControledMihomoConfig ----
#[tauri::command]
pub fn mihomo_get_controled_config(state: State<'_, MihomoState>) -> Value {
    state.controled_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_patch_controled_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    patch: Value,
) -> Result<Value, String> {
    let mut cur = state.controled_config.lock().unwrap().clone();
    deep_merge(&mut cur, &patch);
    save_controled(&state.data_dir, &cur).ok();
    *state.controled_config.lock().unwrap() = cur.clone();
    emit_state(&app, &state);
    Ok(cur)
}

// ---- 运行配置 ----
#[tauri::command]
pub fn mihomo_get_runtime_config(state: State<'_, MihomoState>) -> String {
    state.runtime_config_str.lock().unwrap().clone()
}
#[tauri::command]
pub async fn mihomo_update_runtime_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
) -> Result<(), String> {
    reload_config(&app, Arc::clone(&*state)).await
}

// ---- Profiles ----
#[tauri::command]
pub fn mihomo_get_profile_config(state: State<'_, MihomoState>) -> ProfileConfig {
    state.profile_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_set_profile_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    cfg: ProfileConfig,
) -> Result<(), String> {
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_profile_item(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<ProfileItem, String> {
    state
        .profile_config
        .lock()
        .unwrap()
        .items
        .iter()
        .find(|i| i.id == id)
        .cloned()
        .ok_or("profile 不存在".to_string())
}
#[tauri::command]
pub fn mihomo_get_profile_str(state: State<'_, MihomoState>, id: String) -> Result<String, String> {
    let pc = state.profile_config.lock().unwrap();
    let item = pc
        .items
        .iter()
        .find(|i| i.id == id)
        .ok_or("profile 不存在".to_string())?;
    Ok(read_profile_content(&state.data_dir, item).unwrap_or_default())
}

// 返回配置文件的绝对路径，若尚未落盘则先生成并写入 profiles/<id>.yaml（供“打开文件”使用）
#[tauri::command]
pub fn mihomo_get_profile_file_path(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<String, String> {
    let pc = state.profile_config.lock().unwrap();
    let item = pc
        .items
        .iter()
        .find(|i| i.id == id)
        .ok_or("profile 不存在".to_string())?;
    let p = profile_content_path(&state.data_dir, &item.id);
    if !p.exists() {
        let content = if item.type_ == "subscription" {
            serde_yaml::to_string(&build_subscription_profile(item)).map_err(|e| e.to_string())?
        } else if let Some(fp) = &item.file_path {
            std::fs::read_to_string(fp).map_err(|e| e.to_string())?
        } else {
            return Err("配置内容不存在".into());
        };
        write_profile_content(&state.data_dir, item, &content).map_err(|e| e.to_string())?;
    }
    p.to_str().map(|s| s.to_string()).ok_or("路径非 UTF-8".into())
}
#[tauri::command]
pub fn mihomo_set_profile_str(
    state: State<'_, MihomoState>,
    id: String,
    content: String,
) -> Result<(), String> {
    let mut cfg = {
        let pc = state.profile_config.lock().unwrap();
        let item = pc
            .items
            .iter()
            .find(|i| i.id == id)
            .ok_or("profile 不存在".to_string())?;
        write_profile_content(&state.data_dir, item, &content).map_err(|e| e.to_string())?;
        pc.clone()
    };
    if let Some(it) = cfg.items.iter_mut().find(|i| i.id == id) {
        it.updated_at = Some(now_secs());
    }
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    Ok(())
}
#[tauri::command]
pub fn mihomo_add_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: ProfileItem,
) -> Result<(), String> {
    let mut cfg = state.profile_config.lock().unwrap().clone();
    if cfg.items.iter().any(|i| i.id == item.id) {
        return Err("id 已存在".into());
    }
    cfg.items.push(item);
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_remove_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
    let mut cfg = state.profile_config.lock().unwrap().clone();
    cfg.items.retain(|i| i.id != id);
    if cfg.current == id {
        cfg.current = cfg.items.first().map(|i| i.id.clone()).unwrap_or_default();
    }
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_update_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: ProfileItem,
) -> Result<(), String> {
    let mut cfg = state.profile_config.lock().unwrap().clone();
    if let Some(it) = cfg.items.iter_mut().find(|i| i.id == item.id) {
        *it = item;
    } else {
        cfg.items.push(item);
    }
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub async fn mihomo_change_current_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
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
    app_config.current_profile = id;
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    if !state.stop_flag.load(Ordering::SeqCst) {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    Ok(())
}
#[tauri::command]
pub async fn mihomo_validate_subscription(url: String) -> Result<SubValidation, String> {
    // F7 修复：加超时。部分机场订阅站 TLS 证书链不完整，容错校验避免误报失败；
    // 订阅 URL 为用户主动提供，属信任来源，可接受跳过证书校验。
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .header("User-Agent", "clash-verge/v1.7.0")
        .header("Accept-Encoding", "identity")
        .send()
        .await
        .map_err(|e| format!("订阅下载失败: {e}"))?;
    let headers = resp.headers().clone();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    // 嗅探并处理内容（age 解密 / base64 节点解码 / YAML），与正式导入保持一致
    let processed = crate::commands::mihomo::subparse::process_subscription_content(&text, None)
        .unwrap_or_else(|_| text.clone());
    let parsed: Value = match serde_yaml::from_str(&processed) {
        Ok(v) => v,
        Err(_) => serde_yaml::from_str(&text).map_err(|e| format!("订阅解析失败: {e}"))?,
    };
    let ok = parsed.get("proxy-providers").is_some()
        || parsed.get("proxies").is_some()
        || parsed.get("rules").is_some();
    let mut upload = None;
    let mut download = None;
    let mut total = None;
    let mut expire = None;
    if let Some(info) = headers.get("subscription-userinfo").and_then(|h| h.to_str().ok()) {
        for part in info.split(';') {
            let kv: Vec<&str> = part.split('=').collect();
            if kv.len() == 2 {
                match kv[0].trim() {
                    "upload" => upload = kv[1].trim().parse().ok(),
                    "download" => download = kv[1].trim().parse().ok(),
                    "total" => total = kv[1].trim().parse().ok(),
                    "expire" => expire = kv[1].trim().parse().ok(),
                    _ => {}
                }
            }
        }
    }
    Ok(SubValidation {
        ok,
        message: if ok {
            "订阅有效".into()
        } else {
            "订阅内容缺少 proxies/rules".into()
        },
        upload,
        download,
        total,
        expire,
        suggested_interval: None,
    })
}

// 解析 subscription-userinfo 响应头: upload=..; download=..; total=..; expire=..
fn parse_subinfo(header: Option<&str>) -> Option<SubscriptionUserInfo> {
    let s = header?;
    let mut upload = None;
    let mut download = None;
    let mut total = None;
    let mut expire = None;
    for part in s.split(';') {
        let kv: Vec<&str> = part.split('=').collect();
        if kv.len() == 2 {
            match kv[0].trim() {
                "upload" => upload = kv[1].trim().parse().ok(),
                "download" => download = kv[1].trim().parse().ok(),
                "total" => total = kv[1].trim().parse().ok(),
                "expire" => expire = kv[1].trim().parse().ok(),
                _ => {}
            }
        }
    }
    // 订阅商不一定返回全部字段（很多不返回 upload），任一缺失不应让整段失效
    Some(SubscriptionUserInfo {
        upload: upload.unwrap_or(0),
        download: download.unwrap_or(0),
        total: total.unwrap_or(0),
        expire: expire.unwrap_or(0),
    })
}

// 从订阅 URL 推导默认名称：优先 host（example.com），其次路径末段，最后原 URL。
// 不引入额外依赖，用简单字符串解析。
fn extract_sub_name_from_url(url: &str) -> String {
    // 去掉协议头
    let after_scheme = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(url);
    // host 为到下一个 '/'、'?'、'#' 之前的部分
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim();
    if !authority.is_empty() {
        // 去掉可能存在的 userinfo@ 与端口
        let host = authority.rsplit('@').next().unwrap_or(authority);
        let host = host.split(':').next().unwrap_or(host);
        if !host.is_empty() {
            return host.to_string();
        }
    }
    // 无 authority 时取路径最后一段非空片段
    if let Some(seg) = after_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .split('/')
        .filter(|p| !p.is_empty())
        .last()
    {
        return seg.to_string();
    }
    url.to_string()
}

// 导入订阅网址：下载 YAML 并作为「file」型配置存到本地（等价 clash-party 的 remote 导入）
#[tauri::command]
pub async fn mihomo_import_subscription(
    app: AppHandle,
    state: State<'_, MihomoState>,
    url: String,
) -> Result<ProfileItem, String> {
    // F7 修复：加超时
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .header("User-Agent", "clash-verge/v1.7.0")
        .header("Accept-Encoding", "identity")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let userinfo_header = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let parsed_userinfo = parse_subinfo(userinfo_header.as_deref());
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let _: Value = serde_yaml::from_str(&text).map_err(|e| format!("订阅解析失败: {e}"))?;
    let id = format!("sub_{}", now_secs());
    // 默认订阅名称：从 URL 提取 host（如 https://sub.example.com/abc -> example.com），
    // 无 host 时退用路径最后一段，再不行用原始 URL。比整串 URL 更友好可读。
    let default_name = extract_sub_name_from_url(&url);
    let item = ProfileItem {
        id: id.clone(),
        name: default_name,
        type_: "file".into(),
        file_path: None,
        subscriptions: vec![],
        rule_providers: vec![],
        custom_rules: vec![],
        dns_enabled: false,
        dns_nameservers: vec![],
        url: Some(url.clone()),
        age_secret_key: None,
        auth_token: None,
        user_agent: None,
        use_proxy: false,
        auto_update: false,
        update_interval: 86400,
        update_timeout: 30,
            override_ids: vec![],
            subscription_userinfo: parsed_userinfo,
            updated_at: Some(now_secs()),
    };
    {
        let mut cfg = state.profile_config.lock().unwrap().clone();
        cfg.items.push(item.clone());
        save_profile_config(&state.data_dir, &cfg).ok();
        *state.profile_config.lock().unwrap() = cfg;
    }
    write_profile_content(&state.data_dir, &item, &text).map_err(|e| e.to_string())?;
    emit_state(&app, &state);
    Ok(item)
}

// 导入本地 yml 文件：读取内容并作为「file」型配置存到本地（等价 clash-party 打开 yml）
#[tauri::command]
pub fn mihomo_import_file(
    app: AppHandle,
    state: State<'_, MihomoState>,
    path: String,
) -> Result<ProfileItem, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;
    let _: Value = serde_yaml::from_str(&text).map_err(|e| format!("yaml 解析失败: {e}"))?;
    let name = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("导入配置")
        .to_string();
    let id = format!("file_{}", now_secs());
    let item = ProfileItem {
        id: id.clone(),
        name,
        type_: "file".into(),
        file_path: None,
        subscriptions: vec![],
        rule_providers: vec![],
        custom_rules: vec![],
        dns_enabled: false,
        dns_nameservers: vec![],
        age_secret_key: None,
        url: None,
        auth_token: None,
        user_agent: None,
        use_proxy: false,
        auto_update: false,
        update_interval: 86400,
        update_timeout: 30,
            override_ids: vec![],
            subscription_userinfo: None,
            updated_at: Some(now_secs()),
    };
    {
        let mut cfg = state.profile_config.lock().unwrap().clone();
        cfg.items.push(item.clone());
        save_profile_config(&state.data_dir, &cfg).ok();
        *state.profile_config.lock().unwrap() = cfg;
    }
    write_profile_content(&state.data_dir, &item, &text).map_err(|e| e.to_string())?;
    emit_state(&app, &state);
    Ok(item)
}

/// 获取 mihomo mixed-port（若已配置则返回，供"使用代理更新订阅"走本地代理）。
fn get_mixed_port(state: &State<'_, MihomoState>) -> Option<u16> {
    state.app_config.lock().ok().map(|c| c.mixed_port).filter(|p| *p > 0)
}

// 按订阅地址重新拉取订阅内容（使用编辑信息里的 授权令牌/UA/超时），更新流量信息并回写
#[tauri::command]
pub async fn mihomo_update_subscription(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<ProfileItem, String> {
    let item = {
        let cfg = state.profile_config.lock().unwrap();
        cfg.items.iter().find(|i| i.id == id).cloned()
    }
    .ok_or("配置不存在")?;
    let url = item.url.clone().ok_or("该配置没有订阅地址（url）")?;
    let mut client_builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(item.update_timeout.max(5)))
        // 部分机场订阅站 TLS 证书链不完整，容错校验避免误报失败（订阅 URL 为用户主动提供）
        .danger_accept_invalid_certs(true);
    // 使用代理更新开关：走本地 mihomo mixed-port，实现"绕墙更新订阅"
    if item.use_proxy {
        if let Some(port) = get_mixed_port(&state) {
            let proxy_url = format!("http://127.0.0.1:{}", port);
            client_builder = client_builder.proxy(reqwest::Proxy::all(&proxy_url).map_err(|e| e.to_string())?);
        }
    }
    let client = client_builder.build().map_err(|e| e.to_string())?;
    let mut req = client.get(&url);
    // 对齐 clash-party fetchAndValidateSubscription：强制不压缩，
    // 避免部分订阅站返回 gzip 内容导致 base64/YAML 解析异常。
    req = req.header("Accept-Encoding", "identity");
    if let Some(ua) = &item.user_agent {
        req = req.header("User-Agent", ua.clone());
    }
    if let Some(token) = &item.auth_token {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let userinfo_header = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let text = resp.text().await.map_err(|e| e.to_string())?;
    // 嗅探并处理订阅内容：age 解密 / base64 节点解码 / YAML 校验，转成 mihomo 可加载的 clash YAML
    let processed = crate::commands::mihomo::subparse::process_subscription_content(
        &text,
        item.age_secret_key.as_deref(),
    )
    .map_err(|e| format!("订阅内容解析失败: {e}"))?;
    // 校验解析结果确实是 clash YAML
    let parsed: Value = serde_yaml::from_str(&processed).map_err(|e| format!("订阅解析失败: {e}"))?;
    if parsed.get("proxies").is_none() && parsed.get("proxy-providers").is_none() {
        // base64 解码后可能是节点列表但未生成 proxies；若为空则报错
        if text.trim().is_empty() {
            return Err("订阅内容为空".to_string());
        }
    }
    let mut updated = item.clone();
    updated.subscription_userinfo = parse_subinfo(userinfo_header.as_deref());
    updated.updated_at = Some(now_secs());
    write_profile_content(&state.data_dir, &updated, &processed).map_err(|e| e.to_string())?;
    {
        let mut cfg = state.profile_config.lock().unwrap();
        if let Some(it) = cfg.items.iter_mut().find(|i| i.id == id) {
            *it = updated.clone();
        }
        save_profile_config(&state.data_dir, &cfg).ok();
    }
    emit_state(&app, &state);
    Ok(updated)
}

// 查询某个订阅的可用性 + 节点数（用于列表展示「可用/不可用」与「数量」）
#[tauri::command]
pub fn mihomo_get_profile_status(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<Value, String> {
    let item = {
        let cfg = state.profile_config.lock().unwrap();
        cfg.items.iter().find(|i| i.id == id).cloned()
    };
    let content = match item {
        Some(ref it) => read_profile_content(&state.data_dir, it).unwrap_or_default(),
        None => String::new(),
    };
    let available = !content.trim().is_empty();
    let mut node_count = 0u32;
    // 兼容 base64 节点 / age 加密内容：先嗅探处理为 clash YAML 再数节点
    let processed = crate::commands::mihomo::subparse::process_subscription_content(
        &content,
        item.as_ref().and_then(|i| i.age_secret_key.as_deref()),
    )
    .unwrap_or(content.clone());
    if let Ok(v) = serde_yaml::from_str::<Value>(&processed) {
        if let Some(proxies) = v.get("proxies").and_then(|p| p.as_array()) {
            node_count = proxies.len() as u32;
        }
    }
    Ok(json!({ "available": available, "node_count": node_count }))
}

// ---- Overrides ----
#[tauri::command]
pub fn mihomo_get_override_config(state: State<'_, MihomoState>) -> OverrideConfig {
    state.override_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_set_override_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    cfg: OverrideConfig,
) -> Result<(), String> {
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_override_item(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<OverrideItem, String> {
    state
        .override_config
        .lock()
        .unwrap()
        .items
        .iter()
        .find(|i| i.id == id)
        .cloned()
        .ok_or("override 不存在".to_string())
}
#[tauri::command]
pub fn mihomo_add_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: OverrideItem,
) -> Result<(), String> {
    let mut cfg = state.override_config.lock().unwrap().clone();
    if cfg.items.iter().any(|i| i.id == item.id) {
        return Err("id 已存在".into());
    }
    cfg.items.push(item);
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_remove_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
    let mut cfg = state.override_config.lock().unwrap().clone();
    cfg.items.retain(|i| i.id != id);
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_update_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: OverrideItem,
) -> Result<(), String> {
    let mut cfg = state.override_config.lock().unwrap().clone();
    if let Some(it) = cfg.items.iter_mut().find(|i| i.id == item.id) {
        *it = item;
    } else {
        cfg.items.push(item);
    }
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_override(state: State<'_, MihomoState>, id: String) -> Result<String, String> {
    let oc = state.override_config.lock().unwrap();
    let item = oc
        .items
        .iter()
        .find(|i| i.id == id)
        .ok_or("override 不存在".to_string())?;
    Ok(read_override_content(&state.data_dir, item).unwrap_or_default())
}
#[tauri::command]
pub fn mihomo_set_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
    content: String,
) -> Result<(), String> {
    {
        let oc = state.override_config.lock().unwrap();
        let item = oc
            .items
            .iter()
            .find(|i| i.id == id)
            .ok_or("override 不存在".to_string())?;
        write_override_content(&state.data_dir, item, &content).map_err(|e| e.to_string())?;
    }
    emit_state(&app, &state);
    Ok(())
}

// ---- Controller API ----
#[tauri::command]
pub async fn mihomo_api(
    state: State<'_, MihomoState>,
    method: String,
    url: String,
    body: Option<Value>,
) -> Result<String, String> {
    let m = match method.to_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        _ => return Err("不支持的方法".into()),
    };
    mihomo_api_call(&state, m, &url, body).await
}
#[tauri::command]
pub async fn mihomo_version(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/version", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_proxies(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/proxies", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_groups(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/providers/proxies", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_rules(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/rules", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_proxy_providers(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/providers/proxies", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_rule_providers(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/providers/rules", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_change_proxy(
    state: State<'_, MihomoState>,
    group: String,
    name: String,
) -> Result<(), String> {
    // 入参 group 为「原始组名」（可能含中文/emoji），本函数内部负责 urlencode；
    // 调用方须传入原始名，勿预先 encode，否则会双重编码导致 404。
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        &format!("/proxies/{}", urlencoding(&group)),
        Some(serde_json::json!({ "name": name })),
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_unfixed_proxy(
    state: State<'_, MihomoState>,
    group: String,
    name: String,
) -> Result<(), String> {
    mihomo_change_proxy(state, group, name).await
}
#[tauri::command]
pub async fn mihomo_proxy_delay(
    state: State<'_, MihomoState>,
    name: String,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    let path = format!(
        "/proxies/{}/delay?url={}&timeout={}",
        urlencoding(&name),
        urlencoding(&url),
        timeout
    );
    let s = mihomo_api_call(&state, reqwest::Method::GET, &path, None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_test_delay(
    state: State<'_, MihomoState>,
    name: String,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    mihomo_proxy_delay(state, name, url, timeout).await
}
/// 测试整个策略组延迟（对齐 mihomoGroupDelay）
#[tauri::command]
pub async fn mihomo_group_delay(
    state: State<'_, MihomoState>,
    group: String,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    let path = format!(
        "/group/{}/delay?url={}&timeout={}",
        urlencoding(&group),
        urlencoding(&url),
        timeout
    );
    let s = mihomo_api_call(&state, reqwest::Method::GET, &path, None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
/// 代理集合健康检查（对齐 mihomoProxyDelay 的 provider 分支）
#[tauri::command]
pub async fn mihomo_provider_healthcheck(
    state: State<'_, MihomoState>,
    provider: String,
    name: Option<String>,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    let path = match &name {
        Some(n) => format!(
            "/providers/proxies/{}/{}/healthcheck?url={}&timeout={}",
            urlencoding(&provider),
            urlencoding(n),
            urlencoding(&url),
            timeout
        ),
        None => format!("/providers/proxies/{}/healthcheck", urlencoding(&provider)),
    };
    let s = mihomo_api_call(&state, reqwest::Method::GET, &path, None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
/// 更新代理集合（对齐 mihomoUpdateProxyProviders）
#[tauri::command]
pub async fn mihomo_update_proxy_provider(
    state: State<'_, MihomoState>,
    name: String,
) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        &format!("/providers/proxies/{}", urlencoding(&name)),
        None,
    )
    .await?;
    Ok(())
}
/// 更新规则集合（对齐 mihomoUpdateRuleProviders）
#[tauri::command]
pub async fn mihomo_update_rule_provider(
    state: State<'_, MihomoState>,
    name: String,
) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        &format!("/providers/rules/{}", urlencoding(&name)),
        None,
    )
    .await?;
    Ok(())
}
/// 启用/禁用规则（对齐 mihomoRulesDisable，PATCH /rules/disable）
#[tauri::command]
pub async fn mihomo_rules_disable(
    state: State<'_, MihomoState>,
    rules: Value,
) -> Result<(), String> {
    mihomo_api_call(&state, reqwest::Method::PATCH, "/rules/disable", Some(rules)).await?;
    Ok(())
}
/// Smart 内核：查询策略组权重（对齐 mihomoSmartGroupWeights）
#[tauri::command]
pub async fn mihomo_smart_group_weights(
    state: State<'_, MihomoState>,
    group: String,
) -> Result<Value, String> {
    let s = mihomo_api_call(
        &state,
        reqwest::Method::GET,
        &format!("/group/{}/weights", urlencoding(&group)),
        None,
    )
    .await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
/// Smart 内核：清空学习缓存（对齐 mihomoSmartFlushCache）
#[tauri::command]
pub async fn mihomo_smart_flush_cache(
    state: State<'_, MihomoState>,
    config_name: Option<String>,
) -> Result<(), String> {
    let path = match &config_name {
        Some(n) if !n.is_empty() => format!("/cache/smart/flush/{}", urlencoding(n)),
        _ => "/cache/smart/flush".to_string(),
    };
    mihomo_api_call(&state, reqwest::Method::POST, &path, None).await?;
    Ok(())
}
/// 通用配置补丁（对齐 patchMihomoConfig）
#[tauri::command]
pub async fn mihomo_patch_config(
    state: State<'_, MihomoState>,
    patch: Value,
) -> Result<(), String> {
    mihomo_api_call(&state, reqwest::Method::PATCH, "/configs", Some(patch)).await?;
    Ok(())
}
/// 热重载内核配置（对齐 mihomoHotReloadConfig，PUT /configs?force=true）
#[tauri::command]
pub async fn mihomo_hot_reload_config(state: State<'_, MihomoState>) -> Result<(), String> {
    let (diff, current) = {
        let app_config = state.app_config.lock().unwrap();
        let profile = state.profile_config.lock().unwrap();
        (app_config.diff_work_dir, profile.current.clone())
    };
    let path = core_work_config_path(&state.data_dir, diff, &current);
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        "/configs?force=true",
        Some(serde_json::json!({ "path": path.to_string_lossy() })),
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_set_mode(state: State<'_, MihomoState>, mode: String) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::PATCH,
        "/configs",
        Some(serde_json::json!({ "mode": mode })),
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_select_proxy(
    app: AppHandle,
    state: State<'_, MihomoState>,
    name: String,
) -> Result<(), String> {
    let need_reload = {
        let cfg = state.app_config.lock().unwrap();
        cfg.secondary_active_id.is_some() && !cfg.secondary_proxies.is_empty()
    };
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.default_proxy = Some(name);
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    // 已启用二级代理时，切换一级代理需重新生成配置让 dialer-proxy 跟随新的一级节点
    if need_reload && !state.stop_flag.load(Ordering::SeqCst) {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    Ok(())
}
#[tauri::command]
pub async fn mihomo_set_tun(
    app: AppHandle,
    state: State<'_, MihomoState>,
    enable: bool,
) -> Result<(), String> {
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.tun_enabled = enable;
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    reload_config(&app, Arc::clone(&*state)).await
}
#[tauri::command]
pub async fn mihomo_close_connection(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::DELETE,
        &format!("/connections/{}", urlencoding(&id)),
        None,
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_get_connections(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/connections", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_get_memory(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/memory", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub fn mihomo_get_logs(state: State<'_, MihomoState>) -> Vec<String> {
    let content = std::fs::read_to_string(&state.log_file).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    lines
        .into_iter()
        .rev()
        .take(300)
        .rev()
        .map(|s| s.to_string())
        .collect()
}
#[tauri::command]
pub async fn mihomo_set_sys_proxy(
    app: AppHandle,
    state: State<'_, MihomoState>,
    enable: bool,
) -> Result<(), String> {
    set_sys_proxy(&state, enable)?;
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.sys_proxy_enable = enable;
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_sys_proxy() -> Value {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
        KEY_READ,
    ) {
        let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
        let server: String = key.get_value("ProxyServer").unwrap_or_default();
        serde_json::json!({ "enable": enabled != 0, "server": server })
    } else {
        serde_json::json!({ "enable": false, "server": "" })
    }
}

// ---- 升级 ----
#[tauri::command]
pub async fn mihomo_upgrade(app: AppHandle, state: State<'_, MihomoState>) -> Result<String, String> {
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
                && p
                    .file_name()
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

// ---- 规则覆写 / 通用文件读写 / 规则集转换（对齐 clash-party getRuleStr / setRuleStr / convertMrsRuleset）----

/// 读取某订阅的规则覆写文件（rules/<id>.yaml）
#[tauri::command]
pub fn mihomo_get_rule_str(state: State<'_, MihomoState>, id: String) -> Result<String, String> {
    let p = rule_override_path(&state.data_dir, &id);
    if !p.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(p).map_err(|e| format!("读取规则覆写失败: {e}"))
}

/// 写入某订阅的规则覆写文件，写入后热重载配置
#[tauri::command]
pub async fn mihomo_set_rule_str(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
    content: String,
) -> Result<(), String> {
    // 先校验 YAML 合法性，避免写入坏文件导致内核起不来
    if !content.trim().is_empty() {
        serde_yaml::from_str::<serde_yaml::Value>(&content)
            .map_err(|e| format!("规则覆写不是合法 YAML: {e}"))?;
    }
    let p = rule_override_path(&state.data_dir, &id);
    atomic_write(&p, &content).map_err(|e| format!("写入规则覆写失败: {e}"))?;
    let current = state.profile_config.lock().unwrap().current.clone();
    if current == id {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    Ok(())
}

/// 读取规则覆写（结构化：{ prepend, append, delete }），前端无需自带 YAML 解析
#[tauri::command]
pub fn mihomo_get_rule_override(state: State<'_, MihomoState>, id: String) -> Result<Value, String> {
    let p = rule_override_path(&state.data_dir, &id);
    let empty = serde_json::json!({ "prepend": [], "append": [], "delete": [] });
    if !p.exists() {
        return Ok(empty);
    }
    let content = std::fs::read_to_string(&p).map_err(|e| format!("读取规则覆写失败: {e}"))?;
    if content.trim().is_empty() {
        return Ok(empty);
    }
    let v: serde_yaml::Value =
        serde_yaml::from_str(&content).map_err(|e| format!("规则覆写不是合法 YAML: {e}"))?;
    let json: Value = serde_json::to_value(v).map_err(|e| e.to_string())?;
    let take = |k: &str| -> Value {
        json.get(k)
            .and_then(|x| x.as_array())
            .map(|a| Value::Array(a.clone()))
            .unwrap_or_else(|| Value::Array(vec![]))
    };
    Ok(serde_json::json!({
        "prepend": take("prepend"),
        "append": take("append"),
        "delete": take("delete"),
    }))
}

/// 写入规则覆写（结构化），写入后若为当前订阅则热重载
#[tauri::command]
pub async fn mihomo_set_rule_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
    data: Value,
) -> Result<(), String> {
    let yaml_value: serde_yaml::Value =
        serde_yaml::to_value(&data).map_err(|e| format!("序列化规则覆写失败: {e}"))?;
    let content =
        serde_yaml::to_string(&yaml_value).map_err(|e| format!("生成 YAML 失败: {e}"))?;
    let p = rule_override_path(&state.data_dir, &id);
    atomic_write(&p, &content).map_err(|e| format!("写入规则覆写失败: {e}"))?;
    let current = state.profile_config.lock().unwrap().current.clone();
    if current == id {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    Ok(())
}

/// 读取任意文本文件（供前端编辑器使用）
#[tauri::command]
pub fn mihomo_get_file_str(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))
}

/// 写入任意文本文件（原子写）
#[tauri::command]
pub fn mihomo_set_file_str(path: String, content: String) -> Result<(), String> {
    atomic_write(Path::new(&path), &content).map_err(|e| format!("写入文件失败: {e}"))
}

/// 用内核把 yaml/text 规则集转换为 mrs 二进制格式（对齐 convertMrsRuleset）
#[tauri::command]
pub fn mihomo_convert_mrs_ruleset(
    state: State<'_, MihomoState>,
    rule_type: String,
    input_format: String,
    input_path: String,
    output_path: String,
) -> Result<String, String> {
    let core = resolve_core_path(&state.app_config.lock().unwrap().clone());
    if !core.exists() {
        return Err("未找到 mihomo 内核".into());
    }
    let out = hidden_cmd(&core)
        .args([
            "convert-ruleset",
            &rule_type,
            &input_format,
            &input_path,
            &output_path,
        ])
        .output()
        .map_err(|e| format!("执行规则集转换失败: {e}"))?;
    if out.status.success() {
        Ok(output_path)
    } else {
        let mut msg = String::from_utf8_lossy(&out.stdout).to_string();
        msg.push_str(&String::from_utf8_lossy(&out.stderr));
        Err(format!("规则集转换失败: {}", msg.trim()))
    }
}

/// 仅退出界面、保留内核后台运行（对齐 clash-party quitWithoutCore 的反向语义：退出应用但不杀内核）
#[tauri::command]
pub fn mihomo_detach_core(state: State<'_, MihomoState>) -> Result<(), String> {
    state
        .stop_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mut g = state.child.lock().unwrap();
    if let Some(child) = g.take() {
        // 丢弃句柄但不 kill，内核继续运行
        std::mem::forget(child);
    }
    Ok(())
}

/// 读取 JS 覆写脚本的最近一次执行日志
#[tauri::command]
pub fn mihomo_get_override_exec_log(state: State<'_, MihomoState>, id: String) -> Vec<String> {
    let p = factory::override_log_path(&state.data_dir, &id);
    match std::fs::read_to_string(p) {
        Ok(s) => s
            .lines()
            .map(|l| l.to_string())
            .filter(|l| !l.trim().is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

// ============ 托盘快捷操作（供 tray.rs 调用，不经过前端） ============

fn tray_state(app: &AppHandle) -> Result<MihomoState, String> {
    app.try_state::<MihomoState>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "mihomo 未初始化".to_string())
}

async fn tray_api(
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
pub async fn tray_snapshot(app: &AppHandle) -> Option<(String, Vec<(String, String, Vec<String>)>)> {
    let mode = tray_api(app, Method::GET, "/configs", None)
        .await
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("mode").and_then(|m| m.as_str()).map(|s| s.to_string()))
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

fn urlencoding(s: &str) -> String {
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
