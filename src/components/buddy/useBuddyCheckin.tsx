import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { RefreshCw, Loader2, Gauge, CalendarCheck, Eraser, ChevronDown, ChevronRight, ChevronLeft, CalendarDays, ListChecks, Cat } from "lucide-react";
import { GROWTH_CAT_STATE_KEY, addMinutes, buildMonthCells, dayProgress, localDateStr, localMonthStr, monthBounds, shiftMonthStr, type BuddyAccount, type BuddyActionLogEntry, type BuddyAutoCheckinConfig, type BuddyAutoTravelConfig, type BuddyCheckinTasksView, type BuddyDailyRecords, type BuddyGrowthOverview, type BuddyTravelTasksView, type Tab } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the BuddyCheckin feature, independently of page rendering. */
export function useBuddyCheckin({ setMessage, tab, showMsg, load, platform }: {
  setMessage: Dispatch<SetStateAction<{ ok: boolean; text: string; } | null>>;
  tab: Tab;
  showMsg: (ok: boolean, text: string) => void;
  load: () => Promise<void>;
  platform: string;
}) {
  const { t } = useTranslation();

  // 自动签到
  const [autoConfig, setAutoConfig] = useState<BuddyAutoCheckinConfig | null>(null);

  const [actionLogs, setActionLogs] = useState<BuddyActionLogEntry[]>([]);

  const [autoTasks, setAutoTasks] = useState<BuddyCheckinTasksView | null>(null);

  const [travelConfig, setTravelConfig] = useState<BuddyAutoTravelConfig | null>(null);

  const [autoBusy, setAutoBusy] = useState(false);

  // 签到/派旅行只对 WorkBuddy 账号有效：状态卡固定用 WorkBuddy 账号列表，
  // 与顶部的平台选择解耦（否则在默认的 CodeBuddy CN 标签下会全部显示"待生成/未安排"）。
  const [wbAccounts, setWbAccounts] = useState<BuddyAccount[]>([]);

  // 成长计划视图（任务进度 / 连续活跃 / 猫猫旅行 / 奖励），仅 WorkBuddy 账号
  const [growth, setGrowth] = useState<BuddyGrowthOverview | null>(null);

  const [growthTargetId, setGrowthTargetId] = useState<string | null>(null);

  const [growthBusy, setGrowthBusy] = useState(false);

  const [growthError, setGrowthError] = useState<string | null>(null);

  const [travelTasks, setTravelTasks] = useState<BuddyTravelTasksView | null>(null);

  // 日历：当前月份 + 该月归档 + 选中某天
  const [calendarMonth, setCalendarMonth] = useState<string>(() => localMonthStr());

  const [dailyRecords, setDailyRecords] = useState<BuddyDailyRecords>({});

  const [calendarDay, setCalendarDay] = useState<string | null>(null);

  // 积分页各区块（成长计划/账号状态/日历/日志）的折叠状态
  const [checkinCollapsed, setCheckinCollapsed] = useState({
    growth: false,
    accounts: false,
    calendar: false,
    logs: false,
  });

  const toggleCheckinSection = (key: "growth" | "accounts" | "calendar" | "logs") =>
    setCheckinCollapsed((c) => ({ ...c, [key]: !c[key] }));

  // 成长计划：一次拉齐任务进度/奖励/连续活跃/猫猫旅行（force=true 绕过后端 60 秒缓存）
  const loadGrowth = useCallback(async (accountId: string | null, force = false) => {
    if (!accountId) {
      setGrowth(null);
      setGrowthError(null);
      return;
    }
    setGrowthBusy(true);
    try {
      const view = await invoke<BuddyGrowthOverview>("buddy_growth_overview", {
        platform: "workbuddy",
        accountId,
        force,
      });
      setGrowth(view);
      setGrowthError(null);
    } catch (e) {
      setGrowth(null);
      setGrowthError(String(e));
    } finally {
      setGrowthBusy(false);
    }
  }, []);

  // 成长计划卡展示的账号（与 wbAccounts 同步）
  const growthTarget = useMemo(
    () => wbAccounts.find((account) => account.id === growthTargetId) ?? null,
    [wbAccounts, growthTargetId]
  );

  // 自动签到配置/行为日志/今日任务列表/旅行配置加载（签到与派出仅限 WorkBuddy）
  const loadAutoCheckin = useCallback(async () => {
    try {
      const [config, logs, tasks, travel, travelTaskView, wb] = await Promise.all([
        invoke<BuddyAutoCheckinConfig>("buddy_auto_checkin_get_config"),
        invoke<BuddyActionLogEntry[]>("buddy_get_action_logs"),
        invoke<BuddyCheckinTasksView>("buddy_auto_checkin_tasks"),
        invoke<BuddyAutoTravelConfig>("buddy_auto_travel_get_config"),
        invoke<BuddyTravelTasksView>("buddy_auto_travel_tasks"),
        // 状态卡固定用 WorkBuddy 账号：与签到/派出的数据源保持一致
        invoke<BuddyAccount[]>("buddy_list_accounts", { platform: "workbuddy" }),
      ]);
      setAutoConfig(config);
      setActionLogs(logs ?? []);
      setAutoTasks(tasks);
      setTravelConfig(travel);
      setTravelTasks(travelTaskView);
      setWbAccounts(wb ?? []);
      // 成长计划取第一个 WorkBuddy 账号（与签到/派出的数据源一致）
      const growthTarget = (wb ?? [])[0] ?? null;
      setGrowthTargetId(growthTarget?.id ?? null);
      void loadGrowth(growthTarget?.id ?? null);
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    }
  }, [loadGrowth]);

  // 日历归档加载（按月份查询）
  const loadCalendar = useCallback(async (month: string) => {
    try {
      const { from, to } = monthBounds(month);
      const records = await invoke<BuddyDailyRecords>("buddy_get_daily_records", { from, to });
      setDailyRecords(records ?? {});
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    }
  }, []);

  useEffect(() => {
    if (tab === "checkin") void loadCalendar(calendarMonth);
  }, [tab, calendarMonth, loadCalendar]);

  // ─── 自动签到 ───

  const saveAutoConfig = async () => {
    if (!autoConfig) return;
    setAutoBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_auto_checkin_save_config", { config: autoConfig });
      showMsg(true, t("buddy.autoSaved"));
      await loadAutoCheckin();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

  const runAutoCheckin = async (force: boolean) => {
    setAutoBusy(true);
    setMessage(null);
    try {
      const result = await invoke<string>("buddy_auto_checkin_run", { force });
      showMsg(true, t(`buddy.autoRun.${result}`, { defaultValue: result }));
      await loadAutoCheckin();
      await loadCalendar(calendarMonth);
      await load();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

  const clearAutoLogs = async () => {
    try {
      await invoke("buddy_clear_action_logs");
      setActionLogs([]);
    } catch (e) {
      showMsg(false, String(e));
    }
  };

  const saveTravelConfig = async () => {
    if (!travelConfig) return;
    setAutoBusy(true);
    setMessage(null);
    try {
      await invoke("buddy_auto_travel_save_config", { config: travelConfig });
      showMsg(true, t("buddy.travel.saved"));
      await loadAutoCheckin();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

  const runAutoTravel = async () => {
    setAutoBusy(true);
    setMessage(null);
    try {
      await invoke<string>("buddy_auto_travel_run", { force: true });
      showMsg(true, t("buddy.travel.runDone"));
      await loadAutoCheckin();
      await loadCalendar(calendarMonth);
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setAutoBusy(false);
    }
  };

const renderCheckinWorkspace = () => (<div className="flex-1 overflow-y-auto p-4">
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-4 items-start">
            {/* 成长计划：任务进度 / 奖励 / 连续活跃 / 猫猫旅行 / Buddy（仅 WorkBuddy） */}
            <div className="xl:col-span-2 rounded-card border border-white/10 bg-white/[0.03] p-4">
              <div className="flex items-center gap-2 mb-3">
                <button
                  type="button"
                  onClick={() => toggleCheckinSection("growth")}
                  className="flex items-center gap-2 cursor-pointer select-none"
                >
                  {checkinCollapsed.growth ? <ChevronRight className="w-4 h-4 text-slate-500" /> : <ChevronDown className="w-4 h-4 text-slate-500" />}
                  <Gauge className="w-4 h-4 text-[var(--module-accent)]" />
                  <span className="text-title font-bold text-white">{t("buddy.growth.title")}</span>
                </button>
                <span className="text-caption text-slate-500 truncate max-w-[40%]">
                  {growthTarget
                    ? growthTarget.nickname || growthTarget.email || growthTarget.id
                    : t("buddy.growth.noAccount")}
                </span>
                <div className="flex-1" />
                <button
                  type="button"
                  disabled={!growthTargetId || growthBusy}
                  onClick={() => void loadGrowth(growthTargetId, true)}
                  className="flex items-center gap-1 px-2 py-1 rounded-ctl text-caption bg-white/5 hover:bg-white/10 disabled:opacity-40 cursor-pointer"
                >
                  {growthBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
                  {t("buddy.growth.refresh")}
                </button>
              </div>
              {!checkinCollapsed.growth && (<>
              {growthError && <div className="text-caption text-amber-300 break-all">{growthError}</div>}
              {!growthError && !growth && (
                <div className="text-caption text-slate-500">
                  {growthBusy ? t("buddy.growth.loading") : t("buddy.growth.empty")}
                </div>
              )}
              {growth && (
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-caption">
                  <div className="rounded-ctl bg-white/[0.03] border border-white/5 p-2.5">
                    <div className="flex items-center gap-2 mb-1.5">
                      <span className="font-semibold text-slate-300">{t("buddy.growth.taskProgress")}</span>
                      <span className="ml-auto text-slate-400">
                        {t("buddy.growth.taskCompletedOf", {
                          completed: growth.growth.completed,
                          total: growth.growth.total,
                        })}
                      </span>
                    </div>
                    <div className="h-1.5 rounded-full bg-white/10 overflow-hidden mb-2">
                      <div
                        className="h-full bg-[var(--module-accent)]"
                        style={{ width: `${Math.round(growth.growth.ratio * 100)}%` }}
                      />
                    </div>
                    <div className="space-y-1 max-h-40 overflow-y-auto">
                      {growth.growth.tasks.map((task, index) => (
                        <div key={`${task.taskCode}-${index}`} className="flex items-start gap-2">
                          <span
                            className={`flex-shrink-0 ${
                              task.state === "claimed" || task.state === "completed"
                                ? "text-emerald-400"
                                : "text-slate-400"
                            }`}
                          >
                            {t(`buddy.growth.taskState.${task.state}`, { defaultValue: task.state })}
                          </span>
                          <span className="min-w-0 flex-1 break-all text-slate-300" title={task.guide || undefined}>
                            {task.title}
                          </span>
                          <span className="flex-shrink-0 text-slate-500">
                            {task.current}/{task.target}
                          </span>
                        </div>
                      ))}
                    </div>
                  </div>
                  <div className="rounded-ctl bg-white/[0.03] border border-white/5 p-2.5 space-y-2">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-slate-300">{t("buddy.growth.rewardProgress")}</span>
                      <span className="ml-auto text-slate-400">
                        {t("buddy.growth.rewardClaimedOf", {
                          claimed: growth.rewards.claimed,
                          total: growth.rewards.total,
                          pending: growth.rewards.pending,
                        })}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-slate-300">{t("buddy.growth.streak")}</span>
                      <span className="ml-auto text-slate-400">
                        {growth.streak?.days != null
                          ? t("buddy.growth.streakDays", { days: growth.streak.days })
                          : t("buddy.growth.streakUnavailable")}
                      </span>
                    </div>
                    {growth.streak && growth.streak.tiers.length > 0 && (
                      <div className="flex items-center gap-3 pl-1 flex-wrap">
                        {growth.streak.tiers.map((tier) => (
                          <span
                            key={tier.key}
                            className={tier.status === "claimed" ? "text-emerald-400" : "text-slate-500"}
                          >
                            {tier.key}
                            {t(`buddy.growth.tierStatus.${tier.status}`, { defaultValue: tier.status })}
                          </span>
                        ))}
                        {growth.streak.nextTierRemaining != null && growth.streak.nextTier && (
                          <span className="text-slate-500">
                            {t("buddy.growth.streakNextTier", {
                              tier: growth.streak.nextTier,
                              days: growth.streak.nextTierRemaining,
                            })}
                          </span>
                        )}
                        {growth.streak.makeupCards != null && (
                          <span className="text-slate-500">
                            {t("buddy.growth.streakMakeup", { count: growth.streak.makeupCards })}
                          </span>
                        )}
                      </div>
                    )}
                    <div className="flex items-start gap-2">
                      <span className="font-semibold text-slate-300">{t("buddy.growth.travel")}</span>
                      <span className="ml-auto text-right text-slate-400">
                        {t(GROWTH_CAT_STATE_KEY[growth.cat.state] ?? "buddy.growth.travelUnknown", {
                          defaultValue: growth.cat.state,
                        })}
                        {growth.cat.arriveAt
                          ? ` · ${t("buddy.growth.travelArriveAt", {
                              time: new Date(growth.cat.arriveAt).toLocaleString(),
                            })}`
                          : ""}
                        {growth.cat.rewardCredits > 0
                          ? ` · ${t("buddy.growth.travelReward", { credits: growth.cat.rewardCredits })}`
                          : ""}
                      </span>
                    </div>
                    <div className="flex items-start gap-2">
                      <span className="font-semibold text-slate-300">{t("buddy.growth.buddies")}</span>
                      <span className="ml-auto text-right text-slate-400 break-all">
                        {growth.unlocked
                          ? growth.cat.buddies.map((buddy) => buddy.name).join("、") ||
                            t("buddy.growth.buddyActive")
                          : growth.buddyKnown
                            ? t("buddy.growth.buddyLocked")
                            : t("buddy.growth.buddyUnknown")}
                      </span>
                    </div>
                    {(growth.actions.gachaAvailable || growth.actions.lotteryAvailable) && (
                      <div className="flex items-start gap-2">
                        <span className="font-semibold text-slate-300">{t("buddy.growth.actions")}</span>
                        <span className="ml-auto text-right text-slate-400">
                          {growth.actions.gachaAvailable
                            ? t("buddy.growth.gacha", {
                                count: growth.actions.gachaCount ?? 0,
                                cost: growth.actions.gachaCost ?? 0,
                                energy: growth.actions.gachaEnergy ?? 0,
                              })
                            : ""}
                          {growth.actions.lotteryAvailable
                            ? ` ${t("buddy.growth.lottery", { count: growth.actions.lotteryCount ?? 0 })}`
                            : ""}
                        </span>
                      </div>
                    )}
                  </div>
                  {growth.manualTasks.length > 0 && (
                    <div className="md:col-span-2 text-slate-500">
                      {t("buddy.growth.manualTasks", { tasks: growth.manualTasks.join("、") })}
                    </div>
                  )}
                  {growth.warnings.length > 0 && (
                    <div className="md:col-span-2 text-tiny text-slate-600 break-all">
                      {t("buddy.growth.warnings", { count: growth.warnings.length })}
                    </div>
                  )}
                </div>
              )}
              </>)}
            </div>
            {/* 每账号状态（签到 | 派出）—— 固定用 WorkBuddy 账号：签到/派出只对 WorkBuddy 有效 */}
            <div className="xl:col-span-2 rounded-card border border-white/10 bg-white/[0.03] p-4">
              <div className="flex items-center gap-2 mb-3">
              <button type="button" onClick={() => toggleCheckinSection("accounts")} className="flex items-center gap-2 cursor-pointer select-none">
                {checkinCollapsed.accounts ? <ChevronRight className="w-4 h-4 text-slate-500" /> : <ChevronDown className="w-4 h-4 text-slate-500" />}
                <ListChecks className="w-4 h-4 text-[var(--module-accent)]" />
                <span className="text-title font-bold text-white">{t("buddy.accountStatus.title")}</span>
              </button>
                <div className="flex-1" />
                <button
                  onClick={() => {
                    void loadAutoCheckin();
                    void loadCalendar(calendarMonth);
                  }}
                  className="px-2 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition"
                >
                  <RefreshCw className="w-3 h-3" /> {t("buddy.auto.refresh")}
                </button>
              </div>
              {!checkinCollapsed.accounts && (<>
              {platform !== "workbuddy" && (
                <p className="text-micro text-amber-300/80 mb-2">{t("buddy.accountStatus.wbOnly")}</p>
              )}
              {wbAccounts.length === 0 ? (
                <div className="text-tiny text-slate-600">{t("buddy.auto.noTasks")}</div>
              ) : (
                <div className="grid grid-cols-4 gap-1.5 items-start">
                  {wbAccounts.map((acc) => {
                    const task = autoTasks?.tasks.find((x) => x.accountId === acc.id);
                    const travel = travelTasks?.tasks.find((x) => x.accountId === acc.id);
                    // 签到状态
                    let checkinState: string;
                    let checkinCls: string;
                    if (task?.status === "success") {
                      checkinState = t("buddy.accountStatus.checkinDone");
                      checkinCls = "text-emerald-300";
                    } else if (task?.status === "failed") {
                      checkinState = t("buddy.accountStatus.checkinFailed");
                      checkinCls = "text-rose-300";
                    } else if (task?.status === "unfinished") {
                      // 计划时间已错过且今日窗口已结束：今天不再执行（等明天重新随机安排）
                      checkinState = t("buddy.accountStatus.unfinished");
                      checkinCls = "text-slate-500";
                    } else if (autoTasks && !autoTasks.enabled) {
                      checkinState = t("buddy.accountStatus.notEnabled");
                      checkinCls = "text-slate-500";
                    } else if (task?.scheduledTime) {
                      checkinState = t("buddy.accountStatus.checkinPending");
                      checkinCls = "text-slate-300";
                    } else {
                      checkinState = t("buddy.accountStatus.checkinNotGenerated");
                      checkinCls = "text-slate-400";
                    }
                    // 派出状态（后端 buddy_auto_travel_tasks：状态与时间都来自每日归档）
                    let travelState: string;
                    let travelCls: string;
                    switch (travel?.status) {
                      case "claimed":
                        travelState = t("buddy.travel.stateClaimed", { credit: travel.credit ?? 0 });
                        travelCls = "text-emerald-300";
                        break;
                      case "limit_reached":
                        travelState = t("buddy.travel.stateLimit");
                        travelCls = "text-slate-400";
                        break;
                      case "traveling":
                        travelState = t("buddy.travel.stateTraveling");
                        travelCls = "text-sky-300";
                        break;
                      case "arrived":
                        travelState = t("buddy.travel.stateArrived");
                        travelCls = "text-amber-300";
                        break;
                      case "pending":
                        travelState = t("buddy.travel.statePending", {
                          time: travel?.planTime ?? "—",
                        });
                        travelCls = "text-slate-300";
                        break;
                      case "rejected":
                        travelState = t("buddy.travel.stateRejected");
                        travelCls = "text-rose-300";
                        break;
                      case "failed":
                        travelState = t("buddy.travel.stateFailed");
                        travelCls = "text-rose-300";
                        break;
                      case "unfinished":
                        travelState = t("buddy.accountStatus.unfinished");
                        travelCls = "text-slate-500";
                        break;
                      default:
                        travelState =
                          travelTasks && !travelTasks.enabled
                            ? t("buddy.accountStatus.notEnabled")
                            : t("buddy.travel.stateNotToday");
                        travelCls = "text-slate-500";
                    }
                    return (
                      <div key={acc.id} className="rounded-ctl border border-white/5 bg-black/20 px-2.5 py-2 min-w-0">
                        <div className="text-tiny text-slate-200 truncate mb-1.5" title={acc.email}>
                          {acc.email || acc.id}
                        </div>
                        <div className="flex flex-col gap-1.5">
                          {/* 签到 */}
                          <div className="space-y-0.5 min-w-0">
                            <div className="flex items-center gap-1.5">
                              <CalendarCheck className="w-3 h-3 text-slate-500 flex-shrink-0" />
                              <span className={`text-tiny font-semibold ${checkinCls}`}>{checkinState}</span>
                            </div>
                            <div className="text-micro text-slate-500 pl-[18px]">
                              {t("buddy.accountStatus.planTime", { time: task?.scheduledTime ?? "—" })}
                              {task?.lastCheckinTime
                                ? ` · ${t("buddy.accountStatus.actualTime", { time: task.lastCheckinTime })}`
                                : ""}
                            </div>
                          </div>
                          {/* 派出 */}
                          <div className="space-y-0.5 min-w-0">
                            <div className="flex items-center gap-1.5">
                              <Cat className="w-3 h-3 text-slate-500 flex-shrink-0" />
                              <span className={`text-tiny font-semibold ${travelCls}`}>{travelState}</span>
                            </div>
                            <div className="text-micro text-slate-500 pl-[18px]">
                              {t("buddy.accountStatus.travelCount", { count: travel?.departCount ?? 0 })}
                            </div>
                            <div className="text-micro text-slate-500 pl-[18px]">
                              {t("buddy.accountStatus.planTime", { time: travel?.planTime ?? "—" })}
                              {travel?.departTime
                                ? ` · ${t("buddy.accountStatus.departTime", { time: travel.departTime })}`
                                : ""}
                              {travel?.backTime
                                ? ` · ${t("buddy.accountStatus.backTime", { time: travel.backTime })}`
                                : ""}
                            </div>
                            {travel?.status === "traveling" && travel.departTime && (
                              <div className="text-micro text-slate-500 pl-[18px]">
                                {t("buddy.accountStatus.expectedReturn", {
                                  from: addMinutes(travel.departTime, 60),
                                  to: addMinutes(travel.departTime, 240),
                                })}
                              </div>
                            )}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
              </>)}
            </div>

            {/* 日历：逐日逐账号的计划与实绩（数据来自每日归档） */}
            <div className="xl:col-span-2 rounded-card border border-white/10 bg-white/[0.03] p-4">
              <div className="flex items-center gap-2 mb-3">
                <button type="button" onClick={() => toggleCheckinSection("calendar")} className="flex items-center gap-2 cursor-pointer select-none">
                  {checkinCollapsed.calendar ? <ChevronRight className="w-4 h-4 text-slate-500" /> : <ChevronDown className="w-4 h-4 text-slate-500" />}
                  <CalendarDays className="w-4 h-4 text-[var(--module-accent)]" />
                  <span className="text-title font-bold text-white">{t("buddy.calendar.title")}</span>
                  <span className="text-micro text-slate-500 hidden md:inline">
                    {t("buddy.calendar.hint")}
                  </span>
                </button>
                <div className="flex-1" />
                <button
                  onClick={() => setCalendarMonth((m) => shiftMonthStr(m, -1))}
                  className="p-1 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition"
                  title={t("buddy.calendar.prevMonth")}
                >
                  <ChevronLeft className="w-3 h-3" />
                </button>
                <span className="text-caption font-mono text-slate-200 w-[62px] text-center">
                  {calendarMonth}
                </span>
                <button
                  onClick={() => setCalendarMonth((m) => shiftMonthStr(m, 1))}
                  className="p-1 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition"
                  title={t("buddy.calendar.nextMonth")}
                >
                  <ChevronRight className="w-3 h-3" />
                </button>
                <button
                  onClick={() => {
                    setCalendarMonth(localMonthStr());
                    setCalendarDay(localDateStr());
                  }}
                  className="px-2 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition"
                >
                  {t("buddy.calendar.today")}
                </button>
              </div>
              {!checkinCollapsed.calendar && (<>
              <div className="grid grid-cols-7 gap-1 mb-1">
                {t("buddy.calendar.weekdays")
                  .split(",")
                  .map((weekday) => (
                    <div key={weekday} className="text-center text-micro text-slate-500">
                      {weekday}
                    </div>
                  ))}
              </div>
              <div className="grid grid-cols-7 gap-1">
                {buildMonthCells(calendarMonth).map((date, index) => {
                  if (!date) return <div key={`blank-${index}`} />;
                  const progress = dayProgress(dailyRecords[date]);
                  const isToday = date === localDateStr();
                  const selected = date === calendarDay;
                  const allDone = progress.total > 0 && progress.checkin === progress.total;
                  return (
                    <button
                      key={date}
                      onClick={() => setCalendarDay(selected ? null : date)}
                      title={date}
                      className={`rounded-ctl border px-1 py-1 text-left transition cursor-pointer ${
                        selected
                          ? "border-[var(--module-accent)] bg-[var(--module-accent)]/15"
                          : isToday
                            ? "border-white/20 bg-white/5 hover:bg-white/10"
                            : "border-white/5 bg-black/20 hover:bg-white/5"
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <span
                          className={`text-tiny font-mono ${
                            allDone ? "text-emerald-300" : "text-slate-300"
                          }`}
                        >
                          {Number(date.slice(8))}
                        </span>
                        {progress.total > 0 && (
                          <span className="flex items-center gap-0.5">
                            <span
                              className={`w-1 h-1 rounded-full ${
                                progress.checkin > 0 ? "bg-emerald-400" : "bg-slate-600"
                              }`}
                            />
                            <span
                              className={`w-1 h-1 rounded-full ${
                                progress.travel > 0 ? "bg-sky-400" : "bg-slate-600"
                              }`}
                            />
                          </span>
                        )}
                      </div>
                      {progress.total > 0 && (
                        <div className="text-[8px] text-slate-500 truncate">
                          {t("buddy.calendar.doneCount", {
                            checkin: progress.checkin,
                            total: progress.total,
                          })}
                        </div>
                      )}
                    </button>
                  );
                })}
              </div>
              {calendarDay && (
                <div className="mt-3 pt-3 border-t border-white/5">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-caption font-semibold text-slate-200">{calendarDay}</span>
                    <span className="text-micro text-slate-500">{t("buddy.calendar.detailHint")}</span>
                    <div className="flex-1" />
                    <button
                      onClick={() => setCalendarDay(null)}
                      className="text-micro text-slate-500 hover:text-slate-300 cursor-pointer"
                    >
                      {t("buddy.calendar.collapse")}
                    </button>
                  </div>
                  {Object.keys(dailyRecords[calendarDay] ?? {}).length === 0 ? (
                    <div className="text-tiny text-slate-600">{t("buddy.calendar.noData")}</div>
                  ) : (
                    <div className="space-y-1">
                      <div className="grid grid-cols-3 gap-2 text-micro text-slate-500 px-2">
                        <span>{t("buddy.calendar.colAccount")}</span>
                        <span>{t("buddy.calendar.colCheckin")}</span>
                        <span>{t("buddy.calendar.colTravel")}</span>
                      </div>
                      {Object.entries(dailyRecords[calendarDay]).map(([accountId, record]) => (
                        <div
                          key={accountId}
                          className="grid grid-cols-3 gap-2 text-tiny bg-black/20 rounded-ctl px-2 py-1.5"
                        >
                          <span className="text-slate-300 truncate" title={record.email || accountId}>
                            {record.email || accountId}
                          </span>
                          <span className="text-slate-400 truncate">
                            {record.checkin
                              ? `${t(`buddy.calendar.checkinStatus.${record.checkin.status}`, {
                                  defaultValue: record.checkin.status,
                                })} · ${record.checkin.planTime ?? "—"}${
                                  record.checkin.actualTime ? ` / ${record.checkin.actualTime}` : ""
                                }`
                              : "—"}
                          </span>
                          <span className="text-slate-400 truncate">
                            {record.travel
                              ? `${t(`buddy.calendar.travelStatus.${record.travel.status}`, {
                                  defaultValue: record.travel.status,
                                })} · ${record.travel.planTime ?? "—"}${
                                  record.travel.departTime ? ` / ${record.travel.departTime}` : ""
                                }${
                                  record.travel.backTime ? ` / ${record.travel.backTime}` : ""
                                } · ${t("buddy.accountStatus.travelCount", {
                                  count: record.travel.departCount,
                                })}`
                              : "—"}
                          </span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
              </>)}
            </div>

            {/* 行为日志（签到 + 派旅行，平铺） */}
            <div className="xl:col-span-2 rounded-card border border-white/10 bg-white/[0.03] p-4">
              <div className="flex items-center gap-2 mb-3">
                <button type="button" onClick={() => toggleCheckinSection("logs")} className="flex items-center gap-2 cursor-pointer select-none">
                  {checkinCollapsed.logs ? <ChevronRight className="w-4 h-4 text-slate-500" /> : <ChevronDown className="w-4 h-4 text-slate-500" />}
                  <span className="text-title font-bold text-white">{t("buddy.actionLogs.title")}</span>
                </button>
                <div className="flex-1" />
                <button
                  onClick={clearAutoLogs}
                  className="px-2 py-1 rounded-md text-tiny bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 border border-rose-500/20 flex items-center gap-1 cursor-pointer transition"
                >
                  <Eraser className="w-3 h-3" /> {t("buddy.auto.clearLogs")}
                </button>
              </div>
              {!checkinCollapsed.logs && (<>
              {actionLogs.length === 0 ? (
                <div className="text-tiny text-slate-600">{t("buddy.actionLogs.empty")}</div>
              ) : (
                <div className="space-y-1 max-h-[40vh] overflow-y-auto">
                  {actionLogs.map((entry) => {
                    const okStatus = ["success", "already_checked", "claimed", "departed"].includes(entry.status);
                    const badStatus = ["failed", "inactive"].includes(entry.status);
                    return (
                      <div key={entry.id} className="flex items-center gap-2 text-tiny font-mono">
                        <span
                          className={`px-1 py-0.5 rounded border flex-shrink-0 ${
                            entry.kind === "travel"
                              ? "text-sky-300 bg-sky-500/10 border-sky-500/20"
                              : "text-emerald-300 bg-emerald-500/10 border-emerald-500/20"
                          }`}
                        >
                          {entry.kind === "travel" ? t("buddy.actionLogs.kindTravel") : t("buddy.actionLogs.kindCheckin")}
                        </span>
                        <span className="text-slate-500 flex-shrink-0">{entry.timestamp}</span>
                        <span className="text-slate-300 truncate max-w-[160px]" title={entry.email}>
                          {entry.email || entry.accountId}
                        </span>
                        <span
                          className={`truncate flex-1 ${
                            okStatus ? "text-emerald-400" : badStatus ? "text-rose-400" : "text-slate-400"
                          }`}
                        >
                          {entry.status === "claimed" && entry.credit != null
                            ? `${entry.message ?? ""} +${entry.credit}`
                            : entry.message ?? entry.status}
                        </span>
                      </div>
                    );
                  })}
                </div>
              )}
              </>)}
            </div>
          </div>
        </div>);
  return {
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
  };
}
