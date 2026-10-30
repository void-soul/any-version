use super::*;

pub type MihomoState = Arc<MihomoInner>;

pub struct MihomoInner {
    pub app_config: Mutex<AppConfig>,
    pub controled_config: Mutex<Value>,
    pub profile_config: Mutex<ProfileConfig>,
    pub override_config: Mutex<OverrideConfig>,
    pub child: Mutex<Option<Child>>,
    /// 本次会话内是否由 any-version 拉起过核心（用于退出时按端口补杀，
    /// 避免误杀外部/上次会话遗留的 mihomo）
    pub launched_by_us: Arc<AtomicBool>,
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
pub(super) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(super) fn uid() -> String {
    format!("{:x}", now_secs() ^ (std::process::id() as u64))
}
/// 「当前这一分钟」的键：cron 粒度是分钟，调度器每 60s 一跳，
/// 同一分钟可能被扫到两次（启动即刷新 + 首跳），用它去重。
pub(super) fn minute_key() -> u64 {
    now_secs() / 60
}
/// 当前本地时间是否命中 cron（5 段：分 时 日 月 周）。
/// 未配置（空/None）返回 false —— 交给调用方继续按秒级间隔判断。
pub(super) fn cron_hit_now(expr: Option<&str>) -> bool {
    let Some(e) = expr.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let now = chrono::Local::now();
    crate::commands::mihomo::cron::cron_matches(
        Some(e),
        now.minute(),
        now.hour(),
        now.day(),
        now.month(),
        now.weekday().num_days_from_sunday(),
    )
}
pub(super) fn get_str<'a>(v: &'a Value, k: &str) -> Option<String> {
    v.get(k).and_then(|x| x.as_str()).map(|s| s.to_string())
}
pub(super) fn get_str_opt<'a>(v: &'a Value, k: &str) -> Option<String> {
    get_str(v, k)
}
pub(super) fn get_u64(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64())
}

pub(super) fn refresh_core_version(inner: &MihomoInner) {
    let core = {
        let cfg = inner.app_config.lock().unwrap();
        resolve_core_path(&cfg)
    };
    eprintln!(
        "[mihomo] refresh_core_version: core = {:?}, exists = {}",
        core,
        core.exists()
    );
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
            eprintln!(
                "[mihomo] -v 运行失败: {:?}，但核心文件存在，标记为已安装",
                e
            );
            *inner.core_version.lock().unwrap() = Some("已安装".to_string());
        }
    }
}

pub(super) fn build_state_view(inner: &MihomoInner) -> MihomoStateView {
    // 注意：结构体字面量中的临时 MutexGuard 会活到整条语句结束，
    // 对同一个 Mutex 重复 lock 会死锁，因此这里每个锁都只取一次并提前释放。
    // 锁顺序与 launch_core 保持一致（先 app_config 后 child），避免跨锁顺序反转。
    let (running, pid) = {
        let mixed_port = inner.app_config.lock().unwrap().mixed_port;
        let g = inner.child.lock().unwrap();
        let child_running = g.as_ref().map(|c| c.id() > 0).unwrap_or(false)
            && !inner.stop_flag.load(Ordering::SeqCst);
        // running 判断与 launch_core 幂等检测一致（child 或端口监听），
        // 避免外部/遗留进程占用端口时误判为未运行。
        let port_running = mixed_port > 0 && manager::port_in_use(mixed_port);
        (
            child_running || port_running,
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
