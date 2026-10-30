import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../../contracts/appSettings";

const invokeMock = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<AppSettings>>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invokeMock(cmd, args),
}));

describe("application settings patches", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.resetModules();
  });

  it("sends only fields owned by an action and preserves request order", async () => {
    const { patchAppSettings } = await import("../appSettings");
    let finishFirst!: (value: AppSettings) => void;
    invokeMock.mockImplementationOnce(() => new Promise((resolve) => { finishFirst = resolve; }));
    invokeMock.mockResolvedValueOnce({ musicNextHotkey: "F8", mindmapQuickHotkey: "F9" });
    const first = patchAppSettings({ musicNextHotkey: "F8" });
    const second = patchAppSettings({ mindmapQuickHotkey: "F9" });
    await Promise.resolve();
    expect(invokeMock.mock.calls).toEqual([["launcher_patch_settings", { patch: { musicNextHotkey: "F8" } }]]);
    finishFirst({ musicNextHotkey: "F8" });
    await first;
    expect(await second).toEqual({ musicNextHotkey: "F8", mindmapQuickHotkey: "F9" });
    expect(invokeMock.mock.calls[1]).toEqual(["launcher_patch_settings", { patch: { mindmapQuickHotkey: "F9" } }]);
  });

  it("does not let a failed save block later patches or turn deletion into a whole-map replacement", async () => {
    const { loadAppSettings, patchAppSettings } = await import("../appSettings");
    invokeMock.mockRejectedValueOnce(new Error("database read-only"));
    await expect(patchAppSettings({ musicPrevHotkey: "F7" })).rejects.toThrow("database read-only");
    invokeMock.mockResolvedValueOnce({ moduleHotkeys: { tasks: "F5" } });
    await patchAppSettings({ moduleHotkeys: { music: null } });
    expect(invokeMock.mock.calls[1]).toEqual(["launcher_patch_settings", { patch: { moduleHotkeys: { music: null } } }]);
    invokeMock.mockResolvedValueOnce({ moduleHotkeys: { tasks: "F5" } });
    expect(await loadAppSettings()).toEqual({ moduleHotkeys: { tasks: "F5" } });
  });
});
