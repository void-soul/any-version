import { describe, expect, it } from "vitest";
import { nextRtspPort, parseRtspInstances, serializeRtspInstances } from "./RtspServer.state";
import { DEFAULT_CONFIG, type RtspInstanceItem } from "./RtspServer.model";
const instance = (port: number): RtspInstanceItem => ({ id: "a", title: "A", config: { ...DEFAULT_CONFIG, port }, status: { id: "a", running: false, logs: [], uptimeSeconds: 0 }, showLogs: false, collapsed: false, actionLoading: false, error: null });
describe("RTSP persisted configuration", () => {
  it("rejects corrupt snapshots and preserves valid empty lists", () => {
    expect(parseRtspInstances("[]")).toEqual([]);
    expect(parseRtspInstances(serializeRtspInstances([instance(8554)]))).toHaveLength(1);
    for (const raw of ["{", "{}", "[null]", JSON.stringify([{ ...instance(8554), config: {} }]), serializeRtspInstances([instance(8554), instance(8555)])]) {
      expect(() => parseRtspInstances(raw)).toThrow();
    }
  });
  it("does not write a new snapshot when only polling or action state changes", () => {
    const a = instance(8554);
    expect(serializeRtspInstances([{ ...a, status: { ...a.status, running: true, logs: ["new"], uptimeSeconds: 50 }, showLogs: true, actionLoading: true, error: "failed" }])).toBe(serializeRtspInstances([a]));
    expect(serializeRtspInstances([{ ...a, config: { ...a.config, port: 8555 } }])).not.toBe(serializeRtspInstances([a]));
  });
  it("reuses a free port after deleting an instance instead of colliding by list length", () => {
    expect(nextRtspPort([instance(8554), instance(8556)])).toBe(8555);
    expect(nextRtspPort([])).toBe(8554);
  });
});
