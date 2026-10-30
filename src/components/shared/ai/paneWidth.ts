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

// ── AI 助手右侧栏（数据库设计器等模块共用）──
// 与思维导图 AI 栏同构：右贴边、左缘拖宽、宽度持久化。区间与默认值对齐
// 思维导图的 ai 栏（300~640 / 440），两个模块的 AI 栏手感一致；
// 存储键各自独立，互不影响对方的布局记忆。

export const AI_PANEL_WIDTH_MIN = 300;
export const AI_PANEL_WIDTH_MAX = 640;
export const AI_PANEL_WIDTH_DEFAULT = 440;

const AI_PANEL_STORAGE_KEY = "any_version_ai_panel_width";

export function clampAiPanelWidth(value: number): number {
  if (!Number.isFinite(value)) return AI_PANEL_WIDTH_DEFAULT;
  return Math.min(AI_PANEL_WIDTH_MAX, Math.max(AI_PANEL_WIDTH_MIN, Math.round(value)));
}

export function loadAiPanelWidth(): number {
  if (typeof localStorage === "undefined") return AI_PANEL_WIDTH_DEFAULT;
  try {
    const raw = localStorage.getItem(AI_PANEL_STORAGE_KEY);
    if (!raw) return AI_PANEL_WIDTH_DEFAULT;
    return clampAiPanelWidth(Number(JSON.parse(raw)));
  } catch {
    return AI_PANEL_WIDTH_DEFAULT;
  }
}

export function saveAiPanelWidth(width: number): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(AI_PANEL_STORAGE_KEY, JSON.stringify(clampAiPanelWidth(width)));
  } catch {
    /* 隐私模式 / 配额满：忽略，宽度在本次会话内仍生效 */
  }
}

/** AI 助手侧栏宽度的受控状态 + 持久化。 */
export function useAiPanelWidth(): [number, (width: number) => void] {
  const [width, setWidth] = useState(loadAiPanelWidth);
  const update = useCallback((next: number) => {
    const value = clampAiPanelWidth(next);
    setWidth(value);
    saveAiPanelWidth(value);
  }, []);
  return [width, update];
}
