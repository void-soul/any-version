import { describe, expect, it } from "vitest";

import { formatCountdown } from "../timeLabel";

const HOUR = 3_600_000;
const DAY = 86_400_000;

describe("formatCountdown", () => {
  it("writes days and hours as D.HH", () => {
    expect(formatCountdown(55 * DAY + 21 * HOUR)).toBe("55.21");
    // 小时必须两位补零，否则 `3.5` 会被读成三点五天
    expect(formatCountdown(3 * DAY + 5 * HOUR)).toBe("3.05");
    expect(formatCountdown(3 * DAY + 21 * HOUR)).toBe("3.21");
  });

  it("keeps sub-day spans in the same shape", () => {
    expect(formatCountdown(20 * HOUR)).toBe("0.20");
    expect(formatCountdown(HOUR)).toBe("0.01");
    expect(formatCountdown(0)).toBe("0.00");
  });

  it("marks elapsed time with a minus sign", () => {
    expect(formatCountdown(-(3 * DAY + 5 * HOUR))).toBe("-3.05");
  });

  it("survives junk input (a cell must never render NaN)", () => {
    expect(formatCountdown(Number.NaN)).toBe("0.00");
    expect(formatCountdown(Number.POSITIVE_INFINITY)).toBe("0.00");
  });
});
