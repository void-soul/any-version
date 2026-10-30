import { Bot, Boxes, Globe } from "lucide-react";
// ---- 类型（与后端 node_manager.rs 对应，serde camelCase）----

export interface NodeProjectDef {
  id: string;
  displayName: string;
  repo: string;
  website: string;
  icon: string;
  description: string;
  defaultPort: number;
  webPath: string;
  nodeRequirement: string;
  /** 运行时：node（默认）/ python（venv + pip） */
  runtime?: string;
  packageManager: string;
  buildScript: string;
  startCmd: string[];
  managed: boolean;
  npxPackage: string;
  npxBin: string;
  /** pip 包模式：PyPI 包名（如 headroom-ai[proxy]），直接从 PyPI 安装、无需 clone */
  pipPackage?: string;
  /** pip 包模式下的 -m 模块路径（如 headroom.cli） */
  pipModule?: string;
  /// 控制台 URL 提取正则（第 1 捕获组 = 带凭据主页地址）；空 = 不提取。
  consoleUrlPattern: string;
  /// 捕获到控制台 URL 后是否自动用系统浏览器打开。
  autoOpenConsoleUrl: boolean;
  /** 首次初始化命令（如 wigolo 的 ["init"]）；空 = 该服务无需初始化 */
  initCmd?: string[];
  /** 初始化完成标记（`~` 可展开）；空 = 不做判定 */
  initMarker?: string;
  /** Docker 运行方式专属配置（runtime === "docker" 时存在） */
  docker?: {
    project: string;
    healthPort: number;
    volumes: string[];
    /** 「执行命令」落到哪个容器；空 = 不支持执行命令 */
    execService: string;
  } | null;
}


export interface DepCheck {
  name: string;
  exists: boolean;
  path?: string | null;
  version?: string | null;
  satisfies: boolean;
  requirement?: string | null;
}


export interface DepCheckResult {
  git: DepCheck;
  node: DepCheck;
  packageManager: DepCheck;
  allReady: boolean;
}


export interface NodeProjectStatus {
  id: string;
  displayName: string;
  installed: boolean;
  status: string; // "running" | "stopped" | "not_installed" | "port_conflict"
  port?: number | null;
  /** port_conflict 时占用进程名 */
  conflictProcess?: string | null;
  pid?: number | null;
  gitVersion?: string | null;
  localVersion?: string | null;
  error?: string | null;
  /** 是否配置了首次初始化命令（据此决定是否显示「初始化」入口） */
  initAvailable: boolean;
  /** 是否已完成初始化；未配置 initCmd/initMarker 的服务恒为 true */
  initialized: boolean;
  /** docker 服务：在跑的容器数；非 docker 恒为 0 */
  containers?: number;
}


/// 本机 Docker 环境（后端 docker_env_check；与具体服务无关）
export interface DockerEnv {
  cli: boolean;
  cliVersion: string;
  compose: boolean;
  composeVersion: string;
  daemonRunning: boolean;
  /** 一句话结论，环境条直接显示 */
  summary: string;
  error: string;
}


/// 某个 docker 服务的可编辑配置（端口 / 数据库密码）
export interface DockerServiceConfig {
  id: string;
  port: number;
  /** 第二个对外端口；0 = 该服务只发布一个端口（此时弹窗不显示这个输入框） */
  port2: number;
  pgPassword: string;
  dir: string;
  composePath: string;
  envPath: string;
  prepared: boolean;
}


export interface NodeUpdateInfo {
  hasUpdate: boolean;
  currentCommit: string;
  latestCommit: string;
  behind: number;
  currentVersion?: string | null;
  latestVersion?: string | null;
  error?: string | null;
}


export interface NodeProgress {
  projectId: string;
  phase: string;
  detail: string;
}


export interface NodeLog {
  projectId: string;
  phase: string;
  line: string;
}


/// 控制台 URL 捕获事件（npm-console-url）。
export interface NodeConsoleUrl {
  projectId: string;
  url: string;
}


/// 每个项目日志保留的最大行数。
export const MAX_LOG_LINES = 800;


export const ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  bot: Bot,
  boxes: Boxes,
  globe: Globe,
};


/// 渲染 webPath，替换 {port} 占位符（与后端 NodeProjectDef::resolved_web_path 一致）。
///
/// docker 服务必须传入**配置里的实际端口**：用户可能在配置弹窗里改过端口，
/// 那时 defaultPort 只是初始默认值，拿它拼 URL 会打开一个没人监听的地址。
export function resolvedWebPath(p: NodeProjectDef, port?: number | null): string {
  return p.webPath.replace("{port}", String(port ?? p.defaultPort));
}


/// 服务管理固定标签页 id（不可关闭）
export const MANAGE_TAB = "__manage__";


/// 判断服务端口是否处于监听（running 或 port_conflict 均视为有进程占用端口）。
export function isPortListening(st?: NodeProjectStatus): boolean {
  return st?.status === "running" || st?.status === "port_conflict";
}


/// 把 Docker 环境探测结果包装成 EnvBadge 需要的形状 —— 复用同一套徽章视觉，
/// 不另造一套（docker 服务的环境条与其它服务保持一致的心智）。
export function dockerDep(version: string, ok: boolean): DepCheck {
  return {
    name: "docker",
    exists: ok,
    path: null,
    version: version || null,
    satisfies: ok,
    requirement: null,
  };
}
