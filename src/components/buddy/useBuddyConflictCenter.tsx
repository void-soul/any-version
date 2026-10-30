import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { filterSyncDetails, SYNC_STATUS_FILTERS } from "./sessionSync";
import { AlertTriangle } from "lucide-react";
import { SYNC_STATUS_CLASS, type BuddyConflictMessage, type BuddyConflictProgress, type BuddyConflictResolution, type BuddyPendingConflict, type BuddySessionSyncSummary, type ConflictAction } from "./BuddyPanel.model";
import type { RefObject } from "react";

/** Owns the BuddyConflictCenter feature, independently of page rendering. */
export function useBuddyConflictCenter({ platformEpochRef, platform, showMsg }: {
  platformEpochRef: RefObject<number>;
  platform: string;
  showMsg: (ok: boolean, text: string) => void;
}) {
  const { t } = useTranslation();

  // 冲突裁决进度（后端 buddy-conflict-progress）：裁决要递归复制整个会话目录，
  // 批量处理可能跑很久，没有进度用户只能盯着一个转圈的按钮以为卡死了
  const [conflictProgress, setConflictProgress] = useState<BuddyConflictProgress | null>(null);

  // 最近一次切换的会话同步台账（成功/跳过/冲突/失败明细），切换结束后保留给用户查看
  const [syncSummary, setSyncSummary] = useState<BuddySessionSyncSummary | null>(null);

  const [syncDetailsOpen, setSyncDetailsOpen] = useState(false);

  // 明细的状态筛选（"all" | copied | skipped | partial | conflict | failed）
  const [syncStatusFilter, setSyncStatusFilter] = useState<string>("all");

  // 冲突提醒横幅是否被手动关闭（每次切换都会重新亮起，见 switchAccount）
  const [conflictAlertDismissed, setConflictAlertDismissed] = useState(false);

  // 冲突确认框：必须显式点一个按钮才能关（点背景不关）。与横幅是两档提醒 ——
  // 横幅常驻、可被忽略；确认框强制确认一次。每次切换重新弹（见 switchAccount）。
  const [conflictDialogOpen, setConflictDialogOpen] = useState(false);

  // 本轮切换是否已经弹过确认框：事件与命令返回值都可能带同一份台账，避免弹两次。
  // 用 ref 而非 state —— 事件闭包捕获的 state 是旧值，读不到「已经弹过」。
  const conflictDialogShownRef = useRef(false);

  // 待裁决的会话冲突（切换时被搁置、等用户逐条决定的），切换结束/进入面板时都会拉一次
  const [sessionConflicts, setSessionConflicts] = useState<BuddyPendingConflict[]>([]);

  // 冲突处理弹窗（查看明细 + 合并/覆盖/保留）
  const [conflictPanelOpen, setConflictPanelOpen] = useState(false);

  // 「查看明细」展开到哪一条（两侧账号、IDE、时间都在明细里）
  const [expandedConflictId, setExpandedConflictId] = useState<string | null>(null);

  const [conflictBusyId, setConflictBusyId] = useState<string | null>(null);

  // 批量处理：勾选的会话 id（点表头「全选」一次勾上当前列表）+ 批量执行中的标记
  const [conflictSelectedIds, setConflictSelectedIds] = useState<Set<string>>(new Set());

  const [conflictBatchBusy, setConflictBatchBusy] = useState(false);

  // 冲突处理结果（key = 会话 id）：会话明细据此显示「已合并 / 已覆盖 / 已保留」，
  // 而不是永远停在切换那一刻的初始状态
  const [conflictResolutions, setConflictResolutions] = useState<Record<string, BuddyConflictResolution>>({});

  // 裁决二次确认：合并 / 覆盖 / 保留都会改会话内容，误点不可逆，所以先弹确认再执行。
  // 单条与批量共用一个弹窗，用 kind 区分（批量一次可能改几十条）。
  const [conflictConfirm, setConflictConfirm] = useState<
    | { kind: "single"; conflict: BuddyPendingConflict; action: ConflictAction }
    | { kind: "batch"; ids: string[]; action: ConflictAction }
    | null
  >(null);

  // 对话内容预览（key = `${conflictId}:${side}`）：点「看对话」才拉取，避免展开就搬全部消息。
  // 读取失败也用消息数组表达（role="error"），就地显示后端的可操作提示。
  const [conflictMessages, setConflictMessages] = useState<Record<string, BuddyConflictMessage[] | "loading">>({});

  // 冲突处理结果：会话明细要显示「处理过了」而不是初始状态，所以与冲突列表一起拉
  const loadConflictResolutions = useCallback(async () => {
    const epoch = platformEpochRef.current;
    try {
      const list = await invoke<BuddyConflictResolution[]>("buddy_list_conflict_resolutions", { platform });
      if (platformEpochRef.current !== epoch) return;
      const map: Record<string, BuddyConflictResolution> = {};
      for (const r of list) map[r.id] = r;
      setConflictResolutions(map);
    } catch {
      if (platformEpochRef.current !== epoch) return;
      setConflictResolutions({});
    }
  }, [platform]);

  // 待裁决冲突：进入面板 / 切换平台 / 切换账号后都拉一次（切换时冲突落盘，之后随时可处理）
  const loadConflicts = useCallback(async () => {
    const epoch = platformEpochRef.current;
    try {
      const list = await invoke<BuddyPendingConflict[]>("buddy_list_session_conflicts", { platform });
      if (platformEpochRef.current !== epoch) return;
      setSessionConflicts(list);
    } catch {
      if (platformEpochRef.current !== epoch) return;
      // 拉取失败不打扰用户：横幅与处理入口都依赖这份列表，失败时按「无冲突」降级
      setSessionConflicts([]);
    }
    await loadConflictResolutions();
  }, [platform, loadConflictResolutions]);

  useEffect(() => {
    void loadConflicts();
  }, [loadConflicts]);

  /** 对一条冲突执行裁决，用后端返回的剩余列表整体替换本地状态 */
  const doResolveConflict = async (conflict: BuddyPendingConflict, action: ConflictAction) => {
    setConflictBusyId(conflict.id);
    try {
      const remaining = await invoke<BuddyPendingConflict[]>("buddy_resolve_session_conflict", {
        platform,
        conversationId: conflict.id,
        action,
      });
      setSessionConflicts(remaining);
      // 冲突计数按剩余条数同步（不是只在清零时处理）：处理一半时横幅与明细才对得上
      setSyncSummary((prev) => (prev && prev.conflict !== remaining.length ? { ...prev, conflict: remaining.length } : prev));
      setConflictSelectedIds((prev) => {
        const next = new Set(prev);
        next.delete(conflict.id);
        return next;
      });
      setExpandedConflictId(null);
      void loadConflictResolutions();
      // 冲突清零：面板已经没有可裁决的东西了，直接关掉并告诉用户「都处理完了」，
      // 否则留一个空弹窗让人自己发现「怎么忽然什么都没了」。
      if (remaining.length === 0) {
        setConflictPanelOpen(false);
        showMsg(true, t("buddy.conflictAllResolved"));
      } else {
        showMsg(true, t("buddy.conflictResolved", { label: conflict.label }));
      }
    } catch (e: any) {
      showMsg(false, String(e));
    } finally {
      setConflictBusyId(null);
    }
  };

  /** 点单条裁决按钮：先弹二次确认，不直接动数据（合并/覆盖/保留都会改会话内容，误点代价高） */
  const resolveConflict = (conflict: BuddyPendingConflict, action: ConflictAction) => {
    setConflictConfirm({ kind: "single", conflict, action });
  };

  /** 批量裁决：对勾选的多条执行同一 action。
   *  走后端批量命令（一次加锁 + 一次备份目录），不在这里串行调单条 ——
   *  单条命令每次都会清空备份目录，串行下来只剩最后一条能回滚。 */
  const doResolveConflictsBatch = async (ids: string[], action: ConflictAction) => {
    setConflictBatchBusy(true);
    try {
      const remaining = await invoke<BuddyPendingConflict[]>("buddy_resolve_session_conflicts", {
        platform,
        conversationIds: ids,
        action,
      });
      setSessionConflicts(remaining);
      setSyncSummary((prev) => (prev && prev.conflict !== remaining.length ? { ...prev, conflict: remaining.length } : prev));
      // 已处理的从勾选里清掉；失败时后端整体报错，勾选保留便于重试
      const remainingIds = new Set(remaining.map((c) => c.id));
      setConflictSelectedIds((prev) => new Set([...prev].filter((id) => remainingIds.has(id))));
      setExpandedConflictId(null);
      void loadConflictResolutions();
      if (remaining.length === 0) {
        setConflictPanelOpen(false);
        showMsg(true, t("buddy.conflictAllResolved"));
      } else {
        showMsg(
          true,
          t("buddy.conflictBatchResolved", {
            count: ids.length - remaining.filter((c) => ids.includes(c.id)).length,
            action: conflictActionLabel(action),
          }),
        );
      }
    } catch (e: any) {
      showMsg(false, String(e));
    } finally {
      setConflictBatchBusy(false);
    }
  };

  /** 点批量裁决按钮：同样先弹确认（一次可能改几十条会话） */
  const resolveConflictsBatch = (action: ConflictAction) => {
    const ids = [...conflictSelectedIds];
    if (ids.length === 0) return;
    setConflictConfirm({ kind: "batch", ids, action });
  };

  /**
   * 冲突动作的本地化名（合并/覆盖/保留）。确认弹窗标题、批量描述、成功提示三处共用——
   * 曾因批量描述漏传 action 插值导致界面直接显示 `{{action}}` 字面量（Q-0357），
   * 收敛到一处后不会再漏。
   */
  const conflictActionLabel = (action: ConflictAction) =>
    t(`buddy.conflictAction${action[0].toUpperCase()}${action.slice(1)}`);

  const toggleConflictSelect = (id: string) => {
    setConflictSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  /** 处理过的冲突会话 id 集合（明细「已处理」筛选用；它不是后端状态，是前端视图） */
  const resolvedConflictIds = useMemo(() => new Set(Object.keys(conflictResolutions)), [conflictResolutions]);

  const toggleConflictSelectAll = () => {
    setConflictSelectedIds((prev) =>
      prev.size === sessionConflicts.length ? new Set() : new Set(sessionConflicts.map((c) => c.id)),
    );
  };

  /** 拉取冲突会话某一侧的对话内容（点「看对话」才加载） */
  const loadConflictMessages = async (conflictId: string, side: "source" | "target") => {
    const key = `${conflictId}:${side}`;
    if (conflictMessages[key] === "loading") return;
    setConflictMessages((prev) => ({ ...prev, [key]: "loading" }));
    try {
      const list = await invoke<BuddyConflictMessage[]>("buddy_read_conflict_messages", {
        platform,
        conversationId: conflictId,
        side,
      });
      setConflictMessages((prev) => ({ ...prev, [key]: list }));
    } catch (e: any) {
      // 错误信息就地显示（比如来源账号数据已被清理）
      setConflictMessages((prev) => ({ ...prev, [key]: [{ role: "error", time: "", text: String(e) }] }));
    }
  };

  /** 打开冲突处理弹窗：先强制刷新待处理列表再开。
   *  本地列表可能过期（面板挂载后一直没切账号）或加载失败过（当时按空降级），
   *  每次打开都向后端要最新数据，绝不能让「明明有冲突却打不开处理入口」。 */
  const openConflictPanel = async () => {
    await loadConflicts();
    // 每次打开都从空勾选开始：上次的勾选对应的冲突可能已经处理掉了
    setConflictSelectedIds(new Set());
    setConflictPanelOpen(true);
  };

  const closeConflictPanel = () => {
    setConflictPanelOpen(false);
    setConflictSelectedIds(new Set());
  };

  /**
   * 有冲突会话时的统一动作：展开明细并预筛到「冲突」+ 弹确认框。
   *
   * 明细默认折叠、默认显示全部状态，几百条「跳过」里夹几条冲突基本必被漏掉；
   * 所以既把第一眼落在这些会话上（自动展开 + 预筛），又弹一个必须显式选择才能
   * 关掉的确认框（横幅另有一份，见下方 conflictAlert）。
   */
  const focusConflicts = (conflictCount: number) => {
    if (conflictCount === 0) {
      return;
    }
    setSyncDetailsOpen(true);
    setSyncStatusFilter("conflict");
    if (!conflictDialogShownRef.current) {
      conflictDialogShownRef.current = true;
      setConflictDialogOpen(true);
    }
  };

  const renderConflictSummary = () => syncSummary && (<div className="border-b border-white/5 bg-white/[0.02] flex-shrink-0">
          <button
            onClick={() => setSyncDetailsOpen((open) => !open)}
            className="w-full flex items-center gap-2 px-4 py-1.5 text-caption text-left hover:bg-white/5 cursor-pointer"
          >
            <span className="text-slate-500 w-3">{syncDetailsOpen ? "▾" : "▸"}</span>
            <span className="font-semibold text-slate-300">{t("buddy.syncTitle")}</span>
            <span className="text-slate-500">
              {t("buddy.syncTotal", { count: syncSummary.total })}
            </span>
            <span className="ml-auto flex items-center gap-2">
              <span className="text-emerald-400">{t("buddy.syncCountCopied", { count: syncSummary.copied })}</span>
              <span className="text-slate-400">{t("buddy.syncCountSkipped", { count: syncSummary.skipped })}</span>
              {syncSummary.conflict > 0 && (
                <span className="text-amber-400">{t("buddy.syncCountConflict", { count: syncSummary.conflict })}</span>
              )}
              {(syncSummary.partial > 0 || syncSummary.failed > 0) && (
                <span className="text-rose-400">
                  {t("buddy.syncCountFailed", { count: syncSummary.partial + syncSummary.failed })}
                </span>
              )}
            </span>
          </button>
          {syncDetailsOpen && (
            <div className="max-h-56 overflow-y-auto border-t border-white/5">
              {/* 状态筛选：明细最多 500 条，冲突/失败往往只有几条，不筛要翻半天 */}
              <div className="flex items-center gap-1 px-4 py-1.5 border-b border-white/5 sticky top-0 bg-surface-deep z-10">
                {SYNC_STATUS_FILTERS.map((status) => (
                  <button
                    key={status}
                    onClick={() => setSyncStatusFilter(status)}
                    className={`px-1.5 py-0.5 rounded-md text-tiny cursor-pointer transition-all ${
                      syncStatusFilter === status
                        ? "bg-[var(--module-accent)]/25 text-white font-semibold"
                        : "text-slate-400 hover:bg-white/5"
                    }`}
                  >
                    {status === "all" ? t("buddy.syncFilterAll") : t(`buddy.syncStatus.${status}`)}
                  </button>
                ))}
              </div>
              {syncSummary.unchanged && (
                <div className="px-4 py-1.5 text-caption text-slate-500">{t("buddy.syncUnchanged")}</div>
              )}
              {/* 明细 ≤500 条，直接两次调用纯过滤函数即可，无需额外 state。
                  「已处理」一档要传处理结果集合（它不是后端状态，是前端视图）。 */}
              {filterSyncDetails(syncSummary.details, syncStatusFilter, resolvedConflictIds).length === 0 && !syncSummary.unchanged && (
                <div className="px-4 py-1.5 text-caption text-slate-500">{t("buddy.syncFilterEmpty")}</div>
              )}
              {filterSyncDetails(syncSummary.details, syncStatusFilter, resolvedConflictIds).map((detail, index) => {
                // 处理结果：裁决过的会话显示「已合并 / 已覆盖 / 已保留」+ 时间，
                // 而不是永远停在切换那一刻的初始状态（conflict / bothChanged）
                const resolution = conflictResolutions[detail.id];
                return (
                <div
                  key={`${detail.id}-${index}`}
                  className="flex items-start gap-2 px-4 py-1 text-caption border-b border-white/5 last:border-b-0"
                >
                  <span className={`flex-shrink-0 w-16 ${SYNC_STATUS_CLASS[detail.status] ?? "text-slate-400"}`}>
                    {t(`buddy.syncStatus.${detail.status}`)}
                  </span>
                  {resolution && (
                    <>
                      <span
                        className="flex-shrink-0 px-1.5 py-0.5 rounded-md text-tiny bg-emerald-500/15 text-emerald-300 border border-emerald-500/25"
                        title={resolution.message}
                      >
                        {t(`buddy.conflictResult.${resolution.action}`)}
                      </span>
                      <span
                        className="flex-shrink-0 text-slate-500"
                        title={t("buddy.conflictResolvedAt", { time: new Date(resolution.atMs).toLocaleString() })}
                      >
                        {new Date(resolution.atMs).toLocaleString()}
                      </span>
                    </>
                  )}
                  {/* 目录列：这条会话属于哪个项目；超长按宽度折叠，悬停看完整路径 */}
                  <span
                    className="flex-shrink-0 w-40 truncate text-slate-500 cursor-help"
                    title={detail.workspace ?? undefined}
                  >
                    {detail.workspace || t("buddy.syncWorkspaceUnknown")}
                  </span>
                  {/* 名称列：无标题会话后端已回落到目录名；这里再兜一层，
                      绝不让裸 id 直接怼到用户脸上 */}
                  <span className="min-w-0 flex-1 break-all text-slate-300">
                    {detail.label || detail.workspace || detail.id}
                  </span>
                  <span className="flex-shrink-0 text-slate-500" title={t(`buddy.syncReason.${detail.reason}`)}>
                    {t(`buddy.syncReason.${detail.reason}`)}
                  </span>
                  {/* 冲突行直达处理弹窗：只靠顶部横幅一个入口太脆弱，明细里看到冲突
                      的人就该在冲突旁边拿到处理入口（弹窗打开前会强制刷新列表）。
                      已处理过的（有处理结果）就不再显示这个按钮——它已经不是待办了。 */}
                  {detail.status === "conflict" && !resolution && (
                    <button
                      onClick={() => void openConflictPanel()}
                      className="flex-shrink-0 px-1.5 py-0.5 rounded-md text-tiny bg-amber-500/20 hover:bg-amber-500/30 text-amber-200 font-semibold cursor-pointer transition-colors"
                    >
                      {t("buddy.conflictAlertResolve")}
                    </button>
                  )}
                </div>
                );
              })}
            </div>
          )}
        </div>);

const renderConflictIntro = () => syncSummary && (<div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[380px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-4">
              <div className="w-9 h-9 rounded-card bg-amber-500/15 border border-amber-500/30 flex items-center justify-center">
                <AlertTriangle className="w-4 h-4 text-amber-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">
                  {t("buddy.conflictDialogTitle", { count: syncSummary.conflict })}
                </h3>
                <p className="text-tiny text-slate-500">{t("buddy.conflictDialogHint")}</p>
              </div>
            </div>
            <p className="text-body text-slate-300 leading-relaxed mb-5">
              {t("buddy.conflictDialogDesc")}
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConflictDialogOpen(false)}
                className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer"
              >
                {t("buddy.conflictDialogOk")}
              </button>
              <button
                onClick={() => {
                  setConflictDialogOpen(false);
                  // 直接进处理弹窗逐条裁决（内部会先刷新列表，不依赖本地缓存）
                  void openConflictPanel();
                }}
                className="px-4 py-1.5 rounded-ctl text-caption bg-amber-600 hover:bg-amber-500 text-white font-semibold cursor-pointer"
              >
                {t("buddy.conflictAlertResolve")}
              </button>
            </div>
          </div>
        </div>);

const renderConflictPanel = () => (<div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[560px] max-w-[95vw] max-h-[85vh] flex flex-col rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-3">
              <div className="w-9 h-9 rounded-card bg-amber-500/15 border border-amber-500/30 flex items-center justify-center">
                <AlertTriangle className="w-4 h-4 text-amber-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">
                  {t("buddy.conflictPanelTitle", { count: sessionConflicts.length })}
                </h3>
                <p className="text-tiny text-slate-500">{t("buddy.conflictPanelHint")}</p>
              </div>
              <button
                onClick={closeConflictPanel}
                className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 cursor-pointer"
              >
                ✕
              </button>
            </div>

            {sessionConflicts.length === 0 ? (
              <div className="py-8 text-center text-body text-slate-500">{t("buddy.conflictPanelEmpty")}</div>
            ) : (
              <>
              {/* 批量处理条：勾选后一次对多条执行同一裁决。
                  走后端批量命令，避免串行调单条把备份目录清得只剩最后一条。 */}
              <div className="mb-2 flex flex-wrap items-center gap-1.5 rounded-ctl border border-white/10 bg-white/[0.03] px-2.5 py-2">
                <button
                  onClick={toggleConflictSelectAll}
                  className="px-2 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer"
                >
                  {conflictSelectedIds.size === sessionConflicts.length ? t("buddy.sessions.selectAllClear") : t("buddy.sessions.selectAll")}
                </button>
                <span className="text-tiny text-slate-500">
                  {t("buddy.conflictSelectedCount", { count: conflictSelectedIds.size, total: sessionConflicts.length })}
                </span>
                <span className="flex-1" />
                {/* 处理进度：后端逐条上报（buddy-conflict-progress）。
                    裁决要递归复制整个会话目录，没有它长时间无反馈会被当成卡死。 */}
                {(conflictBatchBusy || conflictBusyId !== null) && (
                  <span className="text-tiny text-slate-400">
                    {conflictProgress && conflictProgress.total > 1
                      ? t("buddy.conflictResolving", { done: conflictProgress.done, total: conflictProgress.total })
                      : t("buddy.conflictResolvingOne")}
                  </span>
                )}
                {(["merge", "overwrite", "keep"] as ConflictAction[]).map((action) => (
                  <button
                    key={action}
                    disabled={conflictSelectedIds.size === 0 || conflictBatchBusy || conflictBusyId !== null}
                    onClick={() => void resolveConflictsBatch(action)}
                    title={t(`buddy.conflictAction${action[0].toUpperCase()}${action.slice(1)}Tip`)}
                    className={`px-2.5 py-1 rounded-md text-tiny font-semibold cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed transition-colors ${
                      action === "merge"
                        ? "bg-[var(--module-accent)]/20 hover:bg-[var(--module-accent)]/30 text-white"
                        : action === "overwrite"
                          ? "bg-rose-600/20 hover:bg-rose-600/30 text-rose-300"
                          : "bg-white/5 hover:bg-white/10 text-slate-300"
                    }`}
                  >
                    {t(`buddy.conflictAction${action[0].toUpperCase()}${action.slice(1)}`)}
                  </button>
                ))}
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto border border-white/5 rounded-card divide-y divide-white/5">
                {sessionConflicts.map((conflict) => {
                  const expanded = expandedConflictId === conflict.id;
                  return (
                    <div key={`${conflict.targetUid}-${conflict.id}`} className="px-3 py-2.5">
                      {/* 主行：真实项目路径 + 会话名，两侧时间一目了然。
                          后端已把 MD5 哈希目录还原成真实路径；还原失败才显示哈希。 */}
                      <div className="flex items-center gap-2 text-caption">
                        <input
                          type="checkbox"
                          checked={conflictSelectedIds.has(conflict.id)}
                          onChange={() => toggleConflictSelect(conflict.id)}
                          className="flex-shrink-0 h-3.5 w-3.5 cursor-pointer accent-[var(--module-accent)]"
                          title={t("buddy.conflictSelectTip")}
                        />
                        <span
                          className="flex-shrink-0 max-w-[220px] truncate text-slate-500 cursor-help"
                          title={conflict.workspacePath || conflict.workspace}
                        >
                          {conflict.workspacePath || conflict.workspace || t("buddy.syncWorkspaceUnknown")}
                        </span>
                        <span className="min-w-0 flex-1 truncate text-slate-200" title={conflict.label}>
                          {conflict.label}
                        </span>
                        <span className="flex-shrink-0 text-slate-500">
                          <span title={t("buddy.conflictSourceTime")}>{t("buddy.conflictSourceShort")}</span>
                          {" "}
                          <span className="text-slate-300">{new Date(conflict.sourceStamp).toLocaleString()}</span>
                          {" · "}
                          <span title={t("buddy.conflictTargetTime")}>{t("buddy.conflictTargetShort")}</span>
                          {" "}
                          <span className="text-slate-300">{new Date(conflict.targetStamp).toLocaleString()}</span>
                        </span>
                      </div>
                      {/* 操作行：查看明细 + 三个裁决按钮 */}
                      <div className="mt-1.5 flex items-center gap-1.5 flex-wrap">
                        <button
                          onClick={() => setExpandedConflictId(expanded ? null : conflict.id)}
                          className="px-2 py-1 rounded-md text-tiny text-slate-400 hover:bg-white/5 hover:text-white cursor-pointer transition-colors"
                        >
                          {expanded ? "▾" : "▸"} {t("buddy.conflictViewDetail")}
                        </button>
                        <span className="flex-1" />
                        <button
                          disabled={conflictBusyId !== null}
                          onClick={() => void resolveConflict(conflict, "merge")}
                          title={t("buddy.conflictActionMergeTip")}
                          className="px-2.5 py-1 rounded-md text-tiny bg-[var(--module-accent)]/20 hover:bg-[var(--module-accent)]/30 text-white font-semibold cursor-pointer disabled:opacity-50 transition-colors"
                        >
                          {t("buddy.conflictActionMerge")}
                        </button>
                        <button
                          disabled={conflictBusyId !== null}
                          onClick={() => void resolveConflict(conflict, "overwrite")}
                          title={t("buddy.conflictActionOverwriteTip")}
                          className="px-2.5 py-1 rounded-md text-tiny bg-rose-600/20 hover:bg-rose-600/30 text-rose-300 font-semibold cursor-pointer disabled:opacity-50 transition-colors"
                        >
                          {t("buddy.conflictActionOverwrite")}
                        </button>
                        <button
                          disabled={conflictBusyId !== null}
                          onClick={() => void resolveConflict(conflict, "keep")}
                          title={t("buddy.conflictActionKeepTip")}
                          className="px-2.5 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 font-semibold cursor-pointer disabled:opacity-50 transition-colors"
                        >
                          {t("buddy.conflictActionKeep")}
                        </button>
                        {/* 忽略：会话目录可能已被清理，给一个「不处理、别再提醒」的出口，
                            否则这条冲突会永远留在列表里、横幅一直亮 */}
                        <button
                          disabled={conflictBusyId !== null}
                          onClick={() => void resolveConflict(conflict, "ignore")}
                          title={t("buddy.conflictActionIgnoreTip")}
                          className="px-2.5 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-400 font-semibold cursor-pointer disabled:opacity-50 transition-colors"
                        >
                          {t("buddy.conflictActionIgnore")}
                        </button>
                      </div>
                      {/* 明细：两侧账号 / IDE / 会话 id / 双方最后消息时间 */}
                      {expanded && (
                        <div className="mt-2 rounded-ctl bg-black/30 border border-white/5 p-2.5 grid grid-cols-[64px_1fr] gap-x-2 gap-y-1 text-tiny">
                          <span className="text-slate-500">{t("buddy.conflictSourceTime")}</span>
                          <span className="text-slate-300">
                            {new Date(conflict.sourceStamp).toLocaleString()}
                            <span className="text-slate-600 ml-2">uid: {conflict.sourceUid}</span>
                          </span>
                          <span className="text-slate-500">{t("buddy.conflictTargetTime")}</span>
                          <span className="text-slate-300">
                            {new Date(conflict.targetStamp).toLocaleString()}
                            <span className="text-slate-600 ml-2">uid: {conflict.targetUid}</span>
                          </span>
                          <span className="text-slate-500">{t("buddy.conflictIde")}</span>
                          <span className="text-slate-300 break-all">{conflict.ide}</span>
                          <span className="text-slate-500">{t("buddy.conflictWorkspace")}</span>
                          <span className="text-slate-300 break-all">
                            {conflict.workspacePath || conflict.workspace}
                          </span>
                          <span className="text-slate-500">ID</span>
                          <span className="text-slate-400 break-all font-mono">{conflict.id}</span>
                        </div>
                      )}
                      {/* 对话内容预览：看真正的对话 JSON 正文，而不是只有名字和路径。
                          懒加载 —— 点「看对话」才读文件，取两侧各自的最后 40 条。 */}
                      {expanded && (
                        <div className="mt-2 flex items-center gap-1.5">
                          <button
                            onClick={() => void loadConflictMessages(conflict.id, "source")}
                            className={`px-2 py-1 rounded-md text-tiny font-semibold cursor-pointer transition-colors ${
                              conflictMessages[`${conflict.id}:source`]
                                ? "bg-white/5 text-slate-400"
                                : "bg-white/5 hover:bg-white/10 text-slate-300"
                            }`}
                          >
                            {t("buddy.conflictViewSource")}
                          </button>
                          <button
                            onClick={() => void loadConflictMessages(conflict.id, "target")}
                            className="px-2 py-1 rounded-md text-tiny font-semibold bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer transition-colors"
                          >
                            {t("buddy.conflictViewTarget")}
                          </button>
                          <span className="text-micro text-slate-600">{t("buddy.conflictPreviewHint")}</span>
                        </div>
                      )}
                      {expanded &&
                        (["source", "target"] as const).map((side) => {
                          const state = conflictMessages[`${conflict.id}:${side}`];
                          if (!state) return null;
                          return (
                            <div key={side} className="mt-1.5 rounded-ctl bg-black/40 border border-white/5 p-2">
                              <div className="text-micro font-semibold text-slate-500 mb-1">
                                {side === "source" ? t("buddy.conflictSourceShort") : t("buddy.conflictTargetShort")}
                              </div>
                              {state === "loading" ? (
                                <div className="text-tiny text-slate-500 py-2">{t("buddy.conflictLoading")}</div>
                              ) : state.length === 0 ? (
                                <div className="text-tiny text-slate-500 py-2">{t("buddy.conflictNoMessages")}</div>
                              ) : (
                                <div className="max-h-52 overflow-y-auto space-y-1.5">
                                  {state.map((message, index) => (
                                    <div key={index} className="text-tiny leading-relaxed">
                                      <span
                                        className={`mr-1.5 font-semibold flex-shrink-0 ${
                                          message.role === "user"
                                            ? "text-cyan-300"
                                            : message.role === "assistant"
                                              ? "text-emerald-300"
                                              : "text-slate-500"
                                        }`}
                                      >
                                        {t(`buddy.conflictRole.${message.role}`)}
                                      </span>
                                      <span className="text-slate-300 whitespace-pre-wrap break-all">{message.text}</span>
                                    </div>
                                  ))}
                                </div>
                              )}
                            </div>
                          );
                        })}
                    </div>
                  );
                })}
              </div>
              </>
            )}
          </div>
        </div>);

const renderConflictConfirm = () => conflictConfirm && (<div className="fixed inset-0 z-[140] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[400px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-4">
              <div className="w-9 h-9 rounded-card bg-amber-500/15 border border-amber-500/30 flex items-center justify-center">
                <AlertTriangle className="w-4 h-4 text-amber-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">
                  {t("buddy.conflictConfirmTitle", {
                    action: conflictActionLabel(conflictConfirm.action),
                  })}
                </h3>
                <p className="text-tiny text-slate-500">{t("buddy.conflictConfirmHint")}</p>
              </div>
            </div>
            <p className="text-body text-slate-300 leading-relaxed mb-5 break-all">
              {conflictConfirm.kind === "single"
                ? conflictConfirm.conflict.label
                : t("buddy.conflictConfirmBatchDesc", {
                    count: conflictConfirm.ids.length,
                    action: conflictActionLabel(conflictConfirm.action),
                  })}
            </p>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setConflictConfirm(null)}
                className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer"
              >
                {t("buddy.conflictConfirmCancel")}
              </button>
              <button
                onClick={() => {
                  const c = conflictConfirm;
                  setConflictConfirm(null);
                  if (c.kind === "single") void doResolveConflict(c.conflict, c.action);
                  else void doResolveConflictsBatch(c.ids, c.action);
                }}
                className="px-4 py-1.5 rounded-ctl text-caption bg-amber-600 hover:bg-amber-500 text-white font-semibold cursor-pointer"
              >
                {t("buddy.conflictConfirmOk")}
              </button>
            </div>
          </div>
        </div>);
  return {
    setConflictProgress,
    syncSummary,
    setSyncSummary,
    setSyncDetailsOpen,
    conflictAlertDismissed,
    setConflictAlertDismissed,
    conflictDialogOpen,
    setConflictDialogOpen,
    conflictDialogShownRef,
    sessionConflicts,
    setSessionConflicts,
    conflictPanelOpen,
    setConflictResolutions,
    conflictConfirm,
    loadConflicts,
    openConflictPanel,
    focusConflicts,
    renderConflictSummary,
    renderConflictIntro,
    renderConflictPanel,
    renderConflictConfirm,
  };
}
