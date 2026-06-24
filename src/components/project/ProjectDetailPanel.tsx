import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ExternalLink,
  ShieldCheck,
  RefreshCw,
  Info,
  Loader,
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
  ServicesTab,
  PackageManagerTab,
  LegacyTab,
  DataDirsTab,
} from "./ProjectSubTabs";

type SubTab = "versions" | "envvars" | "services" | string;

const baseTabLabels: Record<string, string> = {
  versions: "版本管理",
  envvars: "环境变量",
  legacy: "旧版数据",
  services: "服务管理",
  data_dirs: "数据管理",
};

interface ProjectUIState {
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
  detectStep: "",
  detectIndex: 0,
  detectTotal: 0,
};

let _onProgress: ((p: { sdk: string; downloaded: number; total: number; pct: number; speed_str: string }) => void) | null = null;
let _onStep: ((s: string) => void) | null = null;
let _unlistenProgress: (() => void) | null = null;
let _unlistenStep: (() => void) | null = null;

function ensureListeners(
  onProgress: (p: { sdk: string; downloaded: number; total: number; pct: number; speed_str: string }) => void,
  onStep: (s: string) => void,
) {
  _onProgress = onProgress;
  _onStep = onStep;
  if (!_unlistenProgress) {
    listen<{ sdk: string; downloaded: number; total: number; pct: number; speed_str: string }>("download-progress", (e) => {
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

export default function ProjectDetailPanel({
  project,
  onRefresh,
  onProjectUpdate,
}: Props) {
  const [uiMap, setUiMap] = useState<Record<string, ProjectUIState>>({});
  const eventProjectRef = useRef<string | null>(null);

  const pid = project?.id ?? null;
  const ui: ProjectUIState = pid ? (uiMap[pid] ?? EMPTY_UI) : EMPTY_UI;

  // 旧版数据（托管前的备份）相关状态 —— 必须在条件 return 之前声明
  const [hasLegacy, setHasLegacy] = useState(false);
  const [legacyLoaded, setLegacyLoaded] = useState(false);

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

  const handleSelectCustomPath = useCallback(async () => {
    if (!pid) return;
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择安装目录",
      });
      if (selected && typeof selected === "string") {
        await invoke("project_set_custom_path", { id: pid, path: selected });
        await refreshSingle(pid);
      }
    } catch (err) {
      console.error("Select custom path error:", err);
    }
  }, [pid, refreshSingle]);

  const handleClearCustomPath = useCallback(async () => {
    if (!pid) return;
    try {
      await invoke("project_set_custom_path", { id: pid, path: "" });
      await refreshSingle(pid);
    } catch (err) {
      console.error("Clear custom path error:", err);
    }
  }, [pid, refreshSingle]);

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

    // 步骤2：获取远程版本列表（优先读缓存）
    steps.push({
      label: `正在获取 ${proj.display_name} 远程版本列表...`,
      run: async () => {
        try {
          const result = await invoke<{ versions: string[]; updated_at: number; from_cache: boolean }>(
            "project_list_remote_versions", { id, force: false }
          );
          patch(id, { remoteVersions: result.versions, versionsUpdatedAt: result.updated_at });
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
    const defaultTab = project.category === "service" ? "services" : "versions";
    if (!state) {
      patch(pid, { ...EMPTY_UI, activeSubTab: defaultTab });
      runInitialDetection(pid, project);
    } else if (!state.detailLoaded && !state.detailLoading && !state.detectStep) {
      runInitialDetection(pid, project);
    }
  }, [pid]);

  const handleInstall = useCallback(async (version: string) => {
    if (!pid) return;
    eventProjectRef.current = pid;
    patch(pid, { installingVersion: version, installStep: "下载中", downloadProgress: null });
    try {
      const parts = version.includes(" · ") ? version.split(" · ")[1] : version;
      const cleanVer = parts.trim().split(" ")[0];
      await invoke("project_install_version", { id: pid, version: cleanVer });
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

  const handleCancelInstall = useCallback(async () => {
    if (!pid) return;
    try {
      await invoke("project_cancel_install", { id: pid });
    } catch {
      // 忽略错误（任务可能已完成）
    } finally {
      patch(pid, { installingVersion: null, downloadProgress: null, installStep: "" });
      eventProjectRef.current = null;
    }
  }, [pid, patch]);

  const handleRefreshRemoteVersions = useCallback(async () => {
    if (!pid) return;
    patch(pid, { loadingRemote: true });
    try {
      const result = await invoke<{ versions: string[]; updated_at: number; from_cache: boolean }>(
        "project_list_remote_versions", { id: pid, force: true }
      );
      patch(pid, { remoteVersions: result.versions, versionsUpdatedAt: result.updated_at, loadingRemote: false });
    } catch (e: unknown) {
      alert("刷新版本列表失败: " + e);
      patch(pid, { loadingRemote: false });
    }
  }, [pid, patch]);

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

  const handlePreviewManage = useCallback(async (isSimple: boolean) => {
    if (!pid) return;
    patch(pid, { isSimpleManage: isSimple });
    try {
      const preview = await invoke<ManagePreview>("project_preview_manage", { id: pid, isSimple });
      patch(pid, { managePreview: preview, showManagePreview: true });
    } catch {
      patch(pid, { managePreview: null, showManagePreview: true });
    }
  }, [pid, patch]);

  const handleManage = useCallback(async () => {
    if (!pid) return;
    patch(pid, { managing: true });
    try {
      await invoke("project_manage", { id: pid, isSimple: ui.isSimpleManage });
      patch(pid, { showManagePreview: false, managePreview: null, managing: false });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("托管操作失败: " + e);
      patch(pid, { managing: false });
    }
  }, [pid, patch, ui.isSimpleManage, loadDetail, onRefresh]);

  const handlePreviewUnmanage = useCallback(async () => {
    if (!pid) return;
    try {
      const preview = await invoke<ManagePreview>("project_preview_unmanage", { id: pid });
      patch(pid, { managePreview: preview, showManagePreview: true });
    } catch(e: unknown) {
      alert(String(e));
    }
  }, [pid, patch]);

  const handleUnmanage = useCallback(async () => {
    if (!pid || !ui.detail) return;
    patch(pid, { unmanaging: true });
    try {
      await invoke("project_unmanage", { id: pid });
      patch(pid, { showManagePreview: false, managePreview: null });
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

  // 托管后如果有旧版备份，检测是否存在 —— 必须在条件 return 之前
  useEffect(() => {
    if (!pid) { setHasLegacy(false); setLegacyLoaded(false); return; }
    const current = uiMap[pid];
    const managed = current?.detail?.status?.managed ?? false;
    if (managed && !legacyLoaded) {
      invoke<{ project_id: string } | null>("get_legacy_backup", { id: pid }).then((info) => {
        setHasLegacy(info !== null);
        setLegacyLoaded(true);
      }).catch(() => setLegacyLoaded(true));
    }
    if (!managed) {
      setHasLegacy(false);
      setLegacyLoaded(false);
    }
  }, [pid, uiMap]);

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

  // 构建动态 Tab 列表：基础 tabs + 每个包管理器一个独立 tab
  const availableTabs: SubTab[] = [];
  if (status.is_simple_managed || def?.simple_mode) {
    if (def?.category === "service" || def?.is_service) {
      availableTabs.push("services");
    }
    if (def?.data_dirs && def.data_dirs.length > 0) {
      availableTabs.push("data_dirs");
    }
  } else {
    if (def?.category === "service" || def?.is_service) {
      availableTabs.push("services");
      if (def?.data_dirs && def.data_dirs.length > 0) {
        availableTabs.push("data_dirs");
      }
      availableTabs.push("envvars");
      availableTabs.push("versions");
    } else {
      availableTabs.push("versions");
      availableTabs.push("envvars");
    }
  }

  // 包管理器 tabs：每个 PM 一个独立子页面，用 "pm:" 前缀标识
  const pmTabs: Array<{ id: string; label: string }> = [];
  if (def?.package_managers && def.package_managers.length > 0) {
    for (const pm of def.package_managers) {
      availableTabs.push(`pm:${pm.id}`);
      pmTabs.push({ id: `pm:${pm.id}`, label: pm.display_name });
    }
  }
  if (hasLegacy && !status.is_simple_managed && !def?.simple_mode) {
    availableTabs.push("legacy");
  }

  // Tab 标签映射（基础 + 动态）
  const tabLabels: Record<string, string> = { ...baseTabLabels };
  for (const pt of pmTabs) {
    tabLabels[pt.id] = pt.label;
  }

  // Fallback to first available tab if activeSubTab is not in availableTabs
  const activeTab = availableTabs.includes(ui.activeSubTab) ? ui.activeSubTab : (availableTabs[0] || "versions");

  const isOperating = !!ui.installingVersion || !!ui.switchingVersion || ui.managing || ui.unmanaging || !!ui.detectStep;

  const subTabProps: SubTabProps = {
    project: status,
    def,
    remoteVersions: ui.remoteVersions,
    loadingRemote: ui.loadingRemote,
    versionsUpdatedAt: ui.versionsUpdatedAt,
    installingVersion: ui.installingVersion,
    downloadProgress: ui.downloadProgress,
    installStep: ui.installStep,
    onInstall: handleInstall,
    onUninstall: handleUninstall,
    onUse: handleUse,
    onCancelInstall: handleCancelInstall,
    onRefreshRemoteVersions: handleRefreshRemoteVersions,
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
    activeSubTab: activeTab,
    onActiveSubTabChange: (tab: string) => { if (pid) patch(pid, { activeSubTab: tab as SubTab }); },
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
                status.is_simple_managed ? (
                  <span className="px-1.5 py-0.5 rounded text-[9px] bg-amber-500/10 text-amber-400 border border-amber-500/20 font-bold flex items-center gap-0.5">
                    <ShieldCheck className="w-2.5 h-2.5" /> {"简单托管中"}
                  </span>
                ) : (
                  <span className="px-1.5 py-0.5 rounded text-[9px] bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-bold flex items-center gap-0.5">
                    <ShieldCheck className="w-2.5 h-2.5" /> {"托管中"}
                  </span>
                )
              ) : (
                <span className="px-1.5 py-0.5 rounded text-[9px] bg-slate-500/10 text-slate-400 border border-slate-500/20 font-medium">
                  {"未托管"}
                </span>
              )}
              {status.installed ? (
                status.active_version ? (
                  <span className="px-1.5 py-0.5 rounded text-[9px] font-mono font-bold bg-emerald-500/15 text-emerald-400 border border-emerald-500/25">
                    v{status.active_version}
                  </span>
                ) : (
                  <span className="px-1.5 py-0.5 rounded text-[9px] font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/20">
                    {"已安装"}
                  </span>
                )
              ) : (
                <span className="px-1.5 py-0.5 rounded text-[9px] font-semibold bg-red-500/10 text-red-400 border border-red-500/20">
                  {"未安装"}
                </span>
              )}
            </div>
            <div className="flex items-center gap-2 mt-0.5 flex-wrap">
              {def?.official_website && (
                <button onClick={() => openUrl(def.official_website)} className="text-[10px] text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 cursor-pointer mr-1">
                  {"官方网站"} <ExternalLink className="w-2.5 h-2.5" />
                </button>
              )}
              {status.install_source && (
                <><span className="text-slate-600 text-[10px]">.</span><span className="text-[10px] text-slate-400">{"安装方式"}: {status.install_source}</span></>
              )}
              {status.install_root && (
                <><span className="text-slate-600 text-[10px]">.</span><span className="text-[10px] text-slate-400 font-mono truncate max-w-[300px]" title={status.install_root}>{"安装路径"}: {status.install_root}</span></>
              )}
              {(!status.managed || status.is_simple_managed) && (!status.install_root || status.install_source === "手动指定") && (
                <>
                  <span className="text-slate-600 text-[10px]">.</span>
                  <button
                    onClick={handleSelectCustomPath}
                    className="text-[10px] text-blue-400 hover:text-blue-300 hover:underline transition-colors flex items-center gap-0.5 cursor-pointer font-medium"
                  >
                    {status.install_root ? "修改路径" : "手动指定目录"}
                  </button>
                  {status.install_source === "手动指定" && (
                    <>
                      <span className="text-slate-600 text-[10px]">.</span>
                      <button
                        onClick={handleClearCustomPath}
                        className="text-[10px] text-amber-500 hover:text-amber-400 hover:underline transition-colors flex items-center gap-0.5 cursor-pointer font-medium"
                      >
                        {"恢复自动检测"}
                      </button>
                    </>
                  )}
                </>
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
      ) : (status.is_simple_managed || def?.simple_mode) && !status.install_root ? (
        <div className="flex-1 flex flex-col items-center justify-center p-8 text-center space-y-4">
          <div className="w-16 h-16 rounded-full bg-blue-600/10 flex items-center justify-center">
            <Settings className="w-8 h-8 text-blue-500" />
          </div>
          <div>
            <p className="text-sm font-medium text-slate-300">{"未检测到本地安装目录"}</p>
            <p className="text-[11px] text-slate-500 mt-1 max-w-sm">
              {"该项目为简单托管项目，AnyVersion 不提供版本下载与安装服务。请先手动指定本地已安装的目录以进行托管管理。"}
            </p>
          </div>
          <button
            onClick={handleSelectCustomPath}
            className="px-5 py-2.5 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all flex items-center gap-1.5 hover:scale-[1.02] active:scale-[0.98]"
          >
            {"手动指定目录"}
          </button>
        </div>
      ) : (
        <>
          {availableTabs.length > 1 && (
            <div className="flex bg-white/5 border border-white/5 rounded-xl p-0.5 mx-5 mt-4 flex-shrink-0">
              {availableTabs.map((tab) => (
                <button
                  key={tab}
                  onClick={() => pid && patch(pid, { activeSubTab: tab })}
                  className={`flex-1 py-1.5 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
                    activeTab === tab ? "bg-blue-600 text-white shadow-md" : "text-slate-400 hover:text-slate-200"
                  }`}
                >
                  {tabLabels[tab]}
                </button>
              ))}
            </div>
          )}

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
                {activeTab === "versions" && <VersionsTab {...subTabProps} />}
                {activeTab === "envvars" && <EnvVarsTab {...subTabProps} />}
                {activeTab === "services" && <ServicesTab {...subTabProps} />}
                {activeTab === "data_dirs" && <DataDirsTab project={status} onRefresh={async () => { if (pid) await loadDetail(pid); }} />}
                {activeTab === "legacy" && <LegacyTab projectId={pid!} />}
                {/* 动态包管理器子页面 —— key 保证每个 PM 独立且切换时不重新挂载 */}
                {pmTabs.map(pt => {
                  const pmId = pt.id.replace("pm:", "");
                  const pmDef = def?.package_managers?.find(p => p.id === pmId);
                  if (!pmDef) return null;
                  return (
                    <PackageManagerTab 
                      key={pt.id} 
                      projectId={pid!} 
                      pm={pmDef} 
                      hidden={activeTab !== pt.id} 
                      installRoot={status.install_root}
                      installSource={status.install_source}
                    />
                  );
                })}
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
          const preview = ui.managePreview;
          const isUnmanage = status.managed;

          const backupVars = envVars.filter(v => v.tier !== "compat");
          const clearVars = envVars.filter(v => v.tier === "clear");
          const manageVars = envVars.filter(v => v.tier !== "compat" && v.tier !== "clear");

          const stepColorMap: Record<string, string> = {
            clear_env: "bg-red-600/20 text-red-400",
            remove_path: "bg-red-600/20 text-red-400",
            restore_env: "bg-emerald-600/20 text-emerald-400",
            restore_path: "bg-emerald-600/20 text-emerald-400",
            skip_compat: "bg-amber-600/20 text-amber-400",
          };

          return (
            <div className={`mb-4 p-4 rounded-xl space-y-3 animate-fadeIn ${isUnmanage ? "bg-red-600/5 border border-red-500/15" : "bg-blue-600/5 border border-blue-500/15"}`}>
              <h4 className={`text-xs font-semibold flex items-center gap-1.5 ${isUnmanage ? "text-red-300" : "text-blue-300"}`}>
                <Info className="w-3.5 h-3.5" />
                {isUnmanage ? "取消托管预览 - 将要执行以下操作" : "托管操作预览 - 将要执行以下操作"}
              </h4>

              {!isUnmanage ? (
                ui.isSimpleManage ? (
                  <>
                    {preview?.steps?.map((step, idx) => (
                      <div key={idx} className="flex items-start gap-2 text-[11px]">
                        <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">
                          {idx + 1}
                        </span>
                        <div className="min-w-0">
                          <span className="text-slate-200 font-medium">{step.description}</span>
                          {step.target && (
                            <p className="font-mono text-[10px] text-slate-500 mt-0.5 whitespace-pre-wrap break-all">{step.target}</p>
                          )}
                        </div>
                      </div>
                    ))}
                  </>
                ) : (
                  <>
                    {backupVars.length > 0 && (
                      <div className="flex items-start gap-2 text-[11px]">
                        <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">1</span>
                        <div>
                          <span className="text-slate-200 font-medium">{"备份并清除冲突环境变量"}</span>
                          <p className="text-slate-400 mt-0.5">
                            {clearVars.length > 0 ? (
                              <>
                                {"将备份以下冲突环境变量的当前值并将其从系统中删除"}: <span className="font-mono text-[10px] text-red-300 font-semibold">{clearVars.map(v => v.name).join(", ")}</span>
                                {backupVars.filter(v => v.tier !== "clear").length > 0 && (
                                  <>
                                    <br />
                                    {"仅备份但不删除的环境变量"}: <span className="font-mono text-[10px] text-blue-300">{backupVars.filter(v => v.tier !== "clear").map(v => v.name).join(", ")}</span>
                                  </>
                                )}
                              </>
                            ) : (
                              <>
                                {"将备份以下环境变量的当前值"}: <span className="font-mono text-[10px] text-blue-300">{backupVars.map(v => v.name).join(", ")}</span>
                              </>
                            )}
                          </p>
                        </div>
                      </div>
                    )}

                    <div className="flex items-start gap-2 text-[11px]">
                      <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">{backupVars.length > 0 ? "2" : "1"}</span>
                      <div>
                        <span className="text-slate-200 font-medium">{"创建目录联接"}</span>
                        <p className="font-mono text-[10px] text-blue-300 mt-0.5">{linkPath} → {versionsDir}\{pid}\VERSION</p>
                      </div>
                    </div>

                    {(manageVars.length > 0 || clearVars.length > 0) && (
                      <div className="flex items-start gap-2 text-[11px]">
                        <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">{backupVars.length > 0 ? "3" : "2"}</span>
                        <div>
                          <span className="text-slate-200 font-medium">{"更新注册表环境变量"}</span>
                          <div className="space-y-1 mt-0.5">
                            {manageVars.length > 0 && (
                              <p className="text-slate-400">
                                {"设置托管变量 (指向 AnyVersion)"}: <span className="font-mono text-[10px] text-blue-300">{manageVars.map((v: { name: string }) => v.name).join(", ")} → {linkPath}</span>
                              </p>
                            )}
                            {clearVars.length > 0 && (
                              <p className="text-slate-400">
                                <span className="text-red-400 font-semibold">{"清空冲突变量 (后续交由 AnyVersion 管理)"}</span>: <span className="font-mono text-[10px] text-red-300">{clearVars.map((v: { name: string }) => v.name).join(", ")}</span>
                              </p>
                            )}
                          </div>
                        </div>
                      </div>
                    )}

                    {preview && preview.steps.filter((s) => s.action === "add_path" || s.action === "clean_path").map((step, idx) => (
                      <div key={idx} className="flex items-start gap-2 text-[11px]">
                        <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">{4 + idx}</span>
                        <div>
                          <span className="text-slate-200 font-medium">{step.description}</span>
                          {step.target && <p className="font-mono text-[10px] text-slate-500 mt-0.5">{step.target}</p>}
                        </div>
                      </div>
                    ))}

                    {preview?.has_local_install && (
                      <div className="p-3 bg-emerald-500/10 border border-emerald-500/20 rounded-lg text-[10px]">
                        <span className="text-emerald-300 font-medium">检测到本地已安装版本</span>
                        {preview.local_install_root && (
                          <p className="text-slate-400 mt-0.5">路径: {preview.local_install_root}</p>
                        )}
                        {preview.local_install_source && (
                          <p className="text-slate-500">来源: {preview.local_install_source}</p>
                        )}
                        <p className="text-amber-400/80 mt-1">托管后可在版本管理中扫描并注册本地版本</p>
                      </div>
                    )}
                  </>
                )
              ) : (
                <>
                  {preview?.steps?.map((step, idx) => (
                    <div key={idx} className="flex items-start gap-2 text-[11px]">
                      <span className={`w-5 h-5 rounded-full flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5 ${stepColorMap[step.action] || "bg-slate-600/20 text-slate-400"}`}>
                        {idx + 1}
                      </span>
                      <div className="min-w-0">
                        <span className="text-slate-200 font-medium">{step.description}</span>
                        {step.target && (
                          <p className="font-mono text-[10px] text-slate-500 mt-0.5 whitespace-pre-wrap break-all">{step.target}</p>
                        )}
                      </div>
                    </div>
                  ))}
                </>
              )}

              {!(ui.isSimpleManage || status.is_simple_managed) && (
                <div className="p-2.5 rounded-lg bg-black/20 border border-white/5 text-[10px] space-y-1.5">
                  <div className="flex items-center gap-1.5 text-slate-300">
                    <span className="font-semibold text-slate-200">{"备份文件位置："}</span>
                    <span className="font-mono text-blue-300">%USERPROFILE%\\.any-version\\backup\\manage_{pid}_*.json</span>
                  </div>
                  <div className="flex items-center gap-1.5 text-slate-300">
                    <span className="font-semibold text-slate-200">{isUnmanage ? "托管时已备份：" : "取消托管时："}</span>
                    <span>{isUnmanage ? "将从备份还原所有环境变量" : "将从备份还原所有环境变量"}</span>
                  </div>
                </div>
              )}

              <div className="flex items-center gap-2 pt-1">
                {isUnmanage ? (
                  <button onClick={handleUnmanage} disabled={ui.unmanaging} className="px-4 py-2 bg-red-600 hover:bg-red-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all">
                    {ui.unmanaging ? "正在执行..." : "确认取消托管"}
                  </button>
                ) : (
                  <button onClick={() => handleManage()} disabled={ui.managing} className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all">
                    {ui.managing ? "正在执行..." : "确认托管"}
                  </button>
                )}
                <button onClick={() => patch(pid!, { showManagePreview: false, managePreview: null })} className="px-4 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs font-medium cursor-pointer border border-white/10">
                  {"取消"}
                </button>
              </div>
            </div>
          );
        })()}

        <div className="flex items-center justify-between">
          <div className="text-[10px] text-slate-500">
            {status.managed 
              ? status.is_simple_managed 
                ? "简单托管中: 环境变量、代理和缓存已配置，不接管版本" 
                : "托管中: 环境变量和 PATH 已由 AnyVersion 管理" 
              : "未托管: 环境变量由系统或手动管理"}
          </div>
          <div className="flex items-center gap-2">
            {status.managed ? (
              <button onClick={handlePreviewUnmanage} disabled={ui.unmanaging || isOperating} className="px-4 py-2 bg-red-600/80 hover:bg-red-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5">
                {ui.unmanaging ? "取消托管中..." : "取消托管"}
              </button>
            ) : (
              def?.simple_mode ? (
                <button onClick={() => handlePreviewManage(true)} disabled={ui.managing} className="px-5 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all flex items-center gap-1.5 hover:scale-[1.02] active:scale-[0.98]">
                  {ui.managing ? "托管中..." : "托管此项目"}
                </button>
              ) : (
                <>
                  <button onClick={() => handlePreviewManage(true)} disabled={ui.managing} className="px-4 py-2.5 bg-white/5 hover:bg-white/10 border border-white/10 text-slate-300 rounded-xl text-xs font-semibold cursor-pointer transition-all hover:scale-[1.02] active:scale-[0.98]">
                    {"简单托管"}
                  </button>
                  <button onClick={() => handlePreviewManage(false)} disabled={ui.managing} className="px-5 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all flex items-center gap-1.5 hover:scale-[1.02] active:scale-[0.98]">
                    {ui.managing ? "托管中..." : "完全托管"}
                  </button>
                </>
              )
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
