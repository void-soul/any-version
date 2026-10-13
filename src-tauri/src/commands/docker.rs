//! 服务模块的 Docker 运行方式：通用 `docker compose` 封装。
//!
//! 职责边界：`node_manager` 管「服务项定义 + 生命周期契约」，本模块只管
//! 「怎么把一份 compose 模板跑起来」。**这里不写死任何具体服务** —— 任何
//! `runtime = "docker"` 的服务条目都走这套逻辑，再接一个自托管服务只需再丢一份
//! `node-projects/*.json`，不用写代码。
//!
//! 为什么模板放在定义文件里、安装时渲染到数据目录：像 ARTEX 这样的项目只需要
//! 一份 compose，用户不必先 `git clone`（其仓库已声明为最后一版源码备份，原镜像源
//! 也已失效）。渲染时替换 `{{port}}` / `{{pg_password}}` / `{{dir}}`，之后可以改
//! （见 `docker_service_config` / `docker_save_service_config`）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::config::get_data_dir;
use crate::commands::hidden_cmd::{hidden_cmd, output_capped, DEFAULT_OUTPUT_CAP};

/// 本地配置文件（保存用户对模板变量的覆盖值，是「用户改过什么」的唯一真源）。
const CONFIG_FILE: &str = "kira.json";

// ─── 外部命令 ───

/// 一次探测命令的结果。
///
/// stdout/stderr 分开留着：`docker --version` 写 stdout，而 daemon 未启动时报错在
/// stderr —— 两边都要看，混在一起就分不清「没装」和「装了没起」。
#[derive(Debug, Clone, Default)]
pub struct CmdOut {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CmdOut {
    /// 失败时给用户看的一句话：优先 stderr（docker 的错误基本都在这里），
    /// 为空再退回 stdout，最后给一句兜底。
    pub fn err_text(&self) -> String {
        let e = self.stderr.trim();
        if !e.is_empty() {
            return e.to_string();
        }
        let o = self.stdout.trim();
        if !o.is_empty() {
            return o.to_string();
        }
        "命令未返回任何信息（可能未安装）".to_string()
    }
}

/// 跑一条探测命令并取回输出（带上限，避免个别命令输出无界撑爆内存）。
///
/// 命令本身起不来（未安装）不算异常：返回 `ok = false` 并把 io 错误放进 stderr，
/// 由调用方统一按「不可用」处理 —— 环境检测里这是最常见的一种状态，不该是 Err。
pub fn probe(program: &str, args: &[&str]) -> CmdOut {
    let mut cmd = hidden_cmd(program);
    cmd.args(args);
    match output_capped(cmd, DEFAULT_OUTPUT_CAP) {
        Ok(o) => CmdOut {
            ok: o.status.success(),
            stdout: o.stdout,
            stderr: o.stderr,
        },
        Err(e) => CmdOut {
            ok: false,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

// ─── 环境检测 ───

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DockerEnv {
    /// docker CLI 是否可用
    pub cli: bool,
    #[serde(default)]
    pub cli_version: String,
    /// `docker compose`（v2 插件）是否可用
    pub compose: bool,
    #[serde(default)]
    pub compose_version: String,
    /// 引擎是否在运行（`docker info` 成功）
    pub daemon_running: bool,
    /// 给前端环境条直接用的一句话结论
    pub summary: String,
    /// 原始错误（daemon 未启动时 docker 的原文），便于用户自行排查
    #[serde(default)]
    pub error: String,
}

/// 从 `Docker version 27.3.1, build 9d9e0dc` 里取出版本号。
///
/// 按「第一个以数字开头的词」取，而不是固定第 3 个词：不同发行版/构建的前缀
/// 词数会变（`Docker version …` / `Docker version …-rd, build …`），按位置取会漂。
pub fn parse_cli_version(out: &str) -> String {
    out.split_whitespace()
        .find(|t| t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .map(|t| t.trim_end_matches(',').to_string())
        .unwrap_or_default()
}

/// 从 `Docker Compose version v2.29.7` 里取出版本号（去掉 `v` 前缀）。
pub fn parse_compose_version(out: &str) -> String {
    out.split_whitespace()
        .map(|t| t.trim_start_matches('v').trim_end_matches(','))
        .find(|t| t.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or_default()
        .to_string()
}

/// 把三条探测命令的输出汇成一份环境结论（纯函数，便于单测覆盖各种组合）。
///
/// 分档刻意细：Windows 上最常见的不是「没装 Docker」而是「装了但 Docker Desktop
/// 没启动」，这两种必须给出不同的提示，否则用户只会看到一句笼统的失败。
pub fn env_from_outputs(cli: &CmdOut, compose: &CmdOut, info: &CmdOut) -> DockerEnv {
    let cli_version = if cli.ok {
        parse_cli_version(&cli.stdout)
    } else {
        String::new()
    };
    let compose_version = if compose.ok {
        parse_compose_version(&compose.stdout)
    } else {
        String::new()
    };
    let daemon_running = info.ok;

    let summary = if !cli.ok {
        "未检测到 Docker：请先安装 Docker Desktop".to_string()
    } else if !daemon_running {
        format!("Docker {} 已安装，但引擎没在运行：请启动 Docker Desktop", cli_version)
    } else if !compose.ok {
        format!("Docker {} 在运行，但缺少 compose 插件（需 v2）", cli_version)
    } else {
        format!("Docker {} · Compose {} 就绪", cli_version, compose_version)
    };

    DockerEnv {
        cli: cli.ok,
        cli_version,
        compose: compose.ok,
        compose_version,
        daemon_running,
        summary,
        error: if daemon_running || !cli.ok {
            String::new()
        } else {
            info.err_text()
        },
    }
}

/// 探测本机 Docker 环境。CLI 不可用时不再去问 daemon（纯浪费时间且错误更噪）。
pub fn check_env() -> DockerEnv {
    let cli = probe("docker", &["--version"]);
    let compose = probe("docker", &["compose", "version"]);
    let info = if cli.ok {
        probe("docker", &["info", "--format", "{{.ServerVersion}}"])
    } else {
        CmdOut::default()
    };
    env_from_outputs(&cli, &compose, &info)
}

/// 状态轮询用的带缓存版本（TTL 5 秒）。
///
/// 为什么需要：状态每 4 秒查一次，而**引擎没启动时** `docker info` 要等到连接超时
/// 才返回 —— 不缓存的话轮询会被这几秒的超时拖垮，界面跟着卡。
/// 安装/启动前的前置检查仍走 `check_env()` 取实时值：用户可能刚点开 Docker Desktop，
/// 拿 5 秒前的「没在运行」去拦他是错的。
pub fn check_env_cached() -> DockerEnv {
    use std::sync::Mutex;
    use std::time::Instant;
    static CACHE: Mutex<Option<(Instant, DockerEnv)>> = Mutex::new(None);
    const TTL: std::time::Duration = std::time::Duration::from_secs(5);

    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        // 锁中毒时直接实时探测，绝不因为缓存机制把功能弄坏
        Err(_) => return check_env(),
    };
    if let Some((at, cached)) = guard.as_ref() {
        if at.elapsed() < TTL {
            return cached.clone();
        }
    }
    let fresh = check_env();
    *guard = Some((Instant::now(), fresh.clone()));
    fresh
}

// ─── 模板与本地配置 ───

/// 用户对模板变量的覆盖（持久化在 `<dir>/kira.json`）。
///
/// 需要它是因为：端口与数据库密码既不在 compose 模板里写死、也不该每次重新生成 ——
/// 密码一变，已有的 postgres 数据卷就再也连不上了。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalConfig {
    /// 0 = 尚未设置，取定义里的 defaultPort
    #[serde(default)]
    pub port: u16,
    /// 第二个对外端口（部分服务要发布两个，如 ARTEX 的界面 + API）。
    /// 0 = 该服务不需要第二个端口
    #[serde(default)]
    pub port2: u16,
    #[serde(default)]
    pub pg_password: String,
}

/// 渲染一份 docker 服务所需的最小输入。
///
/// 由 `node_manager` 从 `NodeProjectDef` 投影而来 —— 这样本模块不必知道
/// `NodeProjectDef` 长什么样，也就不会因为它加字段而被动跟改。
pub struct ServiceSpec<'a> {
    pub id: &'a str,
    /// 定义里的默认端口（`LocalConfig.port` 为 0 时用它）
    pub port: u16,
    /// 第二个对外端口的默认值（`LocalConfig.port2` 为 0 时用它；0 = 不发布第二个端口）
    pub port2: u16,
    /// compose 项目名（`-p`），与用户自己的 compose 项目隔离
    pub project: &'a str,
    /// 需要持久化的挂载目录名（相对服务目录），安装时创建
    pub volumes: &'a [String],
    pub compose_tpl: &'a str,
    pub env_tpl: &'a str,
}

/// 服务的 docker 目录：`<data>/docker/<id>`。
///
/// 放数据目录而不是 node-projects 托管目录：compose 与数据卷都属于「运行态数据」，
/// 跟着数据目录走，用户换机器/迁盘时一份带走，且不会被「卸载服务」误删。
pub fn docker_dir(id: &str) -> PathBuf {
    get_data_dir().join("docker").join(id)
}

/// 替换模板占位符。
///
/// `{{dir}}` 统一转成正斜杠：compose 在 Windows 上也认正斜杠，而反斜杠在 YAML 里
/// 是转义符（`C:\Users` 会被解析器当成非法转义），直接写进去必然报错。
///
/// `{{port2}}` 是**可选的**第二个对外端口：模板里不写它就原样留着，不会被误替换。
/// 存在的理由是端口会撞车 —— ARTEX 的第二个端口原本写死 8788，而 Kira 自带的 twoapi
/// 代理默认就监听 8788，于是 `docker compose up` 必然失败在 "ports are not available"。
pub fn render_template(tpl: &str, port: u16, port2: u16, pg_password: &str, dir: &str) -> String {
    tpl.replace("{{port}}", &port.to_string())
        .replace("{{port2}}", &port2.to_string())
        .replace("{{pg_password}}", pg_password)
        .replace("{{dir}}", &dir.replace('\\', "/"))
}

/// 生成 32 位随机密码，**只用字母数字、且剔除易混字符**（0/O/1/l/I）。
///
/// 密码会同时出现在 `.env`、postgres DSN、以及用户可能手敲的 psql 命令里，
/// 带符号容易被各处的转义规则（YAML / shell / URL）吃掉，排错成本远高于多给几位。
pub fn generate_password() -> String {
    const CHARS: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 32];
    if getrandom::getrandom(&mut buf).is_err() {
        // 系统 RNG 不可用（极罕见）时退回时间戳派生值，绝不 panic：
        // 这里的密码只用于本机回环上的 postgres，强度够用，且失败也不该中断安装。
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut s = String::with_capacity(32);
        let mut x = nanos | 1;
        for _ in 0..32 {
            s.push(CHARS[(x % CHARS.len() as u128) as usize] as char);
            x /= 7;
        }
        return s;
    }
    buf.iter().map(|b| CHARS[(*b as usize) % CHARS.len()] as char).collect()
}

/// 给前端看的配置快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerServiceConfig {
    pub id: String,
    pub port: u16,
    /// 第二个对外端口；0 = 该服务只发布一个端口
    pub port2: u16,
    pub pg_password: String,
    pub dir: String,
    pub compose_path: String,
    pub env_path: String,
    /// 配置文件是否已生成（等价于「已安装」）
    pub prepared: bool,
}

fn to_view(spec: &ServiceSpec, dir: &Path, cfg: &LocalConfig) -> DockerServiceConfig {
    DockerServiceConfig {
        id: spec.id.to_string(),
        port: cfg.port,
        port2: cfg.port2,
        pg_password: cfg.pg_password.clone(),
        dir: dir.to_string_lossy().to_string(),
        compose_path: dir.join("docker-compose.yml").to_string_lossy().to_string(),
        env_path: dir.join(".env").to_string_lossy().to_string(),
        prepared: dir.join("docker-compose.yml").is_file(),
    }
}

/// 读本地配置；不存在就用定义默认值 + 新生成的密码（不落盘）。
pub fn load_config_in(dir: &Path, spec: &ServiceSpec) -> LocalConfig {
    let path = dir.join(CONFIG_FILE);
    let mut cfg = if path.is_file() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<LocalConfig>(&t).ok())
            .unwrap_or_default()
    } else {
        LocalConfig::default()
    };
    if cfg.port == 0 {
        cfg.port = spec.port;
    }
    // 第二个端口同理；但定义里没给（spec.port2 = 0）就保持 0，
    // 别把「不需要第二个端口」的服务凭空塞一个端口进去
    if cfg.port2 == 0 {
        cfg.port2 = spec.port2;
    }
    if cfg.pg_password.trim().is_empty() {
        cfg.pg_password = generate_password();
    }
    cfg
}

/// 写配置文件：compose 每次都按模板重渲（它是模板的产物），`.env` 与 `kira.json`
/// 记录用户改动。**只有 kira.json 是权威**，其余两个都可由它重建。
pub fn write_files_in(dir: &Path, spec: &ServiceSpec, cfg: &LocalConfig) -> Result<(), String> {
    let dir_str = dir.to_string_lossy().to_string();
    let compose = render_template(spec.compose_tpl, cfg.port, cfg.port2, &cfg.pg_password, &dir_str);
    let env = render_template(spec.env_tpl, cfg.port, cfg.port2, &cfg.pg_password, &dir_str);
    let compose_path = dir.join("docker-compose.yml");
    fs::write(&compose_path, compose)
        .map_err(|e| format!("写入 {} 失败: {}", compose_path.display(), e))?;
    let env_path = dir.join(".env");
    fs::write(&env_path, env).map_err(|e| format!("写入 {} 失败: {}", env_path.display(), e))?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let cfg_path = dir.join(CONFIG_FILE);
    fs::write(&cfg_path, json).map_err(|e| format!("写入 {} 失败: {}", cfg_path.display(), e))?;
    Ok(())
}

/// 生成（或校准）服务目录与配置文件，返回可给前端展示的快照。
///
/// 幂等：重复调用只重渲模板，不会重新生成密码（否则 postgres 卷会连不上）。
pub fn prepare_in(dir: &Path, spec: &ServiceSpec) -> Result<DockerServiceConfig, String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败: {}", dir.display(), e))?;
    for v in spec.volumes {
        let p = dir.join(v);
        fs::create_dir_all(&p).map_err(|e| format!("创建挂载目录 {} 失败: {}", p.display(), e))?;
    }
    let cfg = load_config_in(dir, spec);
    write_files_in(dir, spec, &cfg)?;
    Ok(to_view(spec, dir, &cfg))
}

/// 读当前配置快照（未安装时 `prepared = false`，但密码/端口用内存里的值，便于前端展示）。
pub fn read_config_in(dir: &Path, spec: &ServiceSpec) -> DockerServiceConfig {
    let cfg = load_config_in(dir, spec);
    to_view(spec, dir, &cfg)
}

/// 保存用户改的端口 / 数据库密码并重渲配置文件。
///
/// 改密码会让**已有数据卷连不上**（postgres 只在初始化时用该密码建库），所以前端
/// 必须提示「需要删掉数据卷重建」。这里不做静默兜底 —— 那不是后端该替用户决定的事。
pub fn save_config_in(
    dir: &Path,
    spec: &ServiceSpec,
    port: u16,
    port2: u16,
    pg_password: &str,
) -> Result<DockerServiceConfig, String> {
    if port < 1024 {
        return Err("端口需 ≥ 1024（低端口是系统保留区，且多半已被占用）".to_string());
    }
    // 第二个端口可选：不填（0）就沿用定义里的默认值；填了才校验
    if port2 != 0 && port2 < 1024 {
        return Err("第二个端口需 ≥ 1024（低端口是系统保留区，且多半已被占用）".to_string());
    }
    if port2 != 0 && port2 == port {
        return Err("两个端口不能相同".to_string());
    }
    let pg = pg_password.trim();
    if pg.len() < 8 {
        return Err("数据库密码至少 8 位".to_string());
    }
    let cfg = LocalConfig {
        port,
        // 填 0 时回落定义默认值，别把 0 写进 kira.json —— 那会让 compose 绑到随机端口
        port2: if port2 == 0 { spec.port2 } else { port2 },
        pg_password: pg.to_string(),
    };
    fs::create_dir_all(dir).map_err(|e| format!("创建 {} 失败: {}", dir.display(), e))?;
    write_files_in(dir, spec, &cfg)?;
    Ok(to_view(spec, dir, &cfg))
}

// ─── compose 调用与状态解析 ───

/// 组装 `docker compose …` 的公共参数。
///
/// 项目名 / 工作目录 / compose 文件三者都显式给：`docker compose` 默认按**当前目录**
/// 找 compose 文件、拿目录名当项目名 —— 在别的项目目录下执行就会操作到别人的容器，
/// 这类串台极难排查。
pub fn compose_args(project: &str, dir: &Path, sub: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = vec![
        "compose".to_string(),
        "-p".to_string(),
        project.to_string(),
        "--project-directory".to_string(),
        dir.to_string_lossy().to_string(),
        "-f".to_string(),
        dir.join("docker-compose.yml").to_string_lossy().to_string(),
    ];
    out.extend(sub.iter().map(|s| s.to_string()));
    out
}

/// 解析 `docker compose ps -q` 的输出 → 容器数。
///
/// 用 `-q`（只出 id）而不是解析表格或 JSON：列宽与字段随 compose 版本变，
/// 而「一行一个 id」是稳定的。
pub fn parse_ps_ids(out: &str) -> usize {
    out.lines().filter(|l| !l.trim().is_empty()).count()
}

/// 由「在跑的容器数 + 端口是否被占」判定服务状态。
///
/// 容器数为 0 但端口被占 = 有别人的东西在那个端口上（`port_conflict`），
/// 这与「没启动」必须区分：前者用户需要改端口或停掉占用者。
pub fn compose_state(running_containers: usize, port_busy: bool) -> &'static str {
    if running_containers > 0 {
        "running"
    } else if port_busy {
        "port_conflict"
    } else {
        "stopped"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(ok: bool, stdout: &str, stderr: &str) -> CmdOut {
        CmdOut {
            ok,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn render_replaces_all_placeholders_and_normalizes_slashes() {
        let tpl = "ports:\n  - 127.0.0.1:{{port}}:8787\n  - 127.0.0.1:{{port2}}:8788\n  - {{dir}}/data:/data\nPG={{pg_password}}";
        let got = render_template(tpl, 8787, 8789, "pw123456", r"C:\Users\me\AppData\docker\artex");
        assert!(got.contains("127.0.0.1:8787:8787"));
        assert!(
            got.contains("127.0.0.1:8789:8788"),
            "第二个对外端口也要能被替换，否则会绑到写死的端口上（实测撞了 Kira 的 8788）"
        );
        assert!(got.contains("C:/Users/me/AppData/docker/artex/data:/data"));
        assert!(got.contains("PG=pw123456"));
        // 反斜杠必须一个不剩：YAML 里它是转义符，留着必然解析失败
        assert!(!got.contains('\\'));
    }

    #[test]
    fn generated_password_is_alnum_and_unique() {
        let a = generate_password();
        let b = generate_password();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b, "两次生成不应相同");
    }

    #[test]
    fn parses_versions_from_real_world_output() {
        assert_eq!(parse_cli_version("Docker version 27.3.1, build 9d9e0dc"), "27.3.1");
        assert_eq!(parse_cli_version("Docker version 27.3.1-rd, build abc"), "27.3.1-rd");
        assert_eq!(parse_compose_version("Docker Compose version v2.29.7"), "2.29.7");
        assert_eq!(parse_compose_version("Docker Compose version 2.20.0"), "2.20.0");
    }

    #[test]
    fn env_distinguishes_not_installed_from_daemon_down() {
        // 没装：CLI 就起不来
        let e = env_from_outputs(&out(false, "", "program not found"), &out(false, "", ""), &out(false, "", ""));
        assert!(!e.cli);
        assert!(e.summary.contains("未检测到 Docker"), "{}", e.summary);

        // 装了但引擎没起：最常见的失败态，必须和「没装」给出不同的话
        let e = env_from_outputs(
            &out(true, "Docker version 27.3.1, build 9d9e0dc", ""),
            &out(true, "Docker Compose version v2.29.7", ""),
            &out(false, "", "error during connect: … docker daemon is not running"),
        );
        assert!(e.cli && !e.daemon_running);
        assert!(e.summary.contains("引擎没在运行"), "{}", e.summary);
        assert!(e.error.contains("daemon is not running"), "原文错误要透出给用户");

        // 就绪
        let e = env_from_outputs(
            &out(true, "Docker version 27.3.1, build 9d9e0dc", ""),
            &out(true, "Docker Compose version v2.29.7", ""),
            &out(true, "27.3.1", ""),
        );
        assert!(e.cli && e.compose && e.daemon_running);
        assert!(e.summary.contains("就绪"), "{}", e.summary);
        assert!(e.error.is_empty());
    }

    #[test]
    fn ps_ids_counts_non_empty_lines() {
        assert_eq!(parse_ps_ids(""), 0);
        assert_eq!(parse_ps_ids("abc\n"), 1);
        assert_eq!(parse_ps_ids("abc\n\n def \n"), 2);
    }

    #[test]
    fn compose_state_separates_conflict_from_stopped() {
        assert_eq!(compose_state(2, true), "running");
        assert_eq!(compose_state(0, true), "port_conflict");
        assert_eq!(compose_state(0, false), "stopped");
    }

    fn spec<'a>(compose: &'a str, env: &'a str, volumes: &'a [String]) -> ServiceSpec<'a> {
        ServiceSpec {
            id: "demo",
            port: 8787,
            port2: 8789,
            project: "kira-demo",
            volumes,
            compose_tpl: compose,
            env_tpl: env,
        }
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kira-docker-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn prepare_is_idempotent_and_keeps_password() {
        let dir = tmp_dir("prepare");
        let vols = vec!["data".to_string(), "skills".to_string()];
        let s = spec("port: {{port}}\nPG={{pg_password}}\nd={{dir}}", "PG={{pg_password}}", &vols);

        let first = prepare_in(&dir, &s).expect("首次应成功");
        assert!(first.prepared);
        assert!(dir.join("data").is_dir() && dir.join("skills").is_dir(), "挂载目录要建出来");
        assert!(dir.join("docker-compose.yml").is_file() && dir.join(".env").is_file());

        // 再跑一次：密码必须不变，否则 postgres 数据卷会连不上
        let again = prepare_in(&dir, &s).expect("重复调用应幂等");
        assert_eq!(first.pg_password, again.pg_password);
        let compose = fs::read_to_string(dir.join("docker-compose.yml")).unwrap();
        assert!(compose.contains(&first.pg_password));
        assert!(compose.contains("port: 8787"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_config_validates_and_rerenders() {
        let dir = tmp_dir("save");
        let vols: Vec<String> = Vec::new();
        let s = spec("port: {{port}}\nPG={{pg_password}}", "", &vols);
        prepare_in(&dir, &s).unwrap();

        assert!(save_config_in(&dir, &s, 80, 0, "longenough").is_err(), "低端口应拒绝");
        assert!(save_config_in(&dir, &s, 8788, 0, "short").is_err(), "密码过短应拒绝");
        assert!(
            save_config_in(&dir, &s, 9000, 9000, "longenough").is_err(),
            "两个端口相同应被拒"
        );

        let cfg = save_config_in(&dir, &s, 9000, 0, "newpassword").expect("合法值应成功");
        assert_eq!(cfg.port, 9000);
        assert_eq!(
            cfg.port2, 8789,
            "port2 传 0 时应回落定义默认值，而不是把 0 写进配置让 compose 绑随机端口"
        );
        let compose = fs::read_to_string(dir.join("docker-compose.yml")).unwrap();
        assert!(compose.contains("port: 9000"), "改端口后模板要重渲");
        assert!(compose.contains("newpassword"));

        // 重渲之后读回来仍是新值（端口与密码已落盘）
        let back = read_config_in(&dir, &s);
        assert_eq!(back.port, 9000);
        assert_eq!(back.pg_password, "newpassword");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compose_args_always_pin_project_dir_and_file() {
        let dir = PathBuf::from(r"C:\x\docker\demo");
        let args = compose_args("kira-demo", &dir, &["up", "-d"]);
        assert_eq!(args[0], "compose");
        assert_eq!(args[1], "-p");
        assert_eq!(args[2], "kira-demo");
        assert_eq!(args[3], "--project-directory");
        assert!(args[4].ends_with(r"docker\demo"));
        assert_eq!(args[5], "-f");
        assert!(args[6].ends_with("docker-compose.yml"));
        assert_eq!(&args[7..], &["up", "-d"]);
    }
}
