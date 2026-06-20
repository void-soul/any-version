import React, { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ExternalLink,
  ShieldCheck,
  HelpCircle,
  RefreshCw,
  CheckCircle,
  Info,
  Loader,
  AlertTriangle,
  Settings,
} from "lucide-react";
import type {
  ProjectStatus,
  ProjectDetail,
  ProjectDef,
  ManagePreview,
} from "./types";
import { categoryLabel } from "./types";
import type { SubTabProps } from "./ProjectSubTabs";
import {
  VersionsTab,
  EnvVarsTab,
  CacheTab,
  MirrorTab,
  PackagesTab,
  ServicesTab,
  PkgMgrTab,
} from "./ProjectSubTabs";

type SubTab = "versions" | "envvars" | "cache" | "mirror" | "packages" | "services" | "pkgmgr";

const tabLabels: Record<SubTab, string> = {
  versions: "版本管理",
  pkgmgr: "包管理器",
  envvars: "环境变量",
  cache: "缓存管理",
  mirror: "镜像配置",
  packages: "全局包",
  services: "服务管理",
};

interface ProjectUIState {
  detail: ProjectDetail | null;
  detailLoaded: boolean;
  detailLoading: boolean;
  activeSubTab: SubTab;
  remoteVersions: string[];
  loadingRemote: boolean;
  installingVersion: string | null;
  downloadProgress: { sdk: string; downloaded: number; total: number; pct: number } | null;
  installStep: string;
  localVersion: string;
  localPath: string;
  registering: boolean;
  registerErr: string | null;
  showManagePreview: boolean;
  managePreview: ManagePreview | null;
  managing: boolean;
  unmanaging: boolean;
  cacheDestPath: string;
  migratingCache: boolean;
  packages: SubTabProps["packages"];
  loadingPackages: boolean;
  upgradingPackage: string | null;
  packageError: string | null;
  serviceCtrlLoading: boolean;
  switchingVersion: string | null;
  // 检测进度
  detectStep: string;
  detectIndex: number;
  detectTotal: number;
}

const EMPTY_UI: ProjectUIState = {
  detail: null,
  detailLoaded: false,
  detailLoading: false,
  activeSubTab: "versions",
  remoteVersions: [],
  loadingRemote: false,
  installingVersion: null,
  downloadProgress: null,
  installStep: "",
  localVersion: "",
  localPath: "",
  registering: false,
  registerErr: null,
  showManagePreview: false,
  managePreview: null,
  managing: false,
  unmanaging: false,
  cacheDestPath: "",
  migratingCache: false,
  packages: [],
  loadingPackages: false,
  upgradingPackage: null,
  packageError: null,
  serviceCtrlLoading: false,
  switchingVersion: null,
  detectStep: "",
  detectIndex: 0,
  detectTotal: 0,
};

let _onProgress: ((p: { sdk: string; downloaded: number; total: number; pct: number }) => void) | null = null;
let _onStep: ((s: string) => void) | null = null;
let _unlistenProgress: (() => void) | null = null;
let _unlistenStep: (() => void) | null = null;

function ensureListeners(
  onProgress: (p: { sdk: string; downloaded: number; total: number; pct: number }) => void,
  onStep: (s: string) => void,
) {
  _onProgress = onProgress;
  _onStep = onStep;
  if (!_unlistenProgress) {
    listen<{ sdk: string; downloaded: number; total: number; pct: number }>("download-progress", (e) => {
      _onProgress?.(e.payload);
    }).then((u) => { _unlistenProgress = u; });
  }
  if (!_unlistenStep) {
    listen<{ step: string }>("install-step", (e) => {
      _onStep?.(e.payload.step);
    }).then((u) => { _unlistenStep = u; });
  }
}

interface Props {
  project: ProjectStatus | null;
  onRefresh: () => Promise<void>;
  onProjectUpdate?: (id: string) => Promise<void>;
}

export default function ProjectDetailPanel({ project, onRefresh, onProjectUpdate }: Props) {
  const [uiMap, setUiMap] = useState<Record<string, ProjectUIState>>({});
  const eventProjectRef = useRef<string | null>(null);

  const pid = project?.id ?? null;
  const ui: ProjectUIState = pid ? (uiMap[pid] ?? EMPTY_UI) : EMPTY_UI;

  const patch = useCallback((id: string, partial: Partial<ProjectUIState>) => {
    setUiMap((prev) => ({ ...prev, [id]: { ...(prev[id] ?? EMPTY_UI), ...partial } }));
  }, []);

  useEffect(() => {
    ensureListeners(
      (payload) => {
        const cur = eventProjectRef.current;
        if (cur) patch(cur, { downloadProgress: payload });
      },
      (step) => {
        const cur = eventProjectRef.current;
        if (!cur) return;
        patch(cur, { installStep: step });
        if (step === "完成") {
          setTimeout(() => {
            patch(cur, { installingVersion: null, downloadProgress: null, installStep: "" });
            eventProjectRef.current = null;
          }, 1500);
        }
      },
    );
  }, [patch]);

  const loadDetail = useCallback(async (id: string) => {
    patch(id, { detailLoading: true });
    try {
      const d = await invoke<ProjectDetail>("project_detail", { id });
      patch(id, { detail: d, detailLoaded: true, detailLoading: false });
    } catch {
      patch(id, { detailLoading: false });
    }
  }, [patch]);

  // 刷新单个项目（增量更新列表 + 重新加载详情）
  const refreshSingle = useCallback(async (id: string) => {
    await loadDetail(id);
    if (onProjectUpdate) {
      await onProjectUpdate(id);
    }
  }, [loadDetail, onProjectUpdate]);

  const loadRemote = useCallback(async (id: string) => {
    patch(id, { loadingRemote: true });
    try {
      const v = await invoke<string[]>("project_list_remote_versions", { id });
      patch(id, { remoteVersions: v, loadingRemote: false });
    } catch {
      patch(id, { loadingRemote: false });
    }
  }, [patch]);

  const loadPackages = useCallback(async (id: string) => {
    patch(id, { loadingPackages: true, packageError: null });
    try {
      const list = await invoke<SubTabProps["packages"]>("get_global_packages", { sdkName: id });
      patch(id, { packages: list, loadingPackages: false });
    } catch (e: unknown) {
      patch(id, { packageError: String(e), loadingPackages: false, packages: [] });
    }
  }, [patch]);

  // 首次打开项目时的逐步检测队列
  const runInitialDetection = useCallback(async (id: string, proj: ProjectStatus) => {
    const steps: Array<{ label: string; run: () => Promise<void> }> = [];

    // 步骤1：加载项目详情（版本、环境变量等）
    steps.push({
      label: `正在检测 ${proj.display_name} 安装状态...`,
      run: async () => {
        try {
          const d = await invoke<ProjectDetail>("project_detail", { id });
          patch(id, { detail: d, detailLoaded: true });
        } catch {}
      },
    });

    // 步骤2：获取远程版本列表
    steps.push({
      label: `正在获取 ${proj.display_name} 远程版本列表...`,
      run: async () => {
        try {
          const v = await invoke<string[]>("project_list_remote_versions", { id });
          patch(id, { remoteVersions: v });
        } catch {}
      },
    });

    // 步骤3：标记全局包待检测（不在初始检测中执行，太慢）
    // 全局包检测改为懒加载：用户切换到"全局包"标签页时才触发

    patch(id, { detailLoading: true, loadingRemote: true, loadingPackages: true, detectTotal: steps.length });

    for (let i = 0; i < steps.length; i++) {
      patch(id, { detectIndex: i + 1, detectStep: steps[i].label });
      await steps[i].run();
    }

    patch(id, {
      detailLoading: false,
      loadingRemote: false,
      loadingPackages: false,
      detectStep: "",
      detectIndex: 0,
      detectTotal: 0,
    });
  }, [patch]);

  useEffect(() => {
    if (!pid || !project) return;
    const state = uiMap[pid];
    if (!state || (!state.detailLoaded && !state.detailLoading && !state.detectStep)) {
      runInitialDetection(pid, project);
    }
  }, [pid]);

  const handleInstall = useCallback(async (version: string) => {
    if (!pid) return;
    eventProjectRef.current = pid;
    patch(pid, { installingVersion: version, installStep: "下载中", downloadProgress: null });
    try {
      await invoke("project_install_version", { id: pid, version: version.split(" ")[0] });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("安装失败: " + e);
    } finally {
      setTimeout(() => {
        if (eventProjectRef.current === pid) {
          patch(pid, { installingVersion: null, downloadProgress: null, installStep: "" });
          eventProjectRef.current = null;
        }
      }, 3000);
    }
  }, [pid, patch, loadDetail, onRefresh]);

  const handleUninstall = useCallback(async (version: string) => {
    if (!pid || !ui.detail) return;
    if (!confirm("确定卸载 " + ui.detail.status.display_name + " v" + version + " 吗？")) return;
    try {
      await invoke("project_uninstall_version", { id: pid, version });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("卸载失败: " + e);
    }
  }, [pid, ui.detail, loadDetail, onRefresh]);

  const handleUse = useCallback(async (version: string) => {
    if (!pid) return;
    patch(pid, { switchingVersion: version, detectStep: `正在切换到 ${version}...` });
    try {
      await invoke("project_use_version", { id: pid, version });
      const detail = await invoke<ProjectDetail>("project_detail", { id: pid });
      patch(pid, { detail, switchingVersion: null, detectStep: "" });
      if (onProjectUpdate) await onProjectUpdate(pid);
    } catch (e: unknown) {
      alert("切换版本失败: " + e);
      patch(pid, { switchingVersion: null, detectStep: "" });
    }
  }, [pid, patch, loadDetail, onRefresh]);

  const handleRegisterLocal = useCallback(async () => {
    if (!pid) return;
    const s = uiMap[pid];
    if (!s || !s.localVersion || !s.localPath) return;
    patch(pid, { registering: true, registerErr: null });
    try {
      await invoke("project_register_local", { id: pid, version: s.localVersion.trim(), localPath: s.localPath.trim() });
      patch(pid, { localVersion: "", localPath: "", registering: false });
      await loadDetail(pid);
      await onRefresh();
    } catch (e: unknown) {
      patch(pid, { registerErr: String(e), registering: false });
    }
  }, [pid, uiMap, patch, loadDetail, onRefresh]);

  const handlePreviewManage = useCallback(async () => {
    if (!pid) return;
    try {
      const preview = await invoke<ManagePreview>("project_preview_manage", { id: pid });
      patch(pid, { managePreview: preview, showManagePreview: true });
    } catch {
      patch(pid, { managePreview: null, showManagePreview: true });
    }
  }, [pid, patch]);

  const handleManage = useCallback(async () => {
    if (!pid) return;
    patch(pid, { managing: true });
    try {
      await invoke("project_manage", { id: pid });
      patch(pid, { showManagePreview: false, managePreview: null, managing: false });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("托管操作失败: " + e);
      patch(pid, { managing: false });
    }
  }, [pid, patch, loadDetail, onRefresh]);

  const handleUnmanage = useCallback(async () => {
    if (!pid || !ui.detail) return;
    if (!confirm("确定取消对 " + ui.detail.status.display_name + " 的托管吗？")) return;
    patch(pid, { unmanaging: true });
    try {
      await invoke("project_unmanage", { id: pid });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("取消托管失败: " + e);
    } finally {
      patch(pid, { unmanaging: false });
    }
  }, [pid, ui.detail, patch, loadDetail, onRefresh]);

  const handleServiceToggle = useCallback(async () => {
    if (!pid || !ui.detail?.status?.service_status) return;
    const running = ui.detail.status.service_status.running;
    patch(pid, { serviceCtrlLoading: true });
    try {
      if (running) {
        await invoke("stop_service", { name: pid });
      } else {
        if (!ui.detail.status.active_version) {
          alert("请先启用一个版本，然后才能启动服务");
          patch(pid, { serviceCtrlLoading: false });
          return;
        }
        await invoke("start_service", { name: pid, version: ui.detail.status.active_version });
      }
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("服务操作失败: " + e);
    } finally {
      patch(pid, { serviceCtrlLoading: false });
    }
  }, [pid, ui.detail, patch, loadDetail, onRefresh]);

  const handleMigrateCache = useCallback(async () => {
    if (!pid) return;
    const s = uiMap[pid];
    if (!s || !s.cacheDestPath) return;
    if (s.cacheDestPath.toLowerCase().startsWith("c:")) {
      alert("目标路径必须位于非 C 盘");
      return;
    }
    patch(pid, { migratingCache: true });
    try {
      await invoke("migrate_cache_path", { name: pid, newPath: s.cacheDestPath });
      await loadDetail(pid);
    } catch (e: unknown) {
      alert("缓存迁移失败: " + e);
    } finally {
      patch(pid, { migratingCache: false });
    }
  }, [pid, uiMap, patch, loadDetail]);

  const handleUpgradePackage = useCallback(async (pkgName: string) => {
    if (!pid) return;
    patch(pid, { upgradingPackage: pkgName, packageError: null });
    try {
      await invoke("upgrade_global_package", { sdkName: pid, pkgName });
      await loadPackages(pid);
    } catch (e: unknown) {
      patch(pid, { packageError: "升级 " + pkgName + " 失败: " + e });
    } finally {
      patch(pid, { upgradingPackage: null });
    }
  }, [pid, patch, loadPackages]);

  if (!pid || !project) {
    return (
      <div className="h-full flex items-center justify-center border-l border-white/5 text-slate-500 select-none">
        <div className="text-center space-y-3">
          <Settings className="w-12 h-12 mx-auto text-slate-600" />
          <p className="text-xs font-medium text-slate-400">{"请从左侧选择一个项目"}</p>
          <p className="text-[10px] text-slate-600">{"选中后此处将展示该项目的详细信息与管理功能。"}</p>
        </div>
      </div>
    );
  }

  const def: ProjectDef | null = ui.detail?.def ?? null;
  const status: ProjectStatus = ui.detail?.status ?? project;

  const availableTabs: SubTab[] = ["versions"];
  if (def?.package_managers && def.package_managers.length > 0) availableTabs.push("pkgmgr");
  availableTabs.push("envvars");
  if (def?.has_cache) availableTabs.push("cache");
  if (def?.has_mirror) availableTabs.push("mirror");
  if (def?.has_pkg) availableTabs.push("packages");
  if (def?.is_service || project?.service_status) availableTabs.push("services");

  const isOperating = !!ui.installingVersion || !!ui.switchingVersion || ui.managing || ui.unmanaging || !!ui.detectStep;

  const subTabProps: SubTabProps = {
    project: status,
    def,
    remoteVersions: ui.remoteVersions,
    loadingRemote: ui.loadingRemote,
    installingVersion: ui.installingVersion,
    downloadProgress: ui.downloadProgress,
    installStep: ui.installStep,
    onInstall: handleInstall,
    onUninstall: handleUninstall,
    onUse: handleUse,
    localVersion: ui.localVersion,
    localPath: ui.localPath,
    registering: ui.registering,
    registerErr: ui.registerErr,
    onLocalVersionChange: (v: string) => patch(pid!, { localVersion: v }),
    onLocalPathChange: (v: string) => patch(pid!, { localPath: v }),
    onRegisterLocal: handleRegisterLocal,
    packages: ui.packages,
    loadingPackages: ui.loadingPackages,
    upgradingPackage: ui.upgradingPackage,
    packageError: ui.packageError,
    onRefreshPackages: () => loadPackages(pid!),
    onUpgradePackage: handleUpgradePackage,
    cacheDestPath: ui.cacheDestPath,
    migratingCache: ui.migratingCache,
    onCacheDestPathChange: (v: string) => patch(pid!, { cacheDestPath: v }),
    onMigrateCache: handleMigrateCache,
    serviceCtrlLoading: ui.serviceCtrlLoading,
    onServiceToggle: handleServiceToggle,
    onRefresh: async () => { if (pid) await loadDetail(pid); },
    isOperating,
  };

  return (
    <div className="h-full flex flex-col overflow-hidden">
      <div className="p-5 border-b border-white/5 bg-white/2 flex-shrink-0">
        <div className="flex items-center justify-between">
          <div>
            <div className="flex items-center gap-2">
              <h3 className="text-base font-semibold text-white">{status.display_name}</h3>
              <span className={`px-1.5 py-0.5 rounded text-[9px] font-semibold border ${
                status.category === "language"
                  ? "bg-blue-500/10 text-blue-400 border-blue-500/20"
                  : status.category === "tool"
                  ? "bg-amber-500/10 text-amber-400 border-amber-500/20"
                  : "bg-purple-500/10 text-purple-400 border-purple-500/20"
              }`}>
                {categoryLabel(status.category)}
              </span>
              {status.managed ? (
                <span className="px-1.5 py-0.5 rounded text-[9px] bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-bold flex items-center gap-0.5">
                  <ShieldCheck className="w-2.5 h-2.5" /> {"已托管"}
                </span>
              ) : (
                <span className="px-1.5 py-0.5 rounded text-[9px] bg-amber-500/10 text-amber-400 border border-amber-500/20 font-medium">
                  {"未托管"}
                </span>
              )}
            </div>
            <div className="flex items-center gap-2 mt-0.5">
              {def?.official_website && (
                <button onClick={() => openUrl(def.official_website)} className="text-[10px] text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 cursor-pointer">
                  {"官方网站"} <ExternalLink className="w-2.5 h-2.5" />
                </button>
              )}
              {status.install_source && (
                <><span className="text-slate-600 text-[10px]">.</span><span className="text-[10px] text-slate-400">{"安装方式"}: {status.install_source}</span></>
              )}
            </div>
          </div>
          <button
            onClick={async () => { if (pid) { await loadDetail(pid); await onRefresh(); } }}
            disabled={ui.detailLoading}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
          >
            <RefreshCw className={`w-3 h-3 ${ui.detailLoading ? "animate-spin" : ""}`} /> {"刷新"}
          </button>
        </div>

        <div className="flex items-center gap-4 mt-3 text-[10px]">
          <div className="flex items-center gap-1.5">
            <span className="text-slate-500">{"状态"}:</span>
            {status.installed ? (
              <span className="text-emerald-400 flex items-center gap-1 font-semibold">
                <CheckCircle className="w-3 h-3" /> {"已安装"}
              </span>
            ) : <span className="text-slate-400">{"未安装"}</span>}
          </div>
          {status.active_version && (
            <div className="flex items-center gap-1.5">
              <span className="text-slate-500">{"当前版本"}:</span>
              <span className="text-blue-400 font-mono font-semibold">v{status.active_version}</span>
            </div>
          )}
          {status.install_root && (
            <div className="flex items-center gap-1.5">
              <span className="text-slate-500">{"安装路径"}:</span>
              <span className="text-slate-300 font-mono truncate max-w-[200px]">{status.install_root}</span>
            </div>
          )}
        </div>
      </div>

      {!status.managed ? (
        <div className="flex-1 flex flex-col items-center justify-center p-8 text-center space-y-4">
          <div className="w-16 h-16 rounded-full bg-slate-600/10 flex items-center justify-center">
            <ShieldCheck className="w-8 h-8 text-slate-500" />
          </div>
          <div>
            <p className="text-sm font-medium text-slate-300">{"该项目尚未被 AnyVersion 托管"}</p>
            <p className="text-[11px] text-slate-500 mt-1 max-w-sm">
              {status.installed
                ? "检测到本地已有安装，托管后可统一管理版本、环境变量和缓存。"
                : "托管后将自动配置环境变量，可通过此界面一键安装和切换版本。"}
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="flex bg-white/5 border border-white/5 rounded-xl p-0.5 mx-5 mt-4 flex-shrink-0">
            {availableTabs.map((tab) => (
              <button
                key={tab}
                onClick={() => pid && patch(pid, { activeSubTab: tab })}
                className={`flex-1 py-1.5 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
                  ui.activeSubTab === tab ? "bg-blue-600 text-white shadow-md" : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {tabLabels[tab]}
              </button>
            ))}
          </div>

          <div className="flex-1 overflow-y-auto p-5 space-y-5">
            {ui.detectStep || ui.switchingVersion ? (
              <div className="flex flex-col items-center justify-center py-12 space-y-4">
                <Loader className="w-6 h-6 animate-spin text-blue-400" />
                <div className="text-center space-y-2 max-w-sm">
                  <p className="text-xs text-blue-300 font-medium">{ui.detectStep || `正在切换到 ${ui.switchingVersion}...`}</p>
                  <div className="w-64 mx-auto">
                    <div className="flex items-center justify-between text-[10px] text-slate-500 mb-1">
                      <span>检测进度</span>
                      <span>{ui.detectIndex}/{ui.detectTotal}</span>
                    </div>
                    <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
                      <div
                        className="h-full bg-gradient-to-r from-blue-600 to-blue-400 rounded-full transition-all duration-500"
                        style={{ width: `${(ui.detectIndex / ui.detectTotal) * 100}%` }}
                      />
                    </div>
                  </div>
                  <p className="text-[10px] text-slate-500">首次打开需要全面检测，请稍候...</p>
                </div>
              </div>
            ) : ui.detailLoading && !ui.detailLoaded ? (
              <div className="flex items-center justify-center gap-2 text-xs text-slate-400 py-8">
                <Loader className="w-4 h-4 animate-spin text-blue-400" /> {"正在加载项目详情..."}
              </div>
            ) : (
              <>
                {ui.activeSubTab === "versions" && <VersionsTab {...subTabProps} />}
                {ui.activeSubTab === "envvars" && <EnvVarsTab {...subTabProps} />}
                {ui.activeSubTab === "cache" && <CacheTab {...subTabProps} />}
                {ui.activeSubTab === "mirror" && <MirrorTab {...subTabProps} />}
                {ui.activeSubTab === "packages" && <PackagesTab {...subTabProps} />}
                {ui.activeSubTab === "services" && <ServicesTab {...subTabProps} />}
                {ui.activeSubTab === "pkgmgr" && <PkgMgrTab {...subTabProps} />}
              </>
            )}
          </div>
        </>
      )}

      <div className="border-t border-white/5 p-4 bg-white/2 flex-shrink-0">
        {ui.showManagePreview && (() => {
          const envVars = def?.env_vars || [];
          const linkPath = "%USERPROFILE%\\.any-version\\links\\" + pid;
          const versionsDir = "%USERPROFILE%\\.any-version\\versions";
          return (
            <div className="mb-4 p-4 bg-blue-600/5 border border-blue-500/15 rounded-xl space-y-3 animate-fadeIn">
              <h4 className="text-xs font-semibold text-blue-300 flex items-center gap-1.5">
                <Info className="w-3.5 h-3.5" /> {"托管操作预览 - 将要执行以下操作"}
              </h4>

              {envVars.length > 0 && (
                <div className="flex items-start gap-2 text-[11px]">
                  <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">1</span>
                  <div>
                    <span className="text-slate-200 font-medium">{"备份环境变量"}</span>
                    <p className="text-slate-400 mt-0.5">
                      {"将备份以下环境变量的当前值"}: <span className="font-mono text-[10px] text-blue-300">{envVars.map((v: { name: string }) => v.name).join(", ")}</span>
                    </p>
                  </div>
                </div>
              )}

              <div className="flex items-start gap-2 text-[11px]">
                <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">{envVars.length > 0 ? "2" : "1"}</span>
                <div>
                  <span className="text-slate-200 font-medium">{"创建目录联接"}</span>
                  <p className="font-mono text-[10px] text-blue-300 mt-0.5">{linkPath} → {versionsDir}\{pid}\VERSION</p>
                </div>
              </div>

              {envVars.length > 0 && (
                <div className="flex items-start gap-2 text-[11px]">
                  <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">3</span>
                  <div>
                    <span className="text-slate-200 font-medium">{"设置环境变量"}</span>
                    <p className="font-mono text-[10px] text-blue-300 mt-0.5">{envVars.map((v: { name: string }) => v.name).join(", ")} → {linkPath}</p>
                  </div>
                </div>
              )}

              {ui.managePreview && ui.managePreview.steps.filter((s) => s.action === "add_path" || s.action === "clean_path").map((step, idx) => (
                <div key={idx} className="flex items-start gap-2 text-[11px]">
                  <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">{4 + idx}</span>
                  <div>
                    <span className="text-slate-200 font-medium">{step.description}</span>
                    {step.target && <p className="font-mono text-[10px] text-slate-500 mt-0.5">{step.target}</p>}
                  </div>
                </div>
              ))}

              <div className="p-2.5 rounded-lg bg-black/20 border border-white/5 text-[10px] space-y-1.5">
                <div className="flex items-center gap-1.5 text-slate-300">
                  <span className="font-semibold text-slate-200">{"备份文件位置："}</span>
                  <span className="font-mono text-blue-300">%USERPROFILE%\.any-version\config.json</span>
                </div>
                <div className="flex items-center gap-1.5 text-slate-300">
                  <span className="font-semibold text-slate-200">{"取消托管时："}</span>
                  <span>{"将从备份还原所有环境变量"}</span>
                </div>
              </div>

              <div className="flex items-center gap-2 pt-1">
                <button onClick={handleManage} disabled={ui.managing} className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all">
                  {ui.managing ? "正在执行..." : "确认托管"}
                </button>
                <button onClick={() => patch(pid!, { showManagePreview: false, managePreview: null })} className="px-4 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs font-medium cursor-pointer border border-white/10">
                  {"取消"}
                </button>
              </div>
            </div>
          );
        })()}

        <div className="flex items-center justify-between">
          <div className="text-[10px] text-slate-500">
            {status.managed ? "托管中: 环境变量和 PATH 已由 AnyVersion 管理" : "未托管: 环境变量由系统或手动管理"}
          </div>
          <div className="flex items-center gap-2">
            {status.managed ? (
              <button onClick={handleUnmanage} disabled={ui.unmanaging || isOperating} className="px-4 py-2 bg-red-600/80 hover:bg-red-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5">
                {ui.unmanaging ? "取消托管中..." : "取消托管"}
              </button>
            ) : (
              <button onClick={handlePreviewManage} disabled={ui.managing} className="px-5 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all flex items-center gap-1.5 hover:scale-[1.02] active:scale-[0.98]">
                {ui.managing ? "托管中..." : "托管此项目"}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
