//! 通用「Node 项目管理器」
//!
//! 以配置驱动的方式管理类似 deepseek-harness（dsh）的 Node 项目：
//! 安装、升级、启动、停止、打开主页面。新增项目 = 在 node-projects/
//! 目录添加一个 JSON 定义，零代码改动。
//!
//! 两种安装方式（由配置决定）：
//! - git 模式（默认）：git clone → 包管理器 install → build
//! - npx 模式（配置 npxPackage）：`npm install --prefix <托管目录> <包名>` 一步到位，
//!   免 git clone / 依赖安装 / 编译；启动用 `npx --prefix <托管目录> <bin> <args...>`。
//!
//! 配置目录查找逻辑复用 ai-tools 的候选目录扫描机制（资源目录 / exe
//! 同级 / cwd / ~/.any-version）。

use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Emitter;
use crate::commands::config::get_base_dir;
use crate::commands::utils::find_in_path;

// ─── 配置定义（node-projects/*.json）───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeProjectDef {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub website: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_port")]
    pub default_port: u16,
    #[serde(default = "default_web_path")]
    pub web_path: String,
    /// node 版本 semver 约束，如 ">=20"；空串表示不约束。
    #[serde(default)]
    pub node_requirement: String,
    /// 包管理器：pnpm / npm / yarn
    #[serde(default = "default_pm")]
    pub package_manager: String,
    /// `pnpm run {build_script}` 的 script 名；空串表示无构建步骤。
    #[serde(default)]
    pub build_script: String,
    /// 启动命令子命令数组，如 ["dsh","web"] → `pnpm dsh web`
    #[serde(default)]
    pub start_cmd: Vec<String>,
    /// 是否显示在顶级导航列表
    #[serde(default = "default_true")]
    pub managed: bool,
    /// npx 模式：npm 包名（如 `@deepseek-ai/dsh`）。非空时改用 npx 流程：
    /// 安装/升级 = `npm install --prefix <托管目录> <包名>`，启动 = `npx --prefix <托管目录> <bin> <args...>`，
    /// 无需 git clone / 依赖安装 / 编译（前提是包已发布预编译产物）。
    #[serde(default)]
    pub npx_package: String,
    /// npx 模式下的可执行名；为空时取包名最后一段（`@scope/name` → `name`）。
    #[serde(default)]
    pub npx_bin: String,
}

fn default_port() -> u16 { 3000 }
fn default_web_path() -> String { "http://127.0.0.1:{port}".to_string() }
fn default_pm() -> String { "pnpm".to_string() }
fn default_true() -> bool { true }

impl NodeProjectDef {
    /// 渲染 webPath，替换 {port} 占位符。
    pub fn resolved_web_path(&self) -> String {
        self.web_path.replace("{port}", &self.default_port.to_string())
    }

    /// 托管目录：{node_projects_dir}/{id}
    /// node_projects_dir 可在全局设置中配置（默认 ~/.any-version/node-projects），
    /// 用于将服务类项目安装到非系统盘以节约 C 盘空间。
    pub fn managed_dir(&self) -> PathBuf {
        crate::commands::config::get_node_projects_dir().join(&self.id)
    }

    /// 是否使用 npx 模式（配置了 npxPackage）。
    pub fn is_npx(&self) -> bool {
        !self.npx_package.trim().is_empty()
    }

    /// npx 模式下实际执行的 bin 名；未配置 npxBin 时取包名最后一段。
    pub fn npx_bin_name(&self) -> String {
        let bin = self.npx_bin.trim();
        if !bin.is_empty() {
            return bin.to_string();
        }
        self.npx_package
            .trim()
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string()
    }

    /// npx 模式下判断包是否已安装到托管目录（node_modules/<包名> 存在，scoped 包含 @scope 前缀路径）。
    pub fn npx_installed(&self) -> bool {
        let pkg = self.npx_package.trim();
        if pkg.is_empty() {
            return false;
        }
        let mut rel = PathBuf::from("node_modules");
        for part in pkg.split('/') {
            rel.push(part);
        }
        self.managed_dir().join(rel).exists()
    }

    /// 判断是否已安装：git 模式看 package.json，npx 模式看 node_modules 里的包。
    pub fn installed(&self) -> bool {
        if self.is_npx() {
            return self.npx_installed();
        }
        let dir = self.managed_dir();
        dir.join("package.json").exists()
    }
}

// ─── 依赖检测结果 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepCheck {
    /// "git" | "node" | "pnpm" | "npm" | "yarn"
    pub name: String,
    pub exists: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    /// 是否满足项目约束（node 的 node_requirement；git/包管理器恒为 true 当 exists）
    pub satisfies: bool,
    #[serde(default)]
    pub requirement: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepCheckResult {
    pub git: DepCheck,
    pub node: DepCheck,
    pub package_manager: DepCheck,
    /// 全部就绪（可执行安装/升级）
    pub all_ready: bool,
}

// ─── 状态快照 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeProjectStatus {
    pub id: String,
    pub display_name: String,
    pub installed: bool,
    /// "running" | "stopped" | "not_installed" | "port_conflict"
    pub status: String,
    pub port: Option<u16>,
    pub pid: Option<u32>,
    #[serde(default)]
    pub git_version: Option<String>,
    /// npx 模式：本地已装 npm 包版本号（`node_modules/<pkg>/package.json` 的 version）。
    #[serde(default)]
    pub local_version: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

// ─── 注册表 ───

#[derive(Default)]
pub struct NodeProjectRegistry {
    projects: Vec<NodeProjectDef>,
}

impl NodeProjectRegistry {
    fn load() -> Self {
        let mut projects = Vec::new();
        if let Some(dir) = find_node_projects_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            if let Ok(def) = serde_json::from_str::<NodeProjectDef>(&content) {
                                projects.push(def);
                            } else {
                                eprintln!("[node_manager] 解析 {} 失败", path.display());
                            }
                        }
                    }
                }
            }
        }
        projects.sort_by(|a, b| a.id.cmp(&b.id));
        Self { projects }
    }

    pub fn all(&self) -> &[NodeProjectDef] {
        &self.projects
    }

    pub fn find(&self, id: &str) -> Option<&NodeProjectDef> {
        self.projects.iter().find(|p| p.id == id)
    }
}

static REGISTRY: Mutex<Option<Vec<NodeProjectDef>>> = Mutex::new(None);

/// 获取全局项目定义列表（首次加载后缓存）。
/// 注意：空列表不做缓存，以便后续配置目录就绪后能重新加载。
pub fn registry() -> Vec<NodeProjectDef> {
    {
        let g = REGISTRY.lock().unwrap();
        if let Some(list) = g.as_ref() {
            return list.clone();
        }
    }
    let loaded = NodeProjectRegistry::load().all().to_vec();
    if !loaded.is_empty() {
        let mut g = REGISTRY.lock().unwrap();
        *g = Some(loaded.clone());
    }
    loaded
}

/// 按 id 查找项目定义。
pub fn find_project(id: &str) -> Option<NodeProjectDef> {
    registry().into_iter().find(|p| p.id == id)
}

/// 定位 node-projects 配置目录。
/// 搜索策略与 ai-tools / projects 注册表保持一致：
/// 依次在「资源目录 / exe 同目录及向上 5 层 / 当前工作目录 / 用户配置目录」中
/// 查找 `node-projects` 或 `_up_/node-projects`（Tauri 打包时资源被拷贝进 `_up_` 前缀）。
pub(crate) fn find_node_projects_dir() -> Option<PathBuf> {
    let mut search_dirs: Vec<PathBuf> = Vec::new();

    // 优先在 Tauri 2 打包后的官方资源目录下查找
    if let Some(res_dir) = crate::commands::utils::get_resource_dir() {
        search_dirs.push(res_dir);
    }

    // exe 同目录及向上 5 层
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            search_dirs.push(exe_dir.to_path_buf());
            let mut dir = exe_dir.to_path_buf();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                    search_dirs.push(dir.clone());
                }
            }
        }
    }

    // 当前工作目录
    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd);
    }

    // 用户配置目录（~/.any-version）
    search_dirs.push(get_base_dir());

    // 每个候选目录下查找 node-projects 目录（含 Tauri 打包时的 `_up_` 前缀布局）
    for dir in &search_dirs {
        for candidate in [dir.join("_up_").join("node-projects"), dir.join("node-projects")] {
            if candidate.exists() && candidate.is_dir() {
                eprintln!("[node_manager] 命中 node-projects: {}", candidate.display());
                return Some(candidate);
            }
        }
    }
    None
}

// ─── 版本比较（轻量，满足 >=20 之类约束）───

fn parse_version_parts(v: &str) -> Vec<u64> {
    let mut parts = Vec::new();
    let mut current = 0u64;
    let mut has_digit = false;
    for c in v.chars() {
        if c.is_ascii_digit() {
            current = current * 10 + (c as u64 - '0' as u64);
            has_digit = true;
        } else if has_digit {
            parts.push(current);
            current = 0;
            has_digit = false;
        }
    }
    if has_digit {
        parts.push(current);
    }
    parts
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let pa = parse_version_parts(a);
    let pb = parse_version_parts(b);
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        match va.cmp(&vb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// 判断 version 是否满足形如 `>=20` / `>20` / `<=18` / `<20` / `20` 的约束。
/// 空约束返回 true。
fn version_satisfies(version: &str, requirement: &str) -> bool {
    let req = requirement.trim();
    if req.is_empty() {
        return true;
    }
    let version = version.trim();
    let (op, target) = if let Some(rest) = req.strip_prefix(">=") {
        (">=", rest.trim())
    } else if let Some(rest) = req.strip_prefix("<=") {
        ("<=", rest.trim())
    } else if let Some(rest) = req.strip_prefix('>') {
        (">", rest.trim())
    } else if let Some(rest) = req.strip_prefix('<') {
        ("<", rest.trim())
    } else if let Some(rest) = req.strip_prefix('=') {
        ("=", rest.trim())
    } else {
        // 裸版本号（如 "20"）视为「最低版本」语义，即 >=20
        (">=", req)
    };
    let cmp = compare_versions(version, target);
    match op {
        ">=" => cmp != std::cmp::Ordering::Less,
        "<=" => cmp != std::cmp::Ordering::Greater,
        ">" => cmp == std::cmp::Ordering::Greater,
        "<" => cmp == std::cmp::Ordering::Less,
        _ => cmp == std::cmp::Ordering::Equal,
    }
}

// ─── 命令执行辅助 ───

fn hidden_cmd(program: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    c
}

/// 执行命令并捕获 stdout/stderr，返回 (stdout, stderr, 是否成功)。
fn run_capture(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> (String, String, bool) {
    let mut cmd = hidden_cmd(program);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    match cmd.output() {
        Ok(out) => (
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
            out.status.success(),
        ),
        Err(e) => (String::new(), format!("无法执行 {}: {}", program, e), false),
    }
}

/// 读取某命令的版本号（如 `node --version` / `git --version`）。
fn command_version(exe: &str, args: &[&str]) -> Option<String> {
    let (stdout, _, ok) = run_capture(exe, args, None);
    if !ok {
        return None;
    }
    let text = stdout.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// 检测单个依赖（exe_name 可为 path 或命令名）。
fn check_dep(exe_name: &str, version_args: &[&str]) -> DepCheck {
    let path = find_in_path(exe_name);
    let exists = path.is_some();
    let version = if exists {
        command_version(&path.as_ref().unwrap().to_string_lossy(), version_args)
    } else {
        None
    };
    DepCheck {
        name: exe_name.to_string(),
        exists,
        path: path.map(|p| p.to_string_lossy().to_string()),
        version,
        satisfies: exists, // git/包管理器只要存在即满足；node 的约束由调用方补充
        requirement: None,
    }
}

// ─── 进度事件 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeProgress {
    pub project_id: String,
    pub phase: String,
    #[serde(default)]
    pub detail: String,
}

fn emit_progress(app: &tauri::AppHandle, project_id: &str, phase: &str, detail: &str) {
    let _ = app.emit(
        "npm-progress",
        NodeProgress {
            project_id: project_id.to_string(),
            phase: phase.to_string(),
            detail: detail.to_string(),
        },
    );
}

/// 实时日志事件（git pull / install / build / start 的输出逐行回传前端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeLog {
    pub project_id: String,
    pub phase: String,
    pub line: String,
}

fn emit_log(app: &tauri::AppHandle, project_id: &str, phase: &str, line: &str) {
    let _ = app.emit(
        "npm-log",
        NodeLog {
            project_id: project_id.to_string(),
            phase: phase.to_string(),
            line: line.trim_end().to_string(),
        },
    );
}

/// 执行命令并把 stdout/stderr 逐行实时 emit（phase 归类）。返回 (是否成功, 末尾错误行)。
fn run_capture_live(
    app: &tauri::AppHandle,
    project_id: &str,
    phase: &str,
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
) -> (bool, String) {
    let mut cmd = hidden_cmd(program);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_log(app, project_id, phase, &format!("无法执行 {}: {}", program, e));
            return (false, format!("无法执行 {}: {}", program, e));
        }
    };

    let app_out = app.clone();
    let pid = project_id.to_string();
    let ph = phase.to_string();
    let stdout = child.stdout.take();
    if let Some(mut so) = stdout {
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(&mut so);
            for line in reader.lines().map_while(|l| l.ok()) {
                emit_log(&app_out, &pid, &ph, &line);
            }
        });
    }
    let app_err = app.clone();
    let pid_e = project_id.to_string();
    let ph_e = phase.to_string();
    let stderr = child.stderr.take();
    let last_err = if let Some(mut se) = stderr {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(&mut se);
            let mut last = String::new();
            for line in reader.lines().map_while(|l| l.ok()) {
                emit_log(&app_err, &pid_e, &ph_e, &line);
                last = line;
            }
            let _ = tx.send(last);
        });
        rx.recv().unwrap_or_default()
    } else {
        String::new()
    };

    let status = child.wait();
    let ok = matches!(status, Ok(s) if s.success());
    (ok, last_err)
}

// ─── 安装 / 升级 ───

/// 解析包管理器为「可直接被 Command::new 执行」的调用方式 `(program, prefix_args)`。
/// 不能直接用裸命令名执行：pnpm/npm/yarn 在 Windows 上通常是 `pnpm.cmd` 脚本，
/// 既不在应用进程 PATH 的裸名解析范围内，也需经 `cmd.exe /c` 才能执行。
/// 返回的 program 为绝对路径，prefix_args 为执行前需插入的参数（脚本类为 `/c` 前缀）。
/// 返回 Windows cmd.exe 的绝对路径（避免裸名 `cmd` 受 PATH 影响）。
#[cfg(target_os = "windows")]
fn cmd_exe_path() -> String {
    std::env::var("COMSPEC")
        .or_else(|_| std::env::var("SystemRoot").map(|r| format!("{}\\System32\\cmd.exe", r)))
        .unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_string())
}

/// 解析任意命令为「可直接被 Command::new 执行」的调用方式 `(program, prefix_args)`。
/// 不能直接用裸命令名执行：npm/npx/pnpm 在 Windows 上通常是 `.cmd` 脚本，
/// 既不在应用进程 PATH 的裸名解析范围内，也需经 `cmd.exe /c` 才能执行。
/// 返回的 program 为绝对路径，prefix_args 为执行前需插入的参数（脚本类为 `/c` 前缀）。
/// not_found_hint：命令不在 PATH 时的提示语。
fn resolve_exe_invocation(exe: &str, not_found_hint: &str) -> Result<(String, Vec<String>), String> {
    let path = find_in_path(exe)
        .ok_or_else(|| format!("无法定位 {}（未在 PATH 中找到），{}", exe, not_found_hint))?;
    let p_str = path.to_string_lossy().to_string();
    let lower = p_str.to_lowercase();
    // Windows 上只有 .exe/.com 这类 PE 文件能被 CreateProcess 直接执行；
    // .cmd/.bat 及无扩展名脚本必须经 `cmd.exe /c` 解释，否则报 os error 193。
    let result = if !lower.ends_with(".exe") && !lower.ends_with(".com") {
        #[cfg(target_os = "windows")]
        {
            (cmd_exe_path(), vec!["/c".to_string(), p_str])
        }
        #[cfg(not(target_os = "windows"))]
        {
            (p_str, Vec::new())
        }
    } else {
        (p_str, Vec::new())
    };
    eprintln!(
        "[node_manager] resolve_exe_invocation({}) -> prog={:?}, prefix={:?}",
        exe, result.0, result.1
    );
    Ok(result)
}

fn resolve_pm_invocation(def: &NodeProjectDef) -> Result<(String, Vec<String>), String> {
    resolve_exe_invocation(&def.package_manager, "请先安装或启用 corepack")
}

/// 在托管目录执行包管理器 install。
fn pm_install(app: &tauri::AppHandle, def: &NodeProjectDef, dir: &Path) -> Result<(), String> {
    let pm = &def.package_manager;
    let (prog, prefix) = resolve_pm_invocation(def)?;
    emit_progress(app, &def.id, "install", &format!("正在运行 `{} install`（首次可能较慢）…", pm));
    let mut args = prefix;
    args.push("install".to_string());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err) = run_capture_live(app, &def.id, "install", &prog, &arg_refs, Some(dir), &[]);
    if !ok {
        let msg = last_err.trim();
        return Err(format!("`{} install` 失败: {}", pm, if msg.is_empty() { "未知错误" } else { msg }));
    }
    Ok(())
}

/// 执行构建（若有 build_script）。
fn pm_build(app: &tauri::AppHandle, def: &NodeProjectDef, dir: &Path) -> Result<(), String> {
    if def.build_script.trim().is_empty() {
        return Ok(());
    }
    let pm = &def.package_manager;
    let (prog, prefix) = resolve_pm_invocation(def)?;
    emit_progress(app, &def.id, "build", &format!("正在运行 `{} run {}`…", pm, def.build_script));
    let mut args = prefix;
    args.push("run".to_string());
    args.push(def.build_script.clone());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err) = run_capture_live(app, &def.id, "build", &prog, &arg_refs, Some(dir), &[]);
    if !ok {
        let msg = last_err.trim();
        return Err(format!("`{} run {}` 失败: {}", pm, def.build_script, if msg.is_empty() { "未知错误" } else { msg }));
    }
    Ok(())
}

/// 从包描述中剥离版本段（`@scope/name@1.0.0` → `@scope/name`；`name@1.0.0` → `name`），
/// 用于升级时拼 `@latest`。无版本段时原样返回。
fn strip_pkg_version(pkg: &str) -> &str {
    match pkg.rfind('@') {
        Some(idx) if idx > 0 => &pkg[..idx],
        _ => pkg,
    }
}

/// npx 模式安装/升级：`npm install --prefix <托管目录> <包名>`。
/// 一步到位（免 git clone / 依赖安装 / 编译）；`latest=true` 时强制取最新版（升级语义）。
///
/// 安装进程在 AnyVersion 的 CWD（通常是仓库根）里继承环境，npm 7+ 会向上查找
/// package.json / pnpm-workspace.yaml 把安装当 workspace 操作处理——仓库用的
/// pnpm `workspace:*` 协议依赖会触发 `EUNSUPPORTEDPROTOCOL Unsupported URL Type
/// "workspace:"`（npm 不认识 pnpm 的协议）。因此显式传 `--prefix` 指向托管目录
/// 并注入 `npm_config_workspaces=false` + `npm_config_workspaces_update=false`
/// 彻底禁用 workspace 模式；同时清理继承的 npm/pnpm 环境变量，防止宿主 shell
/// 的 `npm_config_*` 覆盖本次安装行为。
fn npx_install(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    dir: &Path,
    latest: bool,
) -> Result<(), String> {
    const NPM_ISOLATION_ENV: &[(&str, &str)] = &[
        // 显式关闭 workspaces 解析（含向上冒泡查找），npm 7+ 生效
        ("npm_config_workspaces", "false"),
        ("npm_config_workspaces_update", "false"),
        ("npm_config_workspaces_install", "false"),
        // pnpm 环境变量若被继承会改变依赖解析行为，清掉
        ("npm_config_shared_workspace_lockfile", "false"),
        ("npm_config_shell_emulator", "false"),
        // 关闭 fund/audit 提示：减少网络调用与日志噪音（安装的是托管依赖包，无需审计）
        ("npm_config_fund", "false"),
        ("npm_config_audit", "false"),
    ];
    let pkg = def.npx_package.trim();
    let spec = if latest {
        format!("{}@latest", strip_pkg_version(pkg))
    } else {
        pkg.to_string()
    };
    emit_progress(app, &def.id, "npx", &format!("正在通过 npx 安装 {} …", pkg));
    let _ = fs::create_dir_all(dir);
    let (prog, prefix) = resolve_exe_invocation("npm", "请先安装 Node.js (https://nodejs.org)")?;
    let mut args = prefix;
    args.push("install".to_string());
    args.push("--prefix".to_string());
    args.push(dir.to_string_lossy().to_string());
    // 双保险：环境变量之外再加 CLI 级禁用（npm 7+ 识别 --workspaces=false，
    // 老版本 npm 会忽略该 flag 不报错）
    args.push("--workspaces=false".to_string());
    args.push("--no-fund".to_string());
    args.push("--no-audit".to_string());
    args.push(spec);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err) = run_capture_live(app, &def.id, "npx", &prog, &arg_refs, None, NPM_ISOLATION_ENV);
    if !ok {
        let msg = last_err.trim();
        return Err(format!("npx 安装失败: {}", if msg.is_empty() { "未知错误" } else { msg }));
    }
    if !def.npx_installed() {
        let _ = fs::remove_dir_all(dir);
        return Err(format!("安装完成但未找到包 {}，请检查 npxPackage 是否拼写正确", pkg));
    }
    emit_progress(app, &def.id, "done", "npx 安装完成");
    Ok(())
}

/// 安装前检查依赖是否就绪，并返回 deps 结果用于报错。
fn ensure_deps_ready(def: &NodeProjectDef) -> Result<DepCheckResult, String> {
    let deps = check_deps(def);
    // npx 模式只需 Node（npm/npx 随 Node 提供），无需 git / 独立包管理器
    if def.is_npx() {
        if !deps.node.exists {
            return Err("未检测到 node，请先安装 Node.js (https://nodejs.org)".to_string());
        }
        if !deps.node.satisfies {
            return Err(format!(
                "Node 版本 {} 不满足要求 {}，请升级 Node.js",
                deps.node.version.as_deref().unwrap_or("未知"),
                deps.node.requirement.as_deref().unwrap_or("")
            ));
        }
        return Ok(deps);
    }
    if !deps.git.exists {
        return Err("未检测到 git，请先安装 Git for Windows (https://git-scm.com)".to_string());
    }
    if !deps.node.exists {
        return Err("未检测到 node，请先安装 Node.js (https://nodejs.org)".to_string());
    }
    if !deps.node.satisfies {
        return Err(format!(
            "Node 版本 {} 不满足要求 {}，请升级 Node.js",
            deps.node.version.as_deref().unwrap_or("未知"),
            deps.node.requirement.as_deref().unwrap_or("")
        ));
    }
    if !deps.package_manager.exists {
        return Err(format!(
            "未检测到 {}，请先安装（npm i -g {} 或启用 corepack）",
            def.package_manager, def.package_manager
        ));
    }
    Ok(deps)
}

#[tauri::command]
pub async fn npm_install(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    ensure_deps_ready(&def)?;

    let dir = def.managed_dir();
    // 已安装过则报错（避免覆盖；升级请走 npm_upgrade）
    if def.installed() {
        return Err("项目已安装，如需更新请使用「升级」".to_string());
    }

    // npx 模式：直接 npm install --prefix，一步到位，无需 git clone / 依赖安装 / 编译
    if def.is_npx() {
        return npx_install(&app, &def, &dir, false);
    }

    emit_progress(&app, &def.id, "clone", &format!("正在克隆 {} …", def.repo));
    let base = crate::commands::config::get_data_dir();
    let parent = dir.parent().unwrap_or(&base);
    let _ = fs::create_dir_all(parent);
    let _ = fs::remove_dir_all(&dir);
    // 直接指定克隆目标目录为托管目录 {id}，避免依赖仓库名。
    let clone_args: Vec<String> = vec![
        "clone".to_string(),
        "--depth".to_string(),
        "1".to_string(),
        def.repo.clone(),
        dir.to_string_lossy().to_string(),
    ];
    let arg_refs: Vec<&str> = clone_args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err) = run_capture_live(&app, &def.id, "clone", "git", &arg_refs, Some(parent), &[]);
    if !ok {
        let msg = last_err.trim();
        return Err(format!("git clone 失败: {}", if msg.is_empty() { "未知错误" } else { msg }));
    }

    if !dir.join("package.json").exists() {
        let _ = fs::remove_dir_all(&dir);
        return Err("克隆完成但未找到 package.json，可能仓库结构异常".to_string());
    }

    pm_install(&app, &def, &dir)?;
    pm_build(&app, &def, &dir)?;
    emit_progress(&app, &def.id, "done", "安装完成");
    Ok(())
}

#[tauri::command]
pub async fn npm_upgrade(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先安装".to_string());
    }
    ensure_deps_ready(&def)?;

    let dir = def.managed_dir();
    // 先停止运行中的进程，避免 pull / 文件占用冲突
    let _ = stop_project_process(&def);

    // npx 模式：重新 npm install 即更新到最新版
    if def.is_npx() {
        return npx_install(&app, &def, &dir, true);
    }

    emit_progress(&app, &def.id, "pull", "正在拉取最新代码 (git pull)…");
    let (ok, last_err) = run_capture_live(&app, &def.id, "pull", "git", &["pull"], Some(&dir), &[]);
    if !ok {
        let msg = last_err.trim();
        return Err(format!("git pull 失败: {}", if msg.is_empty() { "未知错误" } else { msg }));
    }

    pm_install(&app, &def, &dir)?;
    pm_build(&app, &def, &dir)?;
    emit_progress(&app, &def.id, "done", "升级完成");
    Ok(())
}

/// 单独安装依赖（不重新克隆/拉取代码）：执行包管理器 install + 构建脚本。
/// 用于项目已安装但依赖缺失/损坏时，仅重新安装依赖并打印实时日志。
#[tauri::command]
pub async fn npm_install_deps(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先「安装」".to_string());
    }
    ensure_deps_ready(&def)?;

    let dir = def.managed_dir();
    // npx 模式没有独立依赖步骤：重新安装即拉取最新版
    if def.is_npx() {
        return npx_install(&app, &def, &dir, true);
    }
    pm_install(&app, &def, &dir)?;
    pm_build(&app, &def, &dir)?;
    emit_progress(&app, &def.id, "done", "依赖安装完成");
    Ok(())
}

// ─── 进程生命周期 ───

/// 记录由本应用 spawn 的 Node 服务根 PID。
static RUNNING_PIDS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

fn running_pids() -> std::sync::MutexGuard<'static, Option<HashMap<String, u32>>> {
    RUNNING_PIDS.lock().unwrap()
}

fn record_pid(id: &str, pid: u32) {
    let mut g = running_pids();
    g.get_or_insert_with(HashMap::new).insert(id.to_string(), pid);
}

fn recorded_pid(id: &str) -> Option<u32> {
    running_pids().as_ref().and_then(|m| m.get(id).copied())
}

/// 停止项目进程树。
pub(crate) fn stop_project_process(def: &NodeProjectDef) -> Result<(), String> {
    // 1) 优先杀本应用记录的 PID 进程树
    if let Some(pid) = recorded_pid(&def.id) {
        let (_, stderr, ok) = run_capture("taskkill", &["/f", "/t", "/pid", &pid.to_string()], None);
        if !ok && !stderr.is_empty() && !stderr.contains("not found") && !stderr.contains("没有运行") {
            // 不直接报错，继续尝试端口杀
        }
        if let Some(m) = running_pids().as_mut() {
            m.remove(&def.id);
        }
    }
    // 2) 兜底：杀占用默认端口的进程（若是我们的）
    if let Some(pid) = crate::commands::utils::port_owner_pid(def.default_port) {
        let _ = run_capture("taskkill", &["/f", "/t", "/pid", &pid.to_string()], None);
    }
    Ok(())
}

#[tauri::command]
pub async fn npm_start(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先安装".to_string());
    }
    if def.start_cmd.is_empty() {
        return Err("该项目未配置启动命令".to_string());
    }

    // 端口冲突检测
    if let Some(pid) = crate::commands::utils::port_owner_pid(def.default_port) {
        let name = crate::commands::utils::process_name_by_pid(pid).unwrap_or_default();
        return Err(format!(
            "端口 {} 已被进程 {} (PID {}) 占用，请先停止或处理端口冲突",
            def.default_port, name, pid
        ));
    }

    let dir = def.managed_dir();

    // npx 模式：启动前自动拉取远程最新版（每次启动即自动加载最新版本）。
    // 更新失败时若本地已有包则回退本地版本继续启动，避免离线/网络异常导致无法启动。
    if def.is_npx() {
        match npx_install(&app, &def, &dir, true) {
            Ok(()) => {}
            Err(e) => {
                if def.npx_installed() {
                    emit_log(&app, &def.id, "stderr", &format!("自动更新到最新版失败，使用本地已装版本启动: {}", e));
                } else {
                    return Err(e);
                }
            }
        }
    }

    // 组装启动命令：git 模式 `pnpm dsh web`；npx 模式 `npx -y --prefix <托管目录> <bin> <args...>`
    let (prog, args) = if def.is_npx() {
        let (p, prefix) = resolve_exe_invocation("npx", "请先安装 Node.js (https://nodejs.org)")?;
        let mut a = prefix;
        a.push("-y".to_string());
        a.push("--prefix".to_string());
        a.push(dir.to_string_lossy().to_string());
        a.push(def.npx_bin_name());
        a.extend(def.start_cmd.iter().cloned());
        (p, a)
    } else {
        let (p, prefix) = resolve_pm_invocation(&def)?;
        let mut a = prefix;
        a.push(def.start_cmd[0].clone());
        a.extend_from_slice(&def.start_cmd[1..]);
        (p, a)
    };

    // 启动后台进程（CREATE_NO_WINDOW），记录 PID
    let mut cmd = hidden_cmd(&prog);
    cmd.args(&args);
    cmd.current_dir(&dir);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // 额外确保创建独立的进程组，便于后续 taskkill /T；
    // 通过 spawn_breakaway_fallback 尝试脱离 AnyVersion 生命周期，
    // 若所在 Job 不允许 breakaway 则自动降级（不会整体启动失败）。
    let mut child = crate::commands::hidden_cmd::spawn_breakaway_fallback(cmd)
        .map_err(|e| format!("启动失败: {}", e))?;
    record_pid(&def.id, child.id());

    // 实时回传启动日志（进程常驻，子线程持续读取 stdout/stderr 直到进程退出）
    emit_progress(&app, &def.id, "start", &format!("启动命令: {}", def.start_cmd.join(" ")));
    let app_out = app.clone();
    let pid_out = def.id.clone();
    if let Some(mut so) = child.stdout.take() {
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(&mut so);
            for line in reader.lines().map_while(|l| l.ok()) {
                emit_log(&app_out, &pid_out, "stdout", &line);
            }
        });
    }
    let app_err = app.clone();
    let pid_err = def.id.clone();
    if let Some(mut se) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(&mut se);
            for line in reader.lines().map_while(|l| l.ok()) {
                emit_log(&app_err, &pid_err, "stderr", &line);
            }
        });
    }

    // 等待端口就绪（最多 120s，异步等待，不阻塞 UI）。
    // 期间持续回传进度，并检测进程是否提前退出（启动命令报错时避免干等）。
    let deadline = Instant::now() + Duration::from_secs(120);
    let started_at = Instant::now();
    let mut last_report = Instant::now();
    loop {
        if crate::commands::utils::port_owner_pid(def.default_port).is_some() {
            emit_progress(&app, &def.id, "running", &format!("已启动，端口 {}", def.default_port));
            return Ok(());
        }
        // 进程已退出且端口未就绪 → 启动失败
        if let Ok(Some(_status)) = child.try_wait() {
            emit_log(&app, &def.id, "stderr", &format!("启动进程已退出（端口 {} 未就绪）", def.default_port));
            return Err(format!(
                "启动进程已退出，端口 {} 未就绪。请查看下方日志确认报错",
                def.default_port
            ));
        }
        if Instant::now() >= deadline {
            emit_progress(&app, &def.id, "running", "进程已启动（端口暂未就绪，请查看日志或稍候打开主页）");
            return Ok(());
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            emit_progress(
                &app,
                &def.id,
                "starting",
                &format!(
                    "等待端口 {} 就绪…（已等待 {}s）",
                    def.default_port,
                    started_at.elapsed().as_secs()
                ),
            );
            last_report = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tauri::command]
pub async fn npm_stop(project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    stop_project_process(&def)
}

/// 卸载：停止进程并清空托管目录（{node_projects_dir}/{id}）。
/// 用于 npx 模式切换 / 重装前一键清理旧目录（含 node_modules 等残留）。
#[tauri::command]
pub async fn npm_uninstall(project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    // 先停止运行中的进程，避免文件被占用导致删除失败
    let _ = stop_project_process(&def);

    let base = crate::commands::config::get_node_projects_dir();
    let dir = def.managed_dir();
    // 安全防护：托管目录必须是托管根目录的子目录（防配置异常/非法 id 误删其它目录）
    let base_c = base.canonicalize().unwrap_or_else(|_| base.clone());
    let dir_c = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    if !dir_c.starts_with(&base_c) {
        return Err(format!(
            "拒绝卸载：托管目录 {} 不在托管根目录 {} 内",
            dir.to_string_lossy(),
            base.to_string_lossy()
        ));
    }
    if !dir.exists() {
        return Err(format!(
            "托管目录不存在（可能尚未安装）：{}",
            dir.to_string_lossy()
        ));
    }
    // 标准删除失败（只读/锁定）时用 `cmd /c rmdir /s /q` 兜底
    crate::commands::cache::remove_dir_all_forced(&dir)
        .map_err(|e| format!("清空托管目录失败: {}", e))?;
    Ok(())
}

// ─── 打开主页面 ───

#[tauri::command]
pub async fn npm_open(project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    let url = def.resolved_web_path();
    if let Some(path) = find_in_path("explorer") {
        let _ = std::process::Command::new(path).arg(&url).spawn();
        return Ok(());
    }
    // 兜底：用系统默认浏览器
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("cmd");
        c.creation_flags(0x08000000);
        let _ = c.args(&["/c", "start", "", &url]).spawn();
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("open").arg(&url).spawn();
        Ok(())
    }
}

// ─── 依赖检测命令 ───

#[tauri::command]
pub fn npm_deps(project_id: String) -> Result<DepCheckResult, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    Ok(check_deps(&def))
}

fn check_deps(def: &NodeProjectDef) -> DepCheckResult {
    let git = check_dep("git", &["--version"]);
    let mut node = check_dep("node", &["--version"]);
    // node 版本约束
    if node.exists {
        let v = node.version.as_deref().unwrap_or("").trim_start_matches('v').trim().to_string();
        node.satisfies = version_satisfies(&v, &def.node_requirement);
        node.requirement = Some(def.node_requirement.clone());
    } else {
        node.satisfies = false;
        node.requirement = Some(def.node_requirement.clone());
    }
    // npx 模式：npm/npx 随 Node 分发，node 就绪即视为包管理器就绪；无需 git。
    let pm = if def.is_npx() {
        let mut p = check_dep("npm", &["--version"]);
        p.name = "npm".to_string();
        p.requirement = Some("随 node 提供".to_string());
        p.satisfies = node.exists && node.satisfies;
        p
    } else {
        check_dep(&def.package_manager, &["--version"])
    };
    let all_ready = if def.is_npx() {
        node.exists && node.satisfies
    } else {
        git.exists && node.exists && node.satisfies && pm.exists
    };
    DepCheckResult {
        git,
        node,
        package_manager: pm,
        all_ready,
    }
}

// ─── 状态命令 ───

#[tauri::command]
pub fn npm_status(project_id: String) -> Result<NodeProjectStatus, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    Ok(status_for(&def))
}

/// 更新检查结果：git 模式比较 commit；npx 模式比较本地已装版本与 npm registry 远程版本。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeUpdateInfo {
    pub has_update: bool,
    pub current_commit: String,
    pub latest_commit: String,
    pub behind: i64,
    /// npx 模式：本地已装版本号。
    #[serde(default)]
    pub current_version: Option<String>,
    /// npx 模式：npm registry 远程最新版本号。
    #[serde(default)]
    pub latest_version: Option<String>,
    pub error: Option<String>,
}

/// npx 模式读取本地已装版本：`node_modules/<包名>/package.json` 的 version 字段。
/// scoped 包会按 `@scope/name` 拆成多级目录；pnpm 符号链接也能读到目标文件。
fn npx_local_version(def: &NodeProjectDef) -> Option<String> {
    let pkg = def.npx_package.trim();
    if pkg.is_empty() {
        return None;
    }
    let mut rel = PathBuf::from("node_modules");
    for part in pkg.split('/') {
        rel.push(part);
    }
    let pkg_json = def.managed_dir().join(rel).join("package.json");
    let data = fs::read_to_string(pkg_json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v.get("version")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
}

/// 查询 npm registry 上包的最新版本：`npm view <pkg> version`。
/// 网络不可用 / npm 不在 PATH / 包不存在时返回 None。
fn npm_remote_version(pkg: &str) -> Option<String> {
    let (prog, prefix) =
        match resolve_exe_invocation("npm", "请先安装 Node.js (https://nodejs.org)") {
            Ok(x) => x,
            Err(_) => return None,
        };
    let mut args = prefix;
    args.push("view".to_string());
    args.push(pkg.to_string());
    args.push("version".to_string());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut cmd = hidden_cmd(&prog);
    cmd.args(&arg_refs);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty())
}

/// 简单 semver 比较：`version_gt(a, b)` 判断 a > b（数字段逐一比较，忽略前置 v）。
/// 无法解析的段按 0 处理；相等或 a < b 返回 false。
fn version_gt(a: &str, b: &str) -> bool {
    fn digit_parts(v: &str) -> Vec<i64> {
        v.trim_start_matches('v')
            .split(|c: char| c != '.' && !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<i64>().unwrap_or(0))
            .collect()
    }
    let (pa, pb) = (digit_parts(a), digit_parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (
            pa.get(i).copied().unwrap_or(0),
            pb.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

fn git_cmd(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(dir);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// 检查项目是否有新版本：git fetch 后比较本地 HEAD 与远端跟踪分支。
/// 返回是否落后、本地/远端 commit、落后提交数。
#[tauri::command]
pub fn npm_check_update(project_id: String) -> NodeUpdateInfo {
    let empty = |e: String| NodeUpdateInfo {
        has_update: false,
        current_commit: String::new(),
        latest_commit: String::new(),
        behind: 0,
        current_version: None,
        latest_version: None,
        error: Some(e),
    };
    let Some(def) = find_project(&project_id) else {
        return empty(format!("未找到项目: {}", project_id));
    };
    // npx 模式：比较本地已装版本与 npm registry 远程最新版本
    if def.is_npx() {
        let current = npx_local_version(&def);
        let latest = npm_remote_version(&def.npx_package.trim().to_string());
        let has_update = match (&current, &latest) {
            (Some(c), Some(l)) => c != l && version_gt(l, c),
            _ => false,
        };
        return NodeUpdateInfo {
            has_update,
            current_commit: String::new(),
            latest_commit: String::new(),
            behind: if has_update { 1 } else { 0 },
            current_version: current,
            latest_version: latest.clone(),
            error: latest
                .is_none()
                .then(|| "无法获取远程版本，请检查网络 / npm registry 可达性".to_string()),
        };
    }
    let dir = def.managed_dir();
    if !dir.exists() {
        return empty("项目尚未安装".to_string());
    }

    // 1. fetch 远端（失败不致命，可能离线，仍尝试用本地缓存跟踪分支判断）
    let fetch_result = git_cmd(&dir, &["fetch", "origin", "--prune"]);
    if let Err(e) = &fetch_result {
        eprintln!("[node_manager] check_update fetch 失败(忽略): {}", e);
    }

    // 2. 本地 HEAD
    let current = git_cmd(&dir, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();

    // 3. 远端跟踪分支（@{u} 优先，回退 origin/master / origin/main）
    let upstream = git_cmd(&dir, &["rev-parse", "--short", "@{u}"])
        .or_else(|_| git_cmd(&dir, &["rev-parse", "--short", "origin/master"]))
        .or_else(|_| git_cmd(&dir, &["rev-parse", "--short", "origin/main"]));

    let latest = match upstream {
        Ok(u) => u,
        Err(e) => return empty(e),
    };

    if current.is_empty() || latest.is_empty() {
        return empty("无法获取 git 提交信息".to_string());
    }

    // 4. 落后提交数
    let behind = git_cmd(&dir, &["rev-list", "--count", &format!("{}..{}", current, latest)])
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    NodeUpdateInfo {
        has_update: current != latest && behind > 0,
        current_commit: current,
        latest_commit: latest,
        behind,
        current_version: None,
        latest_version: None,
        error: None,
    }
}

fn status_for(def: &NodeProjectDef) -> NodeProjectStatus {
    let installed = def.installed();
    if !installed {
        return NodeProjectStatus {
            id: def.id.clone(),
            display_name: def.display_name.clone(),
            installed: false,
            status: "not_installed".to_string(),
            port: Some(def.default_port),
            pid: None,
            git_version: None,
            local_version: None,
            error: None,
        };
    }

    let git_version = command_version("git", &["-C", &def.managed_dir().to_string_lossy(), "rev-parse", "--short", "HEAD"]);
    let git_version = git_version.filter(|v| !v.is_empty());
    // npx 模式：从本地 node_modules 读取已装包版本号
    let local_version = if def.is_npx() {
        npx_local_version(def)
    } else {
        None
    };

    // 运行状态：端口被我们的进程占用 → running；被其他进程占用 → port_conflict
    let mut status = "stopped".to_string();
    let mut pid = None;
    if let Some(p) = crate::commands::utils::port_owner_pid(def.default_port) {
        let recorded = recorded_pid(&def.id);
        if recorded == Some(p) || recorded.is_some() {
            pid = Some(p);
            status = "running".to_string();
        } else if recorded.is_some() {
            // 记录过 PID 但端口换了归属，仍视为 running（按记录 pid 判定）
            pid = recorded;
            status = "running".to_string();
        } else {
            status = "port_conflict".to_string();
            pid = Some(p);
        }
    } else if let Some(p) = recorded_pid(&def.id) {
        // 进程被本应用 spawn 但端口未监听（启动慢/异常）
        pid = Some(p);
        status = "running".to_string();
    }

    NodeProjectStatus {
        id: def.id.clone(),
        display_name: def.display_name.clone(),
        installed: true,
        status,
        port: Some(def.default_port),
        pid,
        git_version,
        local_version,
        error: None,
    }
}

#[tauri::command]
pub fn npm_list_projects() -> Vec<NodeProjectDef> {
    registry()
}

/// 返回当前服务类项目存储目录（供前端「设置」展示）。
#[tauri::command]
pub fn get_node_projects_dir() -> String {
    crate::commands::config::get_node_projects_dir()
        .to_string_lossy()
        .to_string()
}

/// 更新服务类项目存储目录，并把旧目录下已安装的项目移动到新目录。
/// 返回迁移结果（移动失败的项目列表）。
#[tauri::command]
pub fn update_node_projects_dir(new_dir: String) -> Result<Vec<String>, String> {
    use crate::commands::config::{get_node_projects_dir, load_config, save_config};

    let new_dir = new_dir.trim().to_string();
    if new_dir.is_empty() {
        return Err("服务类项目存储路径不能为空".to_string());
    }
    let old_dir = get_node_projects_dir();
    let new_path = Path::new(&new_dir);

    // 迁移：把旧目录下每个已存在的子项目移到新目录
    let mut failures: Vec<String> = Vec::new();
    if old_dir.exists() && old_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&old_dir) {
            for entry in entries.flatten() {
                let src = entry.path();
                if !src.is_dir() {
                    continue;
                }
                let name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
                let dst = new_path.join(&name);
                if dst.exists() {
                    failures.push(format!("{} 已存在于新目录，跳过", name));
                    continue;
                }
                if let Some(parent) = dst.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Err(e) = fs::rename(&src, &dst) {
                    eprintln!("[node_manager] 迁移项目 {} 失败: {}", name, e);
                    failures.push(format!("{} 迁移失败: {}", name, e));
                } else {
                    eprintln!("[node_manager] 已迁移项目 {} -> {}", src.display(), dst.display());
                }
            }
        }
    }

    // 保存新配置
    let mut config = load_config();
    config.node_projects_dir = new_path.to_string_lossy().to_string();
    save_config(&config)?;

    Ok(failures)
}

// ─── 单元测试 ───

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_satisfies() {
        assert!(version_satisfies("20.11.0", ">=20"));
        assert!(version_satisfies("22.0.0", ">=20"));
        assert!(!version_satisfies("18.0.0", ">=20"));
        assert!(version_satisfies("18.19.0", ">=18"));
        assert!(!version_satisfies("17.0.0", ">=18"));
        assert!(version_satisfies("20.0.0", ""));
        assert!(version_satisfies("21.0.0", ">20"));
        assert!(!version_satisfies("20.0.0", ">20"));
        assert!(version_satisfies("18.0.0", "<=18"));
        assert!(version_satisfies("20.5.0", "20"));
    }

    #[test]
    fn test_web_path_render() {
        let def = NodeProjectDef {
            id: "harness".into(),
            display_name: "t".into(),
            repo: String::new(),
            website: String::new(),
            icon: String::new(),
            description: String::new(),
            default_port: 3080,
            web_path: "http://127.0.0.1:{port}".into(),
            node_requirement: String::new(),
            package_manager: "pnpm".into(),
            build_script: String::new(),
            start_cmd: Vec::new(),
            managed: true,
            npx_package: String::new(),
            npx_bin: String::new(),
        };
        assert_eq!(def.resolved_web_path(), "http://127.0.0.1:3080");
    }

    #[test]
    fn test_parse_version_parts() {
        assert_eq!(parse_version_parts("20.11.0"), vec![20, 11, 0]);
        assert_eq!(parse_version_parts("v22"), vec![22]);
        assert_eq!(parse_version_parts("0"), vec![0]);
    }

    fn npx_def(pkg: &str, bin: &str) -> NodeProjectDef {
        NodeProjectDef {
            id: "harness".into(),
            display_name: "t".into(),
            repo: String::new(),
            website: String::new(),
            icon: String::new(),
            description: String::new(),
            default_port: 3080,
            web_path: "http://127.0.0.1:{port}".into(),
            node_requirement: String::new(),
            package_manager: String::new(),
            build_script: String::new(),
            start_cmd: Vec::new(),
            managed: true,
            npx_package: pkg.into(),
            npx_bin: bin.into(),
        }
    }

    #[test]
    fn test_npx_bin_name() {
        assert_eq!(npx_def("cowsay", "").npx_bin_name(), "cowsay");
        assert_eq!(npx_def("@scope/name", "").npx_bin_name(), "name");
        assert_eq!(npx_def("@scope/name", "custom-bin").npx_bin_name(), "custom-bin");
        assert_eq!(npx_def("deepseek-harness", "dsh").npx_bin_name(), "dsh");
    }

    #[test]
    fn test_strip_pkg_version() {
        assert_eq!(strip_pkg_version("cowsay"), "cowsay");
        assert_eq!(strip_pkg_version("cowsay@1.5.0"), "cowsay");
        assert_eq!(strip_pkg_version("@scope/name"), "@scope/name");
        assert_eq!(strip_pkg_version("@scope/name@0.0.1-rc.1"), "@scope/name");
    }

    #[test]
    fn test_npx_installed_path() {
        // 无实际安装时不视为已安装
        assert!(!npx_def("cowsay", "").installed());
        assert!(!npx_def("", "").is_npx());
        assert!(npx_def("@scope/name", "").is_npx());
    }
}
