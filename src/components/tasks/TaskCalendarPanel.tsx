// 任务计划：全屏日历（月 / 周），数据来自独立的 tasks.db。
//
// 为什么单独成模块：任务计划与思维导图本是两件事 —— 前者是「什么时候做什么」，
// 后者是「一件事怎么拆」。原先日历挂在思维导图里，等于要求先有导图才能排计划；
// 独立之后任务直接排在某一天，与任何导图无关（旧的节点计划已一次性迁移进来）。
//
// 交互（日历即操作台）：
// - 点某天空白处 = 在当天新建任务；点任务 = 查看/编辑（标题/日期/优先级/进度/标签/描述/预估）
// - 把任务拖到别的日子 = 改期（带原因，写进 move record，便于复盘「为什么一再顺延」）
// - 同一天内把任务拖到另一张卡片上/下沿 = 插入排序（tasks_reorder，刷新后保持）
// - 未排期任务在底部收集箱里，可拖进任意一天完成排期
// - 表头：逾期数量 + 一键结转全部逾期到今天；搜索（结果替换日历区，归档任务在此找回）；复盘统计弹窗

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle, BarChart3, CalendarDays, Check, ChevronLeft, ChevronRight, CornerDownRight,
  Inbox, Loader2, Plus, Search, X,
} from "lucide-react";
import { SharedButton } from "../shared/Button";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { daysRange, isOverdue, monthGrid, shiftMonth, weekRange } from "./calendarRange";
import { TaskEditorModal } from "./TaskEditorModal";
import { ReviewModal } from "./ReviewModal";
import { TaskSearchPanel } from "./TaskSearchPanel";
import {
  PRIORITY_META, addDays, tasksApi, todayStr,
  type TaskItem,
} from "./types";

const WEEKDAY_KEYS = [
  "taskPlan.weekdayMon", "taskPlan.weekdayTue", "taskPlan.weekdayWed", "taskPlan.weekdayThu",
  "taskPlan.weekdayFri", "taskPlan.weekdaySat", "taskPlan.weekdaySun",
];

export default function TaskCalendarPanel() {
  const { t } = useTranslation();
  const today = todayStr();
  const [view, setView] = useState<"month" | "week">("month");
  // 锚点日期：月视图用它定位到某月，周视图用它定位到某周
  const [anchor, setAnchor] = useState<string>(today);
  const [items, setItems] = useState<TaskItem[]>([]);
  const [unscheduled, setUnscheduled] = useState<TaskItem[]>([]);
  const [showInbox, setShowInbox] = useState(false);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  // 编辑（task 非空）与新建（task 为空、day 为默认日期）共用同一个弹窗
  const [editing, setEditing] = useState<TaskItem | null>(null);
  const [draftDay, setDraftDay] = useState<string | null>(null);
  const [removing, setRemoving] = useState<TaskItem | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  /** 插入排序落点：落在某天某张卡片的上/下沿（与格子高亮环 dragOver 互斥） */
  const [chipDrop, setChipDrop] = useState<{ day: string; id: string; pos: "before" | "after" } | null>(null);
  // 搜索：有查询词时结果区替换日历网格（表头保留）
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  /** 编辑弹窗保存后递增，让结果区重搜（归档状态等可能已变） */
  const [refreshNonce, setRefreshNonce] = useState(0);
  const [reviewOpen, setReviewOpen] = useState(false);
  /** 结转确认弹窗的载体：非 null 即打开，值是 listOverdue 现查到的全局逾期数 */
  const [carryCount, setCarryCount] = useState<number | null>(null);
  // 轻量成功提示（3 秒自灭），与 PickyPanel 的 flash 同款
  const [notice, setNotice] = useState("");
  const noticeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const days = useMemo(() => {
    if (view === "week") {
      const { start } = weekRange(anchor);
      return Array.from({ length: 7 }, (_, i) => addDays(start, i));
    }
    return monthGrid(anchor);
  }, [view, anchor]);
  // 拉数据的区间 = 屏幕上真正看得见的格子（月视图含前后月份的补齐格）
  const range = useMemo(() => daysRange(days), [days]);
  const currentMonth = anchor.slice(0, 7);
  const searchActive = debounced.trim() !== "";

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [inRange, inbox] = await Promise.all([
        tasksApi.listRange(range.start, range.end),
        tasksApi.listByDate(null),
      ]);
      setItems(inRange);
      setUnscheduled(inbox);
      setErr("");
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  }, [range.start, range.end]);

  // 首次进入确保库已就绪（含从思维导图旧计划的一次性迁移），再拉数据。
  // init 只跑一次：`load` 随可见区间变化，若把它当依赖每次切月都会重跑 init。
  const inited = useRef(false);
  useEffect(() => {
    if (inited.current) {
      void load();
      return;
    }
    inited.current = true;
    void tasksApi.init().catch(() => {}).then(() => load());
  }, [load]);

  // 搜索防抖：300ms 内连续输入不发请求
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query), 300);
    return () => clearTimeout(timer);
  }, [query]);

  const flash = (msg: string) => {
    if (noticeTimer.current) clearTimeout(noticeTimer.current);
    setNotice(msg);
    noticeTimer.current = setTimeout(() => setNotice(""), 3000);
  };

  const byDay = useMemo(() => {
    const map = new Map<string, TaskItem[]>();
    for (const task of items) {
      if (!task.scheduledDate) continue;
      const list = map.get(task.scheduledDate);
      if (list) list.push(task);
      else map.set(task.scheduledDate, [task]);
    }
    return map;
  }, [items]);

  const overdue = useMemo(() => items.filter((task) => isOverdue(task, today)), [items, today]);

  /** 拖拽落点：改期到该天（带原因，便于日后复盘） */
  const dropOn = async (day: string) => {
    const id = dragId;
    setDragId(null);
    setDragOver(null);
    setChipDrop(null);
    if (!id) return;
    const task = items.find((x) => x.id === id) ?? unscheduled.find((x) => x.id === id);
    if (task?.scheduledDate === day) return;
    try {
      await tasksApi.move(id, { toDate: day, reason: t("taskPlan.dragReason") });
      await load();
    } catch (e) {
      setErr(String(e));
    }
  };

  /** 同一天内拖到某张卡片上/下沿：插入排序。乐观更新，失败回滚（重新加载） */
  const reorderSameDay = async (day: string, srcId: string, targetId: string, pos: "before" | "after") => {
    const dayList = byDay.get(day) ?? [];
    const ids = dayList.map((x) => x.id).filter((id) => id !== srcId);
    let idx = ids.indexOf(targetId);
    if (idx === -1) return;
    if (pos === "after") idx += 1;
    ids.splice(idx, 0, srcId);
    const current = dayList.map((x) => x.id);
    if (ids.every((id, i) => id === current[i])) return;
    const reordered = ids
      .map((id) => dayList.find((x) => x.id === id))
      .filter((x): x is TaskItem => !!x);
    setItems((prev) => [...prev.filter((x) => x.scheduledDate !== day), ...reordered]);
    try {
      await tasksApi.reorder(ids);
    } catch (e) {
      setErr(String(e));
      void load();
    }
  };

  /**
   * 一键结转全部逾期到今天。后端是全局口径（不限当前可见范围），
   * 所以条数必须 listOverdue 现查，不能复用表头 chip 的可见范围计数。
   */
  const requestCarryOver = async () => {
    try {
      const list = await tasksApi.listOverdue();
      if (list.length === 0) {
        flash(t("taskPlan.carryOverNone"));
        return;
      }
      setCarryCount(list.length);
    } catch (e) {
      setErr(String(e));
    }
  };

  const confirmCarryOver = async () => {
    setCarryCount(null);
    try {
      const n = await tasksApi.carryOver();
      flash(t("taskPlan.carryOverDone", { count: n }));
      await load();
    } catch (e) {
      setErr(String(e));
    }
  };

  const jumpTo = (delta: number) => {
    setAnchor((prev) => (view === "month" ? shiftMonth(prev, delta) : addDays(prev, delta * 7)));
  };

  /**
   * 任务卡片。day 存在 = 日历格内（挂插入排序的 dragOver/drop）；
   * day 缺省 = 收集箱（只做拖出源，不做排序落点）。
   */
  const taskChip = (task: TaskItem, day?: string) => {
    const done = task.progress >= 100;
    const late = isOverdue(task, today);
    return (
      <div
        draggable
        onDragStart={(e) => {
          setDragId(task.id);
          e.dataTransfer.effectAllowed = "move";
          // 拖到格子外（收集箱）也要能识别，用 text 兜底
          e.dataTransfer.setData("text/plain", task.id);
        }}
        onDragEnd={() => { setDragId(null); setDragOver(null); setChipDrop(null); }}
        onDragOver={(e) => {
          if (!day) return;
          e.preventDefault();
          // 阻止冒泡到格子：排序指示线亮起时格子环必须熄灭（两种落点语义互斥）
          e.stopPropagation();
          if (!dragId || dragId === task.id) {
            setChipDrop(null);
            return;
          }
          const rect = e.currentTarget.getBoundingClientRect();
          const pos = e.clientY < rect.top + rect.height / 2 ? "before" : "after";
          setChipDrop((prev) =>
            prev && prev.day === day && prev.id === task.id && prev.pos === pos
              ? prev
              : { day, id: task.id, pos },
          );
        }}
        onDragLeave={() => {
          if (!day) return;
          setChipDrop((prev) => (prev?.id === task.id ? null : prev));
        }}
        onDrop={(e) => {
          if (!day) return;
          e.preventDefault();
          e.stopPropagation();
          const srcId = dragId || e.dataTransfer.getData("text/plain");
          const pos = chipDrop?.id === task.id ? chipDrop.pos : "before";
          setDragId(null);
          setDragOver(null);
          setChipDrop(null);
          if (!srcId || srcId === task.id) return;
          const src = items.find((x) => x.id === srcId) ?? unscheduled.find((x) => x.id === srcId);
          if (src?.scheduledDate === day) {
            void reorderSameDay(day, srcId, task.id, pos);
          } else {
            // 跨天/收集箱落到卡片上：等效落到该天（追加到末尾），与格子 drop 行为一致
            void tasksApi
              .move(srcId, { toDate: day, reason: t("taskPlan.dragReason") })
              .then(() => load())
              .catch((err2) => setErr(String(err2)));
          }
        }}
        onClick={(e) => { e.stopPropagation(); setEditing(task); }}
        title={`${task.title} · ${t("taskPlan.progressLabel")} ${task.progress}%`}
        className={`flex cursor-pointer items-center gap-1 rounded border border-white/10 bg-white/[0.04] px-1 py-0.5 text-tiny transition hover:bg-white/[0.1] ${
          late ? "border-l-2 border-l-rose-400" : ""
        } ${dragId === task.id ? "opacity-40" : ""}`}
      >
        <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${PRIORITY_META[task.priority]?.dot ?? "bg-slate-500"}`} />
        <span className={`min-w-0 flex-1 truncate ${done ? "text-slate-500 line-through" : "text-slate-200"}`}>
          {task.title || t("taskPlan.untitled")}
        </span>
        {done
          ? <Check className="h-2.5 w-2.5 shrink-0 text-emerald-400" />
          : task.progress > 0 && <span className="shrink-0 text-micro text-slate-500">{task.progress}%</span>}
      </div>
    );
  };

  const dayCell = (day: string) => {
    const list = byDay.get(day) ?? [];
    const isToday = day === today;
    const past = day < today;
    const outside = view === "month" && !day.startsWith(currentMonth);
    return (
      <div
        key={day}
        onClick={() => setDraftDay(day)}
        onDragOver={(e) => { e.preventDefault(); setDragOver(day); }}
        onDragLeave={() => setDragOver((d) => (d === day ? null : d))}
        onDrop={(e) => { e.preventDefault(); void dropOn(day); }}
        className={`flex min-h-0 cursor-pointer flex-col gap-0.5 border-b border-r border-white/5 p-1 transition ${
          outside ? "bg-black/30" : past ? "bg-black/15" : ""
        } ${isToday ? "bg-[var(--module-accent-soft)]" : ""} ${
          dragOver === day ? "ring-1 ring-inset ring-[var(--module-accent)]" : ""
        }`}
      >
        <div className="flex shrink-0 items-center gap-1">
          <span className={`text-tiny font-semibold ${
            isToday ? "text-[var(--module-accent)]" : outside ? "text-slate-600" : past ? "text-slate-500" : "text-slate-400"
          }`}>
            {isToday ? t("taskPlan.today") : Number(day.slice(8))}
          </span>
          <span className="text-micro text-slate-600">{Number(day.slice(8)) === 1 ? `${Number(day.slice(5, 7))} 月` : ""}</span>
          {list.length > 0 && <span className="ml-auto text-micro text-slate-600">{list.length}</span>}
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto">
          {list.map((task) => (
            <div key={task.id} className="flex flex-col gap-0.5">
              {chipDrop?.day === day && chipDrop.id === task.id && chipDrop.pos === "before" && (
                <div className="h-0.5 shrink-0 rounded bg-[var(--module-accent)]" />
              )}
              {taskChip(task, day)}
              {chipDrop?.day === day && chipDrop.id === task.id && chipDrop.pos === "after" && (
                <div className="h-0.5 shrink-0 rounded bg-[var(--module-accent)]" />
              )}
            </div>
          ))}
        </div>
      </div>
    );
  };

  const rangeLabel = view === "month"
    ? t("taskPlan.monthTitle", { year: anchor.slice(0, 4), month: Number(anchor.slice(5, 7)) })
    : t("taskPlan.weekTitle", { start: range.start.slice(5), end: range.end.slice(5) });

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-3 text-slate-200">
      {/* 表头 */}
      <div className="flex flex-shrink-0 items-center gap-2">
        <CalendarDays className="h-4 w-4 text-[var(--module-accent)]" />
        <h2 className="text-sm font-bold text-white">{t("taskPlan.title")}</h2>
        <span className="text-caption text-slate-400">{rangeLabel}</span>
        <div className="ml-1 flex items-center gap-1">
          <button type="button" onClick={() => jumpTo(-1)} title={view === "month" ? t("taskPlan.prevMonth") : t("taskPlan.prevWeek")}
            className="rounded-md p-1 text-slate-400 transition hover:bg-white/10 hover:text-white">
            <ChevronLeft className="h-3.5 w-3.5" />
          </button>
          <button type="button" onClick={() => setAnchor(today)}
            className="rounded-md border border-white/10 px-2 py-0.5 text-tiny text-slate-300 transition hover:bg-white/10">
            {t("taskPlan.today")}
          </button>
          <button type="button" onClick={() => jumpTo(1)} title={view === "month" ? t("taskPlan.nextMonth") : t("taskPlan.nextWeek")}
            className="rounded-md p-1 text-slate-400 transition hover:bg-white/10 hover:text-white">
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        </div>
        {loading && <Loader2 className="h-3 w-3 animate-spin text-slate-500" />}

        <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
          {overdue.length > 0 && (
            <span className="flex items-center gap-1 rounded-md border border-rose-400/30 bg-rose-500/10 px-2 py-0.5 text-tiny text-rose-300"
              title={t("taskPlan.overdueHint")}>
              <AlertTriangle className="h-3 w-3" />
              {t("taskPlan.overdueChip", { count: overdue.length })}
            </span>
          )}
          {/* 结转是全局口径：不依赖可见范围有逾期，所以按钮常显，空转时 flash 提示 */}
          <button type="button" onClick={() => void requestCarryOver()} title={t("taskPlan.carryOverBtn")}
            className="rounded-md p-1 text-rose-300 transition hover:bg-rose-500/10">
            <CornerDownRight className="h-3.5 w-3.5" />
          </button>
          <div className="relative">
            <Search className="pointer-events-none absolute left-1.5 top-1/2 h-3 w-3 -translate-y-1/2 text-slate-500" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("taskPlan.searchPlaceholder")}
              className="h-7 w-40 rounded-md border border-white/10 bg-white/[0.03] pl-6 pr-5 text-tiny text-slate-200 placeholder:text-slate-600 focus:border-[var(--module-accent)] focus:outline-none"
            />
            {query && (
              <button type="button" onClick={() => setQuery("")}
                className="absolute right-1 top-1/2 -translate-y-1/2 text-slate-500 hover:text-white">
                <X className="h-3 w-3" />
              </button>
            )}
          </div>
          <SharedButton variant="secondary" className="!h-7 !px-2 !text-tiny"
            onClick={() => setReviewOpen(true)} title={t("taskPlan.reviewTitle")}>
            <BarChart3 className="h-3 w-3" />
            {t("taskPlan.reviewBtn")}
          </SharedButton>
          {/* 视图切换：分段按钮，与模块内其它控件同一套外观 */}
          <div className="flex items-center gap-0.5 rounded-ctl border border-white/10 bg-white/[0.03] p-0.5">
            {(["month", "week"] as const).map((v) => (
              <button key={v} type="button" onClick={() => setView(v)}
                className={`rounded-md px-2 py-0.5 text-tiny transition ${
                  view === v ? "bg-[var(--module-accent)]/25 font-semibold text-white" : "text-slate-400 hover:bg-white/10"
                }`}>
                {v === "month" ? t("taskPlan.monthView") : t("taskPlan.weekView")}
              </button>
            ))}
          </div>
          <SharedButton variant={showInbox ? "primary" : "secondary"} className="!h-7 !px-2 !text-tiny"
            onClick={() => setShowInbox((v) => !v)} title={t("taskPlan.inboxHint")}>
            <Inbox className="h-3 w-3" />
            {t("taskPlan.inbox", { count: unscheduled.length })}
          </SharedButton>
          <SharedButton variant="primary" className="!h-7 !px-2 !text-tiny" onClick={() => setDraftDay(today)}>
            <Plus className="h-3 w-3" />
            {t("taskPlan.newTask")}
          </SharedButton>
        </div>
      </div>

      {err && (
        <div className="flex flex-shrink-0 items-start gap-2 rounded-ctl border border-rose-500/25 bg-rose-500/10 px-3 py-2 text-caption text-rose-300">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1 break-all">{t("taskPlan.loadFailed", { err })}</span>
          <button type="button" onClick={() => setErr("")} className="shrink-0 text-rose-300/70 hover:text-white">✕</button>
        </div>
      )}

      {notice && (
        <div className="flex flex-shrink-0 items-center gap-2 rounded-ctl border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-caption text-emerald-300">
          <Check className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1 break-all">{notice}</span>
        </div>
      )}

      {searchActive ? (
        /* 搜索态：结果区替换日历与收集箱（表头保留，搜索框在表头） */
        <TaskSearchPanel
          query={debounced.trim()}
          refreshNonce={refreshNonce}
          onOpenTask={(task) => setEditing(task)}
        />
      ) : (
        <>
          {/* 日历网格 */}
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-card border border-white/10">
            <div className="grid flex-shrink-0 grid-cols-7 border-b border-white/10 bg-white/[0.02]">
              {WEEKDAY_KEYS.map((key) => (
                <div key={key} className="px-2 py-1 text-center text-tiny font-semibold text-slate-500">{t(key)}</div>
              ))}
            </div>
            <div className={view === "month"
              ? "grid min-h-0 flex-1 grid-cols-7 grid-rows-6"
              : "grid min-h-0 flex-1 grid-cols-7"}>
              {days.map(dayCell)}
            </div>
          </div>

          {/* 未排期收集箱：拖到格子里即完成排期（不显示的话，这些任务等于在日历上消失） */}
          {showInbox && (
            <div className="flex max-h-28 flex-shrink-0 flex-col gap-1 overflow-hidden rounded-card border border-white/10 bg-white/[0.02] p-2">
              <div className="flex shrink-0 items-center gap-1.5 text-tiny font-semibold text-slate-400">
                <Inbox className="h-3 w-3" />
                {t("taskPlan.inboxTitle")}
                <span className="text-slate-600">{unscheduled.length}</span>
              </div>
              <div className="flex min-h-0 flex-1 flex-wrap gap-1 overflow-y-auto">
                {unscheduled.length === 0
                  ? <span className="text-tiny text-slate-600">{t("taskPlan.inboxEmpty")}</span>
                  : unscheduled.map((task) => (
                    <div key={task.id} className="w-48">{taskChip(task)}</div>
                  ))}
              </div>
            </div>
          )}
        </>
      )}

      {/* 编辑 / 新建 */}
      {(editing !== null || draftDay !== null) && (
        <TaskEditorModal
          task={editing}
          day={draftDay ?? today}
          onClose={() => { setEditing(null); setDraftDay(null); }}
          onSaved={() => { setEditing(null); setDraftDay(null); setRefreshNonce((n) => n + 1); void load(); }}
          onRequestDelete={(task) => { setEditing(null); setRemoving(task); }}
        />
      )}

      <ReviewModal open={reviewOpen} onClose={() => setReviewOpen(false)} range={range} items={items} />

      <ConfirmDialog
        open={carryCount !== null}
        title={t("taskPlan.carryOverTitle")}
        desc={t("taskPlan.carryOverDesc", { count: carryCount ?? 0 })}
        confirmText={t("taskPlan.carryOverBtn")}
        onCancel={() => setCarryCount(null)}
        onConfirm={() => void confirmCarryOver()}
      />

      <ConfirmDialog
        open={removing !== null}
        danger
        title={t("taskPlan.deleteTitle")}
        desc={t("taskPlan.deleteDesc", { title: removing?.title ?? "" })}
        confirmText={t("taskPlan.delete")}
        onCancel={() => setRemoving(null)}
        onConfirm={() => {
          const task = removing;
          setRemoving(null);
          if (!task) return;
          void tasksApi.remove(task.id).then(() => load()).catch((e) => setErr(String(e)));
        }}
      />
    </div>
  );
}
