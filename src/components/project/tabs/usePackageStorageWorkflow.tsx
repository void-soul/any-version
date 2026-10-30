import { useState } from "react";
import { theamedAlert, alertError } from "../../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckCircle, AlertTriangle, FolderOpen, Loader } from "lucide-react";
import type { PackageManagerDef } from "../types";
import type { ConfirmRequest } from "../../shared/ConfirmDialog";
import type { Dispatch, SetStateAction } from "react";

import type { PackageStorageInfo } from "./packageStorageTypes";

/** Owns the PackageStorageWorkflow feature, independently of page rendering. */
export function usePackageStorageWorkflow({ projectId, extraCacheInfos, cacheInfo, pm, dataInfo, runDetection, setConfirmRequest }: {
  projectId: string;
  extraCacheInfos: Record<string, PackageStorageInfo | undefined>;
  cacheInfo: PackageStorageInfo | null;
  pm: PackageManagerDef;
  dataInfo: PackageStorageInfo | null;
  runDetection: () => Promise<void>;
  setConfirmRequest: Dispatch<SetStateAction<ConfirmRequest | null>>;
}) {
  const { t } = useTranslation();

  // 当前工作流针对的附加缓存 id（null = 主缓存 store）
  const [workflowExtraCacheId, setWorkflowExtraCacheId] = useState<string | null>(null);

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
    setWorkflowExtraCacheId(null);
  };

  // 打开工作流（extraId: 附加缓存 id，null 表示主缓存 store）
  const openWorkflow = (type: "cache" | "data", extraId?: string) => {
    closeWorkflow();
    setWorkflowType(type);
    setWorkflowStep("method");
    setWorkflowExtraCacheId(extraId ?? null);
    // 预填默认值
    if (type === "cache") {
      const info = extraId ? extraCacheInfos[extraId] : cacheInfo;
      if (info) {
        setWorkflowLinkPath(info.path);
        if (info.real_target) {
          setWorkflowActualPath(info.real_target);
        } else {
          const drive = info.path.match(/^([A-Za-z]):\\/);
          if (drive && drive[1].toUpperCase() === "C") {
            const sub = extraId ? `${pm.id}-${extraId}` : pm.id;
            setWorkflowActualPath(`D:\\any-version-caches\\${sub}`);
          }
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
      const selected = await open({ directory: true, title: t("pkgmgr.selectFolder") });
      if (selected) setter(selected as string);
    } catch { theamedAlert(t("pkgmgr.folderPickerUnavailable")); }
  };

  /** 真正执行存储变更工作流（由确认弹窗或无需确认的路径直接调用） */
  const runWorkflowNow = async () => {
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
          extraCacheId: workflowExtraCacheId || undefined,
        });
      } else {
        // Point 模式：修改配置（仅缓存支持）
        const extra = workflowExtraCacheId ? pm.extra_caches?.find(e => e.id === workflowExtraCacheId) : null;
        const hasSetCmd = extra ? !!(extra.set_cmd_template || extra.env_var) : !!(pm.cache_set_cmd_template || pm.cache_env_var);
        if (!hasSetCmd) {
          throw new Error(t("pkgmgr.noPointSupport"));
        }
        // 处理旧文件
        const oldPath = workflowType === "cache"
          ? (workflowExtraCacheId ? extraCacheInfos[workflowExtraCacheId]?.path : cacheInfo?.path)
          : dataInfo?.path;
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
          extraCacheId: workflowExtraCacheId || undefined,
        });
      }
      await runDetection();
      setWorkflowStep("done");
    } catch (e: unknown) {
      alertError(t("pkgmgr.opFailed", { err: String(e) }));
      setWorkflowStep("confirm"); // 回到确认步骤
    } finally {
      unlisten();
      setWorkflowExecuting(false);
      setWorkflowProgress(null);
    }
  };

  /**
   * 工作流执行入口：若「移动作业」的目标与链接原路径相同，先用统一确认弹窗
   * 提示"无需移动文件、将直接创建链接"，确认后再执行。
   */
  const executeWorkflow = async () => {
    const pathsSame = workflowMethod === "junction"
      && workflowLinkPath.toLowerCase().replace(/[\\/]+$/, "")
      === workflowActualPath.toLowerCase().replace(/[\\/]+$/, "");

    if (workflowFileAction === "move" && pathsSame) {
      setConfirmRequest({
        title: t("pkgmgr.wfConfirm"),
        confirmText: t("pkgmgr.confirmExec"),
        desc: t("pkgmgr.samePathConfirm"),
        onConfirm: () => {
          void runWorkflowNow();
        },
      });
      return;
    }

    await runWorkflowNow();
  };

  // ── 工作流 UI 渲染函数 ──
  const renderWorkflow = () => {
    const isData = workflowType === "data";
    // 指向模式是否可用：按当前工作流目标（主缓存 store 或附加缓存）自身的配置能力判断
    const wfExtra = workflowExtraCacheId ? pm.extra_caches?.find(e => e.id === workflowExtraCacheId) : null;
    const wfCanPoint = !isData && !!(wfExtra
      ? (wfExtra.set_cmd_template || wfExtra.env_var)
      : (pm.cache_set_cmd_template || pm.cache_env_var));
    const accentBg = isData ? "bg-red-500/10" : "bg-amber-500/10";
    const accentBorder = isData ? "border-red-500/20" : "border-amber-500/20";
    const accentText = isData ? "text-red-400" : "text-amber-400";
    const btnBg = isData ? "bg-red-600 hover:bg-red-500" : "bg-amber-600 hover:bg-amber-500";
    const progressBarColor = isData ? "bg-red-500/60" : "bg-amber-500/60";

    const kindLabel = isData ? t("pkgmgr.kindData") : t("pkgmgr.kindCache");
    const stepLabels: Record<string, string> = {
      method: t("pkgmgr.wfMethod"),
      paths: t("pkgmgr.wfPaths"),
      confirm: t("pkgmgr.wfConfirm"),
      executing: t("pkgmgr.wfExecuting"),
      done: t("pkgmgr.wfDone"),
    };

    const totalSteps = 4;

    // ── Step: 选择方式 ──
    if (workflowStep === "method") {
      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("pkgmgr.wfStepHeader", { kind: kindLabel, step: 1, total: totalSteps, label: stepLabels.method })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">✕ {t("common.cancel")}</button>
          </div>
          <div className="space-y-1.5">
            <p className="text-body text-slate-300">{t("pkgmgr.chooseMethod")}</p>
            <label className={`flex items-start gap-2 p-2.5 rounded-ctl cursor-pointer transition-all border ${workflowMethod === "junction"
              ? `${accentBorder} bg-white/5`
              : "border-white/5 hover:bg-white/[0.02]"
              }`}>
              <input type="radio" name="wf_method" value="junction" checked={workflowMethod === "junction"}
                onChange={() => setWorkflowMethod("junction")} className="mt-0.5" />
              <div>
                <span className="text-body font-semibold text-slate-200">{t("pkgmgr.junctionTitle")}</span>
                <p className="text-title text-slate-500 mt-0.5">
                  {t("pkgmgr.junctionDesc", { kind: kindLabel })}
                </p>
              </div>
            </label>
            {wfCanPoint && (
              <label className={`flex items-start gap-2 p-2.5 rounded-ctl cursor-pointer transition-all border ${workflowMethod === "point"
                ? `${accentBorder} bg-white/5`
                : "border-white/5 hover:bg-white/[0.02]"
                }`}>
                <input type="radio" name="wf_method" value="point" checked={workflowMethod === "point"}
                  onChange={() => setWorkflowMethod("point")} className="mt-0.5" />
                <div>
                  <span className="text-body font-semibold text-[var(--module-accent)]">{t("pkgmgr.pointTitle")}</span>
                  <p className="text-title text-slate-500 mt-0.5">
                    {t("pkgmgr.pointDesc", { name: pm.display_name, kind: kindLabel })}
                  </p>
                </div>
              </label>
            )}
          </div>
          <div className="flex justify-end">
            <button onClick={workflowNext}
              className={`px-3 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors`}>
              {t("pkgmgr.next")}
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 配置路径 ──
    if (workflowStep === "paths") {
      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("pkgmgr.wfStepHeader", { kind: kindLabel, step: 2, total: totalSteps, label: stepLabels.paths })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">✕ {t("common.cancel")}</button>
          </div>

          {workflowMethod === "junction" ? (
            <>
              <p className="text-caption text-slate-400">
                <span className="font-semibold text-slate-300">{t("pkgmgr.junctionMode")}</span> — {t("pkgmgr.junctionModeDesc")}
              </p>
              <div className="space-y-1.5">
                <div>
                  <label className="text-title text-slate-500 block mb-0.5">{t("pkgmgr.linkPathLabel", { name: pm.display_name, kind: kindLabel })}</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowLinkPath} onChange={(e) => setWorkflowLinkPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-body font-mono" placeholder={t("pkgmgr.sourcePathPh", { kind: kindLabel })} />
                    <button onClick={() => browseWorkflowPath(setWorkflowLinkPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
                <div>
                  <label className="text-title text-slate-500 block mb-0.5">{t("pkgmgr.actualPathLabel")}</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowActualPath} onChange={(e) => setWorkflowActualPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-body font-mono" placeholder={t("pkgmgr.targetPathPh")} />
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
              <p className="text-caption text-slate-400">
                <span className="font-semibold text-[var(--module-accent)]">{t("pkgmgr.pointMode")}</span> — {t("pkgmgr.pointModeDesc", { name: pm.display_name })}
              </p>
              <div>
                <label className="text-title text-slate-500 block mb-0.5">{t("pkgmgr.pointPathLabel", { name: pm.display_name, kind: kindLabel })}</label>
                <div className="flex items-center gap-1">
                  <input type="text" value={workflowPointPath} onChange={(e) => setWorkflowPointPath(e.target.value)}
                    className="flex-1 glass-input px-1.5 py-1 text-body font-mono"
                    placeholder={pm.cache_default_path || t("pkgmgr.newPathPh")} />
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
            <p className="text-title text-slate-400 font-semibold">{t("pkgmgr.oldFileAction")}</p>
            {/* 移动旧文件 */}
            <label className={`flex items-start gap-2 p-2 rounded-ctl cursor-pointer border transition-all ${workflowFileAction === "move" ? "border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)]" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="move" checked={workflowFileAction === "move"}
                onChange={() => setWorkflowFileAction("move")} className="mt-0.5" />
              <div>
                <span className="text-title font-semibold text-[var(--module-accent)]">{t("pkgmgr.moveOldFiles")}</span>
                <p className="text-caption text-slate-500 mt-0.5">{t("pkgmgr.moveOldDesc", { action: workflowMethod === "junction" ? t("pkgmgr.createLink") : t("pkgmgr.modifyPoint") })}</p>
              </div>
            </label>
            {/* 删除旧文件 */}
            <label className={`flex items-start gap-2 p-2 rounded-ctl cursor-pointer border transition-all ${workflowFileAction === "delete" ? "border-red-500/30 bg-red-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="delete" checked={workflowFileAction === "delete"}
                onChange={() => setWorkflowFileAction("delete")} className="mt-0.5" />
              <div>
                <span className="text-title font-semibold text-red-300">{t("pkgmgr.deleteOldFiles")}</span>
                <p className="text-caption text-slate-500 mt-0.5">
                  {isData ? t("pkgmgr.deleteDataDesc") : t("pkgmgr.deleteCacheDesc")}
                </p>
              </div>
            </label>
            {/* 不做改动 */}
            <label className={`flex items-start gap-2 p-2 rounded-ctl cursor-pointer border transition-all ${workflowFileAction === "keep" ? "border-slate-500/30 bg-slate-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="keep" checked={workflowFileAction === "keep"}
                onChange={() => setWorkflowFileAction("keep")} className="mt-0.5" />
              <div>
                <span className="text-title font-semibold text-slate-300">{t("pkgmgr.keepOldFiles")}</span>
                <p className="text-caption text-slate-500 mt-0.5">{t("pkgmgr.keepDesc", { action: workflowMethod === "junction" ? t("pkgmgr.createLinkToDir") : t("pkgmgr.modifyPointToPath") })}</p>
              </div>
            </label>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("pkgmgr.prev")}
            </button>
            <button onClick={workflowNext}
              disabled={workflowMethod === "junction"
                ? (!workflowLinkPath || !workflowActualPath || workflowLinkPath === workflowActualPath)
                : !workflowPointPath}
              className={`px-3 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed`}>
              {t("pkgmgr.preview")}
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 确认预览 ──
    if (workflowStep === "confirm") {
      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("pkgmgr.wfStepHeader", { kind: kindLabel, step: 3, total: totalSteps, label: stepLabels.confirm })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">✕ {t("common.cancel")}</button>
          </div>

          {/* 预览卡片 */}
          <div className="p-3 bg-black/20 rounded-ctl border border-white/5 space-y-2">
            <p className="text-caption text-slate-400 font-semibold uppercase tracking-wider">{t("pkgmgr.opPreview")}</p>
            <div className="space-y-1.5">
              <div className="flex items-center gap-2 text-body">
                <span className={`px-1.5 py-0.5 rounded text-title font-semibold ${workflowMethod === "junction" ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "bg-[var(--module-accent-soft)] text-[var(--module-accent)]"
                  }`}>
                  {workflowMethod === "junction" ? "Junction" : t("pkgmgr.point")}
                </span>
                {workflowMethod === "junction" ? (
                  <div className="font-mono text-slate-300 space-y-0.5">
                    <p className="flex items-center gap-1">
                      <span className="text-title text-slate-500 flex-shrink-0">{t("pkgmgr.linkPath")}</span>
                      <span className="text-caption break-all">{workflowLinkPath}</span>
                    </p>
                    <p className="flex items-center gap-1">
                      <span className="text-title text-[var(--module-accent)] flex-shrink-0">{t("pkgmgr.linkTo")}</span>
                      <span className="text-caption text-[var(--module-accent)] break-all">{workflowActualPath}</span>
                    </p>
                  </div>
                ) : (
                  <p className="font-mono text-slate-300 text-body break-all">
                    {t("pkgmgr.pointTo")} {workflowPointPath || t("pkgmgr.notSet")}
                  </p>
                )}
              </div>
              <div className="flex items-center gap-2 text-body">
                <span className="text-slate-500">{t("pkgmgr.oldFileHandling")}</span>
                <span className={
                  workflowFileAction === "delete" ? "text-red-400 font-semibold" :
                    workflowFileAction === "move" ? "text-[var(--module-accent)] font-semibold" :
                      "text-slate-400"
                }>
                  {workflowFileAction === "delete" ? t("pkgmgr.deleteOldShort") :
                    workflowFileAction === "move" ? t("pkgmgr.moveToNewDir") : t("pkgmgr.keepShort")}
                </span>
              </div>
              {(workflowMethod === "junction" && workflowLinkPath.toLowerCase().startsWith("c:")) && (
                <p className="text-title text-red-400/80 flex items-center gap-1">
                  <AlertTriangle className="w-2.5 h-2.5" />{t("pkgmgr.cDriveWarn")}
                </p>
              )}
            </div>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("pkgmgr.prev")}
            </button>
            <button onClick={workflowNext} disabled={workflowExecuting}
              className={`px-3 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors disabled:opacity-40`}>
              {t("pkgmgr.confirmExec")}
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 执行中 ──
    if (workflowStep === "executing") {
      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center gap-2">
            <Loader className="w-3.5 h-3.5 animate-spin text-[var(--module-accent)]" />
            <span className={`text-body font-semibold ${accentText}`}>
              {t("pkgmgr.executing", { stage: workflowProgress?.stage || t("pkgmgr.preparing") })}
            </span>
          </div>
          {workflowProgress && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between text-title text-slate-400">
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
                <p className="text-title text-slate-500 truncate font-mono">{workflowProgress.file_name}</p>
              )}
            </div>
          )}
          {!workflowProgress && (
            <p className="text-caption text-slate-400 flex items-center gap-1">
              <Loader className="w-3 h-3 animate-spin" />{t("pkgmgr.starting")}
            </p>
          )}
        </div>
      );
    }

    // ── Step: 完成 ──
    if (workflowStep === "done") {
      return (
        <div className={`mt-3 p-3 rounded-card border border-emerald-500/20 bg-emerald-500/5 space-y-3 animate-fadeIn`}>
          <div className="flex items-center gap-2">
            <CheckCircle className="w-4 h-4 text-emerald-400" />
            <span className="text-body font-semibold text-emerald-300">{t("pkgmgr.opSuccess")}</span>
          </div>
          <p className="text-caption text-emerald-400/70">
            {t("pkgmgr.opDone", { kind: kindLabel, action: workflowMethod === "junction" ? t("pkgmgr.migrate") : t("pkgmgr.reconfig") })}
          </p>
          <div className="flex justify-end">
            <button onClick={closeWorkflow}
              className="px-3 py-1 bg-emerald-600/50 hover:bg-emerald-600 text-white rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("common.close")}
            </button>
          </div>
        </div>
      );
    }

    return null;
  };
  return {
    workflowExtraCacheId,
    workflowType,
    openWorkflow,
    renderWorkflow,
  };
}
