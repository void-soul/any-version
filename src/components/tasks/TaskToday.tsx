import React, { useState, useEffect, useCallback } from "react";
import {
  Plus,
  RefreshCw,
  ChevronLeft,
  ChevronRight,
  CalendarDays,
  AlertTriangle,
  Inbox,
  ArrowDownToLine,
  GripVertical,
} from "lucide-react";
import TaskCard from "./TaskCard";
import { TaskEditModal, TaskProgressModal, TaskMoveModal, TaskHistoryModal } from "./TaskModals";
import {
  TaskItem,
  TaskSummary,
  tasksApi,
  todayStr,
  addDays,
  humanDate,
  formatMinutes,
} from "./types";

type ModalKind = "edit" | "progress" | "move" | "history" | null;

export default function TaskToday() {
  const [date, setDate] = useState(todayStr());
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [inbox, setInbox] = useState<TaskItem[]>([]);
  const [overdue, setOverdue] = useState<TaskItem[]>([]);
  const [summary, setSummary] = useState<TaskSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [showInbox, setShowInbox] = useState(false);
  const [quickTitle, setQuickTitle] = useState("");

  const [modal, setModal] = useState<ModalKind>(null);
  const [current, setCurrent] = useState<TaskItem | null>(null);

  // 拖拽排序状态（仅今日列表启用）
  const [dragId, setDragId] = useState<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const isToday = date === todayStr();
  const dragEnabled = isToday && !loading;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [list, inboxList, sum, over] = await Promise.all([
        tasksApi.listByDate(date),
        tasksApi.listByDate(null),
        tasksApi.summary(date),
        // 仅在查看今天时提示逾期
        date === todayStr() ? tasksApi.listOverdue(date) : Promise.resolve([] as TaskItem[]),
      ]);
      setTasks(list);
      setInbox(inboxList);
      setSummary(sum);
      setOverdue(over);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, [date]);

  useEffect(() => {
    load();
  }, [load]);

  const openModal = (kind: ModalKind, t: TaskItem | null) => {
    setCurrent(t);
    setModal(kind);
  };

  const quickAdd = async () => {
    const title = quickTitle.trim();
    if (!title) return;
    setQuickTitle("");
    try {
      await tasksApi.create({ title, scheduledDate: date });
      await load();
    } catch (e) {
      console.error(e);
    }
  };

  const toggleDone = async (t: TaskItem) => {
    try {
      await tasksApi.setProgress(t.id, { progress: t.progress >= 100 ? 0 : 100 });
      await load();
    } catch (e) {
      console.error(e);
    }
  };

  const archive = async (t: TaskItem) => {
    await tasksApi.setArchived(t.id, !t.archived);
    await load();
  };

  const remove = async (t: TaskItem) => {
    if (!window.confirm(`确定删除「${t.title}」？该操作不可恢复。`)) return;
    await tasksApi.remove(t.id);
    await load();
  };

  const carryOverAll = async () => {
    const n = await tasksApi.carryOver(todayStr());
    if (n > 0) await load();
  };

  const pullToToday = async (t: TaskItem) => {
    await tasksApi.move(t.id, { toDate: date, reason: "从收集箱排期" });
    await load();
  };

  const handleDragStart = (e: React.DragEvent, t: TaskItem) => {
    setDragId(t.id);
    e.dataTransfer.effectAllowed = "move";
    // 避免拖动时选中文本/触发按钮点击
    e.dataTransfer.setData("text/plain", t.id);
  };

  const handleDragOver = (e: React.DragEvent, t: TaskItem) => {
    if (!dragId || dragId === t.id) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setDragOverId(t.id);
  };

  const handleDrop = async (e: React.DragEvent, t: TaskItem) => {
    e.preventDefault();
    const from = dragId;
    setDragOverId(null);
    setDragId(null);
    if (!from || from === t.id) return;
    const fromIdx = tasks.findIndex((x) => x.id === from);
    const toIdx = tasks.findIndex((x) => x.id === t.id);
    if (fromIdx < 0 || toIdx < 0) return;
    const next = tasks.slice();
    const [moved] = next.splice(fromIdx, 1);
    next.splice(toIdx, 0, moved);
    setTasks(next); // 乐观更新
    try {
      await tasksApi.reorder(next.map((x) => x.id));
      await load();
    } catch (e) {
      console.error(e);
      await load(); // 失败回滚
    }
  };

  const handleDragEnd = () => {
    setDragId(null);
    setDragOverId(null);
  };

  const cardHandlers = {
    onEdit: (t: TaskItem) => openModal("edit", t),
    onProgress: (t: TaskItem) => openModal("progress", t),
    onMove: (t: TaskItem) => openModal("move", t),
    onHistory: (t: TaskItem) => openModal("history", t),
    onArchive: archive,
    onDelete: remove,
    onToggleDone: toggleDone,
  };

  const rate = summary && summary.total > 0
    ? Math.round((summary.completed / summary.total) * 100)
    : 0;

  return (
    <div className="h-full overflow-y-auto p-6 space-y-4">
      {/* Header：日期导航 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setDate(addDays(date, -1))}
            className="p-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-white cursor-pointer transition-all"
          >
            <ChevronLeft className="w-3.5 h-3.5" />
          </button>
          <div className="min-w-30 text-center">
            <h3 className="text-sm font-bold text-white">{humanDate(date)}</h3>
            <p className="text-[9px] text-slate-500 font-mono">{date}</p>
          </div>
          <button
            onClick={() => setDate(addDays(date, 1))}
            className="p-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-white cursor-pointer transition-all"
          >
            <ChevronRight className="w-3.5 h-3.5" />
          </button>
          {date !== todayStr() && (
            <button
              onClick={() => setDate(todayStr())}
              className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
            >
              <CalendarDays className="w-3 h-3" /> 回到今天
            </button>
          )}
        </div>

        <div className="flex gap-2">
          <button
            onClick={() => setShowInbox((v) => !v)}
            className={`px-2.5 py-1.5 rounded-lg border text-[10px] cursor-pointer transition-all flex items-center gap-1 ${
              showInbox
                ? "bg-[var(--module-accent-soft)] border-[var(--module-accent-ring)] text-[var(--module-accent)]"
                : "bg-white/5 border-white/10 text-slate-400 hover:text-white"
            }`}
          >
            <Inbox className="w-3 h-3" /> 收集箱 {inbox.length > 0 && `(${inbox.length})`}
          </button>
          <button
            onClick={load}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
          >
            <RefreshCw className="w-3 h-3" /> 刷新
          </button>
          <button
            onClick={() => openModal("edit", null)}
            className="px-3 py-1.5 rounded-lg bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-[10px] font-bold cursor-pointer transition-all flex items-center gap-1 shadow-lg shadow-[var(--module-accent-ring)]"
          >
            <Plus className="w-3 h-3" /> 新建任务
          </button>
        </div>
      </div>

      {/* 汇总卡片 */}
      {summary && (
        <div className="grid grid-cols-5 gap-2">
          <StatCard label="总计" value={summary.total} />
          <StatCard label="已完成" value={summary.completed} accent="text-emerald-400" />
          <StatCard label="进行中" value={summary.inProgress} accent="text-amber-400" />
          <StatCard label="完成率" value={`${rate}%`} accent="text-amber-300" />
          <StatCard
            label="投入 / 预计"
            value={`${formatMinutes(summary.totalSpent)} / ${formatMinutes(summary.totalEstimate)}`}
            small
          />
        </div>
      )}

      {/* 逾期提醒 */}
      {overdue.length > 0 && (
        <div className="p-3 rounded-xl bg-red-500/5 border border-red-500/20 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <AlertTriangle className="w-3.5 h-3.5 text-red-400" />
            <span className="text-[11px] text-red-300 font-semibold">
              有 {overdue.length} 个逾期未完成的任务
            </span>
          </div>
          <button
            onClick={carryOverAll}
            className="px-2.5 py-1.5 rounded-lg bg-red-500/15 border border-red-500/30 text-[10px] text-red-200 hover:bg-red-500/25 cursor-pointer transition-all flex items-center gap-1"
          >
            <ArrowDownToLine className="w-3 h-3" /> 全部结转到今天
          </button>
        </div>
      )}

      {/* 拖拽排序提示（仅今日） */}
      {isToday && tasks.length > 1 && (
        <p className="text-[10px] text-slate-600 flex items-center gap-1">
          <GripVertical className="w-3 h-3" />
          悬停卡片左侧手柄可拖拽手动排序，排序会持久化
        </p>
      )}

      {/* 快速添加 */}
      <div className="flex gap-2">
        <input
          value={quickTitle}
          onChange={(e) => setQuickTitle(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && quickAdd()}
          placeholder={`快速添加到「${humanDate(date)}」，回车即可…`}
          className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
        />
        <button
          onClick={quickAdd}
          disabled={!quickTitle.trim()}
          className="px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-300 hover:text-white cursor-pointer transition-all disabled:opacity-40 disabled:cursor-not-allowed flex items-center gap-1"
        >
          <Plus className="w-3 h-3" /> 添加
        </button>
      </div>

      {/* 收集箱 */}
      {showInbox && (
        <div className="p-3 rounded-xl bg-slate-900/40 border border-[var(--module-accent-ring)] space-y-2">
          <h4 className="text-[10px] font-bold text-[var(--module-accent)] flex items-center gap-1.5">
            <Inbox className="w-3 h-3" /> 收集箱（未排期）
          </h4>
          {inbox.length === 0 ? (
            <p className="text-[10px] text-slate-600">暂无未排期任务</p>
          ) : (
            inbox.map((t) => (
              <div key={t.id} className="flex items-center gap-2">
                <div className="flex-1 min-w-0">
                  <TaskCard task={t} {...cardHandlers} />
                </div>
                <button
                  onClick={() => pullToToday(t)}
                  title={`排期到 ${humanDate(date)}`}
                  className="px-2 py-1.5 rounded-lg bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] text-[9px] text-[var(--module-accent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_25%,transparent)] cursor-pointer transition-all flex-shrink-0"
                >
                  排期
                </button>
              </div>
            ))
          )}
        </div>
      )}

      {/* 任务列表 */}
      {loading ? (
        <div className="flex items-center justify-center py-16 text-slate-500">
          <RefreshCw className="w-4 h-4 animate-spin mr-2" />
          <span className="text-xs">加载中…</span>
        </div>
      ) : tasks.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-16 text-slate-600">
          <CalendarDays className="w-8 h-8 mb-2 opacity-40" />
          <p className="text-xs">{humanDate(date)} 还没有任务</p>
          <p className="text-[10px] mt-1">在上方输入框快速添加，或点击「新建任务」</p>
        </div>
      ) : (
        <div className="space-y-2">
          {tasks.map((t) => (
            <div key={t.id}>
              {dragEnabled && dragOverId === t.id && dragId && dragId !== t.id && (
                <div className="h-0.5 mb-1.5 rounded-full bg-amber-400/80 shadow-[0_0_8px_rgba(251,191,36,0.6)]" />
              )}
              <TaskCard
                task={t}
                draggable={dragEnabled}
                onDragStart={handleDragStart}
                onDragEnd={handleDragEnd}
                onDragOver={handleDragOver}
                onDrop={handleDrop}
                {...cardHandlers}
              />
            </div>
          ))}
        </div>
      )}

      {/* 弹窗 */}
      {modal === "edit" && (
        <TaskEditModal
          task={current}
          defaultDate={date}
          onClose={() => setModal(null)}
          onSaved={load}
        />
      )}
      {modal === "progress" && current && (
        <TaskProgressModal task={current} onClose={() => setModal(null)} onSaved={load} />
      )}
      {modal === "move" && current && (
        <TaskMoveModal task={current} onClose={() => setModal(null)} onSaved={load} />
      )}
      {modal === "history" && current && (
        <TaskHistoryModal task={current} onClose={() => setModal(null)} />
      )}
    </div>
  );
}

function StatCard({
  label,
  value,
  accent,
  small,
}: {
  label: string;
  value: string | number;
  accent?: string;
  small?: boolean;
}) {
  return (
    <div className="p-2.5 rounded-xl bg-slate-900/40 border border-white/10">
      <p className="text-[9px] text-slate-500">{label}</p>
      <p className={`${small ? "text-[11px]" : "text-lg"} font-bold ${accent ?? "text-slate-100"} mt-0.5`}>
        {value}
      </p>
    </div>
  );
}
