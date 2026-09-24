import { describe, expect, it } from "vitest";
import { isOverdue, monthGrid, monthRange, shiftMonth, weekRange, weekStartOf } from "../calendarRange";

describe("weekStartOf / weekRange", () => {
  it("周四归到本周一", () => {
    // 2026-09-24 是周四
    expect(weekStartOf("2026-09-24")).toBe("2026-09-21");
  });

  it("周一归到自己", () => {
    expect(weekStartOf("2026-09-21")).toBe("2026-09-21");
  });

  it("周日归到上一个周一（不是下一个）", () => {
    expect(weekStartOf("2026-09-27")).toBe("2026-09-21");
  });

  it("周区间是周一到周日，跨月也正确", () => {
    expect(weekRange("2026-09-24")).toEqual({ start: "2026-09-21", end: "2026-09-27" });
    // 2026-10-01 是周四 → 本周一是 09-28
    expect(weekRange("2026-10-01")).toEqual({ start: "2026-09-28", end: "2026-10-04" });
  });
});

describe("monthRange", () => {
  it("返回当月 1 号到月末", () => {
    expect(monthRange("2026-09-24")).toEqual({ start: "2026-09-01", end: "2026-09-30" });
  });

  it("二月按闰年规则取月末", () => {
    expect(monthRange("2026-02-10")).toEqual({ start: "2026-02-01", end: "2026-02-28" });
    expect(monthRange("2028-02-10")).toEqual({ start: "2028-02-01", end: "2028-02-29" });
  });
});

describe("monthGrid", () => {
  it("固定 42 格，且从周一开始连续", () => {
    // 2026-09-01 是周二 → 网格从「包含 09-01 的那一周的周一」= 08-31 起算，
    // 而不是从当前日期所在周的周一起算（当前日期只在跨月定位时作为锚点）
    const grid = monthGrid("2026-09-24");
    expect(grid).toHaveLength(42);
    expect(grid[0]).toBe("2026-08-31");
    expect(grid[1]).toBe("2026-09-01");
    expect(grid[41]).toBe("2026-10-11");
  });

  it("当月的每一天都在网格里（不会漏掉月末）", () => {
    const grid = new Set(monthGrid("2026-11-15"));
    // 2026-11 有 30 天
    for (let d = 1; d <= 30; d += 1) {
      expect(grid.has(`2026-11-${String(d).padStart(2, "0")}`)).toBe(true);
    }
  });

  it("月初落在周一时网格正好从 1 号开始", () => {
    // 2026-06-01 是周一
    const grid = monthGrid("2026-06-15");
    expect(grid[0]).toBe("2026-06-01");
  });
});

describe("shiftMonth", () => {
  it("向前后平移，落到月初", () => {
    expect(shiftMonth("2026-09-24", 1)).toBe("2026-10-01");
    expect(shiftMonth("2026-09-24", -1)).toBe("2026-08-01");
  });

  it("跨年正确", () => {
    expect(shiftMonth("2026-01-15", -1)).toBe("2025-12-01");
    expect(shiftMonth("2026-12-15", 1)).toBe("2027-01-01");
  });
});

describe("isOverdue", () => {
  const today = "2026-09-24";

  it("过去且未完成 = 逾期", () => {
    expect(isOverdue({ progress: 0, scheduledDate: "2026-09-20" }, today)).toBe(true);
    expect(isOverdue({ progress: 60, scheduledDate: "2026-09-20" }, today)).toBe(true);
  });

  it("已完成不算逾期（哪怕是过去）", () => {
    expect(isOverdue({ progress: 100, scheduledDate: "2026-09-20" }, today)).toBe(false);
  });

  it("今天与未来都不算逾期", () => {
    expect(isOverdue({ progress: 0, scheduledDate: today }, today)).toBe(false);
    expect(isOverdue({ progress: 0, scheduledDate: "2026-09-25" }, today)).toBe(false);
  });

  it("未排期（收集箱）不算逾期：它没有承诺过日期", () => {
    expect(isOverdue({ progress: 0, scheduledDate: null }, today)).toBe(false);
  });
});
