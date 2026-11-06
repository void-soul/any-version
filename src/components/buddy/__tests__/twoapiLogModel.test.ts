import { describe, expect, it } from "vitest";

import {
  bucketByMinute,
  groupIntoRequests,
  loadHiddenKeys,
  parseLogLine,
  parseUsageLine,
  saveHiddenKeys,
  stableRowKey,
  summarize,
  summarizeSelection,
  toggleSelectAll,
  type RequestLogEntry,
} from "../twoapiLogModel";

const entry = (ts: string, message: string): RequestLogEntry => ({ ts, source: "2api", message });

/** 2026-11-06 用户提供的真实一次请求所产生的日志（精简时间）。 */
const REAL_REQUEST = [
  entry("11:01:51.330", "→ POST /v1/chat/completions"),
  entry("11:01:51.332", "IN [openai] model=deepseek-v4.1-flash → OUT [openai]"),
  entry("11:01:52.456", "← 上游 200 (1123ms)"),
  entry("11:01:52.876", "用量: 积分 0.01 · 62→12 (共74) tok · 额度 0%"),
  entry("11:01:52.877", "← 200 /v1/chat/completions (1546ms)"),
];

describe("parseUsageLine", () => {
    it("reads credit, tokens and quota from a real usage line", () => {
        expect(parseUsageLine("用量: 积分 0.01 · 62→12 (共74) tok · 额度 0%")).toEqual({
            credit: 0.01,
            promptTokens: 62,
            completionTokens: 12,
            totalTokens: 74,
            quotaPercent: 0,
        });
    });

    it("tolerates missing segments and fractional values", () => {
        expect(parseUsageLine("用量: 积分 1.25 · 5→7 (共12) tok")).toEqual({
            credit: 1.25,
            promptTokens: 5,
            completionTokens: 7,
            totalTokens: 12,
            quotaPercent: null,
        });
        expect(parseUsageLine("用量: 3→4 (共7) tok")).toEqual({
            credit: null,
            promptTokens: 3,
            completionTokens: 4,
            totalTokens: 7,
            quotaPercent: null,
        });
    });

    it("returns null for non-usage lines and ignores non-numeric junk", () => {
        expect(parseUsageLine("→ POST /v1/chat/completions")).toBeNull();
        expect(parseUsageLine("用量: 积分 abc · x→y (共z) tok · 额度 nope%")).toEqual({
            credit: null,
            promptTokens: null,
            completionTokens: null,
            totalTokens: null,
            quotaPercent: null,
        });
    });
});

describe("parseLogLine", () => {
    it("classifies each real line", () => {
        expect(parseLogLine(REAL_REQUEST[0]).kind).toBe("request");
        expect(parseLogLine(REAL_REQUEST[0]).path).toBe("/v1/chat/completions");
        expect(parseLogLine(REAL_REQUEST[1]).model).toBe("deepseek-v4.1-flash");
        expect(parseLogLine(REAL_REQUEST[2]).kind).toBe("upstream");
        expect(parseLogLine(REAL_REQUEST[2]).status).toBe(200);
        expect(parseLogLine(REAL_REQUEST[3]).kind).toBe("usage");
        expect(parseLogLine(REAL_REQUEST[4]).kind).toBe("response");
        expect(parseLogLine(REAL_REQUEST[4]).status).toBe(200);
        expect(parseLogLine(REAL_REQUEST[4]).durationMs).toBe(1546);
        expect(parseLogLine(entry("11:02:00.000", "✗ 上游 402 积分不足")).kind).toBe("error");
        expect(parseLogLine(entry("11:02:00.000", "some future format")).kind).toBe("info");
    });
});

describe("groupIntoRequests", () => {
    it("merges one request into a single fully populated row", () => {
        const rows = groupIntoRequests(REAL_REQUEST);
        expect(rows).toHaveLength(1);
        expect(rows[0]).toMatchObject({
            ts: "11:01:51.330",
            model: "deepseek-v4.1-flash",
            path: "/v1/chat/completions",
            status: 200,
            durationMs: 1546,
            credit: 0.01,
            inTokens: 62,
            outTokens: 12,
            totalTokens: 74,
            quotaPercent: 0,
            error: null,
        });
        expect(rows[0].rawLines).toHaveLength(5);
    });

    it("keeps consecutive requests separate and records failures", () => {
        const rows = groupIntoRequests([
            ...REAL_REQUEST,
            entry("11:02:01.100", "→ POST /v1/messages"),
            entry("11:02:01.101", "IN [anthropic] model=claude-x → OUT [openai]"),
            entry("11:02:02.200", "✗ 上游 402 积分不足"),
            entry("11:02:02.201", "← 402 /v1/messages (1100ms)"),
        ]);
        expect(rows).toHaveLength(2);
        expect(rows[1].model).toBe("claude-x");
        expect(rows[1].status).toBe(402);
        expect(rows[1].error).toContain("积分不足");
    });

    it("keeps orphaned lines from a truncated buffer instead of dropping them", () => {
        const rows = groupIntoRequests([
            entry("11:01:52.876", "用量: 积分 0.02 · 1→2 (共3) tok"),
            entry("11:01:52.877", "← 200 /v1/chat/completions (1546ms)"),
            entry("11:01:53.000", "unrecognized future line"),
        ]);
        expect(rows).toHaveLength(3);
        expect(rows[0].credit).toBe(0.02);
        expect(rows[1].status).toBe(200);
        expect(rows[2].rawLines[0]).toContain("unrecognized future line");
    });

    it("handles an empty log", () => {
        expect(groupIntoRequests([])).toEqual([]);
    });
});

describe("summarize", () => {
    it("aggregates counts, success rate, credit, tokens and average duration", () => {
        const rows = groupIntoRequests([
            ...REAL_REQUEST,
            entry("11:02:01.100", "→ POST /v1/messages"),
            entry("11:02:02.200", "✗ 上游 402 积分不足"),
            entry("11:02:02.201", "← 402 /v1/messages (1000ms)"),
        ]);
        expect(summarize(rows)).toEqual({
            total: 2,
            success: 1,
            failed: 1,
            successRate: 0.5,
            totalCredit: 0.01,
            totalTokens: 74,
            avgDurationMs: 1273,
        });
    });

    it("returns zeroes for empty rows", () => {
        expect(summarize([])).toMatchObject({ total: 0, success: 0, failed: 0, successRate: 0 });
    });
});

describe("bucketByMinute", () => {
    it("groups rows by HH:MM and keeps at most 60 buckets", () => {
        const rows = groupIntoRequests([
            ...REAL_REQUEST,
            ...REAL_REQUEST.map((e) => entry("11:02:51.330", e.message)),
            ...REAL_REQUEST.map((e) => entry("11:03:51.330", e.message)),
        ]);
        const buckets = bucketByMinute(rows);
        expect(buckets).toHaveLength(3);
        expect(buckets[0]).toEqual({ label: "11:01", requests: 1, tokens: 74, credit: 0.01 });
        expect(buckets[2].label).toBe("11:03");
    });

    it("never exceeds 60 buckets even with many minutes", () => {
        const rows = Array.from({ length: 90 }, (_, i) => {
            const hour = String(10 + Math.floor(i / 60)).padStart(2, "0");
            const minute = String(i % 60).padStart(2, "0");
            return groupIntoRequests(
                REAL_REQUEST.map((e) => entry(`${hour}:${minute}:00.000`, e.message)),
            )[0];
        });
        const buckets = bucketByMinute(rows);
        expect(buckets).toHaveLength(60);
        expect(buckets[59].label).toBe("11:29");
    });
});

describe("stableRowKey", () => {
    const lines = REAL_REQUEST.map((e) => e.message);

    it("is deterministic and independent of the array index", () => {
        const a = stableRowKey("11:01:51.330", lines, 0);
        const b = stableRowKey("11:01:51.330", [...lines], 0);
        expect(a).toBe(b);
        expect(a.length).toBeGreaterThan(8);
        // 不能再用「时间戳 + 数组下标」当键：轮询后下标会漂移。
        expect(a).not.toContain("-0");
    });

    it("changes when content changes and separates same-millisecond duplicates", () => {
        const base = stableRowKey("11:01:51.330", lines, 0);
        expect(stableRowKey("11:01:51.330", lines.slice(0, 3), 0)).not.toBe(base);
        expect(stableRowKey("11:01:51.331", lines, 0)).not.toBe(base);
        expect(stableRowKey("11:01:51.330", lines, 1)).not.toBe(base);
    });

    it("gives every row a unique stable key even when the buffer shifts", () => {
        const rows = groupIntoRequests(REAL_REQUEST);
        const shifted = groupIntoRequests([
            entry("11:00:00.000", "→ POST /v1/models"),
            ...REAL_REQUEST,
        ]);
        expect(rows[0].key).toBe(stableRowKey(rows[0].ts, rows[0].rawLines, 0));
        expect(shifted[1].key).toBe(rows[0].key);
        expect(new Set(shifted.map((r) => r.key)).size).toBe(shifted.length);
    });
});

describe("summarizeSelection", () => {
    const rows = groupIntoRequests([
        ...REAL_REQUEST,
        entry("11:02:01.100", "→ POST /v1/messages"),
        entry("11:02:02.200", "用量: 积分 0.02 · 10→5 (共15) tok · 额度 0%"),
        entry("11:02:02.201", "← 200 /v1/messages (1000ms)"),
    ]);

    it("returns zeroes for an empty selection", () => {
        expect(summarizeSelection(rows, new Set())).toEqual({
            count: 0, inTokens: 0, outTokens: 0, totalTokens: 0,
            totalDurationMs: 0, avgDurationMs: 0, totalCredit: 0,
        });
    });

    it("aggregates token, duration and credit only for selected existing rows", () => {
        expect(summarizeSelection(rows, new Set([rows[0].key, rows[1].key, "gone"]))).toEqual({
            count: 2, inTokens: 72, outTokens: 17, totalTokens: 89,
            totalDurationMs: 2546, avgDurationMs: 1273, totalCredit: 0.03,
        });
    });

    it("treats missing numeric fields as zero", () => {
        const partial = groupIntoRequests([
            entry("11:03:00.000", "→ POST /v1/models"),
            entry("11:03:00.100", "← 200 /v1/models (20ms)"),
        ]);
        expect(summarizeSelection(partial, new Set([partial[0].key]))).toMatchObject({
            count: 1, totalTokens: 0, totalCredit: 0, totalDurationMs: 20, avgDurationMs: 20,
        });
    });
});

describe("toggleSelectAll", () => {
    const rows = groupIntoRequests([
        ...REAL_REQUEST,
        ...REAL_REQUEST.map((e) => entry("11:02:51.330", e.message)),
    ]);

    it("selects all shown rows when only some are selected", () => {
        const next = toggleSelectAll(rows, new Set([rows[0].key]));
        expect(next.size).toBe(2);
        expect(next.has(rows[1].key)).toBe(true);
    });

    it("deselects all shown rows when all are already selected", () => {
        const all = new Set(rows.map((r) => r.key));
        expect(toggleSelectAll(rows, all).size).toBe(0);
    });

    it("handles an empty shown list without throwing", () => {
        expect(toggleSelectAll([], new Set()).size).toBe(0);
    });
});

describe("hidden key persistence", () => {
    const memoryStorage = () => {
        const store = new Map<string, string>();
        return {
            getItem: (k: string) => store.get(k) ?? null,
            setItem: (k: string, v: string) => { store.set(k, v); },
            store,
        };
    };

    it("returns an empty set without DOM and tolerates dirty data", () => {
        expect(loadHiddenKeys(undefined).size).toBe(0);
        const dirty = memoryStorage();
        dirty.setItem("kira.2api.hiddenLogs", "not-json");
        expect(loadHiddenKeys(dirty).size).toBe(0);
        dirty.setItem("kira.2api.hiddenLogs", JSON.stringify({ nope: 1 }));
        expect(loadHiddenKeys(dirty).size).toBe(0);
    });

    it("round-trips a key set", () => {
        const s = memoryStorage();
        saveHiddenKeys(new Set(["aaa", "bbb"]), s);
        expect(loadHiddenKeys(s)).toEqual(new Set(["aaa", "bbb"]));
        saveHiddenKeys(new Set(), s);
        expect(loadHiddenKeys(s).size).toBe(0);
    });
});
