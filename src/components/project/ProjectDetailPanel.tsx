import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { open } from "@tauri-apps/plugin-dialog";
import RssReader from "../RssReader";
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
  ProjectDelegation,
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
  ConfigTab,
} from "./ProjectSubTabs";

type SubTab = "versions" | "envvars" | "services" | "config" | string;

const baseTabLabels: Record<string, string> = {
  versions: "版本管理",
  envvars: "环境变量",
  legacy: "旧版数据",
  services: "服务管理",
  data_dirs: "数据管理",
  config: "参数配置",
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
  repairingEnv: boolean;
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
  repairingEnv: false,
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
  const [isAdmin, setIsAdmin] = useState(true);
  const [showMenuConfig, setShowMenuConfig] = useState(false);
  const [localDelegation, setLocalDelegation] = useState<ProjectDelegation | null>(null);

  useEffect(() => {
    setShowMenuConfig(false);
  }, [pid]);

  useEffect(() => {
    invoke<boolean>("is_admin").then(setIsAdmin).catch(() => setIsAdmin(true));
  }, []);

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

  const handleRepairEnv = useCallback(async () => {
    if (!pid) return;
    patch(pid, { repairingEnv: true });
    try {
      await invoke("project_repair_env_vars", { id: pid });
      await refreshSingle(pid);
      alert("环境变量和 PATH 已重新校准");
    } catch (e: unknown) {
      alert("修复环境变量失败: " + e);
    } finally {
      patch(pid, { repairingEnv: false });
    }
  }, [pid, patch, refreshSingle]);

  const handlePreviewManage = useCallback(async () => {
    if (!pid || !ui.detail) return;
    const def = ui.detail.def;
    const initialDelegation: ProjectDelegation = {
      env_vars: def.env_vars.filter(v => v.tier !== "compat").map(v => v.name),
      path_vars: def.bin_dirs || [],
      version_control: def.download_url_template || def.is_git_repo ? true : false,
      create_symlink: true,
      manage_install_dir: true,
      manage_data_dir: true,
    };
    patch(pid, { isSimpleManage: false });
    try {
      const preview = await invoke<ManagePreview>("project_preview_manage", { id: pid, delegation: initialDelegation });
      setLocalDelegation(initialDelegation);
      patch(pid, { managePreview: preview, showManagePreview: true });
    } catch (e: unknown) {
      alert(String(e));
    }
  }, [pid, ui.detail, patch]);

  const handleCheckboxChange = useCallback(async (updated: Partial<ProjectDelegation>) => {
    if (!pid || !localDelegation) return;
    const next = { ...localDelegation, ...updated };
    setLocalDelegation(next);
    try {
      const preview = await invoke<ManagePreview>("project_preview_manage", { id: pid, delegation: next });
      patch(pid, { managePreview: preview });
    } catch (e) {
      console.error("Preview manage error:", e);
    }
  }, [pid, localDelegation, patch]);

  const handleManage = useCallback(async () => {
    if (!pid || !localDelegation) return;
    patch(pid, { managing: true });
    try {
      await invoke("project_manage", { id: pid, delegation: localDelegation });
      patch(pid, { showManagePreview: false, managePreview: null, managing: false });
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("托管操作失败: " + e);
      patch(pid, { managing: false });
    }
  }, [pid, patch, localDelegation, refreshSingle]);

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
    const simpleService = ui.detail.def.simple_mode || ui.detail.status.is_simple_managed;

    if (!isAdmin && Array.isArray(ui.detail.def.service_names) && ui.detail.def.service_names.length > 0) {
      const confirmed = window.confirm(
        `操作 Windows 系统服务需要管理员权限。当前程序未以管理员身份运行，操作可能会因“拒绝访问（系统错误 5）”而失败。\n\n是否继续？`
      );
      if (!confirmed) return;
    }

    patch(pid, { serviceCtrlLoading: true });
    try {
      if (running) {
        try {
          await invoke("stop_service", { name: pid });
        } catch (stopErr) {
          const msg = String(stopErr);
          const confirmed = window.confirm(
            `安全停止失败：\n${msg}\n\n是否强制终止 ${pid} 服务进程？`
          );
          if (!confirmed) throw stopErr;
          await invoke("force_stop_service", { name: pid });
        }
      } else {
        if (!simpleService && !ui.detail.status.active_version) {
          alert("请先启用一个版本，然后才能启动服务");
          patch(pid, { serviceCtrlLoading: false });
          return;
        }
        await invoke("start_service", {
          name: pid,
          version: simpleService ? null : ui.detail.status.active_version,
        });
      }
      await invoke("refresh_tray_menu");
      await refreshSingle(pid);
    } catch (e: unknown) {
      alert("服务操作失败: " + e);
    } finally {
      patch(pid, { serviceCtrlLoading: false });
    }
  }, [pid, ui.detail, patch, refreshSingle, isAdmin]);

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
      <div className="h-full flex flex-col min-h-0 border-l border-white/5">
        <RssReader />
      </div>
    );
  }

  const def: ProjectDef | null = ui.detail?.def ?? null;
  const status: ProjectStatus = ui.detail?.status ?? project;

  // 构建动态 Tab 列表：基础 tabs + 每个包管理器一个独立 tab
  const availableTabs: SubTab[] = [];
  const delegation = status.delegation;

  if (def?.category === "service" || def?.is_service) {
    availableTabs.push("services");
    availableTabs.push("config");
  }

  const hasVersionSupport = def?.download_url_template || def?.is_git_repo;
  if (hasVersionSupport && (!status.managed || delegation?.version_control)) {
    availableTabs.push("versions");
  }

  if (def?.env_vars && def.env_vars.length > 0 && (!status.managed || (delegation?.env_vars && delegation.env_vars.length > 0))) {
    availableTabs.push("envvars");
  }

  if (def?.data_dirs && def.data_dirs.length > 0 && (!status.managed || delegation?.manage_data_dir)) {
    availableTabs.push("data_dirs");
  }

  // 包管理器 tabs：每个 PM 一个独立子页面，用 "pm:" 前缀标识
  const pmTabs: Array<{ id: string; label: string }> = [];
  if (def?.package_managers && def.package_managers.length > 0) {
    for (const pm of def.package_managers) {
      availableTabs.push(`pm:${pm.id}`);
      pmTabs.push({ id: `pm:${pm.id}`, label: pm.display_name });
    }
  }
  if (hasLegacy && !status.is_simple_managed && !def?.simple_mode && !(def?.category === "service" || def?.is_service)) {
    availableTabs.push("legacy");
  }

  // Tab 标签映射（基础 + 动态）
  const tabLabels: Record<string, string> = { ...baseTabLabels };
  for (const pt of pmTabs) {
    tabLabels[pt.id] = pt.label;
  }

  // Fallback to first available tab if activeSubTab is not in availableTabs
  const activeTab = availableTabs.includes(ui.activeSubTab) ? ui.activeSubTab : (availableTabs[0] || "versions");

  const isOperating = !!ui.installingVersion || !!ui.switchingVersion || ui.managing || ui.unmanaging || ui.repairingEnv || !!ui.detectStep;

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
    repairingEnv: ui.repairingEnv,
    onRepairEnv: handleRepairEnv,
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
          <div className="flex items-center gap-2">
            {status.managed && (
              <button
                onClick={() => setShowMenuConfig(!showMenuConfig)}
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[10px] border cursor-pointer transition-all ${
                  showMenuConfig 
                    ? "bg-blue-600 border-blue-500 text-white" 
                    : "bg-white/5 border-white/5 text-slate-300 hover:bg-white/10"
                }`}
                title="托盘右键菜单显示配置"
              >
                <Settings className="w-3 h-3" /> {"托盘配置"}
              </button>
            )}
            <button
              onClick={async () => { if (pid) { await loadDetail(pid); await onRefresh(); } }}
              disabled={ui.detailLoading}
              className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
            >
              <RefreshCw className={`w-3 h-3 ${ui.detailLoading ? "animate-spin" : ""}`} /> {"刷新"}
            </button>
          </div>
        </div>

      </div>

      {showMenuConfig && status.managed && (
        <div className="mx-5 mt-4 p-4 glass-panel border border-white/5 rounded-2xl bg-white/2 space-y-3 animate-fadeIn flex-shrink-0">
          <div className="flex items-center justify-between border-b border-white/5 pb-2">
            <span className="text-xs font-semibold text-white flex items-center gap-1.5">
              <Settings className="w-3.5 h-3.5 text-blue-400" />
              右键托盘菜单显示配置
            </span>
            <button 
              onClick={() => setShowMenuConfig(false)}
              className="text-[10px] text-slate-400 hover:text-slate-200 cursor-pointer"
            >
              关闭
            </button>
          </div>
          <div className="flex flex-wrap gap-6 py-1">
            <label className="flex items-center gap-2 cursor-pointer text-xs text-slate-300 select-none">
              <input
                type="checkbox"
                checked={status.show_version !== false}
                onChange={async (e) => {
                  if (pid) {
                    await invoke("update_project_menu_config", {
                      id: pid,
                      showVersion: e.target.checked,
                      showService: status.show_service !== false,
                    });
                    await refreshSingle(pid);
                  }
                }}
                className="rounded border-white/10 bg-black/40 text-blue-600 focus:ring-blue-500 w-3.5 h-3.5 cursor-pointer"
              />
              显示版本切换控制
            </label>
            {(def?.category === "service" || def?.is_service) && (
              <label className="flex items-center gap-2 cursor-pointer text-xs text-slate-300 select-none">
                <input
                  type="checkbox"
                  checked={status.show_service !== false}
                  onChange={async (e) => {
                    if (pid) {
                      await invoke("update_project_menu_config", {
                        id: pid,
                        showVersion: status.show_version !== false,
                        showService: e.target.checked,
                      });
                      await refreshSingle(pid);
                    }
                  }}
                  className="rounded border-white/10 bg-black/40 text-blue-600 focus:ring-blue-500 w-3.5 h-3.5 cursor-pointer"
                />
                显示服务启动/停止控制
              </label>
            )}
          </div>
          <p className="text-[10px] text-slate-500 leading-normal">
            提示：此配置将决定该项目是否在系统托盘右键菜单中显示。如果不显示，可以在此处重新开启。
          </p>
        </div>
      )}

      {status.managed && (status.is_simple_managed || def?.simple_mode) && !status.install_root ? (
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
          {!status.managed && (
            <div className="mx-5 mt-4 p-3 bg-blue-500/10 border border-blue-500/20 rounded-xl flex items-start gap-2.5 text-xs text-blue-300">
              <Info className="w-4 h-4 flex-shrink-0 mt-0.5" />
              <div>
                <p className="font-semibold text-slate-200">{"此项目尚未开启托管"}</p>
                <p className="text-[10px] text-slate-400 mt-0.5">
                  {"AnyVersion 尚未为此项目接管系统环境变量或目录链接。你可以直接在下方“版本列表”中下载和使用版本。如果需要，可在底部点击“托管此项目”开启。"}
                </p>
              </div>
            </div>
          )}
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
                {activeTab === "data_dirs" && <DataDirsTab project={status} def={def} onRefresh={async () => { if (pid) await loadDetail(pid); }} />}
                {activeTab === "legacy" && <LegacyTab projectId={pid!} />}
                {activeTab === "config" && <ConfigTab project={status} def={def} onRefresh={async () => { if (pid) await loadDetail(pid); }} />}
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
            <div className={`mb-4 p-4 rounded-xl space-y-4 animate-fadeIn ${isUnmanage ? "bg-red-600/5 border border-red-500/15" : "bg-blue-600/5 border border-blue-500/15"}`}>
              <h4 className={`text-xs font-semibold flex items-center gap-1.5 ${isUnmanage ? "text-red-300" : "text-blue-300"}`}>
                <Info className="w-3.5 h-3.5" />
                {isUnmanage ? "取消托管预览 - 将要执行以下操作" : "托管配置选项 - 自定义要托管的功能范围"}
              </h4>

              {!isUnmanage && localDelegation && (
                <div className="p-3 bg-white/5 border border-white/10 rounded-xl space-y-3">
                  <span className="text-[11px] font-semibold text-slate-300 block">选择需要托管的功能选项：</span>
                  
                  <div className="grid grid-cols-2 gap-3 text-xs">
                    {/* 1. 环境变量 */}
                    {envVars.length > 0 && (
                      <div className="space-y-1.5 p-2 bg-black/25 border border-white/5 rounded-lg col-span-2">
                        <label className="flex items-center gap-2 cursor-pointer font-medium text-slate-200">
                          <input
                            type="checkbox"
                            className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                            checked={localDelegation.env_vars.length === envVars.filter(v => v.tier !== "compat").length}
                            onChange={(e) => {
                              const checked = e.target.checked;
                              const updatedEnvVars = checked 
                                ? envVars.filter(v => v.tier !== "compat").map(v => v.name) 
                                : [];
                              handleCheckboxChange({ env_vars: updatedEnvVars });
                            }}
                          />
                          <span>接管系统环境变量 ({envVars.filter(v => v.tier !== "compat").length})</span>
                        </label>
                        <div className="pl-6 grid grid-cols-2 gap-2 mt-1">
                          {envVars.filter(v => v.tier !== "compat").map(v => (
                            <label key={v.name} className="flex items-center gap-2 cursor-pointer text-[10px] text-slate-400 hover:text-slate-200">
                              <input
                                type="checkbox"
                                className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                                checked={localDelegation.env_vars.includes(v.name)}
                                onChange={(e) => {
                                  const updated = e.target.checked
                                    ? [...localDelegation.env_vars, v.name]
                                    : localDelegation.env_vars.filter((x: string) => x !== v.name);
                                  handleCheckboxChange({ env_vars: updated });
                                }}
                              />
                              <span className="font-mono truncate">{v.name}</span>
                            </label>
                          ))}
                        </div>
                      </div>
                    )}

                    {/* 2. PATH 变量 */}
                    {def?.bin_dirs && def.bin_dirs.length > 0 && (
                      <div className="space-y-1.5 p-2 bg-black/25 border border-white/5 rounded-lg col-span-2">
                        <label className="flex items-center gap-2 cursor-pointer font-medium text-slate-200">
                          <input
                            type="checkbox"
                            className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                            checked={localDelegation.path_vars.length === def.bin_dirs.length}
                            onChange={(e) => {
                              const checked = e.target.checked;
                              const updatedPaths = checked ? [...def.bin_dirs] : [];
                              handleCheckboxChange({ path_vars: updatedPaths });
                            }}
                          />
                          <span>加入系统 PATH 目录 ({def.bin_dirs.length})</span>
                        </label>
                        <div className="pl-6 grid grid-cols-2 gap-2 mt-1">
                          {def.bin_dirs.map(binDir => (
                            <label key={binDir} className="flex items-center gap-2 cursor-pointer text-[10px] text-slate-400 hover:text-slate-200">
                              <input
                                type="checkbox"
                                className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                                checked={localDelegation.path_vars.includes(binDir)}
                                onChange={(e) => {
                                  const updated = e.target.checked
                                    ? [...localDelegation.path_vars, binDir]
                                    : localDelegation.path_vars.filter((x: string) => x !== binDir);
                                  handleCheckboxChange({ path_vars: updated });
                                }}
                              />
                              <span className="truncate">{binDir === "" ? "项目主目录 (bin)" : `子目录: ${binDir}`}</span>
                            </label>
                          ))}
                        </div>
                      </div>
                    )}

                    {/* 3. 版本控制 */}
                    {(def?.download_url_template || def?.is_git_repo) && (
                      <div className="p-2 bg-black/25 border border-white/5 rounded-lg flex items-center">
                        <label className="flex items-center gap-2 cursor-pointer font-medium text-slate-200">
                          <input
                            type="checkbox"
                            className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                            checked={localDelegation.version_control}
                            onChange={(e) => {
                              handleCheckboxChange({ version_control: e.target.checked });
                            }}
                          />
                          <div className="flex flex-col">
                            <span>版本控制与下载</span>
                            <span className="text-[9px] text-slate-400 font-normal">允许在 AnyVersion 内下载多版本</span>
                          </div>
                        </label>
                      </div>
                    )}

                    {/* 4. 创建链接 */}
                    <div className="p-2 bg-black/25 border border-white/5 rounded-lg flex items-center">
                      <label className="flex items-center gap-2 cursor-pointer font-medium text-slate-200">
                        <input
                          type="checkbox"
                          className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                          checked={localDelegation.create_symlink}
                          onChange={(e) => {
                            handleCheckboxChange({ create_symlink: e.target.checked });
                          }}
                        />
                        <div className="flex flex-col">
                          <span>创建目录链接 (Junction)</span>
                          <span className="text-[9px] text-slate-400 font-normal">从映射路径切换到当前激活版本</span>
                        </div>
                      </label>
                    </div>

                    {/* 5. 管理安装目录 */}
                    <div className="p-2 bg-black/25 border border-white/5 rounded-lg flex items-center">
                      <label className="flex items-center gap-2 cursor-pointer font-medium text-slate-200">
                        <input
                          type="checkbox"
                          className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                          checked={localDelegation.manage_install_dir}
                          onChange={(e) => {
                            handleCheckboxChange({ manage_install_dir: e.target.checked });
                          }}
                        />
                        <div className="flex flex-col">
                          <span>管理本地安装目录</span>
                          <span className="text-[9px] text-slate-400 font-normal">支持注册本地已下载的 SDK 路径</span>
                        </div>
                      </label>
                    </div>

                    {/* 6. 管理数据目录 */}
                    {def?.data_dirs && def.data_dirs.length > 0 && (
                      <div className="p-2 bg-black/25 border border-white/5 rounded-lg flex items-center">
                        <label className="flex items-center gap-2 cursor-pointer font-medium text-slate-200">
                          <input
                            type="checkbox"
                            className="rounded border-white/10 bg-slate-800 text-blue-600 focus:ring-blue-500 focus:ring-offset-0"
                            checked={localDelegation.manage_data_dir}
                            onChange={(e) => {
                              handleCheckboxChange({ manage_data_dir: e.target.checked });
                            }}
                          />
                          <div className="flex flex-col">
                            <span>重定向数据目录</span>
                            <span className="text-[9px] text-slate-400 font-normal">分离并重定向工作/日志/数据路径</span>
                          </div>
                        </label>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* 动态步骤预览 */}
              <div className="space-y-2">
                <span className="text-xs font-semibold text-slate-300 block">将要执行的操作步骤：</span>
                {preview?.steps?.map((step, idx) => (
                  <div key={idx} className="flex items-start gap-2 text-[11px]">
                    <span className={`w-5 h-5 rounded-full flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5 ${stepColorMap[step.action] || "bg-blue-600/20 text-blue-400"}`}>
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
                {(!preview?.steps || preview.steps.length === 0) && (
                  <p className="text-[11px] text-slate-500 italic pl-1">暂无需要执行的托管步骤（请勾选上方选项）</p>
                )}
              </div>

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
              <button onClick={() => handlePreviewManage()} disabled={ui.managing} className="px-5 py-2.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all flex items-center gap-1.5 hover:scale-[1.02] active:scale-[0.98]">
                {ui.managing ? "托管中..." : "托管此项目"}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
