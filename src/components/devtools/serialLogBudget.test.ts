import { describe, expect, it } from "vitest";
import { trimSerialLogs } from "./serialLogBudget";
import type { LogEntry } from "./serialUtils";
const entry = (id: number, text = "log"): LogEntry => ({ id, dir: "rx", text, time: "00", bytes: 1 });
describe("serial log budgets", () => {
  it("bounds frame count and combined text/hex retention, retaining newest whole frames", () => {
    const logs = [entry(1), entry(2), entry(3)];
    expect(trimSerialLogs(logs, 2).map((entry) => entry.id)).toEqual([2, 3]);
    expect(trimSerialLogs(logs, 10, 80).map((entry) => entry.id)).toEqual([3]);
    expect(trimSerialLogs([entry(1, "large".repeat(100))], 10, 80)).toEqual([]);
    expect(logs).toHaveLength(3);
  });
});
