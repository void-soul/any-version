import { useCallback, useState } from "react";

/**
 * 思维导图四栏布局里各分栏宽度的持久化。
 *
 * 与模块内既有做法一致（`MM_LAST_DOC_KEY` 也是 localStorage）：纯 UI 偏好，
 * 同步读取、无需后端往返；读到坏值/越界值时回落默认并夹紧，绝不把脏数据写回。
 * 第三栏（画布）是 flex-1 自适应，不在持久化范围内。
 */

export type PaneKey = "sidebar" | "tree" | "ai";

/** 各栏宽度的取值范围（与拖拽把手一致） */
export const PANE_WIDTH_LIMITS: Record<PaneKey, readonly [number, number]> = {
  sidebar: [170, 460],
  tree: [160, 480],
  ai: [300, 640],
};

export const DEFAULT_PANE_WIDTHS: Record<PaneKey, number> = {
  sidebar: 260,
  tree: 224,
  ai: 440,
};

const STORAGE_KEY = "any_version_mindmap_pane_widths";
const PANES: PaneKey[] = ["sidebar", "tree", "ai"];

/** 夹紧到该栏的合法区间并取整（NaN/Infinity 回落到默认值）。 */
export function clampPaneWidth(pane: PaneKey, value: number): number {
  const [min, max] = PANE_WIDTH_LIMITS[pane];
  if (!Number.isFinite(value)) return DEFAULT_PANE_WIDTHS[pane];
  return Math.min(max, Math.max(min, Math.round(value)));
}

/** 读取持久化的三栏宽度：缺失/损坏/越界的字段各自回落默认值。 */
export function loadPaneWidths(): Record<PaneKey, number> {
  const fallback = { ...DEFAULT_PANE_WIDTHS };
  if (typeof localStorage === "undefined") return fallback;
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(STORAGE_KEY);
  } catch {
    return fallback;
  }
  if (!raw) return fallback;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return fallback;
  }
  if (!parsed || typeof parsed !== "object") return fallback;
  const source = parsed as Record<string, unknown>;
  for (const pane of PANES) {
    const value = source[pane];
    if (typeof value === "number") fallback[pane] = clampPaneWidth(pane, value);
  }
  return fallback;
}

/** 写入三栏宽度（写失败不影响界面，宽度仍生效在本次会话内）。 */
export function savePaneWidths(widths: Record<PaneKey, number>): void {
  if (typeof localStorage === "undefined") return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(widths));
  } catch {
    /* 存储不可用（隐私模式/配额满）：忽略，不阻断拖拽 */
  }
}

/**
 * 单个分栏宽度的受控状态 + 持久化。
 *
 * 写入时重新读一次存量再合并：三栏可能由不同组件各自持有状态
 * （节点树在画布内、文档栏与 AI 栏在主面板），这样避免互相覆盖。
 */
export function usePaneWidth(pane: PaneKey): [number, (width: number) => void] {
  const [width, setWidth] = useState(() => loadPaneWidths()[pane]);
  const setPaneWidth = useCallback(
    (next: number) => {
      const value = clampPaneWidth(pane, next);
      setWidth(value);
      savePaneWidths({ ...loadPaneWidths(), [pane]: value });
    },
    [pane],
  );
  return [width, setPaneWidth];
}
