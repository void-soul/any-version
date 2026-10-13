import { describe, expect, it } from "vitest";

import { NODE_COLORS, isPresetColor } from "../nodeColors";

/**
 * 这份色板有两个消费方：选中表的颜色点选，以及**同系列自动配色**
 * （按系列出现顺序连续取色）。
 *
 * 两个坑都只靠肉眼很难发现：
 *   · 出现重复色 → 挨着的两个系列拿到同一个颜色，「按色分系列」当场失效；
 *   · 写错格式（少一位、漏 #）→ 卡片的 `rawColor` 正则判定不过，颜色被静默忽略。
 * 所以把这两条钉住。
 */
describe("NODE_COLORS（表配色色板）", () => {
  it("全部是合法的 6 位 hex（格式错会被卡片静默忽略）", () => {
    for (const c of NODE_COLORS) {
      expect(c, `${c} 不是合法 hex`).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  it("没有重复色（重复会让两个系列同色）", () => {
    expect(new Set(NODE_COLORS).size).toBe(NODE_COLORS.length);
  });

  it("色板足够区分常见规模的系列数", () => {
    expect(NODE_COLORS.length).toBeGreaterThanOrEqual(10);
  });

  it("isPresetColor 只认色板内的颜色（自定义色要另给清除入口）", () => {
    expect(isPresetColor(NODE_COLORS[0])).toBe(true);
    expect(isPresetColor("#123456")).toBe(false);
    expect(isPresetColor("")).toBe(false);
    expect(isPresetColor(undefined)).toBe(false);
    expect(isPresetColor(null)).toBe(false);
  });
});
