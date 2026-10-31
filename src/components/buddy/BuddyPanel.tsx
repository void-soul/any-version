import { runBuddyAccountSwitch, formatBuddySwitchResult } from "./runBuddyAccountSwitch";
import { loadBuddyPlatform } from "./loadBuddyPlatform";
import { useBuddyPlatformScope } from "./useBuddyPlatformScope";
import { BuddyExpiryCell } from "./BuddyExpiryCell";
import { BuddyDeleteAccountsDialog } from "./BuddyDeleteAccountsDialog";
import { BuddyAccountUsageDialog } from "./BuddyAccountUsageDialog";
import { BuddyAutoTravelSettings } from "./BuddyAutoTravelSettings";
import { BuddyAutoCheckinSettings } from "./BuddyAutoCheckinSettings";
import { BuddyPlatformHeader } from "./BuddyPlatformHeader";
import { useBuddyAccountMaintenance } from "./useBuddyAccountMaintenance";
import { useBuddyAccountImport } from "./useBuddyAccountImport";
import { useBuddyExpiryColumns } from "./useBuddyExpiryColumns";
import { useBuddyClientPaths } from "./useBuddyClientPaths";
import { useBuddyCheckin } from "./useBuddyCheckin";
import { useBuddySessions } from "./useBuddySessions";
import { useBuddyConflictCenter } from "./useBuddyConflictCenter";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import CreditSegmentsBar from "./CreditSegmentsBar";
import TwoApiPanel from "./TwoApiPanel";
import { Plug } from "lucide-react";
import { formatQuotaNumber, mergeCreditSegments } from "./quota";
import { RefreshCw, Download, Upload, Trash2, Check, Loader2, Users, AlertTriangle, CircleAlert, CircleCheck, Plus, MessageSquareText, CalendarCheck, LogIn, X, Settings, Star } from "lucide-react";
import { ACC_BTN, COL_ACTIONS, COL_CREDITS_MIN, COL_INFO, COL_W, LABEL_TONE, PLATFORMS, TOOL_BTN, TOOL_BTN_DANGER, TOOL_BTN_PRIMARY, accountCreditSegments, describeLabel, getDosageText, getPlanBadge, parseQuotaItems, summarizeQuota, type BuddyAccount, type BuddyActionLogEntry, type BuddyAutoCheckinConfig, type BuddyAutoTravelConfig, type BuddyCheckinTasksView, type BuddyConflictProgress, type BuddyPaths, type BuddySwitchProgress, type BuddyTravelTasksView, type Tab } from "./BuddyPanel.model";
import { QuotaBar } from "./BuddyPanel.QuotaBar";

export default function BuddyPanel() {
  const { t } = useTranslation();
  const [platform, setPlatform] = useState<string>("codebuddy-cn");
  const platformScope = useBuddyPlatformScope(platform);
  const platformLabel = PLATFORMS.find((p) => p.id === platform)?.label ?? platform;
  const [tab, setTab] = useState<Tab>("accounts");
  const [accounts, setAccounts] = useState<BuddyAccount[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [paths, setPaths] = useState<BuddyPaths | null>(null);
  const [busy, setBusy] = useState(false);
  // 平台切换加载态：切换后立即清空旧平台数据并盖一层遮罩，避免快速来回切时
  // 上一个平台的账号短暂挂在新标签下（加载完成前用户也不该点到行内按钮）。
  const [platformLoading, setPlatformLoading] = useState(false);
  // 请求代次：平台每切一次 +1，异步结果回来时代次不一致就丢弃（防乱序覆盖）。
  const platformEpochRef = useRef(0);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  // 切换进度（检查客户端已退出 → 合并 → 写入 → 启动），由后端 buddy-switch-progress 事件驱动
  const [switchProgress, setSwitchProgress] = useState<BuddySwitchProgress | null>(null);
  // 冲突裁决进度（后端 buddy-conflict-progress）：裁决要递归复制整个会话目录，
  // 批量处理可能跑很久，没有进度用户只能盯着一个转圈的按钮以为卡死了
  const {
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
  } = useBuddyConflictCenter({
    platformEpochRef,
    platform,
    showMsg: (ok, text) => showMsg(ok, text),
  });
  // 客户端路径设置（切换时关闭/重启的 WorkBuddy / CodeBuddy CN）
  const {
    renderClientPaths,
  } = useBuddyClientPaths({
    setMessage,
    tab,
    showMsg: (ok, text) => showMsg(ok, text),
  });
  const {
    selectedIds,
    setSelectedIds,
    deleteIds,
    setDeleteIds,
    setUsageAccountId,
    accountView,
    setAccountView,
    togglePrimary,
    changeOrderMode,
    usageAccount,
    globalQuota,
    sortedAccounts,
    rotationCandidate,
    refreshAccount,
    refreshSelected,
    deleteAccounts,
    exportSelected,
    toggleSelect,
    toggleSelectAll,
    selectedCount,
  } = useBuddyAccountMaintenance({
    platformScope,
    showMsg: (ok, text) => showMsg(ok, text),
    accounts,
    currentId,
    setBusy,
    setMessage,
    load: () => load(),
    platform,
  });
  // 排序模式（额度 / 时间1/2/3 / 默认 last_used 倒序）
  // 过期时间列（CodeBuddy CN 专用）：列 schema + 单元格编辑 + 列增删改 + 用量弹窗 + 倒计时刷新
  const {
    expiryColumns,
    editingCell,
    setEditingCell,
    addingColumn,
    setAddingColumn,
    newColumnName,
    setNewColumnName,
    colBusy,
    now,
    setNow,
    addColumn,
    startEditCell,
    commitCell,
    clearCell,
    normalizedColumnInput,
    renderExpiryHeader,
  } = useBuddyExpiryColumns({
    platform,
    showMsg: (ok, text) => showMsg(ok, text),
    setAccounts,
  });

  // 新增账号
  const {
    showAdd,
    setShowAdd,
    setAddMode,
    setOauth,
    setTokenInput,
    importFromJson,
    renderAddAccountDialog,
  } = useBuddyAccountImport({
    platformScope,
    setBusy,
    setMessage,
    showMsg: (ok, text) => showMsg(ok, text),
    displayName: account => displayName(account),
    load: () => load(),
    platformLabel,
    busy,
    platform,
  });

  // 会话
  const {
    setSessions,
    sessionDelete,
    renderSessionsWorkspace,
    renderSessionDeleteDialog,
  } = useBuddySessions({
    platformEpochRef,
    setMessage,
    platform,
    tab,
    showMsg: (ok, text) => showMsg(ok, text),
  });

  // 自动签到
  const {
    autoConfig,
    setAutoConfig,
    setActionLogs,
    setAutoTasks,
    travelConfig,
    setTravelConfig,
    autoBusy,
    setTravelTasks,
    calendarMonth,
    loadAutoCheckin,
    loadCalendar,
    saveAutoConfig,
    runAutoCheckin,
    saveTravelConfig,
    runAutoTravel,
    renderCheckinWorkspace,
  } = useBuddyCheckin({
    setMessage,
    tab,
    showMsg: (ok, text) => showMsg(ok, text),
    load: () => load(),
    platform,
  });

  const load = useCallback(() => loadBuddyPlatform({
    scope: platformScope, platform,
    onLoaded: snapshot => {
      setAccounts(snapshot.accounts); setCurrentId(snapshot.currentId);
      setPaths(snapshot.paths); setAccountView(snapshot.view);
    },
    onError: error => setMessage({ ok: false, text: String(error) }),
    onFinished: () => setPlatformLoading(false),
  }), [platform, platformScope]);

  // 切换平台：先让旧请求作废、清空旧平台数据，再拉新列表。
  // 不清空的话，加载期间新标签下会挂着上一个平台的账号（快速来回切时尤其明显）。
  useEffect(() => {
    platformEpochRef.current += 1;
    setBusy(false);
    setMessage(null);
    setSwitchProgress(null);
    setShowAdd(false);
    setOauth(null);
    setTokenInput("");
    setDeleteIds(null);
    setUsageAccountId(null);
    setEditingCell(null);
    setAddingColumn(false);
    setAccounts([]);
    setCurrentId(null);
    setPaths(null);
    setAccountView({ orderMode: "lastUsed" });
    setSessions([]);
    setSessionConflicts([]);
    setConflictResolutions({});
    setSelectedIds(new Set());
    setPlatformLoading(true);
    void load();
  }, [platform, load]);

  // 打开面板 / 切平台时若数据已陈旧，自动同步一次用量。
  // 为什么必须自动：账号状态（含「需重新登录」徽标）只有在跑过一次刷新后被写入，
  // 只靠手点「刷新」的结果就是——面板上看着一片正常，实际令牌早就 401 了（2026-10-23 真机）。
  const autoSyncRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    if (platformLoading || accounts.length === 0) return;
    if (autoSyncRef.current.has(platform)) return;
    // usageUpdatedAt 是**秒**，与 quotaQueryLastErrorAt（毫秒）不同，别混用
    const latestSec = Math.max(0, ...accounts.map((a) => a.usageUpdatedAt ?? 0));
    const stale = latestSec === 0 || Date.now() - latestSec * 1000 > 10 * 60 * 1000;
    if (!stale) return;
    autoSyncRef.current.add(platform);
    const lease = platformScope.capture();
    const epoch = platformEpochRef.current;
    void (async () => {
      try {
        await invoke<number>("buddy_query_all_usage", { platform });
      } catch {
        // 静默：这一步只是让状态/积分变新，失败不影响列表可用
      }
      if (lease.current() && platformEpochRef.current === epoch) await load();
    })();
  }, [platform, platformLoading, accounts, load]);

  useEffect(() => {
    // 设置页现在承载自动签到配置，两个 tab 都需要加载
    if (tab === "checkin" || tab === "settings") void loadAutoCheckin();
  }, [tab, loadAutoCheckin]);

  // 自动签到事件刷新（日志 / 配置变化都会影响今日任务列表的状态）
  useEffect(() => {
    const lease = platformScope.capture();
    let disposed = false;
    const active = () => !disposed && lease.current();
    const retain = async (pending: Promise<UnlistenFn>) => {
      const fn = await pending;
      if (active()) unlisteners.push(fn); else fn();
    };
    const unlisteners: (() => void)[] = [];
    const refreshTasks = () => {
      if (!active()) return;
      void invoke<BuddyCheckinTasksView>("buddy_auto_checkin_tasks")
        .then(value => { if (active()) setAutoTasks(value); })
        .catch(() => {});
    };
    const refreshTravel = () => {
      if (!active()) return;
      void invoke<BuddyAutoTravelConfig>("buddy_auto_travel_get_config")
        .then(value => { if (active()) setTravelConfig(value); })
        .catch(() => {});
      void invoke<BuddyTravelTasksView>("buddy_auto_travel_tasks")
        .then(value => { if (active()) setTravelTasks(value); })
        .catch(() => {});
    };
    const setup = async () => {
      await retain(
        listen("buddy-action-logs-changed", () => {
          if (!active()) return;
          void invoke<BuddyActionLogEntry[]>("buddy_get_action_logs").then(value => { if (active()) setActionLogs(value); }).catch(() => {});
          refreshTasks();
          // 有新的签到/派出行事件 → 刷新日历（当天格子与明细）
          void loadCalendar(calendarMonth);
        })
      );
      if (!active()) return;
      await retain(
        listen("buddy-auto-checkin-config-changed", () => {
          if (!active()) return;
          void invoke<BuddyAutoCheckinConfig>("buddy_auto_checkin_get_config")
            .then(value => { if (active()) setAutoConfig(value); })
            .catch(() => {});
          refreshTasks();
        })
      );
      if (!active()) return;
      await retain(
        listen<BuddyConflictProgress>("buddy-conflict-progress", (e) => {
          if (!active() || e.payload.platform !== platform) return;
          // done 事件只是收尾信号，清掉进度条（结果由 invoke 返回值渲染）
          setConflictProgress(e.payload.stage === "done" ? null : e.payload);
        })
      );
      if (!active()) return;
      await retain(
        listen("buddy-auto-travel-config-changed", () => {
          if (!active()) return;
          refreshTravel();
        })
      );
    };
    void setup().catch(error => { if (active()) showMsg(false, String(error)); });
    return () => {
      disposed = true;
      for (const fn of unlisteners) fn();
    };
  }, [platform, platformScope, loadCalendar, calendarMonth]);

  const displayName = useMemo(
    () => (acc: BuddyAccount) =>
      acc.nickname?.trim() || acc.email || acc.uid || "unknown",
    []
  );

  /** `https://www.codebuddy.cn` → `www.codebuddy.cn`（tooltip 里去掉 scheme，读起来更短） */
  const shortOrigin = (origin?: string | null) => {
    const v = (origin || "").trim();
    if (!v) return "?";
    return v.replace(/^https?:\/\//i, "").replace(/\/+$/, "") || v;
  };

  const showMsg = (ok: boolean, text: string) => setMessage({ ok, text });

  // 倒计时每 30s 刷新一次（仅账号页需要）
  useEffect(() => {
    if (tab !== "accounts") return;
    const timer = setInterval(() => setNow(Date.now()), 30000);
    return () => clearInterval(timer);
  }, [tab]);

  // 账号互导：把勾选的账号导入到另一平台（目标侧按 uid/email 去重）
  const syncSelected = async () => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    const other = PLATFORMS.find((p) => p.id !== platform);
    if (!other || selectedIds.size === 0) return;
    setBusy(true);
    setMessage(null);
    try {
      const count = await invoke<number>("buddy_sync_accounts", {
        fromPlatform: platform,
        toPlatform: other.id,
        accountIds: [...selectedIds],
      });
      if (!lease.current()) return;
      showMsg(true, t("buddy.syncImported", { count, target: other.label }));
    } catch (e) {
      if (!lease.current()) return;
      showMsg(false, t("buddy.syncFailed", { error: String(e) }));
    } finally {
      if (lease.current()) {
        setBusy(false);
      }
    }
  };

  const switchAccount = async (id: string) => {
    const lease = platformScope.capture();
    if (!lease.current()) return;
    setBusy(true);
    setMessage(null);
    setSwitchProgress(null);
    setSyncSummary(null);
    setSyncDetailsOpen(false);
    // 每次切换都重新亮起冲突提醒：上一轮点过「关闭」不影响这一次
    setConflictAlertDismissed(false);
    setConflictDialogOpen(false);
    conflictDialogShownRef.current = false;
    await runBuddyAccountSwitch({
      scope: platformScope, platform, accountId: id,
      onProgress: progress => {
        setSwitchProgress(progress);
        if (progress.sync) { setSyncSummary(progress.sync); focusConflicts(progress.sync.conflict); }
      },
      onResult: async (text, report) => {
        if (report?.sync) { setSyncSummary(report.sync); focusConflicts(report.sync.conflict); }
        void loadConflicts();
        showMsg(true, formatBuddySwitchResult(text, report, changed => t("buddy.mergeResult", {
          added: changed.addedConversations, replaced: changed.replacedConversations,
          rows: changed.updatedSessionRows, workspaces: changed.scannedWorkspaces,
        })));
        await load();
      },
      onError: error => showMsg(false, String(error)),
      onFinished: () => { setSwitchProgress(null); setBusy(false); },
    });
  };

  const otherPlatform = PLATFORMS.find((p) => p.id !== platform);

  const tabs: { id: Tab; label: string; icon: React.ReactNode }[] = [
    { id: "accounts", label: t("buddy.tabAccounts"), icon: <Users className="w-3 h-3" /> },
    { id: "twoapi", label: t("buddy.tabTwoApi"), icon: <Plug className="w-3 h-3" /> },
    { id: "sessions", label: t("buddy.tabSessions"), icon: <MessageSquareText className="w-3 h-3" /> },
    { id: "checkin", label: t("buddy.tabCheckin"), icon: <CalendarCheck className="w-3 h-3" /> },
    { id: "settings", label: t("buddy.tabSettings"), icon: <Settings className="w-3 h-3" /> },
  ];

  return (
    <div className="h-full w-full flex flex-col">
      {/* 头部 */}
      {/* 头部工具栏：窄窗口下让整行折行，而不是把按钮压到文字换行 */}
      <BuddyPlatformHeader
        setPlatform={setPlatform}
        platform={platform}
        platformLoading={platformLoading}
        tabs={tabs}
        setTab={setTab}
        tab={tab}
      />

      {/* 切换进度（关闭 → 合并 → 写入 → 启动） */}
      {switchProgress && (
        <div className="flex items-center gap-2 px-4 py-2 text-caption border-b bg-sky-500/10 text-sky-300 border-sky-500/20">
          <Loader2 className="w-3 h-3 flex-shrink-0 animate-spin" />
          <span className="break-all">
            {t(`buddy.switchStage.${switchProgress.stage}`, {
              workspaces: switchProgress.scannedWorkspaces,
            })}
          </span>
        </div>
      )}

      {/* 状态提示 */}
      {message && (
        <div
          className={`flex items-center gap-2 px-4 py-2 text-caption border-b ${
            message.ok
              ? "bg-emerald-500/10 text-emerald-300 border-emerald-500/20"
              : "bg-rose-500/10 text-rose-300 border-rose-500/20"
          }`}
        >
          {message.ok ? (
            <CircleCheck className="w-3 h-3 flex-shrink-0" />
          ) : (
            <AlertTriangle className="w-3 h-3 flex-shrink-0" />
          )}
          <span className="break-all">{message.text}</span>
          <button onClick={() => setMessage(null)} className="ml-auto text-slate-500 hover:text-white cursor-pointer">✕</button>
        </div>
      )}

      {/* 冲突会话重点提醒：两侧都改过的会话 Kira 不替用户选，必须一眼看到。
          整条横幅受 ✕ 控制：关掉就是关掉，下次切换会重新亮起（switchAccount 里重置）。
          显示条件 = 有待处理冲突，或本次切换台账报了冲突（可能是旧快照，
          点「处理冲突」会拿到真实数量）。全部处理完后台账计数会被清零，
          横幅不会赖着不走。 */}
      {!conflictAlertDismissed && ((sessionConflicts.length > 0 || (syncSummary?.conflict ?? 0) > 0)) ? (
        <div className="flex items-start gap-2 px-4 py-2 text-caption border-b bg-amber-500/15 text-amber-200 border-amber-500/30">
          <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-[1px] text-amber-400" />
          <span className="min-w-0 flex-1 leading-relaxed">
            <span className="font-bold">
              {t("buddy.conflictAlertTitle", {
                // 优先显示当前账号的待处理数：台账计数是「本次切换」的快照，
                // 可能包含后来已被处理掉的，两边数字不一致只会让人困惑
                count: sessionConflicts.length || syncSummary?.conflict || 0,
              })}
            </span>
            <span className="block text-amber-200/75">{t("buddy.conflictAlertDesc")}</span>
          </span>
          {/* 唯一入口：处理冲突（openConflictPanel 会强制向后端重拉列表）。
              原「查看冲突」（跳同步明细筛选）已删 —— 看得到明细做不了决定，等于没看。 */}
          <button
            onClick={() => void openConflictPanel()}
            className="flex-shrink-0 px-2 py-0.5 rounded-md bg-amber-500/20 hover:bg-amber-500/30 font-semibold cursor-pointer transition-colors"
          >
            {t("buddy.conflictAlertResolve")}
          </button>
          <button
            onClick={() => setConflictAlertDismissed(true)}
            title={t("buddy.conflictAlertDismiss")}
            className="flex-shrink-0 text-amber-300/70 hover:text-white cursor-pointer"
          >
            ✕
          </button>
        </div>
      ) : null}

      {/* 会话同步台账：只处理有变化的会话，明细含 成功 / 跳过 / 冲突 / 失败 */}
      {syncSummary && syncSummary.total > 0 && (
        renderConflictSummary()
      )}

      {/* ─── 2API Tab ─── */}
      {tab === "twoapi" && <TwoApiPanel />}

      {/* ─── 账号 Tab ─── */}
      {tab === "accounts" && (
        <>
          <div className="flex items-center gap-2 px-4 py-1.5 border-b border-white/5 flex-shrink-0 text-tiny">
            <span className="text-slate-500">
              {platformLoading
                ? t("buddy.accountsLoading")
                : t("buddy.count", { count: accounts.length, platform: platformLabel })}
            </span>
            {globalQuota.hasData && (
              <span className="flex items-center gap-1 text-slate-500">
                <span className="text-slate-600">·</span>
                <span>{t("buddy.quotaTotal")}</span>
                {globalQuota.unlimited ? (
                  <span className="text-emerald-400 tabular-nums">∞</span>
                ) : (
                  <QuotaBar
                    remain={globalQuota.remain}
                    total={globalQuota.total}
                    className="w-20"
                    title={`${formatQuotaNumber(globalQuota.remain)}/${formatQuotaNumber(globalQuota.total)}`}
                  />
                )}
              </span>
            )}
            <select
              value={accountView.orderMode}
              onChange={(e) => void changeOrderMode(e.target.value)}
              disabled={platformLoading}
              title={t("buddy.sortBy")}
              className="bg-black/30 border border-white/10 rounded px-1 py-0.5 text-tiny text-slate-300 outline-none cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <option value="lastUsed">{t("buddy.sortDefault")}</option>
              <option value="quota">{t("buddy.sortQuota")}</option>
              <option value="expiry">{t("buddy.sortExpiry")}</option>
            </select>
            <div className="flex-1" />
            {/* 账号库专用批量操作（与「加列」同尺寸）：
                全选/取消全选 + 刷新 + 导出/导入 + 删除。
                原先摆在面板顶栏与每行操作列，顶栏按钮是全局位置但操作只对这个账号库有意义；
                行内按钮则每行重复一整套 —— 统一收进这一行，按勾选批量执行。 */}
            <button
              onClick={toggleSelectAll}
              disabled={accounts.length === 0}
              className={TOOL_BTN}
              title={t("buddy.selectAll")}
            >
              {selectedCount > 0 && selectedCount === accounts.length
                ? t("buddy.selectAllClear")
                : t("buddy.selectAll")}
            </button>
            <button
              onClick={refreshSelected}
              disabled={busy || selectedCount === 0}
              className={TOOL_BTN}
              title={t("buddy.refreshAllTitle")}
            >
              <RefreshCw className="w-3 h-3" /> {t("buddy.refreshAll")}
            </button>
            <button
              onClick={exportSelected}
              disabled={selectedCount === 0}
              className={TOOL_BTN}
              title={t("buddy.export")}
            >
              <Upload className="w-3 h-3" /> {t("buddy.export")}
            </button>
            <button
              onClick={importFromJson}
              disabled={busy || platformLoading}
              className={TOOL_BTN}
              title={t("buddy.importJson")}
            >
              <Download className="w-3 h-3" /> {t("buddy.importJson")}
            </button>
            <button
              onClick={() => selectedCount > 0 && setDeleteIds([...selectedIds])}
              disabled={selectedCount === 0}
              className={TOOL_BTN_DANGER}
              title={t("buddy.delete")}
            >
              <Trash2 className="w-3 h-3" /> {t("buddy.delete")}
            </button>
            {otherPlatform && (
              <button
                onClick={syncSelected}
                disabled={busy || selectedCount === 0}
                className={TOOL_BTN}
                title={t("buddy.syncImport", { target: otherPlatform.label })}
              >
                <Users className="w-3 h-3" /> {t("buddy.syncImport", { target: otherPlatform.label })}
              </button>
            )}
            <button
              onClick={() => {
                setAddMode("oauth");
                setOauth(null);
                setTokenInput("");
                setShowAdd(true);
              }}
              className={TOOL_BTN_PRIMARY}
            >
              <Plus className="w-3 h-3" /> {t("buddy.addAccount")}
            </button>
            {addingColumn ? (
              <div className="flex items-center gap-1">
                <input
                  autoFocus
                  value={newColumnName}
                  onChange={(e) => setNewColumnName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void addColumn();
                    } else if (e.key === "Escape") {
                      setAddingColumn(false);
                      setNewColumnName("");
                    }
                  }}
                  placeholder={t("buddy.columnNamePlaceholder")}
                  className="w-28 bg-black/40 border border-white/15 rounded px-1.5 py-1 text-tiny text-white outline-none placeholder:text-slate-600"
                />
                <button onClick={() => void addColumn()} disabled={colBusy} className="text-emerald-300 disabled:opacity-50 cursor-pointer"><Check className="w-3 h-3" /></button>
                <button onClick={() => { setAddingColumn(false); setNewColumnName(""); }} className="text-slate-500 cursor-pointer"><X className="w-3 h-3" /></button>
              </div>
            ) : (
              <button
                onClick={() => { setAddingColumn(true); setNewColumnName(""); }}
                className="inline-flex items-center gap-0.5 px-1.5 py-1 rounded-md border border-white/10 text-slate-400 hover:text-white hover:border-white/25 cursor-pointer transition"
              >
                <Plus className="w-3 h-3" />{t("buddy.addColumn")}
              </button>
            )}
          </div>

          {rotationCandidate && (
            <div className="flex items-center gap-2 px-4 py-1.5 border-b border-amber-500/20 bg-amber-500/10 text-tiny text-amber-200 flex-shrink-0">
              <AlertTriangle className="w-3 h-3 flex-shrink-0" />
              <span className="truncate">
                {t("buddy.rotation.suggest", {
                  email: rotationCandidate.label,
                  remain: formatQuotaNumber(rotationCandidate.remaining),
                })}
              </span>
              <button
                onClick={() => void switchAccount(rotationCandidate.accountId)}
                disabled={busy}
                className="ml-auto px-2 py-0.5 rounded border border-amber-400/40 hover:bg-amber-400/15 cursor-pointer transition disabled:opacity-40 flex-shrink-0"
              >
                {t("buddy.rotation.switch")}
              </button>
            </div>
          )}

          <div className="flex-1 overflow-auto relative">
            {/* 平台切换遮罩：加载完成前挡住整张表（含行内按钮），避免误点旧平台的账号行 */}
            {platformLoading && (
              <div
                className="absolute inset-0 z-30 flex items-center justify-center bg-slate-950/80 backdrop-blur-[1px]"
                aria-live="polite"
                aria-busy="true"
              >
                <div className="flex items-center gap-2 text-caption text-slate-300">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  {t("buddy.accountsLoading")}
                </div>
              </div>
            )}
            {accounts.length === 0 ? (
              platformLoading ? null : (
                <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-3 p-6">
                  <Users className="w-10 h-10 text-slate-600" />
                  <p className="text-body">{t("buddy.noAccounts")}</p>
                  <p className="text-tiny text-slate-600 max-w-sm text-center">{t("buddy.noAccountsHint")}</p>
                  <button
                    onClick={() => {
                      setAddMode("oauth");
                      setOauth(null);
                      setTokenInput("");
                      setShowAdd(true);
                    }}
                    className="mt-1 px-3 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold cursor-pointer transition"
                  >
                    {t("buddy.addAccount")}
                  </button>
                </div>
              )
            ) : (
              <div
                style={{
                  // 积分列是填充列，这里只按它的最小宽度算（窗口更宽时它自己会长）
                  minWidth:
                    COL_INFO + COL_CREDITS_MIN + (expiryColumns.length + 1) * COL_W + COL_ACTIONS,
                }}
              >
                {/* 表头 */}
                {renderExpiryHeader()}

                {sortedAccounts.map((acc) => {
                  const isCurrent = acc.id === currentId;
                  const isSelected = selectedIds.has(acc.id);
                  const isPrimary = accountView.primaryAccountId === acc.id;
                  const quota = summarizeQuota(parseQuotaItems(acc.quotaRaw));
                  const planBadge = getPlanBadge(acc);
                  const cellEditingAcc = editingCell && editingCell.accId === acc.id ? editingCell : null;
                  // 认证域提示统一在这里生成：账号列只放一个圆点，悬停才看域名。
                  const originTip = acc.authOrigin
                    ? acc.authOriginMatches === false
                      ? t("buddy.authOriginMismatchTip", {
                          origin: shortOrigin(acc.authOrigin),
                          client: shortOrigin(acc.platformAuthOrigin),
                        })
                      : t("buddy.authOriginMatchTip", {
                          origin: shortOrigin(acc.authOrigin),
                          client: shortOrigin(acc.platformAuthOrigin),
                        })
                    : undefined;
                  return (
                    <div
                      key={acc.id}
                      // 当前账号 = **整行的绿色描边**（原来是一个「当前」标签：它挤在账号名
                      // 旁边，占了本就不够的账号列宽度，而且不如整行描边一眼可见）。
                      // 用 ring 而不是 border：border 会占掉 1px 布局，行与表头就对不齐了。
                      className={`flex items-stretch border-b border-white/5 text-tiny ${
                        isCurrent
                          ? "bg-emerald-500/[0.06] ring-1 ring-inset ring-emerald-400/50"
                          : isSelected
                            ? "bg-sky-500/[0.06]"
                            : "hover:bg-white/[0.03]"
                      }`}
                    >
                      {/* 账号信息 */}
                      <div style={{ width: COL_INFO }} className="flex items-center gap-1.5 px-3 py-1.5 flex-shrink-0 min-w-0">
                        <input
                          type="checkbox"
                          checked={isSelected}
                          onChange={() => toggleSelect(acc.id)}
                          className="accent-[var(--module-accent)] w-3 h-3 flex-shrink-0 cursor-pointer"
                        />
                        <button
                          onClick={() => void togglePrimary(acc.id)}
                          title={t("buddy.primaryToggle")}
                          className={`flex-shrink-0 cursor-pointer transition ${
                            isPrimary ? "text-amber-300" : "text-slate-600 hover:text-amber-300"
                          }`}
                        >
                          <Star className="w-3 h-3" fill={isPrimary ? "currentColor" : "none"} />
                        </button>
                        <span
                          className={`inline-flex items-center text-micro px-1.5 py-0.5 rounded border flex-shrink-0 ${
                            planBadge === "PRO"
                              ? "bg-amber-500/15 text-amber-300 border-amber-500/25"
                              : planBadge === "TRIAL"
                                ? "bg-sky-500/15 text-sky-300 border-sky-500/25"
                                : planBadge === "ENTERPRISE"
                                  ? "bg-[var(--module-accent-soft)] text-violet-300 border-violet-500/25"
                                  : "bg-slate-500/15 text-slate-400 border-slate-500/25"
                          }`}
                        >
                          {t(`buddy.plan.${planBadge.toLowerCase()}`, { defaultValue: planBadge })}
                        </span>
                        {/* 「当前」不再占账号列的位置：整行有绿色描边（见行的 className） */}
                        {/* 认证域：只留一个同色圆点（绿=与客户端同域；琥珀=跨域）。
                            完整域名太长，会把账号名挤掉；悬停圆点才显示，不占列宽。 */}
                        {acc.authOrigin && (
                          // 外层 12px 命中区、内层 6px 圆点：点本身够小不占位，悬停却容易命中
                          // （直接给 6px 的 span 加 title，鼠标得精准停在那个点上才看得到域名）
                          <span
                            title={originTip}
                            role="img"
                            aria-label={originTip}
                            className="inline-flex h-3 w-3 flex-shrink-0 items-center justify-center cursor-help"
                          >
                            <span
                              className={`h-1.5 w-1.5 rounded-full ${
                                acc.authOriginMatches === false ? "bg-amber-400" : "bg-emerald-400"
                              }`}
                            />
                          </span>
                        )}
                        {/* 登录态被服务端吊销：刷新救不回来，只能重新登录/重新导入。
                            只留一个红色感叹号，悬停看原因，避免占账号列的宽度。 */}
                        {acc.status === "auth_revoked" && (
                          <span
                            title={t("buddy.loginRevokedTip", {
                              reason: acc.statusReason ?? "-",
                            })}
                            role="img"
                            aria-label={t("buddy.loginRevokedTip", {
                              reason: acc.statusReason ?? "-",
                            })}
                            className="inline-flex h-3.5 w-3.5 flex-shrink-0 items-center justify-center text-rose-400 cursor-help"
                          >
                            <CircleAlert className="h-3.5 w-3.5" />
                          </span>
                        )}
                        <span className="font-semibold text-white truncate max-w-[150px]" title={acc.email}>{displayName(acc)}</span>
                      </div>
                      {/* 积分：构成条（一段一方块，块内是剩余天数）+ 右侧剩余总额 —— 填充列 */}
                      <div
                        style={{ minWidth: COL_CREDITS_MIN }}
                        className="flex-1 min-w-0 flex items-center px-2 border-l border-white/5"
                      >
                        {quota.unlimited ? (
                          <span className="text-emerald-400">∞</span>
                        ) : (
                          <CreditSegmentsBar
                            segments={mergeCreditSegments(accountCreditSegments(acc))}
                            now={now}
                            onClick={() => setUsageAccountId(acc.id)}
                          />
                        )}
                      </div>
                      {/* 登录 token 到期：只读倒计时（`55.21` = 55 天 21 小时；已过带负号） */}
                      <div style={{ width: COL_W }} className="px-1 py-1 flex-shrink-0 border-l border-white/5">
                        {(() => {
                          const raw = acc.expiresAt ?? 0;
                          if (raw <= 0) return <span className="text-slate-600 px-1">—</span>;
                          const ms = raw < 1e12 ? raw * 1000 : raw; // 秒/毫秒都兼容
                          const st = describeLabel(ms, now);
                          if (st.recovered) {
                            return (
                              <span
                                title={`${t("buddy.expiresAt")} · ${st.absolute}`}
                                className="block w-full rounded border px-1.5 py-0.5 tabular-nums bg-rose-500/10 border-rose-500/25 text-rose-300"
                              >
                                {t("buddy.overdue")}
                              </span>
                            );
                          }
                          const tone = LABEL_TONE[st.tone];
                          return (
                            <span
                              title={`${t("buddy.expiresAt")} · ${st.absolute}`}
                              className={`relative overflow-hidden block rounded border px-1.5 py-0.5 w-full ${tone.chip}`}
                            >
                              <div className={`absolute left-0 top-0 bottom-0 ${tone.bar}`} style={{ width: `${st.progress}%` }} />
                              <span className={`relative tabular-nums ${tone.text}`}>{st.text}</span>
                            </span>
                          );
                        })()}
                      </div>
                      {/* 每个过期时间列一个值 */}
                      {expiryColumns.map(col => <BuddyExpiryCell
                        key={col.id} acc={acc} col={col} cellEditingAcc={cellEditingAcc}
                        setEditingCell={setEditingCell} now={now} startEditCell={startEditCell}
                        normalizedColumnInput={normalizedColumnInput} commitCell={commitCell} clearCell={clearCell}
                      />)}
                      {/* 操作 */}
                      <div style={{ width: COL_ACTIONS }} className="flex items-center justify-end gap-1 px-1 flex-shrink-0 border-l border-white/5">
                        <button
                          onClick={() => switchAccount(acc.id)}
                          disabled={busy || isCurrent}
                          className={ACC_BTN}
                          title={
                            acc.authOriginMatches === false
                              ? originTip
                              : t("buddy.switch")
                          }
                        >
                          <LogIn className="w-3 h-3" />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </>
      )}

      {/* ─── 会话 Tab ─── */}
      {tab === "sessions" && (
        renderSessionsWorkspace()
      )}

      {/* ─── 签到 Tab ─── */}
      {tab === "checkin" && (
        renderCheckinWorkspace()
      )}

      {/* 路径信息 */}
      {/* ─── 设置 Tab：自动签到 + 客户端路径 ─── */}
      {tab === "settings" && (
        <div className="flex-1 overflow-y-auto px-4 py-3 space-y-3">
          <BuddyAutoCheckinSettings
            autoConfig={autoConfig}
            setAutoConfig={setAutoConfig}
            saveAutoConfig={saveAutoConfig}
            autoBusy={autoBusy}
            runAutoCheckin={runAutoCheckin}
          />
          <BuddyAutoTravelSettings
            travelConfig={travelConfig}
            setTravelConfig={setTravelConfig}
            saveTravelConfig={saveTravelConfig}
            autoBusy={autoBusy}
            runAutoTravel={runAutoTravel}
          />
          {renderClientPaths()}
        </div>
      )}

      {paths && (paths.stateDb || paths.authFile || paths.dataDir) && (
        <div className="px-4 py-2 border-t border-white/5 flex-shrink-0 flex flex-wrap gap-x-4 gap-y-1 text-micro text-slate-600">
          {paths.dataDir && <span>{t("buddy.dataDir")}: {paths.dataDir}</span>}
          {paths.stateDb && <span>state.vscdb: {paths.stateDb}</span>}
          {paths.authFile && <span>auth: {paths.authFile}</span>}
        </div>
      )}

      {/* 用量详情弹窗（各账号「用量」按钮） */}
      {usageAccount &&
        (() => {
          const quotaItems = parseQuotaItems(usageAccount.quotaRaw);
          const dosageText = getDosageText(usageAccount);
          const planBadge = getPlanBadge(usageAccount);
          return (
            <BuddyAccountUsageDialog
              displayName={displayName}
              usageAccount={usageAccount}
              planBadge={planBadge}
              setUsageAccountId={setUsageAccountId}
              quotaItems={quotaItems}
              dosageText={dosageText}
              refreshAccount={refreshAccount}
              busy={busy}
            />
          );
        })()}

      {/* 删除确认（单个删除 / 批量删除共用） */}
      {deleteIds && (
        <BuddyDeleteAccountsDialog
          setDeleteIds={setDeleteIds}
          deleteIds={deleteIds}
          accounts={accounts}
          displayName={displayName}
          deleteAccounts={deleteAccounts}
          busy={busy}
        />
      )}

      {/* 会话删除确认 */}
      {sessionDelete && (
        renderSessionDeleteDialog()
      )}

      {/* 冲突确认框：必须显式点一个按钮才能关（点背景不关），
          否则「冲突被静默放过」这个问题会原样复发 */}
      {conflictDialogOpen && syncSummary && syncSummary.conflict > 0 && (
        renderConflictIntro()
      )}

      {/* 冲突处理弹窗：逐条裁决（查看明细 / 合并 / 覆盖目标 / 保留目标）。
          冲突在切换时被搁置（不猜不覆盖），这里就是补上「用户来决定」的出口。 */}
      {conflictPanelOpen && (
        renderConflictPanel()
      )}

      {/* 裁决二次确认弹窗：合并 / 覆盖 / 保留都会改会话内容且不可撤销，误点代价高，
          所以统一先确认再执行（单条与批量共用）。
          层级压在冲突处理面板（z-130）之上；modal-mask / rounded-panel / rounded-ctl /
          bg-slate-900/95 这些 class 都走主题变量，深浅色主题自动跟随。 */}
      {conflictConfirm && (
        renderConflictConfirm()
      )}

      {/* 新增账号弹窗 */}
      {showAdd && (
        renderAddAccountDialog()
      )}
    </div>
  );
}

export { type BuddyAccount, type BuddyPaths, type BuddySessionSyncStatus, type BuddySessionSyncDetail, type BuddySessionSyncSummary, type BuddyTransferReport, type BuddyPendingConflict, type BuddyConflictResolution, type BuddyConflictMessage, type BuddyGrowthTask, type BuddyGrowthOverview, type BuddySwitchProgress, type BuddyConflictProgress, type BuddySessionRecord, type BuddySessionForkReport, type BuddySessionMoveReport, type BuddySessionDeleteReport, type BuddyAutoCheckinConfig, type BuddyAccountTravelState, type BuddyAutoTravelConfig, type BuddyActionLogEntry, type BuddyCheckinTask, type BuddyCheckinTasksView, type BuddyTravelTask, type BuddyTravelTasksView, type BuddyDailyCheckinRecord, type BuddyDailyTravelRecord, type BuddyDailyAccountRecord, type BuddyDailyRecords, type CheckinStatus, type BuddyAccountView } from "./BuddyPanel.model";
