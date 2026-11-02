import { describe, expect, it } from "vitest";
import { jsonTabDirty, recordJsonSave, type JsonTab } from "./JsonBrowser.tabs";
const tab = (id: string, text: string, savedText: string | null): JsonTab => ({ id, path: null, name: id, text, savedText });
describe("JSON document snapshots", () => {
  it("treats typed untitled content as unsaved but not an untouched blank tab", () => {
    expect(jsonTabDirty(tab("a", "", null))).toBe(false);
    expect(jsonTabDirty(tab("a", "false", null))).toBe(true);
    expect(jsonTabDirty(tab("a", "0", "0"))).toBe(false);
  });
  it("records a save against its original tab without replacing newer text or another tab", () => {
    const tabs = [tab("a", "new edit", "old"), tab("b", "other", "other")];
    const saved = recordJsonSave(tabs, "a", "C:\\docs\\a.json", "captured");
    expect(saved[0]).toMatchObject({ name: "a.json", text: "new edit", savedText: "captured" });
    expect(jsonTabDirty(saved[0])).toBe(true);
    expect(saved[1]).toBe(tabs[1]);
    expect(recordJsonSave([tabs[1]], "a", "a.json", "captured")).toEqual([tabs[1]]);
  });
});
