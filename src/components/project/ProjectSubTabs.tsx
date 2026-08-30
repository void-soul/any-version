import React, { useState, useEffect } from "react";
import MonacoEditor from "../shared/MonacoEditor";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {

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

  FolderSync,

  Loader,
  Search,


  X,
  Wrench,
  Info,
  Save,
  FileText,
} from "lucide-react";
import type { ProjectStatus, ProjectDef, EnvVarStatus, ServiceStatus, PackageManagerDef } from "./types";
import { PackageManagerTab as PackageManagerTabModular } from "./tabs/PackageManagerTab";

// ── 共享 detour Props ──
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

// ═══════════════════════════════════════
//  版本管理
// ═══════════════════════════════════════
export function VersionsTab({
  project, remoteVersions, loadingRemote, installingVersion,
  onInstall, onUninstall, onUse,
  downloadProgress, installStep, onCancelInstall,
  versionsUpdatedAt, onRefreshRemoteVersions,
  isOperating,
}: SubTabProps) {
  const currentVersionNumber = installingVersion
    ? (installingVersion.includes(" · ") ? installingVersion.split(" · ")[1] : installingVersion).trim().split(" ")[0]
    : "";

  return (
    <div className="space-y-6">
      {/* 安装进度面板 */}
      {installingVersion && (
        <div className="glass-panel rounded-2xl p-5 border border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] space-y-4 animate-fadeIn">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Loader className="w-4 h-4 text-[var(--module-accent)] animate-spin" />
              <h4 className="text-xs font-semibold text-[var(--module-accent)]">
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
            {["下载中", "解压中", "创建链接中", "完成"].map((step, idx) => {
              const steps = ["下载中", "解压中", "创建链接中", "完成"];
              const currentIdx = steps.indexOf(installStep);
              const isActive = step === installStep;
              const isCompleted = currentIdx > idx;
              return (
                <React.Fragment key={step}>
                  {idx > 0 && (
                    <div className={`flex-1 h-0.5 rounded-full ${isCompleted ? "bg-emerald-500" : isActive ? "bg-[var(--module-accent)]" : "bg-white/10"}`} />
                  )}
                  <div className="flex items-center gap-1.5">
                    <div className={`w-5 h-5 rounded-full flex items-center justify-center text-[11px] font-bold border ${isCompleted
                      ? "bg-emerald-500 text-white border-emerald-500"
                      : isActive
                        ? "bg-[var(--module-accent)] text-white border-[var(--module-accent)] animate-pulse"
                        : "bg-white/5 text-slate-500 border-white/10"
                      }`}>
                      {isCompleted ? <Check className="w-3 h-3" /> : idx + 1}
                    </div>
                    <span className={`text-[13px] font-medium ${isActive ? "text-[var(--module-accent)]" : isCompleted ? "text-emerald-400" : "text-slate-500"}`}>
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
                  <span className="text-[var(--module-accent)] font-mono font-semibold">{downloadProgress.pct}%</span>
                </div>
              </div>
              <div className="w-full h-2 bg-white/5 rounded-full overflow-hidden">
                <div
                  className="h-full bg-gradient-to-r from-[var(--module-accent)] to-[var(--module-accent-strong)] rounded-full transition-all duration-300"
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
            {installStep === "创建链接中" && `解压完成，正在创建 Junction 链接 (v${currentVersionNumber})...`}
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
                    ? "bg-[var(--module-accent-soft)] border-[var(--module-accent-ring)] text-white shadow-md shadow-[var(--module-accent-ring)]"
                    : "bg-black/20 border-white/5 text-slate-300"
                    }`}
                >
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-xs font-medium">{v}</span>
                    {isActive && (
                      <span className="px-1.5 py-0.5 rounded text-[11px] bg-[var(--module-accent)] text-white font-bold">当前</span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5">
                    {!isActive && (
                      <button
                        onClick={() => onUse(v)}
                        disabled={isOperating || !project.managed || !project.delegation?.version_control}
                        className="p-1.5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg text-slate-400 hover:text-slate-200 text-[13px] cursor-pointer transition-all flex items-center gap-0.5"
                        title={
                          !project.managed 
                            ? "请先开启托管以启用此版本" 
                            : !project.delegation?.version_control 
                            ? "请先在参数配置中开启“版本控制与下载”以启用此版本" 
                            : "启用此版本"
                        }
                      >
                        <Check className="w-3.5 h-3.5" /> 启用
                      </button>
                    )}
                    <button
                      onClick={() => onUninstall(v)}
                      disabled={isOperating || !project.managed || !project.delegation?.version_control}
                      className="p-1.5 hover:bg-red-500/10 hover:text-red-400 disabled:opacity-40 disabled:cursor-not-allowed rounded-lg text-slate-500 cursor-pointer transition-all"
                      title={
                        !project.managed 
                          ? "请先开启托管以卸载此版本" 
                          : !project.delegation?.version_control 
                          ? "请先在参数配置中开启“版本控制与下载”以卸载此版本" 
                          : "卸载此版本"
                      }
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
        disabled={!project.managed || !project.delegation?.version_control}
        disabledReason={
          !project.managed 
            ? "项目尚未开启托管，请在底部开启托管后再进行在线安装。" 
            : "项目尚未开启“版本控制与下载”功能，请在右侧“参数配置”中开启后重试。"
        }
      />


    </div>
  );
}

// ═══════════════════════════════════════
//  环境变量
// ═══════════════════════════════════════
export function EnvVarsTab({ project, def, onActiveSubTabChange, isOperating, repairingEnv, onRepairEnv, onRefresh }: SubTabProps) {
  const vars: EnvVarStatus[] = project.env_vars_status ?? [];
  const [isAdmin, setIsAdmin] = useState(true);

  useEffect(() => {
    invoke<boolean>("is_admin").then(setIsAdmin).catch(() => setIsAdmin(true));
  }, []);

  // 冲突版本管理器状态与操作
  const [conflictManagers, setConflictManagers] = useState<any[]>([]);
  const [loadingConflicts, setLoadingConflicts] = useState(false);
  const [operatingManagerId, setOperatingManagerId] = useState<string | null>(null);

  // ── 统一工作流状态机（冲突管理器路径变更） ──
  const [workflowManagerId, setWorkflowManagerId] = useState<string | null>(null);
  const [workflowStep, setWorkflowStep] = useState<"method" | "paths" | "confirm" | "executing" | "done">("method");
  const [workflowMethod, setWorkflowMethod] = useState<"junction" | "point">("junction");
  const [workflowLinkPath, setWorkflowLinkPath] = useState("");
  const [workflowActualPath, setWorkflowActualPath] = useState("");
  const [workflowPointPath, setWorkflowPointPath] = useState("");
  const [workflowFileAction, setWorkflowFileAction] = useState<"delete" | "move" | "keep">("keep");
  const [_workflowExecuting, setWorkflowExecuting] = useState(false);
  const [workflowProgress, setWorkflowProgress] = useState<{ stage: string; current: number; total: number; file_name: string } | null>(null);

  const loadConflictManagers = async () => {
    if (!project.id || !def || !def.conflict_managers || def.conflict_managers.length === 0) {
      setConflictManagers([]);
      return;
    }
    setLoadingConflicts(true);
    try {
      const list = await invoke<any[]>("get_conflict_managers_status", { sdkId: project.id });
      setConflictManagers(list);
    } catch (e) {
      console.error("加载冲突管理器状态失败:", e);
    } finally {
      setLoadingConflicts(false);
    }
  };

  useEffect(() => {
    loadConflictManagers();
  }, [project.id, def]);

  // 打开工作流
  const openWorkflow = (mgr: any) => {
    setWorkflowManagerId(mgr.id);
    setWorkflowStep("method");
    setWorkflowMethod("junction");
    setWorkflowLinkPath(mgr.cache_path || "");
    setWorkflowPointPath(mgr.cache_path || "");
    setWorkflowFileAction("keep");
    setWorkflowExecuting(false);
    setWorkflowProgress(null);

    // 预设默认迁移目标路径
    const drive = mgr.cache_path?.match(/^([A-Za-z]):\\/);
    if (drive && drive[1].toUpperCase() === "C") {
      setWorkflowActualPath(`D:\\any-version-caches\\${mgr.id}`);
    } else {
      setWorkflowActualPath(mgr.cache_path || "");
    }
  };

  // 关闭工作流
  const closeWorkflow = () => {
    setWorkflowManagerId(null);
  };

  const workflowNext = () => {
    if (workflowStep === "method") {
      setWorkflowStep("paths");
    } else if (workflowStep === "paths") {
      setWorkflowStep("confirm");
    } else if (workflowStep === "confirm") {
      executeWorkflow();
    }
  };

  const workflowPrev = () => {
    if (workflowStep === "paths") {
      setWorkflowStep("method");
    } else if (workflowStep === "confirm") {
      setWorkflowStep("paths");
    }
  };

  const browseWorkflowPath = async (setter: (v: string) => void) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择文件夹" });
      if (selected) setter(selected as string);
    } catch {
      alert("文件夹选择器不可用，请手动输入路径。");
    }
  };

  const executeWorkflow = async () => {
    if (!workflowManagerId) return;

    if (!isAdmin) {
      const confirmed = window.confirm(
        `迁移缓存路径涉及创建系统级软链接 (Symlink/Junction)，这需要 Windows 管理员权限。\n当前 Any Version 未以管理员身份运行，操作可能会因“拒绝访问”而失败。\n\n是否继续？`
      );
      if (!confirmed) return;
    }

    // 检查是否向同一目录移动文件
    const pathsSame = workflowMethod === "junction"
      && workflowLinkPath.toLowerCase().replace(/[\\/]+$/, "")
      === workflowActualPath.toLowerCase().replace(/[\\/]+$/, "");

    if (workflowFileAction === "move" && pathsSame) {
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
        await invoke("handle_conflict_manager_action", {
          sdkId: project.id,
          managerId: workflowManagerId,
          action: "migrate",
          targetPath: workflowActualPath
        });
      } else {
        await invoke("handle_conflict_manager_action", {
          sdkId: project.id,
          managerId: workflowManagerId,
          action: "point",
          targetPath: workflowPointPath
        });
      }

      await loadConflictManagers();
      onRefresh();
      setWorkflowStep("done");
    } catch (e: unknown) {
      alert(`操作失败: ${e}`);
      setWorkflowStep("confirm");
    } finally {
      unlisten();
      setWorkflowExecuting(false);
      setWorkflowProgress(null);
    }
  };

  const handleConflictAction = async (managerId: string, action: string) => {
    if (!isAdmin) {
      const confirmed = window.confirm(
        `操作系统冲突版本管理器 [${managerId}] 的状态需要 Windows 管理员权限。\n当前 Any Version 未以管理员身份运行，操作可能会因“拒绝访问”而失败。\n\n是否继续？`
      );
      if (!confirmed) return;
    }
    setOperatingManagerId(managerId);
    try {
      await invoke("handle_conflict_manager_action", {
        sdkId: project.id,
        managerId,
        action,
        targetPath: null
      });
      alert("操作成功！");
      await loadConflictManagers();
      onRefresh();
    } catch (e: any) {
      alert(`操作失败: ${e}`);
    } finally {
      setOperatingManagerId(null);
    }
  };

  const renderConflictWorkflow = (mgr: any) => {
    const accentBg = "bg-amber-500/10";
    const accentBorder = "border-amber-500/20";
    const accentText = "text-amber-400";
    const btnBg = "bg-amber-600 hover:bg-amber-500";
    const progressBarColor = "bg-amber-500/60";

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
              变更缓存配置 · Step 1/{totalSteps} · {stepLabels.method}
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
                  创建一个目录链接，将缓存目录指向新位置。文件实际存储在新位置，原位置通过链接访问。
                </p>
              </div>
            </label>
            <label className={`flex items-start gap-2 p-2.5 rounded-lg cursor-pointer transition-all border ${workflowMethod === "point"
              ? `${accentBorder} bg-white/5`
              : "border-white/5 hover:bg-white/[0.02]"
              }`}>
              <input type="radio" name="wf_method" value="point" checked={workflowMethod === "point"}
                onChange={() => setWorkflowMethod("point")} className="mt-0.5" />
              <div>
                <span className="text-[12px] font-semibold text-purple-300">B. 指向配置</span>
                <p className="text-[13px] text-slate-500 mt-0.5">
                  直接修改该控制器的环境变量，更改缓存目录路径。不改动已有文件。
                </p>
              </div>
            </label>
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
              变更缓存配置 · Step 2/{totalSteps} · {stepLabels.paths}
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
                  <label className="text-[13px] text-slate-500 block mb-0.5">① 形式路径（链接创建位置，即原始默认路径）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowLinkPath} onChange={(e) => setWorkflowLinkPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono" placeholder="缓存源路径" />
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
                      className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono" placeholder={`目标路径（如 D:\\any-version-caches\\${mgr.id}）`} />
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
                <span className="font-semibold text-purple-300">指向配置模式</span> — 直接修改对应的环境变量，指向新路径
              </p>
              <div>
                <label className="text-[13px] text-slate-500 block mb-0.5">指向路径（设置该管理器的缓存根目录）</label>
                <div className="flex items-center gap-1">
                  <input type="text" value={workflowPointPath} onChange={(e) => setWorkflowPointPath(e.target.value)}
                    className="flex-1 glass-input px-1.5 py-1 text-[12px] font-mono"
                    placeholder={mgr.cache_path || "新路径"} />
                  <button onClick={() => browseWorkflowPath(setWorkflowPointPath)}
                    className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                    <FolderOpen className="w-3 h-3" />
                  </button>
                </div>
              </div>
            </>
          )}

          {/* 旧文件处理方式（本卡片默认为移动/保留） */}
          <div className="pt-1 space-y-1">
            <p className="text-[13px] text-slate-400 font-semibold">旧文件处理方式：</p>
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "move" ? "border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)]" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="move" checked={workflowFileAction === "move"}
                onChange={() => setWorkflowFileAction("move")} className="mt-0.5" />
              <div>
                <span className="text-[13px] font-semibold text-[var(--module-accent)]">移动旧文件到新目录</span>
                <p className="text-[11px] text-slate-500 mt-0.5">将现有文件整体复制到新位置，完成后{workflowMethod === "junction" ? "创建链接" : "修改环境变量"}。保留所有已有工具链。</p>
              </div>
            </label>
            <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "keep" ? "border-slate-500/30 bg-slate-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="keep" checked={workflowFileAction === "keep"}
                onChange={() => setWorkflowFileAction("keep")} className="mt-0.5" />
              <div>
                <span className="text-[13px] font-semibold text-slate-300">不做改动</span>
                <p className="text-[11px] text-slate-500 mt-0.5">仅{workflowMethod === "junction" ? "创建链接指向新目录" : "修改环境变量"}，旧目录中的文件保持原样不动。</p>
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
              变更缓存配置 · Step 3/{totalSteps} · {stepLabels.confirm}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>

          <div className="p-3 bg-black/20 rounded-lg border border-white/5 space-y-2">
            <p className="text-[11px] text-slate-400 font-semibold uppercase tracking-wider">操作预览</p>
            <div className="text-[12px] text-slate-300 space-y-1 font-mono">
              {workflowMethod === "junction" ? (
                <>
                  <div><span className="text-slate-500">模式:</span> Junction 链接 (软链接)</div>
                  <div className="break-all"><span className="text-slate-500">形式路径:</span> {workflowLinkPath}</div>
                  <div className="break-all"><span className="text-slate-500">实际路径:</span> {workflowActualPath}</div>
                </>
              ) : (
                <>
                  <div><span className="text-slate-500">模式:</span> 指向配置 (重定向环境变量)</div>
                  <div className="break-all"><span className="text-slate-500">目标路径:</span> {workflowPointPath}</div>
                </>
              )}
              <div><span className="text-slate-500">旧文件处理:</span> {workflowFileAction === "move" ? "复制移动到新路径" : "保持不动"}</div>
            </div>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[11px] font-semibold cursor-pointer transition-colors">
              ← 上一步
            </button>
            <button onClick={executeWorkflow}
              className={`px-4 py-1 ${btnBg} text-white rounded text-[11px] font-semibold cursor-pointer transition-colors flex items-center gap-1`}>
              <CheckCircle className="w-3 h-3" />
              确认并执行
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 执行中 ──
    if (workflowStep === "executing") {
      const progressPercent = workflowProgress && workflowProgress.total > 0
        ? Math.round((workflowProgress.current / workflowProgress.total) * 100)
        : 0;

      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更缓存配置 · Step 4/{totalSteps} · {stepLabels.executing}
            </span>
          </div>

          <div className="p-3 bg-black/20 rounded-lg border border-white/5 space-y-3">
            <div className="flex items-center justify-between text-xs text-slate-300">
              <span>{workflowProgress?.stage || "正在执行操作..."}</span>
              <span className="font-mono">{progressPercent}%</span>
            </div>

            {/* 进度条 */}
            <div className="w-full bg-white/5 rounded-full h-2 overflow-hidden border border-white/5">
              <div className={`${progressBarColor} h-2 rounded-full transition-all duration-300`} style={{ width: `${progressPercent}%` }}></div>
            </div>

            {workflowProgress && (
              <div className="text-[11px] text-slate-500 font-mono space-y-0.5">
                <div className="truncate">文件: {workflowProgress.file_name || "无"}</div>
                <div>进度: {workflowProgress.current} / {workflowProgress.total}</div>
              </div>
            )}
          </div>
        </div>
      );
    }

    // ── Step: 已完成 ──
    if (workflowStep === "done") {
      return (
        <div className={`mt-3 p-3 rounded-xl border border-emerald-500/20 bg-emerald-500/10 space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className="text-[12px] font-semibold text-emerald-400">
              变更缓存配置 · {stepLabels.done}
            </span>
          </div>

          <div className="p-3 bg-black/20 rounded-lg border border-white/5 space-y-1">
            <p className="text-[12px] text-emerald-300 font-semibold flex items-center gap-1">
              <CheckCircle className="w-3.5 h-3.5" />
              缓存路径变更操作已顺利完成！
            </p>
            <p className="text-[11px] text-slate-500 mt-1">相关目录的 Junction 映射及环境变量配置已成功更新。</p>
          </div>

          <div className="flex justify-end">
            <button onClick={closeWorkflow}
              className="px-4 py-1.5 bg-emerald-600 hover:bg-emerald-500 text-white rounded text-[11px] font-semibold cursor-pointer transition-colors">
              关闭向导
            </button>
          </div>
        </div>
      );
    }

    return null;
  };




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
      {!isAdmin && (
        <div className="flex items-start gap-2.5 p-3 rounded-xl border border-amber-500/20 bg-amber-500/10 text-[12.5px] text-amber-200">
          <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
          <span>
            <strong>权限提示：</strong>修改系统环境变量、校准 PATH 或操作冲突版本管理器需要 Windows 管理员权限。当前程序未以管理员身份运行，操作可能会因“拒绝访问（系统错误 5）”而失败。若遇到操作报错，请尝试右键以管理员身份启动 Any Version。
          </span>
        </div>
      )}

      {/* 路径类环境变量（系统管理，不可修改） */}
      <div className="space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <span className="text-xs font-semibold text-slate-300">项目关联环境变量</span>
            <span className="text-[13px] text-slate-500 ml-1.5">{vars.length} 个变量</span>
            <p className="text-[13px] text-slate-500 mt-0.5">路径类环境变量由 Kira 自动管理，不可手动修改。</p>
          </div>
          {onRepairEnv && (
            <button
              onClick={() => {
                if (!isAdmin && !window.confirm("修复系统环境变量和 PATH 需要 Windows 管理员权限。当前未以管理员身份运行，操作可能会因“拒绝访问”而失败。是否继续？")) return;
                onRepairEnv();
              }}
              disabled={isOperating || repairingEnv}
              title="重新将环境变量和 PATH 校准到 Kira links 路径"
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-amber-500/10 hover:bg-amber-500/20 disabled:opacity-50 disabled:cursor-not-allowed text-amber-300 border border-amber-500/20 text-[13px] font-semibold cursor-pointer transition-all whitespace-nowrap"
            >
              {repairingEnv ? <Loader className="w-3.5 h-3.5 animate-spin" /> : <Wrench className="w-3.5 h-3.5" />}
              修复环境变量
            </button>
          )}
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
                        <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-[12px] font-semibold">Package</span>
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
                        <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-[12px] font-semibold">用户级</span>
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
          <div className="flex items-center gap-2 text-[13px] text-slate-400 py-4"><Loader className="w-3 h-3 animate-spin text-[var(--module-accent)]" />加载中...</div>
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
                            <button onClick={() => handleSetVar(v.name, editValue)} disabled={savingVar === v.name} className="px-2 py-0.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded text-[11px] font-semibold cursor-pointer">
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

      {/* 冲突版本管理器检测与管控 (Exclusive Mode) */}
      {def && def.conflict_managers && def.conflict_managers.length > 0 && (
        <div className="border-t border-white/5 pt-5 space-y-4">
          <div>
            <div className="flex items-center gap-2">
              <AlertTriangle className="w-4 h-4 text-amber-400" />
              <h4 className="text-xs font-semibold text-slate-300">系统冲突版本管理器检测</h4>
            </div>
            <p className="text-[13px] text-slate-500 mt-0.5">
              检测到本机系统上存在以下可能会与 Kira 产生冲突的官方或第三方版本管理器。推荐通过禁用它们的环境变量或将其缓存迁移，以实现 Kira 独占。
            </p>
          </div>

          {loadingConflicts ? (
            <div className="flex items-center gap-2 text-[13px] text-slate-400 py-2">
              <Loader className="w-3.5 h-3.5 animate-spin text-[var(--module-accent)]" />正在扫描本地环境...
            </div>
          ) : conflictManagers.length === 0 ? (
            <p className="text-[13px] text-slate-500">未检测到任何冲突管理器配置。</p>
          ) : (
            <div className="space-y-4">
              {conflictManagers.map((mgr) => {
                const isOperatingMgr = operatingManagerId === mgr.id;
                
                // 判断是否是 Junction (通过路径是否被 Any Version 管控路径重定向)
                const isJunction = mgr.cache_path && mgr.cache_path.toLowerCase().includes("links\\");
                
                // 获取对应环境变量作为展示依据
                const primaryEnv = mgr.id === "rustup" ? "RUSTUP_HOME" : mgr.id === "nvm-windows" ? "NVM_HOME" : "PYENV_ROOT";
                const hasEnvConfigured = mgr.env_vars_status[primaryEnv] ? true : false;
                
                return (
                  <div key={mgr.id} className="glass-panel border border-white/5 rounded-2xl p-4 bg-white/1 space-y-4">
                    {/* 顶部标题与状态 */}
                    <div className="flex items-center justify-between pb-2 border-b border-white/3">
                      <span className="text-[14px] font-semibold text-slate-200">{mgr.display_name}</span>
                      {mgr.is_disabled ? (
                        <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[11px] font-semibold">已停用/独占</span>
                      ) : mgr.installed ? (
                        <span className="px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 text-[11px] font-semibold animate-pulse">已激活 (潜在冲突)</span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-[11px] font-semibold">未检测到运行</span>
                      )}
                    </div>

                    {/* 1. 缓存目录管理区域 */}
                    {mgr.cache_path && (
                      <div className="p-3 bg-white/2 rounded-xl border border-white/5 space-y-3">
                        <div className="flex items-center gap-1.5">
                          <HardDrive className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                          <span className="text-[12px] font-semibold text-slate-300">缓存与工具链目录管理</span>
                        </div>
                        
                        {/* 路径与大小状态 */}
                        <div className="flex items-start justify-between text-[12px] p-2.5 bg-black/20 rounded-lg border border-white/3">
                          <div className="space-y-1 flex-1 min-w-0">
                            <div className="flex items-center gap-2 flex-wrap">
                              {hasEnvConfigured ? (
                                <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-[10px] inline-flex items-center font-mono">
                                  已配置环境变量
                                </span>
                              ) : (
                                <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-[10px] inline-flex items-center">
                                  未配置环境变量
                                </span>
                              )}
                              
                              {isJunction ? (
                                <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-[10px] inline-flex items-center font-semibold">
                                  已迁移 (Junction)
                                </span>
                              ) : (
                                <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-[10px] inline-flex items-center">
                                  默认路径
                                </span>
                              )}
                            </div>
                            <p className="font-mono text-[12px] text-slate-300 break-all mt-1">{mgr.cache_path}</p>
                          </div>
                          <div className="flex items-center gap-2 flex-shrink-0 ml-3">
                            <span className="text-slate-300 font-mono text-[12px] font-semibold bg-white/5 px-2 py-0.5 rounded-md border border-white/5">
                              {mgr.cache_size}
                            </span>
                          </div>
                        </div>

                        {/* 缓存操作按钮 */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleConflictAction(mgr.id, "clean")}
                            disabled={isOperating || isOperatingMgr || workflowManagerId !== null}
                            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-red-500/10 hover:bg-red-500/20 text-red-300 border border-red-500/20 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer transition-all font-semibold text-[11px]"
                          >
                            <Trash2 className="w-3 h-3" />
                            清理缓存
                          </button>

                          <button
                            onClick={() => openWorkflow(mgr)}
                            disabled={isOperating || isOperatingMgr || workflowManagerId !== null}
                            className="flex items-center gap-1 px-2.5 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 font-semibold text-[11px] cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                          >
                            <FolderSync className="w-3 h-3" />
                            开始变更 (迁移/指向)
                          </button>
                        </div>

                        {/* 缓存变更的统一分步引导工作流面板 */}
                        {workflowManagerId === mgr.id && renderConflictWorkflow(mgr)}
                      </div>
                    )}

                    {/* 2. 冲突规避与停用区域 */}
                    <div className="p-3 bg-white/2 rounded-xl border border-white/5 space-y-3">
                      <div className="flex items-center gap-1.5">
                        <Wrench className="w-3.5 h-3.5 text-amber-400" />
                        <span className="text-[12px] font-semibold text-slate-300">冲突环境变量与 PATH 管理</span>
                      </div>

                      <div className="text-[12px] text-slate-400 space-y-1.5 bg-black/20 p-2.5 rounded-lg border border-white/3">
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
                          <span className="text-slate-500">注册表变量：</span>
                          {Object.entries(mgr.env_vars_status).map(([key, val]) => (
                            <span key={key} className="font-mono text-[11px] bg-white/3 px-1.5 py-0.5 rounded border border-white/5">
                              {key}={val ? <span className="text-slate-300 break-all select-text">"{val as string}"</span> : <span className="text-slate-600 font-sans text-[10px]">未设置</span>}
                            </span>
                          ))}
                        </div>
                        {mgr.path_status.length > 0 ? (
                          <div className="pt-1">
                            <span className="text-slate-500">在 PATH 中检测到冲突路径：</span>
                            {mgr.path_status.map((p: string) => (
                              <div key={p} className="font-mono text-[11px] text-amber-300/80 break-all select-text ml-4 mt-0.5">• {p}</div>
                            ))}
                          </div>
                        ) : (
                          <div className="text-emerald-400/80 font-semibold text-[11px] pt-1">✓ 在系统 PATH 中未检测到冲突路径</div>
                        )}
                      </div>

                      {!mgr.is_disabled ? (
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleConflictAction(mgr.id, "disable")}
                            disabled={isOperating || isOperatingMgr}
                            className="flex items-center gap-1 px-2.5 py-1.5 rounded-lg bg-orange-500/10 hover:bg-orange-500/20 text-orange-300 border border-orange-500/20 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer transition-all font-semibold text-[11px]"
                            title="注销环境变量并从系统的 PATH 中剔除它们，使该工具彻底退出生效以排除冲突"
                          >
                            <X className="w-3.5 h-3.5" />
                            一键停用 (解绑 PATH & 清理环境变量)
                          </button>
                        </div>
                      ) : (
                        <div className="text-[12px] text-emerald-400 font-semibold flex items-center gap-1 bg-emerald-500/5 p-2 rounded-lg border border-emerald-500/10">
                          <Check className="w-3.5 h-3.5 text-emerald-400" />
                          已完全解除与本机的冲突。Kira 对此项目的版本拥有独占控制权。
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════
//  服务管理
// ═══════════════════════════════════════
export function ServicesTab({ project, def, serviceCtrlLoading, onServiceToggle, onActiveSubTabChange }: SubTabProps) {
  const [isAdmin, setIsAdmin] = useState(true);

  // 当切换到 services 标签页时通知父组件
  useEffect(() => {
    onActiveSubTabChange?.("services");
    invoke<boolean>("is_admin").then(setIsAdmin).catch(() => setIsAdmin(true));
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

  const status = svc.status || (svc.running ? "running" : "stopped");
  const externallyRunning = status === "external_running" || svc.external === true;
  const hasConflict = status === "port_conflict";
  const notInstalled = status === "not_installed";
  const canToggle = !serviceCtrlLoading && !hasConflict && !notInstalled && !externallyRunning;

  return (
    <div className="space-y-4">
      {!isAdmin && svc?.system_service_name && (
        <div className="flex items-start gap-2.5 p-3 rounded-xl border border-amber-500/20 bg-amber-500/10 text-[12.5px] text-amber-200 animate-fadeIn">
          <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
          <span>
            <strong>系统服务权限提示：</strong>检测到该项目在本地注册了 Windows 系统服务（服务名: {svc.system_service_name}）。启动或停止系统服务需要 Windows 管理员权限。当前程序未以管理员身份运行，操作可能会因“拒绝访问（系统错误 5）”而失败。若遇到报错，请尝试右键以管理员身份启动 Any Version。
          </span>
        </div>
      )}

      <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
        <div className="flex items-center gap-2 border-b border-white/5 pb-3">
          <Activity className="w-4 h-4 text-[var(--module-accent)]" />
          <h4 className="text-xs font-semibold text-white">本地服务控制台</h4>
        </div>

        {externallyRunning && (
          <div className="p-3 rounded-xl border border-sky-500/20 bg-sky-500/10 text-[12px] text-sky-200 flex items-start gap-2">
            <Info className="w-4 h-4 text-sky-400 flex-shrink-0 mt-0.5" />
            <span>检测到服务正在外部运行{svc.process_name ? `（${svc.process_name}${svc.pid ? `，PID: ${svc.pid}` : ""}）` : ""}。Kira 只展示状态，不会接管或停止该外部进程。</span>
          </div>
        )}

        {hasConflict && (
          <div className="p-3 rounded-xl border border-amber-500/20 bg-amber-500/10 text-[12px] text-amber-200 flex items-start gap-2">
            <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
            <span>端口 {svc.port || def?.default_port || "未知"} 已被 {svc.process_name || "其他进程"} 占用。为避免误停外部进程，已禁用启动/停止操作。</span>
          </div>
        )}

        {notInstalled && (
          <div className="p-3 rounded-xl border border-slate-500/20 bg-slate-500/10 text-[12px] text-slate-300 flex items-start gap-2">
            <Info className="w-4 h-4 text-slate-400 flex-shrink-0 mt-0.5" />
            <span>未检测到安装目录。请在项目标题栏点击“手动指定目录”，选择已安装的服务根目录。</span>
          </div>
        )}

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-xs">
          <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-1.5">
            <span className="text-[13px] text-slate-400 font-semibold uppercase tracking-wider block">当前状态</span>
            <div className="flex items-center gap-2">
              {externallyRunning ? (
                <span className="px-2.5 py-1 rounded-lg bg-sky-500/10 text-sky-300 border border-sky-500/20 font-semibold flex items-center gap-1 animate-fadeIn">
                  <span className="w-1.5 h-1.5 rounded-full bg-sky-400 animate-ping" />
                  外部运行 {svc.pid ? `(PID: ${svc.pid})` : ""}
                </span>
              ) : svc.running ? (
                <span className="px-2.5 py-1 rounded-lg bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold flex items-center gap-1 animate-fadeIn">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-ping" />
                  运行中 {svc.pid ? `(PID: ${svc.pid})` : ""}
                </span>
              ) : hasConflict ? (
                <span className="px-2.5 py-1 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">端口冲突</span>
              ) : notInstalled ? (
                <span className="px-2.5 py-1 rounded-lg bg-slate-500/10 text-slate-400 border border-white/5 font-semibold">未配置</span>
              ) : (
                <span className="px-2.5 py-1 rounded-lg bg-slate-500/10 text-slate-400 border border-white/5 font-semibold">已停止</span>
              )}
            </div>
          </div>

          <div className="p-3 bg-black/20 rounded-xl border border-white/5 space-y-1">
            <span className="text-[13px] text-slate-400 font-semibold uppercase tracking-wider block">运行参数</span>
            <div className="text-slate-300 font-mono space-y-0.5">
              <p>端口: {svc.port || def?.default_port || "无"}</p>
              <p>进程: {svc.process_name || "未检测到"}</p>
            </div>
          </div>

          <div className="p-3 bg-black/20 rounded-xl border border-white/5 flex items-center justify-center gap-2">
            <button
              onClick={onServiceToggle}
              disabled={!canToggle}
              className={`px-4 py-2 ${svc.running ? "bg-red-600 hover:bg-red-500" : "bg-emerald-600 hover:bg-emerald-500"} disabled:opacity-50 disabled:cursor-not-allowed text-white font-semibold rounded-xl text-xs cursor-pointer shadow-md transition-all flex items-center gap-1`}
            >
              {serviceCtrlLoading ? "操作中..." : externallyRunning ? "外部运行" : svc.running ? "停止服务" : "启动服务"}
            </button>
          </div>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs pt-2">
          {svc.install_root && (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5">
              <span className="text-[13px] text-slate-400 font-semibold block">安装目录</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.install_root}>{svc.install_root}</p>
            </div>
          )}
          {svc.config_file && (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5">
              <span className="text-[13px] text-slate-400 font-semibold block">配置文件</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.config_file}>{svc.config_file}</p>
            </div>
          )}
          {svc.data_dir && (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5">
              <span className="text-[13px] text-slate-400 font-semibold block">数据目录</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.data_dir}>{svc.data_dir}</p>
            </div>
          )}
          {svc.log_dir && (
            <div className="p-3 bg-black/20 rounded-xl border border-white/5">
              <span className="text-[13px] text-slate-400 font-semibold block">日志目录</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.log_dir}>{svc.log_dir}</p>
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
  disabled = false,
  disabledReason = "",
}: {
  remoteVersions: string[];
  loadingRemote: boolean;
  installingVersion: string | null;
  isOperating?: boolean;
  onInstall: (version: string) => void;
  versionsUpdatedAt?: number | null;
  onRefresh?: () => void;
  disabled?: boolean;
  disabledReason?: string;
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
              disabled={disabled || loadingRemote || !!installingVersion}
              title={disabled ? disabledReason : "刷新版本列表"}
              className="flex items-center gap-1 px-2.5 py-1 bg-white/5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed text-slate-300 rounded-lg text-[11px] border border-white/8 cursor-pointer transition-all"
            >
              <RefreshCw className={`w-3 h-3 ${loadingRemote ? "animate-spin text-[var(--module-accent)]" : ""}`} />
              {loadingRemote ? "更新中..." : "更新列表"}
            </button>
          )}
        </div>
      </div>

      {loadingRemote && remoteVersions.length === 0 ? (
        <div className="flex items-center gap-2 text-slate-400 text-xs py-2">
          <RefreshCw className="w-4 h-4 animate-spin text-[var(--module-accent)]" />
          正在获取远程版本列表...
        </div>
      ) : (
        <div className="space-y-2">
          {disabled && disabledReason && (
            <div className="p-3 rounded-xl border border-amber-500/15 bg-amber-500/5 text-amber-400 text-[11px] mb-2 leading-relaxed flex items-center gap-1.5 animate-fadeIn">
              <AlertTriangle className="w-3.5 h-3.5 flex-shrink-0" />
              <span>{disabledReason}</span>
            </div>
          )}
          <div ref={containerRef} className="relative">
            <div className="flex items-center gap-3">
              <div className="relative flex-1">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500 pointer-events-none" />
                <input
                  type="text"
                  value={search}
                  disabled={disabled}
                  onChange={(e) => { setSearch(e.target.value); setOpen(true); }}
                  onFocus={() => setOpen(true)}
                  placeholder={disabled ? disabledReason : "输入关键词过滤版本，例如 18、LTS..."}
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
                disabled={disabled || installingVersion !== null || isOperating || !search.trim() || !remoteVersions.includes(search.trim())}
                title={disabled ? disabledReason : "一键安装"}
                className="px-5 py-2 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-xl text-xs font-semibold shadow-md shadow-[var(--module-accent-ring)] cursor-pointer transition-all flex items-center gap-1.5"
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
                    className={`w-full text-left px-3 py-1.5 text-xs hover:bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] transition-colors cursor-pointer ${search.trim() === v ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "text-slate-300"
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
        <Loader className="w-4 h-4 animate-spin text-[var(--module-accent)]" /> 正在加载旧版数据...
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
              以下数据来自 Kira 托管前的备份。取消托管时将从备份还原原始环境变量和 PATH 条目。
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
            <Globe className="w-4 h-4 text-[var(--module-accent)]" />
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

export function PackageManagerTab(props: {
  projectId: string;
  pm: PackageManagerDef;
  hidden?: boolean;
  installRoot?: string | null;
  installSource?: string | null;
  projectDef?: ProjectDef | null;
  projectStatus?: ProjectStatus | null;
}) {
  return <PackageManagerTabModular {...props} />;
}
export function DataDirsTab({ project, def, onRefresh }: { project: ProjectStatus; def?: ProjectDef | null; onRefresh: () => Promise<void> }) {
  // 当前正在变更的目录 ID
  const [workflowDirId, setWorkflowDirId] = useState<string | null>(null);
  // steps: 'method' | 'paths' | 'confirm' | 'executing' | 'done'
  const [workflowStep, setWorkflowStep] = useState<"method" | "paths" | "confirm" | "executing" | "done">("method");
  const [workflowMethod, setWorkflowMethod] = useState<"junction" | "point">("junction");
  const [workflowLinkPath, setWorkflowLinkPath] = useState("");
  const [workflowActualPath, setWorkflowActualPath] = useState("");
  const [workflowPointPath, setWorkflowPointPath] = useState("");
  const [workflowFileAction, setWorkflowFileAction] = useState<"move" | "keep">("keep");
  const [workflowExecuting, setWorkflowExecuting] = useState(false);
  const [workflowProgress, setWorkflowProgress] = useState<{ stage: string; current: number; total: number; file_name: string } | null>(null);

  // 监听迁移进度
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    if (workflowExecuting) {
      listen<{ stage: string; current: number; total: number; file_name: string }>(
        "migrate-storage-progress",
        (event) => {
          setWorkflowProgress(event.payload);
        }
      ).then((u) => {
        unlisten = u;
      });
    }
    return () => {
      if (unlisten) unlisten();
    };
  }, [workflowExecuting]);

  const closeWorkflow = () => {
    setWorkflowDirId(null);
    setWorkflowStep("method");
    setWorkflowLinkPath("");
    setWorkflowActualPath("");
    setWorkflowPointPath("");
    setWorkflowFileAction("keep");
    setWorkflowExecuting(false);
    setWorkflowProgress(null);
  };

  const openWorkflow = (dir: any) => {
    closeWorkflow();
    setWorkflowDirId(dir.id);
    setWorkflowStep("method");
    setWorkflowLinkPath(dir.path);
    if (dir.is_link && dir.real_target) {
      setWorkflowActualPath(dir.real_target);
    } else {
      const drive = dir.path.match(/^([A-Za-z]):\\/);
      if (drive && drive[1].toUpperCase() === "C") {
        const suffix = dir.path.substring(2); // Remove "C:"
        setWorkflowActualPath(`D:\\AnyVersionData\\${project.id}${suffix}`);
      } else {
        setWorkflowActualPath("");
      }
    }
    setWorkflowPointPath(dir.is_link ? "" : dir.path);
    setWorkflowFileAction("keep");
    setWorkflowExecuting(false);
    setWorkflowProgress(null);
  };

  const workflowNext = () => {
    if (workflowStep === "method") {
      setWorkflowStep("paths");
    } else if (workflowStep === "paths") {
      setWorkflowStep("confirm");
    }
  };

  const workflowPrev = () => {
    if (workflowStep === "paths") {
      setWorkflowStep("method");
    } else if (workflowStep === "confirm") {
      setWorkflowStep("paths");
    }
  };

  const browseWorkflowPath = async (setter: (v: string) => void) => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: "选择文件夹" });
      if (selected) setter(selected as string);
    } catch {
      alert("文件夹选择器不可用，请手动输入路径。");
    }
  };

  const executeWorkflow = async (dirId: string, oldPath: string, exists: boolean) => {
    if (workflowMethod === "junction" && !workflowActualPath) {
      alert("请指定目标路径");
      return;
    }
    if (workflowMethod === "point" && !workflowPointPath) {
      alert("请指定指向路径");
      return;
    }

    setWorkflowStep("executing");
    setWorkflowExecuting(true);
    setWorkflowProgress(null);

    try {
      if (workflowMethod === "junction") {
        await invoke("migrate_data_dir", {
          projectId: project.id,
          origPath: workflowLinkPath,
          newPath: workflowActualPath,
        });
      } else {
        if (exists && workflowFileAction === "move") {
          await invoke("handle_point_storage_files", {
            oldPath,
            newPath: workflowPointPath,
            action: "move",
          });
        }
        await invoke("project_set_data_dir", {
          projectId: project.id,
          dirId,
          newPath: workflowPointPath,
        });
      }
      setWorkflowStep("done");
      await onRefresh();
    } catch (e: any) {
      alert(`操作失败: ${e}`);
      setWorkflowStep("confirm");
    } finally {
      setWorkflowExecuting(false);
      setWorkflowProgress(null);
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

  const renderWorkflow = (dir: any) => {
    const accentBg = "bg-red-500/10";
    const accentBorder = "border-red-500/20";
    const accentText = "text-red-400";
    const btnBg = "bg-red-600 hover:bg-red-500";
    const progressBarColor = "bg-red-500/60";

    const stepLabels = {
      method: "选择方式",
      paths: "配置路径",
      confirm: "确认预览",
      executing: "执行中",
      done: "已完成",
    };

    const totalSteps = 4;
    const dirDef = def?.data_dirs?.find((d) => d.id === dir.id);
    const supportsDirect = !!dirDef?.supports_direct;

    // ── Step: 选择方式 ──
    if (workflowStep === "method") {
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更存储配置 · Step 1/{totalSteps} · {stepLabels.method}
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
                <p className="text-[11px] text-slate-500 mt-0.5">
                  创建一个目录链接，将目录迁移并指向新位置。文件实际存储在新位置，原路径通过链接访问。
                </p>
              </div>
            </label>
            {supportsDirect && (
              <label className={`flex items-start gap-2 p-2.5 rounded-lg cursor-pointer transition-all border ${workflowMethod === "point"
                ? `${accentBorder} bg-white/5`
                : "border-white/5 hover:bg-white/[0.02]"
                }`}>
                <input type="radio" name="wf_method" value="point" checked={workflowMethod === "point"}
                  onChange={() => setWorkflowMethod("point")} className="mt-0.5" />
                <div>
                  <span className="text-[12px] font-semibold text-purple-300">B. 指向配置</span>
                  <p className="text-[11px] text-slate-500 mt-0.5">
                    直接修改 {project.display_name} 的启动路径参数，使其指向新目录。
                  </p>
                </div>
              </label>
            )}
          </div>
          <div className="flex justify-end">
            <button onClick={() => {
              if (!supportsDirect) {
                setWorkflowMethod("junction");
              }
              setWorkflowStep("paths");
            }}
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
              变更存储配置 · Step 2/{totalSteps} · {stepLabels.paths}
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
                  <label className="text-[11px] text-slate-500 block mb-0.5">① 形式路径（链接创建位置，即原始路径）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowLinkPath} onChange={(e) => setWorkflowLinkPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[11px] font-mono" placeholder="数据源路径" />
                    <button onClick={() => browseWorkflowPath(setWorkflowLinkPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
                <div>
                  <label className="text-[11px] text-slate-500 block mb-0.5">② 实际路径（数据真实存放位置，建议选非 C 盘）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowActualPath} onChange={(e) => setWorkflowActualPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[11px] font-mono" placeholder="目标路径" />
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
                <span className="font-semibold text-purple-300">指向配置模式</span> — 直接修改 {project.display_name} 启动参数指向新路径
              </p>
              <div className="space-y-1.5">
                <div>
                  <label className="text-[11px] text-slate-500 block mb-0.5">指向路径（服务读取的数据目录）</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowPointPath} onChange={(e) => setWorkflowPointPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-[11px] font-mono"
                      placeholder="新指向路径" />
                    <button onClick={() => browseWorkflowPath(setWorkflowPointPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>

                {/* 旧文件处理方式（仅 Pointing 模式下） */}
                {dir.exists && (
                  <div className="pt-1 space-y-1">
                    <p className="text-[12px] text-slate-400 font-semibold">旧文件处理方式：</p>
                    <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "move" ? "border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)]" : "border-white/5 hover:bg-white/[0.02]"}`}>
                      <input type="radio" name="wf_file_action" value="move" checked={workflowFileAction === "move"}
                        onChange={() => setWorkflowFileAction("move")} className="mt-0.5" />
                      <div>
                        <span className="text-[12px] font-semibold text-[var(--module-accent)]">移动旧文件到新目录</span>
                        <p className="text-[10px] text-slate-500 mt-0.5">将现有文件整体复制到新位置。保留所有已有数据。</p>
                      </div>
                    </label>
                    <label className={`flex items-start gap-2 p-2 rounded-lg cursor-pointer border transition-all ${workflowFileAction === "keep" ? "border-slate-500/30 bg-slate-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
                      <input type="radio" name="wf_file_action" value="keep" checked={workflowFileAction === "keep"}
                        onChange={() => setWorkflowFileAction("keep")} className="mt-0.5" />
                      <div>
                        <span className="text-[12px] font-semibold text-slate-300">不做改动</span>
                        <p className="text-[10px] text-slate-500 mt-0.5">仅配置指向新路径，旧目录中的文件保持原样不动。</p>
                      </div>
                    </label>
                  </div>
                )}
              </div>
            </>
          )}

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
      const pathsSame = workflowMethod === "junction"
        && workflowLinkPath.toLowerCase().replace(/[\\/]+$/, "")
        === workflowActualPath.toLowerCase().replace(/[\\/]+$/, "");
      return (
        <div className={`mt-3 p-3 rounded-xl border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-[12px] font-semibold ${accentText}`}>
              变更存储配置 · Step 3/{totalSteps} · {stepLabels.confirm}
            </span>
            <button onClick={closeWorkflow} className="text-[11px] text-slate-500 hover:text-slate-300 cursor-pointer">✕ 取消</button>
          </div>

          <div className="p-3 bg-black/20 rounded-lg border border-white/5 space-y-2">
            <p className="text-[10px] text-slate-400 font-semibold uppercase tracking-wider">操作预览</p>
            <div className="space-y-1.5">
              <div className="flex items-center gap-2 text-[12px]">
                <span className={`px-1.5 py-0.5 rounded text-[11px] font-semibold ${workflowMethod === "junction" ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "bg-purple-500/10 text-purple-400"
                  }`}>
                  {workflowMethod === "junction" ? "Junction 链接" : "直接指向"}
                </span>
                {workflowMethod === "junction" ? (
                  <div className="font-mono text-slate-300 space-y-0.5 text-[11px]">
                    <p className="flex items-center gap-1">
                      <span className="text-slate-500 flex-shrink-0">原始路径：</span>
                      <span className="break-all">{workflowLinkPath}</span>
                    </p>
                    <p className="flex items-center gap-1">
                      <span className="text-[var(--module-accent)] flex-shrink-0">↓ 链接到</span>
                      <span className="text-[var(--module-accent)] break-all">{workflowActualPath}</span>
                    </p>
                  </div>
                ) : (
                  <p className="font-mono text-slate-300 text-[11px] break-all">
                    配置指向：{workflowPointPath}
                  </p>
                )}
              </div>
              {workflowMethod === "point" && dir.exists && (
                <div className="flex items-center gap-2 text-[12px]">
                  <span className="text-slate-500">旧文件处理：</span>
                  <span className={workflowFileAction === "move" ? "text-[var(--module-accent)] font-semibold" : "text-slate-400"}>
                    {workflowFileAction === "move" ? "📦 移动到新目录" : "📌 不做改动"}
                  </span>
                </div>
              )}
              {workflowMethod === "junction" && (
                <div className="flex items-center gap-2 text-[12px]">
                  <span className="text-slate-500">文件迁移：</span>
                  <span className="text-[var(--module-accent)] font-semibold">
                    {pathsSame ? "📌 直接建立链接" : "📦 移动文件并建立链接"}
                  </span>
                </div>
              )}
            </div>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-[11px] font-semibold cursor-pointer transition-colors">
              ← 上一步
            </button>
            <button onClick={() => executeWorkflow(dir.id, dir.path, dir.exists)} disabled={workflowExecuting}
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
            <Loader className="w-3.5 h-3.5 animate-spin text-[var(--module-accent)]" />
            <span className={`text-[12px] font-semibold ${accentText}`}>
              正在执行 · {workflowProgress?.stage || "准备中..."}
            </span>
          </div>
          {workflowProgress && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between text-[11px] text-slate-400">
                <span>{workflowProgress.stage}</span>
                <span className="font-mono">{workflowProgress.current}/{workflowProgress.total}</span>
              </div>
              <div className="w-full h-1 bg-white/5 rounded-full overflow-hidden">
                <div
                  className={`h-full ${progressBarColor} rounded-full transition-all duration-200`}
                  style={{ width: `${workflowProgress.total > 0 ? (workflowProgress.current / workflowProgress.total) * 100 : 0}%` }}
                />
              </div>
              {workflowProgress.file_name && (
                <p className="text-[11px] text-slate-500 truncate font-mono">{workflowProgress.file_name}</p>
              )}
            </div>
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
            存储路径已成功变更，现状已更新。
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

  const dataDirs = project.data_dirs_status || [];

  return (
    <div className="space-y-6">
      <div className="glass-panel rounded-2xl p-5 border border-white/5 bg-white/2 space-y-4">
        <div className="flex items-center gap-2">
          <HardDrive className="w-5 h-5 text-[var(--module-accent)]" />
          <div>
            <h4 className="text-sm font-semibold text-white">数据文件与数据残留管理</h4>
            <p className="text-[11px] text-slate-500 mt-0.5">扫描、迁移主数据文件或清除残留 of 旧版本数据以节省 C 盘空间。</p>
          </div>
        </div>

        {dataDirs.length === 0 ? (
          <p className="text-[13px] text-slate-400 py-2">未配置数据目录规则或未扫描到对应路径。</p>
        ) : (
          <div className="space-y-4">
            {dataDirs.map((dir) => {
              const isWorkflowActive = workflowDirId === dir.id;
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
                  {dir.exists && !isWorkflowActive && (
                    <div className="flex items-center gap-2 pt-1 border-t border-white/5">
                      <button
                        onClick={() => openWorkflow(dir)}
                        className="px-3 py-1.5 bg-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] hover:bg-[var(--module-accent)] text-white rounded-lg text-[12px] font-semibold cursor-pointer flex items-center gap-1 transition-all"
                      >
                        <FolderSync className="w-3.5 h-3.5" /> 开始变更
                      </button>
                      <button
                        onClick={() => handleDelete(dir.path)}
                        className="px-3 py-1.5 bg-red-600/10 hover:bg-red-600/20 text-red-400 rounded-lg text-[12px] font-semibold cursor-pointer flex items-center gap-1 transition-all"
                      >
                        <Trash2 className="w-3.5 h-3.5" /> 删除数据
                      </button>
                    </div>
                  )}

                  {/* 变更工作流面板 */}
                  {isWorkflowActive && renderWorkflow(dir)}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

// ═══════════════════════════════════════
//  服务参数与配置文件可视化配置
// ═══════════════════════════════════════
const globalRegistered = {
  nginxCompletions: false,
  iniCompletions: false,
  yamlCompletions: false
};

export function ConfigTab({ project, def, onRefresh }: { project: ProjectStatus; def: ProjectDef | null; onRefresh: () => Promise<void> }) {
  const [configContent, setConfigContent] = useState<string>("");
  const [loadingConfig, setLoadingConfig] = useState(false);
  const [savingConfig, setSavingConfig] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const svc: ServiceStatus | null = project.service_status ?? null;
  const configPath = svc?.config_file ?? null;
  const port = svc?.port || def?.default_port || null;

  // 加载配置文件内容
  const loadConfigContent = async () => {
    if (!configPath) return;
    setLoadingConfig(true);
    setErrorMessage(null);
    try {
      const content = await invoke<string>("read_service_config", { name: project.id });
      setConfigContent(content);
    } catch (e: any) {
      setErrorMessage(e.toString());
    } finally {
      setLoadingConfig(false);
    }
  };

  useEffect(() => {
    loadConfigContent();
  }, [project.id, configPath]);

  const handleSaveConfig = async () => {
    setSavingConfig(true);
    try {
      await invoke("write_service_config", { name: project.id, content: configContent });
      alert("配置文件已保存成功！");
      await onRefresh();
    } catch (e: any) {
      alert("保存失败: " + e);
    } finally {
      setSavingConfig(false);
    }
  };

  // Map project ID to language mode in Monaco
  const getEditorLanguage = (projectId: string): string => {
    if (configPath) {
      const lowerPath = configPath.toLowerCase();
      if (lowerPath.endsWith(".toml")) return "toml";
      if (lowerPath.endsWith(".yaml") || lowerPath.endsWith(".yml")) return "yaml";
      if (lowerPath.endsWith(".json")) return "json";
      if (lowerPath.endsWith(".conf") || lowerPath.endsWith(".nginx")) return "nginx";
    }
    switch (projectId) {
      case "mysql":
      case "redis":
      case "postgresql":
        return "ini";
      case "mongodb":
        return "yaml";
      case "nginx":
        return "nginx";
      case "frps":
      case "frpc":
        return "toml";
      default:
        return "ini";
    }
  };

  const handleEditorDidMount = (_editor: any, monaco: any) => {
    // 1. Register Nginx language if not registered
    const languages = monaco.languages.getLanguages();
    if (!languages.some((lang: any) => lang.id === 'nginx')) {
      monaco.languages.register({ id: 'nginx' });
      monaco.languages.setMonarchTokensProvider('nginx', {
        keywords: [
          'http', 'server', 'location', 'listen', 'server_name', 'root', 'index',
          'proxy_pass', 'proxy_set_header', 'try_files', 'rewrite', 'error_page',
          'ssl_certificate', 'ssl_certificate_key', 'ssl_protocols', 'ssl_ciphers',
          'access_log', 'error_log', 'client_max_body_size', 'keepalive_timeout',
          'gzip', 'gzip_types', 'upstream', 'events', 'worker_processes', 'include',
          'default_type', 'sendfile', 'tcp_nopush', 'tcp_nodelay'
        ],
        tokenizer: {
          root: [
            [/[a-zA-Z_]\w*/, { cases: { '@keywords': 'keyword', '@default': 'identifier' } }],
            [/[{}]/, 'delimiter.bracket'],
            [/[;]/, 'delimiter'],
            [/#.*$/, 'comment'],
            [/"([^"\\]|\\.)*"/, 'string'],
            [/'([^'\\]|\\.)*'/, 'string'],
            [/\d+/, 'number']
          ]
        }
      });
    }

    // 2. Register autocompletions for all configuration files to improve the experience!
    
    // Nginx autocompletions
    if (!globalRegistered.nginxCompletions) {
      monaco.languages.registerCompletionItemProvider('nginx', {
        provideCompletionItems: (_model: any, _position: any) => {
          const suggestions = [
            { label: 'server', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'server {\n\tlisten ${1:80};\n\tserver_name ${2:localhost};\n\tlocation / {\n\t\troot ${3:html};\n\t\tindex ${4:index.html};\n\t}\n}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Server block' },
            { label: 'location', kind: monaco.languages.CompletionItemKind.Keyword, insertText: 'location ${1:/} {\n\t${2:proxy_pass http://localhost:8080;}\n}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Location block' },
            { label: 'listen', kind: monaco.languages.CompletionItemKind.Property, insertText: 'listen ${1:80};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Listen port' },
            { label: 'server_name', kind: monaco.languages.CompletionItemKind.Property, insertText: 'server_name ${1:localhost};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Server name' },
            { label: 'root', kind: monaco.languages.CompletionItemKind.Property, insertText: 'root ${1:html};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Root directory' },
            { label: 'proxy_pass', kind: monaco.languages.CompletionItemKind.Property, insertText: 'proxy_pass ${1:http://localhost:8080};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Proxy pass target' },
            { label: 'gzip', kind: monaco.languages.CompletionItemKind.Property, insertText: 'gzip ${1:on};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Gzip compression' },
            { label: 'client_max_body_size', kind: monaco.languages.CompletionItemKind.Property, insertText: 'client_max_body_size ${1:50m};', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max upload size' }
          ];
          return { suggestions };
        }
      });
      globalRegistered.nginxCompletions = true;
    }

    // ini autocompletions (MySQL, Redis, PostgreSQL)
    if (!globalRegistered.iniCompletions) {
      monaco.languages.registerCompletionItemProvider('ini', {
        provideCompletionItems: (model: any, _position: any) => {
          const text = model.getValue();
          const suggestions = [];

          if (text.includes('[mysqld]') || text.includes('character-set-server') || text.includes('default-storage-engine')) {
            // MySQL completions
            suggestions.push(
              { label: 'port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'port = ${1:3306}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'MySQL Port number' },
              { label: 'max_connections', kind: monaco.languages.CompletionItemKind.Property, insertText: 'max_connections = ${1:151}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max client connections' },
              { label: 'character-set-server', kind: monaco.languages.CompletionItemKind.Property, insertText: 'character-set-server = ${1:utf8mb4}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Server default charset' },
              { label: 'default-storage-engine', kind: monaco.languages.CompletionItemKind.Property, insertText: 'default-storage-engine = INNODB', detail: 'Default database engine' },
              { label: 'innodb_buffer_pool_size', kind: monaco.languages.CompletionItemKind.Property, insertText: 'innodb_buffer_pool_size = ${1:256M}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'InnoDB buffer pool size' },
              { label: 'sql_mode', kind: monaco.languages.CompletionItemKind.Property, insertText: 'sql_mode = STRICT_TRANS_TABLES,NO_ENGINE_SUBSTITUTION', detail: 'SQL syntax modes' }
            );
          } else if (text.includes('requirepass') || text.includes('appendonly') || text.includes('maxmemory') || text.includes('redis')) {
            // Redis completions
            suggestions.push(
              { label: 'port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'port ${1:6379}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Redis Port' },
              { label: 'bind', kind: monaco.languages.CompletionItemKind.Property, insertText: 'bind 127.0.0.1', detail: 'IP binding' },
              { label: 'requirepass', kind: monaco.languages.CompletionItemKind.Property, insertText: 'requirepass ${1:password}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Redis password authentication' },
              { label: 'maxmemory', kind: monaco.languages.CompletionItemKind.Property, insertText: 'maxmemory ${1:512mb}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max memory limit' },
              { label: 'maxmemory-policy', kind: monaco.languages.CompletionItemKind.Property, insertText: 'maxmemory-policy volatile-lru', detail: 'Eviction policy' },
              { label: 'appendonly', kind: monaco.languages.CompletionItemKind.Property, insertText: 'appendonly yes', detail: 'AOF logging enablement' }
            );
          } else if (text.includes('shared_buffers') || text.includes('listen_addresses') || text.includes('logging_collector') || text.includes('postgresql')) {
            // PostgreSQL completions
            suggestions.push(
              { label: 'port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'port = ${1:5432}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'PostgreSQL Port' },
              { label: 'listen_addresses', kind: monaco.languages.CompletionItemKind.Property, insertText: 'listen_addresses = \'${1:*}\'', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Listen addresses' },
              { label: 'max_connections', kind: monaco.languages.CompletionItemKind.Property, insertText: 'max_connections = ${1:100}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Max DB connections' },
              { label: 'shared_buffers', kind: monaco.languages.CompletionItemKind.Property, insertText: 'shared_buffers = ${1:128MB}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'Shared buffer pool size' },
              { label: 'logging_collector', kind: monaco.languages.CompletionItemKind.Property, insertText: 'logging_collector = on', detail: 'Enable log collector' }
            );
          }
          return { suggestions };
        }
      });
      globalRegistered.iniCompletions = true;
    }

    // yaml autocompletions (MongoDB)
    if (!globalRegistered.yamlCompletions) {
      monaco.languages.registerCompletionItemProvider('yaml', {
        provideCompletionItems: (model: any, _position: any) => {
          const text = model.getValue();
          const suggestions = [];
          if (text.includes('dbPath') || text.includes('bindIp') || text.includes('systemLog') || text.includes('mongod')) {
            // MongoDB yaml completions
            suggestions.push(
              { label: 'storage.dbPath', kind: monaco.languages.CompletionItemKind.Property, insertText: 'storage:\n  dbPath: ${1:path}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'DB files path' },
              { label: 'net.port', kind: monaco.languages.CompletionItemKind.Property, insertText: 'net:\n  port: ${1:27017}', insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet, detail: 'MongoDB listen port' },
              { label: 'net.bindIp', kind: monaco.languages.CompletionItemKind.Property, insertText: '  bindIp: 127.0.0.1', detail: 'IP bind list' },
              { label: 'security.authorization', kind: monaco.languages.CompletionItemKind.Property, insertText: 'security:\n  authorization: enabled', detail: 'Enable MongoDB client auth' }
            );
          }
          return { suggestions };
        }
      });
      globalRegistered.yamlCompletions = true;
    }
  };

  return (
    <div className="space-y-6">
      {/* 1. 运行参数显示 */}
      {port && (
        <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
          <div className="flex items-center gap-2 border-b border-white/5 pb-3">
            <Wrench className="w-4 h-4 text-[var(--module-accent)]" />
            <h4 className="text-xs font-semibold text-white">服务运行时参数</h4>
          </div>

          <div className="flex items-center justify-between p-3 bg-black/20 rounded-xl border border-white/5">
            <div>
              <span className="text-[13px] text-slate-400 font-semibold block">服务监听端口</span>
              <span className="text-[11px] text-slate-500 mt-0.5">该参数已通过配置文件解析，如需更改请在下方编辑配置文件。</span>
            </div>
            <span className="text-slate-300 font-mono text-[13px] font-bold bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] px-3 py-1 rounded-lg">
              {port}
            </span>
          </div>
        </div>
      )}

      {/* 2. 配置文件可视化编辑 */}
      {configPath && (
        <div className="glass-panel border border-white/5 rounded-2xl p-5 bg-white/2 space-y-4">
          <div className="flex items-center justify-between border-b border-white/5 pb-3">
            <div className="flex items-center gap-2">
              <FileText className="w-4 h-4 text-[var(--module-accent)]" />
              <div>
                <h4 className="text-xs font-semibold text-white">配置文件可视化编辑</h4>
                <p className="text-[10px] text-slate-500 mt-0.5 font-mono select-all break-all" title={configPath}>
                  正在编辑: {configPath}
                </p>
              </div>
            </div>
            {!loadingConfig && !errorMessage && (
              <button
                onClick={handleSaveConfig}
                disabled={savingConfig}
                className="px-3 py-1.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded-lg text-xs font-semibold cursor-pointer flex items-center gap-1 transition-all"
              >
                {savingConfig ? <Loader className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}
                保存配置
              </button>
            )}
          </div>

          {loadingConfig ? (
            <div className="flex items-center justify-center gap-2 text-xs text-slate-400 py-12">
              <Loader className="w-4 h-4 animate-spin text-[var(--module-accent)]" /> 正在读取配置文件...
            </div>
          ) : errorMessage ? (
            <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-200 text-xs rounded-xl space-y-2">
              <p>无法加载配置文件: {errorMessage}</p>
              <button
                onClick={loadConfigContent}
                className="px-2.5 py-1 bg-white/5 hover:bg-white/10 border border-white/5 text-slate-300 rounded text-[11px] cursor-pointer"
              >
                重试加载
              </button>
            </div>
          ) : (
            <div className="space-y-3">
              <div className="border border-white/5 rounded-xl overflow-hidden h-80 bg-[#1e1e1e]">
                <MonacoEditor
                  height="100%"
                  language={getEditorLanguage(project.id)}
                  value={configContent}
                  onChange={(val) => setConfigContent(val || "")}
                  theme="vs-dark"
                  onMount={handleEditorDidMount}
                  options={{
                    minimap: { enabled: false },
                    fontSize: 13,
                    fontFamily: "Fira Code, Courier New, monospace",
                    automaticLayout: true,
                    lineNumbers: "on",
                    scrollBeyondLastLine: false,
                    wordWrap: "on",
                    padding: { top: 12, bottom: 12 }
                  }}
                />
              </div>
              <p className="text-[11px] text-slate-500">
                提示：支持智能代码高亮与自动补全提示。修改配置后需要手动重启服务才会生效。
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}


