import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  CirclePlus,
  Clock,
  Edit3,
  History,
  RefreshCw,
  Target,
  Trash2,
  Archive,
  CalendarClock,
  Check,
} from "lucide-react";
import TaskCard from "./TaskCard";
import { TaskEditModal, TaskHistoryModal, TaskMoveModal, TaskProgressModal } from "./TaskModals";
import {
  TaskItem,
  tasksApi,
  addDays,
  deriveStatus,
  formatMinutes,
  humanDate,
  PRIORITY_META,
  progressColor,
  todayStr,
} from "./types";

type ModalKind = "edit" | "progress" | "move" | "history" | null;
const DAY_WIDTH = 42;
const RANGE_DAYS = 35;

function dateDistance(start: string, end: string): number {
  const [sy, sm, sd] = start.split("-").map(Number);
  const [ey, em, ed] = end.split("-").map(Number);
  return Math.round((Date.UTC(ey, em - 1, ed) - Date.UTC(sy, sm - 1, sd)) / 86400000);
}

function taskSpan(task: TaskItem): number {
  return Math.max(1, Math.ceil(Math.max(0, task.estimateMinutes) / 480));
}

function TaskTreeRow({
  task,
  children,
  depth,
  collapsed,
  onToggle,
  onAddChild,
  onEdit,
  onProgress,
  onMove,
  onHistory,
  onArchive,
  onDelete,
  onToggleDone,
  onTimeline,
  childrenByParent,
}: {
  task: TaskItem;
  children: TaskItem[];
  depth: number;
  collapsed: Set<string>;
  onToggle: (id: string) => void;
  onAddChild: (task: TaskItem) => void;
  onEdit: (task: TaskItem) => void;
  onProgress: (task: TaskItem) => void;
  onMove: (task: TaskItem) => void;
  onHistory: (task: TaskItem) => void;
  onArchive: (task: TaskItem) => void;
  onDelete: (task: TaskItem) => void;
  onToggleDone: (task: TaskItem) => void;
  onTimeline: (task: TaskItem) => React.ReactNode;
  childrenByParent: Map<string, TaskItem[]>;
}) {
  const hasChildren = children.length > 0;
  const isCollapsed = collapsed.has(task.id);
  const done = deriveStatus(task.progress) === "done";
  const priority = PRIORITY_META[task.priority] ?? PRIORITY_META.medium;

  return (
    <>
      <div className={`group flex min-h-14 items-center gap-2 border-b border-white/5 px-2 hover:bg-white/[0.035] ${done ? "opacity-60" : ""}`}>
        <div className="flex w-[min(290px,35vw)] shrink-0 items-center gap-1.5" style={{ paddingLeft: `${depth * 16}px` }}>
          {hasChildren ? (
            <button type="button" onClick={() => onToggle(task.id)} className="flex h-5 w-5 shrink-0 items-center justify-center text-slate-500 hover:text-white" title={isCollapsed ? "展开子任务" : "折叠子任务"} aria-label={isCollapsed ? "展开子任务" : "折叠子任务"}>
              {isCollapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
            </button>
          ) : <span className="w-5 shrink-0" />}
          <button type="button" onClick={() => onToggleDone(task)} className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border ${done ? "border-emerald-500 bg-emerald-500 text-white" : "border-white/20 text-transparent hover:border-amber-400"}`} title={done ? "标记未完成" : "标记完成"} aria-label={done ? "标记未完成" : "标记完成"}>
            <Check className="h-3 w-3" />
          </button>
          <span className={`min-w-0 truncate text-[11px] font-semibold ${done ? "text-slate-500 line-through" : "text-slate-200"}`} title={task.title}>{task.title}</span>
          <span className={`shrink-0 rounded border px-1 py-0.5 text-[8px] ${priority.bg} ${priority.text}`}>{priority.label}</span>
        </div>
        <div className="relative h-14 shrink-0 border-l border-white/5" style={{ width: `${RANGE_DAYS * DAY_WIDTH}px` }}>
          {onTimeline(task)}
        </div>
        <div className="flex min-w-0 flex-1 items-center gap-2 px-2">
          <div className="h-1.5 w-20 shrink-0 overflow-hidden rounded-full bg-white/10">
            <div className={`h-full ${progressColor(task.progress)}`} style={{ width: `${task.progress}%` }} />
          </div>
          <span className="w-8 shrink-0 text-right font-mono text-[9px] text-slate-500">{task.progress}%</span>
          <span className="hidden shrink-0 items-center gap-1 text-[9px] text-slate-500 md:flex"><Clock className="h-2.5 w-2.5" />{task.estimateMinutes ? formatMinutes(task.estimateMinutes) : "1天"}</span>
        </div>
        <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
          <button type="button" onClick={() => onAddChild(task)} className="p-1 text-slate-500 hover:text-amber-300" title="添加子任务" aria-label="添加子任务"><CirclePlus className="h-3 w-3" /></button>
          <button type="button" onClick={() => onProgress(task)} className="p-1 text-slate-500 hover:text-amber-300" title="更新进度" aria-label="更新进度"><Clock className="h-3 w-3" /></button>
          <button type="button" onClick={() => onMove(task)} className="p-1 text-slate-500 hover:text-amber-300" title="改期" aria-label="改期"><CalendarClock className="h-3 w-3" /></button>
          <button type="button" onClick={() => onHistory(task)} className="p-1 text-slate-500 hover:text-amber-300" title="历史记录" aria-label="历史记录"><History className="h-3 w-3" /></button>
          <button type="button" onClick={() => onEdit(task)} className="p-1 text-slate-500 hover:text-amber-300" title="编辑任务" aria-label="编辑任务"><Edit3 className="h-3 w-3" /></button>
          <button type="button" onClick={() => onArchive(task)} className="p-1 text-slate-500 hover:text-slate-200" title={task.archived ? "取消归档" : "归档"} aria-label={task.archived ? "取消归档" : "归档"}><Archive className="h-3 w-3" /></button>
          <button type="button" onClick={() => onDelete(task)} className="p-1 text-slate-500 hover:text-red-300" title="删除任务" aria-label="删除任务"><Trash2 className="h-3 w-3" /></button>
        </div>
      </div>
      {!isCollapsed && children.map((child) => (
        <TaskTreeRow key={child.id} task={child} children={childrenByParent.get(child.id) ?? []} depth={depth + 1} collapsed={collapsed} onToggle={onToggle} onAddChild={onAddChild} onEdit={onEdit} onProgress={onProgress} onMove={onMove} onHistory={onHistory} onArchive={onArchive} onDelete={onDelete} onToggleDone={onToggleDone} onTimeline={onTimeline} childrenByParent={childrenByParent} />
      ))}
    </>
  );
}

export default function TaskTimeline() {
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [modal, setModal] = useState<ModalKind>(null);
  const [current, setCurrent] = useState<TaskItem | null>(null);
  const [parentForNew, setParentForNew] = useState<string | null>(null);
  const [focusDate, setFocusDate] = useState(todayStr());

  const rangeStart = useMemo(() => addDays(focusDate, -14), [focusDate]);
  const rangeEnd = useMemo(() => addDays(rangeStart, RANGE_DAYS - 1), [rangeStart]);
  const dates = useMemo(() => Array.from({ length: RANGE_DAYS }, (_, index) => addDays(rangeStart, index)), [rangeStart]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [scheduled, inbox] = await Promise.all([
        tasksApi.listRange(rangeStart, rangeEnd),
        tasksApi.listByDate(null),
      ]);
      setTasks([...scheduled, ...inbox]);
    } catch (error) {
      console.error("加载时间轴失败", error);
    } finally {
      setLoading(false);
    }
  }, [rangeEnd, rangeStart]);

  useEffect(() => { load(); }, [load]);

  const taskMap = useMemo(() => new Map(tasks.map((task) => [task.id, task])), [tasks]);
  const visibleTasks = useMemo(() => {
    const visible = new Set(tasks.filter((task) => task.scheduledDate && task.scheduledDate >= rangeStart && task.scheduledDate <= rangeEnd).map((task) => task.id));
    let changed = true;
    while (changed) {
      changed = false;
      tasks.forEach((task) => {
        if (task.parentId && visible.has(task.id) && !visible.has(task.parentId)) {
          visible.add(task.parentId);
          changed = true;
        }
      });
    }
    return tasks.filter((task) => visible.has(task.id));
  }, [rangeEnd, rangeStart, tasks]);
  const childrenByParent = useMemo(() => {
    const map = new Map<string, TaskItem[]>();
    visibleTasks.forEach((task) => {
      if (!task.parentId || !taskMap.has(task.parentId)) return;
      const list = map.get(task.parentId) ?? [];
      list.push(task);
      map.set(task.parentId, list);
    });
    map.forEach((list) => list.sort((a, b) => a.sortOrder - b.sortOrder));
    return map;
  }, [taskMap, visibleTasks]);
  const roots = useMemo(() => visibleTasks.filter((task) => !task.parentId || !taskMap.has(task.parentId)).sort((a, b) => a.sortOrder - b.sortOrder), [taskMap, visibleTasks]);
  const scheduledTasks = useMemo(() => visibleTasks.filter((task) => task.scheduledDate), [visibleTasks]);
  const inboxTasks = useMemo(() => tasks.filter((task) => !task.scheduledDate), [tasks]);
  const parentOptions = useMemo(() => tasks.filter((task) => task.id !== current?.id), [current?.id, tasks]);

  const openModal = (kind: ModalKind, task: TaskItem | null, parentId: string | null = null) => {
    setCurrent(task);
    setParentForNew(parentId);
    setModal(kind);
  };

  const handlers = {
    onEdit: (task: TaskItem) => openModal("edit", task),
    onProgress: (task: TaskItem) => openModal("progress", task),
    onMove: (task: TaskItem) => openModal("move", task),
    onHistory: (task: TaskItem) => openModal("history", task),
    onAddChild: (task: TaskItem) => openModal("edit", null, task.id),
    onArchive: async (task: TaskItem) => { await tasksApi.setArchived(task.id, !task.archived); await load(); },
    onDelete: async (task: TaskItem) => { if (!window.confirm(`确定删除「${task.title}」？`)) return; await tasksApi.remove(task.id); await load(); },
    onToggleDone: async (task: TaskItem) => { await tasksApi.setProgress(task.id, { progress: task.progress >= 100 ? 0 : 100 }); await load(); },
  };

  const barStyle = (task: TaskItem): React.CSSProperties => {
    if (!task.scheduledDate) return { display: "none" };
    const rawOffset = dateDistance(rangeStart, task.scheduledDate);
    if (rawOffset >= RANGE_DAYS) return { display: "none" };
    if (rawOffset < 0) return { left: "3px", width: `${DAY_WIDTH - 6}px` };
    const span = Math.min(taskSpan(task), RANGE_DAYS - rawOffset);
    return { left: `${rawOffset * DAY_WIDTH}px`, width: `${Math.max(DAY_WIDTH - 6, span * DAY_WIDTH - 6)}px` };
  };

  const renderTimelineBar = (task: TaskItem) => {
    const done = deriveStatus(task.progress) === "done";
    return <div className={`absolute top-1/2 flex h-7 -translate-y-1/2 items-center gap-1 overflow-hidden rounded-md border px-2 text-[9px] shadow-sm ${done ? "border-emerald-500/35 bg-emerald-500/20 text-emerald-200" : "border-amber-500/35 bg-amber-500/20 text-amber-100"}`} style={barStyle(task)} title={`${task.title} · ${task.scheduledDate ? humanDate(task.scheduledDate) : "未排期"}`}>
      <span className="truncate">{task.title}</span><span className="shrink-0 font-mono opacity-70">{task.progress}%</span>
    </div>;
  };

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-white/5 px-5 py-3">
        <div className="mr-auto flex items-center gap-2"><Target className="h-4 w-4 text-amber-400" /><div><h3 className="text-sm font-bold text-white">时间轴与子任务</h3><p className="text-[10px] text-slate-500">按计划日期查看任务跨度，层级关系直接可处理</p></div></div>
        <button type="button" onClick={() => setFocusDate(todayStr())} className="rounded-lg border border-white/10 bg-white/5 px-2.5 py-1.5 text-[10px] text-slate-300 hover:bg-white/10">回到今天</button>
        <button type="button" onClick={() => setFocusDate(addDays(focusDate, -7))} className="rounded-lg border border-white/10 bg-white/5 px-2.5 py-1.5 text-[10px] text-slate-300 hover:bg-white/10">前移一周</button>
        <button type="button" onClick={() => setFocusDate(addDays(focusDate, 7))} className="rounded-lg border border-white/10 bg-white/5 px-2.5 py-1.5 text-[10px] text-slate-300 hover:bg-white/10">后移一周</button>
        <button type="button" onClick={() => openModal("edit", null)} className="flex items-center gap-1 rounded-lg bg-amber-500 px-3 py-1.5 text-[10px] font-bold text-slate-900 hover:bg-amber-400"><CirclePlus className="h-3 w-3" />新建任务</button>
        <button type="button" onClick={load} className="rounded-lg border border-white/10 bg-white/5 p-1.5 text-slate-400 hover:text-white" title="刷新" aria-label="刷新"><RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} /></button>
      </div>

      {loading ? <div className="flex flex-1 items-center justify-center text-xs text-slate-500"><RefreshCw className="mr-2 h-4 w-4 animate-spin" />加载中…</div> : (
        <div className="min-h-0 flex-1 overflow-auto p-4">
          <div className="min-w-[760px]">
            <div className="mb-1 flex text-[9px] text-slate-500">
              <div className="w-[min(290px,35vw)] shrink-0 px-2">任务层级</div>
              <div className="relative h-8 flex-1 overflow-hidden border-l border-white/5" style={{ width: `${RANGE_DAYS * DAY_WIDTH}px`, flex: `0 0 ${RANGE_DAYS * DAY_WIDTH}px` }}>
                <div className="absolute inset-0 flex">{dates.map((date, index) => <div key={date} className={`flex h-full w-[42px] shrink-0 flex-col items-center justify-center border-r border-white/5 ${date === todayStr() ? "bg-amber-500/10 text-amber-300" : ""}`}><span>{Number(date.slice(-2))}</span><span className="text-[8px]">{index % 7 === 0 ? date.slice(5, 7) + "月" : ""}</span></div>)}</div>
              </div>
            </div>
            <div className="rounded-xl border border-white/10 bg-slate-950/30">
              {roots.length === 0 ? <div className="px-4 py-12 text-center text-[11px] text-slate-600">时间范围内暂无已排期任务</div> : roots.map((task) => <React.Fragment key={task.id}><TaskTreeRow task={task} children={childrenByParent.get(task.id) ?? []} depth={0} collapsed={collapsed} onToggle={(id) => setCollapsed((previous) => { const next = new Set(previous); next.has(id) ? next.delete(id) : next.add(id); return next; })} onTimeline={renderTimelineBar} childrenByParent={childrenByParent} {...handlers} /><div className="relative ml-[min(290px,35vw)] h-0" /></React.Fragment>)}
            </div>

            <div className="mt-4 grid gap-3 lg:grid-cols-2">
              {inboxTasks.length > 0 && <div className="rounded-xl border border-white/10 bg-slate-900/30 p-3"><h4 className="mb-2 text-[10px] font-bold text-slate-300">收集箱（{inboxTasks.length}）</h4><div className="space-y-2">{inboxTasks.map((task) => <TaskCard key={task.id} task={task} {...handlers} />)}</div></div>}
              <div className="rounded-xl border border-white/10 bg-slate-900/30 p-3"><h4 className="mb-2 text-[10px] font-bold text-slate-300">当前区间</h4><p className="text-[10px] text-slate-500">{rangeStart} 至 {rangeEnd} · {scheduledTasks.length} 个排期任务</p><div className="mt-2 flex flex-wrap gap-1">{["urgent", "high", "medium", "low"].map((priority) => <span key={priority} className={`rounded border px-1.5 py-0.5 text-[9px] ${PRIORITY_META[priority as keyof typeof PRIORITY_META].bg} ${PRIORITY_META[priority as keyof typeof PRIORITY_META].text}`}>{PRIORITY_META[priority as keyof typeof PRIORITY_META].label}</span>)}</div></div>
            </div>
          </div>
        </div>
      )}

      {modal === "edit" && <TaskEditModal task={current} defaultDate={focusDate} defaultParentId={parentForNew} parentOptions={parentOptions} onClose={() => setModal(null)} onSaved={load} />}
      {modal === "progress" && current && <TaskProgressModal task={current} onClose={() => setModal(null)} onSaved={load} />}
      {modal === "move" && current && <TaskMoveModal task={current} onClose={() => setModal(null)} onSaved={load} />}
      {modal === "history" && current && <TaskHistoryModal task={current} onClose={() => setModal(null)} />}
    </div>
  );
}
