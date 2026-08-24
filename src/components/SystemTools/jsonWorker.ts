import type { JsonValue } from "./JsonBrowser";

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

function isContainer(value: JsonValue): boolean {
  return Array.isArray(value) || (typeof value === "object" && value !== null);
}

function childEntries(value: JsonValue, limit = Number.MAX_SAFE_INTEGER): Array<[string, JsonValue]> {
  const result: Array<[string, JsonValue]> = [];
  if (Array.isArray(value)) {
    const end = Math.min(value.length, limit);
    for (let index = 0; index < end; index += 1) result.push([String(index), value[index]]);
    return result;
  }
  if (typeof value === "object" && value !== null) {
    for (const key in value) {
      if (!Object.prototype.hasOwnProperty.call(value, key)) continue;
      result.push([key, value[key]]);
      if (result.length >= limit) break;
    }
  }
  return result;
}

function countNodes(value: JsonValue): { count: number; capped: boolean } {
  let count = 0;
  const stack: JsonValue[] = [value];
  while (stack.length > 0) {
    const current = stack.pop() as JsonValue;
    count += 1;
    if (count >= MAX_COUNT) return { count: MAX_COUNT, capped: true };
    if (isContainer(current)) {
      const children = childEntries(current);
      for (let index = children.length - 1; index >= 0; index -= 1) stack.push(children[index][1]);
    }
  }
  return { count, capped: false };
}

function searchValue(value: JsonValue, query: string): { paths: string[]; directPaths: string[]; truncated: boolean } {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return { paths: [], directPaths: [], truncated: false };
  const paths: string[] = [];
  const directPaths: string[] = [];
  const matched = new Set<string>();
  const stack: Array<{ name: string; value: JsonValue; path: string; parentPath: string | null }> = [{ name: "root", value, path: "root", parentPath: null }];
  let truncated = false;

  while (stack.length > 0) {
    const current = stack.pop() as (typeof stack)[number];
    const direct = `${current.name} ${current.path} ${isContainer(current.value) ? "" : String(current.value)}`.toLowerCase().includes(normalized);
    if (direct) {
      if (directPaths.length < MAX_SEARCH_MATCHES) directPaths.push(current.path);
      else truncated = true;
      if (paths.length < MAX_SEARCH_MATCHES) {
        let path: string | null = current.path;
        while (path) {
          if (!matched.has(path)) {
            matched.add(path);
            paths.push(path);
          }
          const separator = path.lastIndexOf(".");
          path = separator > 0 ? path.slice(0, separator) : path === "root" ? null : "root";
        }
      } else {
        truncated = true;
      }
    }
    if (!isContainer(current.value)) continue;
    const children = childEntries(current.value);
    for (let index = children.length - 1; index >= 0; index -= 1) {
      const [name, child] = children[index];
      stack.push({ name, value: child, path: `${current.path}.${name}`, parentPath: current.path });
    }
  }
  return { paths, directPaths, truncated };
}

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const request = event.data;
  if (request.type === "parse") {
    try {
      if (!request.text.trim()) {
        parsedValue = null;
        const response: ParseResponse = { id: request.id, type: "parse", value: null, error: null, nodeCount: 0, nodeCountCapped: false };
        self.postMessage(response);
        return;
      }
      const value = JSON.parse(request.text) as JsonValue;
      parsedValue = value;
      const stats = countNodes(value);
      const response: ParseResponse = { id: request.id, type: "parse", value, error: null, nodeCount: stats.count, nodeCountCapped: stats.capped };
      self.postMessage(response);
    } catch (error) {
      parsedValue = null;
      const response: ParseResponse = { id: request.id, type: "parse", value: null, error: error instanceof Error ? error.message : String(error), nodeCount: 0, nodeCountCapped: false };
      self.postMessage(response);
    }
    return;
  }

  const result = parsedValue === null ? { paths: [], directPaths: [], truncated: false } : searchValue(parsedValue, request.query);
  const response: SearchResponse = { id: request.id, type: "search", query: request.query, ...result };
  self.postMessage(response);
};
