import { invoke } from "@tauri-apps/api/core";

// ─── 类型定义（与 src-tauri/src/commands/tasks/models.rs 一一对应） ───

export type TaskPriority = "low" | "medium" | "high" | "urgent";
export type TaskStatus = "todo" | "inProgress" | "done";

export interface TaskItem {
  id: string;
  title: string;
  description: string;
  /** 计划日期 YYYY-MM-DD，null 表示未排期（收集箱） */
  scheduledDate: string | null;
  /** 父任务 ID，null 表示顶层任务 */
  parentId: string | null;
  priority: TaskPriority;
  /** 0-100，完成度唯一真相来源 */
  progress: number;
  sortOrder: number;
  estimateMinutes: number;
  /** 逗号分隔 */
  tags: string;
  archived: boolean;
  createdAt: string;
  updatedAt: string;
  completedAt: string | null;
}

export interface TaskLog {
  id: string;
  taskId: string;
  logDate: string;
  content: string;
  progressBefore: number;
  progressAfter: number;
  minutesSpent: number;
  createdAt: string;
}

export interface TaskMoveRecord {
  id: string;
  taskId: string;
  fromStatus: string;
  fromProgress: number;
  fromDate: string | null;
  toStatus: string;
  toProgress: number;
  toDate: string | null;
  reason: string;
  movedAt: string;
}

export interface TaskSummary {
  date: string;
  total: number;
  completed: number;
  inProgress: number;
  pending: number;
  totalEstimate: number;
  totalSpent: number;
}

export interface DayStat {
  date: string;
  total: number;
  completed: number;
  inProgress: number;
  pending: number;
  minutesSpent: number;
}

/// 提醒弹窗用的精简任务项。
export interface TaskBrief {
  id: string;
  title: string;
  priority: string;
  progress: number;
  scheduledDate?: string | null;
}

/// 启动后的今日待办提醒数据。
export interface ReminderData {
  today: string;
  todayPending: TaskBrief[];
  overdue: TaskBrief[];
}

export interface CreateTaskInput {
  title: string;
  description?: string;
  scheduledDate?: string | null;
  parentId?: string | null;
  priority?: TaskPriority;
  progress?: number;
  estimateMinutes?: number;
  tags?: string;
}

export interface UpdateTaskInput {
  title?: string;
  description?: string;
  scheduledDate?: string | null;
  parentId?: string | null;
  priority?: TaskPriority;
  estimateMinutes?: number;
  tags?: string;
  archived?: boolean;
  progress?: number;
  sortOrder?: number;
}

export interface SetProgressInput {
  progress: number;
  logContent?: string;
  minutesSpent?: number;
  /** 未完成时把任务结转到该日期 */
  carryToDate?: string | null;
  moveReason?: string;
}

export interface MoveTaskInput {
  toDate?: string | null;
  toProgress?: number;
  reason?: string;
}

// ─── 状态派生（必须与后端 derive_status 保持一致） ───

export function deriveStatus(progress: number): TaskStatus {
  if (progress >= 100) return "done";
  if (progress > 0) return "inProgress";
  return "todo";
}

// ─── UI 展示常量（任务模块主题色 = 黄色 amber） ───

export const PRIORITY_META: Record<TaskPriority, { label: string; text: string; bg: string; dot: string }> = {
  urgent: { label: "紧急", text: "text-red-300", bg: "bg-red-500/10 border-red-500/25", dot: "bg-red-500" },
  high: { label: "高", text: "text-orange-300", bg: "bg-orange-500/10 border-orange-500/25", dot: "bg-orange-500" },
  medium: { label: "中", text: "text-amber-300", bg: "bg-amber-500/10 border-amber-500/25", dot: "bg-amber-500" },
  low: { label: "低", text: "text-slate-400", bg: "bg-white/5 border-white/10", dot: "bg-slate-500" },
};

export const STATUS_META: Record<TaskStatus, { label: string; text: string }> = {
  done: { label: "已完成", text: "text-emerald-400" },
  inProgress: { label: "进行中", text: "text-amber-400" },
  todo: { label: "待开始", text: "text-slate-400" },
};

/** 进度条配色：低进度偏冷、临近完成偏暖绿 */
export function progressColor(progress: number): string {
  if (progress >= 100) return "bg-emerald-500";
  if (progress >= 60) return "bg-amber-400";
  if (progress > 0) return "bg-amber-500/70";
  return "bg-slate-600";
}

// ─── 日期工具（全部使用本地时间，避免 UTC 偏移导致跨天） ───

export function toDateStr(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function todayStr(): string {
  return toDateStr(new Date());
}

export function addDays(dateStr: string, delta: number): string {
  const [y, m, d] = dateStr.split("-").map(Number);
  const dt = new Date(y, m - 1, d);
  dt.setDate(dt.getDate() + delta);
  return toDateStr(dt);
}

/** 人类友好的日期标签：今天 / 明天 / 昨天 / 07-31 周五 */
export function humanDate(dateStr: string): string {
  const today = todayStr();
  if (dateStr === today) return "今天";
  if (dateStr === addDays(today, 1)) return "明天";
  if (dateStr === addDays(today, -1)) return "昨天";
  const [y, m, d] = dateStr.split("-").map(Number);
  const week = "日一二三四五六"[new Date(y, m - 1, d).getDay()];
  return `${m}月${d}日 周${week}`;
}

export function formatMinutes(min: number): string {
  if (!min) return "0m";
  const h = Math.floor(min / 60);
  const m = min % 60;
  return h ? (m ? `${h}h${m}m` : `${h}h`) : `${m}m`;
}

export function parseTags(tags: string): string[] {
  return tags.split(",").map((t) => t.trim()).filter(Boolean);
}

// ─── 后端调用封装 ───

export const tasksApi = {
  init: () => invoke<void>("tasks_init"),
  listByDate: (date: string | null, includeArchived = false) =>
    invoke<TaskItem[]>("tasks_list_by_date", { date, includeArchived }),
  listRange: (start: string, end: string) => invoke<TaskItem[]>("tasks_list_range", { start, end }),
  search: (keyword: string, includeArchived = false) =>
    invoke<TaskItem[]>("tasks_search", { keyword, includeArchived }),
  listOverdue: (before?: string) => invoke<TaskItem[]>("tasks_list_overdue", { before: before ?? null }),
  create: (input: CreateTaskInput) => invoke<TaskItem>("tasks_create", { input }),
  update: (id: string, input: UpdateTaskInput) => invoke<TaskItem>("tasks_update", { id, input }),
  setProgress: (id: string, input: SetProgressInput) => invoke<TaskItem>("tasks_set_progress", { id, input }),
  move: (id: string, input: MoveTaskInput) => invoke<TaskItem>("tasks_move", { id, input }),
  carryOver: (toDate?: string) => invoke<number>("tasks_carry_over", { toDate: toDate ?? null }),
  reorder: (ids: string[]) => invoke<void>("tasks_reorder", { input: { ids } }),
  setArchived: (id: string, archived: boolean) => invoke<void>("tasks_set_archived", { id, archived }),
  remove: (id: string) => invoke<void>("tasks_delete", { id }),
  addLog: (taskId: string, logDate: string, content: string, minutesSpent = 0) =>
    invoke<TaskLog>("tasks_add_log", { input: { taskId, logDate, content, minutesSpent } }),
  listLogs: (taskId: string) => invoke<TaskLog[]>("tasks_list_logs", { taskId }),
  listMoves: (taskId: string) => invoke<TaskMoveRecord[]>("tasks_list_moves", { taskId }),
  summary: (date: string) => invoke<TaskSummary>("tasks_summary", { date }),
  dayStats: (start: string, end: string) => invoke<DayStat[]>("tasks_day_stats", { start, end }),
  todayReminder: () => invoke<ReminderData>("tasks_today_reminder"),
};
