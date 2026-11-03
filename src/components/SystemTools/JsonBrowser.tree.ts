import type { JsonValue, SearchMatches } from "./JsonBrowser";
export type FlatTreeRow = { path: string; name: string; value: JsonValue; depth: number; container: boolean; collapsed: boolean };
type Entry = { path: string; name: string; value: JsonValue; depth: number };
function* children(entry: Entry): Generator<Entry> {
  const value = entry.value;
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) yield { path: `${entry.path}.${i}`, name: String(i), value: value[i], depth: entry.depth + 1 };
  } else if (value && typeof value === "object") {
    for (const key in value) if (Object.prototype.hasOwnProperty.call(value, key)) yield { path: `${entry.path}.${key}`, name: key, value: value[key], depth: entry.depth + 1 };
  }
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
    if (++visited > maxRows) { truncated = true; break; }
    const current = next.value;
    if (search.query && !search.paths.has(current.path)) continue;
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
