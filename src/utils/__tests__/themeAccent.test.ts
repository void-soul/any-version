import { describe, it, expect } from "vitest";
import {
  normalizeThemeAccent,
  themeAccentVars,
  VEX_CYBER_ACCENT,
  VEX_CYBER_CYAN,
} from "../brand";

describe("normalizeThemeAccent", () => {
  it("接受 6 位 hex 并去掉首尾空白", () => {
    expect(normalizeThemeAccent("#ff2d95")).toBe("#ff2d95");
    expect(normalizeThemeAccent("  #FF2D95 ")).toBe("#FF2D95");
  });

  it("拒绝非法色值", () => {
    // 只认 6 位 hex：3 位简写/颜色名/缺 # 一律判非法，避免坏值污染 CSS 变量
    expect(normalizeThemeAccent("#fff")).toBeNull();
    expect(normalizeThemeAccent("red")).toBeNull();
    expect(normalizeThemeAccent("ff2d95")).toBeNull();
    expect(normalizeThemeAccent("#ff2d9")).toBeNull();
    expect(normalizeThemeAccent("#ff2d955")).toBeNull();
    expect(normalizeThemeAccent("url(evil)")).toBeNull();
  });

  it("空值返回 null", () => {
    expect(normalizeThemeAccent(null)).toBeNull();
    expect(normalizeThemeAccent(undefined)).toBeNull();
    expect(normalizeThemeAccent("")).toBeNull();
    expect(normalizeThemeAccent("   ")).toBeNull();
  });
});

describe("themeAccentVars", () => {
  it("主色变量指向 accent，青色仍是品牌辅助色", () => {
    const vars = themeAccentVars(VEX_CYBER_ACCENT);
    expect(vars["--module-accent"]).toBe(VEX_CYBER_ACCENT);
    expect(vars["--neon"]).toBe(VEX_CYBER_ACCENT);
    expect(vars["--cyan"]).toBe(VEX_CYBER_CYAN);
  });

  it("派生色基于 accent 计算，且数量固定", () => {
    const vars = themeAccentVars("#123456");
    expect(vars["--module-accent-soft"]).toContain("#123456");
    expect(vars["--module-accent-ring"]).toContain("#123456");
    expect(vars["--module-accent-strong"]).toContain("#123456");
    // 与 App 首帧预置共用同一份定义，键数量漂移会让两处渲染不一致
    expect(Object.keys(vars).sort()).toEqual([
      "--cyan",
      "--module-accent",
      "--module-accent-ring",
      "--module-accent-soft",
      "--module-accent-strong",
      "--neon",
    ]);
  });
});
