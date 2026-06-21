import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ExternalLink,
  CheckCircle,
  AlertTriangle,
  RefreshCw,
  Check,
  Trash2,
  Download,
  Plus,
  Globe,
  HardDrive,
  Activity,
  FolderOpen,
  ArrowRight,
  Link,
  FolderSync,
  ArrowUpCircle,
  Package,
  Loader,
} from "lucide-react";
import type { ProjectStatus, ProjectDef, EnvVarStatus, CacheStatus, ServiceStatus, PackageManagerDef } from "./types";

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
  downloadProgress: { sdk: string; downloaded: number; total: number; pct: number } | null;
  installStep: string;
  // 本地注册
  localVersion: string;
  localPath: string;
  registering: boolean;
  registerErr: string | null;
  onLocalVersionChange: (v: string) => void;
  onLocalPathChange: (v: string) => void;
  onRegisterLocal: () => void;
  // 自动扫描
  scanResults: Array<{ path: string; version: string; source: string }>;
  scanning: boolean;
  onScanLocal: () => void;
  onSelectScanResult: (r: { path: string; version: string }) => void;
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
}

// ═══════════════════════════════════════
//  版本管理
// ═══════════════════════════════════════
export function VersionsTab({
  project, remoteVersions, loadingRemote, installingVersion,
  onInstall, onUninstall, onUse,
  downloadProgress, installStep,
  localVersion, localPath, registering, registerErr,
  onLocalVersionChange, onLocalPathChange, onRegisterLocal,
  scanResults, scanning, onScanLocal, onSelectScanResult,
  isOperating,
}: SubTabProps) {
  return (
    <div className="space-y-6">
      {/* 安装进度面板 */}
      {installingVersion && (
        <div className="glass-panel rounded-2xl p-5 border border-blue-500/20 bg-blue-600/5 space-y-4 animate-fadeIn">
          <div className="flex items-center gap-2">
            <Loader className="w-4 h-4 text-blue-400 animate-spin" />
            <h4 className="text-xs font-semibold text-blue-300">
              正在安装 {project.display_name} v{installingVersion.split(" ")[0]}
            </h4>
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
                    <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[8px] font-bold border ${
                      isCompleted
                        ? "bg-emerald-500 text-white border-emerald-500"
                        : isActive
                        ? "bg-blue-600 text-white border-blue-500 animate-pulse"
                        : "bg-white/5 text-slate-500 border-white/10"
                    }`}>
                      {isCompleted ? <Check className="w-3 h-3" /> : idx + 1}
                    </div>
                    <span className={`text-[10px] font-medium ${isActive ? "text-blue-300" : isCompleted ? "text-emerald-400" : "text-slate-500"}`}>
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
              <div className="flex items-center justify-between text-[10px]">
                <span className="text-slate-400">下载进度</span>
                <span className="text-blue-300 font-mono font-semibold">{downloadProgress.pct}%</span>
              </div>
              <div className="w-full h-2 bg-white/5 rounded-full overflow-hidden">
                <div
                  className="h-full bg-gradient-to-r from-blue-600 to-blue-400 rounded-full transition-all duration-300"
                  style={{ width: `${downloadProgress.pct}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-[10px] text-slate-500">
                <span>{(downloadProgress.downloaded / 1024 / 1024).toFixed(1)} MB</span>
                <span>{(downloadProgress.total / 1024 / 1024).toFixed(1)} MB</span>
              </div>
            </div>
          )}

          {/* 当前步骤文字说明 */}
          <p className="text-[10px] text-slate-400">
            {installStep === "下载中" && "正在从远程服务器下载安装包，请稍候..."}
            {installStep === "解压中" && "下载完成，正在解压安装文件..."}
            {installStep === "配置中" && "解压完成，正在配置环境变量和路径..."}
            {installStep === "完成" && "安装配置完成！"}
          </p>
        </div>
      )}

      {/* 已安装版本 */}
      <div className="space-y-3">
        <div>
          <h4 className="text-xs font-semibold text-slate-300">本地已安装版本</h4>
          <p className="text-[10px] text-slate-500 mt-0.5">已下载到本机的版本，点击「启用」可切换当前使用的版本。</p>
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
                  className={`p-3 rounded-xl border flex items-center justify-between transition-all ${
                    isActive
                      ? "bg-blue-600/10 border-blue-500/30 text-white shadow-md shadow-blue-500/5"
                      : "bg-black/20 border-white/5 text-slate-300"
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs font-medium">{v}</span>
                    {isActive && (
                      <span className="px-1.5 py-0.5 rounded text-[8px] bg-blue-600 text-white font-bold">当前</span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5">
                    {!isActive && (
                      <button
                        onClick={() => onUse(v)}
                        disabled={isOperating}
                        className="p-1.5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg text-slate-400 hover:text-slate-200 text-[10px] cursor-pointer transition-all flex items-center gap-0.5"
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
      <div className="space-y-3 border-t border-white/5 pt-4">
        <div>
          <h4 className="text-xs font-semibold text-slate-300">在线安装远程版本</h4>
          <p className="text-[10px] text-slate-500 mt-0.5">从官方服务器下载并安装新版本，选择版本后点击「一键安装」即可。</p>
        </div>
        {loadingRemote ? (
          <div className="flex items-center gap-2 text-slate-400 text-xs py-2">
            <RefreshCw className="w-4 h-4 animate-spin text-blue-400" />
            正在获取远程版本列表...
          </div>
        ) : (
          <div className="flex items-center gap-3">
            <select className="flex-1 glass-input px-3.5 py-2 text-xs" id="remote-version-select">
              <option value="">-- 请选择版本 --</option>
              {remoteVersions.map((v) => (
                <option key={v} value={v}>{v}</option>
              ))}
            </select>
            <button
              onClick={() => {
                const sel = document.getElementById("remote-version-select") as HTMLSelectElement;
                if (sel?.value) onInstall(sel.value);
              }}
              disabled={installingVersion !== null || isOperating}
              className="px-5 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-md shadow-blue-500/10 cursor-pointer transition-all flex items-center gap-1.5"
            >
              <Download className="w-3.5 h-3.5" />
              {installingVersion ? "正在安装..." : "一键安装"}
            </button>
          </div>
        )}
      </div>

      {/* 手动注册 */}
      <div className="space-y-3 border-t border-white/5 pt-4">
        <h4 className="text-xs font-semibold text-slate-300">手动注册本地已存在版本</h4>
        <div className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/1 space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <label className="text-[10px] text-slate-400 font-medium">指定版本号:</label>
              <input
                type="text"
                className="w-full glass-input px-3.5 py-2 text-xs font-mono"
                value={localVersion}
                onChange={(e) => onLocalVersionChange(e.target.value)}
                placeholder="例如: 18.16.0"
              />
            </div>
            <div className="space-y-1.5">
              <label className="text-[10px] text-slate-400 font-medium">本地路径 (bin 的父目录):</label>
              <input
                type="text"
                className="w-full glass-input px-3.5 py-2 text-xs font-mono"
                value={localPath}
                onChange={(e) => onLocalPathChange(e.target.value)}
                placeholder="例如: D:\my-sdks\nodejs"
              />
            </div>
          </div>
          <div className="flex items-center justify-between pt-1">
            <div>
              {registerErr && <span className="text-[10px] text-red-400 font-medium">{registerErr}</span>}
            </div>
            <button
              onClick={onRegisterLocal}
              disabled={registering || !localVersion || !localPath}
              className="px-5 py-2 bg-white/5 border border-white/10 hover:bg-white/10 disabled:opacity-50 text-slate-300 rounded-lg text-xs font-medium cursor-pointer transition-all flex items-center gap-1.5"
            >
              <Plus className="w-3.5 h-3.5" /> 注册本地版本
            </button>

            {/* 自动扫描按钮 */}
            <div className="mt-3 pt-3 border-t border-white/5">
              <button
                onClick={onScanLocal}
                disabled={scanning}
                className="px-5 py-2 bg-emerald-600/20 border border-emerald-500/30 hover:bg-emerald-600/30 disabled:opacity-50 text-emerald-300 rounded-lg text-xs font-medium cursor-pointer transition-all flex items-center gap-1.5"
              >
                <RefreshCw className={`w-3 h-3 ${scanning ? "animate-spin" : ""}`} />
                {scanning ? "扫描中..." : project.managed ? "自动扫描本地版本" : "请先托管项目再扫描"}
              </button>

              {scanResults.length > 0 && (
                <div className="mt-2 space-y-1 max-h-32 overflow-y-auto">
                  {scanResults.map((r, idx) => (
                    <div
                      key={idx}
                      onClick={() => onSelectScanResult(r)}
                      className="flex items-center justify-between p-2 bg-white/5 hover:bg-white/10 rounded cursor-pointer transition-all text-[10px]"
                    >
                      <div className="flex-1 min-w-0">
                        <span className="text-slate-200 font-mono">{r.version}</span>
                        <span className="text-slate-500 ml-2 truncate">{r.path}</span>
                      </div>
                      <span className="text-slate-500 text-[9px] flex-shrink-0 ml-2">{r.source}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  环境变量
// ═══════════════════════════════════════
export function EnvVarsTab({ project }: SubTabProps) {
  const vars: EnvVarStatus[] = project.env_vars_status ?? [];
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <span className="text-xs font-semibold text-slate-300">项目关联环境变量</span>
          <span className="text-[10px] text-slate-500 ml-1.5">{vars.length} 个变量</span>
          <p className="text-[10px] text-slate-500 mt-0.5">环境变量告诉系统这个工具安装在哪里、怎么找到它。托管后会自动配置。</p>
        </div>
      </div>
      {vars.length === 0 ? (
        <div className="p-8 text-center text-slate-500">
          <Globe className="w-10 h-10 mx-auto text-slate-600 mb-3" />
          <p className="text-xs">该项目无需配置环境变量</p>
        </div>
      ) : (
        <div className="border border-white/5 rounded-xl overflow-hidden overflow-x-auto">
          <table className="w-full text-left border-collapse text-[10px] min-w-[450px]">
            <thead>
              <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-medium">
                <th className="p-2.5 w-32">变量名</th>
                <th className="p-2.5 w-36">说明</th>
                <th className="p-2.5">当前配置值</th>
                <th className="p-2.5 w-20">来源</th>
                <th className="p-2.5 w-16">状态</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/5 text-slate-300">
              {vars.map((v) => (
                <tr key={v.name} className="hover:bg-white/1 font-mono">
                  <td className="p-2.5 font-semibold text-slate-200">{v.name}</td>
                  <td className="p-2.5 text-slate-400 font-sans">{v.desc}</td>
                  <td className="p-2.5 break-all select-text">
                    {v.value || <span className="text-slate-600 font-sans">未配置</span>}
                  </td>
                  <td className="p-2.5">
                    {v.source === "HKCU" ? (
                      <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-semibold">用户级</span>
                    ) : v.source === "HKLM" ? (
                      <span className="px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-400 border border-indigo-500/20 text-[9px] font-semibold">系统级</span>
                    ) : (
                      <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-[9px]">未设置</span>
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
  );
}

// ═══════════════════════════════════════
//  缓存管理
// ═══════════════════════════════════════
export function CacheTab({ project, cacheDestPath, migratingCache, onCacheDestPathChange, onMigrateCache }: SubTabProps) {
  const cache: CacheStatus | null = project.cache_status ?? null;
  if (!cache) {
    return (
      <div className="p-8 text-center text-slate-500">
        <HardDrive className="w-10 h-10 mx-auto text-slate-600 mb-3" />
        <p className="text-xs font-medium text-slate-400">未检测到缓存目录</p>
        <p className="text-[10px] text-slate-500 mt-1">该项目暂无可管理的缓存文件。</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-4 bg-white/2">
        <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
          <div className="space-y-1 flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <h3 className="text-xs font-semibold text-white">缓存目录</h3>
              {cache.is_link && (
                <span className="px-2 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-medium flex items-center gap-0.5 font-mono">
                  <Link className="w-3 h-3" /> 已重定向
                </span>
              )}
            </div>
            <div className="font-mono text-[10px] text-slate-400 space-y-0.5 mt-1">
              <p>路径: {cache.path}</p>
              {cache.is_link && (
                <p className="text-blue-400 flex items-center gap-1 font-semibold">
                  <ArrowRight className="w-3 h-3" /> 实际位置: {cache.real_target}
                </p>
              )}
            </div>
          </div>
          <div className="flex items-center gap-1 bg-black/20 px-3.5 py-2.5 rounded-xl border border-white/5">
            <HardDrive className="w-3.5 h-3.5 text-slate-500" />
            <span className="font-mono font-bold text-xs text-white">{cache.size}</span>
          </div>
        </div>

        {!cache.is_link && (
          <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-2">
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={cacheDestPath}
                onChange={(e) => onCacheDestPathChange(e.target.value)}
                className="flex-1 glass-input px-3 py-1.5 text-xs font-mono"
                placeholder="目标路径: D:\any-version-caches\..."
              />
              <button
                onClick={onMigrateCache}
                disabled={migratingCache || !cacheDestPath}
                className="px-3.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer transition-all flex items-center gap-1"
              >
                <FolderSync className="w-3 h-3" />
                {migratingCache ? "迁移中..." : "迁移缓存"}
              </button>
            </div>
            <p className="text-[9px] text-slate-500">
              将缓存从 C 盘迁移至指定目录，原路径自动替换为 NTFS Junction 链接，完全无感。
            </p>
          </div>
        )}

        <div className="p-3 rounded-xl bg-amber-500/5 border border-amber-500/15 text-[10px] space-y-1">
          <span className="font-semibold text-amber-400">检测来源</span>
          <p className="text-slate-300 font-mono break-all">{cache.detect_source}</p>
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  镜像配置
// ═══════════════════════════════════════
export function MirrorTab(_props: SubTabProps) {
  return (
    <div className="space-y-4">
      <div className="glass-panel rounded-2xl p-5 border border-white/5 space-y-4 bg-white/2">
        <div className="flex items-center gap-2">
          <Globe className="w-4 h-4 text-blue-400" />
          <h3 className="text-xs font-semibold text-white">镜像配置</h3>
        </div>
        <p className="text-[11px] text-slate-400">
          该项目支持镜像加速配置。启用托管后，可在下方切换下载源。
        </p>
        <div className="p-8 text-center text-slate-500">
          <Globe className="w-10 h-10 mx-auto text-slate-600 mb-3" />
          <p className="text-xs">镜像配置功能加载中...</p>
          <p className="text-[10px] text-slate-500 mt-1">请先启用托管以查看可用镜像源</p>
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  全局包
// ═══════════════════════════════════════
export function PackagesTab({ packages, loadingPackages, upgradingPackage, packageError, onRefreshPackages, onUpgradePackage }: SubTabProps) {
  return (
    <div className="space-y-4 flex flex-col h-full min-h-0">
      <div className="flex items-center justify-between">
        <div>
          <span className="text-xs font-semibold text-slate-300">全局依赖包列表</span>
          <p className="text-[10px] text-slate-500 mt-0.5">查看当前安装的所有全局包，可一键升级到最新版本。</p>
        </div>
        <button
          onClick={onRefreshPackages}
          disabled={loadingPackages}
          className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
        >
          <RefreshCw className={`w-3 h-3 ${loadingPackages ? "animate-spin" : ""}`} /> 刷新
        </button>
      </div>

      {packageError && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-400 rounded-xl text-[11px] font-mono break-all">
          {packageError}
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
                packages.map((pkg) => (
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
                        <span className="px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 text-[9px] font-semibold">可升级</span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[9px] font-semibold">最新</span>
                      )}
                    </td>
                    <td className="p-3 text-center">
                      {pkg.status === "outdated" ? (
                        <button
                          onClick={() => onUpgradePackage(pkg.name)}
                          disabled={upgradingPackage === pkg.name}
                          className="px-2.5 py-1 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-md text-[9px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-0.5 mx-auto"
                        >
                          <ArrowUpCircle className="w-3 h-3" />
                          {upgradingPackage === pkg.name ? "升级中" : "升级"}
                        </button>
                      ) : (
                        <span className="text-[10px] text-slate-600">无需更新</span>
                      )}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  服务管理
// ═══════════════════════════════════════
export function ServicesTab({ project, def, serviceCtrlLoading, onServiceToggle }: SubTabProps) {
  const svc: ServiceStatus | null = project.service_status ?? null;
  if (!svc) {
    return (
      <div className="p-8 text-center text-slate-500">
        <Activity className="w-10 h-10 mx-auto text-slate-600 mb-3" />
        <p className="text-xs font-medium text-slate-400">未检测到服务信息</p>
        <p className="text-[10px] text-slate-500 mt-1">该项目暂无可管理的本地服务。</p>
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
            <span className="text-[10px] text-slate-400 font-semibold uppercase tracking-wider block">当前状态</span>
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
            <span className="text-[10px] text-slate-400 font-semibold uppercase tracking-wider block">运行参数</span>
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
            <div className="p-3 bg-black/20 rounded-xl border border-white/5 flex items-center justify-between gap-4">
              <div className="min-w-0 flex-1">
                <span className="text-[10px] text-slate-400 font-semibold block">数据目录</span>
                <p className="font-mono text-slate-300 truncate mt-1">{svc.data_dir}</p>
              </div>
              <button
                onClick={() => openUrl(svc.data_dir)}
                className="p-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg border border-white/5 cursor-pointer flex-shrink-0"
                title="在资源管理器中打开"
              >
                <FolderOpen className="w-3.5 h-3.5" />
              </button>
            </div>
          )}
          {svc.log_dir && (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5 flex items-center justify-between gap-4">
              <div className="min-w-0 flex-1">
                <span className="text-[10px] text-slate-400 font-semibold block">日志目录</span>
                <p className="font-mono text-slate-300 truncate mt-1">{svc.log_dir}</p>
              </div>
              <button
                onClick={() => openUrl(svc.log_dir)}
                className="p-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg border border-white/5 cursor-pointer flex-shrink-0"
                title="在资源管理器中打开"
              >
                <FolderOpen className="w-3.5 h-3.5" />
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  包管理器管理
// ═══════════════════════════════════════
export function PkgMgrTab({ def }: SubTabProps) {
  const [detectStep, setDetectStep] = useState("");
  const [detectIndex, setDetectIndex] = useState(0);
  const [detectTotal, setDetectTotal] = useState(0);
  const [pkgStatuses, setPkgStatuses] = useState<Record<string, { installed: boolean; version: string | null }>>({});
  const [cacheInfos, setCacheInfos] = useState<Record<string, { path: string; size: string; is_link: boolean; real_target: string }>>({});
  const [checking, setChecking] = useState(false);
  const [installingPkg, setInstallingPkg] = useState<string | null>(null);
  const [switchingMirror, setSwitchingMirror] = useState<string | null>(null);
  const [migratingCache, setMigratingCache] = useState<string | null>(null);
  const [cacheTargets, setCacheTargets] = useState<Record<string, string>>({});

  const managers: PackageManagerDef[] = def?.package_managers ?? [];

  const checkInstalled = async () => {
    if (!def || managers.length === 0) return;
    setChecking(true);
    const result: Record<string, { installed: boolean; version: string | null }> = {};
    const caches: Record<string, { path: string; size: string; is_link: boolean; real_target: string }> = {};

    // 构建检测队列：每个包管理器的版本检测 + 缓存检测各算一步
    const steps: Array<{ label: string; run: () => Promise<void> }> = [];
    for (const mgr of managers) {
      steps.push({
        label: `正在检测 ${mgr.display_name} 版本...`,
        run: async () => {
          if (mgr.version_cmd) {
            try {
              const output = await invoke<string>("run_cmd_capture", { cmd: mgr.version_cmd });
              result[mgr.id] = { installed: true, version: output.trim() };
            } catch {
              result[mgr.id] = { installed: false, version: null };
            }
          } else {
            result[mgr.id] = { installed: false, version: null };
          }
          setPkgStatuses({ ...result });
        },
      });
      if (mgr.cache_detect_cmd || mgr.cache_default_path) {
        steps.push({
          label: `正在检测 ${mgr.display_name} 缓存路径...`,
          run: async () => {
            if (mgr.cache_detect_cmd) {
              try {
                const info = await invoke<{ path: string; size: string; is_link: boolean; real_target: string }>("get_pkg_cache_info", { cmd: mgr.cache_detect_cmd });
                caches[mgr.id] = info;
                if (!cacheTargets[mgr.id] && !info.is_link) {
                  const drive = info.path.match(/^([A-Za-z]):\\/);
                  if (drive && drive[1].toUpperCase() === "C") {
                    setCacheTargets(prev => ({ ...prev, [mgr.id]: `D:\\any-version-caches\\${mgr.id}` }));
                  }
                }
              } catch {
                // 缓存检测失败，忽略
              }
            }
            setCacheInfos({ ...caches });
          },
        });
      }
    }

    setDetectTotal(steps.length);
    for (let i = 0; i < steps.length; i++) {
      setDetectIndex(i + 1);
      setDetectStep(steps[i].label);
      await steps[i].run();
    }
    setDetectStep("");
    setDetectIndex(0);
    setDetectTotal(0);
    setChecking(false);
  };

  // 懒加载：仅当标签页可见且尚未检测过时才执行
  const [hasChecked, setHasChecked] = useState(false);
  useEffect(() => {
    if (def?.id && !hasChecked && !checking) {
      setHasChecked(true);
      checkInstalled();
    }
  }, [def?.id]);

  const handleInstall = async (mgr: PackageManagerDef) => {
    if (!mgr.install_cmd) return;
    setInstallingPkg(mgr.id);
    try {
      await invoke("run_cmd_capture", { cmd: mgr.install_cmd });
      await checkInstalled();
    } catch (e: unknown) {
      alert("安装 " + mgr.display_name + " 失败: " + e);
    } finally {
      setInstallingPkg(null);
    }
  };

  const handleSwitchMirror = async (mgr: PackageManagerDef, mirrorUrl: string) => {
    if (!mgr.mirror_cmd_template) return;
    setSwitchingMirror(mgr.id + mirrorUrl);
    try {
      const cmd = mgr.mirror_cmd_template.replace("{url}", mirrorUrl);
      await invoke("run_cmd_capture", { cmd });
      alert(mgr.display_name + " 镜像源已切换");
    } catch (e: unknown) {
      alert("切换镜像源失败: " + e);
    } finally {
      setSwitchingMirror(null);
    }
  };

  const handleMigrateCache = async (mgr: PackageManagerDef) => {
    if (!mgr.cache_detect_cmd) return;
    const target = cacheTargets[mgr.id];
    if (!target) return;
    if (target.toLowerCase().startsWith("c:")) {
      alert("目标路径必须位于非 C 盘");
      return;
    }
    setMigratingCache(mgr.id);
    try {
      await invoke("migrate_pkg_cache", { cacheDetectCmd: mgr.cache_detect_cmd, newPath: target });
      await checkInstalled();
    } catch (e: unknown) {
      alert("缓存迁移失败: " + e);
    } finally {
      setMigratingCache(null);
    }
  };

  const handleBrowseFolder = async (mgrId: string) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择缓存迁移目标文件夹" });
      if (selected) {
        setCacheTargets(prev => ({ ...prev, [mgrId]: selected as string }));
      }
    } catch {
      alert("文件夹选择器不可用，请手动输入路径。");
    }
  };

  if (managers.length === 0) {
    return (
      <div className="p-8 text-center text-slate-500">
        <Package className="w-10 h-10 mx-auto text-slate-600 mb-3" />
        <p className="text-xs font-medium text-slate-400">未配置包管理器</p>
        <p className="text-[10px] text-slate-500 mt-1">该项目暂无关联的包管理器。</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Package className="w-4 h-4 text-blue-400" />
          <div>
            <span className="text-xs font-semibold text-slate-300">包管理器</span>
            <span className="text-[10px] text-slate-500 ml-1.5">{managers.length} 个</span>
            <p className="text-[10px] text-slate-500 mt-0.5">检测并管理该语言的包管理工具（如 npm、pip），可查看和迁移缓存路径。</p>
            {detectStep && (
              <div className="mt-1.5 space-y-1">
                <div className="flex items-center gap-2">
                  <Loader className="w-3 h-3 animate-spin text-blue-400" />
                  <span className="text-[10px] text-blue-300 font-medium">{detectStep}</span>
                  <span className="text-[9px] text-slate-500">{detectIndex}/{detectTotal}</span>
                </div>
                <div className="w-full h-1 bg-white/5 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-blue-500 rounded-full transition-all duration-300"
                    style={{ width: `${(detectIndex / detectTotal) * 100}%` }}
                  />
                </div>
              </div>
            )}
          </div>
        </div>
        <button
          onClick={checkInstalled}
          disabled={checking}
          className="flex items-center gap-1.5 px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg text-[10px] border border-white/5 cursor-pointer"
        >
          <RefreshCw className={`w-3 h-3 ${checking ? "animate-spin" : ""}`} /> 刷新检测
        </button>
      </div>

      <div className="space-y-3">
        {managers.map((mgr) => {
          const st = pkgStatuses[mgr.id];
          const isInstalled = st?.installed ?? false;
          const isInstallingThis = installingPkg === mgr.id;
          return (
            <div key={mgr.id} className="glass-panel rounded-2xl p-4 border border-white/5 bg-white/2 space-y-3">
              {/* 标题行 */}
              <div className="flex items-center justify-between">
                <h4 className="text-xs font-semibold text-white">{mgr.display_name}</h4>
                {st && (
                  isInstalled ? (
                    <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[9px] font-bold flex items-center gap-0.5">
                      <CheckCircle className="w-2.5 h-2.5" /> {st.version || "已安装"}
                    </span>
                  ) : (
                    <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-white/5 text-[9px] font-medium">
                      未安装
                    </span>
                  )
                )}
                {!st && checking && (
                  <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[9px] font-medium flex items-center gap-0.5">
                    <Loader className="w-2.5 h-2.5 animate-spin" /> 检测中
                  </span>
                )}
              </div>

              {/* 安装命令 */}
              {mgr.install_cmd && (
                <p className="text-[10px] text-slate-500 font-mono truncate" title={mgr.install_cmd}>
                  {mgr.install_cmd}
                </p>
              )}

              {/* 操作按钮 */}
              {!isInstalled && mgr.install_cmd && (
                <button
                  onClick={() => handleInstall(mgr)}
                  disabled={isInstallingThis}
                  className="w-full px-3 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg text-[10px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-1.5"
                >
                  <Download className="w-3 h-3" />
                  {isInstallingThis ? "正在安装..." : "安装"}
                </button>
              )}

              {/* 缓存路径管理 */}
              {(mgr.cache_detect_cmd || mgr.cache_default_path) && (() => {
                const ci = cacheInfos[mgr.id];
                const displayPath = ci?.path || mgr.cache_default_path || "";
                const isLink = ci?.is_link ?? false;
                const realTarget = ci?.real_target ?? "";
                return (
                <div className="pt-2 border-t border-white/5 space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[9px] text-slate-400 font-semibold uppercase tracking-wider">缓存路径</span>
                    <div className="flex items-center gap-1.5">
                      {ci?.size && (
                        <span className="font-mono text-[9px] text-slate-300 bg-black/20 px-1.5 py-0.5 rounded">{ci.size}</span>
                      )}
                      {isLink && (
                        <span className="px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 border border-blue-500/20 text-[8px] font-medium flex items-center gap-0.5">
                          <Link className="w-2 h-2" /> 已重定向
                        </span>
                      )}
                    </div>
                  </div>
                  <p className="font-mono text-[10px] text-slate-300 truncate" title={displayPath}>
                    {displayPath || "未检测到"}
                  </p>
                  {isLink && realTarget && (
                    <p className="font-mono text-[9px] text-blue-400 flex items-center gap-1">
                      <ArrowRight className="w-2.5 h-2.5" /> 实际位置: {realTarget}
                    </p>
                  )}
                  {!isLink && mgr.cache_detect_cmd && (
                    <div className="flex items-center gap-1.5">
                      <input
                        type="text"
                        value={cacheTargets[mgr.id] || ""}
                        onChange={(e) => setCacheTargets(prev => ({ ...prev, [mgr.id]: e.target.value }))}
                        className="flex-1 glass-input px-2 py-1.5 text-[10px] font-mono"
                        placeholder="迁移目标路径..."
                      />
                      <button
                        onClick={() => handleBrowseFolder(mgr.id)}
                        className="p-1.5 bg-white/5 hover:bg-white/10 text-slate-400 hover:text-slate-200 rounded-md border border-white/5 cursor-pointer transition-all"
                        title="选择文件夹"
                      >
                        <FolderOpen className="w-3.5 h-3.5" />
                      </button>
                      <button
                        onClick={() => handleMigrateCache(mgr)}
                        disabled={migratingCache === mgr.id || !cacheTargets[mgr.id]}
                        className="px-2.5 py-1.5 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-md text-[9px] font-semibold cursor-pointer transition-all flex items-center gap-0.5 flex-shrink-0"
                      >
                        <FolderSync className="w-3 h-3" />
                        {migratingCache === mgr.id ? "迁移中" : "迁移"}
                      </button>
                    </div>
                  )}
                </div>
                );
              })()}
            </div>
          );
        })}
      </div>
    </div>
  );
}
