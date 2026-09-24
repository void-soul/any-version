import { afterEach, describe, expect, it, vi } from "vitest";
import {
  clampPaneWidth,
  DEFAULT_PANE_WIDTHS,
  loadPaneWidths,
  PANE_WIDTH_LIMITS,
  savePaneWidths,
  type PaneKey,
} from "../paneWidths";

/** 内存版 localStorage（vitest 为 node 环境，无内置实现）。 */
function stubStorage(initial: Record<string, string> = {}) {
  const map = new Map(Object.entries(initial));
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => (map.has(k) ? map.get(k)! : null),
    setItem: (k: string, v: string) => { map.set(k, v); },
    removeItem: (k: string) => { map.delete(k); },
  });
  return map;
}

afterEach(() => vi.unstubAllGlobals());

describe("clampPaneWidth", () => {
  it("区间内取整", () => {
    expect(clampPaneWidth("tree", 240.4)).toBe(240);
    expect(clampPaneWidth("tree", 240.6)).toBe(241);
  });

  it("越界夹到上下限", () => {
    for (const pane of ["sidebar", "tree", "ai"] as PaneKey[]) {
      const [min, max] = PANE_WIDTH_LIMITS[pane];
      expect(clampPaneWidth(pane, -1)).toBe(min);
      expect(clampPaneWidth(pane, 99_999)).toBe(max);
    }
  });

  it("非有限数回落该栏默认值", () => {
    expect(clampPaneWidth("ai", Number.NaN)).toBe(DEFAULT_PANE_WIDTHS.ai);
    expect(clampPaneWidth("ai", Number.POSITIVE_INFINITY)).toBe(DEFAULT_PANE_WIDTHS.ai);
  });
});

describe("loadPaneWidths / savePaneWidths", () => {
  it("没有存储时返回默认三栏宽度", () => {
    stubStorage();
    expect(loadPaneWidths()).toEqual(DEFAULT_PANE_WIDTHS);
  });

  it("能存能取", () => {
    stubStorage();
    savePaneWidths({ sidebar: 300, tree: 200, ai: 500 });
    expect(loadPaneWidths()).toEqual({ sidebar: 300, tree: 200, ai: 500 });
  });

  it("损坏的 JSON 回落默认值而不是抛错", () => {
    stubStorage({ any_version_mindmap_pane_widths: "{oops" });
    expect(loadPaneWidths()).toEqual(DEFAULT_PANE_WIDTHS);
  });

  it("越界/非数字字段逐项回落或夹紧，其余字段保留", () => {
    stubStorage({
      any_version_mindmap_pane_widths: JSON.stringify({ sidebar: 9999, tree: "big", ai: 333.6 }),
    });
    const loaded = loadPaneWidths();
    expect(loaded.sidebar).toBe(PANE_WIDTH_LIMITS.sidebar[1]);
    expect(loaded.tree).toBe(DEFAULT_PANE_WIDTHS.tree);
    expect(loaded.ai).toBe(334);
  });

  it("localStorage 不可用时读取回落默认、写入不抛错", () => {
    vi.stubGlobal("localStorage", undefined);
    expect(loadPaneWidths()).toEqual(DEFAULT_PANE_WIDTHS);
    expect(() => savePaneWidths({ sidebar: 300, tree: 200, ai: 500 })).not.toThrow();
  });
});
