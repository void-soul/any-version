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
    /// Python 项目复用为 python 版本约束（如 ">=3.8"）。
    #[serde(default)]
    pub node_requirement: String,
    /// 运行时类型：空 / "node" = Node（pnpm/npm/yarn + node_modules）；"python" = Python
    /// （托管目录内创建 .venv，pip 安装 requirements.txt，startCmd 作为 venv python 的参数）。
    #[serde(default)]
    pub runtime: String,
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
    /// pip 包模式：PyPI 包名（可带 extras，如 `headroom-ai[proxy]`）。非空时改用 pip 流程，
    /// 无需 git clone：安装/升级 = `pip install [--upgrade] [--target .deps] <包名>`
    /// （venv 可用时优先装进 .venv，否则复用 .deps 降级方案），
    /// 启动 = `<python> -m {pip_module} {start_cmd...}`。
    #[serde(default)]
    pub pip_package: String,
    /// pip 包模式的附加依赖（随主包一次 pip install）。
    /// 典型：headroom 的 proxy 在 Windows 依赖 pywintypes（pywin32），
    /// 但 `headroom-ai[proxy]` extra 未声明该 Windows 依赖，需在此补装。
    #[serde(default)]
    pub pip_extra_packages: Vec<String>,
    /// pip 包模式下的 `-m` 模块路径（如 `headroom.cli`）。
    #[serde(default)]
    pub pip_module: String,
    /// 启动时注入的环境变量（如 `HEADROOM_BEACON=off`、`HEADROOM_DISABLE_KOMPRESS=1`）。
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 启动后从控制台输出（stdout/stderr）提取「带凭据主页地址」的正则。
    /// 第 1 捕获组 = URL（如 dsh: `"dsh web: (http\\S+)"`）。
    /// 为空表示不提取。适用于启动时在控制台打印带 token 认证地址、
    /// 且该地址无法用普通 webPath 直接访问的服务（iframe 会被鉴权拒绝）。
    #[serde(default)]
    pub console_url_pattern: String,
    /// 提取到控制台 URL 后是否自动用系统默认浏览器打开。
    #[serde(default)]
    pub auto_open_console_url: bool,
}

impl NodeProjectDef {
    /// 是否启用控制台 URL 提取。
    pub fn has_console_url_pattern(&self) -> bool {
        !self.console_url_pattern.trim().is_empty()
    }
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

    /// 是否为 Python 运行时（runtime = "python"）：venv + pip，不走 pnpm/npm。
    pub fn is_python(&self) -> bool {
        self.runtime.trim().eq_ignore_ascii_case("python")
    }

    /// 是否为 pip 包模式（配置了 pipPackage）：直接从 PyPI 安装，无需 git clone。
    pub fn is_pip_package(&self) -> bool {
        !self.pip_package.trim().is_empty()
    }

    /// pip 包模式的安装标记文件（记录已安装的包名，供 installed() 判定）。
    pub fn pip_marker_path(&self) -> PathBuf {
        self.managed_dir().join(".pip-package.json")
    }

    /// venv 内的 python 解释器路径（Windows: `.venv/Scripts/python.exe`，其余: `.venv/bin/python`）。
    pub fn venv_python(&self) -> PathBuf {
        self.managed_dir()
            .join(".venv")
            .join(if cfg!(windows) { "Scripts" } else { "bin" })
            .join(if cfg!(windows) { "python.exe" } else { "python" })
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

    /// npx 模式的干净运行目录：不复用可能是 Git workspace 的项目根目录。
    ///
    /// npm 会沿 `--prefix` 向上读取 package.json；如果直接把托管目录作为 prefix，
    /// 旧版 Git 安装留下的 workspace:^ 依赖会再次被 npm 解析。专用子目录只放发布包，
    /// 从根上隔离这种 workspace 元数据。
    pub fn npx_runtime_dir(&self) -> PathBuf {
        self.managed_dir().join(".npx-runtime")
    }

    /// npx 模式下判断包是否已安装到干净运行目录。
    pub fn npx_installed(&self) -> bool {
        let pkg = self.npx_package.trim();
        if pkg.is_empty() {
            return false;
        }
        let mut rel = PathBuf::from("node_modules");
        for part in pkg.split('/') {
            rel.push(part);
        }
        self.npx_runtime_dir().join(rel).exists()
    }

    /// 判断是否已安装：git 模式看 package.json；npx 模式看 node_modules 里的包；
    /// pip 包模式看安装标记 `.pip-package.json`；Python git 项目看 .venv / .deps 运行时。
    pub fn installed(&self) -> bool {
        self.installed_in(&self.managed_dir())
    }

    /// [`Self::installed`] 的可注入目录版本（便于测试）。
    pub(crate) fn installed_in(&self, dir: &Path) -> bool {
        if self.is_npx() {
            return self.npx_installed();
        }
        if self.is_pip_package() {
            return self.pip_marker_path().exists();
        }
        if self.is_python() {
            // Python 项目没有 package.json：venv 解释器或 .deps 运行时标记（Q-0100 降级方案）
            // 任一存在即视为已安装。
            let venv_py = dir
                .join(".venv")
                .join(if cfg!(windows) { "Scripts" } else { "bin" })
                .join(if cfg!(windows) { "python.exe" } else { "python" });
            return venv_py.exists() || dir.join(".deps").join(".python-runtime.json").exists();
        }
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
    /// port_conflict 时占用进程名（前端明确展示「端口 X 被 <proc> 占用」）。
    #[serde(default)]
    pub conflict_process: Option<String>,
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
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return;
    }
    let _ = app.emit(
        "npm-log",
        NodeLog {
            project_id: project_id.to_string(),
            phase: phase.to_string(),
            line: line.to_string(),
        },
    );
}

// ─── 控制台 URL 捕获（带凭据主页地址）───

/// 控制台 URL 事件：从启动输出中捕获到带凭据地址时通知前端。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConsoleUrl {
    pub project_id: String,
    pub url: String,
}

fn emit_console_url(app: &tauri::AppHandle, project_id: &str, url: &str) {
    let _ = app.emit(
        "npm-console-url",
        NodeConsoleUrl {
            project_id: project_id.to_string(),
            url: url.to_string(),
        },
    );
}

/// 各项目最近一次捕获的控制台 URL（进程级状态；服务重启后由新输出覆盖）。
static CONSOLE_URLS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn console_urls() -> std::sync::MutexGuard<'static, Option<HashMap<String, String>>> {
    CONSOLE_URLS.lock().unwrap()
}

/// 单锁内完成「与已捕获值比较 + 覆盖记录」，返回是否发生变化。
/// stdout/stderr 两个读取线程可能并发命中同一 URL，原子比较避免重复自动打开浏览器。
fn record_console_url_if_changed(project_id: &str, url: &str) -> bool {
    let mut g = console_urls();
    let m = g.get_or_insert_with(HashMap::new);
    let changed = m.get(project_id).map(String::as_str) != Some(url);
    m.insert(project_id.to_string(), url.to_string());
    changed
}

/// 控制台 URL 的持久化文件（项目托管目录内）：应用重启后仍能取回最近捕获的地址，
/// 避免「服务在跑、主页按钮却提示未捕获/回落到无 token 地址」。
fn console_url_file_path(project_id: &str) -> Option<PathBuf> {
    find_project(project_id).map(|d| d.managed_dir().join(".console-url.json"))
}

/// 持久化最近捕获的控制台 URL。
fn persist_console_url(project_id: &str, url: &str) {
    if let Some(path) = console_url_file_path(project_id) {
        let data = serde_json::json!({ "url": url }).to_string();
        let _ = fs::write(&path, data);
    }
}

/// 读取某项目最近捕获的控制台 URL：内存优先，回落到持久化文件。
fn console_url_of(project_id: &str) -> Option<String> {
    if let Some(u) = console_urls().as_ref().and_then(|m| m.get(project_id).cloned()) {
        return Some(u);
    }
    let path = console_url_file_path(project_id)?;
    let raw = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value
        .get("url")
        .and_then(|v| v.as_str())
        .map(String::from)
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
}

/// 清除某项目捕获的控制台 URL（停止/卸载时调用；内存 + 持久化文件一并清除）。
fn clear_console_url(project_id: &str) {
    if let Some(m) = console_urls().as_mut() {
        m.remove(project_id);
    }
    if let Some(path) = console_url_file_path(project_id) {
        let _ = fs::remove_file(path);
    }
}

/// 逐行读取子进程输出流（**按字节、lossy 解码**）。
///
/// 禁止用 `BufRead::lines()`：它按 UTF-8 解码，Windows 上子进程往管道写
/// locale 编码（GBK）时，中文/emoji 是非法 UTF-8 字节 → `lines()` 返回 Err →
/// reader 线程静默退出并关闭管道读端 → 子进程写 stderr 变 broken pipe →
/// 退出时 flush 失败（**exit code 120**）且尾部日志全部丢失。
/// 字节读取 + `from_utf8_lossy` 永不因编码断流。
fn read_child_stream<R: std::io::Read>(
    stream: &mut R,
    app: &tauri::AppHandle,
    project_id: &str,
    level: &str,
    pattern: &str,
    auto_open: bool,
) {
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(stream);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\r', '\n']);
                maybe_capture_console_url(app, project_id, pattern, auto_open, line);
                emit_log(app, project_id, level, line);
            }
        }
    }
}

/// Python 启动参数统一加 `-X utf8`：让子进程的 stdio 强制 UTF-8，
/// 否则 Windows 管道默认 locale 编码（GBK），日志在 Rust 侧显示为乱码。
/// （嵌入式 Python 的 ._pth 隔离模式忽略 PYTHONUTF8 等环境变量，只能用 -X。）
pub(crate) fn with_utf8_flag(mut args: Vec<String>) -> Vec<String> {
    let mut out = vec!["-X".to_string(), "utf8".to_string()];
    out.append(&mut args);
    out
}

/// 在控制台输出行中提取捕获组 URL；无 pattern 或不匹配返回 None。
fn extract_console_url(pattern: &str, line: &str) -> Option<String> {
    let re = regex::Regex::new(pattern).ok()?;
    let m = re.captures(line)?;
    let url = m.get(1)?.as_str().trim().trim_end_matches([')', ';', ',']);
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else {
        None
    }
}

/// 尝试从一行控制台输出中捕获带凭据主页地址。
/// pattern 为空时直接返回；捕获到则覆盖记录、通知前端，并按配置自动打开。
/// 同一行最多命中一次；同一次输出的重复行以后出现的为准（服务重启后 token 会更新）。
fn maybe_capture_console_url(
    app: &tauri::AppHandle,
    project_id: &str,
    pattern: &str,
    auto_open: bool,
    line: &str,
) {
    if pattern.trim().is_empty() {
        return;
    }
    let Some(url) = extract_console_url(pattern, line) else {
        return;
    };
    // 原子「比较+记录」：仅当地址变化（服务重启后 token 更新）时自动打开，
    // stdout/stderr 并发命中同一行不会重复打开浏览器。
    let changed = record_console_url_if_changed(project_id, &url);
    persist_console_url(project_id, &url);
    emit_console_url(app, project_id, &url);
    if changed && auto_open {
        open_url_in_browser(&url);
    }
}

/// 用系统默认浏览器打开 URL。
///
/// Windows 上**不要**用 `explorer.exe <url>`：explorer 对带 query（如 `?token=...`）
/// 的 URL 处理不可靠，常退化成打开「此电脑」/资源管理器。首选 `rundll32 url.dll,
/// FileProtocolHandler`（无窗口、URL 语义交给默认浏览器），回退 `cmd /c start`。
fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if let Some(rundll) = find_in_path("rundll32") {
            let _ = std::process::Command::new(rundll)
                .args(["url.dll,FileProtocolHandler", url])
                .creation_flags(0x08000000)
                .spawn();
            return;
        }
        let mut c = std::process::Command::new("cmd");
        c.creation_flags(0x08000000);
        let _ = c.args(&["/c", "start", "", url]).spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

fn format_command(program: &str, args: &[&str]) -> String {
    let mut command = program.to_string();
    for arg in args {
        if arg.is_empty() || arg.chars().any(|c| c.is_whitespace() || c == '"') {
            command.push(' ');
            command.push('"');
            command.push_str(&arg.replace('"', "\\\""));
            command.push('"');
        } else {
            command.push(' ');
            command.push_str(arg);
        }
    }
    command
}

/// 按换行或回车分隔实时读取命令输出。npm 的进度渲染通常只使用 `\\r`，
/// `BufRead::lines()` 会一直等到 `\\n`，导致前端看起来像卡住；这里两种分隔符都立即转发。
/// 返回 (最后一行, 完整输出)。完整输出用于判断命令是否只是「警告性」失败
/// （如 pnpm 跳过构建脚本），仅看最后一行会漏掉真正的错误码。
fn emit_reader_live<R: std::io::Read>(
    reader: R,
    app: &tauri::AppHandle,
    project_id: &str,
    phase: &str,
) -> (String, String) {
    use std::io::Read;

    let mut reader = std::io::BufReader::new(reader);
    let mut bytes = [0u8; 4096];
    let mut pending = Vec::new();
    let mut last = String::new();
    let mut full = String::new();
    let mut previous_was_cr = false;

    loop {
        let count = match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };
        for byte in &bytes[..count] {
            if *byte == b'\r' || *byte == b'\n' {
                // CRLF 只作为一条记录；单独的 CR（npm 进度条）也要立即发送。
                if !pending.is_empty() {
                    let line = String::from_utf8_lossy(&pending).to_string();
                    if !line.trim().is_empty() {
                        last = line.clone();
                        full.push_str(&line);
                        full.push('\n');
                        emit_log(app, project_id, phase, &line);
                    }
                    pending.clear();
                }
                previous_was_cr = *byte == b'\r';
            } else {
                // 某些程序会在 CR 后紧接新内容；previous_was_cr 仅用于说明分隔已处理。
                previous_was_cr = false;
                pending.push(*byte);
            }
        }
    }
    if !pending.is_empty() {
        let line = String::from_utf8_lossy(&pending).to_string();
        if !line.trim().is_empty() {
            last = line.clone();
            full.push_str(&line);
            full.push('\n');
            emit_log(app, project_id, phase, &line);
        }
    }
    let _ = previous_was_cr;
    (last, full)
}

/// 杀掉子进程及其整棵进程树（npm/npx 经 `cmd /c` 启动，真正的 node 进程是孙进程，
/// 仅 `child.kill()` 会留下孤儿的 npm/node 进程继续占用网络与文件）。
fn kill_process_tree(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    #[cfg(windows)]
    {
        let _ = run_capture("taskkill", &["/f", "/t", "/pid", &pid], None);
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
}

/// 执行命令并把 stdout/stderr 实时 emit（支持 newline/CR 分隔）。
/// 返回 (是否成功, 末尾错误行, stdout+stderr 完整输出)。
/// `timeout`：总超时；超时后杀掉整棵进程树并返回失败（npm 网络请求挂死时避免永久转圈）。
fn run_capture_live(
    app: &tauri::AppHandle,
    project_id: &str,
    phase: &str,
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
    timeout: Option<Duration>,
) -> (bool, String, String) {
    let command = format_command(program, args);
    emit_log(
        app,
        project_id,
        phase,
        &format!("$ {}", command),
    );
    if let Some(dir) = cwd {
        emit_log(app, project_id, phase, &format!("工作目录: {}", dir.display()));
    }

    let mut cmd = hidden_cmd(program);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    run_cmd_live(app, project_id, phase, &command, cmd, timeout)
}

/// 执行调用方预构建的 `Command`（npm_exec 等需要 raw_arg / 自定义 env 的场景）。
/// 与 run_capture_live 相同：实时转发 stdout/stderr、心跳、超时杀进程树。
fn run_cmd_live(
    app: &tauri::AppHandle,
    project_id: &str,
    phase: &str,
    display_command: &str,
    mut cmd: std::process::Command,
    timeout: Option<Duration>,
) -> (bool, String, String) {
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_log(app, project_id, phase, &format!("无法执行命令: {}", e));
            let msg = format!("无法执行命令: {}", e);
            return (false, msg.clone(), msg);
        }
    };

    let stdout_thread = child.stdout.take().map(|so| {
        let app_out = app.clone();
        let pid = project_id.to_string();
        let ph = phase.to_string();
        std::thread::spawn(move || emit_reader_live(so, &app_out, &pid, &ph))
    });
    let stderr_thread = child.stderr.take().map(|se| {
        let app_err = app.clone();
        let pid = project_id.to_string();
        let ph = phase.to_string();
        std::thread::spawn(move || emit_reader_live(se, &app_err, &pid, &ph))
    });

    // npm 在解析依赖、下载大文件时可能暂时没有任何输出；定期发送心跳，
    // 让服务面板明确知道进程仍在运行，而不是看起来像卡死。
    let started_at = Instant::now();
    let mut last_report = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if let Some(t) = timeout {
                    if started_at.elapsed() >= t {
                        emit_log(
                            app,
                            project_id,
                            phase,
                            &format!("命令超时（{} 秒），终止进程树…", t.as_secs()),
                        );
                        kill_process_tree(&mut child);
                        timed_out = true;
                        break child.wait();
                    }
                }
                if last_report.elapsed() >= Duration::from_secs(5) {
                    emit_progress(
                        app,
                        project_id,
                        phase,
                        &format!(
                            "命令仍在运行…已用时 {} 秒：{}",
                            started_at.elapsed().as_secs(),
                            display_command
                        ),
                    );
                    last_report = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => break Err(error),
        }
    };
    let stdout_all = stdout_thread
        .and_then(|thread| thread.join().ok())
        .unwrap_or((String::new(), String::new()));
    let stderr_all = stderr_thread
        .and_then(|thread| thread.join().ok())
        .unwrap_or((String::new(), String::new()));
    let mut full = stdout_all.1;
    full.push_str(&stderr_all.1);
    let last_err = if stderr_all.0.trim().is_empty() {
        stdout_all.0
    } else {
        stderr_all.0
    };
    if timed_out {
        let secs = timeout.map(|t| t.as_secs()).unwrap_or(0);
        let msg = format!("命令超时（{} 秒）已终止，请检查网络后重试", secs);
        return (false, msg.clone(), full);
    }
    let ok = matches!(status, Ok(s) if s.success());
    (ok, last_err, full)
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
    // Python 项目不走 pnpm/npm：requirements.txt 是唯一依赖清单，用 venv + pip 安装。
    if def.is_python() {
        return python_install(app, def, dir);
    }
    let pm = &def.package_manager;
    let (prog, prefix) = resolve_pm_invocation(def)?;
    emit_progress(app, &def.id, "install", &format!("正在运行 `{} install`（首次可能较慢）…", pm));
    let mut args = prefix;
    args.push("install".to_string());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err, _out) = run_capture_live(app, &def.id, "install", &prog, &arg_refs, Some(dir), &[], None);
    if !ok {
        let msg = last_err.trim();
        return Err(format!("`{} install` 失败: {}", pm, if msg.is_empty() { "未知错误" } else { msg }));
    }
    Ok(())
}

/// Python 解释器候选（按优先级）：`py -3`（官方启动器）→ `python3` → `python`。
/// 不再无脑用裸 `python`：PATH 上的可能是嵌入式发行版（无 venv 模块），
/// 如本应用 SDK 管理器安装的 embeddable Python。
fn python_candidates() -> Vec<(String, Vec<String>)> {
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    if cfg!(windows) {
        candidates.push(("py".to_string(), vec!["-3".to_string()]));
    }
    candidates.push(("python3".to_string(), Vec::new()));
    candidates.push(("python".to_string(), Vec::new()));
    candidates
}

/// 探测解释器能否导入某模块（`<py> -c "import xxx"`）。
fn probe_python_module(program: &str, prefix: &[String], code: &str) -> bool {
    let mut cmd = hidden_cmd(program);
    for a in prefix {
        cmd.arg(a);
    }
    cmd.args(["-c", code]);
    matches!(cmd.output(), Ok(out) if out.status.success())
}

/// 探测所有候选解释器，返回 (program, prefix, has_venv, has_pip) 列表。
fn probe_python_candidates() -> Vec<(String, Vec<String>, bool, bool)> {
    python_candidates()
        .into_iter()
        .map(|(program, prefix)| {
            let has_venv = probe_python_module(&program, &prefix, "import venv, ensurepip");
            let has_pip = has_venv || probe_python_module(&program, &prefix, "import pip");
            (program, prefix, has_venv, has_pip)
        })
        .collect()
}

/// Python 运行时安装方案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PythonRuntimePlan {
    /// 用该解释器创建 .venv（program + 前缀参数，如 py + ["-3"]）
    Venv(String, Vec<String>),
    /// 解释器无 venv 模块但可用 pip → 降级 `pip install --target .deps`
    TargetDeps(String, Vec<String>),
    /// 无可用解释器
    None,
}

/// 纯函数：venv 优先，其次 pip-only 降级，最后 None。
pub(crate) fn select_python_runtime(
    probes: &[(String, Vec<String>, bool, bool)],
) -> PythonRuntimePlan {
    for (program, prefix, has_venv, _has_pip) in probes {
        if *has_venv {
            return PythonRuntimePlan::Venv(program.clone(), prefix.clone());
        }
    }
    for (program, prefix, has_venv, has_pip) in probes {
        if !*has_venv && *has_pip {
            return PythonRuntimePlan::TargetDeps(program.clone(), prefix.clone());
        }
    }
    PythonRuntimePlan::None
}

/// Python 项目启动计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PythonLaunchPlan {
    /// 使用 .venv 内解释器
    Venv(PathBuf),
    /// 使用基准解释器 + .deps 依赖目录（PYTHONPATH / ._pth 注入）
    Deps {
        program: String,
        prefix: Vec<String>,
        deps_dir: PathBuf,
    },
    /// 运行时未就绪
    Missing,
}

/// 纯函数（仅读文件系统）：优先 .venv，其次读 `.deps/.python-runtime.json` 标记。
/// marker 里的解释器必须真实存在，否则按 Missing 处理（启动必然失败，应提示重装）。
pub(crate) fn plan_python_launch(managed_dir: &Path, venv_python: &Path) -> PythonLaunchPlan {
    if venv_python.exists() {
        return PythonLaunchPlan::Venv(venv_python.to_path_buf());
    }
    let deps_dir = managed_dir.join(".deps");
    let Ok(raw) = fs::read_to_string(deps_dir.join(".python-runtime.json")) else {
        return PythonLaunchPlan::Missing;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return PythonLaunchPlan::Missing;
    };
    let program = value
        .get("program")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if program.is_empty() || !Path::new(&program).exists() {
        return PythonLaunchPlan::Missing;
    }
    let prefix = value
        .get("prefix")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    PythonLaunchPlan::Deps {
        program,
        prefix,
        deps_dir,
    }
}

/// `.deps` 模式下 `--target` 安装的 pywin32 需要显式加入 sys.path 的子目录：
/// 模块在 `win32/`（pywintypes 等）与 `win32/lib/`，DLL 在 `pywin32_system32/`，
/// 都不在 `.deps` 顶层，裸 PYTHONPATH=.deps 导入不了 pywintypes。
/// 只返回真实存在的子目录（未安装 pywin32 的项目返回空）。
pub(crate) fn pywin32_deps_dirs(deps_dir: &Path) -> Vec<PathBuf> {
    ["win32", "win32/lib", "pywin32_system32"]
        .iter()
        .map(|rel| deps_dir.join(rel))
        .filter(|p| p.exists())
        .collect()
}

/// `.deps` 模式启动的 PYTHONPATH（纯函数，便于测试）：
/// .deps（pip --target 依赖）+ 项目源码目录（`-m core.converter` 的包所在，
/// 嵌入式 Python 不会自动把 cwd 加进 sys.path）+ pywin32 子目录。
pub(crate) fn deps_pythonpath(project_dir: &Path, deps_dir: &Path) -> String {
    let mut paths = vec![deps_dir.to_path_buf(), project_dir.to_path_buf()];
    paths.extend(pywin32_deps_dirs(deps_dir));
    let sep = if cfg!(windows) { ";" } else { ":" };
    paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(sep)
}

/// 嵌入式 Python（带 `*._pth`）会忽略 PYTHONPATH；
/// 把 .deps / 项目源码目录 / pywin32 子目录追加进 `._pth` 才能让它 import。
/// 只在解释器目录存在 `*._pth` 时生效（官方完整安装没有这个文件，走 PYTHONPATH 即可）。
fn append_deps_to_embeddable_pth(interpreter: &str, deps_dir: &Path, project_dir: Option<&Path>) {
    let path = Path::new(interpreter);
    let Some(dir) = path.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with("._pth") {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let mut lines: Vec<String> = Vec::new();
        lines.push(deps_dir.to_string_lossy().to_string());
        if let Some(pd) = project_dir {
            if pd != deps_dir {
                lines.push(pd.to_string_lossy().to_string());
            }
        }
        lines.extend(pywin32_deps_dirs(deps_dir).into_iter().map(|p| p.to_string_lossy().to_string()));
        // 已全部追加过则不动
        if lines.iter().all(|l| content.lines().any(|x| x.trim().eq_ignore_ascii_case(l))) {
            return;
        }
        lines.retain(|l| !content.lines().any(|x| x.trim().eq_ignore_ascii_case(l)));
        let mut out = content.clone();
        if !out.ends_with('\n') {
            out.push_str("\r\n");
        }
        for l in &lines {
            out.push_str(l);
            out.push_str("\r\n");
        }
        let _ = fs::write(entry.path(), out);
        eprintln!("[node_manager] 已把 .deps（含项目目录/pywin32 子目录）追加到 {}（嵌入式 Python 依赖可见性）", entry.path().display());
        return;
    }
}

/// 把降级方案写盘：marker（启动时读回）+ 嵌入式 ._pth 追加（.deps + 项目源码目录 + pywin32 子目录）。
fn record_deps_runtime(dir: &Path, program: &str, prefix: &[String]) -> Result<(), String> {
    let deps = dir.join(".deps");
    let resolved = find_in_path(program)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| program.to_string());
    let marker = serde_json::json!({ "program": resolved, "prefix": prefix });
    fs::write(deps.join(".python-runtime.json"), marker.to_string())
        .map_err(|e| format!("写入 .deps 运行时标记失败: {}", e))?;
    append_deps_to_embeddable_pth(&resolved, &deps, Some(dir));
    Ok(())
}

/// 纯函数：pip 安装参数（spec 可带 extras，如 `headroom-ai[proxy]`）。
/// `extras` 为项目声明的附加依赖（如 pywin32），与主包同一条命令安装，保证环境一致。
/// `target` 为 Some 时走 `--target`（嵌入式 Python 的 .deps 降级方案），否则装进当前环境（venv）。
pub(crate) fn pip_install_args(spec: &str, extras: &[String], target: Option<&str>, upgrade: bool) -> Vec<String> {
    let mut args = vec!["-m".to_string(), "pip".to_string(), "install".to_string()];
    if upgrade {
        args.push("--upgrade".to_string());
    }
    if let Some(dir) = target {
        args.push("--target".to_string());
        args.push(dir.to_string());
    }
    args.push(spec.to_string());
    args.extend(extras.iter().filter(|s| !s.trim().is_empty()).cloned());
    args
}

/// 纯函数：pip 包模式的启动参数 = `-m <module> <startCmd...>`。
pub(crate) fn pip_launch_args(module: &str, start_cmd: &[String]) -> Vec<String> {
    let mut args = vec!["-m".to_string(), module.to_string()];
    args.extend(start_cmd.iter().cloned());
    args
}

/// pip 包模式安装/升级：直接从 PyPI 安装（可带 extras），无需 git clone。
///
/// 解释器选择复用 [`select_python_runtime`]：支持 venv 就建 `.venv` 装进去（隔离、卸载即删目录）；
/// 只有嵌入式 Python（无 venv 但有 pip）时降级 `--target .deps`，并把解释器/依赖路径记录给启动侧。
fn pip_package_install(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    dir: &Path,
    upgrade: bool,
) -> Result<(), String> {
    let spec = def.pip_package.trim().to_string();
    if spec.is_empty() {
        return Err("该项目未配置 pipPackage".to_string());
    }
    fs::create_dir_all(dir).map_err(|e| format!("创建托管目录失败: {}", e))?;

    emit_progress(app, &def.id, "install", "正在检测可用的 Python 解释器…");
    let probes = probe_python_candidates();
    match select_python_runtime(&probes) {
        PythonRuntimePlan::Venv(program, prefix) => {
            let venv_py = def.venv_python();
            if !venv_py.exists() {
                emit_progress(app, &def.id, "install", "正在创建 Python 虚拟环境 (.venv)…");
                let mut args = prefix.clone();
                args.extend(["-m".to_string(), "venv".to_string(), ".venv".to_string()]);
                let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let (ok, last_err, _out) = run_capture_live(
                    app, &def.id, "install", &program, &arg_refs, Some(dir), &[], None,
                );
                if !ok {
                    let msg = last_err.trim();
                    return Err(format!(
                        "创建虚拟环境失败: {}",
                        if msg.is_empty() { "未知错误" } else { msg }
                    ));
                }
            }
            if !venv_py.exists() {
                return Err("虚拟环境创建后未找到 python 解释器".to_string());
            }
            emit_progress(app, &def.id, "install", &format!("正在 pip install {} …", spec));
            let py = venv_py.to_string_lossy().to_string();
            let args = pip_install_args(&spec, &def.pip_extra_packages, None, upgrade);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) =
                run_capture_live(app, &def.id, "install", &py, &arg_refs, Some(dir), &[], None);
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
        }
        PythonRuntimePlan::TargetDeps(program, prefix) => {
            emit_progress(
                app,
                &def.id,
                "install",
                &format!(
                    "当前 Python（{}）不带 venv 模块（常见于嵌入式发行版），改用 .deps 目录安装…",
                    program
                ),
            );
            let mut args = prefix.clone();
            args.extend(pip_install_args(&spec, &def.pip_extra_packages, Some(".deps"), upgrade));
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) =
                run_capture_live(app, &def.id, "install", &program, &arg_refs, Some(dir), &[], None);
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            record_deps_runtime(dir, &program, &prefix)?;
        }
        PythonRuntimePlan::None => {
            return Err(
                "未找到支持 venv 或 pip 的 Python。请安装官方 Python 3.8+（python.org），\
                 或在本应用 SDK 页为托管 Python 执行 install_pip 后重试"
                    .to_string(),
            )
        }
    }

    let marker = serde_json::json!({ "package": spec });
    fs::write(def.pip_marker_path(), marker.to_string())
        .map_err(|e| format!("写入安装标记失败: {}", e))?;
    Ok(())
}

/// Python 项目安装。
///
/// 首选 `venv`（隔离、卸载即删目录）；解释器不带 venv 模块时（典型：嵌入式/embeddable
/// 发行版，如 SDK 管理器装的 Python），降级为 `pip install --target .deps`，
/// 启动时通过 PYTHONPATH / ._pth 注入依赖路径。
fn python_install(app: &tauri::AppHandle, def: &NodeProjectDef, dir: &Path) -> Result<(), String> {
    emit_progress(app, &def.id, "install", "正在检测可用的 Python 解释器…");
    let probes = probe_python_candidates();
    match select_python_runtime(&probes) {
        PythonRuntimePlan::Venv(program, prefix) => {
            let mut args = prefix.clone();
            args.extend(["-m".to_string(), "venv".to_string(), ".venv".to_string()]);
            emit_progress(app, &def.id, "install", "正在创建 Python 虚拟环境 (.venv)…");
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) =
                run_capture_live(app, &def.id, "install", &program, &arg_refs, Some(dir), &[], None);
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "创建虚拟环境失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            let py = def.venv_python();
            if !py.exists() {
                return Err("虚拟环境创建后未找到 python 解释器".to_string());
            }
            emit_progress(app, &def.id, "install", "正在安装 requirements.txt 依赖（pip）…");
            let py_s = py.to_string_lossy().to_string();
            let (ok, last_err, _out) = run_capture_live(
                app,
                &def.id,
                "install",
                &py_s,
                &["-m", "pip", "install", "-r", "requirements.txt"],
                Some(dir),
                &[],
                None,
            );
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            Ok(())
        }
        PythonRuntimePlan::TargetDeps(program, prefix) => {
            emit_progress(
                app,
                &def.id,
                "install",
                &format!(
                    "当前 Python（{}）不带 venv 模块（常见于嵌入式发行版），改用 .deps 目录隔离安装依赖…",
                    program
                ),
            );
            let mut args = prefix.clone();
            args.extend([
                "-m".to_string(),
                "pip".to_string(),
                "install".to_string(),
                "-r".to_string(),
                "requirements.txt".to_string(),
                "--target".to_string(),
                ".deps".to_string(),
                "--upgrade".to_string(),
            ]);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, last_err, _out) =
                run_capture_live(app, &def.id, "install", &program, &arg_refs, Some(dir), &[], None);
            if !ok {
                let msg = last_err.trim();
                return Err(format!(
                    "pip install 失败: {}",
                    if msg.is_empty() { "未知错误" } else { msg }
                ));
            }
            record_deps_runtime(dir, &program, &prefix)?;
            Ok(())
        }
        PythonRuntimePlan::None => Err(
            "未找到支持 venv 或 pip 的 Python。请安装官方 Python 3.8+（python.org），\
             或在本应用 SDK 页为托管 Python 执行 install_pip 后重试"
                .to_string(),
        ),
    }
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
    let (ok, last_err, _out) = run_capture_live(app, &def.id, "build", &prog, &arg_refs, Some(dir), &[], None);
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
const NPM_ISOLATION_ENV: &[(&str, &str)] = &[
    // 显式关闭 workspaces 解析（含向上冒泡查找），npm 7+ 生效
    ("npm_config_workspaces", "false"),
    ("npm_config_workspaces_update", "false"),
    ("npm_config_workspaces_install", "false"),
    // pnpm 环境变量若被继承会改变依赖解析行为，清掉
    ("npm_config_shared_workspace_lockfile", "false"),
    ("npm_config_shell_emulator", "false"),
    // 关闭 fund/audit，同时强制 npm 输出可被前端逐条显示的详细日志。
    ("npm_config_fund", "false"),
    ("npm_config_audit", "false"),
    ("npm_config_loglevel", "verbose"),
    ("npm_config_progress", "true"),
    ("npm_config_foreground_scripts", "true"),
    ("npm_config_timing", "true"),
    ("npm_config_update_notifier", "false"),
];

const NPM_OUTPUT_ARGS: &[&str] = &[
    "--workspaces=false",
    "--no-fund",
    "--no-audit",
    "--loglevel=verbose",
    "--progress=true",
    "--foreground-scripts",
    "--timing",
    // 显式 fetch 超时/重试，缓解 npm(undici) 网络请求 CloseWait 挂死问题
    "--fetch-timeout=60000",
    "--fetch-retries=2",
    "--fetch-retry-mintimeout=10000",
    "--fetch-retry-maxtimeout=60000",
];

fn npx_install_args(prefix: Vec<String>, runtime_dir: &Path, spec: String) -> Vec<String> {
    let mut args = prefix;
    args.push("install".to_string());
    args.push("--prefix".to_string());
    args.push(runtime_dir.to_string_lossy().to_string());
    args.extend(NPM_OUTPUT_ARGS.iter().map(|arg| (*arg).to_string()));
    args.push(spec);
    args
}

/// pnpm 安装参数：`pnpm add --dir <运行目录> --ignore-workspace <spec>`。
///
/// `--ignore-workspace` 必须加：运行目录位于托管目录内部，而托管目录可能残留旧版
/// Git checkout 的 `pnpm-workspace.yaml`，pnpm 会向上识别为 workspace 而对本次安装报错。
const PNPM_OUTPUT_ARGS: &[&str] = &["--ignore-workspace"];

/// 按包管理器名组装 npx 模式的安装参数。
/// pnpm → `pnpm add --dir <dir> --ignore-workspace <spec>`
/// yarn → `yarn --cwd <dir> add <spec>`
/// npm  → `npm install --prefix <dir> <flags> <spec>`
fn npx_pm_args(pm: &str, prefix: Vec<String>, runtime_dir: &Path, spec: String) -> Vec<String> {
    match pm {
        "pnpm" => {
            let mut a = prefix;
            a.push("add".to_string());
            a.push("--dir".to_string());
            a.push(runtime_dir.to_string_lossy().to_string());
            a.extend(PNPM_OUTPUT_ARGS.iter().map(|arg| (*arg).to_string()));
            a.push(spec);
            a
        }
        "yarn" => {
            let mut a = prefix;
            a.push("--cwd".to_string());
            a.push(runtime_dir.to_string_lossy().to_string());
            a.push("add".to_string());
            a.push(spec);
            a
        }
        _ => npx_install_args(prefix, runtime_dir, spec),
    }
}

/// 选择 npx 模式的安装器：优先项目配置的包管理器（harness 为 pnpm），未安装则回退 npm。
///
/// 必须优先 pnpm 的原因（实测 @deepseek-ai/dsh）：该包有 62 个直接依赖、505 个
/// 传递包，且**全部是预发布版本**（`0.1.1-rc.2` + `^0.1.1-rc.2` 范围）。npm 的依赖
/// 解析（idealTree/placeDep）在这种图上会组合爆炸：CPU 跑满、内存涨到 2.4GB 且不
/// 收敛，15 分钟仍停在同一个 placeDep 步骤（与网络无关，换镜像/代理都一样）。
/// pnpm 的解析器可在 1 分钟内完成同样的安装，且用硬链接复用全局 store，更省磁盘。
fn npx_installer(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
) -> Result<(String, String, Vec<String>), String> {
    let configured = def.package_manager.trim().to_lowercase();
    if !configured.is_empty() && configured != "npm" {
        match resolve_exe_invocation(&configured, "") {
            Ok((prog, prefix)) => return Ok((configured, prog, prefix)),
            Err(_) => {
                emit_log(
                    app,
                    &def.id,
                    "npx",
                    &format!("未检测到 {}，回退使用 npm 安装", configured),
                );
            }
        }
    }
    let (prog, prefix) = resolve_exe_invocation("npm", "请先安装 Node.js (https://nodejs.org)")?;
    Ok(("npm".to_string(), prog, prefix))
}

fn npx_install(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    latest: bool,
) -> Result<(), String> {
    let pkg = def.npx_package.trim();
    let spec = if latest {
        format!("{}@latest", strip_pkg_version(pkg))
    } else {
        pkg.to_string()
    };
    emit_progress(app, &def.id, "npx", &format!("正在通过 npx 安装 {} …", pkg));
    // 不要把 Git workspace 根目录作为安装 prefix：旧版 harness checkout 中包含
    // `workspace:^`，npm 即使收到 --workspaces=false 也会先解析该根 package.json。
    let runtime_dir = def.npx_runtime_dir();
    fs::create_dir_all(&runtime_dir).map_err(|e| format!("创建 npx 运行目录失败: {}", e))?;
    // 预先创建最小 package.json，阻止包管理器向上冒泡到 managed_dir 的 workspace 元数据。
    let runtime_manifest = runtime_dir.join("package.json");
    if !runtime_manifest.exists() {
        fs::write(&runtime_manifest, "{\"private\":true}\n")
            .map_err(|e| format!("初始化 npx 运行目录失败: {}", e))?;
    }
    let (pm_name, prog, prefix) = npx_installer(app, def)?;
    emit_progress(
        app,
        &def.id,
        "npx",
        &format!("使用 {} 安装 {} …", pm_name, spec),
    );
    let args = npx_pm_args(&pm_name, prefix, &runtime_dir, spec);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err, full_output) = run_capture_live(app, &def.id, "npx", &prog, &arg_refs, None, NPM_ISOLATION_ENV, Some(Duration::from_secs(900)));
    if !ok {
        let msg = last_err.trim();
        // pnpm 10+ 默认不执行依赖的构建脚本，并会以 ERR_PNPM_IGNORED_BUILDS
        // 非零码退出——但包本身已完整安装，只是原生模块（node-pty/koffi 等）未编译。
        // 这类失败不构成安装失败：批准构建需要本机 C++ 工具链（node-gyp + MSVC），
        // 在无构建环境的机器上强制开启会让安装彻底失败；而绝大多数服务
        // （实测 dsh web）并不依赖这些原生模块即可正常运行。
        // 仅当「确实是构建被跳过」且「包已安装」时才降级为警告。
        let ignored_builds = full_output.contains("ERR_PNPM_IGNORED_BUILDS")
            || msg.contains("ERR_PNPM_IGNORED_BUILDS")
            || full_output.contains("Ignored build scripts");
        if !(ignored_builds && def.npx_installed()) {
            return Err(format!("{} 安装失败: {}", pm_name, if msg.is_empty() { "未知错误" } else { msg }));
        }
        emit_log(
            app,
            &def.id,
            "npx",
            "已跳过部分原生模块构建（需本机 C++ 构建工具链），服务仍可启动；如确需原生模块，请安装构建工具后手动执行 pnpm approve-builds",
        );
    }
    if !def.npx_installed() {
        let _ = fs::remove_dir_all(&runtime_dir);
        return Err(format!("安装完成但未找到包 {}，请检查 npxPackage 是否拼写正确", pkg));
    }
    emit_progress(app, &def.id, "done", "npx 安装完成");
    Ok(())
}

/// 安装前检查依赖是否就绪，并返回 deps 结果用于报错。
fn ensure_deps_ready(def: &NodeProjectDef) -> Result<DepCheckResult, String> {
    let deps = check_deps(def);
    // pip 包模式：直接从 PyPI 安装，无需 git（仅需 Python 可用）
    if def.is_pip_package() {
        if !deps.node.exists {
            return Err("未检测到 python，请先安装 Python 3.8+ (https://www.python.org)".to_string());
        }
        if !deps.node.satisfies {
            return Err(format!(
                "Python 版本 {} 不满足要求 {}，请升级 Python",
                deps.node.version.as_deref().unwrap_or("未知"),
                deps.node.requirement.as_deref().unwrap_or("")
            ));
        }
        return Ok(deps);
    }
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
    if def.is_python() {
        if !deps.git.exists {
            return Err("未检测到 git，请先安装 Git for Windows (https://git-scm.com)".to_string());
        }
        if !deps.node.exists {
            return Err("未检测到 python，请先安装 Python 3.8+ (https://www.python.org)".to_string());
        }
        if !deps.node.satisfies {
            return Err(format!(
                "Python 版本 {} 不满足要求 {}，请升级 Python",
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
        return npx_install(&app, &def, false);
    }

    // pip 包模式：直接从 PyPI 安装，无需 git clone
    if def.is_pip_package() {
        return pip_package_install(&app, &def, &dir, false);
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
    let (ok, last_err, _out) = run_capture_live(&app, &def.id, "clone", "git", &arg_refs, Some(parent), &[], None);
    if !ok {
        let msg = last_err.trim();
        return Err(format!("git clone 失败: {}", if msg.is_empty() { "未知错误" } else { msg }));
    }

    let manifest = if def.is_python() { "requirements.txt" } else { "package.json" };
    if !dir.join(manifest).exists() {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("克隆完成但未找到 {}，可能仓库结构异常", manifest));
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
        return npx_install(&app, &def, true);
    }

    // pip 包模式：pip install --upgrade 即更新到最新版
    if def.is_pip_package() {
        return pip_package_install(&app, &def, &dir, true);
    }

    emit_progress(&app, &def.id, "pull", "正在拉取最新代码 (git pull)…");
    let (ok, last_err, _out) = run_capture_live(&app, &def.id, "pull", "git", &["pull"], Some(&dir), &[], None);
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
        return npx_install(&app, &def, true);
    }

    // pip 包模式：重装即修复依赖
    if def.is_pip_package() {
        pip_package_install(&app, &def, &dir, false)?;
        emit_progress(&app, &def.id, "done", "依赖安装完成");
        return Ok(());
    }
    pm_install(&app, &def, &dir)?;
    pm_build(&app, &def, &dir)?;
    emit_progress(&app, &def.id, "done", "依赖安装完成");
    Ok(())
}

/// 编译原生模块（npx 模式专用）。
///
/// pnpm 10+ 默认跳过依赖的构建脚本，安装时会以 ERR_PNPM_IGNORED_BUILDS 非零码
/// 结束（包已完整安装，仅原生模块未编译）。本命令执行
/// `pnpm approve-builds --all`：非交互批准全部待构建依赖**并立即执行其构建脚本**
/// （实测 node-pty/koffi 等会当场完成 postinstall/编译；需要本机 C++ 工具链）。
/// npm 本身默认执行构建脚本，重跑用 `npm rebuild`；yarn 1 无拦截，重跑 `yarn rebuild`。
#[tauri::command]
pub async fn npm_build_native(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先「安装」".to_string());
    }
    if !def.is_npx() {
        return Err("仅 npx 模式的项目需要单独编译原生模块".to_string());
    }
    let runtime_dir = def.npx_runtime_dir();
    let (pm_name, prog, prefix) = npx_installer(&app, &def)?;
    emit_progress(
        &app,
        &def.id,
        "native",
        &format!("使用 {} 编译原生模块…", pm_name),
    );
    let args = if pm_name == "pnpm" {
        let mut a = prefix;
        a.push("approve-builds".to_string());
        a.push("--all".to_string());
        a.push("--dir".to_string());
        a.push(runtime_dir.to_string_lossy().to_string());
        a
    } else if pm_name == "yarn" {
        let mut a = prefix;
        a.push("--cwd".to_string());
        a.push(runtime_dir.to_string_lossy().to_string());
        a.push("rebuild".to_string());
        a
    } else {
        let mut a = prefix;
        a.push("rebuild".to_string());
        a.push("--prefix".to_string());
        a.push(runtime_dir.to_string_lossy().to_string());
        a
    };
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    // pnpm 用 --dir、npm 用 --prefix、yarn 用 --cwd 指定运行目录，均无需额外 cwd
    let (ok, last_err, _out) = run_capture_live(
        &app,
        &def.id,
        "native",
        &prog,
        &arg_refs,
        None,
        NPM_ISOLATION_ENV,
        Some(Duration::from_secs(900)),
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "{} 编译原生模块失败: {}",
            pm_name,
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }
    emit_progress(&app, &def.id, "done", "原生模块编译完成");
    Ok(())
}

/// 在服务的运行目录中执行任意命令行。
///
/// npx 模式的服务不做全局安装，`dsh plugin --profile web add` 这类命令无法在
/// 系统任意位置使用；本命令把运行目录下的 `node_modules/.bin` 前置到 PATH 后
/// 在运行目录（npx = .npx-runtime，git = 托管目录）内执行整条命令行，
/// 项目自带命令与任意命令均可直接运行。
///
/// Windows 上用 `raw_arg` 传递整条命令行：若走普通 arg，Rust 会给含空格的命令
/// 加引号，触发 cmd.exe「首字符为引号时剥离首尾引号」的规则，把命令拆坏。
#[tauri::command]
pub async fn npm_exec(
    app: tauri::AppHandle,
    project_id: String,
    command: String,
) -> Result<(), String> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("命令不能为空".to_string());
    }
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先「安装」".to_string());
    }
    let cwd = if def.is_npx() {
        def.npx_runtime_dir()
    } else {
        def.managed_dir()
    };
    if !cwd.exists() {
        return Err(format!("运行目录不存在: {}", cwd.display()));
    }

    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = hidden_cmd(&cmd_exe_path());
        c.arg("/c");
        c.raw_arg(&command);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c");
        c.arg(&command);
        c
    };
    cmd.current_dir(&cwd);
    // 前置 node_modules/.bin，使项目自带命令（如 dsh）无需全局安装即可调用
    let bin_dir = cwd.join("node_modules").join(".bin");
    if bin_dir.exists() {
        let path = std::env::var("PATH").unwrap_or_default();
        if let Ok(joined) = std::env::join_paths(
            std::iter::once(bin_dir).chain(std::env::split_paths(&path)),
        ) {
            cmd.env("PATH", joined);
        }
    }

    emit_log(&app, &def.id, "exec", &format!("$ {}", command));
    emit_log(&app, &def.id, "exec", &format!("工作目录: {}", cwd.display()));
    let (ok, last_err, _out) = run_cmd_live(
        &app,
        &def.id,
        "exec",
        &command,
        cmd,
        Some(Duration::from_secs(600)),
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "命令执行失败（退出码非 0）: {}",
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }
    emit_progress(&app, &def.id, "done", "命令执行完成");
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

/// 停止项目进程树，并清除该项目的控制台 URL 捕获记录。
pub(crate) fn stop_project_process(def: &NodeProjectDef) -> Result<(), String> {
    // 控制台 URL 随进程失效（重启后由新输出重新捕获）
    clear_console_url(&def.id);
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

    // npx 模式启动策略：
    // - 未安装 → 先完整安装（npx_install 内部带 15 分钟超时兜底，网络挂死时
    //   不会永久卡住，超时会终止进程树并返回失败）。
    // - 已安装 → 先用 `npm view`（30s 超时）快速比对远程最新版，仅在有新版时
    //   才重新安装；比对失败/超时/离线则直接用本地已装版本启动，保证可启动性。
    //   之前「每次启动都全量 npm install pkg@latest」在网络慢或 npm fetch 挂死时
    //   会导致服务永远起不来，且正常情况下也要先花数分钟下载校验才能启动。
    if def.is_npx() {
        if !def.npx_installed() {
            npx_install(&app, &def, false)?;
        } else {
            let current = npx_local_version(&def);
            let latest = npm_remote_version(def.npx_package.trim());
            let outdated = match (&current, &latest) {
                (Some(c), Some(l)) => version_gt(l, c),
                _ => false,
            };
            if outdated {
                emit_progress(
                    &app,
                    &def.id,
                    "npx",
                    &format!(
                        "发现新版本 {}（本地 {}），正在更新…",
                        latest.as_deref().unwrap_or("?"),
                        current.as_deref().unwrap_or("?")
                    ),
                );
                if let Err(e) = npx_install(&app, &def, true) {
                    emit_log(
                        &app,
                        &def.id,
                        "stderr",
                        &format!("更新到最新版失败，使用本地已装版本启动: {}", e),
                    );
                }
            }
        }
    }

    // 组装启动命令：git 模式 `pnpm dsh web`；npx 模式 `npx -y --prefix <托管目录> <bin> <args...>`；
    // python 模式 `.venv 内 python <startCmd...>`（startCmd 即 python 参数，如 `-m core.converter`）；
    // .deps 降级模式：基准 python + PYTHONPATH=<托管目录>/.deps（嵌入式 Python 已在安装期补 ._pth）。
    let mut python_extra_env: Vec<(String, String)> = Vec::new();
    let (prog, args) = if def.is_python() {
        // pip 包模式：`python -m <pipModule> <startCmd...>`；普通 python 项目：startCmd 直接是 python 参数
        let pip_module = def.pip_module.trim().to_string();
        if def.is_pip_package() && pip_module.is_empty() {
            return Err("pip 包模式需要配置 pipModule（如 headroom.cli）".to_string());
        }
        let build_args = |cmd: &[String]| -> Vec<String> {
            let inner = if def.is_pip_package() {
                pip_launch_args(&pip_module, cmd)
            } else {
                cmd.to_vec()
            };
            with_utf8_flag(inner)
        };
        match plan_python_launch(&dir, &def.venv_python()) {
            PythonLaunchPlan::Venv(py) => (py.to_string_lossy().to_string(), build_args(&def.start_cmd)),
            PythonLaunchPlan::Deps { program, prefix, deps_dir } => {
                let mut a = prefix;
                a.extend(build_args(&def.start_cmd));
                // PYTHONPATH = .deps + 项目源码目录（`-m core.converter` 的 core 包在托管目录，
                // 嵌入式 Python 的 ._pth 锁死 sys.path 且不自动加 cwd，必须显式给出）
                // + pywin32 子目录（--target 安装的 pywin32 把模块放在 win32/ 与
                // pywin32_system32/ 下，不在 .deps 顶层）。
                python_extra_env.push(("PYTHONPATH".to_string(), deps_pythonpath(&dir, &deps_dir)));
                // pywintypes 的 DLL 依赖需要 pywin32_system32 进入 DLL 搜索路径（PATH）
                if let Some(sys32) = pywin32_deps_dirs(&deps_dir)
                    .into_iter()
                    .find(|p| p.file_name().map(|n| n == "pywin32_system32").unwrap_or(false))
                {
                    let mut path_val = sys32.to_string_lossy().to_string();
                    if let Some(existing) = std::env::var_os("PATH") {
                        path_val.push_str(";");
                        path_val.push_str(&existing.to_string_lossy());
                    }
                    python_extra_env.push(("PATH".to_string(), path_val));
                }
                (program, a)
            }
            PythonLaunchPlan::Missing => {
                return Err(
                    "Python 运行时未就绪（.venv 与 .deps 均缺失），请先「安装」或「安装依赖」".to_string(),
                )
            }
        }
    } else if def.is_npx() {
        let (p, prefix) = resolve_exe_invocation("npx", "请先安装 Node.js (https://nodejs.org)")?;
        let mut a = prefix;
        a.push("-y".to_string());
        a.push("--prefix".to_string());
        a.push(def.npx_runtime_dir().to_string_lossy().to_string());
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

    // npx 发布包运行在隔离 prefix 中，避免从旧 workspace checkout 读取配置。
    let runtime_cwd = if def.is_npx() {
        def.npx_runtime_dir()
    } else {
        dir.clone()
    };

    // 启动后台进程（CREATE_NO_WINDOW），记录 PID
    let mut cmd = hidden_cmd(&prog);
    cmd.args(&args);
    cmd.current_dir(&runtime_cwd);
    for (key, val) in &python_extra_env {
        cmd.env(key, val);
    }
    for (key, val) in &def.env {
        cmd.env(key, val);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // 额外确保创建独立的进程组，便于后续 taskkill /T；
    // 通过 spawn_breakaway_fallback 尝试脱离 AnyVersion 生命周期，
    // 若所在 Job 不允许 breakaway 则自动降级（不会整体启动失败）。
    let mut child = crate::commands::hidden_cmd::spawn_breakaway_fallback(cmd)
        .map_err(|e| format!("启动失败: {}", e))?;
    record_pid(&def.id, child.id());

    // 实时回传启动日志（进程常驻，子线程持续读取 stdout/stderr 直到进程退出）。
    // 配置了 consoleUrlPattern 的项目，同时在输出行中提取带凭据主页地址：
    // 捕获到新地址（以最后一次输出为准）时记录、通知前端，并按配置自动打开。
    emit_progress(&app, &def.id, "start", &format!("启动命令: {}", def.start_cmd.join(" ")));
    // 诊断：把实际命令行与关键 env 打进日志（排查「无输出退出」时能看到真实启动形态）
    {
        let pypp = python_extra_env
            .iter()
            .find(|(k, _)| k == "PYTHONPATH")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        emit_log(&app, &def.id, "stdout", &format!("$ {} {}", prog, args.join(" ")));
        emit_log(&app, &def.id, "stdout", &format!("  cwd: {}", runtime_cwd.display()));
        if !pypp.is_empty() {
            emit_log(&app, &def.id, "stdout", &format!("  PYTHONPATH: {}", pypp));
        }
    }
    let mut readers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let app_out = app.clone();
    let pid_out = def.id.clone();
    let pattern_out = def.console_url_pattern.clone();
    let auto_open_out = def.auto_open_console_url;
    if let Some(mut so) = child.stdout.take() {
        readers.push(std::thread::spawn(move || {
            read_child_stream(&mut so, &app_out, &pid_out, "stdout", &pattern_out, auto_open_out);
        }));
    }
    let app_err = app.clone();
    let pid_err = def.id.clone();
    let pattern_err = def.console_url_pattern.clone();
    let auto_open_err = def.auto_open_console_url;
    if let Some(mut se) = child.stderr.take() {
        readers.push(std::thread::spawn(move || {
            read_child_stream(&mut se, &app_err, &pid_err, "stderr", &pattern_err, auto_open_err);
        }));
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
        if let Ok(Some(status)) = child.try_wait() {
            let code_desc = status
                .code()
                .map(|c| format!("exit code {c}"))
                .unwrap_or_else(|| format!("{status:?}"));
            emit_log(
                &app,
                &def.id,
                "stderr",
                &format!("启动进程已退出（端口 {} 未就绪，{}）", def.default_port, code_desc),
            );
            // 等 reader 线程读到 EOF 并把尾部日志发到前端，避免「退出即失败、日志丢失」
            for h in readers.drain(..) {
                let _ = h.join();
            }
            return Err(format!(
                "启动进程已退出，端口 {} 未就绪（{}）。请查看上方日志确认报错",
                def.default_port, code_desc
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

/// 查询某项目最近捕获的控制台 URL（带凭据主页地址）。
/// 未配置 consoleUrlPattern 或尚未捕获时返回 None。
#[tauri::command]
pub fn npm_console_url(project_id: String) -> Result<Option<String>, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.has_console_url_pattern() {
        return Ok(None);
    }
    Ok(console_url_of(&def.id))
}

/// 在独立的 Kira 窗口中打开服务主页。
///
/// 为什么不用主窗口 iframe：dsh 等服务的鉴权依赖 `SameSite=Strict` 的 HttpOnly
/// Cookie（token URL → 303 → Set-Cookie → /）。iframe 是第三方上下文，WebView2
/// 会拦截该 Cookie，重定向后的 / 永远 401——token 正确也进不去。独立顶级窗口的
/// Cookie 是第一方，可正常完成握手，同时仍属于本应用（而非外部浏览器）。
#[tauri::command]
pub async fn npm_open_window(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    use tauri::Manager;
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    let url = if def.has_console_url_pattern() {
        console_url_of(&def.id).unwrap_or_else(|| def.resolved_web_path())
    } else {
        def.resolved_web_path()
    };
    let label = format!("svc-{}", def.id);
    // 已有窗口：导航到最新地址并聚焦（token 轮换后复用窗口即可）
    if let Some(win) = app.get_webview_window(&label) {
        let js = format!(
            "window.location.replace({});",
            serde_json::to_string(&url).unwrap_or_else(|_| "\"about:blank\"".into())
        );
        let _ = win.eval(&js);
        let _ = win.set_focus();
        return Ok(());
    }
    let parsed: tauri::Url = url.parse().map_err(|e| format!("URL 解析失败: {}", e))?;
    tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::External(parsed),
    )
    .title(format!("{} — Kira", def.display_name))
    .inner_size(1280.0, 820.0)
    .build()
    .map_err(|e| format!("创建窗口失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn npm_open(project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    // 配置了控制台 URL 提取的项目优先用捕获的带凭据地址（token 随每次启动变化）；
    // 尚未捕获时回退普通 webPath（服务自己无鉴权时也能直接打开）。
    let url = if def.has_console_url_pattern() {
        console_url_of(&def.id).unwrap_or_else(|| def.resolved_web_path())
    } else {
        def.resolved_web_path()
    };
    open_url_in_browser(&url);
    Ok(())
}

// ─── 开发者模式打开主页面 ───

/// 返回浏览器的常见安装位置。
/// Windows 桌面应用通常不会把 Edge/Chrome 加入 Kira 进程的 PATH，
/// 仅调用 find_in_path 会导致「开发者模式」看起来完全没有反应。
fn browser_candidates(name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(path) = find_in_path(name) {
        out.push(path);
    }

    #[cfg(target_os = "windows")]
    {
        let (relative, exe) = match name.to_ascii_lowercase().as_str() {
            "msedge" | "msedge.exe" => ("Microsoft\\Edge\\Application", "msedge.exe"),
            "chrome" | "chrome.exe" => ("Google\\Chrome\\Application", "chrome.exe"),
            _ => return out,
        };
        for env_name in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(root) = std::env::var(env_name) {
                out.push(PathBuf::from(root).join(relative).join(exe));
            }
        }
    }
    out
}

fn find_browser(name: &str) -> Option<PathBuf> {
    browser_candidates(name).into_iter().find(|p| p.is_file())
}

/// 生成独立浏览器实例的启动参数。
/// 使用独立 profile 是关键：如果复用已运行的 Edge/Chrome，浏览器可能把 URL
/// 转发给旧进程并忽略 `--auto-open-devtools-for-tabs`。
fn devtools_browser_args(url: &str, profile_dir: &Path) -> Vec<String> {
    vec![
        "--new-window".to_string(),
        "--auto-open-devtools-for-tabs".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        format!("--user-data-dir={}", profile_dir.to_string_lossy()),
        url.to_string(),
    ]
}

/// 在系统浏览器中打开服务首页，并请求自动打开 DevTools。
/// 内嵌 iframe 无法跨域控制浏览器 DevTools，因此开发者模式使用独立浏览器窗口，
/// 同时保留服务管理器里的内嵌首页作为日常查看入口。
#[tauri::command]
pub async fn npm_open_devtools(project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    let url = def.resolved_web_path();

    #[cfg(target_os = "windows")]
    {
        let browser = find_browser("msedge").or_else(|| find_browser("chrome"));
        let path = browser.ok_or_else(|| {
            "未找到 Edge 或 Chrome。请确认浏览器已安装，或将浏览器目录加入 PATH 后重试".to_string()
        })?;
        let profile = std::env::temp_dir()
            .join("any-version-devtools")
            .join(format!("{}-{}", project_id, std::process::id()));
        let args = devtools_browser_args(&url, &profile);
        eprintln!("[node_manager] 开发者模式: {} {:?}", path.display(), args);
        std::process::Command::new(&path)
            .args(&args)
            .spawn()
            .map_err(|e| format!("启动开发者模式浏览器失败（{}）: {}", path.display(), e))?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let args = devtools_browser_args(&url, &std::env::temp_dir().join("any-version-devtools"));
        std::process::Command::new("open")
            .args(["-na", "Google Chrome", "--args"])
            .args(&args)
            .spawn()
            .map_err(|e| format!("启动开发者模式浏览器失败: {}", e))?;
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let browser = find_browser("google-chrome")
            .or_else(|| find_browser("chromium"))
            .ok_or_else(|| "未找到 Google Chrome 或 Chromium，无法打开开发者模式".to_string())?;
        let args = devtools_browser_args(&url, &std::env::temp_dir().join("any-version-devtools"));
        std::process::Command::new(browser)
            .args(&args)
            .spawn()
            .map_err(|e| format!("启动开发者模式浏览器失败: {}", e))?;
        return Ok(());
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
    // Python 运行时：node 槽位复用为 python 检查（前端徽章按 runtime 显示 python），
    // pm 槽位显示 pip（随 venv 提供，系统只需 python 就绪）。
    // `python --version` 输出 "Python 3.12.4"，parse_version_parts 会忽略非数字前缀。
    if def.is_python() {
        let mut py = check_dep("python", &["--version"]);
        py.name = "python".to_string();
        if py.exists {
            let v = py.version.as_deref().unwrap_or("");
            py.satisfies = version_satisfies(v, &def.node_requirement);
        } else {
            py.satisfies = false;
        }
        py.requirement = Some(def.node_requirement.clone());
        let mut pm = py.clone();
        pm.name = "pip".to_string();
        pm.requirement = Some("随 python 提供".to_string());
        pm.satisfies = py.exists && py.satisfies;
        // pip 包模式无需 git（直接从 PyPI 安装）
        let all_ready = py.exists && py.satisfies && (def.is_pip_package() || git.exists);
        return DepCheckResult { git, node: py, package_manager: pm, all_ready };
    }
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

/// npx 模式读取本地已装版本：`.npx-runtime/node_modules/<包名>/package.json` 的 version 字段。
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
    let pkg_json = def.npx_runtime_dir().join(rel).join("package.json");
    let data = fs::read_to_string(pkg_json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v.get("version")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
}

/// 查询 npm registry 上包的最新版本：`npm view <pkg> version`。
/// 带超时（npm view 的网络请求同样可能挂死）；网络不可用 / 超时 / npm 不在
/// PATH / 包不存在时返回 None。
fn npm_remote_version(pkg: &str) -> Option<String> {
    npm_remote_version_with_timeout(pkg, Duration::from_secs(30))
}

fn npm_remote_version_with_timeout(pkg: &str, timeout: Duration) -> Option<String> {
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
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let started_at = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                if let Some(mut so) = child.stdout.take() {
                    use std::io::Read;
                    let _ = so.read_to_string(&mut out);
                }
                return out
                    .lines()
                    .map(|l| l.trim().to_string())
                    .find(|l| !l.is_empty());
            }
            Ok(None) => {
                if started_at.elapsed() >= timeout {
                    kill_process_tree(&mut child);
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    }
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
            conflict_process: None,
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
    let mut conflict_process = None;
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
            // 借用 port.rs 的 tasklist 查询占用进程名，供前端明确展示
            conflict_process = super::port::find_port_owner(&def.default_port.to_string())
                .map(|o| o.process_name);
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
        conflict_process,
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
            runtime: String::new(),
            package_manager: "pnpm".into(),
            build_script: String::new(),
            start_cmd: Vec::new(),
            managed: true,
            npx_package: String::new(),
            npx_bin: String::new(),
            console_url_pattern: String::new(),
            auto_open_console_url: false,
            pip_package: String::new(),
            pip_extra_packages: Vec::new(),
            pip_module: String::new(),
            env: HashMap::new(),
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
            runtime: String::new(),
            package_manager: String::new(),
            build_script: String::new(),
            start_cmd: Vec::new(),
            managed: true,
            npx_package: pkg.into(),
            npx_bin: bin.into(),
            console_url_pattern: String::new(),
            auto_open_console_url: false,
            pip_package: String::new(),
            pip_extra_packages: Vec::new(),
            pip_module: String::new(),
            env: HashMap::new(),
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

    #[test]
    fn test_npx_runtime_dir_isolated_from_managed_root() {
        let def = npx_def("@deepseek-ai/dsh", "dsh");
        let runtime = def.npx_runtime_dir();
        assert_eq!(runtime.file_name().and_then(|x| x.to_str()), Some(".npx-runtime"));
        assert!(runtime.starts_with(def.managed_dir()));
        assert_ne!(runtime, def.managed_dir());
    }

    #[test]
    fn test_npx_install_shows_verbose_output_flags() {
        let args = npx_install_args(
            vec!["/c".into(), "npm.cmd".into()],
            Path::new("runtime"),
            "@scope/pkg@latest".into(),
        );
        assert_eq!(args[2], "install");
        assert_eq!(args[3], "--prefix");
        assert!(args.contains(&"--loglevel=verbose".to_string()));
        assert!(args.contains(&"--progress=true".to_string()));
        assert!(args.contains(&"--foreground-scripts".to_string()));
        assert!(args.contains(&"--timing".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("@scope/pkg@latest"));
    }

    #[test]
    fn test_npx_pnpm_args_use_add_and_ignore_workspace() {
        let args = npx_pm_args(
            "pnpm",
            vec!["/c".into(), "pnpm.cmd".into()],
            Path::new("runtime"),
            "@scope/pkg@latest".into(),
        );
        assert_eq!(args[2], "add");
        assert_eq!(args[3], "--dir");
        assert_eq!(args[4], "runtime");
        assert!(args.contains(&"--ignore-workspace".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("@scope/pkg@latest"));
    }

    #[test]
    fn test_npx_npm_args_use_install_prefix() {
        let args = npx_pm_args(
            "npm",
            vec!["/c".into(), "npm.cmd".into()],
            Path::new("runtime"),
            "pkg".into(),
        );
        assert_eq!(args[2], "install");
        assert_eq!(args[3], "--prefix");
        assert_eq!(args[4], "runtime");
        assert_eq!(args.last().map(String::as_str), Some("pkg"));
    }

    #[test]
    fn test_npx_yarn_args_use_cwd() {
        let args = npx_pm_args(
            "yarn",
            vec!["/c".into(), "yarn.cmd".into()],
            Path::new("runtime"),
            "pkg".into(),
        );
        assert_eq!(args[2], "--cwd");
        assert_eq!(args[3], "runtime");
        assert_eq!(args[4], "add");
        assert_eq!(args.last().map(String::as_str), Some("pkg"));
    }

    #[test]
    fn test_devtools_browser_args_isolated_profile() {
        let args = devtools_browser_args("http://127.0.0.1:3080", Path::new("C:\\Temp\\Kira DevTools"));
        assert!(args.contains(&"--new-window".to_string()));
        assert!(args.contains(&"--auto-open-devtools-for-tabs".to_string()));
        assert!(args.contains(&"--no-first-run".to_string()));
        assert!(args.contains(&"--no-default-browser-check".to_string()));
        assert!(args.iter().any(|x| x == "--user-data-dir=C:\\Temp\\Kira DevTools"));
        assert_eq!(args.last().map(String::as_str), Some("http://127.0.0.1:3080"));
    }

    #[test]
    fn test_browser_candidates_keep_path_lookup_first() {
        // 不依赖本机是否安装浏览器：候选列表至少保持可由 PATH 命中的结果优先，
        // Windows 上再追加 Program Files / LOCALAPPDATA 的标准目录。
        let candidates = browser_candidates("a-browser-that-is-not-installed");
        assert!(candidates.is_empty());
        let candidates = browser_candidates("chrome");
        if !candidates.is_empty() {
            assert!(candidates.iter().any(|p| p.to_string_lossy().to_lowercase().contains("chrome")));
        }
    }

    #[test]
    fn test_format_command_quotes_paths() {
        let command = format_command(
            "npm.cmd",
            &["install", "--prefix", "C:\\Program Files\\Kira", "pkg"],
        );
        assert_eq!(
            command,
            "npm.cmd install --prefix \"C:\\Program Files\\Kira\" pkg"
        );
    }

    // ─── 控制台 URL 捕获 ───

    #[test]
    fn test_extract_console_url_dsh_line() {
        // dsh web 启动时打印：dsh web: http://127.0.0.1:3080/?token=xxx（可能带 LAN 后缀）
        let line = "dsh web: http://127.0.0.1:3080/?token=dV2LhQhkX5yF18tf9baCDDsdbIfeLslc2g8P8wiTMNY";
        assert_eq!(
            extract_console_url("dsh web: (http\\S+)", line).as_deref(),
            Some("http://127.0.0.1:3080/?token=dV2LhQhkX5yF18tf9baCDDsdbIfeLslc2g8P8wiTMNY")
        );
        // 行尾括号包裹的 URL 也要剥干净
        let line2 = "dsh web: http://127.0.0.1:3080/?token=abc123)";
        assert_eq!(
            extract_console_url("dsh web: (http\\S+)", line2).as_deref(),
            Some("http://127.0.0.1:3080/?token=abc123")
        );
        // 无匹配行 / 非 http 开头捕获组 → None
        assert_eq!(extract_console_url("dsh web: (http\\S+)", "some other log line"), None);
        assert_eq!(extract_console_url("found at (\\S+)", "found at ftp://x"), None);
        // 空 pattern → None
        assert_eq!(extract_console_url("", "dsh web: http://x"), None);
        // 非法 regex → None（不 panic）
        assert_eq!(extract_console_url("(unclosed", "anything"), None);
    }

    #[test]
    fn test_console_url_lifecycle() {
        let id = "test-console-url-lifecycle";
        // 初始为空
        clear_console_url(id);
        assert_eq!(console_url_of(id), None);
        // 首次记录：changed = true，且可读回
        assert!(record_console_url_if_changed(id, "http://127.0.0.1:3080/?token=first"));
        assert_eq!(console_url_of(id).as_deref(), Some("http://127.0.0.1:3080/?token=first"));
        // 重复命中同一 URL：changed = false（不重复自动打开浏览器）
        assert!(!record_console_url_if_changed(id, "http://127.0.0.1:3080/?token=first"));
        // 覆盖（服务重启后新 token）：changed = true
        assert!(record_console_url_if_changed(id, "http://127.0.0.1:3080/?token=second"));
        assert_eq!(console_url_of(id).as_deref(), Some("http://127.0.0.1:3080/?token=second"));
        // 清除后为空（停止服务）
        clear_console_url(id);
        assert_eq!(console_url_of(id), None);
    }

    #[test]
    fn test_def_console_url_flags() {
        let mut def = npx_def("@deepseek-ai/dsh", "dsh");
        assert!(!def.has_console_url_pattern());
        def.console_url_pattern = "dsh web: (http\\S+)".into();
        assert!(def.has_console_url_pattern());
        // 配置文件 JSON 反序列化：camelCase 字段映射
        let json = r#"{
            "id": "x", "displayName": "X", "defaultPort": 1,
            "consoleUrlPattern": "dsh web: (http\\S+)", "autoOpenConsoleUrl": true
        }"#;
        let def: NodeProjectDef = serde_json::from_str(json).unwrap();
        assert_eq!(def.console_url_pattern, "dsh web: (http\\S+)");
        assert!(def.auto_open_console_url);
        assert!(def.has_console_url_pattern());
    }

    // ─── pip 包模式（PyPI 直装，无需 clone） ───

    #[test]
    fn test_pip_install_args_variants() {
        // 基础：装进当前环境（venv）
        assert_eq!(
            pip_install_args("headroom-ai[proxy]", &[], None, false),
            vec!["-m", "pip", "install", "headroom-ai[proxy]"]
        );
        // 升级 + 目标目录（嵌入式 Python 的 .deps 降级）
        assert_eq!(
            pip_install_args("headroom-ai[proxy]", &[], Some(".deps"), true),
            vec!["-m", "pip", "install", "--upgrade", "--target", ".deps", "headroom-ai[proxy]"]
        );
    }

    #[test]
    fn test_pip_launch_args_prepends_module() {
        let args = pip_launch_args(
            "headroom.cli",
            &["proxy", "--port", "8791"].map(String::from),
        );
        assert_eq!(args, vec!["-m", "headroom.cli", "proxy", "--port", "8791"]);
        // 无子命令时也能工作（仅 -m module）
        assert_eq!(pip_launch_args("pkg.mod", &[]), vec!["-m", "pkg.mod"]);
    }

    #[test]
    fn test_headroom_service_def_is_pip_mode() {
        // 内置 headroom 服务项必须是 pip 包模式，且启动命令/端口自洽
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../node-projects/headroom.json");
        let raw = fs::read_to_string(&path).expect("headroom.json 读取失败");
        let def: NodeProjectDef = serde_json::from_str(&raw).expect("headroom.json 解析失败");
        assert!(def.is_pip_package(), "headroom 服务项应配置 pipPackage");
        assert_eq!(def.runtime, "python");
        assert_eq!(def.pip_module, "headroom.cli");
        assert!(
            def.start_cmd.iter().any(|a| a == &def.default_port.to_string()),
            "startCmd 里的端口应与 defaultPort 一致"
        );
        assert!(def.start_cmd.first().map(|s| s.as_str()) == Some("proxy"));
        // 默认关闭匿名遥测（隐私）；文本压缩开关由 AI 设置页控制
        assert_eq!(def.env.get("HEADROOM_BEACON").map(String::as_str), Some("off"));
    }

    // ─── Python 运行时选择（venv 探测 / .deps 降级） ───

    #[test]
    fn test_select_python_runtime_prefers_venv_capable() {
        let probes = vec![
            ("py".to_string(), vec!["-3".to_string()], false, false),
            ("python".to_string(), vec![], true, true),
        ];
        assert_eq!(
            select_python_runtime(&probes),
            PythonRuntimePlan::Venv("python".to_string(), vec![])
        );
    }

    #[test]
    fn test_select_python_runtime_falls_back_to_pip_target() {
        // 嵌入式发行版：无 venv（即便有 pip）→ 降级 --target 安装
        let probes = vec![
            ("py".to_string(), vec!["-3".to_string()], false, false),
            ("python".to_string(), vec![], false, true),
        ];
        assert_eq!(
            select_python_runtime(&probes),
            PythonRuntimePlan::TargetDeps("python".to_string(), vec![])
        );
    }

    #[test]
    fn test_select_python_runtime_none_when_no_usable_python() {
        let probes = vec![("python".to_string(), vec![], false, false)];
        assert_eq!(select_python_runtime(&probes), PythonRuntimePlan::None);
    }

    #[test]
    fn test_plan_python_launch_prefers_venv() {
        let root = std::env::temp_dir().join(format!("kira-pyplan-{}", std::process::id()));
        let venv_py = root.join(".venv").join("Scripts").join("python.exe");
        std::fs::create_dir_all(&venv_py.parent().unwrap()).unwrap();
        std::fs::write(&venv_py, b"").unwrap();

        assert_eq!(plan_python_launch(&root, &venv_py), PythonLaunchPlan::Venv(venv_py.clone()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_plan_python_launch_deps_mode_uses_marker() {
        let root = std::env::temp_dir().join(format!("kira-pyplan-deps-{}", std::process::id()));
        let fake_py = root.join("base-python.exe");
        std::fs::create_dir_all(&root);
        std::fs::write(&fake_py, b"").unwrap();
        let deps = root.join(".deps");
        std::fs::create_dir_all(&deps);
        let marker = serde_json::json!({ "program": fake_py.to_string_lossy(), "prefix": [] });
        std::fs::write(deps.join(".python-runtime.json"), marker.to_string()).unwrap();

        let venv_py = root.join(".venv").join("Scripts").join("python.exe");
        match plan_python_launch(&root, &venv_py) {
            PythonLaunchPlan::Deps { program, prefix, deps_dir } => {
                assert_eq!(program, fake_py.to_string_lossy());
                assert!(prefix.is_empty());
                assert_eq!(deps_dir, deps);
            }
            other => panic!("期望 Deps，实际 {:?}", other),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // ─── pywin32 .deps 子目录探测 / pip extras 拼参 ───

    #[test]
    fn test_with_utf8_flag_prepends_x_utf8() {
        let args = with_utf8_flag(vec!["-m".into(), "core.converter".into()]);
        assert_eq!(args, vec!["-X", "utf8", "-m", "core.converter"]);
        let empty = with_utf8_flag(vec![]);
        assert_eq!(empty, vec!["-X", "utf8"]);
    }

    #[test]
    fn test_deps_pythonpath_includes_project_and_deps() {
        let root = std::env::temp_dir().join(format!("kira-pypp-{}", std::process::id()));
        let proj = root.join("workbuddy2api");
        let deps = proj.join(".deps");
        std::fs::create_dir_all(&deps).unwrap();
        let pp = deps_pythonpath(&proj, &deps);
        let parts: Vec<&str> = if cfg!(windows) { pp.split(';').collect() } else { pp.split(':').collect() };
        assert_eq!(parts[0], deps.to_string_lossy());
        assert_eq!(parts[1], proj.to_string_lossy(), "项目源码目录必须在 PYTHONPATH（-m core 可导入）");
        // 项目目录排在 .deps 之后（依赖优先于同名源码目录是安全序；这里仅验证包含）
        assert!(parts.len() >= 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_pywin32_deps_dirs_only_existing() {
        let root = std::env::temp_dir().join(format!("kira-pyw32-{}", std::process::id()));
        let deps = root.join(".deps");
        std::fs::create_dir_all(deps.join("win32").join("lib")).unwrap();
        std::fs::create_dir_all(deps.join("pywin32_system32")).unwrap();
        let dirs = pywin32_deps_dirs(&deps);
        assert_eq!(dirs.len(), 3, "应命中 win32、win32/lib、pywin32_system32: {:?}", dirs);
        // 未安装 pywin32 的项目：.deps 存在但无子目录 → 空
        let bare = root.join(".deps-bare");
        std::fs::create_dir_all(&bare).unwrap();
        assert!(pywin32_deps_dirs(&bare).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_pip_install_args_appends_extras() {
        let extras = vec!["pywin32; sys_platform == 'win32'".to_string()];
        let args = pip_install_args("headroom-ai[proxy]", &extras, Some(".deps"), false);
        assert_eq!(args, vec![
            "-m", "pip", "install", "--target", ".deps", "headroom-ai[proxy]",
            "pywin32; sys_platform == 'win32'",
        ]);
        // 空 extras 不产生多余参数
        let no_extra = pip_install_args("foo", &[], None, true);
        assert_eq!(no_extra, vec!["-m", "pip", "install", "--upgrade", "foo"]);
    }

    #[test]
    fn test_installed_python_project_accepts_deps_runtime() {
        // Python git 项目没有 package.json：.deps 运行时标记存在即视为已安装
        let def = NodeProjectDef {
            id: "wb-test".into(),
            display_name: "wb".into(),
            repo: "https://x.git".into(),
            website: String::new(),
            icon: String::new(),
            description: String::new(),
            default_port: 8788,
            web_path: String::new(),
            node_requirement: String::new(),
            runtime: "python".into(),
            package_manager: "pip".into(),
            build_script: String::new(),
            start_cmd: vec!["-m".into(), "core.converter".into()],
            managed: true,
            npx_package: String::new(),
            npx_bin: String::new(),
            console_url_pattern: String::new(),
            auto_open_console_url: false,
            pip_package: String::new(),
            pip_extra_packages: Vec::new(),
            pip_module: String::new(),
            env: Default::default(),
        };
        let root = std::env::temp_dir().join(format!("kira-inst-{}", std::process::id()));
        let deps = root.join(".deps");
        std::fs::create_dir_all(&deps).unwrap();
        assert!(!def.installed_in(&root), "无标记时不应视为已安装");
        std::fs::write(deps.join(".python-runtime.json"), r#"{"program":"py","prefix":[]}"#).unwrap();
        assert!(def.installed_in(&root), ".deps 运行时标记存在应为已安装");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_plan_python_launch_missing_when_no_marker_or_bad_program() {
        let root = std::env::temp_dir().join(format!("kira-pyplan-miss-{}", std::process::id()));
        std::fs::create_dir_all(&root);
        let venv_py = root.join(".venv").join("Scripts").join("python.exe");

        // 无 .deps → Missing
        assert_eq!(plan_python_launch(&root, &venv_py), PythonLaunchPlan::Missing);

        // 有 .deps 但 marker 指向不存在的解释器 → 仍 Missing（启动必然失败）
        let deps = root.join(".deps");
        std::fs::create_dir_all(&deps);
        let marker = serde_json::json!({ "program": root.join("gone.exe").to_string_lossy(), "prefix": [] });
        std::fs::write(deps.join(".python-runtime.json"), marker.to_string()).unwrap();
        assert_eq!(plan_python_launch(&root, &venv_py), PythonLaunchPlan::Missing);

        let _ = std::fs::remove_dir_all(&root);
    }
}
