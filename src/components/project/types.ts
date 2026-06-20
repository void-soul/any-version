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
}

export interface PackageManagerDef {
  id: string;
  display_name: string;
  install_cmd: string | null;
  version_cmd: string | null;
  cache_detect_cmd: string | null;
  pkg_list_cmd: string | null;
  mirror_cmd_template: string | null;
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
  env_vars_status: EnvVarStatus[];
  cache_status: CacheStatus | null;
  service_status: ServiceStatus | null;
}

export interface ProjectDef {
  id: string;
  display_name: string;
  category: ProjectCategory;
  official_website: string;
  env_vars: Array<{ name: string; desc: string; check_type: string }>;
  has_cache: boolean;
  has_mirror: boolean;
  has_pkg: boolean;
  is_service: boolean;
  default_port: number | null;
  package_managers: PackageManagerDef[];
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