import type { JsonValue, SearchMatches, FlatTreeRow } from "../../contracts/jsonBrowser";
export type { FlatTreeRow } from "../../contracts/jsonBrowser";
type Entry = { path: string; name: string; value: JsonValue; depth: number };
export function* jsonChildEntries(value: JsonValue): Generator<[string, JsonValue]> {
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) yield [String(i), value[i]];
  } else if (value && typeof value === "object") {
    for (const key in value) if (Object.prototype.hasOwnProperty.call(value, key)) yield [key, value[key]];
  }
}
export function countJsonNodes(value: JsonValue, limit = 2_000_000): { count: number; capped: boolean } {
  const stack: Iterator<[string, JsonValue]>[] = [[['root', value] as [string, JsonValue]][Symbol.iterator]()];
  let count = 0;
  while (stack.length) {
    const next = stack[stack.length - 1].next();
    if (next.done) { stack.pop(); continue; }
    if (count >= limit) return { count, capped: true };
    count++;
    if (next.value[1] !== null && typeof next.value[1] === "object") stack.push(jsonChildEntries(next.value[1]));
  }
  return { count, capped: false };
}
function* children(entry: Entry): Generator<Entry> {
  for (const [name, value] of jsonChildEntries(entry.value)) yield { path: `${entry.path}.${name}`, name, value, depth: entry.depth + 1 };
}

/** Lazy DFS keeps pending siblings out of memory; cap depth to bound path growth. */
export function flattenTreeRows(value: JsonValue, search: SearchMatches, collapsed: Set<string>, large: boolean, expanded: Set<string>, maxRows = 200_000, maxDepth = 256): { rows: FlatTreeRow[]; truncated: boolean } {
  const rows: FlatTreeRow[] = [];
  const stack: Iterator<Entry>[] = [[{ path: "root", name: "root", value, depth: 0 }][Symbol.iterator]()];
  let truncated = false;
  let visited = 0;
  while (stack.length) {
    const next = stack[stack.length - 1].next();
    if (next.done) { stack.pop(); continue; }
    if (++visited > (search.query ? 2_000_000 : maxRows)) { truncated = true; break; }
    const current = next.value;
    if (current.path.length > 8192) { truncated = true; continue; }
    if (search.query && !search.paths.has(current.path)) continue;
    if (rows.length >= maxRows) { truncated = true; break; }
    const container = current.value !== null && typeof current.value === "object";
    const collapsedHere = container && !search.query && (collapsed.has(current.path) || (large && current.depth > 0 && !expanded.has(current.path)));
    rows.push({ ...current, container, collapsed: collapsedHere });
    if (!container || collapsedHere) continue;
    const iterator = children(current);
    if (current.depth >= maxDepth) { if (!iterator.next().done) truncated = true; continue; }
    stack.push(iterator);
  }
  return { rows, truncated };
}
