import { useState, useEffect } from "react";
import { theamedAlert, alertError } from "../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckCircle, Trash2, HardDrive, FolderOpen, FolderSync, Loader, RotateCcw } from "lucide-react";
import type { ProjectStatus, ProjectDef } from "./types";
import { ConfirmDialogHost } from "../shared/ConfirmDialog";
import type { ConfirmRequest } from "../shared/ConfirmDialog";
export function DataDirsTab({ project, def, onRefresh }: { project: ProjectStatus; def?: ProjectDef | null; onRefresh: () => Promise<void> }) {
  const { t } = useTranslation();
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
  // 统一确认弹窗请求（替代原生 window.confirm）
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);

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
      const selected = await open({ directory: true, title: t("projsub.pickFolder") });
      if (selected) setter(selected as string);
    } catch {
      theamedAlert(t("projsub.pickerUnavailable"));
    }
  };

  const executeWorkflow = async (dirId: string, oldPath: string, exists: boolean) => {
    if (workflowMethod === "junction" && !workflowActualPath) {
      theamedAlert(t("projsub.needTargetPath"));
      return;
    }
    if (workflowMethod === "point" && !workflowPointPath) {
      theamedAlert(t("projsub.needPointPath"));
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
      alertError(t("projsub.opFail", { err: String(e) }));
      setWorkflowStep("confirm");
    } finally {
      setWorkflowExecuting(false);
      setWorkflowProgress(null);
    }
  };

  /**
   * 清除记录的自定义路径，回到默认位置。
   * 后端 `project_set_data_dir` 传空路径 = 删除 config.custom_data_paths 里的这条记录；
   * 目录内容一个字节都不动，只是不再被 Kira 当作「自定义路径」展示 ——
   * 用于纠正「指向」模式下记录的、工具其实不会去读的路径（如 scoop 这类没有启动器的工具）。
   */
  const handleResetCustomPath = async (dir: { id: string }) => {
    try {
      await invoke("project_set_data_dir", { projectId: project.id, dirId: dir.id, newPath: "" });
      theamedAlert(t("projsub.resetCustomPathDone"));
      await onRefresh();
    } catch (e: any) {
      alertError(t("projsub.opFail", { err: String(e) }));
    }
  };

  /** 真正删除数据目录（由确认弹窗回调触发） */
  const doDelete = async (path: string) => {
    try {
      await invoke("delete_data_dir", {
        projectId: project.id,
        path,
      });
      theamedAlert(t("projsub.deleteSuccess"));
      await onRefresh();
    } catch (e: unknown) {
      alertError(t("projsub.deleteFail", { err: String(e) }));
    }
  };

  /**
   * 删除数据目录确认：原来这里是连续两次原生 confirm（"确认删除"→"再次确认"），
   * 改用统一确认弹窗，把两段警告一次性摆在同一个 danger 弹窗里。
   */
  const handleDelete = (path: string) => {
    setConfirmRequest({
      title: t("projsub.deleteData"),
      danger: true,
      width: 460,
      confirmText: t("projsub.deleteData"),
      desc: (
        <div className="space-y-2.5">
          <p className="whitespace-pre-line">{t("projsub.warnDeleteDir", { path })}</p>
          <p className="text-red-300 font-semibold whitespace-pre-line">{t("projsub.confirmDeleteDir", { path })}</p>
        </div>
      ),
      onConfirm: () => {
        void doDelete(path);
      },
    });
  };

  const renderWorkflow = (dir: any) => {
    const accentBg = "bg-red-500/10";
    const accentBorder = "border-red-500/20";
    const accentText = "text-red-400";
    const btnBg = "bg-red-600 hover:bg-red-500";
    const progressBarColor = "bg-red-500/60";

    const stepLabels = {
      method: t("projsub.stepLabelsMethod"),
      paths: t("projsub.stepLabelsPaths"),
      confirm: t("projsub.stepLabelsConfirm"),
      executing: t("projsub.stepLabelsExec"),
      done: t("projsub.stepLabelsDone"),
    };

    const totalSteps = 4;
    const dirDef = def?.data_dirs?.find((d) => d.id === dir.id);
    const supportsDirect = !!dirDef?.supports_direct;

    // ── Step: 选择方式 ──
    if (workflowStep === "method") {
      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("projsub.wfStoreTitle", { step: 1, total: totalSteps, label: stepLabels.method })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">{t("projsub.cancel")}</button>
          </div>
          <div className="space-y-1.5">
            <p className="text-body text-slate-300">{t("projsub.pickStoreMethod")}</p>
            <label className={`flex items-start gap-2 p-2.5 rounded-ctl cursor-pointer transition-all border ${workflowMethod === "junction"
              ? `${accentBorder} bg-white/5`
              : "border-white/5 hover:bg-white/[0.02]"
              }`}>
              <input type="radio" name="wf_method" value="junction" checked={workflowMethod === "junction"}
                onChange={() => setWorkflowMethod("junction")} className="mt-0.5" />
              <div>
                <span className="text-body font-semibold text-slate-200">{t("projsub.optAStore")}</span>
                <p className="text-caption text-slate-500 mt-0.5">
                  {t("projsub.optAStoreDesc")}
                </p>
              </div>
            </label>
            {supportsDirect && (
              <label className={`flex items-start gap-2 p-2.5 rounded-ctl cursor-pointer transition-all border ${workflowMethod === "point"
                ? `${accentBorder} bg-white/5`
                : "border-white/5 hover:bg-white/[0.02]"
                }`}>
                <input type="radio" name="wf_method" value="point" checked={workflowMethod === "point"}
                  onChange={() => setWorkflowMethod("point")} className="mt-0.5" />
                <div>
                  <span className="text-body font-semibold text-[var(--module-accent)]">{t("projsub.optBStore")}</span>
                  <p className="text-caption text-slate-500 mt-0.5">
                    {t("projsub.optBStoreDesc", { name: project.display_name })}
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
              className={`px-3 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors`}>
              {t("projsub.nextStep")}
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
              {t("projsub.wfStoreTitle", { step: 2, total: totalSteps, label: stepLabels.paths })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">{t("projsub.cancel")}</button>
          </div>

          {workflowMethod === "junction" ? (
            <>
              <p className="text-caption text-slate-400">
                <span className="font-semibold text-slate-300">{t("projsub.storeJunctionDesc")}</span> — {t("projsub.storeLinkFrom")} → {t("projsub.storeLinkTo")}
              </p>
              <div className="space-y-1.5">
                <div>
                  <label className="text-caption text-slate-500 block mb-0.5">{t("projsub.storeLinkFrom")}</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowLinkPath} onChange={(e) => setWorkflowLinkPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-caption font-mono" placeholder={t("projsub.storeLinkFromPh")} />
                    <button onClick={() => browseWorkflowPath(setWorkflowLinkPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
                <div>
                  <label className="text-caption text-slate-500 block mb-0.5">{t("projsub.storeLinkTo")}</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowActualPath} onChange={(e) => setWorkflowActualPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-caption font-mono" placeholder={t("projsub.storeLinkToPh")} />
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
                <span className="font-semibold text-[var(--module-accent)]">{t("projsub.storePointDesc")}</span> {t("projsub.storePointHint", { name: project.display_name })}
              </p>
              <div className="space-y-1.5">
                <div>
                  <label className="text-caption text-slate-500 block mb-0.5">{t("projsub.storePointLabel")}</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowPointPath} onChange={(e) => setWorkflowPointPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-caption font-mono"
                      placeholder={t("projsub.storePointPh")} />
                    <button onClick={() => browseWorkflowPath(setWorkflowPointPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>

                {/* 旧文件处理方式（仅 Pointing 模式下） */}
                {dir.exists && (
                  <div className="pt-1 space-y-1">
                    <p className="text-body text-slate-400 font-semibold">{t("projsub.storeOldLabel")}</p>
                    <label className={`flex items-start gap-2 p-2 rounded-ctl cursor-pointer border transition-all ${workflowFileAction === "move" ? "border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)]" : "border-white/5 hover:bg-white/[0.02]"}`}>
                      <input type="radio" name="wf_file_action" value="move" checked={workflowFileAction === "move"}
                        onChange={() => setWorkflowFileAction("move")} className="mt-0.5" />
                      <div>
                        <span className="text-body font-semibold text-[var(--module-accent)]">{t("projsub.storeMoveOld")}</span>
                        <p className="text-tiny text-slate-500 mt-0.5">{t("projsub.storeMoveOldDesc")}</p>
                      </div>
                    </label>
                    <label className={`flex items-start gap-2 p-2 rounded-ctl cursor-pointer border transition-all ${workflowFileAction === "keep" ? "border-slate-500/30 bg-slate-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
                      <input type="radio" name="wf_file_action" value="keep" checked={workflowFileAction === "keep"}
                        onChange={() => setWorkflowFileAction("keep")} className="mt-0.5" />
                      <div>
                        <span className="text-body font-semibold text-slate-300">{t("projsub.storeKeepOld")}</span>
                        <p className="text-tiny text-slate-500 mt-0.5">{t("projsub.storeKeepOldDesc")}</p>
                      </div>
                    </label>
                  </div>
                )}
              </div>
            </>
          )}

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("projsub.prevStep")}
            </button>
            <button onClick={workflowNext}
              disabled={workflowMethod === "junction"
                ? (!workflowLinkPath || !workflowActualPath || workflowLinkPath === workflowActualPath)
                : !workflowPointPath}
              className={`px-3 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed`}>
              {t("projsub.preview")}
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
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("projsub.wfStoreTitle", { step: 3, total: totalSteps, label: stepLabels.confirm })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">{t("projsub.cancel")}</button>
          </div>

          <div className="p-3 bg-black/20 rounded-ctl border border-white/5 space-y-2">
            <p className="text-tiny text-slate-400 font-semibold uppercase tracking-wider">{t("projsub.storePreviewOp")}</p>
            <div className="space-y-1.5">
              <div className="flex items-center gap-2 text-body">
                <span className={`px-1.5 py-0.5 rounded text-caption font-semibold ${workflowMethod === "junction" ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "bg-[var(--module-accent-soft)] text-[var(--module-accent)]"
                  }`}>
                  {workflowMethod === "junction" ? t("projsub.storeModeJunction") : t("projsub.storeModePoint")}
                </span>
                {workflowMethod === "junction" ? (
                  <div className="font-mono text-slate-300 space-y-0.5 text-caption">
                    <p className="flex items-center gap-1">
                      <span className="text-slate-500 flex-shrink-0">{t("projsub.storeOrigPath")}</span>
                      <span className="break-all">{workflowLinkPath}</span>
                    </p>
                    <p className="flex items-center gap-1">
                      <span className="text-[var(--module-accent)] flex-shrink-0">{t("projsub.storeLinkTo")}</span>
                      <span className="text-[var(--module-accent)] break-all">{workflowActualPath}</span>
                    </p>
                  </div>
                ) : (
                  <p className="font-mono text-slate-300 text-caption break-all">
                    {t("projsub.storePointTo", { path: workflowPointPath })}
                  </p>
                )}
              </div>
              {workflowMethod === "point" && dir.exists && (
                <div className="flex items-center gap-2 text-body">
                  <span className="text-slate-500">{t("projsub.storeOldHandle")}</span>
                  <span className={workflowFileAction === "move" ? "text-[var(--module-accent)] font-semibold" : "text-slate-400"}>
                    {workflowFileAction === "move" ? t("projsub.storeMoveToNew") : t("projsub.storeKeep")}
                  </span>
                </div>
              )}
              {workflowMethod === "junction" && (
                <div className="flex items-center gap-2 text-body">
                  <span className="text-slate-500">{t("projsub.storeMigrate")}</span>
                  <span className="text-[var(--module-accent)] font-semibold">
                    {!dir.exists
                      ? t("projsub.storeCreateLink")
                      : pathsSame
                        ? t("projsub.storeDirectLink")
                        : t("projsub.storeMoveLink")}
                  </span>
                </div>
              )}
            </div>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("projsub.prevStep")}
            </button>
            <button onClick={() => executeWorkflow(dir.id, dir.path, dir.exists)} disabled={workflowExecuting}
              className={`px-3 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors disabled:opacity-40`}>
              {t("projsub.storeConfirmExec")}
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
              {t("projsub.storeExecuting", { stage: workflowProgress?.stage || t("projsub.storePreparing") })}
            </span>
          </div>
          {workflowProgress && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between text-caption text-slate-400">
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
                <p className="text-caption text-slate-500 truncate font-mono">{workflowProgress.file_name}</p>
              )}
            </div>
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
            <span className="text-body font-semibold text-emerald-300">{t("projsub.storeSuccess")}</span>
          </div>
          <p className="text-caption text-emerald-400/70">
            {t("projsub.storeSuccessDesc")}
          </p>
          <div className="flex justify-end">
            <button onClick={closeWorkflow}
              className="px-3 py-1 bg-emerald-600/50 hover:bg-emerald-600 text-white rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("projsub.storeClose")}
            </button>
          </div>
        </div>
      );
    }

    return null;
  };

  const dataDirs = project.data_dirs_status || [];

  // 目录类型（data / log / config / cache）徽章文案
  const dirKindLabel = (kind?: string | null): string | null => {
    switch (kind) {
      case "data": return t("projsub.dirKindData");
      case "log": return t("projsub.dirKindLog");
      case "config": return t("projsub.dirKindConfig");
      case "cache": return t("projsub.dirKindCache");
      default: return null;
    }
  };

  // 路径来源徽章文案。取值由后端 resolve_data_dir 给出：
  // custom（用户改过）/ config（读自服务配置文件）/ env:VAR / detected（探测到已存在）/ default（默认模板）
  const dirSourceLabel = (source?: string | null): string | null => {
    if (!source) return null;
    if (source.startsWith("env:")) return t("projsub.dirSourceEnv", { name: source.slice(4) });
    switch (source) {
      case "custom": return t("projsub.dirSourceCustom");
      case "config": return t("projsub.dirSourceConfig");
      case "detected": return t("projsub.dirSourceDetected");
      case "default": return t("projsub.defaultPath");
      default: return source;
    }
  };

  return (
    <div className="space-y-6">
      <div className="glass-panel rounded-panel p-5 border border-white/5 bg-white/2 space-y-4">
        <div className="flex items-center gap-2">
          <HardDrive className="w-5 h-5 text-[var(--module-accent)]" />
          <div>
            <h4 className="text-sm font-semibold text-white">{t("projsub.dataFilesTitle")}</h4>
            <p className="text-caption text-slate-500 mt-0.5">{t("projsub.dataFilesDesc")}</p>
          </div>
        </div>

        {dataDirs.length === 0 ? (
          <p className="text-title text-slate-400 py-2">{t("projsub.noDataRules")}</p>
        ) : (
          <div className="space-y-4">
            {dataDirs.map((dir) => {
              const isWorkflowActive = workflowDirId === dir.id;
              const dirDef = def?.data_dirs?.find((d) => d.id === dir.id);
              const kindLabel = dirKindLabel(dir.kind);
              const sourceLabel = dirSourceLabel(dir.source);
              return (
                <div key={dir.id + "_" + dir.path} className="p-4 bg-black/20 rounded-card border border-white/5 space-y-3 animate-fadeIn">
                  <div className="flex items-start justify-between">
                    <div className="space-y-1">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="text-title font-semibold text-white">{dir.display_name}</span>
                        {kindLabel && (
                          <span className="text-tiny px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] font-semibold">
                            {kindLabel}
                          </span>
                        )}
                        {dir.is_link && (
                          <span className="text-tiny px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold">
                            {t("projsub.migratedJunction")}
                          </span>
                        )}
                        {!dir.exists && (
                          <span className="text-tiny px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20">
                            {t("projsub.noPathFound")}
                          </span>
                        )}
                      </div>
                      <p className="font-mono text-body text-slate-400 break-all">{dir.path}</p>
                      {dir.is_link && dir.real_target && (
                        <p className="font-mono text-caption text-slate-500 break-all">
                          {t("projsub.realTarget", { path: dir.real_target })}
                        </p>
                      )}
                      {sourceLabel && (
                        <p className="text-caption text-slate-500">
                          {t("projsub.dirSource")}{" "}
                          <span className="text-slate-400">{sourceLabel}</span>
                        </p>
                      )}
                      {!dir.exists && dirDef?.required_for_start && !dirDef?.auto_create && (
                        <p className="text-caption text-amber-400/80">{t("projsub.dirMustExist")}</p>
                      )}
                      {!dir.exists && dirDef?.auto_create && (
                        <p className="text-caption text-slate-500">{t("projsub.dirAutoCreate")}</p>
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-slate-300 font-mono text-title font-semibold bg-white/5 px-2.5 py-1 rounded-ctl">
                        {dir.size}
                      </span>
                    </div>
                  </div>

                  {/* 操作按钮区：目录还不存在时也要能设置位置（如 scoop 的全局安装目录，
                      没装过 -g 软件就一直不存在）——「设置位置」会建出空目录再挂 Junction；
                      没有内容可删，所以「删除数据」只在目录存在时出现。 */}
                  {!isWorkflowActive && (
                    <div className="flex items-center gap-2 pt-1 border-t border-white/5">
                      <button
                        onClick={() => openWorkflow(dir)}
                        className="px-3 py-1.5 bg-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] hover:bg-[var(--module-accent)] text-white rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-all"
                      >
                        <FolderSync className="w-3.5 h-3.5" /> {dir.exists ? t("projsub.startChangeBtn") : t("projsub.setLocationBtn")}
                      </button>
                      {dir.exists && (
                        <button
                          onClick={() => handleDelete(dir.path)}
                          className="px-3 py-1.5 bg-red-600/10 hover:bg-red-600/20 text-red-400 rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-all"
                        >
                          <Trash2 className="w-3.5 h-3.5" /> {t("projsub.deleteData")}
                        </button>
                      )}
                      {/* 「指向」模式给没有启动器的工具（scoop / winget）记录过路径时，
                          那个路径工具其实不会读；给一个出口回到默认位置 + Junction 流程 */}
                      {dir.source === "custom" && (
                        <button
                          onClick={() => handleResetCustomPath(dir)}
                          title={t("projsub.resetCustomPathHint")}
                          className="px-3 py-1.5 bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-all"
                        >
                          <RotateCcw className="w-3.5 h-3.5" /> {t("projsub.resetCustomPath")}
                        </button>
                      )}
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

      {/* 统一确认弹窗：删除数据目录（不可逆，danger） */}
      {<ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />}
    </div>
  );
}
