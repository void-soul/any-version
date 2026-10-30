
export interface Config {
  versions_dir?: string;
  links_dir?: string;
  data_dir?: string;
  sdk_dir?: string;
  node_projects_dir?: string;
}

export interface MigrateResult {
  moved_versions: boolean;
  moved_links: boolean;
  recreated_junctions: string[];
  updated_env_vars: string[];
  updated_path_entries: string[];
  errors: string[];
  old_dirs_remain: string[];
}

export interface MigrateProgress {
  stage: string;
  current: number;
  total: number;
  file_name: string;
}

/** 托盘右键菜单配置（与后端 TrayMenuConfig 对应） */
export interface TrayMenuConfig {
  /** 是否启用托盘右键菜单（总开关） */
  enabled: boolean;
  /** 是否在托盘菜单显示 Mihomo 项（细分项已迁至代理模块设置） */
  show_mihomo: boolean;
}
