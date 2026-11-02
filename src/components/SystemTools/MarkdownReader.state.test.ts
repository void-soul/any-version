import { describe, expect, it } from "vitest";
import { recordMarkdownSave, trimMarkdownHistory } from "./MarkdownReader.state";
import type { Tab } from "./MarkdownReader.model";
describe("Markdown editing ownership", () => {
  it("keeps newer edits dirty after an earlier snapshot finishes saving", () => {
    const tab = { path: "a.md", content: "new", savedContent: "old", dirty: true } as Tab;
    expect(recordMarkdownSave([tab], "a.md", "captured")[0]).toMatchObject({ content: "new", savedContent: "captured", dirty: true });
    expect(recordMarkdownSave([tab], "a.md", "new")[0].dirty).toBe(false);
  });
  it("bounds whole-document undo snapshots by bytes and count without truncating text", () => {
    const history = ["old", "middle", "new"];
    expect(trimMarkdownHistory(history, 18)).toEqual(["middle", "new"]);
    expect(trimMarkdownHistory(history, 100, 1)).toEqual(["new"]);
    expect(trimMarkdownHistory(["oversized"], 2)).toEqual([]);
    expect(history).toEqual(["old", "middle", "new"]);
  });
});
