import { describe, expect, it, vi } from "vitest";
import { createModuleLayoutSaver, reconcileModulePages } from "../moduleLifecycle";

describe("module lifecycle", () => {
  it("removes disabled cached pages and falls back to settings when launcher is also disabled", () => {
    const next = reconcileModulePages("mindmap", new Set(["launcher", "mindmap", "settings"]), ["settings"]);
    expect(next.active).toBe("settings");
    expect([...next.mounted]).toEqual(["settings"]);
    const enabledAgain = reconcileModulePages("settings", next.mounted, ["launcher", "mindmap", "settings"]);
    expect(enabledAgain.mounted.has("mindmap")).toBe(false);
  });
  it("keeps enabled hidden pages mounted during ordinary navigation", () => {
    const next = reconcileModulePages("launcher", new Set(["launcher", "mindmap"]), ["launcher", "mindmap", "settings"]);
    expect([...next.mounted]).toEqual(["launcher", "mindmap"]);
  });
  it("rejects concurrent writes and applies only persisted state", async () => {
    let complete!: () => void;
    const apply = vi.fn();
    const save = vi.fn(() => new Promise<void>((resolve) => { complete = resolve; }));
    const saver = createModuleLayoutSaver({ save, read: async () => ({ disabledModules: ["mindmap"] }), apply, notify: async () => {}, report: vi.fn() });
    const first = saver([], ["mindmap"]);
    expect(apply).not.toHaveBeenCalled();
    expect(await saver([], [])).toBe(false);
    expect(save).toHaveBeenCalledTimes(1);
    complete(); expect(await first).toBe(true);
    expect(apply).toHaveBeenCalledWith({ disabledModules: ["mindmap"] });
  });
  it("reconciles the actual configuration and reports failure; subsequent saves remain usable", async () => {
    const save = vi.fn().mockRejectedValueOnce(new Error("backend rollback")).mockResolvedValue(undefined);
    const apply = vi.fn(), report = vi.fn(), notify = vi.fn(async () => {});
    const saver = createModuleLayoutSaver({ save, read: async () => ({ disabledModules: [] }), apply, report, notify });
    expect(await saver([], ["mindmap"])).toBe(false);
    expect(apply).toHaveBeenCalledWith({ disabledModules: [] });
    expect(report).toHaveBeenCalledTimes(1);
    expect(notify).toHaveBeenCalledTimes(1);
    expect(await saver([], [])).toBe(true);
  });
});
