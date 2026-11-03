import { expect, it } from "vitest";
import { countJsonNodes, flattenTreeRows } from "./JsonBrowser.tree";
import type { JsonValue } from "./JsonBrowser";
const search = { query: "", paths: new Set<string>(), directPaths: new Set<string>() };
const flatten = (value: JsonValue, limit = 10, depth = 256) => flattenTreeRows(value, search, new Set(), false, new Set(), limit, depth);
it("counts a bounded prefix without materializing wide sibling lists", () => {
  let reads = 0;
  const wide = new Proxy(Array(100_000).fill(1), { get(target, property, receiver) { if (/^\d+$/.test(String(property))) reads++; return Reflect.get(target, property, receiver); } });
  expect(countJsonNodes(wide, 10)).toEqual({ count: 10, capped: true });
  expect(reads).toBeLessThan(12);
  expect(countJsonNodes([1, 2], 3)).toEqual({ count: 3, capped: false });
});
it("renders the available prefix before marking wide trees truncated", () => {
  const result = flatten(Array.from({ length: 100_000 }, (_, i) => i));
  expect(result.rows.map(row => row.path)).toEqual(["root", ...Array.from({ length: 9 }, (_, i) => `root.${i}`)]);
  expect(result.truncated).toBe(true);
  expect(flatten([1, 2], 3).truncated).toBe(false);
});
it("bounds deep paths while continuing pending sibling traversal", () => {
  const result = flatten({ a: { b: { c: 1 } }, z: 2 }, 20, 2);
  expect(result.rows.map(row => row.path)).toEqual(["root", "root.a", "root.a.b", "root.z"]);
  expect(result.truncated).toBe(true);
});
it("retains collapsed and searched tree behavior", () => {
  expect(flattenTreeRows({ a: [1] }, search, new Set(["root.a"]), false, new Set()).rows).toHaveLength(2);
  const matched = { ...search, query: "a", paths: new Set(["root", "root.a", "root.a.0"]) };
  expect(flattenTreeRows({ a: [1], b: 2 }, matched, new Set(["root.a"]), true, new Set()).rows.map(row => row.path)).toEqual(["root", "root.a", "root.a.0"]);
});
