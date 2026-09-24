// ════════════ 任务计划日历：日期区间与网格计算（纯函数） ════════════
//
// 约定：一周从**周一**开始（与国内日历习惯一致）；月视图固定 6×7 = 42 格、
// 跨月补齐 —— 固定格数才不会在切月时让整个网格高度跳动。
//
// 全部按本地日期字符串（YYYY-MM-DD）计算，不碰 UTC，避免「跨天差一天」。

import { addDays, toDateStr } from "./types";

/** 解析 YYYY-MM-DD 为本地 Date（不用 new Date(str)：那会按 UTC 解析）。 */
function parse(dateStr: string): Date {
  const [y, m, d] = dateStr.split("-").map(Number);
  return new Date(y, m - 1, d);
}

/** 该日期所在周的周一。 */
export function weekStartOf(dateStr: string): string {
  const dt = parse(dateStr);
  const mondayBased = (dt.getDay() + 6) % 7; // 0 = 周一
  return addDays(dateStr, -mondayBased);
}

/** 该日期所在周（周一 ~ 周日）的闭区间。 */
export function weekRange(dateStr: string): { start: string; end: string } {
  const start = weekStartOf(dateStr);
  return { start, end: addDays(start, 6) };
}

/** 该日期所在月的闭区间（1 号 ~ 月末）。 */
export function monthRange(dateStr: string): { start: string; end: string } {
  const dt = parse(dateStr);
  const y = dt.getFullYear();
  const m = dt.getMonth();
  const pad = (n: number) => String(n).padStart(2, "0");
  const lastDay = new Date(y, m + 1, 0).getDate();
  return { start: `${y}-${pad(m + 1)}-01`, end: `${y}-${pad(m + 1)}-${pad(lastDay)}` };
}

/**
 * 月视图的 42 个格子（含跨月补齐），从所在周的周一开始连续 6 周。
 *
 * 取的是**包含当月 1 号的那一周**的周一作为起点，因此当月每天都必然落在网格内。
 */
export function monthGrid(dateStr: string): string[] {
  const first = monthRange(dateStr).start;
  const gridStart = weekStartOf(first);
  return Array.from({ length: 42 }, (_, i) => addDays(gridStart, i));
}

/** 按月平移锚点（跨年由 Date 归一，避免手写进位）。 */
export function shiftMonth(dateStr: string, delta: number): string {
  const dt = parse(dateStr);
  const target = new Date(dt.getFullYear(), dt.getMonth() + delta, 1);
  return toDateStr(target);
}

/**
 * 是否逾期：排期早于今天且未完成。
 *
 * 未排期（`scheduledDate` 为空，即收集箱里的任务）不算逾期 —— 它没有承诺过日期。
 */
export function isOverdue(
  task: { progress: number; scheduledDate: string | null },
  today: string,
): boolean {
  if (!task.scheduledDate) return false;
  return task.progress < 100 && task.scheduledDate < today;
}
