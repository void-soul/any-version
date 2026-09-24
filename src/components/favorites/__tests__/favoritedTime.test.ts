import { describe, expect, it } from "vitest";
import { favoritedDateLabel, sinceToLocalString } from "../favoritedTime";

describe("sinceToLocalString", () => {
  const now = new Date(2026, 8, 24, 12, 30, 45); // 本地时间 2026-09-24 12:30:45

  it("all 不过滤", () => {
    expect(sinceToLocalString("all", now)).toBeNull();
  });

  it("按天数回推并补零成后端存储格式", () => {
    expect(sinceToLocalString("7d", now)).toBe("2026-09-17T12:30:45");
    expect(sinceToLocalString("30d", now)).toBe("2026-08-25T12:30:45");
    expect(sinceToLocalString("365d", now)).toBe("2025-09-24T12:30:45");
  });

  it("跨月跨年时日期被正确归一（由 Date 处理，不手写算术）", () => {
    const endOfYear = new Date(2026, 0, 2, 8, 0, 0); // 2026-01-02
    expect(sinceToLocalString("7d", endOfYear)).toBe("2025-12-26T08:00:00");
  });

  it("默认取当前时间且始终是 19 位的时间串", () => {
    const value = sinceToLocalString("7d");
    expect(value).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/);
  });
});

describe("favoritedDateLabel", () => {
  it("取出日期部分", () => {
    expect(favoritedDateLabel("2021-03-04T05:06:07")).toBe("2021-03-04");
  });

  it("空值返回 null（由调用方回退到入库时间）", () => {
    expect(favoritedDateLabel(null)).toBeNull();
    expect(favoritedDateLabel(undefined)).toBeNull();
    expect(favoritedDateLabel("")).toBeNull();
  });

  it("格式异常时返回 null 而不是显示半截乱码", () => {
    expect(favoritedDateLabel("12/34/5678")).toBeNull();
    expect(favoritedDateLabel("not-a-date")).toBeNull();
  });
});
