import { describe, expect, it } from "vitest";

import {
  promotionCountdown,
  promotionState,
  prunePromotions,
} from "../promotions";
import type { ProviderPromotion } from "../types";

const NOW = Date.parse("2026-10-01T10:00:00Z");
const HOUR = 3_600_000;
const DAY = 86_400_000;

const promo = (endsAt: number, name = "Flash 免费"): ProviderPromotion => ({
  id: `p-${endsAt}-${name}`,
  name,
  ends_at: endsAt,
});

describe("promotionState", () => {
  it("classifies urgency by time left", () => {
    expect(promotionState(NOW + 3 * DAY, NOW)).toBe("urgent");
    expect(promotionState(NOW + 7 * DAY, NOW)).toBe("soon");
    expect(promotionState(NOW + 30 * DAY, NOW)).toBe("normal");
  });

  it("marks expired promotions as ended", () => {
    expect(promotionState(NOW, NOW)).toBe("ended");
    expect(promotionState(NOW - DAY, NOW)).toBe("ended");
  });
});

describe("prunePromotions", () => {
  it("keeps the 7-day grace period for ended ones", () => {
    const list = [
      promo(NOW + DAY, "进行中"),
      promo(NOW - DAY, "刚过期"),
      promo(NOW - 7 * DAY, "整一周"),
      promo(NOW - 8 * DAY, "超一周"),
    ];
    const kept = prunePromotions(list, NOW).map((p) => p.name);
    expect(kept).toEqual(["进行中", "刚过期", "整一周"]);
  });

  it("tolerates junk input", () => {
    expect(prunePromotions(undefined as unknown as ProviderPromotion[], NOW)).toEqual([]);
  });
});

describe("promotionCountdown", () => {
  it("formats a live one as D.HH", () => {
    expect(promotionCountdown(NOW + 12 * DAY + 5 * HOUR, NOW)).toBe("12.05");
  });

  it("gives null once ended (UI renders its own ended copy)", () => {
    expect(promotionCountdown(NOW, NOW)).toBeNull();
    expect(promotionCountdown(NOW - DAY, NOW)).toBeNull();
  });
});
