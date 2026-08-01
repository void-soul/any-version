import React, { useState, useEffect, useCallback, useMemo } from "react";
import { ChevronLeft, ChevronRight, RefreshCw, CalendarDays } from "lucide-react";
import TaskCard from "./TaskCard";
import { TaskEditModal, TaskProgressModal, TaskMoveModal, TaskHistoryModal } from "./TaskModals";
import { TaskItem, tasksApi, toDateStr, todayStr, humanDate, deriveStatus } from "./types";

type ModalKind = "edit" | "progress" | "move" | "history" | null;

/** 生成月历网格（周一为起始，补齐前后空位共 6 周） */
function buildGrid(year: number, month: number): string[] {
  const first = new Date(year, month, 1);
  // JS: 0=周日，转换为周一起始的偏移
  const offset = (first.getDay() + 6) % 7;
  const start = new Date(year, month, 1 - offset);
  return Array.from({ length: 42 }, (_, i) => {
    const d = new Date(start);
    d.setDate(start.getDate() + i);
    return toDateStr(d);
  });
}

export default function TaskCalendar() {
  const now = new Date();
  const [year, setYear] = useState(now.getFullYear());
  const [month, setMonth] = useState(now.getMonth());
  const [selected, setSelected] = useState(todayStr());
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [loading, setLoading] = useState(true);

  const [modal, setModal] = useState<ModalKind>(null);
  const [current, setCurrent] = useState<TaskItem | null>(null);

  const grid = useMemo(() => buildGrid(year, month), [year, month]);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list = await tasksApi.listRange(grid[0], grid[grid.length - 1]);
      setTasks(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, [grid]);

  useEffect(() => {
    load();
  }, [load]);

  /** 按日期分组 */
  const byDate = useMemo(() => {
    const map: Record<string, TaskItem[]> = {};
    for (const t of tasks) {
      if (!t.scheduledDate) continue;
      (map[t.scheduledDate] ||= []).push(t);
    }
    return map;
  }, [tasks]);

  const selectedTasks = byDate[selected] ?? [];

  const shiftMonth = (delta: number) => {
    const d = new Date(year, month + delta, 1);
    setYear(d.getFullYear());
    setMonth(d.getMonth());
  };

  const openModal = (kind: ModalKind, t: TaskItem | null) => {
    setCurrent(t);
    setModal(kind);
  };

  const cardHandlers = {
    onEdit: (t: TaskItem) => openModal("edit", t),
    onProgress: (t: TaskItem) => openModal("progress", t),
    onMove: (t: TaskItem) => openModal("move", t),
    onHistory: (t: TaskItem) => openModal("history", t),
    onArchive: async (t: TaskItem) => {
      await tasksApi.setArchived(t.id, !t.archived);
      await load();
    },
    onDelete: async (t: TaskItem) => {
      if (!window.confirm(`确定删除「${t.title}」？`)) return;
      await tasksApi.remove(t.id);
      await load();
    },
    onToggleDone: async (t: TaskItem) => {
      await tasksApi.setProgress(t.id, { progress: t.progress >= 100 ? 0 : 100 });
      await load();
    },
  };

  return (
    <div className="h-full flex min-h-0">
      {/* 左：月历 */}
      <div className="flex-1 min-w-0 overflow-y-auto p-6 space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <button
              onClick={() => shiftMonth(-1)}
              className="p-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-white cursor-pointer transition-all"
            >
              <ChevronLeft className="w-3.5 h-3.5" />
            </button>
            <h3 className="text-sm font-bold text-white min-w-24 text-center">
              {year} 年 {month + 1} 月
            </h3>
            <button
              onClick={() => shiftMonth(1)}
              className="p-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-white cursor-pointer transition-all"
            >
              <ChevronRight className="w-3.5 h-3.5" />
            </button>
          </div>
          <div className="flex gap-2">
            <button
              onClick={() => {
                const d = new Date();
                setYear(d.getFullYear());
                setMonth(d.getMonth());
                setSelected(todayStr());
              }}
              className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
            >
              <CalendarDays className="w-3 h-3" /> 今天
            </button>
            <button
              onClick={load}
              className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
            >
              <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} /> 刷新
            </button>
          </div>
        </div>

        {/* 星期表头 */}
        <div className="grid grid-cols-7 gap-1">
          {["一", "二", "三", "四", "五", "六", "日"].map((w) => (
            <div key={w} className="text-center text-[9px] text-slate-500 font-semibold py-1">
              {w}
            </div>
          ))}
        </div>

        {/* 日期格子 */}
        <div className="grid grid-cols-7 gap-1">
          {grid.map((d) => {
            const list = byDate[d] ?? [];
            const total = list.length;
            const done = list.filter((t) => deriveStatus(t.progress) === "done").length;
            const allDone = total > 0 && done === total;
            const isCurMonth = Number(d.split("-")[1]) === month + 1;
            const isToday = d === todayStr();
            const isSel = d === selected;

            return (
              <button
                key={d}
                onClick={() => setSelected(d)}
                className={`aspect-square p-1 rounded-lg border flex flex-col items-center justify-start transition-all cursor-pointer ${
                  isSel
                    ? "bg-amber-500/20 border-amber-500/50"
                    : isToday
                    ? "bg-white/5 border-amber-500/30"
                    : "bg-slate-900/30 border-white/5 hover:border-white/15"
                } ${isCurMonth ? "" : "opacity-35"}`}
              >
                <span
                  className={`text-[10px] font-semibold ${
                    isToday ? "text-amber-300" : "text-slate-300"
                  }`}
                >
                  {Number(d.split("-")[2])}
                </span>
                {total > 0 && (
                  <>
                    <span
                      className={`text-[8px] font-mono mt-0.5 ${
                        allDone ? "text-emerald-400" : "text-slate-400"
                      }`}
                    >
                      {done}/{total}
                    </span>
                    {/* 完成度小进度条 */}
                    <div className="w-full h-0.5 rounded-full bg-white/10 mt-auto overflow-hidden">
                      <div
                        className={`h-full ${allDone ? "bg-emerald-500" : "bg-amber-400"}`}
                        style={{ width: `${(done / total) * 100}%` }}
                      />
                    </div>
                  </>
                )}
              </button>
            );
          })}
        </div>
      </div>

      {/* 右：所选日期的任务 */}
      <div className="w-80 flex-shrink-0 border-l border-white/5 overflow-y-auto p-4 space-y-2">
        <div className="flex items-center justify-between">
          <h4 className="text-xs font-bold text-white">{humanDate(selected)}</h4>
          <button
            onClick={() => openModal("edit", null)}
            className="px-2 py-1 rounded-lg bg-amber-500 hover:bg-amber-400 text-slate-900 text-[9px] font-bold cursor-pointer transition-all"
          >
            + 新建
          </button>
        </div>
        {selectedTasks.length === 0 ? (
          <p className="text-[10px] text-slate-600 py-6 text-center">这一天没有任务</p>
        ) : (
          selectedTasks.map((t) => <TaskCard key={t.id} task={t} {...cardHandlers} />)
        )}
      </div>

      {/* 弹窗 */}
      {modal === "edit" && (
        <TaskEditModal
          task={current}
          defaultDate={selected}
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
