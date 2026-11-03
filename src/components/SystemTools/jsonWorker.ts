import type { JsonValue } from "./JsonBrowser";
import { countJsonNodes, jsonChildEntries } from "./JsonBrowser.tree";

type ParseRequest = { id: number; type: "parse"; text: string };
type SearchRequest = { id: number; type: "search"; query: string };
type WorkerRequest = ParseRequest | SearchRequest;

type ParseResponse = {
  id: number;
  type: "parse";
  value: JsonValue | null;
  error: string | null;
  nodeCount: number;
  nodeCountCapped: boolean;
};

type SearchResponse = {
  id: number;
  type: "search";
  query: string;
  paths: string[];
  directPaths: string[];
  truncated: boolean;
};

const MAX_COUNT = 2_000_000;
const MAX_SEARCH_MATCHES = 20_000;
let parsedValue: JsonValue | null = null;
let hasParsedValue = false;

function isContainer(value: JsonValue): boolean {
  return Array.isArray(value) || (typeof value === "object" && value !== null);
}

function searchValue(value: JsonValue, query: string): { paths: string[]; directPaths: string[]; truncated: boolean } {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return { paths: [], directPaths: [], truncated: false };
  const paths: string[] = [];
  const directPaths: string[] = [];
  const matched = new Set<string>();
  type Entry = { name: string; value: JsonValue; path: string; depth: number };
  const stack: Iterator<Entry>[] = [[{ name: "root", value, path: "root", depth: 0 }][Symbol.iterator]()];
  function* children(entry: Entry): Generator<Entry> {
    for (const [name, value] of jsonChildEntries(entry.value)) yield { name, value, path: `${entry.path}.${name}`, depth: entry.depth + 1 };
  }
  let truncated = false;
  let visited = 0;
  let pathChars = 0;
  while (stack.length > 0) {
    const next = stack[stack.length - 1].next();
    if (next.done) { stack.pop(); continue; }
    if (++visited > MAX_COUNT) { truncated = true; break; }
    const current = next.value;
    if (current.depth > 256 || current.path.length > 8192) { truncated = true; continue; }
    const direct = `${current.name} ${current.path} ${isContainer(current.value) ? "" : String(current.value)}`.toLowerCase().includes(normalized);
    if (direct) {
      if (directPaths.length < MAX_SEARCH_MATCHES) directPaths.push(current.path);
      else { truncated = true; break; }
      if (paths.length < MAX_SEARCH_MATCHES) {
        let path: string | null = current.path;
        while (path) {
          if (paths.length >= MAX_SEARCH_MATCHES || pathChars + path.length > 2_000_000) { truncated = true; break; }
          if (!matched.has(path)) {
            matched.add(path);
            paths.push(path);
            pathChars += path.length;
          }
          const separator = path.lastIndexOf(".");
          path = separator > 0 ? path.slice(0, separator) : path === "root" ? null : "root";
        }
      } else {
        truncated = true;
      }
    }
    if (!isContainer(current.value)) continue;
    stack.push(children(current));
  }
  return { paths, directPaths, truncated };
}

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const request = event.data;
  if (request.type === "parse") {
    try {
      if (!request.text.trim()) {
        parsedValue = null; hasParsedValue = false;
        const response: ParseResponse = { id: request.id, type: "parse", value: null, error: null, nodeCount: 0, nodeCountCapped: false };
        self.postMessage(response);
        return;
      }
      const value = JSON.parse(request.text) as JsonValue;
      parsedValue = value; hasParsedValue = true;
      const stats = countJsonNodes(value, MAX_COUNT);
      const response: ParseResponse = { id: request.id, type: "parse", value, error: null, nodeCount: stats.count, nodeCountCapped: stats.capped };
      self.postMessage(response);
    } catch (error) {
      parsedValue = null; hasParsedValue = false;
      const response: ParseResponse = { id: request.id, type: "parse", value: null, error: error instanceof Error ? error.message : String(error), nodeCount: 0, nodeCountCapped: false };
      self.postMessage(response);
    }
    return;
  }

  const result = !hasParsedValue ? { paths: [], directPaths: [], truncated: false } : searchValue(parsedValue, request.query);
  const response: SearchResponse = { id: request.id, type: "search", query: request.query, ...result };
  self.postMessage(response);
};
