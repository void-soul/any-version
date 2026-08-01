import React from "react";
import {
  Check,
  Clock,
  Edit3,
  CalendarClock,
  Archive,
  Trash2,
  History,
  Tag as TagIcon,
  GripVertical,
} from "lucide-react";
import {
  TaskItem,
  PRIORITY_META,
  STATUS_META,
  deriveStatus,
  progressColor,
  formatMinutes,
  parseTags,
  humanDate,
  todayStr,
} from "./types";

interface Props {
  task: TaskItem;
  /** 显示计划日期（搜索/复盘等跨天列表里用） */
  showDate?: boolean;
  /** 是否开启拖拽排序（仅今日列表启用） */
  draggable?: boolean;
  onDragStart?: (e: React.DragEvent, task: TaskItem) => void;
  onDragEnd?: (e: React.DragEvent) => void;
  onDragOver?: (e: React.DragEvent, task: TaskItem) => void;
  onDrop?: (e: React.DragEvent, task: TaskItem) => void;
  onEdit: (t: TaskItem) => void;
  onProgress: (t: TaskItem) => void;
  onMove: (t: TaskItem) => void;
  onHistory: (t: TaskItem) => void;
  onArchive: (t: TaskItem) => void;
  onDelete: (t: TaskItem) => void;
  /** 一键完成/取消完成 */
  onToggleDone: (t: TaskItem) => void;
}

export default function TaskCard({
  task,
  showDate,
  draggable,
  onDragStart,
  onDragEnd,
  onDragOver,
  onDrop,
  onEdit,
  onProgress,
  onMove,
  onHistory,
  onArchive,
  onDelete,
  onToggleDone,
}: Props) {
  const status = deriveStatus(task.progress);
  const prio = PRIORITY_META[task.priority] ?? PRIORITY_META.medium;
  const done = status === "done";
  const tags = parseTags(task.tags);
  const overdue =
    !done && !!task.scheduledDate && task.scheduledDate < todayStr();

  return (
    <div
      draggable={draggable}
      onDragStart={(e) => onDragStart?.(e, task)}
      onDragEnd={onDragEnd}
      onDragOver={(e) => onDragOver?.(e, task)}
      onDrop={(e) => onDrop?.(e, task)}
      data-task-id={task.id}
      className={`group p-3 rounded-xl border transition-all ${
        done
          ? "bg-slate-900/30 border-white/5 opacity-60"
          : overdue
          ? "bg-slate-900/40 border-red-500/25 hover:border-red-500/40"
          : "bg-slate-900/40 border-white/10 hover:border-amber-500/30"
      }`}
    >
      <div className="flex items-start gap-2.5">
        {/* 拖拽手柄（仅开启拖拽时显示） */}
        {draggable && (
          <div
            title="拖拽排序"
            className="mt-0.5 w-3.5 h-4 flex-shrink-0 flex items-center justify-center text-slate-600 hover:text-amber-400 cursor-grab active:cursor-grabbing opacity-0 group-hover:opacity-100 transition-opacity"
          >
            <GripVertical className="w-3.5 h-3.5" />
          </div>
        )}
        {/* 完成勾选 */}
        <button
          onClick={() => onToggleDone(task)}
          title={done ? "标记为未完成" : "标记为已完成"}
          className={`mt-0.5 w-4 h-4 flex-shrink-0 rounded border flex items-center justify-center transition-all cursor-pointer ${
            done
              ? "bg-emerald-500 border-emerald-500 text-white"
              : "border-white/20 hover:border-amber-400 text-transparent hover:text-amber-400"
          }`}
        >
          <Check className="w-3 h-3" />
        </button>

        <div className="flex-1 min-w-0">
          {/* 标题行 */}
          <div className="flex items-center gap-2 flex-wrap">
            <span
              className={`text-xs font-semibold ${
                done ? "text-slate-500 line-through" : "text-slate-100"
              }`}
            >
              {task.title}
            </span>
            <span
              className={`px-1.5 py-0.5 rounded border text-[9px] font-semibold ${prio.bg} ${prio.text}`}
            >
              {prio.label}
            </span>
            {overdue && (
              <span className="px-1.5 py-0.5 rounded border border-red-500/30 bg-red-500/10 text-[9px] font-semibold text-red-300">
                逾期
              </span>
            )}
            {showDate && task.scheduledDate && (
              <span className="text-[9px] text-slate-500 flex items-center gap-0.5">
                <CalendarClock className="w-2.5 h-2.5" />
                {humanDate(task.scheduledDate)}
              </span>
            )}
          </div>

          {/* 描述 */}
          {task.description && (
            <p className="text-[10px] text-slate-500 mt-1 line-clamp-2 whitespace-pre-wrap">
              {task.description}
            </p>
          )}

          {/* 进度条 */}
          <div className="flex items-center gap-2 mt-2">
            <div className="flex-1 h-1.5 rounded-full bg-white/5 overflow-hidden">
              <div
                className={`h-full rounded-full transition-all ${progressColor(task.progress)}`}
                style={{ width: `${task.progress}%` }}
              />
            </div>
            <span
              className={`text-[9px] font-mono font-semibold w-9 text-right ${STATUS_META[status].text}`}
            >
              {task.progress}%
            </span>
          </div>

          {/* 元信息 */}
          <div className="flex items-center gap-2.5 mt-1.5 flex-wrap">
            <span className={`text-[9px] font-semibold ${STATUS_META[status].text}`}>
              {STATUS_META[status].label}
            </span>
            {task.estimateMinutes > 0 && (
              <span className="text-[9px] text-slate-500 flex items-center gap-0.5">
                <Clock className="w-2.5 h-2.5" />
                预计 {formatMinutes(task.estimateMinutes)}
              </span>
            )}
            {tags.map((t) => (
              <span
                key={t}
                className="text-[9px] text-amber-300/70 flex items-center gap-0.5"
              >
                <TagIcon className="w-2.5 h-2.5" />
                {t}
              </span>
            ))}
          </div>
        </div>

        {/* 操作区（hover 显示） */}
        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
          <IconBtn title="更新进度 / 写日志" onClick={() => onProgress(task)}>
            <Clock className="w-3 h-3" />
          </IconBtn>
          <IconBtn title="改期 / 结转" onClick={() => onMove(task)}>
            <CalendarClock className="w-3 h-3" />
          </IconBtn>
          <IconBtn title="历史记录" onClick={() => onHistory(task)}>
            <History className="w-3 h-3" />
          </IconBtn>
          <IconBtn title="编辑" onClick={() => onEdit(task)}>
            <Edit3 className="w-3 h-3" />
          </IconBtn>
          <IconBtn
            title={task.archived ? "取消归档" : "归档"}
            onClick={() => onArchive(task)}
          >
            <Archive className="w-3 h-3" />
          </IconBtn>
          <IconBtn title="删除" danger onClick={() => onDelete(task)}>
            <Trash2 className="w-3 h-3" />
          </IconBtn>
        </div>
      </div>
    </div>
  );
}

function IconBtn({
  title,
  onClick,
  danger,
  children,
}: {
  title: string;
  onClick: () => void;
  danger?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      className={`p-1.5 rounded-lg transition-all cursor-pointer ${
        danger
          ? "text-slate-500 hover:text-red-400 hover:bg-red-500/10"
          : "text-slate-500 hover:text-amber-300 hover:bg-white/5"
      }`}
    >
      {children}
    </button>
  );
}
