import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { 
  Download, 
  Trash2, 
  Check, 
  ExternalLink,
  Plus,
  RefreshCw,
  FolderOpen,
  CheckCircle,
  HelpCircle,
  FileCheck,
  Globe,
  Box,
  HardDrive,
  FolderSync,
  ArrowRight,
  Link,
  ArrowUpCircle,
  ShieldCheck,
  TrendingUp,
  Settings,
  Info,
  Shield,
  Activity,
  AlertTriangle
} from "lucide-react";

interface SdkInfoTs {
  name: string;
  display_name: string;
  category: string;
  active_version: string;
  installed_versions: string[];
  official_website: string;
  has_cache: boolean;
  has_mirror: boolean;
  has_pkg: boolean;
}

interface ProgressPayload {
  sdk: string;
  downloaded: number;
  total: number;
  pct: number;
}

interface MirrorInfo {
  tool: string;
  current: string;
  mirror_name: string;
}

interface PackageInfo {
  name: string;
  current_version: string;
  latest_version: string;
  status: string; // "latest" | "outdated"
  homepage: string;
}

interface CacheInfo {
  name: string;
  installed: boolean;
  path: string;
  size: string;
  is_link: boolean;
  real_target: string;
  detect_source: string;
  detect_content: string;
}

function categoryLabel(cat: string): string {
  switch (cat) {
    case "language":    return "编程语言";
    case "lib_manager": return "库管理";
    case "tool":        return "开发工具";
    case "service":     return "本地服务";
    default: return cat;
  }
}

const getMirrorToolId = (sdkName: string): string => {
  if (sdkName === "nodejs") return "npm";
  if (sdkName === "python") return "pip";
  return sdkName; // maven, go, rust
};

const getCacheName = (sdkName: string): string => {
  if (sdkName === "nodejs") return "npm";
  if (sdkName === "python") return "pip";
  return sdkName; // yarn, pnpm, maven, nuget
};

const mirrorConfigFile = (tool: string): string => {
  switch (tool) {
    case "npm":
      return "npm 全局配置 (~/.npmrc 或通过 npm config set 修改)";
    case "pip":
      return "%APPDATA%\\pip\\pip.ini（或 PIP_INDEX_URL 环境变量）";
    case "maven":
      return "%USERPROFILE%\\.m2\\settings.xml（Maven 全局 settings）";
    case "go":
      return "通过 go env -w 修改 GOPROXY（Go 模块代理地址）";
    case "rust":
      return "%USERPROFILE%\\.cargo\\config.toml（Cargo 镜像源配置）";
    default:
      return "";
  }
};

const getOptionsForTool = (tool: string) => {
  switch (tool) {
    case "npm":
      return [
        { type: "official", name: "官方源 (Official)" },
        { type: "aliyun", name: "阿里云 (Aliyun)" },
        { type: "tencent", name: "腾讯云 (Tencent)" },
      ];
    case "pip":
      return [
        { type: "official", name: "官方源 (PyPI)" },
        { type: "aliyun", name: "阿里云 (Aliyun)" },
        { type: "tsinghua", name: "清华大学 (Tsinghua)" },
      ];
    case "maven":
      return [
        { type: "official", name: "官方源 (Maven Central)" },
        { type: "aliyun", name: "阿里云 (Aliyun)" },
      ];
    case "go":
      return [
        { type: "official", name: "官方源 (GOPROXY)" },
        { type: "aliyun", name: "阿里云 (Aliyun)" },
        { type: "goproxy", name: "七牛云 (Goproxy.cn)" },
      ];
    case "rust":
      return [
        { type: "official", name: "官方源 (crates.io)" },
        { type: "rsproxy", name: "Rsproxy 镜像 (推荐)" },
        { type: "tsinghua", name: "清华大学 (Tsinghua)" },
      ];
    default:
      return [];
  }
};

export default function SdkManager() {
  const [sdks, setSdks] = useState<SdkInfoTs[]>([]);
  const [selectedSdk, setSelectedSdk] = useState<SdkInfoTs | null>(null);
  const [remoteVersions, setRemoteVersions] = useState<string[]>([]);
  const [loadingRemote, setLoadingRemote] = useState(false);
  const [loadingList, setLoadingList] = useState(false);
  const [installingVersion, setInstallingVersion] = useState<string | null>(null);
  const [downloadProgress, setDownloadProgress] = useState<ProgressPayload | null>(null);
  
  // Local register form state
  const [localVersion, setLocalVersion] = useState("");
  const [localPath, setLocalPath] = useState("");
  const [registering, setRegistering] = useState(false);
  const [registerErr, setRegisterErr] = useState<string | null>(null);

  // Custom version query state
  const [customQueryVersion, setCustomQueryVersion] = useState("");
  const [queryingCustom, setQueryingCustom] = useState(false);
  const [queryUrl, setQueryUrl] = useState<string | null>(null);
  const [queryErr, setQueryErr] = useState<string | null>(null);

  // Picked version download transparency
  const [pickedVersion, setPickedVersion] = useState("");
  const [downloadInfo, setDownloadInfo] = useState<{ url: string; host: string; file_ext: string } | null>(null);
  const [downloadInfoErr, setDownloadInfoErr] = useState<string | null>(null);

  // Detailed status diagnostics state
  const [detailedStatus, setDetailedStatus] = useState<any | null>(null);
  const [loadingStatus, setLoadingStatus] = useState(false);

  // Integrated Config and Sub-Tab state
  const [config, setConfig] = useState<any>(null);
  const [togglingManagement, setTogglingManagement] = useState(false);
  const [activeSubTab, setActiveSubTab] = useState<"control" | "mirrors" | "packages" | "cache">("control");
  const [migrateCacheOnEnable, setMigrateCacheOnEnable] = useState(true);
  const [cacheDestPath, setCacheDestPath] = useState("");

  // Integrated Mirrors state
  const [mirrors, setMirrors] = useState<MirrorInfo[]>([]);
  const [loadingMirrors, setLoadingMirrors] = useState(false);
  const [togglingMirrorTool, setTogglingMirrorTool] = useState<string | null>(null);

  // Integrated Packages state
  const [packages, setPackages] = useState<PackageInfo[]>([]);
  const [loadingPackages, setLoadingPackages] = useState(false);
  const [upgradingPackageName, setUpgradingPackageName] = useState<string | null>(null);
  const [packageErrorMsg, setPackageErrorMsg] = useState<string | null>(null);

  // Integrated Cache state
  const [caches, setCaches] = useState<CacheInfo[]>([]);
  const [loadingCaches, setLoadingCaches] = useState(false);
  const [migratingCacheName, setMigratingCacheName] = useState<string | null>(null);
  const [customCachePaths, setCustomCachePaths] = useState<Record<string, string>>({});

  const fetchConfig = async () => {
    try {
      const conf = await invoke("get_config");
      setConfig(conf);
    } catch (e) {
      console.error("Failed to load config", e);
    }
  };

  const fetchMirrors = async () => {
    setLoadingMirrors(true);
    try {
      const list = await invoke<MirrorInfo[]>("get_mirrors_list");
      setMirrors(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoadingMirrors(false);
    }
  };

  const fetchCaches = async () => {
    setLoadingCaches(true);
    try {
      const list = await invoke<CacheInfo[]>("get_caches_list");
      setCaches(list);
      
      const paths: Record<string, string> = {};
      list.forEach(c => {
        if (!c.is_link) {
          paths[c.name] = `D:\\any-version-caches\\${c.name}`;
        } else {
          paths[c.name] = c.real_target;
        }
      });
      setCustomCachePaths(paths);
    } catch (e) {
      console.error(e);
    } finally {
      setLoadingCaches(false);
    }
  };

  const fetchPackages = async (sdk: string) => {
    setLoadingPackages(true);
    setPackageErrorMsg(null);
    try {
      const list = await invoke<PackageInfo[]>("get_global_packages", { sdkName: sdk });
      setPackages(list);
    } catch (e: any) {
      setPackageErrorMsg(e.toString());
      setPackages([]);
    } finally {
      setLoadingPackages(false);
    }
  };

  const fetchDetailedStatus = async (sdkId: string) => {
    setLoadingStatus(true);
    try {
      const res = await invoke<any>("get_sdk_detailed_status", { sdkId });
      setDetailedStatus(res);
    } catch (e) {
      console.error("Failed to fetch detailed status", e);
      setDetailedStatus(null);
    } finally {
      setLoadingStatus(false);
    }
  };

  const fetchSdks = async () => {
    setLoadingList(true);
    try {
      const list = await invoke<SdkInfoTs[]>("get_sdks_list");
      setSdks(list);
      if (selectedSdk) {
        const updated = list.find(s => s.name === selectedSdk.name);
        if (updated) {
          setSelectedSdk(updated);
          fetchDetailedStatus(updated.name);
        }
      }
    } catch (e) {
      console.error(e);
    } finally {
      setLoadingList(false);
    }
  };

  useEffect(() => {
    fetchSdks();
    fetchConfig();
    fetchMirrors();
    fetchCaches();
  }, []);

  // Listen to download progress events
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    const setupListener = async () => {
      unlisten = await listen<ProgressPayload>("download-progress", (event) => {
        setDownloadProgress(event.payload);
      });
    };
    setupListener();
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const handleSelectSdk = async (sdk: SdkInfoTs) => {
    setSelectedSdk(sdk);
    setRemoteVersions([]);
    setLoadingRemote(true);
    setLocalVersion("");
    setLocalPath("");
    setRegisterErr(null);
    setCustomQueryVersion("");
    setQueryUrl(null);
    setQueryErr(null);
    setPickedVersion("");
    setDownloadInfo(null);
    setDownloadInfoErr(null);

    // Setup cache relocation defaults
    const cacheName = getCacheName(sdk.name);
    setCacheDestPath(`D:\\any-version-caches\\${cacheName}`);
    setMigrateCacheOnEnable(true);
    setActiveSubTab("control");

    try {
      const versions = await invoke<string[]>("list_remote_versions", { sdkName: sdk.name });
      setRemoteVersions(versions);
    } catch (e) {
      console.error(e);
    } finally {
      setLoadingRemote(false);
    }

    if (sdk.has_pkg) {
      fetchPackages(sdk.name);
    }
    fetchDetailedStatus(sdk.name);
  };

  const handleToggleManagement = async () => {
    if (!selectedSdk) return;
    const isManaged = config?.managed_items?.includes(selectedSdk.name) || false;
    const targetStatus = !isManaged;
    
    setTogglingManagement(true);
    try {
      const cacheDest = (targetStatus && selectedSdk.has_cache && migrateCacheOnEnable) 
        ? cacheDestPath 
        : null;
      await invoke("toggle_item_management", {
        id: selectedSdk.name,
        enable: targetStatus,
        cacheDest
      });
      await fetchConfig();
      await fetchSdks();
      if (selectedSdk.has_cache) {
        await fetchCaches();
      }
    } catch (e: any) {
      alert(`切换托管状态失败: ${e}`);
    } finally {
      setTogglingManagement(false);
    }
  };

  const handleSetMirror = async (tool: string, type: string) => {
    setTogglingMirrorTool(tool);
    try {
      await invoke("set_mirror", { tool, mirrorType: type });
      await fetchMirrors();
    } catch (e: any) {
      alert(`配置镜像失败: ${e}`);
    } finally {
      setTogglingMirrorTool(null);
    }
  };

  const handleUpgradePackage = async (pkgName: string) => {
    if (!selectedSdk) return;
    setUpgradingPackageName(pkgName);
    setPackageErrorMsg(null);
    try {
      await invoke("upgrade_global_package", { sdkName: selectedSdk.name, pkgName });
      alert(`包 ${pkgName} 升级成功！`);
      await fetchPackages(selectedSdk.name);
    } catch (e: any) {
      setPackageErrorMsg(`升级 ${pkgName} 失败: ${e}`);
    } finally {
      setUpgradingPackageName(null);
    }
  };

  const handleMigrateCache = async (name: string) => {
    const target = customCachePaths[name];
    if (!target) return;
    if (target.toLowerCase().startsWith("c:")) {
      alert("错误: 目标重定向目录必须位于非 C 盘 (例如 D:\\...)，以腾出 C 盘空间。");
      return;
    }
    setMigratingCacheName(name);
    try {
      await invoke("migrate_cache_path", { name, newPath: target });
      await fetchCaches();
    } catch (e: any) {
      alert(`重定向缓存失败: ${e}`);
    } finally {
      setMigratingCacheName(null);
    }
  };

  const fetchDownloadInfo = async (version: string) => {
    setDownloadInfo(null);
    setDownloadInfoErr(null);
    if (!selectedSdk || !version) return;
    try {
      const info = await invoke<{ url: string; host: string; file_ext: string }>("get_sdk_download_info", {
        sdkName: selectedSdk.name,
        version,
      });
      setDownloadInfo(info);
    } catch (e: any) {
      setDownloadInfoErr(e.toString());
    }
  };

  const handleQueryCustomVersion = async () => {
    if (!selectedSdk || !customQueryVersion) return;
    setQueryingCustom(true);
    setQueryUrl(null);
    setQueryErr(null);
    try {
      const url = await invoke<string>("query_custom_version", {
        sdkName: selectedSdk.name,
        version: customQueryVersion.trim()
      });
      setQueryUrl(url);
    } catch (e: any) {
      setQueryErr(e.toString());
    } finally {
      setQueryingCustom(false);
    }
  };

  const handleInstall = async (version: string) => {
    if (!selectedSdk) return;
    setInstallingVersion(version);
    setDownloadProgress(null);
    try {
      await invoke("install_sdk_version", { 
        sdkName: selectedSdk.name, 
        version: version.split(" ")[0]
      });
      await fetchSdks();
    } catch (e: any) {
      alert(`安装失败: ${e}`);
    } finally {
      setInstallingVersion(null);
      setDownloadProgress(null);
    }
  };

  const handleUninstall = async (version: string) => {
    if (!selectedSdk) return;
    if (!confirm(`确定卸载 ${selectedSdk.display_name} v${version} 吗？`)) return;
    try {
      await invoke("uninstall_sdk_version", { sdkName: selectedSdk.name, version });
      await fetchSdks();
    } catch (e: any) {
      alert(`卸载失败: ${e}`);
    }
  };

  const handleUse = async (version: string) => {
    if (!selectedSdk) return;
    try {
      await invoke("use_sdk_version", { sdkName: selectedSdk.name, version });
      await fetchSdks();
    } catch (e: any) {
      alert(`切换版本失败: ${e}`);
    }
  };

  const handleRegisterLocal = async () => {
    if (!selectedSdk || !localVersion || !localPath) return;
    setRegistering(true);
    setRegisterErr(null);
    try {
      await invoke("add_local_sdk_version", {
        sdkName: selectedSdk.name,
        version: localVersion.trim(),
        localPath: localPath.trim()
      });
      setLocalVersion("");
      setLocalPath("");
      await fetchSdks();
    } catch (e: any) {
      setRegisterErr(e);
    } finally {
      setRegistering(false);
    }
  };

  const isManaged = selectedSdk ? (config?.managed_items?.includes(selectedSdk.name) || false) : false;

  const renderControlTab = () => {
    if (!selectedSdk) return null;
    return (
      <div className="space-y-6">
        {/* Toggle Management Card */}
        <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <h4 className="text-xs font-semibold text-white">AnyVersion 托管状态</h4>
              <p className="text-[10px] text-slate-400 mt-1">
                托管后，AnyVersion 将接管该 SDK 的环境变量和 PATH；未托管时将安全还原您原本的环境变量。
              </p>
            </div>
            <button
              onClick={handleToggleManagement}
              disabled={togglingManagement}
              className={`px-4 py-2 rounded-xl text-xs font-semibold cursor-pointer transition-all ${
                isManaged 
                  ? "bg-emerald-600 hover:bg-emerald-500 text-white shadow-md shadow-emerald-500/10" 
                  : "bg-blue-600 hover:bg-blue-500 text-white shadow-md shadow-blue-500/10"
              }`}
            >
              {togglingManagement ? "操作中..." : isManaged ? "已启用托管 (点击关闭)" : "开启 AnyVersion 托管"}
            </button>
          </div>

          {!isManaged && selectedSdk.has_cache && (
            <div className="p-3 bg-black/20 rounded-xl space-y-2 border border-white/5">
              <label className="flex items-center gap-2 text-[11px] text-slate-300 cursor-pointer select-none">
                <input
                  type="checkbox"
                  checked={migrateCacheOnEnable}
                  onChange={(e) => setMigrateCacheOnEnable(e.target.checked)}
                  className="rounded border-white/10 bg-black/20 text-blue-600"
                />
                <span>同时开启缓存重定向（转移 C 盘缓存并链接到指定非 C 盘目录）</span>
              </label>
              {migrateCacheOnEnable && (
                <div className="flex items-center gap-2 mt-1.5 animate-fadeIn">
                  <span className="text-[10px] text-slate-500 flex-shrink-0">目标路径:</span>
                  <input
                    type="text"
                    value={cacheDestPath}
                    onChange={(e) => setCacheDestPath(e.target.value)}
                    className="flex-1 glass-input px-2.5 py-1 text-[11px] font-mono"
                    placeholder="例如: D:\any-version-caches"
                  />
                </div>
              )}
            </div>
          )}
        </div>

        {/* Warning if unmanaged */}
        {!isManaged && (
          <div className="p-3.5 bg-amber-500/5 border border-amber-500/15 text-amber-400 rounded-xl text-xs flex items-start gap-2 animate-fadeIn">
            <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />
            <div>
              <p className="font-semibold">当前 SDK/服务 未开启 AnyVersion 托管</p>
              <p className="text-[10px] text-slate-400 mt-0.5">请先在上方启用托管。开启托管后，环境变量和可执行二进制文件的优先级将被 AnyVersion 安全注入，您即可进行版本切换与服务生命周期管理。</p>
            </div>
          </div>
        )}

        {/* Detailed Diagnostics Card */}
        {loadingStatus ? (
          <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 flex items-center justify-center gap-2 text-xs text-slate-400">
            <RefreshCw className="w-4 h-4 animate-spin text-blue-400" />
            正在扫描本地环境状态...
          </div>
        ) : detailedStatus ? (
          <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
            <div className="flex items-center gap-2 border-b border-white/5 pb-3">
              <ShieldCheck className="w-4 h-4 text-blue-400" />
              <h4 className="text-xs font-semibold text-white">电脑本地当前状态诊断</h4>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs">
              <div className="space-y-3">
                <div className="flex items-start gap-2">
                  <div className="w-20 text-slate-500 font-medium flex-shrink-0">安装状态:</div>
                  <div className="flex items-center gap-1.5 flex-wrap">
                    {detailedStatus.is_installed ? (
                      <>
                        <span className="text-emerald-400 font-semibold flex items-center gap-1">
                          <CheckCircle className="w-3.5 h-3.5" /> 已检测到安装
                        </span>
                        {detailedStatus.version && (
                          <span className="px-1.5 py-0.2 bg-blue-500/10 text-blue-400 border border-blue-500/20 rounded font-mono text-[10px]">
                            v{detailedStatus.version}
                          </span>
                        )}
                      </>
                    ) : (
                      <span className="text-slate-400 font-medium">未检测到安装</span>
                    )}
                  </div>
                </div>

                <div className="flex items-start gap-2">
                  <div className="w-20 text-slate-500 font-medium flex-shrink-0">安装目录:</div>
                  <div className="font-mono text-slate-300 break-all select-text flex-1">
                    {detailedStatus.install_path || <span className="text-slate-500">无</span>}
                  </div>
                </div>

                <div className="flex items-start gap-2">
                  <div className="w-20 text-slate-500 font-medium flex-shrink-0">管理类型:</div>
                  <div className="flex items-center gap-1.5">
                    {detailedStatus.is_managed ? (
                      <span className="text-emerald-400 font-semibold flex items-center gap-1">
                        <Shield className="w-3.5 h-3.5" /> AnyVersion 托管中
                      </span>
                    ) : (
                      <span className="text-amber-400 font-semibold flex items-center gap-1">
                        <AlertTriangle className="w-3.5 h-3.5" /> 自行安装 (未托管)
                      </span>
                    )}
                  </div>
                </div>
              </div>

              {selectedSdk.has_cache && (
                <div className="space-y-3 border-t md:border-t-0 md:border-l border-white/5 pt-3 md:pt-0 md:pl-4">
                  <div className="flex items-start gap-2">
                    <div className="w-20 text-slate-500 font-medium flex-shrink-0">缓存路径:</div>
                    <div className="font-mono text-slate-300 break-all select-text flex-1">
                      {detailedStatus.cache_path || <span className="text-slate-500">无</span>}
                    </div>
                  </div>

                  <div className="flex items-start gap-2">
                    <div className="w-20 text-slate-500 font-medium flex-shrink-0">缓存体积:</div>
                    <div className="font-mono text-slate-300">
                      {detailedStatus.cache_size || <span className="text-slate-500">0 B</span>}
                    </div>
                  </div>

                  <div className="flex items-start gap-2">
                    <div className="w-20 text-slate-500 font-medium flex-shrink-0">缓存重定向:</div>
                    <div>
                      {detailedStatus.cache_is_redirected ? (
                        <span className="text-emerald-400 font-semibold flex items-center gap-1">
                          <CheckCircle className="w-3.5 h-3.5" /> 已完成重定向 (非 C 盘)
                        </span>
                      ) : (
                        <span className="text-amber-400 font-semibold flex items-center gap-1">
                          <AlertTriangle className="w-3.5 h-3.5" /> 位于 C 盘 (占用空间)
                        </span>
                      )}
                    </div>
                  </div>
                </div>
              )}

              {detailedStatus.is_service && (
                <div className="space-y-3 border-t md:border-t-0 md:border-l border-white/5 pt-3 md:pt-0 md:pl-4">
                  <div className="flex items-start gap-2">
                    <div className="w-20 text-slate-500 font-medium flex-shrink-0">运行状态:</div>
                    <div>
                      {detailedStatus.service_status === "running" ? (
                        <span className="text-emerald-400 font-semibold flex items-center gap-1 animate-fadeIn">
                          <Activity className="w-3.5 h-3.5 animate-pulse text-emerald-400" /> 运行中 (PID: {detailedStatus.pid})
                        </span>
                      ) : (
                        <span className="text-slate-400 font-medium flex items-center gap-1">
                          <AlertTriangle className="w-3.5 h-3.5" /> 已停止
                        </span>
                      )}
                    </div>
                  </div>

                  <div className="flex items-start gap-2">
                    <div className="w-20 text-slate-500 font-medium flex-shrink-0">服务端口:</div>
                    <div className="font-mono text-slate-300">
                      {detailedStatus.port || detailedStatus.default_port}
                    </div>
                  </div>

                  {detailedStatus.data_path && (
                    <div className="flex items-start gap-2">
                      <div className="w-20 text-slate-500 font-medium flex-shrink-0">数据路径:</div>
                      <div className="font-mono text-slate-300 break-all select-text flex-1">
                        {detailedStatus.data_path}
                      </div>
                    </div>
                  )}

                  {detailedStatus.log_path && (
                    <div className="flex items-start gap-2">
                      <div className="w-20 text-slate-500 font-medium flex-shrink-0">日志路径:</div>
                      <div className="font-mono text-slate-300 break-all select-text flex-1">
                        {detailedStatus.log_path}
                      </div>
                    </div>
                  )}
                </div>
              )}
            </div>

            {detailedStatus.env_vars && detailedStatus.env_vars.length > 0 && (
              <div className="space-y-2 pt-3 border-t border-white/5">
                <span className="text-[11px] font-semibold text-slate-400 block">环境变量诊断详情</span>
                <div className="border border-white/5 rounded-xl overflow-hidden overflow-x-auto">
                  <table className="w-full text-left border-collapse text-[10px] min-w-[500px]">
                    <thead>
                      <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-medium">
                        <th className="p-2 w-28">变量名</th>
                        <th className="p-2 w-32">说明</th>
                        <th className="p-2">当前配置值</th>
                        <th className="p-2 w-24">配置来源</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-white/5 text-slate-300">
                      {detailedStatus.env_vars.map((v: any) => (
                        <tr key={v.name} className="hover:bg-white/1 font-mono">
                          <td className="p-2 font-semibold text-slate-200">{v.name}</td>
                          <td className="p-2 text-slate-400 font-sans">{v.desc}</td>
                          <td className="p-2 break-all select-text">{v.current_value || <span className="text-slate-600 font-sans">未配置</span>}</td>
                          <td className="p-2">
                            {v.source === "HKCU" ? (
                              <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-semibold">
                                用户级 (HKCU)
                              </span>
                            ) : v.source === "HKLM" ? (
                              <span className="px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-400 border border-indigo-500/20 text-[9px] font-semibold">
                                系统级 (HKLM)
                              </span>
                            ) : (
                              <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-[9px]">
                                未设置
                              </span>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}
          </div>
        ) : null}

        {/* Managed-only configurations */}
        {isManaged && (
          <div className="space-y-6">
            {/* Service Controls Panel */}
            {selectedSdk.category === "service" && (
              <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4 animate-fadeIn">
                <div className="flex items-center gap-2 border-b border-white/5 pb-3">
                  <Activity className="w-4 h-4 text-blue-400" />
                  <h4 className="text-xs font-semibold text-white">本地服务控制台</h4>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-xs">
                  {/* Status */}
                  <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-1.5">
                    <span className="text-[10px] text-slate-400 font-semibold uppercase tracking-wider block">当前状态</span>
                    <div className="flex items-center gap-2">
                      {detailedStatus?.service_status === "running" ? (
                        <span className="px-2.5 py-1 rounded-lg bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold flex items-center gap-1 animate-fadeIn">
                          <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-ping" />
                          运行中 (PID: {detailedStatus?.pid})
                        </span>
                      ) : (
                        <span className="px-2.5 py-1 rounded-lg bg-slate-500/10 text-slate-400 border border-white/5 font-semibold flex items-center gap-1">
                          已停止
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Port & Version */}
                  <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-1">
                    <span className="text-[10px] text-slate-400 font-semibold uppercase tracking-wider block">运行参数</span>
                    <div className="text-slate-300 font-mono space-y-0.5">
                      <p>服务端口: {detailedStatus?.port || detailedStatus?.default_port || "无"}</p>
                      <p>启动版本: {selectedSdk.active_version || "未启用"}</p>
                    </div>
                  </div>

                  {/* Controls */}
                  <div className="p-3 bg-black/20 rounded-xl border border-white/5 flex items-center justify-center gap-2">
                    {detailedStatus?.service_status === "running" ? (
                      <button
                        onClick={async () => {
                          try {
                            await invoke("stop_service", { name: selectedSdk.name });
                            fetchDetailedStatus(selectedSdk.name);
                            fetchSdks();
                          } catch (e) {
                            alert(`停止服务失败: ${e}`);
                          }
                        }}
                        className="px-4 py-2 bg-red-600 hover:bg-red-500 text-white font-semibold rounded-xl text-xs cursor-pointer shadow-md shadow-red-500/10 transition-all flex items-center gap-1"
                      >
                        停止服务
                      </button>
                    ) : (
                      <button
                        onClick={async () => {
                          if (!selectedSdk.active_version) {
                            alert("请先在下方选择并启用一个本地安装版本，然后才能启动服务");
                            return;
                          }
                          try {
                            await invoke("start_service", { name: selectedSdk.name, version: selectedSdk.active_version });
                            fetchDetailedStatus(selectedSdk.name);
                            fetchSdks();
                          } catch (e) {
                            alert(`启动服务失败: ${e}`);
                          }
                        }}
                        className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white font-semibold rounded-xl text-xs cursor-pointer shadow-md shadow-emerald-500/10 transition-all flex items-center gap-1"
                      >
                        启动服务
                      </button>
                    )}
                  </div>
                </div>

                {/* Paths Control */}
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs pt-2">
                  {detailedStatus?.data_path && (
                    <div className="p-3 bg-black/20 rounded-xl border border-white/5 flex items-center justify-between gap-4">
                      <div className="min-w-0 flex-1">
                        <span className="text-[10px] text-slate-400 font-semibold block">服务数据路径</span>
                        <p className="font-mono text-slate-300 truncate mt-1">{detailedStatus.data_path}</p>
                      </div>
                      <button
                        onClick={() => openUrl(detailedStatus.data_path)}
                        className="p-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg border border-white/5 cursor-pointer flex-shrink-0"
                        title="在资源管理器中打开数据目录"
                      >
                        <FolderOpen className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  )}

                  {detailedStatus?.log_path && (
                    <div className="p-3 bg-black/20 rounded-xl border border-white/5 flex items-center justify-between gap-4">
                      <div className="min-w-0 flex-1">
                        <span className="text-[10px] text-slate-400 font-semibold block">服务日志路径</span>
                        <p className="font-mono text-slate-300 truncate mt-1">{detailedStatus.log_path}</p>
                      </div>
                      <button
                        onClick={() => openUrl(detailedStatus.log_path)}
                        className="p-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg border border-white/5 cursor-pointer flex-shrink-0"
                        title="在资源管理器中打开日志目录"
                      >
                        <FolderOpen className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* Mobile SDK notes */}
            {selectedSdk.category === "tool" && (selectedSdk.name === "android" || selectedSdk.name === "harmony") && (
              <div className="p-3.5 rounded-xl bg-indigo-500/5 border border-indigo-500/15 space-y-1.5">
                <span className="text-[11px] font-semibold text-indigo-300">
                  {selectedSdk.name === "android" ? "关于 Android SDK" : "关于鸿蒙 HarmonyOS SDK"}
                </span>
                {selectedSdk.name === "android" ? (
                  <p className="text-[10px] text-slate-300 leading-relaxed">
                    这里下载的是 Google 官方「命令行工具(commandline-tools)」，列表里的数字是官方构建号(build number)。
                    启用后，AnyVersion 会将 <span className="font-mono text-indigo-300">ANDROID_HOME</span> 指向本项目的稳定链接目录。
                  </p>
                ) : (
                  <p className="text-[10px] text-slate-300 leading-relaxed">
                    鸿蒙命令行工具需要在华为开发者官网登录后下载。请先下载并解压，然后用下方「手动注册本地已存在 SDK」填入版本号和解压目录即可。注册后会自动配置
                    <span className="font-mono text-indigo-300"> OHOS_SDK_HOME</span>。
                  </p>
                )}
              </div>
            )}

            {/* Installed versions */}
            <div className="space-y-3">
              <h4 className="text-xs font-semibold text-slate-300">本地已安装版本</h4>
              {selectedSdk.installed_versions.length === 0 ? (
                <p className="text-[11px] text-slate-500">尚未安装任何版本。请从下方远程版本选择安装，或注册本地已有路径。</p>
              ) : (
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  {selectedSdk.installed_versions.map((v) => {
                    const isActive = selectedSdk.active_version === v;
                    return (
                      <div 
                        key={v}
                        className={`p-3 rounded-xl border flex items-center justify-between transition-all ${
                          isActive 
                            ? "bg-blue-600/10 border-blue-500/30 text-white shadow-md shadow-blue-500/5" 
                            : "bg-black/20 border-white/5 text-slate-300"
                        }`}
                      >
                        <span className="font-mono text-xs font-medium">{v}</span>
                        <div className="flex items-center gap-1.5">
                          {!isActive && (
                            <button
                              onClick={async () => {
                                await handleUse(v);
                                fetchDetailedStatus(selectedSdk.name);
                              }}
                              className="p-1.5 hover:bg-white/10 rounded-lg text-slate-400 hover:text-slate-200 text-[10px] cursor-pointer transition-all flex items-center gap-0.5"
                            >
                              <Check className="w-3.5 h-3.5" />
                              启用
                            </button>
                          )}
                          <button
                            onClick={async () => {
                              await handleUninstall(v);
                              fetchDetailedStatus(selectedSdk.name);
                            }}
                            className="p-1.5 hover:bg-red-500/10 hover:text-red-400 rounded-lg text-slate-500 cursor-pointer transition-all"
                            title="卸载"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>

            {/* Remote Installer */}
            <div className="space-y-3.5 border-t border-white/5 pt-4">
              <h4 className="text-xs font-semibold text-slate-300">在线安装远程版本</h4>
              {loadingRemote ? (
                <div className="flex items-center gap-2 text-slate-400 text-xs py-2">
                  <RefreshCw className="w-4 h-4 animate-spin text-blue-400" />
                  正在获取远程版本列表...
                </div>
              ) : (
                <div className="space-y-4">
                  <div className="flex items-center gap-3">
                    <select
                      className="flex-1 glass-input px-3.5 py-2 text-xs"
                      value={pickedVersion}
                      onChange={(e) => {
                        const v = e.target.value;
                        setPickedVersion(v);
                        if (v) fetchDownloadInfo(v.split(" ")[0]);
                      }}
                    >
                      <option value="">-- 请选择版本 --</option>
                      {remoteVersions.map((v) => (
                        <option key={v} value={v}>
                          {v}
                        </option>
                      ))}
                    </select>

                    <button
                      onClick={async () => {
                        await handleInstall(pickedVersion);
                        fetchDetailedStatus(selectedSdk.name);
                      }}
                      disabled={!pickedVersion || installingVersion !== null}
                      className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-md shadow-blue-500/10 cursor-pointer transition-all flex items-center gap-1.5"
                    >
                      <Download className="w-3.5 h-3.5" />
                      {installingVersion ? "正在安装..." : "一键安装"}
                    </button>
                  </div>

                  {downloadInfoErr && (
                    <p className="text-[10px] text-red-400 font-medium">获取下载信息失败: {downloadInfoErr}</p>
                  )}

                  {downloadInfo && (
                    <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-1.5 animate-fadeIn">
                      <div className="flex items-center justify-between text-[9px] text-slate-400">
                        <span>下载透明度 (透明下载，安全保证)</span>
                        <span className="font-semibold text-blue-400 flex items-center gap-0.5">
                          <Globe className="w-3 h-3" />
                          已解析至国内镜像
                        </span>
                      </div>
                      <div className="font-mono text-[9px] text-slate-300 break-all space-y-1">
                        <p>
                          <span className="text-slate-500">源地址:</span> {downloadInfo.url}
                        </p>
                        <p>
                          <span className="text-slate-500">服务器:</span> {downloadInfo.host}
                        </p>
                        <p>
                          <span className="text-slate-500">文件格式:</span> {downloadInfo.file_ext}
                        </p>
                      </div>
                    </div>
                  )}

                  {installingVersion && downloadProgress && (
                    <div className="space-y-2 p-3 bg-blue-600/5 border border-blue-500/10 rounded-xl animate-fadeIn">
                      <div className="flex justify-between text-[10px] text-slate-300">
                        <span>正在下载 {selectedSdk.display_name} v{installingVersion.split(" ")[0]}...</span>
                        <span className="font-mono font-semibold">{downloadProgress.pct}%</span>
                      </div>
                      <div className="w-full bg-white/5 rounded-full h-1.5 overflow-hidden">
                        <div 
                          className="bg-blue-500 h-1.5 rounded-full transition-all duration-300"
                          style={{ width: `${downloadProgress.pct}%` }}
                        />
                      </div>
                      <div className="flex justify-between text-[9px] text-slate-500 font-mono">
                        <span>已下载: {(downloadProgress.downloaded / 1024 / 1024).toFixed(2)} MB</span>
                        <span>总大小: {(downloadProgress.total / 1024 / 1024).toFixed(2)} MB</span>
                      </div>
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* Custom installer/registration */}
            <div className="space-y-3.5 border-t border-white/5 pt-4">
              <h4 className="text-xs font-semibold text-slate-300">手动注册本地已存在 SDK/服务</h4>
              <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/1 space-y-4">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div className="space-y-1.5">
                    <label className="text-[10px] text-slate-400 font-medium">指定版本号:</label>
                    <input
                      type="text"
                      className="w-full glass-input px-3.5 py-2 text-xs font-mono"
                      value={localVersion}
                      onChange={(e) => setLocalVersion(e.target.value)}
                      placeholder="例如: 18.16.0"
                    />
                  </div>
                  <div className="space-y-1.5">
                    <label className="text-[10px] text-slate-400 font-medium">本地路径 (bin 的父目录):</label>
                    <input
                      type="text"
                      className="w-full glass-input px-3.5 py-2 text-xs font-mono"
                      value={localPath}
                      onChange={(e) => setLocalPath(e.target.value)}
                      placeholder="例如: D:\my-local-sdks\nodejs"
                    />
                  </div>
                </div>

                <div className="flex items-center justify-between pt-1">
                  <div>
                    {registerErr && (
                      <span className="text-[10px] text-red-400 font-medium">{registerErr}</span>
                    )}
                  </div>
                  <button
                    onClick={async () => {
                      await handleRegisterLocal();
                      fetchDetailedStatus(selectedSdk.name);
                    }}
                    disabled={registering || !localVersion || !localPath}
                    className="px-5 py-2 bg-white/5 border border-white/10 hover:bg-white/10 disabled:opacity-50 text-slate-300 rounded-lg text-xs font-medium cursor-pointer transition-all flex items-center gap-1.5"
                  >
                    <Plus className="w-3.5 h-3.5" />
                    注册本地版本
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    );
  };;

  const renderMirrorsTab = () => {
    if (!selectedSdk) return null;
    if (!isManaged) {
      return (
        <div className="p-8 text-center text-slate-500">
          <Globe className="w-12 h-12 mx-auto text-slate-600 mb-3" />
          <p className="text-xs font-semibold text-slate-400">托管未开启</p>
          <p className="text-[10px] text-slate-500 mt-1">请先在「托管与版本管理」中开启 AnyVersion 托管，然后再配置国内镜像加速。</p>
        </div>
      );
    }

    const mirrorToolId = getMirrorToolId(selectedSdk.name);
    const m = mirrors.find(item => item.tool === mirrorToolId);
    if (!m) {
      return (
        <div className="p-8 text-center text-slate-500">
          <Globe className="w-12 h-12 mx-auto text-slate-600 mb-3" />
          <p className="text-xs font-semibold text-slate-400">未找到镜像代理规则</p>
          <p className="text-[10px] text-slate-500 mt-1">未能在注册表中为该 SDK 匹配到合法的镜像代理写入规则。</p>
        </div>
      );
    }

    const isToggling = togglingMirrorTool === m.tool;
    const options = getOptionsForTool(m.tool);

    return (
      <div className="space-y-6">
        <div className="glass-panel rounded-2xl p-5 border border-white/5 flex flex-col justify-between gap-4 bg-white/2">
          <div className="space-y-1.5 flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <h3 className="text-xs font-semibold text-white uppercase">{m.tool.toUpperCase()} 镜像加速</h3>
              <span className="px-2 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-mono">
                {m.mirror_name}
              </span>
            </div>
            <p className="font-mono text-[10px] text-slate-400 truncate">
              当前配置源: {m.current}
            </p>
            <p className="text-[9px] text-slate-500 flex items-center gap-1 mt-1.5">
              <Info className="w-3 h-3 text-slate-400" />
              写入位置: <span className="font-mono text-slate-400">{mirrorConfigFile(m.tool)}</span>
            </p>
          </div>

          <div className="flex items-center gap-2 flex-wrap pt-2">
            {options.map((opt) => {
              const isCurrent = m.mirror_name.toLowerCase().includes(opt.type.toLowerCase()) 
                || (opt.type === "official" && m.mirror_name.toLowerCase() === "official")
                || (opt.type === "goproxy" && m.mirror_name.toLowerCase().includes("goproxy"))
                || (opt.type === "rsproxy" && m.mirror_name.toLowerCase().includes("rsproxy"));

              return (
                <button
                  key={opt.type}
                  onClick={() => handleSetMirror(m.tool, opt.type)}
                  disabled={isToggling}
                  className={`px-3.5 py-2 rounded-lg text-xs font-medium cursor-pointer transition-all ${
                    isCurrent 
                      ? "bg-blue-600 text-white shadow-md shadow-blue-500/10" 
                      : "bg-white/5 border border-white/5 hover:bg-white/10 text-slate-300"
                  }`}
                >
                  {opt.name}
                </button>
              );
            })}
          </div>
        </div>
      </div>
    );
  };

  const renderPackagesTab = () => {
    if (!selectedSdk) return null;
    if (!isManaged) {
      return (
        <div className="p-8 text-center text-slate-500">
          <Box className="w-12 h-12 mx-auto text-slate-600 mb-3" />
          <p className="text-xs font-semibold text-slate-400">托管未开启</p>
          <p className="text-[10px] text-slate-500 mt-1">请先在「托管与版本管理」中开启 AnyVersion 托管，然后再管理全局第三方依赖包。</p>
        </div>
      );
    }

    return (
      <div className="space-y-4 flex flex-col h-full min-h-0">
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold text-slate-300">全局依赖包列表</span>
          <button
            onClick={() => fetchPackages(selectedSdk.name)}
            disabled={loadingPackages}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
          >
            <RefreshCw className={`w-3 h-3 ${loadingPackages ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>

        {packageErrorMsg && (
          <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-[11px] font-mono break-all">
            {packageErrorMsg}
          </div>
        )}

        <div className="flex-1 min-h-0 glass-panel border border-white/5 rounded-2xl overflow-hidden flex flex-col">
          <div className="flex-1 overflow-y-auto max-h-[300px]">
            <table className="w-full text-left border-collapse text-[11px]">
              <thead>
                <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-semibold">
                  <th className="p-3">依赖包名称</th>
                  <th className="p-3 w-24">当前版本</th>
                  <th className="p-3 w-24">最新版本</th>
                  <th className="p-3 w-20">状态</th>
                  <th className="p-3 w-20 text-center">操作</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/5">
                {loadingPackages ? (
                  <tr>
                    <td colSpan={5} className="p-8 text-center text-slate-500">
                      <RefreshCw className="w-5 h-5 animate-spin text-blue-400 mx-auto mb-2" />
                      正在扫描全局依赖包列表...
                    </td>
                  </tr>
                ) : packages.length === 0 ? (
                  <tr>
                    <td colSpan={5} className="p-8 text-center text-slate-500">
                      无全局依赖包，或环境未安装就绪
                    </td>
                  </tr>
                ) : (
                  packages.map((pkg) => {
                    const isUpgrading = upgradingPackageName === pkg.name;
                    return (
                      <tr key={pkg.name} className="hover:bg-white/2 text-slate-300">
                        <td className="p-3 font-semibold text-slate-200">
                          <button
                            onClick={() => openUrl(pkg.homepage)}
                            className="inline-flex items-center gap-1 hover:text-blue-400 transition-colors cursor-pointer group text-[11px]"
                          >
                            {pkg.name}
                            <ExternalLink className="w-3 h-3 text-slate-500 group-hover:text-blue-400 opacity-0 group-hover:opacity-100" />
                          </button>
                        </td>
                        <td className="p-3 font-mono">{pkg.current_version}</td>
                        <td className="p-3 font-mono text-slate-400">{pkg.latest_version}</td>
                        <td className="p-3">
                          {pkg.status === "outdated" ? (
                            <span className="px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 text-[9px] font-semibold">
                              可升级
                            </span>
                          ) : (
                            <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[9px] font-semibold">
                              最新
                            </span>
                          )}
                        </td>
                        <td className="p-3 text-center">
                          {pkg.status === "outdated" ? (
                            <button
                              onClick={() => handleUpgradePackage(pkg.name)}
                              disabled={isUpgrading}
                              className="px-2.5 py-1 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-md text-[9px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-0.5 mx-auto"
                            >
                              <ArrowUpCircle className="w-3 h-3" />
                              {isUpgrading ? "升级中" : "升级"}
                            </button>
                          ) : (
                            <span className="text-[10px] text-slate-600">无需更新</span>
                          )}
                        </td>
                      </tr>
                    );
                  })
                )}
              </tbody>
            </table>
          </div>
        </div>
      </div>
    );
  };

  const renderCacheTab = () => {
    if (!selectedSdk) return null;
    if (!isManaged) {
      return (
        <div className="p-8 text-center text-slate-500">
          <HardDrive className="w-12 h-12 mx-auto text-slate-600 mb-3" />
          <p className="text-xs font-semibold text-slate-400">托管未开启</p>
          <p className="text-[10px] text-slate-500 mt-1">请先在「托管与版本管理」中开启 AnyVersion 托管，然后再重定向或清理开发缓存文件。</p>
        </div>
      );
    }

    const cacheName = getCacheName(selectedSdk.name);
    const cache = caches.find(c => c.name === cacheName);
    if (!cache) {
      return (
        <div className="p-8 text-center text-slate-500">
          <HardDrive className="w-12 h-12 mx-auto text-slate-600 mb-3" />
          <p className="text-xs font-semibold text-slate-400">未找到该 SDK 关联的缓存目录</p>
          <p className="text-[10px] text-slate-500 mt-1">未能在注册表中为该 SDK 关联到可重定向的全局包缓存路径。</p>
        </div>
      );
    }

    const isMigrating = migratingCacheName === cache.name;
    const target = customCachePaths[cache.name] || "";

    return (
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <span className="text-xs font-semibold text-slate-300">开发缓存目录管理</span>
          <button
            onClick={fetchCaches}
            disabled={loadingCaches}
            className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
          >
            <RefreshCw className={`w-3 h-3 ${loadingCaches ? "animate-spin" : ""}`} />
            刷新体积
          </button>
        </div>

        <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-4 bg-white/2">
          <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
            <div className="space-y-1 flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <h3 className="text-xs font-semibold text-white uppercase">{cache.name.toUpperCase()} 缓存目录</h3>
                {cache.is_link && (
                  <span className="px-2 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-medium flex items-center gap-0.5 font-mono">
                    <Link className="w-3 h-3" />
                    已完成重定向
                  </span>
                )}
              </div>
              <div className="font-mono text-[10px] text-slate-400 space-y-0.5 truncate mt-1">
                <p>原默认位置: {cache.path}</p>
                {cache.is_link && (
                  <p className="text-blue-400 flex items-center gap-1 font-semibold">
                    <ArrowRight className="w-3 h-3" />
                    真实物理位置: {cache.real_target}
                  </p>
                )}
              </div>
            </div>

            <div className="flex items-center gap-4">
              <div className="flex items-center gap-1 bg-black/20 px-3.5 py-2.5 rounded-xl border border-white/5 min-w-[80px] justify-center">
                <HardDrive className="w-3.5 h-3.5 text-slate-500" />
                <span className="font-mono font-bold text-xs text-white">{cache.size}</span>
              </div>
            </div>

            <div className="flex items-center gap-2">
              {!cache.is_link ? (
                <>
                  <input
                    type="text"
                    value={target}
                    onChange={(e) => setCustomCachePaths({ ...customCachePaths, [cache.name]: e.target.value })}
                    className="glass-input px-3 py-1.5 text-xs w-44 font-mono"
                    placeholder="例如: D:\caches"
                  />
                  <button
                    onClick={() => handleMigrateCache(cache.name)}
                    disabled={isMigrating || !target}
                    className="px-3.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center gap-1"
                  >
                    <FolderSync className="w-3 h-3" />
                    {isMigrating ? "正在迁移..." : "迁移"}
                  </button>
                </>
              ) : (
                <span className="px-3 py-1.5 bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 rounded-lg text-xs font-semibold">
                  重定向已就绪
                </span>
              )}
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-3 pt-3 border-t border-white/5 text-[10px]">
            <div className="p-3 rounded-xl bg-amber-500/5 border border-amber-500/15 space-y-1">
              <span className="font-semibold text-amber-400 uppercase tracking-wide">检测依据</span>
              <p className="text-slate-300 leading-relaxed">{cache.detect_source}</p>
              <p className="font-mono text-slate-400 break-all bg-black/30 rounded p-1.5 border border-white/5 mt-1">{cache.detect_content}</p>
            </div>

            {!cache.is_link ? (
              <div className="p-3 rounded-xl bg-blue-500/5 border border-blue-500/15 space-y-1.5">
                <span className="font-semibold text-blue-400 uppercase tracking-wide">如何修复/转移</span>
                <p className="text-slate-300 leading-relaxed">
                  将缓存从 <span className="font-mono text-slate-200">{cache.path}</span> 整体复制迁移至
                  <span className="font-mono text-emerald-300"> {target || "（请输入目标路径）"}</span>，并在原路径下构建 NTFS 目录 Junction 链接，做到完全无感转移。
                </p>
              </div>
            ) : (
              <div className="p-3 rounded-xl bg-emerald-500/5 border border-emerald-500/15 space-y-1">
                <span className="font-semibold text-emerald-400 uppercase tracking-wide">重定向信息</span>
                <p className="text-slate-300">已在此前通过 AnyVersion 自动重定向成功。真实存储已迁移出 C 盘。</p>
                <p className="font-mono text-emerald-300 break-all bg-black/30 rounded p-1.5 border border-white/5 mt-1">{cache.real_target}</p>
              </div>
            )}
          </div>
        </div>
      </div>
    );
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between flex-shrink-0">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">开发环境管理</h2>
          <p className="text-xs text-slate-400 mt-1">
            以纯净模式接管和管理系统的 SDK，支持配置代理镜像源、移动全局缓存与全局依赖版本更新。
          </p>
        </div>

        <button 
          onClick={fetchSdks}
          disabled={loadingList}
          className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 transition-all cursor-pointer"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loadingList ? "animate-spin" : ""}`} />
          刷新列表
        </button>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 flex-1 min-h-0">
        {/* Left: SDKs list */}
        <div className="lg:col-span-4 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col h-[520px]">
          <div className="p-4 bg-white/3 border-b border-white/5 flex items-center justify-between">
            <span className="text-xs font-semibold text-slate-300">支持的开发环境 / 库</span>
          </div>

          <div className="flex-1 overflow-y-auto divide-y divide-white/5">
            {sdks.map((sdk) => {
              const isSelected = selectedSdk?.name === sdk.name;
              const isManagedItem = config?.managed_items?.includes(sdk.name) || false;
              return (
                <div
                  key={sdk.name}
                  onClick={() => handleSelectSdk(sdk)}
                  className={`p-4 flex items-center justify-between hover:bg-white/2 cursor-pointer transition-all ${
                    isSelected ? "bg-blue-600/5 border-l-2 border-blue-500" : ""
                  }`}
                >
                  <div>
                    <div className="flex items-center gap-1.5">
                      <h4 className="font-semibold text-white text-xs capitalize">{sdk.display_name}</h4>
                      {isManagedItem && (
                        <span className="px-1.5 py-0.2 rounded text-[8px] bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-bold">
                          托管中
                        </span>
                      )}
                    </div>
                    <span className="text-[10px] text-slate-500">{categoryLabel(sdk.category)}</span>
                  </div>

                  <div className="text-right">
                    {sdk.active_version ? (
                      <span className="text-[10px] bg-blue-500/10 text-blue-400 border border-blue-500/20 px-2 py-0.5 rounded-md font-medium font-mono">
                        {sdk.active_version}
                      </span>
                    ) : (
                      <span className="text-[10px] text-slate-500">未启用</span>
                    )}
                    <p className="text-[9px] text-slate-400 mt-1">{sdk.installed_versions.length} 个已安装</p>
                  </div>
                </div>
              );
            })}
          </div>
        </div>

        {/* Right: Selected SDK management */}
        <div className="lg:col-span-8 flex flex-col h-[520px]">
          {selectedSdk ? (
            <div className="flex-1 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col">
              {/* SDK Header */}
              <div className="p-5 border-b border-white/5 flex items-center justify-between bg-white/2 flex-shrink-0">
                <div>
                  <h3 className="text-base font-semibold text-white capitalize">{selectedSdk.display_name}</h3>
                  <div className="flex items-center gap-2 mt-0.5">
                    <span className="text-[10px] text-slate-400 uppercase">当前管理控制台</span>
                    {selectedSdk.official_website && (
                      <>
                        <span className="text-slate-600 text-[10px]">•</span>
                        <button
                          onClick={() => openUrl(selectedSdk.official_website)}
                          className="text-[10px] text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 cursor-pointer inline-flex"
                        >
                          官方网站
                          <ExternalLink className="w-2.5 h-2.5" />
                        </button>
                      </>
                    )}
                  </div>
                </div>

                <div className="flex items-center gap-1.5">
                  <span className="text-[11px] text-slate-400">已启用版本:</span>
                  <span className="text-xs font-mono font-bold text-blue-400 bg-blue-500/5 px-2 py-1 border border-blue-500/15 rounded-md">
                    {selectedSdk.active_version || "无"}
                  </span>
                </div>
              </div>

              {/* Sub Tabs Switcher */}
              {isManaged && (
                <div className="flex bg-white/5 border border-white/5 rounded-xl p-0.5 mx-5 mt-4 flex-shrink-0">
                <button
                  onClick={() => setActiveSubTab("control")}
                  className={`flex-1 py-1.5 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
                    activeSubTab === "control" ? "bg-blue-600 text-white shadow-md" : "text-slate-400 hover:text-slate-200"
                  }`}
                >
                  托管与版本管理
                </button>
                {selectedSdk.has_mirror && (
                  <button
                    onClick={() => setActiveSubTab("mirrors")}
                    className={`flex-1 py-1.5 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
                      activeSubTab === "mirrors" ? "bg-blue-600 text-white shadow-md" : "text-slate-400 hover:text-slate-200"
                    }`}
                  >
                    国内镜像配置
                  </button>
                )}
                {selectedSdk.has_pkg && (
                  <button
                    onClick={() => setActiveSubTab("packages")}
                    className={`flex-1 py-1.5 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
                      activeSubTab === "packages" ? "bg-blue-600 text-white shadow-md" : "text-slate-400 hover:text-slate-200"
                    }`}
                  >
                    全局包管理
                  </button>
                )}
                {selectedSdk.has_cache && (
                  <button
                    onClick={() => setActiveSubTab("cache")}
                    className={`flex-1 py-1.5 rounded-lg text-[10px] font-semibold transition-all cursor-pointer ${
                      activeSubTab === "cache" ? "bg-blue-600 text-white shadow-md" : "text-slate-400 hover:text-slate-200"
                    }`}
                  >
                    缓存与文件管理
                  </button>
                )}
              </div>
              )}

              {/* Operations Pane Content */}
              <div className="flex-1 overflow-y-auto p-5">
                {activeSubTab === "control" && renderControlTab()}
                {activeSubTab === "mirrors" && renderMirrorsTab()}
                {activeSubTab === "packages" && renderPackagesTab()}
                {activeSubTab === "cache" && renderCacheTab()}
              </div>
            </div>
          ) : (
            <div className="flex-1 glass-panel rounded-2xl border border-white/5 flex flex-col items-center justify-center text-center text-slate-500 p-8">
              <HelpCircle className="w-12 h-12 text-slate-600 mb-4" />
              <p className="text-xs font-medium text-slate-400 font-sans">请在左侧列表中选择一个开发库/语言/服务进行管理</p>
            </div>
         )}
        </div>
      </div>
    </div>
  );
}
