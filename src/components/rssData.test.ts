import { expect, it } from "vitest";
import { readRssCache, writeRssCache, articleHttpUrl } from "./rssData";
it("validates RSS cache TTL, data shape, and future timestamps", () => {
  const read = (value: unknown) => readRssCache({ getItem: () => JSON.stringify(value), setItem() {}, removeItem() {} }, "feed", 1000, 100);
  expect(read({xml:"<rss/>",fetchedAt:950})).toBe("<rss/>");
  for (const value of [null, {xml:"",fetchedAt:950}, {xml:5,fetchedAt:950}, {xml:"x",fetchedAt:1001}, {xml:"x",fetchedAt:900}]) expect(read(value)).toBeNull();
});
it("storage quota and permission failures never discard network data", () => {
  const storage = { getItem(): string { throw Error("disabled"); }, setItem() { throw Error("quota"); }, removeItem() { throw Error("disabled"); } };
  expect(readRssCache(storage, "feed", 1000, 100)).toBeNull();
  expect(() => writeRssCache(storage, "feed", "<rss/>", 1000)).not.toThrow();
  expect(() => writeRssCache(storage, "feed", "", 1000)).not.toThrow();
});
it("article links allow network pages and reject executable or local schemes", () => {
  expect(articleHttpUrl("https://example.com/news")).toBe("https://example.com/news");
  for (const value of ["javascript:alert(1)", "file:///C:/test", "data:text/html,x", "mailto:x@y.z", "/relative", "invalid"]) expect(articleHttpUrl(value)).toBeNull();
});
