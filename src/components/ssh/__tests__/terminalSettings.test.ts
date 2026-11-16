import { describe, expect, it } from "vitest";
import { DEFAULT_PREFS, FONT_FAMILIES, normalizePrefs, xtermTheme } from "../terminalSettings";

describe("终端外观设置", () => {
  it("空/脏数据全部回落到默认值", () => {
    expect(normalizePrefs(null)).toEqual(DEFAULT_PREFS);
    expect(normalizePrefs(undefined)).toEqual(DEFAULT_PREFS);
    expect(normalizePrefs("garbage")).toEqual(DEFAULT_PREFS);
    expect(normalizePrefs({})).toEqual(DEFAULT_PREFS);
  });

  it("字号被夹在 8–32，非数字回落默认", () => {
    expect(normalizePrefs({ fontSize: 16 }).fontSize).toBe(16);
    expect(normalizePrefs({ fontSize: 4 }).fontSize).toBe(8);
    expect(normalizePrefs({ fontSize: 999 }).fontSize).toBe(32);
    expect(normalizePrefs({ fontSize: Number.NaN }).fontSize).toBe(DEFAULT_PREFS.fontSize);
    expect(normalizePrefs({ fontSize: "18" }).fontSize).toBe(DEFAULT_PREFS.fontSize);
  });

  it("字体只接受预置项，未知值回落默认", () => {
    const known = FONT_FAMILIES[1].value;
    expect(normalizePrefs({ fontFamily: known }).fontFamily).toBe(known);
    expect(normalizePrefs({ fontFamily: "Comic Sans" }).fontFamily).toBe(DEFAULT_PREFS.fontFamily);
  });

  it("主题只接受 dark/light", () => {
    expect(normalizePrefs({ theme: "light" }).theme).toBe("light");
    expect(normalizePrefs({ theme: "solarized" }).theme).toBe(DEFAULT_PREFS.theme);
  });

  it("两套主题都给出完整可用的前景/背景色", () => {
    for (const theme of ["dark", "light"] as const) {
      const t = xtermTheme(theme);
      expect(t.background).toMatch(/^#[0-9a-f]{6}$/i);
      expect(t.foreground).toMatch(/^#[0-9a-f]{6}$/i);
      expect(t.cursor).toMatch(/^#[0-9a-f]{6}$/i);
    }
    expect(xtermTheme("dark").background).not.toBe(xtermTheme("light").background);
  });
});
