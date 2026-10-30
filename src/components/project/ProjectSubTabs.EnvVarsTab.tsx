import { ConflictMethodStep, ConflictProgressStep, ConflictDoneStep } from "./ProjectConflictSteps";
import { useState, useEffect } from "react";
import { theamedAlert, alertError } from "../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckCircle, AlertTriangle, RefreshCw, Check, Trash2, HardDrive, FolderOpen, FolderSync, Loader, X, Wrench } from "lucide-react";
import type { EnvVarStatus, ManagedPathEntry } from "./types";
import { ConfirmDialogHost } from "../shared/ConfirmDialog";
import type { ConfirmRequest } from "../shared/ConfirmDialog";
import { type SubTabProps } from "./ProjectSubTabs.model";
// ═══════════════════════════════════════
//  环境变量
// ═══════════════════════════════════════
export function EnvVarsTab({ project, def, onActiveSubTabChange, isOperating, repairingEnv, onRepairEnv, onRefresh }: SubTabProps) {
  const { t } = useTranslation();
  const vars: EnvVarStatus[] = project.env_vars_status ?? [];
  // Kira 写入用户 PATH 的条目（托管时写入、取消托管时撤下），与环境变量同属托管面。
  // 后端口径：只要 Kira 设置过且仍在用户 PATH 就返回；非完全托管时可能是残留条目。
  const managedPaths: ManagedPathEntry[] = project.managed_path_entries ?? [];
  const fullyManaged = project.managed && !project.is_simple_managed;
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

  // 统一确认弹窗请求（替代原生 window.confirm）
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);

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
      const selected = await open({ directory: true, title: t("projsub.pickFolder") });
      if (selected) setter(selected as string);
    } catch {
      theamedAlert(t("projsub.pickerUnavailable"));
    }
  };

  /** 真正执行缓存变更（由确认弹窗或无需确认的路径直接调用） */
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
      alertError(t("projsub.opFail", { err: String(e) }));
      setWorkflowStep("confirm");
    } finally {
      unlisten();
      setWorkflowExecuting(false);
      setWorkflowProgress(null);
    }
  };

  /**
   * 工作流执行入口：先做预检（非管理员权限 / 移动到与链接相同的目录），
   * 有风险点时用统一确认弹窗一次性列清，确认后再真正执行。
   * （原来这里是两次原生 window.confirm，弹窗被系统样式接管且无法逐条展示。）
   */
  const executeWorkflow = async () => {
    if (!workflowManagerId) return;

    const warnings: string[] = [];
    if (!isAdmin) warnings.push(t("projsub.migrateAdminWarn"));

    const pathsSame = workflowMethod === "junction"
      && workflowLinkPath.toLowerCase().replace(/[\\/]+$/, "")
      === workflowActualPath.toLowerCase().replace(/[\\/]+$/, "");
    if (workflowFileAction === "move" && pathsSame) warnings.push(t("projsub.samePathConfirm"));

    if (warnings.length > 0) {
      setConfirmRequest({
        title: t("projsub.workflowConfirmTitle"),
        confirmText: t("projsub.storeConfirmExec"),
        desc: (
          <div className="space-y-2">
            {warnings.map((w, i) => (
              <p key={i}>{w}</p>
            ))}
          </div>
        ),
        onConfirm: () => {
          void runWorkflowNow();
        },
      });
      return;
    }

    await runWorkflowNow();
  };

  // 环境变量层级文案（与后端 DisableVarAction.level 对应）
  const levelLabel = (level: string) => {
    switch (level) {
      case "user": return t("projsub.levelUser");
      case "system": return t("projsub.levelSystem");
      case "both": return t("projsub.levelBoth");
      default: return t("projsub.notSet");
    }
  };

  /**
   * 「一键停用」确认弹窗内容（结构化列出影响面）。
   *
   * 停用是不可逆操作（原值不归 Kira 备份管），系统级 PATH 条目删掉后尤其难找回，
   * 因此把后端预案（preview_conflict_manager_disable）原样摊开给用户逐条确认，
   * 而不是只用一句"确定吗"。
   */
  const renderDisablePlan = (plan: any) => {
    const vars: any[] = plan.variables || [];
    const clearVars = vars.filter((v) => v.action === "clear");
    const keptVars = vars.filter((v) => v.action === "keep_managed");
    const up: string[] = plan.user_path_entries || [];
    const sp: string[] = plan.system_path_entries || [];

    return (
      <div className="space-y-3">
        <p className="text-slate-300">{t("projsub.disableConfirmTitle", { name: plan.manager_display_name })}</p>

        <div className="space-y-0.5">
          <p className="text-amber-300 font-semibold">
            {t("projsub.disableConfirmVars", { count: clearVars.length })}
          </p>
          {clearVars.map((v) => (
            <p key={v.name} className="font-mono text-caption break-all pl-2">
              • {v.name}
              <span className="text-slate-500">　[{levelLabel(v.level)}]　{v.current_value ?? t("projsub.notSet")}</span>
            </p>
          ))}
        </div>

        {keptVars.length > 0 && (
          <div className="space-y-0.5">
            <p className="text-emerald-300 font-semibold">{t("projsub.disableConfirmKept")}</p>
            <p className="font-mono text-caption break-all pl-2">
              • {keptVars.map((v) => v.name).join("　• ")}
            </p>
          </div>
        )}

        <div className="space-y-0.5">
          <p className="text-slate-300 font-semibold">
            {t("projsub.disableConfirmPathUser", { count: up.length })}
          </p>
          {up.map((p) => (
            <p key={p} className="font-mono text-caption break-all pl-2 text-slate-400">• {p}</p>
          ))}
        </div>

        <div className="space-y-0.5">
          <p className="text-slate-300 font-semibold">
            {t("projsub.disableConfirmPathSystem", { count: sp.length })}
          </p>
          {sp.map((p) => (
            <p key={p} className="font-mono text-caption break-all pl-2 text-slate-400">• {p}</p>
          ))}
        </div>

        <p className="text-slate-500 pt-1 border-t border-white/5">{t("projsub.disableConfirmBackup")}</p>
      </div>
    );
  };

  /** 真正执行冲突管理器操作（由按钮或停用确认弹窗触发） */
  const runConflictAction = async (managerId: string, action: string) => {
    setOperatingManagerId(managerId);
    try {
      await invoke("handle_conflict_manager_action", {
        sdkId: project.id,
        managerId,
        action,
        targetPath: null
      });
      theamedAlert(t("projsub.opSuccess"));
      await loadConflictManagers();
      onRefresh();
    } catch (e: any) {
      alertError(t("projsub.opFail", { err: String(e) }));
    } finally {
      setOperatingManagerId(null);
    }
  };

  const handleConflictAction = async (managerId: string, action: string) => {
    if (action === "disable") {
      // 先取预案（只读、无副作用），再弹确认弹窗——管理员态同样要确认。
      let plan: any;
      try {
        plan = await invoke<any>("preview_conflict_manager_disable", {
          sdkId: project.id,
          managerId,
        });
      } catch (e: any) {
        alertError(t("projsub.opFail", { err: String(e) }));
        return;
      }
      setConfirmRequest({
        title: t("projsub.deactivateTitle2"),
        danger: true,
        width: 520,
        confirmText: t("projsub.disableConfirmOk"),
        desc: renderDisablePlan(plan),
        onConfirm: () => {
          void runConflictAction(managerId, "disable");
        },
      });
      return;
    }
    if (!isAdmin) {
      setConfirmRequest({
        title: t("projsub.conflictOpsTitle"),
        confirmText: t("common.confirm"),
        desc: t("projsub.conflictOpsWarn", { managerId }),
        onConfirm: () => {
          void runConflictAction(managerId, action);
        },
      });
      return;
    }
    await runConflictAction(managerId, action);
  };

  const renderConflictWorkflow = (mgr: any) => {
    const accentBg = "bg-amber-500/10";
    const accentBorder = "border-amber-500/20";
    const accentText = "text-amber-400";
    const btnBg = "bg-amber-600 hover:bg-amber-500";

    const stepLabels: Record<string, string> = {
      method: t("projsub.stepLabelsMethod"),
      paths: t("projsub.stepLabelsPaths"),
      confirm: t("projsub.stepLabelsConfirm"),
      executing: t("projsub.stepLabelsExec"),
      done: t("projsub.stepLabelsDone"),
    };

    const totalSteps = 4;

    // ── Step: 选择方式 ──
    if (workflowStep === "method") return <ConflictMethodStep method={workflowMethod} onMethodChange={setWorkflowMethod} onNext={workflowNext} onClose={closeWorkflow} />;

    // ── Step: 配置路径 ──
    if (workflowStep === "paths") {
      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("projsub.wfTitle", { step: 2, total: totalSteps, label: stepLabels.paths })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">{t("projsub.cancel")}</button>
          </div>

          {workflowMethod === "junction" ? (
            <>
              <p className="text-caption text-slate-400">
                <span className="font-semibold text-slate-300">{t("projsub.junctionModeDesc")}</span> — {t("projsub.linkFrom")} → {t("projsub.linkTo")}
              </p>
              <div className="space-y-1.5">
                <div>
                  <label className="text-title text-slate-500 block mb-0.5">{t("projsub.linkFrom")}</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowLinkPath} onChange={(e) => setWorkflowLinkPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-body font-mono" placeholder={t("projsub.linkFromPh")} />
                    <button onClick={() => browseWorkflowPath(setWorkflowLinkPath)}
                      className="p-1 bg-white/5 hover:bg-white/10 text-slate-400 rounded border border-white/5 cursor-pointer">
                      <FolderOpen className="w-3 h-3" />
                    </button>
                  </div>
                </div>
                <div>
                  <label className="text-title text-slate-500 block mb-0.5">{t("projsub.linkTo")}</label>
                  <div className="flex items-center gap-1">
                    <input type="text" value={workflowActualPath} onChange={(e) => setWorkflowActualPath(e.target.value)}
                      className="flex-1 glass-input px-1.5 py-1 text-body font-mono" placeholder={t("projsub.linkToPh", { id: mgr.id })} />
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
                <span className="font-semibold text-[var(--module-accent)]">{t("projsub.pointModeDesc")}</span> {t("projsub.pointModeHint")}
              </p>
              <div>
                <label className="text-title text-slate-500 block mb-0.5">{t("projsub.pointPathLabel")}</label>
                <div className="flex items-center gap-1">
                  <input type="text" value={workflowPointPath} onChange={(e) => setWorkflowPointPath(e.target.value)}
                    className="flex-1 glass-input px-1.5 py-1 text-body font-mono"
                    placeholder={mgr.cache_path || t("projsub.pointPathPh")} />
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
            <p className="text-title text-slate-400 font-semibold">{t("projsub.oldFilesLabel")}</p>
            <label className={`flex items-start gap-2 p-2 rounded-ctl cursor-pointer border transition-all ${workflowFileAction === "move" ? "border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)]" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="move" checked={workflowFileAction === "move"}
                onChange={() => setWorkflowFileAction("move")} className="mt-0.5" />
              <div>
                <span className="text-title font-semibold text-[var(--module-accent)]">{t("projsub.moveOld")}</span>
                <p className="text-caption text-slate-500 mt-0.5">{t("projsub.moveOldDesc", { action: workflowMethod === "junction" ? t("projsub.actionCreateLink") : t("projsub.actionModifyEnv") })}</p>
              </div>
            </label>
            <label className={`flex items-start gap-2 p-2 rounded-ctl cursor-pointer border transition-all ${workflowFileAction === "keep" ? "border-slate-500/30 bg-slate-500/5" : "border-white/5 hover:bg-white/[0.02]"}`}>
              <input type="radio" name="wf_file_action" value="keep" checked={workflowFileAction === "keep"}
                onChange={() => setWorkflowFileAction("keep")} className="mt-0.5" />
              <div>
                <span className="text-title font-semibold text-slate-300">{t("projsub.keepOld")}</span>
                <p className="text-caption text-slate-500 mt-0.5">{t("projsub.keepOldDesc", { action: workflowMethod === "junction" ? t("projsub.actionLinkToNew") : t("projsub.actionModifyEnv") })}</p>
              </div>
            </label>
          </div>

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
      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("projsub.wfTitle", { step: 3, total: totalSteps, label: stepLabels.confirm })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">{t("projsub.cancel")}</button>
          </div>

          <div className="p-3 bg-black/20 rounded-ctl border border-white/5 space-y-2">
            <p className="text-caption text-slate-400 font-semibold uppercase tracking-wider">{t("projsub.opPreview")}</p>
            <div className="text-body text-slate-300 space-y-1 font-mono">
              {workflowMethod === "junction" ? (
                <>
                  <div><span className="text-slate-500">{t("projsub.mode")}</span> {t("projsub.modeJunction")}</div>
                  <div className="break-all"><span className="text-slate-500">{t("projsub.linkFromLabel")}</span> {workflowLinkPath}</div>
                  <div className="break-all"><span className="text-slate-500">{t("projsub.linkToLabel")}</span> {workflowActualPath}</div>
                </>
              ) : (
                <>
                  <div><span className="text-slate-500">{t("projsub.mode")}</span> {t("projsub.modePoint")}</div>
                  <div className="break-all"><span className="text-slate-500">{t("projsub.targetPathLabel")}</span> {workflowPointPath}</div>
                </>
              )}
              <div><span className="text-slate-500">{t("projsub.oldFileHandle")}</span> {workflowFileAction === "move" ? t("projsub.copyMove") : t("projsub.keepStatic")}</div>
            </div>
          </div>

          <div className="flex justify-between">
            <button onClick={workflowPrev}
              className="px-3 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("projsub.prevStep")}
            </button>
            <button onClick={executeWorkflow}
              className={`px-4 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors flex items-center gap-1`}>
              <CheckCircle className="w-3 h-3" />
              {t("projsub.confirmExec")}
            </button>
          </div>
        </div>
      );
    }

    // ── Step: 执行中 ──
    if (workflowStep === "executing") return <ConflictProgressStep progress={workflowProgress} />;

    // ── Step: 已完成 ──
    if (workflowStep === "done") return <ConflictDoneStep onClose={closeWorkflow} />;

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
      alertError(t("projsub.setFail", { name, err: String(e) }));
    } finally {
      setSavingVar(null);
    }
  };



  return (
    <div className="space-y-5">
      {!isAdmin && (
        <div className="flex items-start gap-2.5 p-3 rounded-card border border-amber-500/20 bg-amber-500/10 text-[12.5px] text-amber-200">
          <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
          <span>
            <strong>{t("projsub.permHint")}</strong>{t("projsub.permHintDesc")}
          </span>
        </div>
      )}

      {/* 路径类环境变量（系统管理，不可修改） */}
      <div className="space-y-3">
        <div className="flex items-start justify-between gap-3">
          <div>
            <span className="text-body font-semibold text-slate-300">{t("projsub.envVarsTitle")}</span>
            <span className="text-title text-slate-500 ml-1.5">{t("projsub.envVarCount", { count: vars.length })}</span>
            <p className="text-title text-slate-500 mt-0.5">{t("projsub.envVarsDesc")}</p>
          </div>
          {onRepairEnv && (
            <button
              onClick={() => {
                if (!isAdmin) {
                  setConfirmRequest({
                    title: t("projsub.repairEnv"),
                    confirmText: t("projsub.repairEnv"),
                    desc: t("projsub.repairConfirm"),
                    onConfirm: () => onRepairEnv(),
                  });
                  return;
                }
                onRepairEnv();
              }}
              disabled={isOperating || repairingEnv}
              title={t("projsub.repairTitle")}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-ctl bg-amber-500/10 hover:bg-amber-500/20 disabled:opacity-50 disabled:cursor-not-allowed text-amber-300 border border-amber-500/20 text-title font-semibold cursor-pointer transition-all whitespace-nowrap"
            >
              {repairingEnv ? <Loader className="w-3.5 h-3.5 animate-spin" /> : <Wrench className="w-3.5 h-3.5" />}
              {t("projsub.repairEnv")}
            </button>
          )}
        </div>
        {vars.length === 0 ? (
          <p className="text-caption text-slate-500">{t("projsub.noEnvVars")}</p>
        ) : (
          <div className="border border-white/5 rounded-card overflow-hidden overflow-x-auto">
            <table className="w-full text-left border-collapse text-title min-w-[450px]">
              <thead>
                <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-medium">
                  <th className="p-2.5 w-32">{t("projsub.thVarName")}</th>
                  <th className="p-2.5 w-16">Tier</th>
                  <th className="p-2.5 w-36">{t("projsub.thDesc")}</th>
                  <th className="p-2.5">{t("projsub.thValue")}</th>
                  <th className="p-2.5 w-28 whitespace-nowrap">{t("projsub.thSource")}</th>
                  <th className="p-2.5 w-14">{t("projsub.thStatus")}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/5 text-slate-300">
                {vars.map((v) => (
                  <tr key={v.name} className="hover:bg-white/1 font-mono">
                    <td className="p-2.5 font-semibold text-slate-200">{v.name}</td>
                    <td className="p-2.5">
                      {v.tier === "core" ? (
                        <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-body font-semibold">Core</span>
                      ) : v.tier === "package" ? (
                        <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-body font-semibold">Package</span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-body">-</span>
                      )}
                    </td>
                    <td className="p-2.5 text-slate-400 font-sans">{v.desc}</td>
                    <td className="p-2.5 break-all select-text">
                      {v.value || <span className="text-slate-600 font-sans">{t("projsub.notConfigured")}</span>}
                    </td>
                    <td className="p-2.5">
                      {v.source === "HKCU" ? (
                        <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-body font-semibold">{t("projsub.userLevel")}</span>
                      ) : v.source === "HKLM" ? (
                        <span className="px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-400 border border-indigo-500/20 text-body font-semibold">{t("projsub.systemLevel")}</span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-body">{t("projsub.notSet")}</span>
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




      {/* Kira 写入用户 PATH 的条目 —— 与环境变量同属托管面（只要 Kira 设置过就展示） */}
      {managedPaths.length > 0 && (
        <div className="border-t border-white/5 pt-4">
          <div className="flex items-center gap-2 mb-2">
            <span className="text-body font-semibold text-slate-300">{t("projsub.managedPathsTitle")}</span>
            {!fullyManaged && (
              <span className="px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 text-body font-semibold">{t("projsub.managedPathsResidual")}</span>
            )}
            <span className="text-body text-slate-500">{t("projsub.managedPathsHint")}</span>
          </div>
          <div className="border border-white/5 rounded-card overflow-hidden overflow-x-auto">
            <table className="w-full text-left border-collapse text-title min-w-[420px]">
              <thead>
                <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-medium">
                  <th className="p-2.5">{t("projsub.thManagedPath")}</th>
                  <th className="p-2.5 w-36">{t("projsub.thManagedKind")}</th>
                  <th className="p-2.5 w-24">{t("projsub.thStatus")}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-white/5 text-slate-300">
                {managedPaths.map((p) => {
                  const statusText = !p.in_path
                    ? t("projsub.managedPathNotInPath")
                    : !p.exists
                      ? t("projsub.managedPathDirMissing")
                      : t("projsub.managedPathInPath");
                  const ok = p.in_path && p.exists;
                  return (
                    <tr key={p.path} className="hover:bg-white/1">
                      <td className="p-2.5 break-all select-text font-mono text-body text-slate-200">{p.path}</td>
                      <td className="p-2.5">
                        {p.kind === "cache_bin" ? (
                          <span className="px-1.5 py-0.5 rounded bg-sky-500/10 text-sky-400 border border-sky-500/20 text-body font-semibold">{t("projsub.managedKindCache")}</span>
                        ) : (
                          <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-body font-semibold">{t("projsub.managedKindSdk")}</span>
                        )}
                      </td>
                      <td className="p-2.5">
                        {ok ? (
                          <CheckCircle className="w-3.5 h-3.5 text-emerald-400" />
                        ) : (
                          <span title={statusText}>
                            <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
                          </span>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          <p className="text-caption text-slate-500 mt-1.5">{t("projsub.managedPathsFootnote")}</p>
        </div>
      )}

      {/* 高级模式：运行时环境变量（用户可配置） */}
      <div className="border-t border-white/5 pt-4">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <button
              onClick={() => setAdvanced(!advanced)}
              className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors cursor-pointer ${advanced ? "bg-[var(--module-accent)]" : "bg-white/10"}`}
            >
              <span className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${advanced ? "translate-x-[18px]" : "translate-x-[3px]"}`} />
            </button>
            <span className="text-body font-semibold text-slate-300">{t("projsub.advModeTitle")}</span>
            {advanced && (
              <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-caption font-semibold">{t("projsub.advBadge")}</span>
            )}
          </div>
          {advanced && (
            <button onClick={loadUserVars} disabled={loadingUserVars} className="flex items-center gap-1 px-2 py-1 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-title border border-white/5 cursor-pointer">
              <RefreshCw className={`w-3 h-3 ${loadingUserVars ? "animate-spin" : ""}`} />{t("projsub.refreshList")}
            </button>
          )}
        </div>

        {!advanced ? (
          <p className="text-title text-slate-500">{t("projsub.advDesc", { name: def?.display_name || t("projsub.project") })}</p>
        ) : loadingUserVars ? (
          <div className="flex items-center gap-2 text-title text-slate-400 py-4"><Loader className="w-3 h-3 animate-spin text-[var(--module-accent)]" />{t("projsub.loading")}</div>
        ) : userVars.length === 0 ? (
          <p className="text-title text-slate-500">{t("projsub.noRuntimeVars")}</p>
        ) : (
          <div className="border border-white/5 rounded-card overflow-hidden overflow-x-auto">
            <table className="w-full text-left border-collapse text-title">
              <thead>
                <tr className="bg-white/3 border-b border-white/5 text-slate-400 font-medium">
                  <th className="p-2.5 w-40">{t("projsub.thVarName")}</th>
                  <th className="p-2.5">{t("projsub.thDesc")}</th>
                  <th className="p-2.5">{t("projsub.thCurrentVal")}</th>
                  <th className="p-2.5 w-24 whitespace-nowrap">{t("projsub.thSource")}</th>
                  <th className="p-2.5 w-28 text-center">{t("projsub.thAction")}</th>
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
                            <select value={editValue} onChange={(e) => setEditValue(e.target.value)} className="glass-input px-2 py-1 text-title font-mono rounded w-full">
                              <option value="">{t("projsub.unsetOpt")}</option>
                              {v.options.map(o => <option key={o} value={o}>{o}</option>)}
                            </select>
                          ) : v.var_type === "boolean" ? (
                            <select value={editValue} onChange={(e) => setEditValue(e.target.value)} className="glass-input px-2 py-1 text-title font-mono rounded w-full">
                              <option value="">{t("projsub.unsetOpt")}</option>
                              <option value="1">{t("projsub.enabled1")}</option>
                              <option value="0">{t("projsub.disabled0")}</option>
                            </select>
                          ) : (
                            <input type="text" value={editValue} onChange={(e) => setEditValue(e.target.value)} className="glass-input px-2 py-1 text-title font-mono rounded w-full" placeholder={v.placeholder} />
                          )
                        ) : (
                          hasValue ? (
                            <span className="font-mono text-slate-200 break-all">{v.current_value}</span>
                          ) : (
                            <span className="text-slate-600 font-sans">{t("projsub.notSet")}</span>
                          )
                        )}
                      </td>
                      <td className="p-2.5 text-slate-500">
                        {v.source || "-"}
                      </td>
                      <td className="p-2.5 text-center">
                        {isEditing ? (
                          <div className="flex items-center gap-1 justify-center">
                            <button onClick={() => handleSetVar(v.name, editValue)} disabled={savingVar === v.name} className="px-2 py-0.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-white rounded text-caption font-semibold cursor-pointer">
                              {savingVar === v.name ? t("projsub.saving") : t("projsub.save")}
                            </button>
                            <button onClick={() => setEditingVar(null)} className="px-2 py-0.5 bg-white/5 hover:bg-white/10 text-slate-400 rounded text-caption cursor-pointer">{t("projsub.cancel")}</button>
                          </div>
                        ) : (
                          <button onClick={() => { setEditingVar(v.name); setEditValue(v.current_value || ""); }} className="px-2 py-0.5 bg-white/5 hover:bg-white/10 text-slate-300 rounded text-caption border border-white/5 cursor-pointer">
                            {hasValue ? t("projsub.modify") : t("projsub.set")}
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
              <h4 className="text-body font-semibold text-slate-300">{t("projsub.conflictTitle")}</h4>
            </div>
            <p className="text-title text-slate-500 mt-0.5">
              {t("projsub.conflictDesc")}
            </p>
          </div>

          {loadingConflicts ? (
            <div className="flex items-center gap-2 text-title text-slate-400 py-2">
              <Loader className="w-3.5 h-3.5 animate-spin text-[var(--module-accent)]" />{t("projsub.scanningEnv")}
            </div>
          ) : conflictManagers.length === 0 ? (
            <p className="text-title text-slate-500">{t("projsub.noConflict")}</p>
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
                  <div key={mgr.id} className="glass-panel border border-white/5 rounded-panel p-4 bg-white/1 space-y-4">
                    {/* 顶部标题与状态 */}
                    <div className="flex items-center justify-between pb-2 border-b border-white/3">
                      <span className="text-[14px] font-semibold text-slate-200">{mgr.display_name}</span>
                      {mgr.is_disabled ? (
                        <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-caption font-semibold">{t("projsub.statusExclusive")}</span>
                      ) : mgr.installed ? (
                        <span className="px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 text-caption font-semibold animate-pulse">{t("projsub.statusActive")}</span>
                      ) : (
                        <span className="px-1.5 py-0.5 rounded bg-white/5 text-slate-500 border border-white/5 text-caption font-semibold">{t("projsub.statusNotRunning")}</span>
                      )}
                    </div>

                    {/* 1. 缓存目录管理区域 */}
                    {mgr.cache_path && (
                      <div className="p-3 bg-white/2 rounded-card border border-white/5 space-y-3">
                        <div className="flex items-center gap-1.5">
                          <HardDrive className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                          <span className="text-body font-semibold text-slate-300">{t("projsub.cacheDirTitle")}</span>
                        </div>

                        {/* 路径与大小状态 */}
                        <div className="flex items-start justify-between text-body p-2.5 bg-black/20 rounded-ctl border border-white/3">
                          <div className="space-y-1 flex-1 min-w-0">
                            <div className="flex items-center gap-2 flex-wrap">
                              {hasEnvConfigured ? (
                                <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-tiny inline-flex items-center font-mono">
                                  {t("projsub.envConfigured")}
                                </span>
                              ) : (
                                <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-tiny inline-flex items-center">
                                  {t("projsub.envNotConfigured")}
                                </span>
                              )}

                              {isJunction ? (
                                <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-tiny inline-flex items-center font-semibold">
                                  {t("projsub.migratedJunction")}
                                </span>
                              ) : (
                                <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-tiny inline-flex items-center">
                                  {t("projsub.defaultPath")}
                                </span>
                              )}
                            </div>
                            <p className="font-mono text-body text-slate-300 break-all mt-1">{mgr.cache_path}</p>
                          </div>
                          <div className="flex items-center gap-2 flex-shrink-0 ml-3">
                            <span className="text-slate-300 font-mono text-body font-semibold bg-white/5 px-2 py-0.5 rounded-md border border-white/5">
                              {mgr.cache_size}
                            </span>
                          </div>
                        </div>

                        {/* 缓存操作按钮 */}
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleConflictAction(mgr.id, "clean")}
                            disabled={isOperating || isOperatingMgr || workflowManagerId !== null}
                            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-ctl bg-red-500/10 hover:bg-red-500/20 text-red-300 border border-red-500/20 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer transition-all font-semibold text-caption"
                          >
                            <Trash2 className="w-3 h-3" />
                            {t("projsub.cleanCache")}
                          </button>

                          <button
                            onClick={() => openWorkflow(mgr)}
                            disabled={isOperating || isOperatingMgr || workflowManagerId !== null}
                            className="flex items-center gap-1 px-2.5 py-1.5 rounded-ctl bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 font-semibold text-caption cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                          >
                            <FolderSync className="w-3 h-3" />
                            {t("projsub.startChange")}
                          </button>
                        </div>

                        {/* 缓存变更的统一分步引导工作流面板 */}
                        {workflowManagerId === mgr.id && renderConflictWorkflow(mgr)}
                      </div>
                    )}

                    {/* 2. 冲突规避与停用区域 */}
                    <div className="p-3 bg-white/2 rounded-card border border-white/5 space-y-3">
                      <div className="flex items-center gap-1.5">
                        <Wrench className="w-3.5 h-3.5 text-amber-400" />
                        <span className="text-body font-semibold text-slate-300">{t("projsub.conflictRegVars")}</span>
                      </div>

                      <div className="text-body text-slate-400 space-y-1.5 bg-black/20 p-2.5 rounded-ctl border border-white/3">
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
                          <span className="text-slate-500">{t("projsub.regVar")}</span>
                          {Object.entries(mgr.env_vars_status).map(([key, val]) => (
                            <span key={key} className="font-mono text-caption bg-white/3 px-1.5 py-0.5 rounded border border-white/5">
                              {key}={val ? <span className="text-slate-300 break-all select-text">"{val as string}"</span> : <span className="text-slate-600 font-sans text-tiny">{t("projsub.notSet")}</span>}
                            </span>
                          ))}
                        </div>
                        {mgr.path_status.length > 0 ? (
                          <div className="pt-1">
                            <span className="text-slate-500">{t("projsub.conflictPathInPath")}</span>
                            {mgr.path_status.map((p: string) => (
                              <div key={p} className="font-mono text-caption text-amber-300/80 break-all select-text ml-4 mt-0.5">• {p}</div>
                            ))}
                          </div>
                        ) : (
                          <div className="text-emerald-400/80 font-semibold text-caption pt-1">{t("projsub.noConflictPath")}</div>
                        )}
                      </div>

                      {!mgr.is_disabled ? (
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => handleConflictAction(mgr.id, "disable")}
                            disabled={isOperating || isOperatingMgr}
                            className="flex items-center gap-1 px-2.5 py-1.5 rounded-ctl bg-orange-500/10 hover:bg-orange-500/20 text-orange-300 border border-orange-500/20 disabled:opacity-50 disabled:cursor-not-allowed cursor-pointer transition-all font-semibold text-caption"
                            title={t("projsub.deactivateTitle")}
                          >
                            <X className="w-3.5 h-3.5" />
                            {t("projsub.deactivateBtn")}
                          </button>
                        </div>
                      ) : (
                        <div className="text-body text-emerald-400 font-semibold flex items-center gap-1 bg-emerald-500/5 p-2 rounded-ctl border border-emerald-500/10">
                          <Check className="w-3.5 h-3.5 text-emerald-400" />
                          {t("projsub.exclusiveMsg")}
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

      {/* 统一确认弹窗（替代原生 window.confirm；含一键停用影响面确认） */}
      {<ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />}
    </div>
  );
}
