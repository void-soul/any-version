use super::*;

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
    /// 第二个对外端口的默认值（部分服务要发布两个，如 ARTEX 的界面 + API）。
    /// 0 = 只发布第一个端口；**别再把第二个端口写死** —— 端口会撞车：
    /// ARTEX 原本写死 8788，而 Kira 自带的 twoapi 代理默认就监听 8788。
    #[serde(default)]
    pub default_port2: u16,
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
    /// 首次初始化命令（如 wigolo 的 `init`：下载浏览器引擎与端上模型，约 1.5 GB）。
    ///
    /// npx 模式按 `<npxBin> {init_cmd...}` 在运行目录里执行；空表示该服务无需初始化
    /// （此时前端不显示「初始化」按钮，行为与既有服务完全一致）。
    #[serde(default)]
    pub init_cmd: Vec<String>,
    /// 初始化完成标记：`~` 已展开，路径存在即视为已初始化；空表示不做判定。
    #[serde(default)]
    pub init_marker: String,
    /// Docker 运行方式专属配置（`runtime == "docker"` 时必填）。
    /// 通用实现在 `commands/docker.rs`（不针对任何具体服务）。
    #[serde(default)]
    pub docker: Option<DockerSpec>,
}

/// Docker 运行方式专属配置。
///
/// 模板（`compose` / `env`）写在定义文件里，而不是要求用户先 `git clone` 仓库：
/// 这类自托管服务通常只需要一份 compose，而且像 ARTEX 的仓库已声明为「最后一版源码备份」。
/// 模板在安装时渲染到 `<数据目录>/docker/<id>/`，占位符见 `docker::render_template`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerSpec {
    /// compose 项目名（`docker compose -p`）：与用户自己的 compose 项目隔离，避免串台。
    /// 空 = 用服务 id。
    #[serde(default)]
    pub project: String,
    /// 对外端口（容器端口可与服务端口不同）；0 = 用 defaultPort
    #[serde(default)]
    pub health_port: u16,
    /// 需要持久化的挂载目录名（相对服务目录），安装时创建
    #[serde(default)]
    pub volumes: Vec<String>,
    /// compose 模板（含 `{{port}}` / `{{pg_password}}` / `{{dir}}` 占位符）
    #[serde(default)]
    pub compose: String,
    /// `.env` 模板（占位符同上）
    #[serde(default)]
    pub env: String,
    /// 「执行命令」落到哪个容器（compose 里的服务名）；空 = 该服务不支持执行命令
    #[serde(default)]
    pub exec_service: String,
}

impl NodeProjectDef {
    /// 是否启用控制台 URL 提取。
    pub fn has_console_url_pattern(&self) -> bool {
        !self.console_url_pattern.trim().is_empty()
    }
}

pub(super) fn default_port() -> u16 {
    3000
}
pub(super) fn default_web_path() -> String {
    "http://127.0.0.1:{port}".to_string()
}
pub(super) fn default_pm() -> String {
    "pnpm".to_string()
}
pub(super) fn default_true() -> bool {
    true
}

impl NodeProjectDef {
    /// 渲染 webPath，替换 {port} 占位符。
    /// 端口用 `effective_port()`：docker 服务的对外端口可能是 healthPort，
    /// 用户还可能在配置弹窗里改过（改后的值在 kira.json 里，由状态接口带回前端）。
    pub fn resolved_web_path(&self) -> String {
        self.web_path
            .replace("{port}", &self.effective_port().to_string())
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

    /// 是否为 Docker 运行时（runtime = "docker"）：服务由 `docker compose` 托管，
    /// 本机需有可用的 Docker Desktop。与 node / python 并列的第三种运行方式。
    pub fn is_docker(&self) -> bool {
        self.runtime.trim().eq_ignore_ascii_case("docker") && self.docker.is_some()
    }

    /// 服务对外端口：docker 用 `healthPort`（容器端口可与服务端口不同），其余用 defaultPort。
    pub fn effective_port(&self) -> u16 {
        self.docker
            .as_ref()
            .map(|d| d.health_port)
            .filter(|p| *p > 0)
            .unwrap_or(self.default_port)
    }

    /// 投影成 docker 层的输入 —— `docker.rs` 因此不必知道本结构长什么样，
    /// 这里加字段也不会连带改它。
    pub fn docker_spec(&self) -> Option<crate::commands::docker::ServiceSpec<'_>> {
        let d = self.docker.as_ref()?;
        Some(crate::commands::docker::ServiceSpec {
            id: &self.id,
            port: self.effective_port(),
            port2: self.default_port2,
            project: if d.project.trim().is_empty() {
                self.id.as_str()
            } else {
                d.project.as_str()
            },
            volumes: &d.volumes,
            compose_tpl: &d.compose,
            env_tpl: &d.env,
        })
    }

    /// 是否为 pip 包模式（配置了 pipPackage）：直接从 PyPI 安装，无需 git clone。
    pub fn is_pip_package(&self) -> bool {
        !self.pip_package.trim().is_empty()
    }

    /// 是否配置了首次初始化命令。
    pub fn has_init(&self) -> bool {
        !self.init_cmd.is_empty()
    }

    /// 初始化标记路径（展开开头的 `~` / `~\`）；未配置 marker 时返回 None。
    pub fn init_marker_path(&self) -> Option<PathBuf> {
        let marker = self.init_marker.trim();
        if marker.is_empty() {
            return None;
        }
        if let Some(tail) = marker
            .strip_prefix("~/")
            .or_else(|| marker.strip_prefix("~\\"))
        {
            return Some(crate::commands::utils::get_home_dir().join(tail));
        }
        Some(PathBuf::from(marker))
    }

    /// 是否已完成初始化。
    ///
    /// 未配置 `initCmd`（不需要初始化）或未配置 `initMarker`（无法判定）时都返回 true ——
    /// 这样既有服务的行为完全不变，只有显式声明了两者的服务才会出现「初始化」入口。
    pub fn initialized(&self) -> bool {
        if !self.has_init() {
            return true;
        }
        match self.init_marker_path() {
            Some(marker) => marker.exists(),
            None => true,
        }
    }

    /// 初始化命令行（`initCmd` 的展示形态，如 `["init"]` → `init`）。
    ///
    /// npx 模式带上包自带的 bin 名（`wigolo init`）；其余模式直接拼 `initCmd`。
    pub fn init_command_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.is_npx() {
            parts.push(self.npx_bin_name());
        }
        parts.extend(self.init_cmd.iter().cloned());
        parts.join(" ")
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
            .join(if cfg!(windows) {
                "python.exe"
            } else {
                "python"
            })
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
        // docker 服务的「安装」产物是渲染出来的 compose 文件，它落在数据目录
        // （`<data>/docker/<id>`）而不是 node-projects 托管目录里。
        if self.is_docker() {
            return crate::commands::docker::docker_dir(&self.id)
                .join("docker-compose.yml")
                .is_file();
        }
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
                .join(if cfg!(windows) {
                    "python.exe"
                } else {
                    "python"
                });
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
    /// docker 服务：在跑的容器数（`docker compose ps -q` 的行数）；非 docker 恒为 0
    #[serde(default)]
    pub containers: usize,
    /// 是否已配置首次初始化命令（前端据此决定是否显示「初始化」入口）。
    #[serde(default)]
    pub init_available: bool,
    /// 是否已完成初始化；未配置 initCmd/initMarker 的服务恒为 true。
    #[serde(default)]
    pub initialized: bool,
}
