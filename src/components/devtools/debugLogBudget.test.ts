import { expect, it } from "vitest";
import { trimDebugLogs } from "./debugLogBudget";
it("accounts for SSE metadata retained alongside visible text", () => {
  const logs = [{ text: "a", time: "00", meta: { data: "large".repeat(100) } }];
  expect(trimDebugLogs(logs, 100, 200)).toEqual([]);
  expect(trimDebugLogs([{ text: "ok", time: "00" }], 100, 200)).toHaveLength(1);
});
