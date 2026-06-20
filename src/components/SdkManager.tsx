import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  FileCheck
} from "lucide-react";

interface SdkInfo {
  name: string;
  category: string;
  active_version: string;
  installed_versions: string[];
}

interface SdkInfoTs {
  name: string;
  category: string;
  active_version: string;
  installed_versions: string[];
}

interface ProgressPayload {
  sdk: string;
  downloaded: number;
  total: number;
  pct: number;
}

function categoryLabel(cat: string): string {
  switch (cat) {
    case "language": return "编程语言";
    case "service":  return "本地服务";
    case "build_tool": return "构建工具";
    case "mobile":   return "移动端 SDK";
    default: return cat;
  }
}

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

  // 远程下载透明化：选中某个版本后，展示将要访问的远程地址
  const [pickedVersion, setPickedVersion] = useState("");
  const [downloadInfo, setDownloadInfo] = useState<{ url: string; host: string; file_ext: string } | null>(null);
  const [downloadInfoErr, setDownloadInfoErr] = useState<string | null>(null);

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

  const fetchSdks = async () => {
    setLoadingList(true);
    try {
      const list = await invoke<SdkInfoTs[]>("get_sdks_list");
      setSdks(list);
      // Update selected SDK if any
      if (selectedSdk) {
        const updated = list.find(s => s.name === selectedSdk.name);
        if (updated) setSelectedSdk(updated);
      }
    } catch (e) {
      console.error(e);
    } finally {
      setLoadingList(false);
    }
  };

  useEffect(() => {
    fetchSdks();
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
    try {
      const versions = await invoke<string[]>("list_remote_versions", { sdkName: sdk.name });
      setRemoteVersions(versions);
    } catch (e) {
      console.error(e);
    } finally {
      setLoadingRemote(false);
    }
  };

  const handleInstall = async (version: string) => {
    if (!selectedSdk) return;
    setInstallingVersion(version);
    setDownloadProgress(null);
    try {
      await invoke("install_sdk_version", { 
        sdkName: selectedSdk.name, 
        version: version.split(" ")[0] // Strip any labels like LTS
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
    if (!confirm(`确定卸载 ${selectedSdk.name} v${version} 吗？`)) return;
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
      alert(`切换失败: ${e}`);
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

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">SDK 版本管理</h2>
          <p className="text-xs text-slate-400 mt-1">下载、安装、切换任意版本的开发语言与工具。点击版本后会先展示「从哪里下载」，确认后才开始下载。</p>
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
        <div className="lg:col-span-5 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col h-[520px]">
          <div className="p-4 bg-white/3 border-b border-white/5">
            <span className="text-xs font-semibold text-slate-300">支持的开发环境 / 库</span>
          </div>

          <div className="flex-1 overflow-y-auto divide-y divide-white/5">
            {sdks.map((sdk) => {
              const isSelected = selectedSdk?.name === sdk.name;
              return (
                <div
                  key={sdk.name}
                  onClick={() => handleSelectSdk(sdk)}
                  className={`p-4 flex items-center justify-between hover:bg-white/2 cursor-pointer transition-all ${
                    isSelected ? "bg-blue-600/5 border-l-2 border-blue-500" : ""
                  }`}
                >
                  <div>
                    <h4 className="font-semibold text-white text-xs capitalize">{sdk.name}</h4>
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
        <div className="lg:col-span-7 flex flex-col h-[520px]">
          {selectedSdk ? (
            <div className="flex-1 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col">
              {/* SDK Header */}
              <div className="p-5 border-b border-white/5 flex items-center justify-between bg-white/2">
                <div>
                  <h3 className="text-base font-semibold text-white capitalize">{selectedSdk.name}</h3>
                  <span className="text-[10px] text-slate-400 uppercase">当前管理控制台</span>
                </div>

                <div className="flex items-center gap-1">
                  <span className="text-[11px] text-slate-400">已启用版本:</span>
                  <span className="text-xs font-mono font-bold text-blue-400">
                    {selectedSdk.active_version || "无"}
                  </span>
                </div>
              </div>

              {/* Operations Tabs */}
              <div className="flex-1 overflow-y-auto p-5 space-y-6">
                {/* 移动端 SDK 的新手说明 */}
                {selectedSdk.category === "mobile" && (
                  <div className="p-3.5 rounded-xl bg-indigo-500/5 border border-indigo-500/15 space-y-1.5">
                    <span className="text-[11px] font-semibold text-indigo-300">
                      {selectedSdk.name === "android" ? "关于 Android SDK" : "关于鸿蒙 HarmonyOS SDK"}
                    </span>
                    {selectedSdk.name === "android" ? (
                      <p className="text-[10px] text-slate-300 leading-relaxed">
                        这里下载的是 Google 官方「命令行工具(commandline-tools)」，列表里的数字是官方构建号(build number)。
                        安装后会自动把 <span className="font-mono text-indigo-300">ANDROID_HOME</span> 和
                        <span className="font-mono text-indigo-300"> ANDROID_SDK_ROOT</span> 指向稳定的链接目录，切换版本无需再改环境变量。
                      </p>
                    ) : (
                      <p className="text-[10px] text-slate-300 leading-relaxed">
                        鸿蒙命令行工具需要在华为开发者官网登录后下载（无免登录直链）。请先到
                        <span className="font-mono text-indigo-300"> developer.huawei.com/consumer/cn/download/ </span>
                        下载并解压，然后用下方的「手动注册本地已存在 SDK」填入版本号和解压目录即可。注册后会自动配置
                        <span className="font-mono text-indigo-300"> OHOS_SDK_HOME</span>。
                      </p>
                    )}
                  </div>
                )}

                {/* Installed versions */}
                <div className="space-y-3.5">
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
                                ? "bg-blue-600/10 border-blue-500/30 text-white" 
                                : "bg-black/20 border-white/5 text-slate-300"
                            }`}
                          >
                            <span className="font-mono text-xs font-medium">{v}</span>
                            <div className="flex items-center gap-1.5">
                              {!isActive && (
                                <button
                                  onClick={() => handleUse(v)}
                                  className="p-1.5 hover:bg-white/10 rounded-lg text-slate-400 hover:text-slate-200 text-[10px] cursor-pointer transition-all flex items-center gap-0.5"
                                >
                                  <Check className="w-3.5 h-3.5" />
                                  启用
                                </button>
                              )}
                              <button
                                onClick={() => handleUninstall(v)}
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
                <div className="space-y-3.5">
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
                          id="remote-versions-select"
                          className="flex-1 glass-input px-3.5 py-2 text-xs"
                          value={pickedVersion}
                          onChange={(e) => {
                            const v = e.target.value;
                            setPickedVersion(v);
                            if (v) fetchDownloadInfo(v.split(" ")[0]);
                          }}
                        >
                          <option value="" disabled>选择可在线安装的远程版本...</option>
                          {remoteVersions.map((v) => (
                            <option key={v} value={v}>{v}</option>
                          ))}
                        </select>

                        <button
                          onClick={() => {
                            if (pickedVersion) handleInstall(pickedVersion);
                          }}
                          disabled={installingVersion !== null || !pickedVersion}
                          className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-medium cursor-pointer transition-all flex items-center gap-1.5"
                        >
                          <Download className="w-3.5 h-3.5" />
                          下载安装
                        </button>
                      </div>

                      {/* 远程地址透明化：下载前先告诉用户将从哪里下载什么 */}
                      {downloadInfoErr && (
                        <div className="p-3 rounded-xl bg-amber-500/5 border border-amber-500/15 text-[10px] text-amber-300 leading-relaxed whitespace-pre-wrap">
                          {downloadInfoErr}
                        </div>
                      )}
                      {downloadInfo && (
                        <div className="p-3 rounded-xl bg-blue-500/5 border border-blue-500/15 space-y-1.5 animate-fadeIn">
                          <span className="text-[10px] font-semibold text-blue-400 uppercase tracking-wide">下载来源透明展示</span>
                          <div className="grid grid-cols-1 gap-1 text-[10px]">
                            <p className="text-slate-400">远程服务器: <span className="font-mono text-slate-200">{downloadInfo.host}</span></p>
                            <p className="text-slate-400">文件类型: <span className="font-mono text-slate-200">.{downloadInfo.file_ext}</span></p>
                            <p className="text-slate-400 break-all">完整下载地址:</p>
                            <p className="font-mono text-slate-300 break-all bg-black/20 rounded p-1.5 border border-white/5">{downloadInfo.url}</p>
                          </div>
                        </div>
                      )}

                      {/* Custom input for historic/ancient versions query */}
                      <div className="pt-3 border-t border-white/5 space-y-2">
                        <span className="text-[10px] text-slate-400 block">或者，手动输入版本号查询安装（支持古老或特定版本）：</span>
                        <div className="flex items-center gap-2">
                          <input
                            type="text"
                            placeholder="例如: 1.18.10 或 11.0.12 (Adoptium 等)"
                            value={customQueryVersion}
                            onChange={(e) => {
                              setCustomQueryVersion(e.target.value);
                              setQueryUrl(null);
                              setQueryErr(null);
                            }}
                            className="flex-1 glass-input px-3.5 py-2 text-xs font-mono"
                          />
                          <button
                            onClick={handleQueryCustomVersion}
                            disabled={!customQueryVersion || queryingCustom}
                            className="px-4 py-2 bg-white/5 border border-white/10 hover:bg-white/10 text-slate-300 rounded-lg text-xs font-medium cursor-pointer transition-all disabled:opacity-50"
                          >
                            {queryingCustom ? "查询中..." : "查询可用性"}
                          </button>
                        </div>

                        {queryErr && (
                          <p className="text-[10px] text-red-400 font-mono">{queryErr}</p>
                        )}

                        {queryUrl && (
                          <div className="flex items-center justify-between bg-emerald-500/5 border border-emerald-500/10 p-2.5 rounded-xl text-[10px] animate-fadeIn">
                            <div className="text-emerald-400 font-medium">
                              <p>找到可用版本！</p>
                              <p className="text-[9px] text-slate-500 font-mono mt-0.5 break-all max-w-[320px]">URL: {queryUrl}</p>
                            </div>
                            <button
                              onClick={() => handleInstall(customQueryVersion)}
                              disabled={installingVersion !== null}
                              className="px-3.5 py-1.5 bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg text-[10px] font-semibold cursor-pointer transition-all shadow-md shadow-emerald-500/10"
                            >
                              立即下载
                            </button>
                          </div>
                        )}
                      </div>
                    </div>
                  )}

                  {/* Installation / Download Progress bar */}
                  {installingVersion && (
                    <div className="p-4 bg-blue-900/10 border border-blue-500/20 rounded-xl space-y-2">
                      <div className="flex items-center justify-between text-xs">
                        <span className="text-slate-300 font-medium">正在下载安装: v{installingVersion}</span>
                        <span className="text-blue-400 font-bold font-mono">
                          {downloadProgress ? `${downloadProgress.pct}%` : "准备中..."}
                        </span>
                      </div>
                      
                      <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
                        <div 
                          className="h-full bg-blue-500 transition-all duration-300"
                          style={{ width: `${downloadProgress?.pct || 0}%` }}
                        ></div>
                      </div>

                      {downloadProgress && downloadProgress.total > 0 && (
                        <p className="text-[10px] text-slate-500 text-right">
                          进度: {(downloadProgress.downloaded / 1024 / 1024).toFixed(1)}MB / {(downloadProgress.total / 1024 / 1024).toFixed(1)}MB
                        </p>
                      )}
                    </div>
                  )}
                </div>

                {/* Local registration */}
                <div className="space-y-3.5 border-t border-white/5 pt-4">
                  <h4 className="text-xs font-semibold text-slate-300">手动注册本地已存在 SDK</h4>
                  <div className="grid grid-cols-3 gap-3">
                    <input 
                      type="text"
                      placeholder="版本名称 (e.g. 17.0.2)"
                      value={localVersion}
                      onChange={(e) => setLocalVersion(e.target.value)}
                      className="glass-input px-3.5 py-2 text-xs"
                    />
                    <input 
                      type="text"
                      placeholder="本地物理路径 (C:\Go)"
                      value={localPath}
                      onChange={(e) => setLocalPath(e.target.value)}
                      className="glass-input px-3.5 py-2 text-xs col-span-2"
                    />
                  </div>

                  <div className="flex items-center justify-between">
                    <div>
                      {registerErr && (
                        <span className="text-[10px] text-red-400 font-medium">{registerErr}</span>
                      )}
                    </div>
                    <button
                      onClick={handleRegisterLocal}
                      disabled={registering || !localVersion || !localPath}
                      className="px-5 py-2 bg-white/5 border border-white/10 hover:bg-white/10 disabled:opacity-50 text-slate-300 rounded-lg text-xs font-medium cursor-pointer transition-all flex items-center gap-1.5"
                    >
                      <Plus className="w-3.5 h-3.5" />
                      注册本地 SDK
                    </button>
                  </div>
                </div>
              </div>
            </div>
          ) : (
            <div className="flex-1 glass-panel rounded-2xl border border-white/5 flex flex-col items-center justify-center text-center text-slate-500 p-8">
              <HelpCircle className="w-12 h-12 text-slate-600 mb-4" />
              <p className="text-xs font-medium text-slate-400">请在左侧列表中选择一个开发库/服务进行管理</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
