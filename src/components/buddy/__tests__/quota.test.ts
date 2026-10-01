import { describe, expect, it } from "vitest";

import {
  CREDIT_HUE_SAFE,
  blockLayout,
  formatQuotaPlain,
  mergeCreditSegments,
  nearestExpiringSegment,
  segmentDaysLeft,
  segmentDayLabels,
  segmentHue,
  segmentsFromQuotaItems,
  selectRotationCandidate,
  summarizeCreditSegments,
  type CreditSegment,
  type QuotaItemLike,
  type RotationAccount,
} from "../quota";

const NOW = Date.parse("2026-09-17T10:00:00Z");
const DAY = 86_400_000;

const item = (over: Partial<QuotaItemLike> = {}): QuotaItemLike => ({
  packageName: "体验包",
  used: 0,
  total: 500,
  remain: 500,
  cycleEndTime: "2026-10-01T00:00:00Z",
  ...over,
});

const segment = (over: Partial<CreditSegment> = {}): CreditSegment => ({
  remaining: 100,
  total: 100,
  expiresAt: null,
  source: "体验包",
  packageCode: "",
  ...over,
});

describe("segmentsFromQuotaItems", () => {
  it("drops consumed items and normalizes total", () => {
    const segments = segmentsFromQuotaItems([
      item({ remain: 0, total: 100 }),
      item({ remain: 300, total: 100 }),
    ]);
    expect(segments).toHaveLength(1);
    expect(segments[0].remaining).toBe(300);
    // total 不得小于 remaining
    expect(segments[0].total).toBe(300);
  });

  it("keeps expiry timestamps (expired rows stay visible, only rotation skips them)", () => {
    const segments = segmentsFromQuotaItems([
      item({ remain: 100, cycleEndTime: new Date(NOW - 1000).toISOString() }),
      item({ remain: 100, packageName: "礼包", cycleEndTime: new Date(NOW + 1000).toISOString() }),
    ]);
    expect(segments.map((s) => s.source)).toEqual(["体验包", "礼包"]);
    expect(segments[0].expiresAt).toBe(NOW - 1000);
    expect(segments[1].expiresAt).toBe(NOW + 1000);
  });

  it("ignores unlimited rows (handled separately by the account card)", () => {
    expect(segmentsFromQuotaItems([item({ unlimited: true, remain: 0, total: 0 })])).toHaveLength(0);
  });
});

describe("segmentDayLabels", () => {
  it("shows the days left inside a wide enough block", () => {
    const labels = segmentDayLabels(
      [
        segment({ remaining: 800, expiresAt: NOW + 3 * DAY }),
        segment({ remaining: 200, expiresAt: NOW + 20 * DAY }),
      ],
      NOW
    );
    expect(labels).toEqual(["3", "20"]);
  });

  it("hides expired blocks (their credits are unusable anyway)", () => {
    const labels = segmentDayLabels(
      [
        segment({ remaining: 500, expiresAt: NOW - DAY }),
        segment({ remaining: 500, expiresAt: NOW + 5 * DAY }),
      ],
      NOW
    );
    expect(labels[0]).toBeNull();
    expect(labels[1]).toBe("5");
  });

  it("hides blocks that never expire (there is no count down)", () => {
    expect(segmentDayLabels([segment({ expiresAt: null })], NOW)).toEqual([null]);
  });

  it("keeps tiny blocks readable by widening them, not by dropping the number", () => {
    // 2% 的段也要显示天数：放不下是布局问题（条更宽 + 每块有最小宽度），
    // 不能让小段的到期信息凭空消失
    const labels = segmentDayLabels(
      [
        segment({ remaining: 20, expiresAt: NOW + 3 * DAY }),
        segment({ remaining: 980, expiresAt: NOW + 30 * DAY }),
      ],
      NOW
    );
    expect(labels).toEqual(["3", "30"]);
  });

  it("caps long countdowns at 99+ (the exact date is in the tooltip)", () => {
    expect(segmentDayLabels([segment({ expiresAt: NOW + 100 * DAY })], NOW)).toEqual(["99+"]);
    expect(segmentDayLabels([segment({ expiresAt: NOW + 1000 * DAY })], NOW)).toEqual(["99+"]);
  });
});

describe("formatQuotaPlain", () => {
  it("drops the thousands separator (it just eats width in a narrow column)", () => {
    expect(formatQuotaPlain(1_234)).toBe("1234");
    expect(formatQuotaPlain(1_234_567)).toBe("1234567");
  });

  it("rounds to whole numbers (decimals just eat width here)", () => {
    expect(formatQuotaPlain(85)).toBe("85");
    expect(formatQuotaPlain(85.3)).toBe("85");
    expect(formatQuotaPlain(85.6)).toBe("86");
  });
});

describe("blockLayout", () => {
  it("caps the per-block min at 16px when there is room", () => {
    expect(blockLayout(4, 250)).toEqual({ minWidth: 16, labelScale: 1 });
  });

  it("shrinks the min so every block fits (fixed mins clipped the last block)", () => {
    // 16 段：每段均分 ~14.7px —— 恰好都放得下，不再溢出裁掉尾部；
    // 数字基准宽 18px，块只有 14.7px → 稍微缩一点（0.82），完整显示
    const layout = blockLayout(16, 250);
    expect(layout.minWidth).toBeCloseTo((250 - 15) / 16, 5);
    expect(layout.labelScale).toBeCloseTo(((250 - 15) / 16) / 18, 5);
  });

  it("scales the number down instead of hiding it on very narrow blocks", () => {
    // 40 段：每段只有 ~5px —— 数字跟着缩（用户要求：再小也要显示出来）
    const tiny = blockLayout(40, 250);
    expect(tiny.minWidth).toBeCloseTo((250 - 39) / 40, 5);
    expect(tiny.labelScale).toBeCloseTo(((250 - 39) / 40) / 18, 5);
    expect(tiny.labelScale).toBeLessThan(1);
    expect(tiny.labelScale).toBeGreaterThan(0);
  });

  it("never lets the blocks overflow the bar", () => {
    // minWidth 之和 + 间隙不得超过条宽（否则尾部的块被裁）
    for (const count of [1, 3, 8, 16, 25, 40, 64]) {
      const { minWidth } = blockLayout(count, 250);
      expect(minWidth * count + (count - 1)).toBeLessThanOrEqual(250 + 1e-9);
    }
  });

  it("handles degenerate input", () => {
    expect(blockLayout(0, 250).labelScale).toBe(0);
    expect(blockLayout(5, 0).labelScale).toBe(0);
  });
});

describe("mergeCreditSegments", () => {
  it("merges records of the same grant package into one row", () => {
    // 官方把「一次赠送 5000」拆成 10 条 500 的记录
    const rows = Array.from({ length: 10 }, () =>
      segment({ remaining: 500, total: 500, expiresAt: NOW + 86400000, packageCode: "p_tcaca" })
    );
    const merged = mergeCreditSegments(rows);
    expect(merged).toHaveLength(1);
    expect(merged[0].remaining).toBe(5000);
    expect(merged[0].total).toBe(5000);
  });

  it("keeps different expiry times apart", () => {
    const merged = mergeCreditSegments([
      segment({ remaining: 100, total: 100, expiresAt: NOW + 1000, packageCode: "p" }),
      segment({ remaining: 200, total: 200, expiresAt: NOW + 2000, packageCode: "p" }),
    ]);
    expect(merged).toHaveLength(2);
    // 先到期排前面
    expect(merged[0].remaining).toBe(100);
  });
});

describe("summarizeCreditSegments", () => {
  it("sums after merging and reports used", () => {
    const summary = summarizeCreditSegments(
      mergeCreditSegments([
        segment({ remaining: 500, total: 1000, expiresAt: NOW + 1000, packageCode: "p" }),
        segment({ remaining: 500, total: 1000, expiresAt: NOW + 1000, packageCode: "p" }),
      ])
    );
    expect(summary.remain).toBe(1000);
    expect(summary.total).toBe(2000);
    expect(summary.used).toBe(1000);
    expect(summary.hasData).toBe(true);
  });
});

describe("nearestExpiringSegment", () => {
  it("ignores expired segments and sorts never-expiring last", () => {
    const nearest = nearestExpiringSegment(
      [
        segment({ remaining: 10, expiresAt: NOW - 1 }),
        segment({ remaining: 20, expiresAt: null }),
        segment({ remaining: 30, expiresAt: NOW + 5000 }),
      ],
      NOW
    );
    expect(nearest?.remaining).toBe(30);
  });
});

describe("selectRotationCandidate", () => {
  const account = (
    accountId: string,
    uid: string | null,
    segments: CreditSegment[]
  ): RotationAccount => ({ accountId, uid, label: `${accountId}@example.com`, segments });

  it("skips the current account and accounts without credits", () => {
    const candidate = selectRotationCandidate(
      [
        account("current", "u1", [segment({ remaining: 100, expiresAt: NOW + 1000 })]),
        account("empty", "u2", [segment({ remaining: 0, expiresAt: NOW + 1000 })]),
        account("ok", "u3", [segment({ remaining: 50, expiresAt: NOW + 9000 })]),
      ],
      "u1",
      NOW
    );
    expect(candidate?.accountId).toBe("ok");
    expect(candidate?.remaining).toBe(50);
  });

  it("prefers the earliest expiry, then the larger remaining", () => {
    const candidate = selectRotationCandidate(
      [
        account("later", "u2", [segment({ remaining: 9999, expiresAt: NOW + 9000 })]),
        account("soon-small", "u3", [segment({ remaining: 10, expiresAt: NOW + 1000 })]),
        account("soon-big", "u4", [segment({ remaining: 88, expiresAt: NOW + 1000 })]),
      ],
      "u1",
      NOW
    );
    expect(candidate?.accountId).toBe("soon-big");
  });

  it("returns null when nobody has credits", () => {
    expect(selectRotationCandidate([account("a", "u2", [])], "u1", NOW)).toBeNull();
  });

  it("falls back to uid matching when the current account has no uid", () => {
    // 当前账号 uid 未知时，不应把任意账号当成「当前账号」而排除
    const candidate = selectRotationCandidate(
      [account("only", "u2", [segment({ remaining: 5, expiresAt: NOW + 1000 })])],
      null,
      NOW
    );
    expect(candidate?.accountId).toBe("only");
  });
});

describe("segmentDaysLeft", () => {
  it("counts whole days, clamps sub-day leftovers to 0", () => {
    expect(segmentDaysLeft(segment({ expiresAt: NOW + 3 * DAY }), NOW)).toBe(3);
    // 剩不到一天：既不能显示 0 天以外的数，也不能四舍五入成 1 天
    expect(segmentDaysLeft(segment({ expiresAt: NOW + 1000 }), NOW)).toBe(0);
    expect(segmentDaysLeft(segment({ expiresAt: NOW - 1000 }), NOW)).toBe(0);
  });

  it("reports null for credits that never expire", () => {
    expect(segmentDaysLeft(segment({ expiresAt: null }), NOW)).toBeNull();
  });
});

describe("segmentHue", () => {
  it("is redder the sooner it expires", () => {
    const soon = segmentHue(segment({ expiresAt: NOW + DAY }), NOW);
    const later = segmentHue(segment({ expiresAt: NOW + 20 * DAY }), NOW);
    expect(soon).not.toBeNull();
    expect(later).not.toBeNull();
    expect(soon!).toBeLessThan(later!);
  });

  it("caps at the safe hue for far-off or never-expiring credits", () => {
    expect(segmentHue(segment({ expiresAt: null }), NOW)).toBe(CREDIT_HUE_SAFE);
    expect(segmentHue(segment({ expiresAt: NOW + 999 * DAY }), NOW)).toBe(CREDIT_HUE_SAFE);
  });

  it("returns null when already expired (caller paints it grey, not red)", () => {
    // 过期的积分用不掉了，按「紧急程度」染红会让人以为还能抢救
    expect(segmentHue(segment({ expiresAt: NOW - 1 }), NOW)).toBeNull();
  });
});
