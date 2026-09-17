import { describe, expect, it } from "vitest";

import {
  mergeCreditSegments,
  nearestExpiringSegment,
  segmentsFromQuotaItems,
  selectRotationCandidate,
  summarizeCreditSegments,
  type CreditSegment,
  type QuotaItemLike,
  type RotationAccount,
} from "../quota";

const NOW = Date.parse("2026-09-17T10:00:00Z");

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
