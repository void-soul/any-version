import { expect, it } from "vitest";
import { ClipboardImageCache } from "./clipboardImageCache";
it("keeps recent thumbnails under the count bound", () => {
  const cache = new ClipboardImageCache(2, 100);
  cache.set(1, "one"); cache.set(2, "two"); cache.get(1); cache.set(3, "three");
  expect(cache.get(2)).toBeUndefined(); expect(cache.get(1)).toBe("one"); expect(cache.size).toBe(2);
});
it("enforces a byte budget and refuses oversized thumbnails", () => {
  const cache = new ClipboardImageCache(10, 10);
  cache.set(1, "123"); cache.set(2, "456");
  expect(cache.get(1)).toBeUndefined(); expect(cache.byteSize).toBe(6);
  cache.set(2, "123456"); expect(cache.size).toBe(0); expect(cache.byteSize).toBe(0);
});
it("deletion and module cleanup release byte accounting", () => {
  const cache = new ClipboardImageCache(); cache.set(1, "abc"); cache.delete(1);
  expect(cache.byteSize).toBe(0); cache.set(2, "abc"); cache.clear();
  expect(cache.size).toBe(0); expect(cache.byteSize).toBe(0);
});

it("invalid capacities cannot create an unbounded or non-terminating cache", () => {
  expect(() => new ClipboardImageCache(0, 10)).toThrow(RangeError);
  expect(() => new ClipboardImageCache(1, -1)).toThrow(RangeError);
  expect(() => new ClipboardImageCache(1, Infinity)).toThrow(RangeError);
});
