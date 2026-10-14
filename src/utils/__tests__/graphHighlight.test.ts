import { describe, expect, it } from "vitest";

import {
  EDGE_DIM_STROKE,
  GRAPH_DIMMED_CLASS,
  HL_FALLBACK_RELATED,
  HL_FALLBACK_SELECT,
  DIM_MAX,
  HL_BOOST_MAX,
  clampPref,
  dimmedEdgeStroke,
  dimmedEdgeWidth,
  dimmedOpacity,
  readPref,
  relatedEdgeWidth,
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

/**
 * 「高亮强度 / 淡化强度」两个滑块（2026-10 加）。
 *
 * 最该钉住的是**默认值等于改动前的常量** —— 这条不变量保证「升级后谁的画面都没变」，
 * 偏好只对主动调过的人生效；一旦有人把公式里的常数动了，这里会立刻红。
 */
describe("graphHighlight 强度偏好", () => {
  it("默认（=1）复现改动前的观感：节点 0.35、无关连线 alpha 0.4、选中线宽 2.6", () => {
    expect(dimmedOpacity(1)).toBe(0.35); // 与旧的 opacity-35 一致
    expect(dimmedEdgeStroke(1)).toBe("rgba(148,163,184,0.4)");
    expect(dimmedEdgeWidth(1)).toBe(1.3);
    expect(relatedEdgeWidth(1)).toBe(2.6);
    // 不传参数（保持旧调用点可用）也要等于默认值
    expect(dimmedOpacity()).toBe(0.35);
    expect(neonRing("#ef4444", "select")).toBe(neonRing("#ef4444", "select", 1));
    expect(edgeGlow("#22c55e")).toBe(edgeGlow("#22c55e", 1));
  });

  it("淡化强度：0 = 完全不淡化，1 = 改动前的样子，**上限调到能完全看不见**", () => {
    expect(dimmedOpacity(0)).toBe(1);
    expect(dimmedOpacity(0.5)).toBeGreaterThan(dimmedOpacity(0.9)); // 越接近 1 越透明
    expect(dimmedOpacity(0.5)).toBeLessThan(1);
    // 上限留到200% 就是为了「调到看不见」：透明度必须在滑块顶到头时真的归零
    expect(dimmedOpacity(DIM_MAX)).toBe(0);
    // 连线跟着一起淡：不能出现「节点淡没了、线反而更显眼」
    expect(dimmedEdgeStroke(DIM_MAX)).toContain("rgba(148,163,184,0)");
    expect(dimmedEdgeStroke(1)).toBe("rgba(148,163,184,0.4)"); // 默认不变
    const seq = [0, 0.5, 1, 1.5, 2].map((d) => Number(/([0-9.]+)\)$/.exec(dimmedEdgeStroke(d))![1]));
    for (let i = 1; i < seq.length; i++) {
      if (seq[i - 1] === 0) break; // 过了零点就不再单调
      expect(seq[i]).toBeLessThanOrEqual(seq[i - 1]);
    }
  });

  it("高亮强度：描边更粗、辉光更浓，且始终合法（算不出 100% 以上的 color-mix）", () => {
    const base = neonRing("#ef4444", "select", 1);
    const louder = neonRing("#ef4444", "select", 1.5);
    expect(louder).not.toBe(base);
    expect(louder.startsWith("0 0 0 3px")).toBe(true); // 描边 2px → 3px
    const crazy = neonRing("#ef4444", "select", 99); // 超上限要被夹住
    expect(crazy).toBe(louder);
    expect(crazy).not.toMatch(/[0-9]{3}%/);
    expect(headerTint("#ef4444", "select", 1.5)).toContain("39%");
    expect(relatedEdgeWidth(1.5)).toBeGreaterThan(relatedEdgeWidth(1));
  });

  it("偏好解析：非法 / 越界输入回落到默认，不抛错也不给用户一个坏画面", () => {
    expect(readPref(null, 1)).toBe(1);
    expect(readPref("", 1)).toBe(1);
    expect(readPref("abc", 1)).toBe(1); // Number("abc") = NaN
    expect(clampPref(Number.NaN, 1)).toBe(1);
    expect(clampPref(-3, 1)).toBe(0);
    expect(clampPref(9, HL_BOOST_MAX)).toBe(HL_BOOST_MAX);
    expect(readPref("1.5", HL_BOOST_MAX)).toBe(1.5);
    expect(readPref("0.5", 1)).toBe(0.5);
  });
});
