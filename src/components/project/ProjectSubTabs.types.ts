import type { ProjectStatus, ProjectDef } from "./types";

export interface SubTabProps {
  project: ProjectStatus;
  def: ProjectDef | null;
  // 版本管理
  remoteVersions: string[];
  loadingRemote: boolean;
  installingVersion: string | null;
  onInstall: (version: string) => void;
  onUninstall: (version: string) => void;
  onUse: (version: string) => void;
  // 下载进度
  downloadProgress: { sdk: string; downloaded: number; total: number; pct: number; speed_str: string } | null;
  installStep: string;
  onCancelInstall?: () => void;
  // 远程版本列表缓存
  versionsUpdatedAt?: number | null;
  onRefreshRemoteVersions?: () => void;
  // 包管理
  packages: Array<{ name: string; current_version: string; latest_version: string; status: string; homepage: string }>;
  loadingPackages: boolean;
  upgradingPackage: string | null;
  packageError: string | null;
  onRefreshPackages: () => void;
  onUpgradePackage: (name: string) => void;
  // 缓存管理
  cacheDestPath: string;
  migratingCache: boolean;
  onCacheDestPathChange: (v: string) => void;
  onMigrateCache: () => void;
  // 服务管理
  serviceCtrlLoading: boolean;
  onServiceToggle: () => void;
  // 刷新
  onRefresh: () => void;
  // 环境变量修复
  repairingEnv?: boolean;
  onRepairEnv?: () => void;
  /** 操作进行中，禁用按钮 */
  isOperating?: boolean;
  /** 当前活跃标签页 */
  activeSubTab?: string;
  /** 通知父组件当前切换到的标签页（用于懒加载） */
  onActiveSubTabChange?: (tab: string) => void;
}
