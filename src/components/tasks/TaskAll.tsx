import React, { useState, useEffect, useCallback, useMemo } from "react";
import { Search, RefreshCw, Plus, ListTodo } from "lucide-react";
import TaskCard from "./TaskCard";
import { TaskEditModal, TaskProgressModal, TaskMoveModal, TaskHistoryModal } from "./TaskModals";
import { TaskItem, TaskPriority, tasksApi, deriveStatus, PRIORITY_META } from "./types";

type ModalKind = "edit" | "progress" | "move" | "history" | null;
type StatusFilter = "all" | "todo" | "inProgress" | "done";

export default function TaskAll() {
  const [keyword, setKeyword] = useState("");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [priorityFilter, setPriorityFilter] = useState<TaskPriority | "all">("all");
  const [includeArchived, setIncludeArchived] = useState(false);
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [loading, setLoading] = useState(true);

  const [modal, setModal] = useState<ModalKind>(null);
  const [current, setCurrent] = useState<TaskItem | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // 空关键字时用 % 通配匹配全部
      setTasks(await tasksApi.search(keyword.trim(), includeArchived));
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, [keyword, includeArchived]);

  useEffect(() => {
    const t = setTimeout(load, 250); // 输入防抖
    return () => clearTimeout(t);
  }, [load]);

  const filtered = useMemo(
    () =>
      tasks.filter((t) => {
        if (statusFilter !== "all" && deriveStatus(t.progress) !== statusFilter) return false;
        if (priorityFilter !== "all" && t.priority !== priorityFilter) return false;
        return true;
      }),
    [tasks, statusFilter, priorityFilter]
  );

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
      if (!window.confirm(`确定删除「${t.title}」？该操作不可恢复。`)) return;
      await tasksApi.remove(t.id);
      await load();
    },
    onToggleDone: async (t: TaskItem) => {
      await tasksApi.setProgress(t.id, { progress: t.progress >= 100 ? 0 : 100 });
      await load();
    },
  };

  const chip = (active: boolean) =>
    `px-2.5 py-1 rounded-md text-[10px] font-semibold cursor-pointer transition-all ${
      active ? "bg-amber-500 text-slate-900" : "text-slate-400 hover:text-slate-200"
    }`;

  return (
    <div className="h-full overflow-y-auto p-6 space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-white">全部任务</h3>
          <p className="text-[10px] text-slate-500 mt-0.5">搜索、筛选与批量管理所有任务</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={load}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
          >
            <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} /> 刷新
          </button>
          <button
            onClick={() => openModal("edit", null)}
            className="px-3 py-1.5 rounded-lg bg-amber-500 hover:bg-amber-400 text-slate-900 text-[10px] font-bold cursor-pointer transition-all flex items-center gap-1 shadow-lg shadow-amber-500/10"
          >
            <Plus className="w-3 h-3" /> 新建任务
          </button>
        </div>
      </div>

      {/* 搜索 + 筛选 */}
      <div className="space-y-2">
        <div className="relative">
          <Search className="w-3.5 h-3.5 text-slate-500 absolute left-3 top-1/2 -translate-y-1/2" />
          <input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索标题、描述或标签…"
            className="w-full bg-slate-900 border border-white/10 rounded-lg pl-9 pr-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-amber-500"
          />
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          <div className="flex items-center gap-0.5 bg-white/5 border border-white/10 rounded-lg p-0.5">
            {(
              [
                ["all", "全部"],
                ["todo", "待开始"],
                ["inProgress", "进行中"],
                ["done", "已完成"],
              ] as [StatusFilter, string][]
            ).map(([k, label]) => (
              <button key={k} onClick={() => setStatusFilter(k)} className={chip(statusFilter === k)}>
                {label}
              </button>
            ))}
          </div>

          <div className="flex items-center gap-0.5 bg-white/5 border border-white/10 rounded-lg p-0.5">
            <button
              onClick={() => setPriorityFilter("all")}
              className={chip(priorityFilter === "all")}
            >
              全优先级
            </button>
            {(["urgent", "high", "medium", "low"] as TaskPriority[]).map((p) => (
              <button
                key={p}
                onClick={() => setPriorityFilter(p)}
                className={chip(priorityFilter === p)}
              >
                {PRIORITY_META[p].label}
              </button>
            ))}
          </div>

          <label className="flex items-center gap-1.5 text-[10px] text-slate-400 cursor-pointer">
            <input
              type="checkbox"
              checked={includeArchived}
              onChange={(e) => setIncludeArchived(e.target.checked)}
              className="accent-amber-500"
            />
            含已归档
          </label>

          <span className="text-[10px] text-slate-500 ml-auto">共 {filtered.length} 条</span>
        </div>
      </div>

      {/* 列表 */}
      {loading ? (
        <div className="flex items-center justify-center py-16 text-slate-500">
          <RefreshCw className="w-4 h-4 animate-spin mr-2" />
          <span className="text-xs">加载中…</span>
        </div>
      ) : filtered.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-16 text-slate-600">
          <ListTodo className="w-8 h-8 mb-2 opacity-40" />
          <p className="text-xs">没有符合条件的任务</p>
        </div>
      ) : (
        <div className="space-y-2">
          {filtered.map((t) => (
            <TaskCard key={t.id} task={t} showDate {...cardHandlers} />
          ))}
        </div>
      )}

      {/* 弹窗 */}
      {modal === "edit" && (
        <TaskEditModal
          task={current}
          defaultDate={null}
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
