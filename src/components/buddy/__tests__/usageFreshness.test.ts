import { describe, expect, it } from "vitest";
import { formatTime } from "../BuddyPanel.model";
import {
  usageAgeLabel,
  usageFreshness,
  usageFreshnessLabel,
  usageUpdatedAtSeconds,
} from "../usageFreshness";

const NOW_MS = 1_800_000_000_000;
const NOW_SEC = NOW_MS / 1000;

const account = (usageUpdatedAt: number | null, quotaQueryLastError: string | null = null) => ({
  usageUpdatedAt,
  quotaQueryLastError,
});

describe("usageUpdatedAtSeconds", () => {
  it("accepts seconds and normalizes legacy millis", () => {
    expect(usageUpdatedAtSeconds(NOW_SEC)).toBe(NOW_SEC);
    expect(usageUpdatedAtSeconds(NOW_MS)).toBe(NOW_SEC);
  });

  it("treats missing and non-positive values as unknown", () => {
    expect(usageUpdatedAtSeconds(null)).toBeNull();
    expect(usageUpdatedAtSeconds(undefined)).toBeNull();
    expect(usageUpdatedAtSeconds(0)).toBeNull();
    expect(usageUpdatedAtSeconds(-1)).toBeNull();
  });

  it("formatTime renders legacy millis the same as seconds", () => {
    expect(formatTime(NOW_MS)).toBe(formatTime(NOW_SEC));
    expect(formatTime(null)).toBe("—");
  });
});

describe("usageFreshness", () => {
  it("is fresh within 60 seconds of a successful query", () => {
    expect(usageFreshness(account(NOW_SEC - 30), NOW_MS).kind).toBe("fresh");
  });

  it("is cached once the stored data is older than 60 seconds", () => {
    const result = usageFreshness(account(NOW_SEC - 180), NOW_MS);
    expect(result.kind).toBe("cached");
    expect(result.ageSec).toBe(180);
  });

  it("normalizes legacy millis before judging staleness", () => {
    const result = usageFreshness(account(NOW_MS - 180_000), NOW_MS);
    expect(result.kind).toBe("cached");
    expect(result.ageSec).toBe(180);
  });

  it("reports failure even when a timestamp exists", () => {
    const result = usageFreshness(account(NOW_SEC - 10, "gateway 401"), NOW_MS);
    expect(result.kind).toBe("failed");
    expect(result.ageSec).toBe(10);
  });

  it("reports failure without any previous data", () => {
    const result = usageFreshness(account(null, "gateway 401"), NOW_MS);
    expect(result.kind).toBe("failed");
    expect(result.ageSec).toBeNull();
  });

  it("is unknown when there is neither data nor error", () => {
    expect(usageFreshness(account(null), NOW_MS)).toEqual({ kind: "unknown", ageSec: null });
  });

  it("clamps a future timestamp to zero age", () => {
    expect(usageFreshness(account(NOW_SEC + 9999), NOW_MS)).toEqual({ kind: "fresh", ageSec: 0 });
  });
});

describe("usageFreshnessLabel", () => {
  it("has no label for fresh or unknown data", () => {
    expect(usageFreshnessLabel(usageFreshness(account(NOW_SEC), NOW_MS), "zh-CN")).toBeNull();
    expect(usageFreshnessLabel(usageFreshness(account(null), NOW_MS), "zh-CN")).toBeNull();
  });

  it("uses the cached key with a localized age", () => {
    const hint = usageFreshnessLabel(usageFreshness(account(NOW_SEC - 180), NOW_MS), "zh-CN");
    expect(hint?.key).toBe("buddy.usageCached");
    expect(hint?.age).toBeTruthy();
  });

  it("escalates to the stale key after 10 minutes", () => {
    const hint = usageFreshnessLabel(usageFreshness(account(NOW_SEC - 900), NOW_MS), "zh-CN");
    expect(hint?.key).toBe("buddy.usageStale");
  });

  it("uses the failure key without an age", () => {
    const hint = usageFreshnessLabel(
      usageFreshness(account(NOW_SEC - 900, "gateway 401"), NOW_MS),
      "zh-CN",
    );
    expect(hint).toEqual({ key: "buddy.usageFailedStale" });
  });
});

describe("usageAgeLabel", () => {
  it("produces a localized relative age for each unit", () => {
    for (const age of [5, 180, 7200, 172800]) {
      const label = usageAgeLabel(age, "zh-CN");
      expect(label.length).toBeGreaterThan(0);
      expect(label).toMatch(/\d/);
    }
  });
});
