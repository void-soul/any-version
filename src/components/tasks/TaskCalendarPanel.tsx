// 任务计划：全屏日历（月 / 周），数据来自独立的 tasks.db。
//
// 为什么单独成模块：任务计划与思维导图本是两件事 —— 前者是「什么时候做什么」，
// 后者是「一件事怎么拆」。原先日历挂在思维导图里，等于要求先有导图才能排计划；
// 独立之后任务直接排在某一天，与任何导图无关（旧的节点计划已一次性迁移进来）。
//
// 交互（日历即操作台）：
// - 点某天空白处 = 在当天新建任务；点任务 = 编辑（标题/日期/优先级/进度/标签/描述）
// - 把任务拖到别的日子 = 改期（带原因，写进 move record，便于复盘「为什么一再顺延」）
// - 未排期任务在底部收集箱里，可拖进任意一天完成排期
// - 表头汇总逾期数量；逾期任务在格子里以红色左边条标出，可在编辑弹窗里一键顺延到今天

import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle, CalendarDays, Check, ChevronLeft, ChevronRight, Clock, Inbox, Loader2, Plus, Trash2,
} from "lucide-react";
import { SharedButton, inputCls, labelCls } from "../shared/Button";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { SharedModal } from "../shared/Modal";
import { isOverdue, monthGrid, monthRange, shiftMonth, weekRange } from "./calendarRange";
import {
  PRIORITY_META, addDays, deriveStatus, progressColor, tasksApi, todayStr,
  type TaskItem, type TaskPriority,
} from "./types";

const WEEKDAY_KEYS = [
  "taskPlan.weekdayMon", "taskPlan.weekdayTue", "taskPlan.weekdayWed", "taskPlan.weekdayThu",
  "taskPlan.weekdayFri", "taskPlan.weekdaySat", "taskPlan.weekdaySun",
];
const PRIORITY_ORDER: TaskPriority[] = ["low", "medium", "high", "urgent"];
const PRIORITY_LABEL: Record<TaskPriority, string> = {
  low: "taskPlan.priorityLow",
  medium: "taskPlan.priorityMedium",
  high: "taskPlan.priorityHigh",
  urgent: "taskPlan.priorityUrgent",
};

/** 一天的毫秒数（快速按钮「明天」用） */
const DAY_MS = 24 * 60 * 60 * 1000;

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

  const range = useMemo(
    () => (view === "month" ? monthRange(anchor) : weekRange(anchor)),
    [view, anchor],
  );
  const days = useMemo(() => {
    if (view === "week") {
      const { start } = weekRange(anchor);
      return Array.from({ length: 7 }, (_, i) => addDays(start, i));
    }
    return monthGrid(anchor);
  }, [view, anchor]);
  const currentMonth = anchor.slice(0, 7);

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

  // 首次进入确保库已就绪（含从思维导图旧计划的一次性迁移），再拉数据
  useEffect(() => {
    void tasksApi.init().catch(() => {}).then(() => load());
  }, [load]);

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

  const jumpTo = (delta: number) => {
    setAnchor((prev) => (view === "month" ? shiftMonth(prev, delta) : addDays(prev, delta * 7)));
  };

  const taskChip = (task: TaskItem) => {
    const done = task.progress >= 100;
    const late = isOverdue(task, today);
    return (
      <div
        key={task.id}
        draggable
        onDragStart={(e) => {
          setDragId(task.id);
          e.dataTransfer.effectAllowed = "move";
          // 拖到格子外（收集箱）也要能识别，用 text 兜底
          e.dataTransfer.setData("text/plain", task.id);
        }}
        onDragEnd={() => { setDragId(null); setDragOver(null); }}
        onClick={(e) => { e.stopPropagation(); setEditing(task); }}
        title={`${task.title} · ${t("taskPlan.progressLabel")} ${task.progress}%`}
        className={`flex cursor-pointer items-center gap-1 rounded border border-white/10 bg-white/[0.04] px-1 py-0.5 text-[10px] transition hover:bg-white/[0.1] ${
          late ? "border-l-2 border-l-rose-400" : ""
        } ${dragId === task.id ? "opacity-40" : ""}`}
      >
        <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${PRIORITY_META[task.priority]?.dot ?? "bg-slate-500"}`} />
        <span className={`min-w-0 flex-1 truncate ${done ? "text-slate-500 line-through" : "text-slate-200"}`}>
          {task.title || t("taskPlan.untitled")}
        </span>
        {done
          ? <Check className="h-2.5 w-2.5 shrink-0 text-emerald-400" />
          : task.progress > 0 && <span className="shrink-0 text-[9px] text-slate-500">{task.progress}%</span>}
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
          <span className={`text-[10px] font-semibold ${
            isToday ? "text-[var(--module-accent)]" : outside ? "text-slate-600" : past ? "text-slate-500" : "text-slate-400"
          }`}>
            {isToday ? t("taskPlan.today") : Number(day.slice(8))}
          </span>
          <span className="text-[9px] text-slate-600">{Number(day.slice(8)) === 1 ? `${Number(day.slice(5, 7))} 月` : ""}</span>
          {list.length > 0 && <span className="ml-auto text-[9px] text-slate-600">{list.length}</span>}
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto">
          {list.map(taskChip)}
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
        <span className="text-[11px] text-slate-400">{rangeLabel}</span>
        <div className="ml-1 flex items-center gap-1">
          <button type="button" onClick={() => jumpTo(-1)} title={view === "month" ? t("taskPlan.prevMonth") : t("taskPlan.prevWeek")}
            className="rounded-md p-1 text-slate-400 transition hover:bg-white/10 hover:text-white">
            <ChevronLeft className="h-3.5 w-3.5" />
          </button>
          <button type="button" onClick={() => setAnchor(today)}
            className="rounded-md border border-white/10 px-2 py-0.5 text-[10px] text-slate-300 transition hover:bg-white/10">
            {t("taskPlan.today")}
          </button>
          <button type="button" onClick={() => jumpTo(1)} title={view === "month" ? t("taskPlan.nextMonth") : t("taskPlan.nextWeek")}
            className="rounded-md p-1 text-slate-400 transition hover:bg-white/10 hover:text-white">
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        </div>
        {loading && <Loader2 className="h-3 w-3 animate-spin text-slate-500" />}

        <div className="ml-auto flex items-center gap-2">
          {overdue.length > 0 && (
            <span className="flex items-center gap-1 rounded-md border border-rose-400/30 bg-rose-500/10 px-2 py-0.5 text-[10px] text-rose-300"
              title={t("taskPlan.overdueHint")}>
              <AlertTriangle className="h-3 w-3" />
              {t("taskPlan.overdueChip", { count: overdue.length })}
            </span>
          )}
          {/* 视图切换：分段按钮，与模块内其它控件同一套外观 */}
          <div className="flex items-center gap-0.5 rounded-lg border border-white/10 bg-white/[0.03] p-0.5">
            {(["month", "week"] as const).map((v) => (
              <button key={v} type="button" onClick={() => setView(v)}
                className={`rounded-md px-2 py-0.5 text-[10px] transition ${
                  view === v ? "bg-[var(--module-accent)]/25 font-semibold text-white" : "text-slate-400 hover:bg-white/10"
                }`}>
                {v === "month" ? t("taskPlan.monthView") : t("taskPlan.weekView")}
              </button>
            ))}
          </div>
          <SharedButton variant={showInbox ? "primary" : "secondary"} className="!h-7 !px-2 !text-[10px]"
            onClick={() => setShowInbox((v) => !v)} title={t("taskPlan.inboxHint")}>
            <Inbox className="h-3 w-3" />
            {t("taskPlan.inbox", { count: unscheduled.length })}
          </SharedButton>
          <SharedButton variant="primary" className="!h-7 !px-2 !text-[10px]" onClick={() => setDraftDay(today)}>
            <Plus className="h-3 w-3" />
            {t("taskPlan.newTask")}
          </SharedButton>
        </div>
      </div>

      {err && (
        <div className="flex flex-shrink-0 items-start gap-2 rounded-lg border border-rose-500/25 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-300">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1 break-all">{t("taskPlan.loadFailed", { err })}</span>
          <button type="button" onClick={() => setErr("")} className="shrink-0 text-rose-300/70 hover:text-white">✕</button>
        </div>
      )}

      {/* 日历网格 */}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-white/10">
        <div className="grid flex-shrink-0 grid-cols-7 border-b border-white/10 bg-white/[0.02]">
          {WEEKDAY_KEYS.map((key) => (
            <div key={key} className="px-2 py-1 text-center text-[10px] font-semibold text-slate-500">{t(key)}</div>
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
        <div className="flex max-h-28 flex-shrink-0 flex-col gap-1 overflow-hidden rounded-xl border border-white/10 bg-white/[0.02] p-2">
          <div className="flex shrink-0 items-center gap-1.5 text-[10px] font-semibold text-slate-400">
            <Inbox className="h-3 w-3" />
            {t("taskPlan.inboxTitle")}
            <span className="text-slate-600">{unscheduled.length}</span>
          </div>
          <div className="flex min-h-0 flex-1 flex-wrap gap-1 overflow-y-auto">
            {unscheduled.length === 0
              ? <span className="text-[10px] text-slate-600">{t("taskPlan.inboxEmpty")}</span>
              : unscheduled.map((task) => (
                <div key={task.id} className="w-48">{taskChip(task)}</div>
              ))}
          </div>
        </div>
      )}

      {/* 编辑 / 新建 */}
      {(editing !== null || draftDay !== null) && (
        <TaskEditorModal
          task={editing}
          day={draftDay ?? today}
          onClose={() => { setEditing(null); setDraftDay(null); }}
          onSaved={() => { setEditing(null); setDraftDay(null); void load(); }}
          onRequestDelete={(task) => { setEditing(null); setRemoving(task); }}
        />
      )}

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

// ════════════ 任务编辑弹窗（新建与编辑共用） ════════════

function TaskEditorModal({
  task, day, onClose, onSaved, onRequestDelete,
}: {
  task: TaskItem | null;
  day: string;
  onClose: () => void;
  onSaved: () => void;
  onRequestDelete: (task: TaskItem) => void;
}) {
  const { t } = useTranslation();
  const today = todayStr();
  const [title, setTitle] = useState(task?.title ?? "");
  const [date, setDate] = useState(task?.scheduledDate ?? day);
  const [priority, setPriority] = useState<TaskPriority>((task?.priority as TaskPriority) ?? "medium");
  const [progress, setProgress] = useState(task?.progress ?? 0);
  const [tags, setTags] = useState(task?.tags ?? "");
  const [description, setDescription] = useState(task?.description ?? "");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const late = !!task && isOverdue({ progress, scheduledDate: date || null }, today);

  const save = async () => {
    const trimmed = title.trim();
    if (!trimmed) {
      setErr(t("taskPlan.titleRequired"));
      return;
    }
    setBusy(true);
    setErr("");
    try {
      if (task) {
        await tasksApi.update(task.id, {
          title: trimmed,
          scheduledDate: date || null,
          priority,
          tags,
          description,
        });
        // 进度走专用命令：它会记 move record（进度变化也值得留痕）
        if (progress !== task.progress) await tasksApi.setProgress(task.id, { progress });
      } else {
        await tasksApi.create({ title: trimmed, scheduledDate: date || null, priority, tags, description, progress });
      }
      onSaved();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const carryToToday = async () => {
    if (!task) return;
    setBusy(true);
    setErr("");
    try {
      await tasksApi.move(task.id, { toDate: today, reason: t("taskPlan.carryReason") });
      onSaved();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <SharedModal
      open
      onClose={onClose}
      width={520}
      title={task ? t("taskPlan.editTask") : t("taskPlan.newTask")}
      footer={
        <>
          {task && (
            <SharedButton variant="ghost" className="!text-rose-300 hover:!bg-rose-500/10" disabled={busy}
              onClick={() => onRequestDelete(task)}>
              <Trash2 className="h-3 w-3" />
              {t("taskPlan.delete")}
            </SharedButton>
          )}
          <div className="ml-auto flex items-center gap-2">
            <SharedButton variant="secondary" onClick={onClose}>{t("taskPlan.cancel")}</SharedButton>
            <SharedButton variant="primary" disabled={busy} onClick={() => void save()}>{t("taskPlan.save")}</SharedButton>
          </div>
        </>
      }
    >
      {err && (
        <div className="flex items-start gap-2 rounded-lg border border-rose-500/25 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-300">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1 break-all">{err}</span>
        </div>
      )}

      <div>
        <label className={labelCls}>{t("taskPlan.titleLabel")}</label>
        <input className={inputCls} autoFocus value={title}
          placeholder={t("taskPlan.titlePlaceholder")}
          onChange={(e) => setTitle(e.target.value)} />
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls}>{t("taskPlan.dateLabel")}</label>
          <input type="date" className={inputCls} value={date} onChange={(e) => setDate(e.target.value)} />
          <div className="mt-1 flex items-center gap-1">
            <button type="button" onClick={() => setDate(todayStr())}
              className="rounded border border-white/10 px-1.5 py-0.5 text-[10px] text-slate-400 hover:bg-white/10">
              {t("taskPlan.setToday")}
            </button>
            <button type="button" onClick={() => setDate(new Date(Date.now() + DAY_MS).toISOString().slice(0, 10))}
              className="rounded border border-white/10 px-1.5 py-0.5 text-[10px] text-slate-400 hover:bg-white/10">
              {t("taskPlan.setTomorrow")}
            </button>
            <button type="button" onClick={() => setDate("")}
              className="rounded border border-white/10 px-1.5 py-0.5 text-[10px] text-slate-400 hover:bg-white/10">
              {t("taskPlan.unschedule")}
            </button>
          </div>
          {!date && <div className="mt-1 text-[10px] text-slate-500">{t("taskPlan.unscheduledHint")}</div>}
        </div>
        <div>
          <label className={labelCls}>{t("taskPlan.priorityLabel")}</label>
          <div className="flex items-center gap-1">
            {PRIORITY_ORDER.map((p) => (
              <button key={p} type="button" onClick={() => setPriority(p)}
                className={`flex-1 rounded-lg border px-1.5 py-1 text-[10px] transition ${
                  priority === p ? `${PRIORITY_META[p].bg} ${PRIORITY_META[p].text} font-semibold` : "border-white/10 text-slate-400 hover:bg-white/5"
                }`}>
                {t(PRIORITY_LABEL[p])}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div>
        <label className={labelCls}>
          {t("taskPlan.progressLabel")} · {progress}%
          <span className="ml-2 font-normal text-slate-500">{t(`taskPlan.status${deriveStatus(progress) === "done" ? "Done" : deriveStatus(progress) === "inProgress" ? "InProgress" : "Todo"}`)}</span>
        </label>
        <div className="flex items-center gap-2">
          <input type="range" min={0} max={100} step={5} value={progress}
            onChange={(e) => setProgress(Number(e.target.value))}
            className="h-1.5 flex-1 cursor-pointer accent-[var(--module-accent)]" />
          <div className="flex items-center gap-1">
            {[0, 50, 100].map((p) => (
              <button key={p} type="button" onClick={() => setProgress(p)}
                className={`rounded border px-1.5 py-0.5 text-[10px] transition ${
                  progress === p ? "border-white/25 bg-white/10 text-white" : "border-white/10 text-slate-400 hover:bg-white/5"
                }`}>
                {p}%
              </button>
            ))}
          </div>
        </div>
        <div className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-white/10">
          <div className={`h-full ${progressColor(progress)}`} style={{ width: `${progress}%` }} />
        </div>
      </div>

      <div>
        <label className={labelCls}>{t("taskPlan.tagsLabel")}</label>
        <input className={inputCls} value={tags} placeholder={t("taskPlan.tagsPlaceholder")}
          onChange={(e) => setTags(e.target.value)} />
      </div>

      <div>
        <label className={labelCls}>{t("taskPlan.descriptionLabel")}</label>
        <textarea className={`${inputCls} !h-20 resize-none py-1.5`} value={description}
          onChange={(e) => setDescription(e.target.value)} />
      </div>

      {task && (
        <div className="flex flex-wrap items-center gap-2 border-t border-white/10 pt-2 text-[10px] text-slate-500">
          <Clock className="h-3 w-3" />
          {t("taskPlan.metaLine", {
            created: task.createdAt.slice(0, 10),
            updated: task.updatedAt.slice(0, 10),
          })}
          {late && (
            <button type="button" disabled={busy} onClick={() => void carryToToday()}
              className="ml-auto rounded-md border border-amber-400/30 bg-amber-400/10 px-2 py-0.5 text-[10px] text-amber-200 transition hover:bg-amber-400/20 disabled:opacity-50">
              {t("taskPlan.carryToToday")}
            </button>
          )}
        </div>
      )}
    </SharedModal>
  );
}
