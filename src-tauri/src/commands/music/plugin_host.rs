//! MusicFree 插件宿主（常驻 Node 子进程桥）。
//!
//! 插件是**标准 CommonJS**（真实插件顶部就是 `require("axios") / require("cheerio") /
//! require("crypto-js")`），所以交给真实 Node 跑、配一份真实 `node_modules` 就够，
//! 不需要内嵌 JS 引擎，也不需要复刻 MusicFree 的宿主注入。
//!
//! 目录布局（`data_dir/music/plugins/`）：
//!
//! ```text
//! music/plugins/
//!   bridge.js        ← 宿主桥（随 app 资产释放，升级时自动更新）
//!   package.json     ← 依赖清单（同上）
//!   scripts/*.js     ← 插件本体。放子目录是为了：快照只挑这里、插件的相对引用不外溢
//!   node_modules/    ← 依赖（一次装、所有插件共享；不进快照，可重建）
//! ```
//!
//! `scripts/` 下能直接 `require("axios")`：Node 的解析会从该文件所在目录向上找
//! `node_modules`，因此 `music/plugins/node_modules` 正好命中。
//!
//! **node/npm 只认裸名，且只在「用户真实 PATH」里找** —— 见 [`user_path_entries`]。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

/// 宿主桥脚本（编译期内联，避免依赖打包资源的目录布局）。
const BRIDGE_JS: &str = include_str!("../../../assets/musicfree-bridge/bridge.js");
/// 桥的依赖清单。
const BRIDGE_MANIFEST: &str = include_str!("../../../assets/musicfree-bridge/package.json");

/// 桥保证可用的依赖，与 `package.json` 一一对应。
/// 用来判断「依赖是否就绪」：要求每个包在 `node_modules` 里都有实体。
const REQUIRED_PACKAGES: &[&str] = &[
    "axios",
    "cheerio",
    "crypto-js",
    "dayjs",
    "big-integer",
    "qs",
    "he",
];

// ─── 目录布局 ───

/// 测试注入的数据根。
///
/// **必须有这个钩子**：本模块会往磁盘写 bridge.js / package.json。
/// 若测试直接走 `super::library::music_dir()`，一次 `cargo test` 就会往真实用户的
/// `data_dir/music/plugins` 里写东西 —— 本项目已经因为「测试绕过声明目录」踩过一次
/// 真实事故（见 `codex_catalog` 的注释），所以这里照 `buddy::store::TEST_DATA_ROOT`
/// 的既有做法，一开始就留出注入点。
#[cfg(test)]
static TEST_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
fn set_test_root(root: PathBuf) {
    *TEST_ROOT.lock().unwrap() = Some(root);
}

/// 写入型测试的全局串行锁。
///
/// `plugin_host` 与 `plugin_registry` 共用同一个测试根（同一份 `registry.json`、
/// 同一个 `scripts/`），而 `cargo test` 默认并行 —— 不串行的话，
/// 「A 测试刚设好的启用状态被 B 测试的 save 覆盖」这类随机失败必然出现
/// （同 `buddy::store` 的处理方式）。
#[cfg(test)]
pub(super) fn serialize_test() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // 单个测试失败会毒化锁，恢复它以免连锁拖垮后续用例
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 本模块与 `plugin_registry` 共用的测试根。
///
/// **必须只有一份**：两个测试模块各自设一次根的话，后写的会盖掉先写的，
/// 于是「另一个模块的测试把文件写进前一个模块的目录」这种乱象会出现，且是随机失败。
#[cfg(test)]
pub(super) fn shared_test_root() -> PathBuf {
    static INIT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("kira_music_plugin_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建测试根失败");
        set_test_root(dir.clone());
        dir
    })
    .clone()
}

/// 插件根目录：`<data_dir>/music/plugins`。
pub fn plugins_root() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(root) = TEST_ROOT.lock().unwrap().clone() {
            return root.join("plugins");
        }
    }
    super::library::music_dir().join("plugins")
}

/// 插件本体目录（`scripts/*.js`）。
pub fn scripts_dir() -> PathBuf {
    plugins_root().join("scripts")
}

/// 宿主桥脚本路径。
pub fn bridge_script_path() -> PathBuf {
    plugins_root().join("bridge.js")
}

/// 依赖清单路径。
pub fn manifest_path() -> PathBuf {
    plugins_root().join("package.json")
}

/// 依赖安装目录。
pub fn node_modules_dir() -> PathBuf {
    plugins_root().join("node_modules")
}

/// 确保目录布局与桥资产就位。返回**本次是否写了东西**（幂等：内容一致就不动）。
///
/// 用内容比对而非「存在即跳过」：app 升级后桥脚本会变，不更新的话就是在跑旧协议。
pub fn ensure_layout() -> Result<bool, String> {
    let root = plugins_root();
    std::fs::create_dir_all(scripts_dir())
        .map_err(|e| format!("创建插件目录 {} 失败: {e}", root.display()))?;

    let mut wrote = false;
    for (path, content) in [
        (bridge_script_path(), BRIDGE_JS),
        (manifest_path(), BRIDGE_MANIFEST),
    ] {
        let same = std::fs::read_to_string(&path)
            .map(|existing| existing == content)
            .unwrap_or(false);
        if !same {
            std::fs::write(&path, content.as_bytes())
                .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
            wrote = true;
        }
    }
    Ok(wrote)
}

// ─── node / npm 解析（策略：只认裸名 + 用户真实 PATH）───

/// 用户真实 PATH 的分段。
///
/// **刻意不用 `std::env::var("PATH")`**：`lib.rs::sync_process_path` 会把 Kira 托管目录
/// （`data_dir/sdk/*`、`links_dir`）**前置**到本进程 PATH，于是裸名 `node` 会解析到
/// Kira 自己管的那个，而不是用户终端里那个 —— 与「使用用户本机的 node、不与 SDK 模块
/// 交互」直接相悖。读注册表里的用户级 / 系统级 PATH，才是用户终端里的真实顺序。
#[cfg(windows)]
fn user_path_entries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for raw in [
        crate::commands::env::get_registry_env("PATH"),
        crate::commands::env::get_system_registry_env("PATH"),
    ] {
        if let Some(value) = raw {
            out.extend(std::env::split_paths(&value));
        }
    }
    // 注册表读不到（受限环境）时退回进程 PATH：总比完全没有强。
    if out.is_empty() {
        if let Ok(value) = std::env::var("PATH") {
            out.extend(std::env::split_paths(&value));
        }
    }
    out
}

#[cfg(not(windows))]
fn user_path_entries() -> Vec<PathBuf> {
    std::env::var("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

/// 候选文件名。
///
/// Windows 上 `npm` 在 PATH 里通常只有 `npm.cmd` 与 `npm.ps1` —— 而 `.ps1` **不可执行**
/// （CreateProcess 会报 os error 193），所以只认 PE/批处理扩展名。
fn candidate_names(base: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{base}.exe"),
            format!("{base}.cmd"),
            format!("{base}.bat"),
            base.to_string(),
        ]
    } else {
        vec![base.to_string()]
    }
}

fn find_exe(base: &str) -> Option<PathBuf> {
    for dir in user_path_entries() {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for name in candidate_names(base) {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 一次可执行的调用方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// 磁盘上真实命中的文件（展示 / 日志用）。
    pub resolved: PathBuf,
    /// 实际交给 `Command::new` 的程序。
    pub program: PathBuf,
    /// 需要插入的前置参数（脚本类要走 `cmd /c`）。
    pub prefix_args: Vec<String>,
}

/// 把命中路径包装成可执行的调用方式。
///
/// Windows 上只有 `.exe` / `.com` 能被 CreateProcess 直接执行；`.cmd` / `.bat` 必须经
/// `cmd.exe /c`，否则报 os error 193（本项目 `node_manager::resolve_exe_invocation`
/// 有同款注释与处理）。
fn invocation(resolved: PathBuf) -> Invocation {
    let lower = resolved.to_string_lossy().to_lowercase();
    let needs_shell = cfg!(windows) && !(lower.ends_with(".exe") || lower.ends_with(".com"));
    if needs_shell {
        let cmd = std::env::var("COMSPEC")
            .unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_string());
        Invocation {
            program: PathBuf::from(cmd),
            prefix_args: vec![
                "/c".to_string(),
                resolved.to_string_lossy().to_string(),
            ],
            resolved,
        }
    } else {
        Invocation {
            program: resolved.clone(),
            prefix_args: Vec::new(),
            resolved,
        }
    }
}

/// 本机可用的 node / npm。
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub node: Invocation,
    pub npm: Invocation,
}

fn missing_node_hint() -> String {
    "未检测到 Node.js（PATH 中找不到 node/npm）。音乐插件需要本机已安装的 Node.js。\
     请先安装（https://nodejs.org）后重试"
        .to_string()
}

/// 解析 node / npm。两者缺一都算不可用 —— 装依赖要用 npm。
pub fn resolve_toolchain() -> Result<Toolchain, String> {
    let node = find_exe("node").ok_or_else(missing_node_hint)?;
    let npm = find_exe("npm").ok_or_else(missing_node_hint)?;
    Ok(Toolchain {
        node: invocation(node),
        npm: invocation(npm),
    })
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

// ─── 沙箱（Node 权限模型）───

/// 给 node 子进程清掉会被上层注入的 Node 环境变量。
///
/// `NODE_OPTIONS` 常被 IDE / 工具链用来注入 `--require` 预加载（编辑器自带的 node shim
/// 就这么干）。那种预加载会去读它自己所在目录，在权限模型下直接 `ERR_ACCESS_DENIED`。
///
/// **探测与正式启动都必须清**：不清的话探测会失败，于是得出
/// 「本机 Node 不支持权限模型」的错误结论，沙箱被静默关掉 —— 真机上正是这么踩到的
/// （单元测试当场抓到：所有危险能力都报 allowed）。
fn clean_node_env(cmd: &mut Command) {
    cmd.env_remove("NODE_OPTIONS").env_remove("NODE_PATH");
}

/// Node 是否支持权限模型（`--permission`，Node 20+）。
///
/// 用 `--version` 探测：Node 不认识该参数时会直接以 `bad option` 退出，与脚本内容无关，
/// 因此不必写任何临时文件。结论只取决于 node 二进制本身，全进程缓存一次即可。
fn sandbox_supported(node: &Invocation) -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let mut cmd = Command::new(&node.program);
        cmd.args(&node.prefix_args).arg("--permission").arg("--version");
        clean_node_env(&mut cmd);
        hide_console(&mut cmd);
        let supported = cmd.output().map(|out| out.status.success()).unwrap_or(false);
        eprintln!("[plugin] 沙箱探测：node 是否支持权限模型 = {supported}");
        supported
    })
}

/// 沙箱状态（给界面展示）。
pub fn sandbox_status(node: &Invocation) -> SandboxStatus {
    if sandbox_supported(node) {
        SandboxStatus {
            enabled: true,
            reason: None,
        }
    } else {
        SandboxStatus {
            enabled: false,
            reason: Some(
                "当前 Node 不支持权限模型（需 Node 20+），插件将以完整权限运行".to_string(),
            ),
        }
    }
}

/// 沙箱启动参数：启用权限模型，只放行**读插件目录**。
///
/// 刻意**不**给 `--allow-fs-write` / `--allow-child-process` / `--allow-worker`：
/// 插件只需要「读自己的目录 + 联网」，其余一律拒绝（实测这套配置下
/// 读系统文件、写外部文件、起子进程、建 worker 全部 `ERR_ACCESS_DENIED`，
/// 而 `require("axios")` 与 https 请求照常可用）。
///
/// 网络不在权限模型管辖内 —— Node 24 没有 `--allow-net`（那是更高版本的开关），
/// 而插件本来就靠它取数据，所以这一项不需要也无法放行。
///
/// 注意：Node 官方把权限模型定位为「安全带」而非安全边界，恶意代码理论上仍可绕过。
/// 它挡的是**插件无意/恶意的越权文件访问**，不是铁笼。
fn sandbox_args() -> Vec<String> {
    vec![
        "--permission".to_string(),
        format!("--allow-fs-read={}", plugins_root().display()),
    ]
}

/// 给 node 命令加上沙箱参数（不支持则记一条日志）。
///
/// ⚠️ **必须在脚本路径之前调用**：Node 的命令行是 `node [options] script [args]`，
/// 选项写到脚本后面会被当成**脚本参数**原样转给脚本 —— 沙箱静默失效，
/// 而插件照常能跑，从表象上完全看不出来（真机上正是如此，靠单元测试才抓出来）。
fn apply_sandbox(cmd: &mut Command, node: &Invocation) {
    if sandbox_supported(node) {
        cmd.args(sandbox_args());
    } else {
        eprintln!("[plugin] 当前 Node 不支持权限模型（需 20+），插件将以完整权限运行");
    }
}

// ─── 依赖状态与安装 ───

/// 插件沙箱状态（Node 权限模型）。
#[derive(Debug, Clone, Serialize)]
pub struct SandboxStatus {
    /// 是否启用了 `--permission`
    pub enabled: bool,
    /// 未启用时的原因（如 Node 版本过低）
    pub reason: Option<String>,
}

/// 依赖现状（直接喂给前端展示）。
#[derive(Debug, Clone, Serialize)]
pub struct DepsReport {
    /// 依赖是否齐备（可以跑插件）。
    pub ready: bool,
    pub node_found: bool,
    /// node 可执行文件路径（找到了才有）。
    pub node_path: Option<String>,
    pub npm_path: Option<String>,
    /// 缺失的包名（空 = 齐备）。
    pub missing_packages: Vec<String>,
    /// 插件根目录（界面上「打开目录」用）。
    pub root: String,
    /// 环境问题（缺 node 等）；有值时 `ready` 必为 false。
    pub problem: Option<String>,
    /// 插件沙箱状态
    pub sandbox: SandboxStatus,
}

/// 读磁盘得出依赖现状（除沙箱探测外不跑命令）。
pub fn deps_report() -> DepsReport {
    let missing: Vec<String> = REQUIRED_PACKAGES
        .iter()
        .filter(|name| {
            !node_modules_dir()
                .join(name)
                .join("package.json")
                .is_file()
        })
        .map(|name| name.to_string())
        .collect();

    match resolve_toolchain() {
        Ok(toolchain) => DepsReport {
            ready: missing.is_empty(),
            node_found: true,
            node_path: Some(toolchain.node.resolved.to_string_lossy().to_string()),
            npm_path: Some(toolchain.npm.resolved.to_string_lossy().to_string()),
            missing_packages: missing,
            root: plugins_root().to_string_lossy().to_string(),
            problem: None,
            sandbox: sandbox_status(&toolchain.node),
        },
        Err(problem) => DepsReport {
            ready: false,
            node_found: false,
            node_path: None,
            npm_path: None,
            missing_packages: missing,
            root: plugins_root().to_string_lossy().to_string(),
            problem: Some(problem),
            // 连 node 都没有，沙箱无从谈起；问题已经由 problem 说清楚了
            sandbox: SandboxStatus {
                enabled: false,
                reason: None,
            },
        },
    }
}

/// 跑一个子进程，把 stdout / stderr 的每一行交给 `log`（npm 的进度在 stderr 上）。
///
/// 用「两个转发线程 + 一个消费循环」而不是 `Command::output()`：
/// 安装要几十秒，必须能边跑边把进度报给前端，否则界面只能干等。
fn run_streaming(cmd: &mut Command, log: &mut dyn FnMut(&str)) -> Result<bool, String> {
    hide_console(cmd);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动进程失败: {e}"))?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut pumps = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        pumps.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        pumps.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }
    drop(tx); // 主线程不持有发送端，子进程结束后 rx 才会断开

    while let Ok(line) = rx.recv() {
        if !line.trim().is_empty() {
            log(&line);
        }
    }
    for pump in pumps {
        let _ = pump.join();
    }
    let status = child.wait().map_err(|e| format!("等待进程结束失败: {e}"))?;
    Ok(status.success())
}

/// 安装 / 补齐插件依赖（幂等：已就绪直接返回，不跑 npm）。
///
/// 依赖是**功能级前置**（一次装、所有插件共享），不像插件那样按个安装 ——
/// 插件依赖的都是 axios/cheerio 这类通用库。
pub fn install_deps(log: &mut dyn FnMut(&str)) -> Result<DepsReport, String> {
    ensure_layout()?;
    let before = deps_report();
    if let Some(problem) = before.problem.clone() {
        return Err(problem);
    }
    if before.ready {
        log("依赖已就绪，无需安装");
        return Ok(before);
    }

    let toolchain = resolve_toolchain()?;
    log(&format!(
        "使用 node: {} / npm: {}",
        toolchain.node.resolved.display(),
        toolchain.npm.resolved.display()
    ));
    log(&format!("正在安装插件依赖到 {}", plugins_root().display()));

    let mut cmd = Command::new(&toolchain.npm.program);
    cmd.args(&toolchain.npm.prefix_args)
        .arg("install")
        .arg("--no-audit")
        .arg("--no-fund")
        .arg("--loglevel=error")
        .current_dir(plugins_root());

    if !run_streaming(&mut cmd, log)? {
        // npm 失败时磁盘上可能留下半个 node_modules，下次会被「缺失包」检查抓出来，
        // 所以这里只报错、不做清理（清理反而可能删掉用户已有的其它内容）。
        return Err("npm install 失败，请检查网络或用终端进入插件目录手动安装".to_string());
    }

    let after = deps_report();
    if !after.ready {
        return Err(format!(
            "安装完成但仍缺少依赖: {}",
            after.missing_packages.join(", ")
        ));
    }
    log("依赖安装完成");
    Ok(after)
}

// ─── 桥进程 ───

/// 常驻桥进程。
struct Bridge {
    child: Child,
    stdin: ChildStdin,
    /// 桥的 stdout（协议流），由转发线程推进来。
    lines: Receiver<String>,
    next_id: u64,
    /// 已载入的插件及其元信息，避免每次调用都重复 load。
    loaded: Option<(PathBuf, Value)>,
}

impl Bridge {
    fn spawn(toolchain: &Toolchain) -> Result<Self, String> {
        let mut cmd = Command::new(&toolchain.node.program);
        cmd.args(&toolchain.node.prefix_args);
        // 沙箱参数必须排在脚本路径之前（见 apply_sandbox 的注释）
        apply_sandbox(&mut cmd, &toolchain.node);
        cmd.arg(bridge_script_path())
            // 工作目录 = 插件根，保证桥自身与被 require 的插件都能解析到 node_modules。
            .current_dir(plugins_root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        clean_node_env(&mut cmd);
        hide_console(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("启动插件宿主失败: {e}"))?;
        let stdin = child.stdin.take().ok_or("无法获取插件宿主 stdin")?;
        let stdout = child.stdout.take().ok_or("无法获取插件宿主 stdout")?;

        let (tx, lines) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("[plugin] {line}");
                }
            });
        }

        Ok(Bridge {
            child,
            stdin,
            lines,
            next_id: 0,
            loaded: None,
        })
    }

    fn is_dead(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// 杀掉并回收。超时 / 管道断开时调用：**必须连进程一起换掉**，
    /// 否则插件里那个卡住的 Promise 会一直占着下一次调用。
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.loaded = None;
    }

    fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        if self.is_dead() {
            return Err("插件宿主已退出".to_string());
        }
        self.next_id += 1;
        let id = self.next_id;
        let request = serde_json::to_string(&json!({ "id": id, "method": method, "params": params }))
            .map_err(|e| format!("序列化请求失败: {e}"))?;

        let write = self
            .stdin
            .write_all(request.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush());
        if let Err(e) = write {
            self.kill();
            return Err(format!("写入插件宿主失败（宿主可能已退出）: {e}"));
        }

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_default();
            if remaining.is_zero() {
                self.kill();
                return Err(format!(
                    "插件 {method} 超时（{} 秒），已重启宿主",
                    timeout.as_secs()
                ));
            }
            match self.lines.recv_timeout(remaining) {
                Ok(text) => {
                    // 非协议行（理论上不该有：桥已把插件日志改道 stderr）直接忽略，
                    // 不能让一行杂音把整次调用判死。
                    let Ok(message) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    if message.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if message.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                    }
                    let reason = message
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("插件返回了未知错误");
                    return Err(format!("插件 {method} 失败: {reason}"));
                }
                Err(RecvTimeoutError::Timeout) => continue, // 下一轮走 deadline 判定
                Err(RecvTimeoutError::Disconnected) => {
                    self.kill();
                    return Err("插件宿主已退出（插件进程可能崩溃）".to_string());
                }
            }
        }
    }
}

static HOST: Mutex<Option<Bridge>> = Mutex::new(None);

fn host_lock() -> Result<std::sync::MutexGuard<'static, Option<Bridge>>, String> {
    HOST.lock()
        .map_err(|_| "插件宿主状态损坏（上一次调用可能 panic）".to_string())
}

/// 取宿主进程（没有就起一个）。
fn ensure_bridge(guard: &mut Option<Bridge>) -> Result<&mut Bridge, String> {
    if guard.is_none() {
        let toolchain = resolve_toolchain()?;
        *guard = Some(Bridge::spawn(&toolchain)?);
    }
    Ok(guard.as_mut().expect("刚刚填入"))
}

/// 确保指定插件已载入，返回它的自述。已载入就直接复用，不再打扰插件。
fn ensure_loaded(
    bridge: &mut Bridge,
    plugin_path: &Path,
    timeout: Duration,
) -> Result<Value, String> {
    if let Some((loaded, meta)) = bridge.loaded.as_ref() {
        if loaded == plugin_path {
            return Ok(meta.clone());
        }
    }
    let meta = bridge.call(
        "load",
        json!({ "path": plugin_path.to_string_lossy() }),
        timeout,
    )?;
    bridge.loaded = Some((plugin_path.to_path_buf(), meta.clone()));
    Ok(meta)
}

/// 载入插件并返回其自述（`platform` / `version` / 能力…）。
///
/// **必须与 [`call_plugin`] 分开**：`call_plugin(path, "load", …)` 的语义是
/// 「先确保载入，**再调用名为 `load` 的方法**」—— 第二次 load 没有 `path` 参数，
/// 桥会 `require(undefined)` 报「The "id" argument must be of type string」。
/// 真机验证时正是这么踩到的：探测插件信息失败，还连带把宿主状态搞坏。
pub fn load_plugin(plugin_path: &Path, timeout: Duration) -> Result<Value, String> {
    ensure_layout()?;
    let mut guard = host_lock()?;
    let result = {
        let bridge = ensure_bridge(&mut guard)?;
        ensure_loaded(bridge, plugin_path, timeout)
    };
    if result.is_err() {
        *guard = None; // 载入失败多半是宿主状态坏了，丢掉重来
    }
    result
}

/// 该方法需要的能力位。缺了就返回一句人话，而不是等 JS 抛 `TypeError`。
fn capability_gap(meta: &Value, method: &str) -> Option<String> {
    let (key, label) = match method {
        "search" => ("hasSearch", "搜索"),
        "mediaSource" => ("hasMediaSource", "获取播放地址"),
        _ => return None,
    };
    if meta.get(key).and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let platform = meta
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("该插件");
    Some(format!("{platform} 不支持{label}"))
}

/// 调用某插件的方法（自动载入 / 复用宿主进程）。
///
/// 每次调用都带插件路径：与「当前载入的是谁」不一致时会重新 `load`，
/// 调用方不必关心宿主的生命周期。宿主全局只留一个（单进程串行调用）。
pub fn call_plugin(
    plugin_path: &Path,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    ensure_layout()?;
    let mut guard = host_lock()?;
    let mut reset = false;
    let result = {
        let bridge = ensure_bridge(&mut guard)?;
        match ensure_loaded(bridge, plugin_path, timeout) {
            Ok(meta) => match capability_gap(&meta, method) {
                Some(message) => Err(message),
                None => {
                    let call = bridge.call(method, params, timeout);
                    if call.is_err() && bridge.is_dead() {
                        reset = true; // 进程没了：下次调用自动重启，而不是一直报「已退出」
                    }
                    call
                }
            },
            Err(error) => {
                reset = true;
                Err(error)
            }
        }
    };
    if reset {
        *guard = None;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试绝不能落到真实用户的数据目录 —— 本模块会往磁盘写 bridge.js。
    #[test]
    fn plugin_root_stays_under_the_injected_test_root() {
        let _guard = serialize_test();
        let root = shared_test_root();
        let actual = plugins_root();
        assert!(
            actual.starts_with(&root),
            "插件目录跑到真实数据目录去了: {}",
            actual.display()
        );
    }

    #[test]
    fn layout_is_written_and_then_left_alone() {
        let _guard = serialize_test();
        shared_test_root();
        // 第二次调用必须返回 false（内容一致就不重写）：否则每次启动都改文件时间，
        // 快照/同步类功能会被无意义的 mtime 变化晃到。
        let first = ensure_layout().unwrap();
        let second = ensure_layout().unwrap();
        assert!(second == false || first == false, "重复调用不应反复重写");

        assert_eq!(
            std::fs::read_to_string(bridge_script_path()).unwrap(),
            BRIDGE_JS
        );
        assert_eq!(
            std::fs::read_to_string(manifest_path()).unwrap(),
            BRIDGE_MANIFEST
        );
        assert!(scripts_dir().is_dir());
    }

    /// 桥脚本必须能解析出依赖清单里那些包，否则插件一 `require` 就炸。
    /// 这条锁住「资产里的 package.json」与「REQUIRED_PACKAGES」不漂移。
    #[test]
    fn required_packages_match_the_manifest() {
        let manifest: Value = serde_json::from_str(BRIDGE_MANIFEST).unwrap();
        let deps = manifest.get("dependencies").and_then(Value::as_object).unwrap();
        for name in REQUIRED_PACKAGES {
            assert!(deps.contains_key(*name), "package.json 缺少依赖 {name}");
        }
        assert_eq!(
            deps.len(),
            REQUIRED_PACKAGES.len(),
            "清单与检查表数量不一致（有依赖没被检查，或反之）"
        );
    }

    /// `npm` 在 PATH 上通常是 `.cmd`/`.ps1`，`.ps1` 直接执行会 os error 193。
    #[test]
    fn script_commands_are_wrapped_for_cmd() {
        if !cfg!(windows) {
            return;
        }
        let wrapped = invocation(PathBuf::from("C:\\some\\where\\npm.cmd"));
        assert!(
            wrapped.program.to_string_lossy().to_lowercase().ends_with("cmd.exe"),
            "npm.cmd 必须经 cmd.exe 调用: {:?}",
            wrapped.program
        );
        assert_eq!(wrapped.prefix_args.first().map(String::as_str), Some("/c"));

        // .exe 是 PE 文件，不能再套一层 shell（会多出一个 cmd 进程）
        let direct = invocation(PathBuf::from("C:\\some\\where\\node.exe"));
        assert!(direct.prefix_args.is_empty());
        assert_eq!(direct.program, direct.resolved);
    }

    /// 候选扩展名里不能出现 `.ps1`（不可执行），且 `.exe` 优先。
    #[test]
    fn candidate_names_never_include_powershell_scripts() {
        let names = candidate_names("npm");
        if cfg!(windows) {
            assert_eq!(names[0], "npm.exe");
            assert!(!names.iter().any(|n| n.ends_with(".ps1")), "{names:?}");
            assert!(names.contains(&"npm.cmd".to_string()));
        } else {
            assert_eq!(names, vec!["npm".to_string()]);
        }
    }

    /// 真机链路：真 Node 启桥 → ping。没装 node 时跳过（不把测试变成环境依赖）。
    #[test]
    fn bridge_answers_ping_with_a_real_node() {
        let _guard = serialize_test();
        shared_test_root();
        if resolve_toolchain().is_err() {
            eprintln!("跳过：本机未检测到 node");
            return;
        }
        ensure_layout().unwrap();
        let toolchain = resolve_toolchain().unwrap();
        let mut bridge = Bridge::spawn(&toolchain).unwrap();
        let pong = bridge
            .call("ping", json!({}), Duration::from_secs(20))
            .expect("桥应能应答 ping");
        assert_eq!(pong.get("pong").and_then(Value::as_bool), Some(true));
        bridge.kill();
    }

    /// 未知方法要变成可读错误传给上层，而不是把桥搞挂。
    #[test]
    fn unknown_method_surfaces_as_a_readable_error() {
        let _guard = serialize_test();
        shared_test_root();
        if resolve_toolchain().is_err() {
            return;
        }
        ensure_layout().unwrap();
        let toolchain = resolve_toolchain().unwrap();
        let mut bridge = Bridge::spawn(&toolchain).unwrap();
        let err = bridge
            .call("noSuchMethod", json!({}), Duration::from_secs(20))
            .unwrap_err();
        assert!(err.contains("未知方法"), "{err}");
        // 报错之后桥还得活着：一个方法的失败不能污染后续调用
        let pong = bridge
            .call("ping", json!({}), Duration::from_secs(20))
            .expect("出错后桥仍应可用");
        assert_eq!(pong.get("pong").and_then(Value::as_bool), Some(true));
        bridge.kill();
    }

    /// 造一个假依赖（不装 npm 包、不联网）：只要解析路径对，`require` 就能命中。
    fn write_fake_axios() {
        let pkg = node_modules_dir().join("axios");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"axios","version":"0.0.0","main":"index.js"}"#,
        )
        .unwrap();
        std::fs::write(
            pkg.join("index.js"),
            "module.exports = { tag: 'fake-axios' };\n",
        )
        .unwrap();
    }

    /// 写一个最小可用插件：它 `require("axios")`，并把结果放进搜索返回值。
    fn write_probe_plugin(name: &str) -> PathBuf {
        let path = scripts_dir().join(name);
        std::fs::write(
            &path,
            r#"
const axios = require("axios");
module.exports = {
  platform: "Probe",
  version: "9.9.9",
  srcUrl: "https://example.com/probe.js",
  search: async (keyword, page) => ({
    isEnd: page > 1,
    data: [{ id: "1", title: keyword, axiosTag: axios.tag }],
  }),
  getMediaSource: async () => ({ url: "https://cdn.example.com/a.mp3" }),
};
"#,
        )
        .unwrap();
        path
    }

    /// **整套方案的关键布局假设**：插件放 `plugins/scripts/x.js`，依赖放
    /// `plugins/node_modules/`，隔了一层 —— Node 的 `require` 必须能从 `scripts/`
    /// 向上找到它。这条不成立的话，真实插件一 `require("axios")` 就炸。
    #[test]
    fn plugin_in_scripts_resolves_dependencies_one_level_up() {
        let _guard = serialize_test();
        shared_test_root();
        if resolve_toolchain().is_err() {
            eprintln!("跳过：本机未检测到 node");
            return;
        }
        ensure_layout().unwrap();
        write_fake_axios();
        let plugin = write_probe_plugin("probe-layout.js");

        let meta = load_plugin(&plugin, Duration::from_secs(30)).expect("应能载入插件");
        assert_eq!(meta.get("platform").and_then(Value::as_str), Some("Probe"));
        assert_eq!(meta.get("hasSearch").and_then(Value::as_bool), Some(true));
        assert_eq!(meta.get("hasMediaSource").and_then(Value::as_bool), Some(true));

        let hits = call_plugin(
            &plugin,
            "search",
            json!({ "keyword": "稻香", "page": 1 }),
            Duration::from_secs(30),
        )
        .expect("应能搜索");
        assert_eq!(hits["data"][0]["title"].as_str(), Some("稻香"), "{hits}");
        // 真的在插件里 require 到了上一层 node_modules 里的假 axios
        assert_eq!(
            hits["data"][0]["axiosTag"].as_str(),
            Some("fake-axios"),
            "插件没解析到上一层 node_modules: {hits}"
        );

        let source = call_plugin(
            &plugin,
            "mediaSource",
            json!({ "item": { "id": "1" }, "quality": "standard" }),
            Duration::from_secs(30),
        )
        .expect("应能取流");
        assert_eq!(source["url"].as_str(), Some("https://cdn.example.com/a.mp3"));

        let _ = std::fs::remove_file(&plugin);
    }

    /// 畸形 `load`（没有 `path`）不能把已载入的插件搞瘫。
    ///
    /// 这是真机验证踩到的真实事故：`call_plugin(path, "load", {})` 先正常载入，
    /// 紧接着又调用了一个没有 `path` 的 `load`；桥在报错**前**就清空了 `plugin`，
    /// 于是后续 `search` 全部报「尚未加载插件」，症状离原因非常远。
    #[test]
    fn malformed_load_keeps_the_loaded_plugin_usable() {
        let _guard = serialize_test();
        shared_test_root();
        if resolve_toolchain().is_err() {
            return;
        }
        ensure_layout().unwrap();
        write_fake_axios();
        let plugin = write_probe_plugin("probe-malformed.js");

        load_plugin(&plugin, Duration::from_secs(30)).expect("先正常载入");
        let bad = call_plugin(&plugin, "load", json!({}), Duration::from_secs(30));
        assert!(bad.is_err(), "没有 path 的 load 必须报错");

        // 关键：报错之后插件仍然可用
        let hits = call_plugin(
            &plugin,
            "search",
            json!({ "keyword": "x", "page": 1 }),
            Duration::from_secs(30),
        )
        .expect("畸形请求之后仍应能搜索");
        assert_eq!(hits["data"][0]["title"].as_str(), Some("x"), "{hits}");

        let _ = std::fs::remove_file(&plugin);
    }

    /// **沙箱的核心断言**：越权文件访问 / 起子进程 / 建 worker 必须全部被拒，
    /// 而插件干活需要的（读自己目录、加载依赖）必须照常。
    ///
    /// 这条不成立的话，「启用沙箱」就等于把插件功能一起关掉了。
    #[test]
    fn sandbox_blocks_privilege_while_plugins_keep_working() {
        let _guard = serialize_test();
        shared_test_root();
        if resolve_toolchain().is_err() {
            return;
        }
        let toolchain = resolve_toolchain().unwrap();
        if !sandbox_supported(&toolchain.node) {
            eprintln!("跳过：本机 node 不支持权限模型");
            return;
        }
        ensure_layout().unwrap();
        write_fake_axios();

        // 这个插件把「各项能力是被允许还是被拒」如实报回来
        let path = scripts_dir().join("probe-sandbox.js");
        std::fs::write(
            &path,
            r#"
const fs = require("fs");
const can = (fn) => { try { fn(); return "allowed"; } catch (e) { return e.code || "denied"; } };
const axios = require("axios");
module.exports = {
  platform: "SandboxProbe",
  search: async () => ({
    data: [{
      id: "1",
      title: "probe",
      axiosTag: axios.tag,
      readSystem: can(() => fs.readFileSync("C:/Windows/win.ini")),
      writeOutside: can(() => fs.writeFileSync(__dirname + "/../_sandbox_probe.txt", "x")),
      spawn: can(() => require("child_process").execSync("cmd /c echo hi")),
      worker: can(() => new (require("worker_threads").Worker)("0", { eval: true })),
      readOwn: can(() => fs.readFileSync(__dirname + "/probe-sandbox.js")),
    }],
  }),
};
"#,
        )
        .unwrap();

        let hits = call_plugin(
            &path,
            "search",
            json!({ "keyword": "x", "page": 1 }),
            Duration::from_secs(30),
        )
        .expect("沙箱下搜索应正常工作");
        let item = &hits["data"][0];

        for key in ["readSystem", "writeOutside", "spawn", "worker"] {
            assert_eq!(
                item[key].as_str(),
                Some("ERR_ACCESS_DENIED"),
                "{key} 没被拦住: {item}"
            );
        }
        // 必须能干活的照常干
        assert_eq!(item["readOwn"].as_str(), Some("allowed"), "{item}");
        assert_eq!(
            item["axiosTag"].as_str(),
            Some("fake-axios"),
            "依赖加载被沙箱误伤: {item}"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(plugins_root().join("_sandbox_probe.txt"));
        // 宿主里还缓存着这个测试插件，清掉免得影响后面的用例
        *HOST.lock().unwrap() = None;
    }

    /// 插件不支持某方法时，要给出人话而不是等 JS 抛异常。
    #[test]
    fn missing_capability_reports_a_readable_message() {
        let _guard = serialize_test();
        shared_test_root();
        if resolve_toolchain().is_err() {
            return;
        }
        ensure_layout().unwrap();
        let path = scripts_dir().join("probe-search-only.js");
        std::fs::write(
            &path,
            "module.exports = { platform: \"OnlySearch\", search: async () => ({ data: [] }) };\n",
        )
        .unwrap();

        let error = call_plugin(
            &path,
            "mediaSource",
            json!({ "item": {}, "quality": "standard" }),
            Duration::from_secs(30),
        )
        .unwrap_err();
        assert!(error.contains("OnlySearch") && error.contains("不支持"), "{error}");
        let _ = std::fs::remove_file(&path);
    }
}
