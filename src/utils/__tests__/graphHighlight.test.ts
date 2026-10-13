import { describe, expect, it } from "vitest";

import {
  EDGE_DIM_STROKE,
  GRAPH_DIMMED_CLASS,
  HL_FALLBACK_RELATED,
  HL_FALLBACK_SELECT,
  edgeGlow,
  headerTint,
  highlightColor,
  neonRing,
  neonTextGlow,
} from "../graphHighlight";

/**
 * 这层样式是三个画布（数据库设计器 / 思维导图 / JSON 浏览）共用的纯函数。
 *
 * 值得钉住的是「档位区分」这条不变量：选中必须比关联更醒目。
 * 之前三处各写一份字符串，改着改着就会出现「关联比选中还亮」，
 * 而纯观感问题不会报错、只会让人觉得哪里怪。
 */
describe("graphHighlight", () => {
  it("节点有色时跟随节点色，没有才回退状态色", () => {
    expect(highlightColor("#ef4444", "select")).toContain("#ef4444");
    expect(highlightColor("#ef4444", "related")).toBe("#ef4444");
    expect(highlightColor("", "select")).toBe(HL_FALLBACK_SELECT);
    expect(highlightColor(null, "related")).toBe(HL_FALLBACK_RELATED);
    expect(highlightColor(undefined, "select")).toBe(HL_FALLBACK_SELECT);
  });

  it("选中档把表色往白里提亮（暗色表在深色画布上也看得见环）", () => {
    expect(highlightColor("#1e3a8a", "select")).toContain("white 12%");
  });

  it("选中档比关联档更醒目：描边更粗、辉光层数更多、浓度更高", () => {
    const sel = neonRing("#ef4444", "select");
    const rel = neonRing("#ef4444", "related");
    expect(sel).toContain("0 0 0 2px");
    expect(rel).toContain("0 0 0 1px");
    // 选中 4 段（描边 + 三层辉光），关联 3 段（描边 + 两层）
    expect(sel.split("color-mix").length).toBeGreaterThan(rel.split("color-mix").length);
    expect(neonTextGlow("#ef4444", "select").split("color-mix").length).toBeGreaterThan(
      neonTextGlow("#ef4444", "related").split("color-mix").length,
    );
    expect(headerTint("#ef4444", "select")).toContain("26%");
    expect(headerTint("#ef4444", "related")).toContain("14%");
  });

  it("高亮色一旦含「#」就不能是 CSS 变量（否则 color-mix 会失效）", () => {
    // 回退色本身就是 var()，调用方必须保证不把 var() 再套进 color-mix
    expect(HL_FALLBACK_SELECT.startsWith("var(")).toBe(true);
    expect(HL_FALLBACK_RELATED.startsWith("var(")).toBe(true);
    expect(edgeGlow("#22c55e")).toBe("drop-shadow(0 0 3px #22c55e)");
  });

  it("压暗用的常量与 className 对齐（换透明度只需改这一处）", () => {
    expect(GRAPH_DIMMED_CLASS).toBe("opacity-35");
    expect(EDGE_DIM_STROKE).toMatch(/^rgba\(/);
  });
});
