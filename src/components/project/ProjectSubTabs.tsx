import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ExternalLink,
  CheckCircle,
  AlertTriangle,
  RefreshCw,
  Check,
  Trash2,
  Download,
  Globe,
  HardDrive,
  Activity,
  FolderOpen,
  Link,
  FolderSync,
  Package,
  Loader,
  Search,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import type { ProjectStatus, ProjectDef, EnvVarStatus, ServiceStatus, PackageManagerDef } from "./types";

// ── 共享的子标签页 Props ──
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
  /** 操作进行中，禁用按钮 */
  isOperating?: boolean;
  /** 当前活跃标签页 */
  activeSubTab?: string;
  /** 通知父组件当前切换到的标签页（用于懒加载） */
  onActiveSubTabChange?: (tab: string) => void;
}

// ═══════════════════════════════════════
//  版本管理
// ═══════════════════════════════════════
export function VersionsTab({
  project, remoteVersions, loadingRemote, installingVersion,
  onInstall, onUninstall, onUse,
  downloadProgress, installStep, onCancelInstall,
  versionsUpdatedAt, onRefreshRemoteVersions,
  isOperating, activeSubTab, onActiveSubTabChange,
}: SubTabProps) {
  const currentVersionNumber = installingVersion
    ? (installingVersion.includes(" · ") ? installingVersion.split(" · ")[1] : installingVersion).trim().split(" ")[0]
    : "";

  // 格式化上次更新时间
  const formatUpdatedAt = (ts: number | null | undefined): string => {
    if (!ts) return "";
    const diff = Math.floor(Date.now() / 1000) - ts;
    if (diff < 60) return "刚刚";
    if (diff < 3600) return `${Math.floor(diff / 60)}分钟前`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}小时前`;
    const d = new Date(ts * 1000);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };

  return (
    <div className="space-y-6">
      {/* 安装进度面板 */}
      {installingVersion && (
        <div className="glass-panel rounded-2xl p-5 border border-blue-500/20 bg-blue-600/5 space-y-4 animate-fadeIn">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Loader className="w-4 h-4 text-blue-400 animate-spin" />
              <h4 className="text-xs font-semibold text-blue-300">
                正在安装 {project.display_name} v{currentVersionNumber}
              </h4>
            </div>
            {onCancelInstall && (
              <button
                onClick={onCancelInstall}
                className="flex items-center gap-1 px-2.5 py-1 rounded-lg bg-red-500/10 hover:bg-red-500/20 text-red-400 hover:text-red-300 text-[11px] font-semibold border border-red-500/20 cursor-pointer transition-all"
                title="取消安装"
              >
                <X className="w-3 h-3" /> 取消安装
              </button>
            )}
          </div>

          {/* 步骤指示器 */}
          <div className="flex items-center gap-1">
            {["下载中", "解压中", "配置中", "完成"].map((step, idx) => {
              const steps = ["下载中", "解压中", "配置中", "完成"];
              const currentIdx = steps.indexOf(installStep);
              const isActive = step === installStep;
              const isCompleted = currentIdx > idx;
              return (
                <React.Fragment key={step}>
                  {idx > 0 && (
                    <div className={`flex-1 h-0.5 rounded-full ${isCompleted ? "bg-emerald-500" : isActive ? "bg-blue-500" : "bg-white/10"}`} />
                  )}
                  <div className="flex items-center gap-1.5">
                    <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[11px] font-bold border ${isCompleted
                      ? "bg-emerald-500 text-white border-emerald-500"
                      : isActive
                        ? "bg-blue-600 text-white border-blue-500 animate-pulse"
                        : "bg-white/5 text-slate-500 border-white/10"
                      }`}>
                      {isCompleted ? <Check className="w-3 h-3" /> : idx + 1}
                    </div>
                    <span className={`text-[13px] font-medium ${isActive ? "text-blue-300" : isCompleted ? "text-emerald-400" : "text-slate-500"}`}>
                      {step}
                    </span>
                  </div>
                </React.Fragment>
              );
            })}
          </div>

          {/* 下载进度条 */}
          {downloadProgress && installStep === "下载中" && (
            <div className="space-y-2">
              <div className="flex items-center justify-between text-[13px]">
                <span className="text-slate-400">下载进度 ({currentVersionNumber})</span>
                <div className="flex items-center gap-3">
                  {downloadProgress.speed_str && (
                    <span className="text-cyan-400 font-mono font-semibold text-[11px]">
                      ↓ {downloadProgress.speed_str}
                    </span>
                  )}
                  <span className="text-blue-300 font-mono font-semibold">{downloadProgress.pct}%</span>
                </div>
              </div>
              <div className="w-full h-2 bg-white/5 rounded-full overflow-hidden">
                <div
                  className="h-full bg-gradient-to-r from-blue-600 to-blue-400 rounded-full transition-all duration-300"
                  style={{ width: `${downloadProgress.pct}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[13px] text-slate-500">
                <span>{(downloadProgress.downloaded / 1024 / 1024).toFixed(1)} MB</span>
                <span>{(downloadProgress.total / 1024 / 1024).toFixed(1)} MB</span>
              </div>
            </div>
          )}

          {/* 当前步骤文字说明 */}
          <p className="text-[13px] text-slate-400">
            {installStep === "下载中" && `正在从远程服务器下载安装包 (v${currentVersionNumber})，请稍候...`}
            {installStep === "解压中" && `下载完成，正在解压安装文件 (v${currentVersionNumber})...`}
            {installStep === "配置中" && `解压完成，正在配置环境变量和创建 Junction 链接 (v${currentVersionNumber})...`}
            {installStep === "完成" && `v${currentVersionNumber} 安装成功！`}
          </p>
        </div>
      )}

      {/* 已安装版本 */}
      <div className="space-y-3">
        <div>
          <h4 className="text-xs font-semibold text-slate-300">本地已安装版本</h4>
          <p className="text-[13px] text-slate-500 mt-0.5">已下载到本机的版本，点击「启用」可切换当前使用的版本。</p>
        </div>
        {!project.installed_versions || project.installed_versions.length === 0 ? (
          <p className="text-[11px] text-slate-500">尚未安装任何版本。请从下方远程版本列表安装。</p>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            {project.installed_versions.map((v) => {
              const isActive = project.active_version === v;
              return (
                <div
                  key={v}
                  className={`p-3 rounded-xl border flex items-center justify-between transition-all ${isActive
                    ? "bg-blue-600/10 border-blue-500/30 text-white shadow-md shadow-blue-500/5"
                    : "bg-black/20 border-white/5 text-slate-300"
                    }`}
                >
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs font-medium">{v}</span>
                    {isActive && (
                      <span className="px-1.5 py-0.5 rounded text-[11px] bg-blue-600 text-white font-bold">当前</span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5">
                    {!isActive && (
                      <button
                        onClick={() => onUse(v)}
                        disabled={isOperating}
                        className="p-1.5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg text-slate-400 hover:text-slate-200 text-[13px] cursor-pointer transition-all flex items-center gap-0.5"
                      >
                        <Check className="w-3.5 h-3.5" /> 启用
                      </button>
                    )}
                    <button
                      onClick={() => onUninstall(v)}
                      disabled={isOperating}
                      className="p-1.5 hover:bg-red-500/10 hover:text-red-400 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg text-slate-500 cursor-pointer transition-all"
                      title="卸载此版本"
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

      {/* 远程版本安装 */}
      <RemoteVersionSelector
        remoteVersions={remoteVersions}
        loadingRemote={loadingRemote}
        installingVersion={installingVersion}
        isOperating={isOperating}
        onInstall={onInstall}
        versionsUpdatedAt={versionsUpdatedAt}
        onRefresh={onRefreshRemoteVersions}
      />


    </div>
  );
}

// ═══════════════════════════════════════
//  环境变量
// ═══════════════════════════════════════
export function EnvVarsTab({ project, def, activeSubTab, onActiveSubTabChange }: SubTabProps) {
  const vars: EnvVarStatus[] = project.env_vars_status ?? [];

  // 高级模式：显示用户可配置的运行时环境变量
  const [advanced, setAdvanced] = useState(false);
  const [userVars, setUserVars] = useState<Array<{
    name: string; desc: string; placeholder?: string; options?: string[];
    var_type?: string; current_value?: string; source?: string;
  }>>([]);
  const [loadingUserVars, setLoadingUserVars] = useState(false);
  const [editingVar, setEditingVar] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const [savingVar, setSavingVar] = useState<string | null>(null);

  const loadUserVars = async () => {
    if (!project.id) return;
    setLoadingUserVars(true);
    try {
      const list = await invoke<Array<{
        name: string; desc: string; placeholder?: string; options?: string[];
        var_type?: string; current_value?: string; source?: string;
      }>>("get_user_configurable_vars", { projectId: project.id });
      setUserVars(list);
    } catch { /* ignore */ } finally {
      setLoadingUserVars(false);
    }
  };

  // 当切换到 envvars 标签页时通知父组件
  useEffect(() => {
    onActiveSubTabChange?.("envvars");
  }, []);

  useEffect(() => {
    if (advanced && userVars.length === 0) {
      loadUserVars();
    }
  }, [advanced]);

  const handleSetVar = async (name: string, value: string) => {
    setSavingVar(name);
    try {
      if (value.trim()) {
        await invoke("set_user_configurable_var", { name, value: value.trim() });
      } else {
        await invoke("delete_user_configurable_var", { name });
      }
      setEditingVar(null);
      await loadUserVars();
    } catch (e: unknown) {
      alert(`设置 ${name} 失败: ${e}`);
    } finally {
      setSavingVar(null);
    }
  };



  return (
    <div className="space-y-5">
      {/* 路径类环境变量（系统管理，不可修改） */}
      <div className="space-y-3">
        <div>
          <span className="text-xs font-semibold text-slate-300">项目关联环境变量</span>
          <span className="text-[13px] text-slate-500 ml-1.5">{vars.length} 个变量</span>
          <p className="text-[13px] text-slate-500 mt-0.5">路径类环境变量由 AnyVersion 自动管理，不可手动修改。</p>
        </div>
        {vars.length === 0 ? (
          <p className="text-[11px] text-slate-500">该项目无需配置路径类环境变量。</p>
        ) : (
          <div className="border border-white/5 rounded-xl overflow-hidden overflow-x-auto">
            <table className="w-full text-left border-collapse text-[13px] min-w-[450px]">
              <thead>
                <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-medium">
                  <th className="p-2.5 w-32">变量名</th>
                  <th className="p-2.5 w-16">Tier</th>
                  <th className="p-2.5 w-36">说明</th>
                  <th className="p-2.5">当前配置值</th>
                  <th className="p-2.5 w-28 whitespace-nowrap">来源</th>
                  <th className="p-2.5 w-14">状态</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/5 text-slate-300">
                {vars.map((v) => (
                  <tr key={v.name} className="hover:bg-white/1 font-mono">
                    <td className="p-2.5 font-semibold text-slate-200">{v.name}</td>
                    <td className="p-2.5">
                      {v.tier === "core" ? (
                        <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[12px] font-semibold">Core</span>
                      ) : v.tier === "package" ? (
                        <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[12px] font-semibold">Package</span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-[12px]">-</span>
                      )}
                    </td>
                    <td className="p-2.5 text-slate-400 font-sans">{v.desc}</td>
                    <td className="p-2.5 break-all select-text">
                      {v.value || <span className="text-slate-600 font-sans">未配置</span>}
                    </td>
                    <td className="p-2.5">
                      {v.source === "HKCU" ? (
                        <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[12px] font-semibold">用户级</span>
                      ) : v.source === "HKLM" ? (
                        <span className="px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-400 border border-indigo-500/20 text-[12px] font-semibold">系统级</span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-[12px]">未设置</span>
                      )}
                    </td>
                    <td className="p-2.5">
                      {v.exists ? (
                        <CheckCircle className="w-3.5 h-3.5 text-emerald-400" />
                      ) : (
                        <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>




      {/* 高级模式：运行时环境变量（用户可配置） */}
      <div className="border-t border-white/5 pt-4">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <button
              onClick={() => setAdvanced(!advanced)}
              className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors cursor-pointer ${advanced ? "bg-purple-600" : "bg-white/10"}`}
            >
              <span className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${advanced ? "translate-x-[18px]" : "translate-x-[3px]"}`} />
            </button>
            <span className="text-xs font-semibold text-slate-300">高级模式 - 运行时参数</span>
            {advanced && (
              <span className="px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400 border border-purple-500/20 text-[11px] font-semibold">高级</span>
            )}
          </div>
          {advanced && (
            <button onClick={loadUserVars} disabled={loadingUserVars} className="flex items-center gap-1 px-2 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[13px] border border-white/5 cursor-pointer">
              <RefreshCw className={`w-3 h-3 ${loadingUserVars ? "animate-spin" : ""}`} />刷新
            </button>
          )}
        </div>

        {!advanced ? (
          <p className="text-[13px] text-slate-500">开启后可设置 {def?.display_name || "项目"} 的运行时环境变量（如 NODE_OPTIONS、DEBUG 等），适用于高级用户。</p>
        ) : loadingUserVars ? (
          <div className="flex items-center gap-2 text-[13px] text-slate-400 py-4"><Loader className="w-3 h-3 animate-spin text-blue-400" />加载中...</div>
        ) : userVars.length === 0 ? (
          <p className="text-[13px] text-slate-500">该项目没有可配置的运行时环境变量。</p>
        ) : (
          <div className="border border-white/5 rounded-xl overflow-hidden overflow-x-auto">
            <table className="w-full text-left border-collapse text-[13px]">
              <thead>
                <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-medium">
                  <th className="p-2.5 w-40">变量名</th>
                  <th className="p-2.5">说明</th>
                  <th className="p-2.5">当前值</th>
                  <th className="p-2.5 w-24 whitespace-nowrap">来源</th>
                  <th className="p-2.5 w-28 text-center">操作</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/5 text-slate-300">
                {userVars.map((v) => {
                  const isEditing = editingVar === v.name;
                  const hasValue = v.current_value && v.current_value !== "null";
                  return (
                    <tr key={v.name} className="hover:bg-white/1">
                      <td className="p-2.5 font-mono font-semibold text-slate-200">{v.name}</td>
                      <td className="p-2.5 text-slate-500 font-sans">{v.desc}</td>
                      <td className="p-2.5">
                        {isEditing ? (
                          v.options ? (
                            <select value={editValue} onChange={(e) => setEditValue(e.target.value)} className="glass-input px-2 py-1 text-[13px] font-mono rounded w-full">
                              <option value="">(未设置)</option>
                              {v.options.map(o => <option key={o} value={o}>{o}</option>)}
                            </select>
                          ) : v.var_type === "boolean" ? (
                            <select value={editValue} onChange={(e) => setEditValue(e.target.value)} className="glass-input px-2 py-1 text-[13px] font-mono rounded w-full">
                              <option value="">(未设置)</option>
                              <option value="1">1 (启用)</option>
                              <option value="0">0 (禁用)</option>
                            </select>
                          ) : (
                            <input type="text" value={editValue} onChange={(e) => setEditValue(e.target.value)} className="glass-input px-2 py-1 text-[13px] font-mono rounded w-full" placeholder={v.placeholder} />
                          )
                        ) : (
                          hasValue ? (
                            <span className="font-mono text-slate-200 break-all">{v.current_value}</span>
                          ) : (
                            <span className="text-slate-600 font-sans">未设置</span>
                          )
                        )}
                      </td>
                      <td className="p-2.5 text-slate-500">
                        {v.source || "-"}
                      </td>
                      <td className="p-2.5 text-center">
                        {isEditing ? (
                          <div className="flex items-center gap-1 justify-center">
                            <button onClick={() => handleSetVar(v.name, editValue)} disabled={savingVar === v.name} className="px-2 py-0.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded text-[11px] font-semibold cursor-pointer">
                              {savingVar === v.name ? "保存中" : "保存"}
                            </button>
                            <button onClick={() => setEditingVar(null)} className="px-2 py-0.5 bg-white/5 hover:bg-white/10 text-slate-400 rounded text-[11px] cursor-pointer">取消</button>
                          </div>
                        ) : (
                          <button onClick={() => { setEditingVar(v.name); setEditValue(v.current_value || ""); }} className="px-2 py-0.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[11px] border border-white/5 cursor-pointer">
                            {hasValue ? "修改" : "设置"}
                          </button>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  服务管理
// ═══════════════════════════════════════
export function ServicesTab({ project, def, serviceCtrlLoading, onServiceToggle, activeSubTab, onActiveSubTabChange }: SubTabProps) {
  // 当切换到 services 标签页时通知父组件
  useEffect(() => {
    onActiveSubTabChange?.("services");
  }, []);

  const svc: ServiceStatus | null = project.service_status ?? null;
  if (!svc) {
    return (
      <div className="p-8 text-center text-slate-500">
        <Activity className="w-10 h-10 mx-auto text-slate-600 mb-3" />
        <p className="text-xs font-medium text-slate-400">未检测到服务信息</p>
        <p className="text-[13px] text-slate-500 mt-1">该项目暂无可管理的本地服务。</p>
      </div>
    );
  }


  return (
    <div className="space-y-4">
      <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
        <div className="flex items-center gap-2 border-b border-white/5 pb-3">
          <Activity className="w-4 h-4 text-blue-400" />
          <h4 className="text-xs font-semibold text-white">本地服务控制台</h4>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-xs">
          <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-1.5">
            <span className="text-[13px] text-slate-400 font-semibold uppercase tracking-wider block">当前状态</span>
            <div className="flex items-center gap-2">
              {svc.running ? (
                <span className="px-2.5 py-1 rounded-lg bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold flex items-center gap-1 animate-fadeIn">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-ping" />
                  运行中 {svc.pid ? `(PID: ${svc.pid})` : ""}
                </span>
              ) : (
                <span className="px-2.5 py-1 rounded-lg bg-slate-500/10 text-slate-400 border border-white/5 font-semibold">已停止</span>
              )}
            </div>
          </div>

          <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-1">
            <span className="text-[13px] text-slate-400 font-semibold uppercase tracking-wider block">运行参数</span>
            <div className="text-slate-300 font-mono space-y-0.5">
              <p>端口: {svc.port || def?.default_port || "无"}</p>
              <p>版本: {project.active_version || "未启用"}</p>
            </div>
          </div>

          <div className="p-3 bg-black/20 rounded-xl border border-white/5 flex items-center justify-center gap-2">
            <button
              onClick={onServiceToggle}
              disabled={serviceCtrlLoading}
              className={`px-4 py-2 ${svc.running ? "bg-red-600 hover:bg-red-500" : "bg-emerald-600 hover:bg-emerald-500"} disabled:opacity-50 text-white font-semibold rounded-xl text-xs cursor-pointer shadow-md transition-all flex items-center gap-1`}
            >
              {serviceCtrlLoading ? "操作中..." : svc.running ? "停止服务" : "启动服务"}
            </button>
          </div>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs pt-2">
          {svc.data_dir && (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5">
              <div className="min-w-0 flex-1">
                <span className="text-[13px] text-slate-400 font-semibold block">数据目录</span>
                <p className="font-mono text-slate-300 truncate mt-1">{svc.data_dir}</p>
              </div>
            </div>
          )}
          {svc.log_dir && (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5">
              <div className="min-w-0 flex-1">
                <span className="text-[13px] text-slate-400 font-semibold block">日志目录</span>
                <p className="font-mono text-slate-300 truncate mt-1">{svc.log_dir}</p>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  远程版本选择器（可搜索过滤）
// ═══════════════════════════════════════
function RemoteVersionSelector({
  remoteVersions,
  loadingRemote,
  installingVersion,
  isOperating,
  onInstall,
  versionsUpdatedAt,
  onRefresh,
}: {
  remoteVersions: string[];
  loadingRemote: boolean;
  installingVersion: string | null;
  isOperating?: boolean;
  onInstall: (version: string) => void;
  versionsUpdatedAt?: number | null;
  onRefresh?: () => void;
}) {
  const [search, setSearch] = useState("");
  const [open, setOpen] = useState(false);
  const containerRef = React.useRef<HTMLDivElement>(null);

  const filtered = search.trim()
    ? remoteVersions.filter((v) => v.toLowerCase().includes(search.toLowerCase()))
    : remoteVersions;

  // 格式化上次更新时间
  const formatUpdatedAt = (ts: number | null | undefined): string => {
    if (!ts) return "";
    const diff = Math.floor(Date.now() / 1000) - ts;
    if (diff < 60) return "刚刚";
    if (diff < 3600) return `${Math.floor(diff / 60)}分钟前`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}小时前`;
    const d = new Date(ts * 1000);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')} ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };

  // 点击外部关闭下拉
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const handleSelect = (v: string) => {
    setSearch(v);
    setOpen(false);
  };

  const handleInstall = () => {
    if (search.trim() && remoteVersions.includes(search.trim())) {
      onInstall(search.trim());
      setSearch("");
    }
  };

  return (
    <div className="space-y-3 border-t border-white/5 pt-4">
      {/* 标题行：含上次更新时间和刷新按钮 */}
      <div className="flex items-center justify-between">
        <div>
          <h4 className="text-xs font-semibold text-slate-300">在线安装远程版本</h4>
          <p className="text-[13px] text-slate-500 mt-0.5">输入关键词过滤版本，从官方服务器下载并安装新版本。</p>
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          {versionsUpdatedAt && !loadingRemote && (
            <span className="text-[11px] text-slate-600">
              上次更新：{formatUpdatedAt(versionsUpdatedAt)}
            </span>
          )}
          {onRefresh && (
            <button
              onClick={onRefresh}
              disabled={loadingRemote || !!installingVersion}
              title="刷新版本列表"
              className="flex items-center gap-1 px-2.5 py-1 bg-white/5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed text-slate-300 rounded-lg text-[11px] border border-white/8 cursor-pointer transition-all"
            >
              <RefreshCw className={`w-3 h-3 ${loadingRemote ? "animate-spin text-blue-400" : ""}`} />
              {loadingRemote ? "更新中..." : "更新列表"}
            </button>
          )}
        </div>
      </div>

      {loadingRemote && remoteVersions.length === 0 ? (
        <div className="flex items-center gap-2 text-slate-400 text-xs py-2">
          <RefreshCw className="w-4 h-4 animate-spin text-blue-400" />
          正在获取远程版本列表...
        </div>
      ) : (
        <div className="space-y-2">
          <div ref={containerRef} className="relative">
            <div className="flex items-center gap-3">
              <div className="relative flex-1">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500 pointer-events-none" />
                <input
                  type="text"
                  value={search}
                  onChange={(e) => { setSearch(e.target.value); setOpen(true); }}
                  onFocus={() => setOpen(true)}
                  placeholder="输入关键词过滤版本，例如 18、LTS..."
                  className="w-full glass-input pl-9 pr-9 py-2 text-xs"
                />
                {search && (
                  <button
                    onClick={() => { setSearch(""); setOpen(true); }}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300 cursor-pointer"
                  >
                    <span className="text-xs">×</span>
                  </button>
                )}
              </div>
              <button
                onClick={handleInstall}
                disabled={installingVersion !== null || isOperating || !search.trim() || !remoteVersions.includes(search.trim())}
                className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-md shadow-blue-500/10 cursor-pointer transition-all flex items-center gap-1.5"
              >
                <Download className="w-3.5 h-3.5" />
                {installingVersion ? "正在安装..." : "一键安装"}
              </button>
            </div>

            {/* 下拉列表 */}
            {open && filtered.length > 0 && (
              <div className="absolute z-50 mt-1 w-full max-h-48 overflow-y-auto glass-panel rounded-xl border border-white/10 bg-[#1a1f2e] shadow-2xl">
                {filtered.map((v) => (
                  <button
                    key={v}
                    onClick={() => handleSelect(v)}
                    className={`w-full text-left px-3 py-1.5 text-xs hover:bg-blue-600/20 transition-colors cursor-pointer ${search.trim() === v ? "bg-blue-600/10 text-blue-300" : "text-slate-300"
                      }`}
                  >
                    {v}
                  </button>
                ))}
              </div>
            )}

            {/* 无匹配提示 */}
            {open && search.trim() && filtered.length === 0 && (
              <div className="absolute z-50 mt-1 w-full glass-panel rounded-xl border border-white/10 bg-[#1a1f2e] shadow-2xl p-3 text-center">
                <p className="text-[13px] text-slate-500">未找到匹配 <span className="text-slate-300 font-mono">{search}</span> 的版本</p>
              </div>
            )}
          </div>

          {/* 版本统计 */}
          <p className="text-[12px] text-slate-600">
            共 {remoteVersions.length} 个远程版本{search.trim() && filtered.length !== remoteVersions.length ? `，匹配 ${filtered.length} 个` : ""}
          </p>
        </div>
      )}
    </div>
  );
}


// ═══════════════════════════════════════
//  旧版数据（托管前备份的安装信息）
// ═══════════════════════════════════════
export function LegacyTab({ projectId }: { projectId: string }) {
  const [data, setData] = useState<{
    install_source?: string;
    install_root?: string;
    version?: string;
    backed_env_vars: Record<string, string>;
    removed_path_entries: string[];
    timestamp: number;
  } | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    invoke<{
      project_id: string;
      install_source?: string;
      install_root?: string;
      version?: string;
      backed_env_vars: Record<string, string>;
      removed_path_entries: string[];
      timestamp: number;
    } | null>("get_legacy_backup", { id: projectId })
      .then((info) => setData(info))
      .catch(() => setData(null))
      .finally(() => setLoading(false));
  }, [projectId]);

  if (loading) {
    return (
      <div className="flex items-center justify-center gap-2 text-xs text-slate-400 py-8">
        <Loader className="w-4 h-4 animate-spin text-blue-400" /> 正在加载旧版数据...
      </div>
    );
  }

  if (!data) {
    return (
      <div className="p-8 text-center text-slate-500">
        <FolderOpen className="w-10 h-10 mx-auto text-slate-600 mb-3" />
        <p className="text-xs">暂无旧版安装数据备份</p>
        <p className="text-[13px] text-slate-500 mt-1">托管时会自动备份之前通过其他工具安装的版本信息。</p>
      </div>
    );
  }

  const envVarEntries = Object.entries(data.backed_env_vars || {});

  return (
    <div className="space-y-5">
      {/* 标题说明 */}
      <div className="glass-panel rounded-2xl p-4 border border-amber-500/10 bg-amber-500/3">
        <div className="flex items-start gap-3">
          <div className="w-9 h-9 rounded-xl flex items-center justify-center bg-amber-500/10 flex-shrink-0 mt-0.5">
            <AlertTriangle className="w-4.5 h-4.5 text-amber-400" />
          </div>
          <div>
            <h4 className="text-xs font-semibold text-amber-300">托管前旧版数据</h4>
            <p className="text-[13px] text-amber-400/60 mt-0.5">
              以下数据来自 AnyVersion 托管前的备份。取消托管时将从备份还原原始环境变量和 PATH 条目。
            </p>
          </div>
        </div>
      </div>

      {/* 旧版安装信息 */}
      {(data.install_source || data.install_root || data.version) && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Download className="w-4 h-4 text-slate-400" />
            <h4 className="text-xs font-semibold text-white">旧版安装信息</h4>
          </div>
          <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-[13px]">
            {data.version && (
              <div className="p-2.5 bg-black/20 rounded-xl border border-white/5">
                <span className="text-slate-500 block mb-0.5">版本号</span>
                <span className="font-mono text-slate-200 font-semibold">{data.version}</span>
              </div>
            )}
            {data.install_source && (
              <div className="p-2.5 bg-black/20 rounded-xl border border-white/5">
                <span className="text-slate-500 block mb-0.5">安装方式</span>
                <span className="font-mono text-slate-200">{data.install_source}</span>
              </div>
            )}
            {data.install_root && (
              <div className="p-2.5 bg-black/20 rounded-xl border border-white/5">
                <span className="text-slate-500 block mb-0.5">安装路径</span>
                <span className="font-mono text-slate-200 text-[12px] break-all">{data.install_root}</span>
              </div>
            )}
          </div>
        </div>
      )}

      {/* 备份的环境变量 */}
      {envVarEntries.length > 0 && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-blue-400" />
            <h4 className="text-xs font-semibold text-white">备份的环境变量</h4>
            <span className="text-[12px] text-slate-500">({envVarEntries.length} 个)</span>
          </div>
          <div className="w-full">
            <table className="w-full text-left text-[13px]">
              <thead><tr className="text-slate-500 border-b border-white/5"><th className="p-2 w-48">变量名</th><th className="p-2">原始值</th></tr></thead>
              <tbody className="divide-y divide-white/5">
                {envVarEntries.map(([name, val]) => (
                  <tr key={name} className="hover:bg-white/2 text-slate-300">
                    <td className="p-2 font-mono font-semibold">{name}</td>
                    <td className="p-2 font-mono text-[12px] break-all text-slate-400">{val || "(空)"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* 移除的 PATH 条目 */}
      {data.removed_path_entries.length > 0 && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Trash2 className="w-4 h-4 text-red-400" />
            <h4 className="text-xs font-semibold text-white">移除的 PATH 条目</h4>
            <span className="text-[12px] text-slate-500">({data.removed_path_entries.length} 条)</span>
          </div>
          <div className="w-full space-y-1">
            {data.removed_path_entries.map((entry, idx) => (
              <div key={idx} className="p-2 bg-black/20 rounded-lg border border-white/5 text-[12px] font-mono text-slate-400 break-all">
                {entry}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════
//  包管理器独立子页面
//  每个包管理器（npm/yarn/pnpm）都有自己的管理页，包含：
//  版本检测、缓存管理、镜像配置、代理设置、全局包管理
// ═══════════════════════════════════════
export function PackageManagerTab({ projectId, pm, hidden }: { projectId: string; pm: PackageManagerDef; hidden?: boolean }) {
  const [checking, setChecking] = useState(false);
  const [detectStep, setDetectStep] = useState("");
  const [installed, setInstalled] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installProgress, setInstallProgress] = useState(false);
  const [upgrading, setUpgrading] = useState(false);
  const [latestVersion, setLatestVersion] = useState<string | null>(null);

  // 缓存 & 数据存储管理
  type ParentLink = { parent_path: string; parent_target: string; child_rel: string };
  const [cacheInfo, setCacheInfo] = useState<{ path: string; size: string; is_link: boolean; real_target: string; parent_link: ParentLink | null; detect_source: string } | null>(null);
  const [dataInfo, setDataInfo] = useState<{ path: string; size: string; is_link: boolean; real_target: string; detect_source: string } | null>(null);
  // 清理
  const [cleaningCache, setCleaningCache] = useState(false);
  const [cleanProgress, setCleanProgress] = useState<{ stage: string; current: number; total: number; file_name: string } | null>(null);

  // ── 工作流（缓存/数据变更） ──
  // null=关闭, 'cache'=缓存变更, 'data'=数据迁移
  const [workflowType, setWorkflowType] = useState<"cache" | "data" | null>(null);
  // method / paths / confirm / executing / done
  const [workflowStep, setWorkflowStep] = useState<"method" | "paths" | "confirm" | "executing" | "done">("method");
  const [workflowMethod, setWorkflowMethod] = useState<"junction" | "point">("junction");
  // junction: linkPath=形式路径(链接所在), actualPath=实际路径(数据所在)
  const [workflowLinkPath, setWorkflowLinkPath] = useState("");
  const [workflowActualPath, setWorkflowActualPath] = useState("");
  // point: 指向路径
  const [workflowPointPath, setWorkflowPointPath] = useState("");
  // 旧文件处理方式: delete=删除, move=移动到新目录, keep=不做改动
  const [workflowFileAction, setWorkflowFileAction] = useState<"delete" | "move" | "keep">("keep");
  // 执行阶段
  const [workflowExecuting, setWorkflowExecuting] = useState(false);
  const [workflowProgress, setWorkflowProgress] = useState<{ stage: string; current: number; total: number; file_name: string } | null>(null);

  // 关闭工作流，重置所有状态
  const closeWorkflow = () => {
    setWorkflowType(null);
    setWorkflowStep("method");
    setWorkflowMethod("junction");
    setWorkflowLinkPath("");
    setWorkflowActualPath("");
    setWorkflowPointPath("");
    setWorkflowFileAction("keep");
    setWorkflowExecuting(false);
    setWorkflowProgress(null);
  };

  // 打开工作流
  const openWorkflow = (type: "cache" | "data") => {
    closeWorkflow();
    setWorkflowType(type);
    setWorkflowStep("method");
    // 预填默认值
    if (type === "cache" && cacheInfo) {
      setWorkflowLinkPath(cacheInfo.path);
      if (cacheInfo.real_target) {
        setWorkflowActualPath(cacheInfo.real_target);
      } else {
        const drive = cacheInfo.path.match(/^([A-Za-z]):\\/);
        if (drive && drive[1].toUpperCase() === "C") {
          setWorkflowActualPath(`D:\\any-version-caches\\${pm.id}`);
        }
      }
    }
    if (type === "data") {
      if (dataInfo) {
        setWorkflowLinkPath(dataInfo.path);
        if (dataInfo.real_target) {
          setWorkflowActualPath(dataInfo.real_target);
        } else {
          const drive = dataInfo.path.match(/^([A-Za-z]):\\/);
          if (drive && drive[1].toUpperCase() === "C") {
            setWorkflowActualPath(`D:\\any-version-data\\${pm.id}`);
          }
        }
      } else {
        // 未检测到数据路径，预填一个建议目标路径，源路径留空由用户填写
        setWorkflowActualPath(`D:\\any-version-data\\${pm.id}`);
      }
    }
  };

  // 工作流下一步
  const workflowNext = () => {
    if (workflowStep === "method") {
      setWorkflowStep("paths");
    } else if (workflowStep === "paths") {
      setWorkflowStep("confirm");
    } else if (workflowStep === "confirm") {
      executeWorkflow();
    }
  };

  // 工作流上一步
  const workflowPrev = () => {
    if (workflowStep === "paths") {
      setWorkflowStep("method");
    } else if (workflowStep === "confirm") {
      setWorkflowStep("paths");
    }
  };

  // 浏览文件夹
  const browseWorkflowPath = async (setter: (v: string) => void) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择文件夹" });
      if (selected) setter(selected as string);
    } catch { alert("文件夹选择器不可用，请手动输入路径。"); }
  };

  // 执行工作流
  const executeWorkflow = async () => {
    // 检查是否向同一目录移动文件
    const pathsSame = workflowMethod === "junction"
      && workflowLinkPath.toLowerCase().replace(/[\\/]+$/, "")
      === workflowActualPath.toLowerCase().replace(/[\\/]+$/, "");

    if (workflowFileAction === "move" && pathsSame) {
      // 同目录，提示用户无需移动
      if (!confirm("源路径和目标路径相同，无需移动文件。将直接创建链接，继续？")) {
        return;
      }
    }

    setWorkflowStep("executing");
    setWorkflowExecuting(true);
    setWorkflowProgress(null);

    const unlisten = await listen<{ stage: string; current: number; total: number; file_name: string }>(
      "migrate-storage-progress",
      (event) => setWorkflowProgress(event.payload)
    );

    try {
      if (workflowMethod === "junction") {
        // Junction 模式：调用 migrate_pkg_storage
        // delete: deleteOldFirst=true, move: deleteOldFirst=false(copy then junction), keep: deleteOldFirst=false
        const deleteOldFirst = workflowFileAction === "delete";
        await invoke("migrate_pkg_storage", {
          projectId,
          pmId: pm.id,
          newPath: workflowActualPath,
          storageKind: workflowType as string,
          deleteOldFirst,
          origPath: workflowLinkPath || undefined,
        });
      } else {
        // Point 模式：修改配置（仅缓存支持）
        if (!pm.cache_set_cmd_template && !pm.cache_env_var) {
          throw new Error("该项目不支持配置指向");
        }
        // 处理旧文件
        const oldPath = workflowType === "cache" ? cacheInfo?.path : dataInfo?.path;
        if (oldPath && workflowFileAction !== "keep") {
          await invoke("handle_point_storage_files", {
            oldPath,
            newPath: workflowPointPath,
            action: workflowFileAction,
          });
        }
        await invoke("project_set_cache_path", {
          projectId,
          pmId: pm.id,
          newPath: workflowPointPath,
        });
      }
      await runDetection();
      setWorkflowStep("done");
    } catch (e: unknown) {
      alert(`操作失败: ${e}`);
      setWorkflowStep("confirm"); // 回到确认步骤
    } finally {
      unlisten();
      setWorkflowExecuting(false);
      setWorkflowProgress(null);
    }
  };

  // 镜像
  const [switchingMirror, setSwitchingMirror] = useState<string | null>(null);
  const [currentMirror, setCurrentMirror] = useState<string | null>(null);

  // 代理
  const [proxyDetected, setProxyDetected] = useState<string | null>(null);
  const [proxyInput, setProxyInput] = useState("");
  const [settingProxy, setSettingProxy] = useState(false);

  // 全局包
  const [packages, setPackages] = useState<Array<{ name: string; current_version: string; latest_version: string; status: string; homepage: string }>>([]);
  const [loadingPackages, setLoadingPackages] = useState(false);
  const [upgradingPkg, setUpgradingPkg] = useState<string | null>(null);

  // 首次检测
  const [hasChecked, setHasChecked] = useState(false);

  const runDetection = async () => {
    setChecking(true);
    const steps: Array<{ label: string; run: () => Promise<void> }> = [];
    setDetectStep(`正在检测 ${pm.display_name}...`);

    // Step 1: version
    steps.push({
      label: `正在检测 ${pm.display_name} 版本...`,
      run: async () => {
        if (pm.version_cmd) {
          try {
            const out = await invoke<string>("run_cmd_capture", { cmd: pm.version_cmd });
            setInstalled(true);
            setVersion(out.trim());
          } catch {
            setInstalled(false);
            setVersion(null);
          }
        }
      },
    });

    // Step 1b: 检测最新版本（仅在已安装时）
    if (pm.latest_version_cmd) {
      steps.push({
        label: `正在检查 ${pm.display_name} 最新版本...`,
        run: async () => {
          try {
            const out = await invoke<string>("run_cmd_capture", { cmd: pm.latest_version_cmd! });
            setLatestVersion(out.trim());
          } catch {
            setLatestVersion(null);
          }
        },
      });
    }

    // Step 2: cache
    if (pm.cache_detect_cmd || pm.cache_default_path || pm.cache_env_var) {
      steps.push({
        label: `正在检测 ${pm.display_name} 缓存路径...`,
        run: async () => {
          try {
            const info = await invoke<{ path: string; size: string; is_link: boolean; real_target: string; parent_link: ParentLink | null }>("get_pkg_cache_info", {
              projectId,
              pmId: pm.id,
              storageKind: "cache"
            });
            setCacheInfo({ ...info, detect_source: pm.cache_detect_cmd || pm.cache_env_var || pm.cache_default_path || "" });
          } catch { /* ignore */ }
        },
      });
    }

    // Step 2b: data
    if (pm.data_detect_cmd || pm.data_default_path || pm.data_env_var) {
      steps.push({
        label: `正在检测 ${pm.display_name} 数据路径...`,
        run: async () => {
          try {
            const info = await invoke<{ path: string; size: string; is_link: boolean; real_target: string; parent_link: ParentLink | null }>("get_pkg_cache_info", {
              projectId,
              pmId: pm.id,
              storageKind: "data"
            });
            setDataInfo({ path: info.path, size: info.size, is_link: info.is_link, real_target: info.real_target, detect_source: pm.data_detect_cmd || pm.data_env_var || pm.data_default_path || "" });
          } catch { /* ignore */ }
        },
      });
    }

    // Step 3: proxy
    if (pm.proxy_detect_cmd) {
      steps.push({
        label: `正在检测 ${pm.display_name} 代理配置...`,
        run: async () => {
          try {
            const out = await invoke<string>("run_cmd_capture", { cmd: pm.proxy_detect_cmd });
            const v = out.trim();
            if (v && v !== "null" && v !== "undefined") {
              setProxyDetected(v);
              setProxyInput(v);
            }
          } catch { /* ignore */ }
        },
      });
    }
    // Step 4: current mirror
    if (pm.mirror_cmd_template || (pm.mirror_options && pm.mirror_options.length > 0)) {
      steps.push({
        label: `正在检测 ${pm.display_name} 当前镜像源...`,
        run: async () => {
          try {
            if (pm.mirror_cmd_template) {
              const getCmd = pm.mirror_cmd_template.replace("set ", "get ").replace("{url}", "");
              const out = await invoke<string>("run_cmd_capture", { cmd: getCmd });
              const v = out.trim();
              if (v && v !== "null" && v !== "undefined") {
                setCurrentMirror(v);
              }
            } else {
              const list = await invoke<Array<{ tool: string; current: string; mirror_name: string }>>("get_mirrors_list");
              const entry = list.find(m => m.tool.toLowerCase() === pm.id.toLowerCase() || (pm.id === "cargo" && m.tool === "rust"));
              if (entry) {
                setCurrentMirror(entry.current);
              }
            }
          } catch { /* ignore */ }
        },
      });
    }

    // 执行所有步骤
    for (let i = 0; i < steps.length; i++) {
      setDetectStep(steps[i].label);
      await steps[i].run();
    }

    setDetectStep("");
    setChecking(false);
    setHasChecked(true);
  };

  // 懒加载 —— hidden 为 true 时跳过检测，等 tab 切换到此 PM 时才触发
  useEffect(() => {
    if (!hasChecked && !checking && !hidden) {
      runDetection();
    }
  }, [pm.id, hidden]);

  // 安装
  const handleInstall = async () => {
    if (!pm.install_cmd) return;
    setInstalling(true);
    setInstallProgress(true);
    try {
      await invoke("run_cmd_capture", { cmd: pm.install_cmd });
      await runDetection();
    } catch (e: unknown) {
      alert(`安装 ${pm.display_name} 失败: ${e}`);
    } finally {
      setInstalling(false);
      setInstallProgress(false);
    }
  };

  // 升级包管理器
  const handleUpgrade = async () => {
    if (!pm.install_cmd) return;
    setUpgrading(true);
    setInstallProgress(true);
    try {
      await invoke("run_cmd_capture", { cmd: pm.install_cmd });
      await runDetection();
    } catch (e: unknown) {
      alert(`升级 ${pm.display_name} 失败: ${e}`);
    } finally {
      setUpgrading(false);
      setInstallProgress(false);
    }
  };

  // 简单版本比较：返回 true 表示 a > b
  const versionGt = (a: string, b: string): boolean => {
    const pa = a.replace(/^v/, "").split(".").map(Number);
    const pb = b.replace(/^v/, "").split(".").map(Number);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const va = pa[i] || 0;
      const vb = pb[i] || 0;
      if (va > vb) return true;
      if (va < vb) return false;
    }
    return false;
  };

  // 切换镜像
  const handleSwitchMirror = async (url: string, mirrorType: string) => {
    setSwitchingMirror(url);
    try {
      if (pm.mirror_cmd_template) {
        const cmd = pm.mirror_cmd_template.replace("{url}", url);
        await invoke("run_cmd_capture", { cmd });
      } else {
        await invoke("set_mirror", { tool: pm.id, mirrorType });
      }
      setCurrentMirror(url);
    } catch (e: unknown) {
      alert(`切换镜像失败: ${e}`);
    } finally {
      setSwitchingMirror(null);
    }
  };

  // ── 清理缓存（带进度条） ──
  const handleCleanCache = async () => {
    if (!pm.cache_detect_cmd && !pm.cache_default_path && !pm.cache_env_var) return;
    if (!confirm(`将删除所有缓存文件（约 ${cacheInfo?.size || "?"}），确定继续？`)) return;
    setCleaningCache(true);
    setCleanProgress(null);
    const unlisten = await listen<{ stage: string; current: number; total: number; file_name: string }>("clean-cache-progress", (event) => {
      setCleanProgress(event.payload);
    });
    try {
      await invoke("clean_pkg_cache", {
        projectId,
        pmId: pm.id,
        cachePath: cacheInfo?.path || null,
      });
      await runDetection();
    } catch (e: unknown) {
      alert(`清理缓存失败: ${e}`);
    } finally {
      unlisten();
      setCleaningCache(false);
      setCleanProgress(null);
    }
  };

  // 设置代理
  const handleSetProxy = async () => {
    if (!pm.proxy_set_cmd_template) return;
    setSettingProxy(true);
    try {
      if (proxyInput.trim()) {
        const cmd = pm.proxy_set_cmd_template.replace("{url}", proxyInput.trim());
        await invoke("run_cmd_capture", { cmd });
      } else {
        // 清空代理：npm/yarn 用 delete，我们也用 set 命令模板传空值解决
        // 实际上 npm config delete proxy 更合适
        if (pm.id === "npm") {
          await invoke("run_cmd_capture", { cmd: "npm config delete proxy" });
          await invoke("run_cmd_capture", { cmd: "npm config delete https-proxy" });
        } else {
          const cmd = pm.proxy_set_cmd_template.replace("{url}", "");
          await invoke("run_cmd_capture", { cmd });
        }
      }
      setProxyDetected(proxyInput.trim() || null);
    } catch (e: unknown) {
      alert(`设置代理失败: ${e}`);
    } finally {
      setSettingProxy(false);
    }
  };

  // 全局包
  const loadPackages = async () => {
    if (!pm.pkg_list_cmd) return;
    setLoadingPackages(true);
    try {
      const list = await invoke<Array<{ name: string; current_version: string; latest_version: string; status: string; homepage: string }>>("get_global_packages", { sdkName: pm.id });
      setPackages(list);
    } catch { /* ignore */ } finally {
      setLoadingPackages(false);
    }
  };

  useEffect(() => {
    if (hasChecked && installed && pm.pkg_list_cmd && packages.length === 0 && !loadingPackages) {
      loadPackages();
    }
  }, [hasChecked, installed]);

  const handleUpgradePackage = async (pkgName: string) => {
    if (!pm.id) return;
    setUpgradingPkg(pkgName);
    try {
      await invoke("upgrade_global_package", { sdkName: pm.id, pkgName });
      await loadPackages();
    } catch (e: unknown) {
      alert(`升级 ${pkgName} 失败: ${e}`);
    } finally {
      setUpgradingPkg(null);
    }
  };

  // ── 工作流 UI 渲染函数 ──
  const renderWorkflow = () => {
    const isData = workflowType === "data";
    const accentBg = isData ? "bg-red-500/10" : "bg-amber-500/10";
    const accentBorder = isData ? "border-red-500/20" : "border-amber-500/20";
    const accentText = isData ? "text-red-400" : "text-amber-400";
    const btnBg = isData ? "bg-red-600 hover:bg-red-500" : "bg-amber-600 hover:bg-amber-500";
    const progressBarColor = isData ? "bg-red-500/60" : "bg-amber-500/60";

    const stepLabels: Record<string, string> = {
      method: "选择方式",
      paths: "配置路径",
      confirm: "确认预览",
      executing: "执行中",
      done: "已完成",
    };

    const totalSteps = 4;

    // ── Step: 选择方式 ──
    if (workflowStep === "method") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更{isData ? "数据" : "缓存"}配置 · Step 1/{totalSteps} · {stepLabels.method}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>
          <div className="space-y-1.5">
            <p className="text-[12px] text-slate-300">请选择变更方式：</p>
            <label className={`flex items-start gap-2 p-2.5 rounded-lg cursor-pointer transition-all border ${workflowMethod === "junction"
              ? `${accentBorder} bg-white/5`
              : "border-white/5 hover:bg-white/[0.02]"
              }`}>
              <input type="radio" name="wf_method" value="junction" checked={workflowMethod === "junction"}
                onChange={() => setWorkflowMethod("junction")} className="mt-0.5" />
              <div>
                <span className="text-[12px] font-semibold text-slate-200">A. Junction 链接</span>
                <p className="text-[13px] text-slate-500 mt-0.5">
                  创建一个目录链接，将{isData ? "数据" : "缓存"}目录指向新位置。文件实际存储在新位置，原位置通过链接访问。
                </p>
              </div>
            </label>
            {!isData && (pm.cache_set_cmd_template || pm.cache_env_var) && (
              <label className={`flex items-start gap-2 p-2.5 rounded-lg cursor-pointer transition-all border ${workflowMethod === "point"
                ? `${accentBorder} bg-white/5`
                : "border-white/5 hover:bg-white/[0.02]"
                }`}>
                <input type="radio" name="wf_method" value="point" checked={workflowMethod === "point"}
                  onChange={() => setWorkflowMethod("point")} className="mt-0.5" />
                <div>
                  <span className="text-[12px] font-semibold text-purple-300">B. 指向配置</span>
                  <p className="text-[13px] text-slate-500 mt-0.5">
                    直接修改 {pm.display_name} 的配置或环境变量，更改{isData ? "数据" : "缓存"}目录路径。不改动已有文件。
                  </p>
                </div>
              </label>
            )}
          </div>
          <div className="flex justify-end">
            <button onClick={workflowNext}
              className={`px-3 py-1 ${btnBg} text-white rounded text-[11px] font-semibold cursor-pointer transition-colors`}>
              下一步 →
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 配置路径 ──
    if (workflowStep === "paths") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更{isData ? "数据" : "缓存"}配置 · Step 2/{totalSteps} · {stepLabels.paths}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>

          {workflowMethod === "junction" ? (
            <>
              <p className="text-[11px] text-slate-400">
                <span className="font-semibold text-slate-300">Junction 链接模式</span> — ① 形式路径（链接所在位置）→ ② 实际路径（数据存放位置）
              </p>
              <div className="space-y-1.5">
                <div>
                  <label className="text-[13px] text-slate-500 block mb-0.5">① 形式路径（链接创建位置，即 {pm.display_name} {isData ? "数据" : "缓存"}的原始路径）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowLinkPath} onChange={(e) => setWorkflowLinkPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono" placeholder={`${isData ? "数据" : "缓存"}源路径`} />
                    <button onClick={() => browseWorkflowPath(setWorkflowLinkPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
                <div>
                  <label className="text-[13px] text-slate-500 block mb-0.5">② 实际路径（数据真实存放位置，建议选非 C 盘）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowActualPath} onChange={(e) => setWorkflowActualPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono" placeholder="目标路径（如 D:\any-version-caches\npm）" />
                    <button onClick={() => browseWorkflowPath(setWorkflowActualPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
              </div>
            </>
          ) : (
            <>
              <p className="text-[11px] text-slate-400">
                <span className="font-semibold text-purple-300">指向配置模式</span> — 直接修改 {pm.display_name} 配置，指向新路径
              </p>
              <div>
                <label className="text-[13px] text-slate-500 block mb-0.5">指向路径（设置 {pm.display_name} 的{isData ? "数据" : "缓存"}目录）</label>
                <div className="flex items-center gap-1">
                  <input type="text" value={workflowPointPath} onChange={(e) => setWorkflowPointPath(e.target.value)}
                    className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono"
                    placeholder={pm.cache_default_path || "新路径"} />
                  <button onClick={() => browseWorkflowPath(setWorkflowPointPath)}
                    className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                    <FolderOpen className="w-3 h-3" />
                  </button>
                </div>
              </div>
            </>
          )}

          {/* 旧文件处理方式（Junction 和 Point 共用） */}
          <div className="pt-1 space-y-1">
            <p className="text-[13px] text-slate-400 font-semibold">旧文件处理方式：</p>
            {/* 移动旧文件 */}
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "move" ? "border-blue-500/30 bg-blue-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="move" checked={workflowFileAction === "move"}
                onChange={() => setWorkflowFileAction("move")} className="mt-0.5" />
              <div>
                <span className="text-[13px] font-semibold text-blue-300">移动旧文件到新目录</span>
                <p className="text-[11px] text-slate-500 mt-0.5">将现有文件整体复制到新位置，完成后{workflowMethod === "junction" ? "创建链接" : "修改配置指向"}。保留所有已有数据。</p>
              </div>
            </label>
            {/* 删除旧文件 */}
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "delete" ? "border-red-500/30 bg-red-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="delete" checked={workflowFileAction === "delete"}
                onChange={() => setWorkflowFileAction("delete")}
                disabled={isData} className="mt-0.5" />
              <div>
                <span className={`text-[13px] font-semibold ${isData ? "text-slate-600" : "text-red-300"}`}>删除旧文件</span>
                <p className="text-[11px] text-slate-500 mt-0.5">
                  {isData ? "数据文件不可直接删除以保证安全性" : "直接删除旧文件。（缓存可从网络重新下载，适合清空重建）"}
                </p>
              </div>
            </label>
            {/* 不做改动 */}
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "keep" ? "border-slate-500/30 bg-slate-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="keep" checked={workflowFileAction === "keep"}
                onChange={() => setWorkflowFileAction("keep")} className="mt-0.5" />
              <div>
                <span className="text-[13px] font-semibold text-slate-300">不做改动</span>
                <p className="text-[11px] text-slate-500 mt-0.5">仅{workflowMethod === "junction" ? "创建链接指向新目录" : "修改配置指向新路径"}，旧目录中的文件保持原样不动。</p>
              </div>
            </label>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[11px] font-semibold cursor-pointer transition-colors">
              ← 上一步
            </button>
            <button onClick={workflowNext}
              disabled={workflowMethod === "junction"
                ? (!workflowLinkPath || !workflowActualPath || workflowLinkPath === workflowActualPath)
                : !workflowPointPath}
              className={`px-3 py-1 ${btnBg} text-white rounded text-[11px] font-semibold cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed`}>
              预览 →
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 确认预览 ──
    if (workflowStep === "confirm") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更{isData ? "数据" : "缓存"}配置 · Step 3/{totalSteps} · {stepLabels.confirm}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>

          {/* 预览卡片 */}
          <div className="p-3 bg-black/20 rounded-lg border border-white/5 space-y-2">
            <p className="text-[11px] text-slate-400 font-semibold uppercase tracking-wider">操作预览</p>
            <div className="space-y-1.5">
              <div className="flex items-center gap-2 text-[12px]">
                <span className={`px-1.5 py-0.5 rounded text-[13px] font-semibold ${workflowMethod === "junction" ? "bg-blue-500/10 text-blue-400" : "bg-purple-500/10 text-purple-400"
                  }`}>
                  {workflowMethod === "junction" ? "Junction" : "指向"}
                </span>
                {workflowMethod === "junction" ? (
                  <div className="font-mono text-slate-300 space-y-0.5">
                    <p className="flex items-center gap-1">
                      <span className="text-[13px] text-slate-500 flex-shrink-0">形式路径：</span>
                      <span className="text-[11px] break-all">{workflowLinkPath}</span>
                    </p>
                    <p className="flex items-center gap-1">
                      <span className="text-[13px] text-blue-400 flex-shrink-0">↓ 链接到</span>
                      <span className="text-[11px] text-blue-300 break-all">{workflowActualPath}</span>
                    </p>
                  </div>
                ) : (
                  <p className="font-mono text-slate-300 text-[12px] break-all">
                    配置指向：{workflowPointPath || "(未设置)"}
                  </p>
                )}
              </div>
              <div className="flex items-center gap-2 text-[12px]">
                <span className="text-slate-500">旧文件处理：</span>
                <span className={
                  workflowFileAction === "delete" ? "text-red-400 font-semibold" :
                    workflowFileAction === "move" ? "text-blue-400 font-semibold" :
                      "text-slate-400"
                }>
                  {workflowFileAction === "delete" ? "🗑 删除旧文件" :
                    workflowFileAction === "move" ? "📦 移动到新目录" : "📌 不做改动"}
                </span>
              </div>
              {(workflowMethod === "junction" && workflowLinkPath.toLowerCase().startsWith("c:")) && (
                <p className="text-[13px] text-red-400/80 flex items-center gap-1">
                  <AlertTriangle className="w-2.5 h-2.5" />当前路径在 C 盘，建议迁移到非系统盘
                </p>
              )}
            </div>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[11px] font-semibold cursor-pointer transition-colors">
              ← 上一步
            </button>
            <button onClick={workflowNext} disabled={workflowExecuting}
              className={`px-3 py-1 ${btnBg} text-white rounded text-[11px] font-semibold cursor-pointer transition-colors disabled:opacity-40`}>
              确认执行
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 执行中 ──
    if (workflowStep === "executing") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center gap-2">
            <Loader className="w-3.5 h-3.5 animate-spin text-blue-400" />
            <span className={`text-[12px] font-semibold ${accentText}`}>
              正在执行 · {workflowProgress?.stage || "准备中..."}
            </span>
          </div>
          {workflowProgress && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between text-[13px] text-slate-400">
                <span>{workflowProgress.stage}</span>
                <span className="font-mono">{workflowProgress.current}/{workflowProgress.total}</span>
              </div>
              <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
                <div
                  className={`h-full ${progressBarColor} rounded-full transition-all duration-200`}
                  style={{ width: `${workflowProgress.total > 0 ? (workflowProgress.current / workflowProgress.total) * 100 : 0}%` }}
                />
              </div>
              {workflowProgress.file_name && (
                <p className="text-[13px] text-slate-500 truncate font-mono">{workflowProgress.file_name}</p>
              )}
            </div>
          )}
          {!workflowProgress && (
            <p className="text-[11px] text-slate-400 flex items-center gap-1">
              <Loader className="w-3 h-3 animate-spin" />正在启动操作...
            </p>
          )}
        </div>
      );
    }

    // ── Step: 完成 ──
    if (workflowStep === "done") {
      return (
        <div className={`mt-3 p-3 rounded-xl border border-emerald-500/20 bg-emerald-500/5 space-y-3 animate-fadeIn`}>
          <div className="flex items-center gap-2">
            <CheckCircle className="w-4 h-4 text-emerald-400" />
            <span className="text-[12px] font-semibold text-emerald-300">操作成功！</span>
          </div>
          <p className="text-[11px] text-emerald-400/70">
            {isData ? "数据" : "缓存"}已成功{workflowMethod === "junction" ? "迁移" : "重新配置"}，现状已更新。
          </p>
          <div className="flex justify-end">
            <button onClick={closeWorkflow}
              className="px-3 py-1 bg-emerald-600/50 hover:bg-emerald-600 text-white rounded text-[11px] font-semibold cursor-pointer transition-colors">
              关闭
            </button>
          </div>
        </div>
      );
    }

    return null;
  };

  // 隐藏时跳过渲染，但保持组件挂载和状态（避免切换回此 tab 时重新检测）
  if (hidden) return null;

  return (
    <div className="space-y-5">
      {/* 头部状态栏 */}
      <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className={`w-9 h-9 rounded-xl flex items-center justify-center ${installed ? "bg-emerald-500/10" : "bg-slate-500/10"}`}>
              <Package className={`w-4.5 h-4.5 ${installed ? "text-emerald-400" : "text-slate-500"}`} />
            </div>
            <div>
              <div className="flex items-center gap-2">
                <h3 className="text-sm font-bold text-white">{pm.display_name}</h3>
                {pm.built_in && (
                  <span className="px-1.5 py-0.5 rounded text-[11px] bg-purple-500/10 text-purple-400 border border-purple-500/20 font-semibold">内置</span>
                )}
              </div>
              {installed ? (
                <span className="text-[13px] text-emerald-400 font-mono">{version || "已安装"}</span>
              ) : checking ? (
                <span className="text-[13px] text-blue-400 flex items-center gap-1"><Loader className="w-3 h-3 animate-spin" />检测中...</span>
              ) : (
                <span className="text-[13px] text-slate-400">未安装</span>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button onClick={runDetection} disabled={checking || installing || upgrading} className="p-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg border border-white/5 cursor-pointer transition-all" title="刷新检测">
              <RefreshCw className={`w-3.5 h-3.5 ${checking ? "animate-spin" : ""}`} />
            </button>
            {installed && !pm.built_in && pm.install_cmd && (
              <>
                {latestVersion && version && versionGt(latestVersion, version) ? (
                  <button onClick={handleUpgrade} disabled={upgrading || installing} className="px-4 py-1.5 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-lg text-[11px] font-semibold cursor-pointer transition-all flex items-center gap-1.5">
                    <Download className="w-3.5 h-3.5" />{upgrading ? "升级中..." : `升级至 ${latestVersion}`}
                  </button>
                ) : latestVersion && version && !versionGt(latestVersion, version) ? (
                  <span className="text-[11px] text-emerald-400 px-2 py-1 rounded bg-emerald-500/10 border border-emerald-500/20 font-semibold">已是最新</span>
                ) : null}
              </>
            )}
            {!installed && pm.install_cmd && (
              <button onClick={handleInstall} disabled={installing || upgrading} className="px-4 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-[11px] font-semibold cursor-pointer transition-all flex items-center gap-1.5">
                <Download className="w-3.5 h-3.5" />{installing ? "安装中..." : "安装"}
              </button>
            )}
          </div>
        </div>
        {/* 安装/升级进度条 */}
        {installProgress && (
          <div className="mt-3 space-y-1.5 animate-fadeIn">
            <div className="flex items-center gap-2 text-[13px] text-blue-300">
              <Loader className="w-3 h-3 animate-spin" />
              {upgrading ? `正在升级 ${pm.display_name}...` : `正在安装 ${pm.display_name}...`}
            </div>
            <div className="w-full h-1.5 bg-white/5 rounded-full overflow-hidden">
              <div className="h-full bg-blue-500/60 rounded-full animate-pulse" style={{ width: "100%" }} />
            </div>
          </div>
        )}
        {detectStep && (
          <div className="mt-3 flex items-center gap-2 text-[13px] text-blue-300">
            <Loader className="w-3 h-3 animate-spin" />{detectStep}
          </div>
        )}
      </div>

      {/* 未安装提示 */}
      {hasChecked && !installed && !checking && (
        <div className="glass-panel rounded-2xl p-6 border border-white/5 bg-white/2 text-center animate-fadeIn">
          <Package className="w-10 h-10 text-slate-500 mx-auto mb-3 opacity-50" />
          <p className="text-slate-400 text-sm font-semibold">{pm.display_name} 未安装</p>
          <p className="text-[13px] text-slate-500 mt-1">安装后可管理缓存、数据、镜像、代理和全局依赖包。</p>
        </div>
      )}

      {/* 缓存管理 */}
      {hasChecked && installed && (pm.cache_detect_cmd || pm.cache_default_path) && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <HardDrive className="w-4 h-4 text-amber-400" />
            <h4 className="text-xs font-semibold text-white">缓存管理</h4>
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20">缓存</span>
          </div>
          {cacheInfo ? (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-3">
              {/* 缓存状态 — 格式: xxx（读取自 ccc）→ yyy */}
              <div className="space-y-1">
                <p className="text-[12px] font-mono text-slate-300 break-all leading-relaxed">
                  <span className="text-slate-300">{cacheInfo.path}</span>
                  <span className="mx-1 px-1.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[11px] inline-flex items-center gap-0.5">{cacheInfo.detect_source}</span>
                  {cacheInfo.real_target ? (
                    <>
                      <span className="mx-1 px-1.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[11px] inline-flex items-center gap-0.5"> 链接到(junction) </span>
                      <span className="text-[11px] font-mono text-slate-300">{cacheInfo.real_target}</span>
                      <span className="ml-1 px-1.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[11px] inline-flex items-center gap-0.5">
                        {cacheInfo.size}
                      </span>
                    </>
                  ) : (
                    <span className="ml-1 text-[13px] text-slate-500">{cacheInfo.size}</span>
                  )}
                </p>
              </div>

              {/* 操作行 */}
              <div className="pt-2 border-t border-white/5 flex items-center gap-2">
                <button onClick={() => openWorkflow("cache")} disabled={workflowType !== null}
                  className="px-2.5 py-1 bg-amber-600/80 hover:bg-amber-600 disabled:opacity-40 text-white rounded text-[11px] font-semibold cursor-pointer flex items-center gap-1 flex-shrink-0 transition-colors">
                  <FolderSync className="w-3 h-3" />开始变更
                </button>
                <button onClick={handleCleanCache} disabled={cleaningCache || workflowType !== null}
                  className="px-2.5 py-1 bg-red-600/80 hover:bg-red-600 disabled:opacity-40 text-white rounded text-[11px] font-semibold cursor-pointer flex items-center gap-1 flex-shrink-0 transition-colors">
                  <Trash2 className="w-3 h-3" />{cleaningCache ? "清理中" : "清理缓存"}
                </button>
              </div>
              {cleanProgress && (
                <div className="space-y-1 pt-1">
                  <div className="flex items-center justify-between text-[13px] text-slate-400">
                    <span>{cleanProgress.stage}</span>
                    <span>{cleanProgress.current}/{cleanProgress.total}</span>
                  </div>
                  <div className="w-full h-1 bg-white/5 rounded-full overflow-hidden">
                    <div className="h-full bg-red-500/60 rounded-full transition-all duration-200"
                      style={{ width: `${cleanProgress.total > 0 ? (cleanProgress.current / cleanProgress.total) * 100 : 0}%` }} />
                  </div>
                  {cleanProgress.file_name && (
                    <p className="text-[13px] text-slate-500 truncate font-mono">{cleanProgress.file_name}</p>
                  )}
                </div>
              )}

              {/* 工作流面板 — 缓存变更 */}
              {workflowType === "cache" && renderWorkflow()}
            </div>
          ) : (
            <p className="text-[13px] text-slate-500">默认路径: <span className="font-mono text-slate-400">{pm.cache_default_path || "未配置"}</span></p>
          )}
        </div>
      )}

      {/* 数据管理 — 安全迁移（必须拷贝，不可删） */}
      {hasChecked && installed && pm.data_detect_cmd && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <HardDrive className="w-4 h-4 text-red-400" />
            <h4 className="text-xs font-semibold text-white">数据管理</h4>
            <span className="text-[11px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 border border-red-500/20">数据</span>
          </div>
          <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-3">
            {/* 数据状态 */}
            {dataInfo ? (
              <div className="space-y-1">
                <p className="text-[12px] font-mono text-slate-300 break-all leading-relaxed">
                  <span className="text-slate-300">{dataInfo.path}</span>
                  <span className="mx-1 px-1.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[11px] inline-flex items-center gap-0.5">{dataInfo.detect_source}</span>
                  {dataInfo.real_target ? (
                    <>
                      <span className="mx-1 px-1.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[11px] inline-flex items-center gap-0.5"> 链接到(junction) </span>
                      <span className="text-[11px] font-mono text-slate-300">{dataInfo.real_target}</span>
                      <span className="ml-1 px-1.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[11px] inline-flex items-center gap-0.5">
                        {dataInfo.size}
                      </span>
                    </>
                  ) : (
                    <span className="ml-1 text-[13px] text-slate-500">{dataInfo.size}</span>
                  )}
                </p>
              </div>
            ) : (
              <div className="space-y-1">
                <p className="text-[12px] text-slate-500 flex items-center gap-1.5">
                  <AlertTriangle className="w-3 h-3 text-yellow-400" />
                  <span>未设置 — 执行检测命令 </span>
                  <span className="text-[11px] font-mono text-slate-400">{pm.data_detect_cmd}</span>
                  <span> 未返回有效路径</span>
                </p>
              </div>
            )}

            <p className="text-[11px] text-red-400/70">⚠ 数据文件必须拷贝后迁移，不可直接删除（保证安全性）。</p>

            {/* 操作行 */}
            <div className="pt-2 border-t border-white/5">
              <button onClick={() => openWorkflow("data")} disabled={workflowType !== null}
                className="px-2.5 py-1 bg-red-600/80 hover:bg-red-600 disabled:opacity-40 text-white rounded text-[11px] font-semibold cursor-pointer flex items-center gap-1 transition-colors">
                <FolderSync className="w-3 h-3" />{dataInfo ? "开始迁移" : "设置数据目录"}
              </button>
            </div>

            {/* 工作流面板 — 数据迁移 */}
            {workflowType === "data" && renderWorkflow()}
          </div>
        </div>
      )}

      {/* 镜像配置 */}
      {hasChecked && installed && pm.mirror_options && pm.mirror_options.length > 0 && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            <Globe className="w-4 h-4 text-blue-400" />
            <h4 className="text-xs font-semibold text-white">镜像配置</h4>
            {currentMirror && (
              <span className="ml-auto text-[11px] text-slate-400 font-mono bg-black/20 px-2 py-0.5 rounded border border-white/5 break-all max-w-[400px]">当前: {currentMirror}</span>
            )}
          </div>
          <div className="grid grid-cols-1 gap-1.5">
            {pm.mirror_options.map((opt) => {
              const isCurrent = currentMirror === opt.url;
              return (
                <button key={opt.mirror_type} onClick={() => handleSwitchMirror(opt.url, opt.mirror_type)} disabled={switchingMirror !== null || isCurrent}
                  className={`flex items-center justify-between px-3 py-2 rounded-lg text-[13px] font-medium cursor-pointer transition-all border
                    ${isCurrent ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300" : "bg-black/20 border-white/5 text-slate-300 hover:bg-white/5"}`}>
                  <span>{opt.name}</span>
                  <div className="flex items-center gap-1.5 ml-auto">
                    <span className={`text-[12px] ${isCurrent ? "text-emerald-400" : "text-slate-500"} font-mono`}>
                      {opt.url}
                    </span>
                    {switchingMirror === opt.url ? <Loader className="w-3 h-3 animate-spin text-blue-400" /> : isCurrent && <CheckCircle className="w-3 h-3 text-emerald-400" />}
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* 代理配置 */}
      {hasChecked && installed && pm.proxy_detect_cmd && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center gap-2">
            {proxyDetected ? <Wifi className="w-4 h-4 text-emerald-400" /> : <WifiOff className="w-4 h-4 text-slate-500" />}
            <h4 className="text-xs font-semibold text-white">代理配置</h4>
            {proxyDetected && <span className="text-[12px] text-emerald-400 font-mono">已配置</span>}
          </div>
          {proxyDetected && (
            <p className="font-mono text-[13px] text-slate-300 truncate" title={proxyDetected}>当前代理: {proxyDetected}</p>
          )}
          <div className="flex items-center gap-1.5">
            <input type="text" value={proxyInput} onChange={(e) => setProxyInput(e.target.value)}
              className="flex-1 glass-input px-3 py-1.5 text-[13px] font-mono" placeholder="http://proxy.example.com:8080" />
            <button onClick={handleSetProxy} disabled={settingProxy}
              className="px-3 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-[13px] font-semibold cursor-pointer flex-shrink-0">
              {settingProxy ? "设置中..." : proxyInput ? "设置代理" : "清除代理"}
            </button>
          </div>
          <p className="text-[12px] text-slate-500">设置 HTTP/HTTPS 代理，留空并点击清除可移除代理配置。</p>
        </div>
      )}

      {/* 全局包 */}
      {hasChecked && installed && pm.pkg_list_cmd && (
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Package className="w-4 h-4 text-blue-400" />
              <h4 className="text-xs font-semibold text-white">全局依赖包</h4>
            </div>
            <button onClick={loadPackages} disabled={loadingPackages} className="flex items-center gap-1 px-2.5 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[13px] border border-white/5 cursor-pointer">
              <RefreshCw className={`w-3 h-3 ${loadingPackages ? "animate-spin" : ""}`} />刷新
            </button>
          </div>
          {loadingPackages ? (
            <div className="flex items-center gap-2 text-[13px] text-slate-400 py-2"><Loader className="w-3 h-3 animate-spin text-blue-400" />正在扫描...</div>
          ) : packages.length === 0 ? (
            <p className="text-[13px] text-slate-500">无全局依赖包，或无法获取列表。</p>
          ) : (
            <div className="max-h-[250px] overflow-y-auto">
              <table className="w-full text-left text-[13px]">
                <thead><tr className="text-slate-500 border-b border-white/5"><th className="p-2">包名</th><th className="p-2 w-20">当前</th><th className="p-2 w-20">最新</th><th className="p-2 w-16">状态</th><th className="p-2 w-16 text-center">操作</th></tr></thead>
                <tbody className="divide-y divide-white/5">
                  {packages.map((p) => (
                    <tr key={p.name} className="hover:bg-white/2 text-slate-300">
                      <td className="p-2 font-medium">
                        <button onClick={() => openUrl(p.homepage)} className="hover:text-blue-400 transition-colors cursor-pointer group">{p.name}<ExternalLink className="w-2.5 h-2.5 inline ml-0.5 text-slate-600 group-hover:text-blue-400 opacity-0 group-hover:opacity-100" /></button>
                      </td>
                      <td className="p-2 font-mono">{p.current_version}</td>
                      <td className="p-2 font-mono text-slate-400">{p.latest_version}</td>
                      <td className="p-2">{p.status === "outdated" ? <span className="text-[11px] px-1 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">可升级</span> : <span className="text-[11px] px-1 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold">最新</span>}</td>
                      <td className="p-2 text-center">
                        {p.status === "outdated" && <button onClick={() => handleUpgradePackage(p.name)} disabled={upgradingPkg === p.name} className="px-2 py-0.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded text-[11px] font-semibold cursor-pointer">{upgradingPkg === p.name ? "升级中" : "升级"}</button>}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════
//  数据目录管理
// ═══════════════════════════════════════
export function DataDirsTab({ project, onRefresh }: { project: ProjectStatus; onRefresh: () => Promise<void> }) {
  const [migratingId, setMigratingId] = useState<string | null>(null);
  const [newPath, setNewPath] = useState("");
  const [loading, setLoading] = useState(false);
  const [migrateProgress, setMigrateProgress] = useState<{ stage: string; current: number; total: number; file_name: string } | null>(null);

  // 监听迁移进度
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    if (loading) {
      listen<{ stage: string; current: number; total: number; file_name: string }>(
        "migrate-storage-progress",
        (event) => {
          setMigrateProgress(event.payload);
        }
      ).then((u) => {
        unlisten = u;
      });
    }
    return () => {
      if (unlisten) unlisten();
    };
  }, [loading]);

  const browseFolder = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择数据迁移目标目录" });
      if (selected) setNewPath(selected as string);
    } catch {
      alert("文件夹选择器不可用，请手动输入路径。");
    }
  };

  const handleMigrate = async (dirId: string, origPath: string) => {
    if (!newPath) {
      alert("请先选择或输入迁移目标路径");
      return;
    }
    if (newPath.toLowerCase().startsWith("c:")) {
      if (!confirm("警告：目标路径仍在 C 盘下，这无法解决 C 盘空间问题。确定继续？")) {
        return;
      }
    }
    setLoading(true);
    setMigrateProgress(null);
    try {
      await invoke("migrate_data_dir", {
        projectId: project.id,
        origPath,
        newPath,
      });
      alert("数据迁移成功！已自动建立 Junction 目录链接。");
      setMigratingId(null);
      setNewPath("");
      await onRefresh();
    } catch (e: unknown) {
      alert("迁移失败: " + e);
    } finally {
      setLoading(false);
      setMigrateProgress(null);
    }
  };

  const handleDelete = async (path: string) => {
    if (!confirm(`警告：该操作将永久删除以下目录及其全部数据：\n${path}\n\n该操作不可撤销，确定继续？`)) {
      return;
    }
    if (!confirm(`再次确认：确定要删除 ${path} 吗？`)) {
      return;
    }
    try {
      await invoke("delete_data_dir", {
        projectId: project.id,
        path,
      });
      alert("删除成功！");
      await onRefresh();
    } catch (e: unknown) {
      alert("删除失败: " + e);
    }
  };

  const dataDirs = project.data_dirs_status || [];

  return (
    <div className="space-y-6">
      <div className="glass-panel rounded-2xl p-5 border border-white/5 bg-white/2 space-y-4">
        <div className="flex items-center gap-2">
          <HardDrive className="w-5 h-5 text-blue-400" />
          <div>
            <h4 className="text-sm font-semibold text-white">数据文件与数据残留管理</h4>
            <p className="text-[11px] text-slate-500 mt-0.5">扫描、迁移主数据文件或清除残留的旧版本数据以节省 C 盘空间。</p>
          </div>
        </div>

        {dataDirs.length === 0 ? (
          <p className="text-[13px] text-slate-400 py-2">未配置数据目录规则或未扫描到对应路径。</p>
        ) : (
          <div className="space-y-4">
            {dataDirs.map((dir) => {
              const isMigrating = migratingId === dir.id;
              return (
                <div key={dir.id + "_" + dir.path} className="p-4 bg-black/20 rounded-xl border border-white/5 space-y-3 animate-fadeIn">
                  <div className="flex items-start justify-between">
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="text-[13px] font-semibold text-white">{dir.display_name}</span>
                        {dir.is_link && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold">
                            已迁移 (Junction)
                          </span>
                        )}
                        {!dir.exists && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20">
                            未发现路径
                          </span>
                        )}
                      </div>
                      <p className="font-mono text-[12px] text-slate-400 break-all">{dir.path}</p>
                      {dir.is_link && dir.real_target && (
                        <p className="font-mono text-[11px] text-slate-500 break-all">
                          ↳ 实际指向: {dir.real_target}
                        </p>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-slate-300 font-mono text-[13px] font-semibold bg-white/5 px-2.5 py-1 rounded-lg">
                        {dir.size}
                      </span>
                    </div>
                  </div>

                  {/* 操作按钮区 */}
                  {dir.exists && !isMigrating && (
                    <div className="flex items-center gap-2 pt-1 border-t border-white/5">
                      {!dir.is_link && (
                        <button
                          onClick={() => {
                            setMigratingId(dir.id);
                            // 预设建议目标路径
                            const driveMatch = dir.path.match(/^([A-Za-z]):\\/);
                            if (driveMatch && driveMatch[1].toUpperCase() === "C") {
                              const suffix = dir.path.substring(2); // Remove "C:"
                              setNewPath(`D:\\AnyVersionData\\${project.id}${suffix}`);
                            } else {
                              setNewPath("");
                            }
                          }}
                          className="px-3 py-1.5 bg-blue-600/80 hover:bg-blue-600 text-white rounded-lg text-[12px] font-semibold cursor-pointer flex items-center gap-1 transition-all"
                        >
                          <FolderSync className="w-3.5 h-3.5" /> 迁移数据
                        </button>
                      )}
                      <button
                        onClick={() => handleDelete(dir.path)}
                        className="px-3 py-1.5 bg-red-600/10 hover:bg-red-600/20 text-red-400 rounded-lg text-[12px] font-semibold cursor-pointer flex items-center gap-1 transition-all"
                      >
                        <Trash2 className="w-3.5 h-3.5" /> 删除数据
                      </button>
                    </div>
                  )}

                  {/* 迁移进行中 / 迁移配置区 */}
                  {isMigrating && (
                    <div className="p-3 bg-white/5 rounded-xl border border-white/5 space-y-3 mt-2 animate-fadeIn">
                      <div className="flex items-center justify-between">
                        <span className="text-[12px] text-slate-300 font-semibold">
                          数据迁移设置 (C 盘 ➔ 非 C 盘)
                        </span>
                        <button
                          onClick={() => {
                            setMigratingId(null);
                            setNewPath("");
                          }}
                          disabled={loading}
                          className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer"
                        >
                          取消
                        </button>
                      </div>

                      {loading ? (
                        <div className="space-y-2 py-2">
                          <div className="flex items-center gap-2 text-[12px] text-blue-300 font-medium">
                            <Loader className="w-3.5 h-3.5 animate-spin" />
                            <span>{migrateProgress?.stage || "正在准备迁移..."}</span>
                          </div>
                          {migrateProgress && migrateProgress.total > 0 && (
                            <div className="space-y-1">
                              <div className="flex items-center justify-between text-[10px] text-slate-500">
                                <span className="truncate max-w-[200px]">{migrateProgress.file_name}</span>
                                <span>{migrateProgress.current} / {migrateProgress.total}</span>
                              </div>
                              <div className="w-full h-1 bg-white/5 rounded-full overflow-hidden">
                                <div
                                  className="h-full bg-blue-500 rounded-full transition-all"
                                  style={{ width: `${(migrateProgress.current / migrateProgress.total) * 100}%` }}
                                />
                              </div>
                            </div>
                          )}
                        </div>
                      ) : (
                        <div className="space-y-2.5">
                          <div className="flex gap-2">
                            <input
                              type="text"
                              value={newPath}
                              onChange={(e) => setNewPath(e.target.value)}
                              className="flex-1 glass-input px-3 py-1.5 text-[12px] font-mono"
                              placeholder="例如 D:\AnyVersionData\mysql_data"
                            />
                            <button
                              onClick={browseFolder}
                              className="px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[12px] border border-white/5 cursor-pointer flex items-center gap-1"
                            >
                              <FolderOpen className="w-3.5 h-3.5" /> 浏览
                            </button>
                          </div>
                          <div className="flex gap-2 justify-end">
                            <button
                              onClick={() => handleMigrate(dir.id, dir.path)}
                              className="px-4 py-1.5 bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg text-[12px] font-semibold cursor-pointer"
                            >
                              开始迁移
                            </button>
                          </div>
                        </div>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

