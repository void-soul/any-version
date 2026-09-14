// ============================================================
// Kira 项目管理 - TypeScript 类型定义
// ============================================================

export type ProjectCategory = "language" | "tool" | "service" | "ai_tool";

export interface EnvVarStatus {
  name: string;
  desc: string;
  value: string | null;
  source: string; // "HKCU" | "HKLM" | "未设置"
  exists: boolean;
  in_anyversion: boolean;
  tier?: "core" | "package" | "compat" | "clear";
}

export interface CacheStatus {
  path: string;
  size: string;
  is_link: boolean;
  real_target: string;
  detect_source: string;
}

export interface ServiceStatus {
  running: boolean;
  port: number | null;
  pid: number | null;
  data_dir: string;
  log_dir: string;
  status?: "running" | "stopped" | "not_installed" | "port_conflict" | "external_running" | string | null;
  external?: boolean;
  process_name?: string | null;
  install_root?: string | null;
  config_file?: string | null;
  system_service_name?: string | null;
}

/**
 * 包管理器定义 —— 字段与 Rust 侧 `commands/project/types.rs::PackageManagerDef` 一一对应。
 * 新增/改名配置字段时必须两边同步，否则前端会读到 undefined（静默失效）。
 */
export interface PackageManagerDef {
  id: string;
  display_name: string;
  built_in?: boolean;
  install_cmd: string | null;
  version_cmd: string | null;
  version_exe?: string | null;
  cache_detect_cmd: string | null;
  cache_detect_json_path?: string | null;
  pkg_list_cmd: string | null;
  mirror_cmd_template: string | null;
  mirror_detect_cmd?: string | null;
  mirror_options: Array<{ mirror_type: string; name: string; url: string }> | null;
  // 缓存路径
  cache_default_path: string | null;
  cache_env_var: string | null;
  cache_set_cmd_template: string | null;
  // 附加缓存目录（一个包管理器可有多个缓存，如 pnpm 的 store + 元数据 cache-dir）
  extra_caches?: Array<{
    id: string;
    display_name: string;
    detect_cmd: string | null;
    detect_json_path?: string | null;
    default_path: string | null;
    env_var: string | null;
    set_cmd_template: string | null;
  }>;
  // 数据文件路径
  data_detect_cmd: string | null;
  data_default_path: string | null;
  data_env_var: string | null;
  data_set_cmd_template: string | null;
  // 代理配置
  proxy_detect_cmd: string | null;
  proxy_set_cmd_template: string | null;
  proxy_clear_cmd?: string | null;
  // 全局依赖包列表 / 升级
  pkg_list_format?: string | null;
  pkg_upgrade_cmd_template?: string | null;
  pkg_homepage_template?: string | null;
  pkg_outdated_cmd?: string | null;
  pkg_outdated_format?: string | null;
  // 镜像配置文件（{home} / {roaming_appdata} 等目录根占位符由后端统一展开）
  mirror_config_file?: string | null;
  mirror_detect_file_regex?: string | null;
  mirror_config_desc?: string | null;
  // 基于配置文件的缓存解析器（nuget maven 等）
  cache_config_source?: Record<string, unknown> | null;
  // 通过运行时参数执行包管理器（如 ["-m", "pip"]）
  run_via_runtime_args?: string[] | null;
  remote_versions_config?: Record<string, unknown> | null;
}

export interface DataDirStatus {
  id: string;
  display_name: string;
  path: string;
  size: string;
  is_link: boolean;
  real_target: string;
  exists: boolean;
  kind?: "data" | "log" | "config" | "cache" | string | null;
  source?: string | null;
}

export interface ProjectDelegation {
  env_vars: string[];
  path_vars: string[];
  version_control: boolean;
  create_symlink: boolean;
  manage_install_dir: boolean;
  manage_data_dir: boolean;
  manage_cache_dir: boolean;
  manage_optional_tools: string[];
}

export interface ProjectStatus {
  id: string;
  display_name: string;
  category: ProjectCategory;
  installed: boolean;
  active_version: string | null;
  installed_versions: string[];
  install_source: string | null; // "Scoop" | "AnyVersion" | "手动" | ...
  install_root: string | null;
  managed: boolean;
  is_simple_managed: boolean;
  env_vars_status: EnvVarStatus[];
  cache_status: CacheStatus | null;
  service_status: ServiceStatus | null;
  data_dirs_status?: DataDirStatus[];
  show_version?: boolean;
  show_service?: boolean;
  delegation: ProjectDelegation;
}

export interface UserConfigurableVar {
  name: string;
  desc: string;
  placeholder?: string;
  options?: string[];
  var_type?: string; // "boolean" | undefined (free text)
  current_value?: string;
  source?: string;
}

export interface DataDirDef {
  id: string;
  display_name: string;
  possible_paths: string[];
  default_path: string;
  env_var?: string;
  kind?: "data" | "log" | "config" | "cache" | string | null;
  supports_direct?: boolean;
  auto_create?: boolean | null;
  required_for_start?: boolean;
}

export interface ConflictManagerDef {
  id: string;
  display_name: string;
  env_vars: string[];
  path_keywords: string[];
  exe_name?: string | null;
  /** 缓存/工具链目录的默认路径模板（允许 {home} / {program_files} 等目录根占位符） */
  cache_default_path?: string | null;
  /** 缓存位置对应的环境变量名；缺省时后端回退到 env_vars 首项 */
  cache_env_var?: string | null;
}

export interface ProjectDef {
  id: string;
  display_name: string;
  category: ProjectCategory;
  official_website: string;
  simple_mode?: boolean;
  is_git_repo?: boolean;
  bootstrap_cmd?: string;
  env_vars: Array<{
    name: string;
    desc: string;
    /** "path" | "nonempty" | "runtime" */
    check_type: string;
    tier?: "core" | "package" | "compat" | "clear";
    /** 托管时的值子目录（相对于 link_dir）；不填则值 = link_dir */
    sub_dir?: string | null;
  }>;
  bin_dirs: string[];
  has_cache: boolean;
  has_mirror: boolean;
  has_pkg: boolean;
  is_service: boolean;
  default_port: number | null;
  package_managers: PackageManagerDef[];
  user_configurable_vars?: UserConfigurableVar[];
  data_dirs?: DataDirDef[];
  service_process_exes?: string[];
  config_file_candidates?: string[];
  service_start_mode?: "wait" | "detached" | string | null;
  service_allow_force_kill?: boolean;
  service_auto_create_dirs?: boolean;
  conflict_managers?: ConflictManagerDef[];
  // npm 包类型：存在时该项目通过 `npm install --prefix` 安装（如 GitNexus）
  npm_pkg_name?: string;
  pkg_manager?: string;
  // ... 其他字段
  [key: string]: unknown;
}

export interface ProjectDetail {
  def: ProjectDef;
  status: ProjectStatus;
}

export interface ManagePreview {
  steps: Array<{
    action: string;
    description: string;
    target: string;
  }>;
  has_local_install: boolean;
  local_install_root: string | null;
  local_install_source: string | null;
}

export function categoryLabel(cat: ProjectCategory): string {
  switch (cat) {
    case "language":
      return "语言";
    case "tool":
      return "工具";
    case "service":
      return "服务";
    case "ai_tool":
      return "AI";
    default:
      return cat;
  }
}