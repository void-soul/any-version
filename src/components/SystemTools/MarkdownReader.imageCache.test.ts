import { describe, expect, it } from "vitest";
import { MarkdownImageCache } from "./MarkdownReader.imageCache";
describe("Markdown image retention", () => {
  it("evicts least recently used pending entries", () => {
    const cache = new MarkdownImageCache(2, 100);
    const pending = new Promise<string | null>(() => {});
    cache.set("a", pending); cache.set("b", pending); cache.get("a"); cache.set("c", pending);
    expect(cache.get("b")).toBeUndefined(); expect(cache.get("a")).toBe(pending);
  });
  it("drops oversized results and failed reads so they can be retried", async () => {
    const cache = new MarkdownImageCache(2, 10);
    cache.set("a", Promise.resolve("oversized"));
    cache.set("b", Promise.reject(new Error("failed")));
    await Promise.resolve();
    expect(cache.get("a")).toBeUndefined(); expect(cache.get("b")).toBeUndefined();
  });
  it("does not reinsert a late result after module cleanup", async () => {
    const cache = new MarkdownImageCache();
    let resolve!: (value: string) => void;
    cache.set("a", new Promise((done) => { resolve = done; })); cache.clear(); resolve("late");
    await Promise.resolve(); expect(cache.get("a")).toBeUndefined();
  });
});
