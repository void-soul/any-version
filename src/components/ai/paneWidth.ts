import { useCallback, useState } from "react";

/**
 * AI 工具页两栏布局（工具列表 / 工具详情）的宽度持久化。
 *
 * 与思维导图分栏同一套做法（`mindmap/paneWidths.ts`）：纯 UI 偏好走 localStorage，
 * 不值得为它开一个后端命令；读到坏值/越界值一律夹紧回落，绝不把脏数据写回。
 */

export const LIST_WIDTH_MIN = 150;
export const LIST_WIDTH_MAX = 420;
export const LIST_WIDTH_DEFAULT = 208;

const STORAGE_KEY = "any_version_ai_tool_list_width";

/** 夹紧到合法区间并取整（NaN / Infinity 回落到默认宽度）。 */
export function clampListWidth(value: number): number {
  if (!Number.isFinite(value)) return LIST_WIDTH_DEFAULT;
  return Math.min(LIST_WIDTH_MAX, Math.max(LIST_WIDTH_MIN, Math.round(value)));
}

export function loadListWidth(): number {
  if (typeof localStorage === "undefined") return LIST_WIDTH_DEFAULT;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return LIST_WIDTH_DEFAULT;
    const parsed = Number(JSON.parse(raw));
    return clampListWidth(parsed);
  } catch {
    // 存储不可用或值损坏：用默认宽度，不阻断渲染
    return LIST_WIDTH_DEFAULT;
  }
}

export function saveListWidth(width: number): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(clampListWidth(width)));
  } catch {
    /* 隐私模式 / 配额满：忽略，宽度在本次会话内仍生效 */
  }
}

/** 工具列表宽度的受控状态 + 持久化。 */
export function useToolListWidth(): [number, (width: number) => void] {
  const [width, setWidth] = useState(loadListWidth);
  const update = useCallback((next: number) => {
    const value = clampListWidth(next);
    setWidth(value);
    saveListWidth(value);
  }, []);
  return [width, update];
}
