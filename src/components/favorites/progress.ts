// 收藏模块的进度事件 → 界面行的映射（纯函数，便于单测）。

/** 进度载荷里与「落到哪一行」有关的字段 */
export interface ProgressKeySource {
  task?: string | null;
  source?: string | null;
  stage?: string | null;
}

/**
 * 一条进度事件应该落到哪一行。
 *
 * 顺序 task → source → stage：后端的 `task` 是权威字段（导入 / 归类 / 检测各自一行），
 * 但**漏填时必须回退**——知乎导入就漏填过 task，事件照发、界面一行都不显示，
 * 看起来像「点了没反应」。
 */
export function progressKeyOf(p: ProgressKeySource): string {
  return p.task || p.source || p.stage || "";
}

/**
 * 取出「当前正在跑的任务」对应的进度行（保持事件到达顺序）。
 *
 * 过滤很重要：任务结束后后端可能还飘来一两条事件，不能让它挂在界面上。
 */
export function progressRowsOf<T extends ProgressKeySource>(
  progressMap: Record<string, T>,
  running: readonly string[],
): [string, T][] {
  return Object.entries(progressMap).filter(([key]) => running.includes(key));
}
