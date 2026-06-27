// ============================================================
// AnyVersion 项目管理 - TypeScript 类型定义
// ============================================================

export type ProjectCategory = "language" | "tool" | "service";

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
  status?: "running" | "stopped" | "not_installed" | "port_conflict" | string | null;
  process_name?: string | null;
  install_root?: string | null;
  config_file?: string | null;
}

export interface PackageManagerDef {
  id: string;
  display_name: string;
  built_in?: boolean;
  install_cmd: string | null;
  upgrade_cmd: string | null;
  latest_version_cmd: string | null;
  version_cmd: string | null;
  cache_detect_cmd: string | null;
  pkg_list_cmd: string | null;
  mirror_cmd_template: string | null;
  mirror_detect_cmd?: string | null;
  mirror_options: Array<{ mirror_type: string; name: string; url: string }> | null;
  // 缓存路径
  cache_default_path: string | null;
  cache_env_var: string | null;
  cache_set_cmd_template: string | null;
  // 数据文件路径
  data_detect_cmd: string | null;
  data_default_path: string | null;
  data_env_var: string | null;
  data_set_cmd_template: string | null;
  // 代理配置
  proxy_detect_cmd: string | null;
  proxy_set_cmd_template: string | null;
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

export interface ProjectDef {
  id: string;
  display_name: string;
  category: ProjectCategory;
  official_website: string;
  simple_mode?: boolean;
  is_git_repo?: boolean;
  bootstrap_cmd?: string;
  env_vars: Array<{ name: string; desc: string; check_type: string; tier?: "core" | "package" | "compat" | "clear" }>;
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
    default:
      return cat;
  }
}