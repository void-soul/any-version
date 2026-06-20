import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ExternalLink,
  ShieldCheck,
  HelpCircle,
  RefreshCw,
  CheckCircle,
  Trash2,
  Info,
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
} from "./ProjectSubTabs";

// ── 子标签页类型 ──
type SubTab = "versions" | "envvars" | "cache" | "mirror" | "packages" | "services";

const tabLabels: Record<SubTab, string> = {
  versions: "版本管理",
  envvars: "环境变量",
  cache: "缓存管理",
  mirror: "镜像配置",
  packages: "全局包",
  services: "服务管理",
};

// ── Props ──
interface Props {
  project: ProjectStatus | null;
  onRefresh: () => void;
}

export default function ProjectDetailPanel({ project, onRefresh }: Props) {
  const [detail, setDetail] = useState<ProjectDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [activeSubTab, setActiveSubTab] = useState<SubTab>("versions");
  const [remoteVersions, setRemoteVersions] = useState<string[]>([]);
  const [loadingRemote, setLoadingRemote] = useState(false);
  const [installingVersion, setInstallingVersion] = useState<string | null>(null);
  const [localVersion, setLocalVersion] = useState("");
  const [localPath, setLocalPath] = useState("");
  const [registering, setRegistering] = useState(false);
  const [registerErr, setRegisterErr] = useState<string | null>(null);

  // 托管相关
  const [showManagePreview, setShowManagePreview] = useState(false);
  const [managePreview, setManagePreview] = useState<ManagePreview | null>(null);
  const [managing, setManaging] = useState(false);
  const [unmanaging, setUnmanaging] = useState(false);

  // 缓存管理状态
  const [cacheDestPath, setCacheDestPath] = useState("");
  const [migratingCache, setMigratingCache] = useState(false);

  // 全局包状态
  const [packages, setPackages] = useState<SubTabProps["packages"]>([]);
  const [loadingPackages, setLoadingPackages] = useState(false);
  const [upgradingPackage, setUpgradingPackage] = useState<string | null>(null);
  const [packageError, setPackageError] = useState<string | null>(null);

  // 服务状态
  const [serviceCtrlLoading, setServiceCtrlLoading] = useState(false);

  const def: ProjectDef | null = detail?.def ?? null;
  const status: ProjectStatus | null = detail?.status ?? project;

  // ── 数据获取 ──
  const fetchDetail = useCallback(async () => {
    if (!project) return;
    setLoadingDetail(true);
    try {
      const d = await invoke<ProjectDetail>("project_detail", { id: project.id });
      setDetail(d);
    } catch (e) {
      console.error("Failed to fetch project detail:", e);
      setDetail(null);
    } finally {
      setLoadingDetail(false);
    }
  }, [project]);

  const fetchRemoteVersions = useCallback(async () => {
    if (!project) return;
    setLoadingRemote(true);
    try {
      const versions = await invoke<string[]>("list_remote_versions", { sdkName: project.id });
      setRemoteVersions(versions);
    } catch (e) {
      console.error("Failed to fetch remote versions:", e);
      setRemoteVersions([]);
    } finally {
      setLoadingRemote(false);
    }
  }, [project]);

  const fetchPackages = useCallback(async () => {
    if (!project) return;
    setLoadingPackages(true);
    setPackageError(null);
    try {
      const list = await invoke<SubTabProps["packages"]>("get_global_packages", { sdkName: project.id });
      setPackages(list);
    } catch (e: unknown) {
      setPackageError(String(e));
      setPackages([]);
    } finally {
      setLoadingPackages(false);
    }
  }, [project]);

  // 切换项目时重新加载
  useEffect(() => {
    if (!project) { setDetail(null); return; }
    setActiveSubTab("versions");
    setRemoteVersions([]);
    setDetail(null);
    setLocalVersion("");
    setLocalPath("");
    setRegisterErr(null);
    setShowManagePreview(false);
    setManagePreview(null);
    setPackages([]);
    setPackageError(null);
    setCacheDestPath("");
    fetchDetail();
    fetchRemoteVersions();
  }, [project?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (def?.has_pkg && project) fetchPackages();
  }, [def?.has_pkg, project?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── 操作处理函数 ──
  const handleInstall = async (version: string) => {
    if (!project) return;
    setInstallingVersion(version);
    try {
      await invoke("install_sdk_version", { sdkName: project.id, version: version.split(" ")[0] });
      await fetchDetail();
      onRefresh();
    } catch (e: unknown) {
      alert(`安装失败: ${e}`);
    } finally {
      setInstallingVersion(null);
    }
  };

  const handleUninstall = async (version: string) => {
    if (!project || !status) return;
    if (!confirm(`确定卸载 ${status.display_name} v${version} 吗？`)) return;
    try {
      await invoke("uninstall_sdk_version", { sdkName: project.id, version });
      await fetchDetail();
      onRefresh();
    } catch (e: unknown) {
      alert(`卸载失败: ${e}`);
    }
  };

  const handleUse = async (version: string) => {
    if (!project) return;
    try {
      await invoke("use_sdk_version", { sdkName: project.id, version });
      await fetchDetail();
      onRefresh();
    } catch (e: unknown) {
      alert(`切换版本失败: ${e}`);
    }
  };

  const handleRegisterLocal = async () => {
    if (!project || !localVersion || !localPath) return;
    setRegistering(true);
    setRegisterErr(null);
    try {
      await invoke("add_local_sdk_version", { sdkName: project.id, version: localVersion.trim(), localPath: localPath.trim() });
      setLocalVersion("");
      setLocalPath("");
      await fetchDetail();
      onRefresh();
    } catch (e: unknown) {
      setRegisterErr(String(e));
    } finally {
      setRegistering(false);
    }
  };

  const handlePreviewManage = async () => {
    if (!project) return;
    try {
      const preview = await invoke<ManagePreview>("project_preview_manage", { id: project.id });
      setManagePreview(preview);
    } catch (e) {
      console.warn("Preview not available:", e);
      setManagePreview(null);
    }
    setShowManagePreview(true);
  };

  const handleManage = async () => {
    if (!project) return;
    setManaging(true);
    try {
      await invoke("project_manage", { id: project.id });
      setShowManagePreview(false);
      setManagePreview(null);
      await fetchDetail();
      onRefresh();
    } catch (e: unknown) {
      alert(`托管操作失败: ${e}`);
    } finally {
      setManaging(false);
    }
  };

  const handleUnmanage = async () => {
    if (!project || !status) return;
    if (!confirm(`确定取消对 ${status.display_name} 的托管吗？环境变量将恢复为原始状态。`)) return;
    setUnmanaging(true);
    try {
      await invoke("project_unmanage", { id: project.id });
      await fetchDetail();
      onRefresh();
    } catch (e: unknown) {
      alert(`取消托管失败: ${e}`);
    } finally {
      setUnmanaging(false);
    }
  };

  const handleServiceToggle = async () => {
    if (!project || !status?.service_status) return;
    setServiceCtrlLoading(true);
    try {
      if (status.service_status.running) {
        await invoke("stop_service", { name: project.id });
      } else {
        if (!status.active_version) {
          alert("请先启用一个版本，然后才能启动服务");
          setServiceCtrlLoading(false);
          return;
        }
        await invoke("start_service", { name: project.id, version: status.active_version });
      }
      await fetchDetail();
      onRefresh();
    } catch (e: unknown) {
      alert(`服务操作失败: ${e}`);
    } finally {
      setServiceCtrlLoading(false);
    }
  };

  const handleMigrateCache = async () => {
    if (!project || !cacheDestPath) return;
    if (cacheDestPath.toLowerCase().startsWith("c:")) {
      alert("目标路径必须位于非 C 盘");
      return;
    }
    setMigratingCache(true);
    try {
      await invoke("migrate_cache_path", { name: project.id, newPath: cacheDestPath });
      await fetchDetail();
    } catch (e: unknown) {
      alert(`缓存迁移失败: ${e}`);
    } finally {
      setMigratingCache(false);
    }
  };

  const handleUpgradePackage = async (pkgName: string) => {
    if (!project) return;
    setUpgradingPackage(pkgName);
    setPackageError(null);
    try {
      await invoke("upgrade_global_package", { sdkName: project.id, pkgName });
      await fetchPackages();
    } catch (e: unknown) {
      setPackageError(`升级 ${pkgName} 失败: ${e}`);
    } finally {
      setUpgradingPackage(null);
    }
  };

  // ── 计算可显示的子标签页 ──
  const availableTabs: SubTab[] = ["versions", "envvars"];
  if (def?.has_cache) availableTabs.push("cache");
  if (def?.has_mirror) availableTabs.push("mirror");
  if (def?.has_pkg) availableTabs.push("packages");
  if (def?.is_service || project?.service_status) availableTabs.push("services");

  // ── 子标签页 Props 汇总 ──
  if (!project || !status) {
    return (
      <div className="flex-1 glass-panel rounded-2xl border border-white/5 flex flex-col items-center justify-center text-center text-slate-500 p-8">
        <HelpCircle className="w-12 h-12 text-slate-600 mb-4" />
        <p className="text-xs font-medium text-slate-400">请在左侧列表中选择一个项目进行管理</p>
        <p className="text-[10px] text-slate-500 mt-1">支持语言、开发工具和本地服务的统一管理</p>
      </div>
    );
  }

  const subTabProps: SubTabProps = {
    project: status,
    def,
    remoteVersions, loadingRemote, installingVersion,
    onInstall: handleInstall, onUninstall: handleUninstall, onUse: handleUse,
    localVersion, localPath, registering, registerErr,
    onLocalVersionChange: setLocalVersion, onLocalPathChange: setLocalPath,
    onRegisterLocal: handleRegisterLocal,
    packages, loadingPackages, upgradingPackage, packageError,
    onRefreshPackages: fetchPackages, onUpgradePackage: handleUpgradePackage,
    cacheDestPath, migratingCache,
    onCacheDestPathChange: setCacheDestPath, onMigrateCache: handleMigrateCache,
    serviceCtrlLoading, onServiceToggle: handleServiceToggle,
    onRefresh,
  };

  const tabComponents: Record<SubTab, React.ReactNode> = {
    versions: <VersionsTab {...subTabProps} />,
    envvars: <EnvVarsTab {...subTabProps} />,
    cache: <CacheTab {...subTabProps} />,
    mirror: <MirrorTab {...subTabProps} />,
    packages: <PackagesTab {...subTabProps} />,
    services: <ServicesTab {...subTabProps} />,
  };

  return (
    <div className="flex-1 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col">
      {/* 头部 */}
      <div className="p-5 border-b border-white/5 bg-white/2 flex-shrink-0">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-lg bg-white/5 border border-white/10 flex items-center justify-center text-lg">
              {status.icon || "?"}
            </div>
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
                {status.managed && (
                  <span className="px-1.5 py-0.5 rounded text-[9px] bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-bold flex items-center gap-0.5">
                    <ShieldCheck className="w-2.5 h-2.5" /> 已托管
                  </span>
                )}
              </div>
              <div className="flex items-center gap-2 mt-0.5">
                {def?.official_website && (
                  <button onClick={() => openUrl(def.official_website)} className="text-[10px] text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 cursor-pointer">
                    官方网站 <ExternalLink className="w-2.5 h-2.5" />
                  </button>
                )}
                {status.install_source && (
                  <><span className="text-slate-600 text-[10px]">.</span><span className="text-[10px] text-slate-400">来源: {status.install_source}</span></>
                )}
              </div>
            </div>
          </div>

          <button
            onClick={() => { fetchDetail(); onRefresh(); }}
            disabled={loadingDetail}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
          >
            <RefreshCw className={`w-3 h-3 ${loadingDetail ? "animate-spin" : ""}`} /> 刷新
          </button>
        </div>

        {/* 快速状态栏 */}
        <div className="flex items-center gap-4 mt-3 text-[10px]">
          <div className="flex items-center gap-1.5">
            <span className="text-slate-500">状态:</span>
            {status.installed ? (
              <span className="text-emerald-400 flex items-center gap-1 font-semibold">
                <CheckCircle className="w-3 h-3" /> 已安装
              </span>
            ) : <span className="text-slate-400">未安装</span>}
          </div>
          {status.active_version && (
            <div className="flex items-center gap-1.5">
              <span className="text-slate-500">当前版本:</span>
              <span className="text-blue-400 font-mono font-semibold">v{status.active_version}</span>
            </div>
          )}
          {status.install_root && (
            <div className="flex items-center gap-1.5">
              <span className="text-slate-500">安装路径:</span>
              <span className="text-slate-300 font-mono truncate max-w-[200px]">{status.install_root}</span>
            </div>
          )}
        </div>
      </div>

      {/* 子标签页切换器 */}
      <div className="flex bg-white/5 border border-white/5 rounded-xl p-0.5 mx-5 mt-4 flex-shrink-0">
        {availableTabs.map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveSubTab(tab)}
            className={`flex-1 py-1.5 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
              activeSubTab === tab ? "bg-blue-600 text-white shadow-md" : "text-slate-400 hover:text-slate-200"
            }`}
          >
            {tabLabels[tab]}
          </button>
        ))}
      </div>

      {/* 标签页内容 */}
      <div className="flex-1 overflow-y-auto p-5 space-y-5">
        {loadingDetail ? (
          <div className="flex items-center justify-center gap-2 text-xs text-slate-400 py-8">
            <RefreshCw className="w-4 h-4 animate-spin text-blue-400" /> 正在加载项目详情...
          </div>
        ) : tabComponents[activeSubTab]}
      </div>

      {/* 底部操作栏 */}
      <div className="border-t border-white/5 p-4 bg-white/2 flex-shrink-0">
        {showManagePreview && (
          <div className="mb-4 p-4 bg-blue-600/5 border border-blue-500/15 rounded-xl space-y-3 animate-fadeIn">
            <h4 className="text-xs font-semibold text-blue-300 flex items-center gap-1.5">
              <Info className="w-3.5 h-3.5" /> 托管操作预览 - 将要执行以下操作
            </h4>
            {managePreview && managePreview.steps.length > 0 ? (
              <div className="space-y-2">
                {managePreview.steps.map((step, idx) => (
                  <div key={idx} className="flex items-start gap-2 text-[11px]">
                    <span className="w-5 h-5 rounded-full bg-blue-600/20 text-blue-400 flex items-center justify-center text-[9px] font-bold flex-shrink-0 mt-0.5">{idx + 1}</span>
                    <div>
                      <span className="text-slate-200 font-medium">{step.action}</span>
                      <span className="text-slate-400 ml-1">- {step.description}</span>
                      {step.target && <p className="font-mono text-[10px] text-slate-500 mt-0.5">{step.target}</p>}
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <p className="text-[11px] text-slate-400">将为该项目启用 AnyVersion 托管，接管环境变量和 PATH 配置。</p>
            )}
            <div className="flex items-center gap-2 pt-1">
              <button onClick={handleManage} disabled={managing} className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all">
                {managing ? "正在执行..." : "确认托管"}
              </button>
              <button onClick={() => { setShowManagePreview(false); setManagePreview(null); }} className="px-4 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs font-medium cursor-pointer border border-white/10">
                取消
              </button>
            </div>
          </div>
        )}

        <div className="flex items-center justify-between">
          <div className="text-[10px] text-slate-500">
            {status.managed ? "托管中: 环境变量和 PATH 已由 AnyVersion 管理" : "未托管: 环境变量由系统或手动管理"}
          </div>
          <div className="flex items-center gap-2">
            {status.managed ? (
              <button onClick={handleUnmanage} disabled={unmanaging} className="px-4 py-2 bg-red-600/80 hover:bg-red-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer transition-all flex items-center gap-1.5">
                <Trash2 className="w-3.5 h-3.5" /> {unmanaging ? "正在取消..." : "取消托管"}
              </button>
            ) : (
              <button onClick={handlePreviewManage} disabled={managing} className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold cursor-pointer shadow-md shadow-blue-500/10 transition-all flex items-center gap-1.5">
                <ShieldCheck className="w-3.5 h-3.5" /> 托管此项目
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
