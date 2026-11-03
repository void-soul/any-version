import { afterEach, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn(), isTauri: vi.fn(() => false) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, isTauri: mocks.isTauri }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.open }));
import { pickLogSource, sameLogBytes } from "./source";
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks(); mocks.isTauri.mockReturnValue(false); });
it("reads a fresh handle snapshot for append, truncate and same-size rewrite", async () => {
  let content = "first";
  const getFile = vi.fn(async () => new File([content], "test.log"));
  vi.stubGlobal("window", { showOpenFilePicker: async () => [{ getFile }] });
  const source = await pickLogSource();
  let previous: Uint8Array | null = null;
  for (const next of ["first", "first\nappend", "short", "other"]) {
    content = next;
    const bytes = new Uint8Array(await (await source!.getFile()).arrayBuffer());
    expect(sameLogBytes(previous, bytes)).toBe(false);
    expect(sameLogBytes(bytes, bytes.slice())).toBe(true);
    previous = bytes;
  }
  getFile.mockRejectedValueOnce(new Error("temporarily missing"));
  await expect(source!.getFile()).rejects.toThrow("temporarily missing");
  expect(await (await source!.getFile()).text()).toBe("other");
});
it("uses binary IPC for each native refresh and preserves picker cancellation", async () => {
  mocks.isTauri.mockReturnValue(true);
  mocks.open.mockResolvedValue(null);
  expect(await pickLogSource()).toBeNull();
  mocks.open.mockResolvedValue("C:/fixture/test.log");
  mocks.invoke.mockResolvedValue(new TextEncoder().encode("hello").buffer);
  const source = await pickLogSource();
  expect(await (await source!.getFile()).text()).toBe("hello");
  await source!.getFile();
  expect(mocks.invoke).toHaveBeenCalledTimes(2);
  expect(mocks.invoke).toHaveBeenLastCalledWith("read_log_file", { path: "C:/fixture/test.log" });
});
it("reports that legacy file inputs provide no refreshable source", async () => {
  vi.stubGlobal("window", {});
  expect(await pickLogSource()).toBeUndefined();
});
