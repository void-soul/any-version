import { listen } from "@tauri-apps/api/event";
import type { ProjectStatus, ProjectDetail, ProjectDef, ManagePreview } from "./types";
import type { SubTabProps } from "./ProjectSubTabs";
export type SubTab = "versions" | "envvars" | "services" | "config" | string;

export const baseTabKeys: Record<string, string> = {
  versions: "projdetail.tabVersions",
  envvars: "projdetail.tabEnvVars",
  legacy: "projdetail.tabLegacy",
  services: "projdetail.tabServices",
  data_dirs: "projdetail.tabDataDirs",
  config: "projdetail.tabConfig",
};

// 判断项目是否支持版本管理（下载多版本 / Git 仓库 / 远程版本清单 / GitHub assets / npm 包）。
// mysql 等 SDK 使用 version_url_prefix_map + remote_versions_config 提供版本，
// npm 包（npm_pkg_name 型）使用 npm install 安装，
// Ghidra 用的是 remote_versions_config + github_asset —— 其 tag 是 `Ghidra_12.1.4_build`
// 而资产叫 `ghidra_12.1.4_PUBLIC_20260921.zip`，中间那个构建日期既不在 tag 也不在
// published_at 里，只有 github_asset 能表达（见 types.rs 的 github_asset 注释）。
// 这三个字段任一存在都须纳入判定，否则托管后会被误判为「简单托管」而看不到下载入口。
export function hasVersionSupportOf(def: ProjectDef | null | undefined): boolean {
  if (!def) return false;
  return !!(
    def.download_url_template ||
    def.is_git_repo ||
    def.version_url_prefix_map ||
    def.npm_pkg_name ||
    def.remote_versions_config ||
    def.github_asset
  );
}

export interface ProjectUIState {
  detail: ProjectDetail | null;
  detailLoaded: boolean;
  detailLoading: boolean;
  activeSubTab: SubTab;
  remoteVersions: string[];
  loadingRemote: boolean;
  versionsUpdatedAt: number | null;
  installingVersion: string | null;
  downloadProgress: { sdk: string; downloaded: number; total: number; pct: number; speed_str: string } | null;
  installStep: string;
  showManagePreview: boolean;
  managePreview: ManagePreview | null;
  managing: boolean;
  unmanaging: boolean;
  isSimpleManage: boolean;
  cacheDestPath: string;
  migratingCache: boolean;
  packages: SubTabProps["packages"];
  loadingPackages: boolean;
  upgradingPackage: string | null;
  packageError: string | null;
  serviceCtrlLoading: boolean;
  switchingVersion: string | null;
  repairingEnv: boolean;
  // 检测进度
  detectStep: string;
  detectIndex: number;
  detectTotal: number;
}

export const EMPTY_UI: ProjectUIState = {
  detail: null,
  detailLoaded: false,
  detailLoading: false,
  activeSubTab: "versions",
  remoteVersions: [],
  loadingRemote: false,
  versionsUpdatedAt: null,
  installingVersion: null,
  downloadProgress: null,
  installStep: "",
  showManagePreview: false,
  managePreview: null,
  managing: false,
  unmanaging: false,
  isSimpleManage: false,
  cacheDestPath: "",
  migratingCache: false,
  packages: [],
  loadingPackages: false,
  upgradingPackage: null,
  packageError: null,
  serviceCtrlLoading: false,
  switchingVersion: null,
  repairingEnv: false,
  detectStep: "",
  detectIndex: 0,
  detectTotal: 0,
};

export let _onProgress: ((p: { sdk: string; downloaded: number; total: number; pct: number; speed_str: string }) => void) | null = null;

export let _onStep: ((s: { sdk: string; step: string }) => void) | null = null;

export let _unlistenProgress: (() => void) | null = null;

export let _unlistenStep: (() => void) | null = null;

export function ensureListeners(
  onProgress: (p: { sdk: string; downloaded: number; total: number; pct: number; speed_str: string }) => void,
  onStep: (s: { sdk: string; step: string }) => void,
) {
  _onProgress = onProgress;
  _onStep = onStep;
  if (!_unlistenProgress) {
    listen<{ sdk: string; downloaded: number; total: number; pct: number; speed_str: string }>("download-progress", (e) => {
      _onProgress?.(e.payload);
    }).then((u) => { _unlistenProgress = u; });
  }
  if (!_unlistenStep) {
    listen<{ sdk: string; step: string }>("install-step", (e) => {
      _onStep?.(e.payload);
    }).then((u) => { _unlistenStep = u; });
  }
}

export interface Props {
  project: ProjectStatus | null;
  onRefresh: () => Promise<void>;
  onProjectUpdate?: (id: string) => Promise<void>;
}
