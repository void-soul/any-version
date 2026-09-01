import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { JsonFlowCanvas } from "../CanvasFlow";
import MonacoEditor from "../shared/MonacoEditor";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Braces,
  Check,
  ChevronDown,
  ChevronRight,
  Clipboard,
  Copy,
  Download,
  FileJson,
  FilePlus2,
  FolderOpen,
  Indent,
  ListTree,
  Map as MapIcon,
  Minimize2,
  PanelLeft,
  PanelRight,
  Play,
  RefreshCw,
  Save,
  Search,
  Sparkles,
  Split,
  X,
} from "lucide-react";

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
type ViewMode = "tree" | "graph";


type JsonTab = {
  id: string;
  path: string | null;
  name: string;
  text: string;
  savedText: string | null;
};

type ParsedJson = {
  value: JsonValue | null;
  error: string | null;
  nodeCount: number;
  nodeCountCapped: boolean;
};

export type SearchMatches = {
  query: string;
  paths: Set<string>;
  directPaths: Set<string>;
  truncated?: boolean;
};

const EMPTY_SEARCH_MATCHES: SearchMatches = { query: "", paths: new Set<string>(), directPaths: new Set<string>() };

const JSON_RECENT_KEY = "any_version_json_recent";
const MAX_TABS = 9;
const MAX_STRUCTURED_TEXT_BYTES = 20 * 1024 * 1024;
const JSON_EXTENSIONS = ["json", "jsonc", "json5", "ndjson"];

function typeOf(value: JsonValue): "null" | "array" | "object" | "string" | "number" | "boolean" {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value as "object" | "string" | "number" | "boolean";
}

function parseJson(text: string): ParsedJson {
  if (!text.trim()) return { value: null, error: null, nodeCount: 0, nodeCountCapped: false };
  try {
    const value = JSON.parse(text) as JsonValue;
    return { value, error: null, nodeCount: countNodes(value), nodeCountCapped: false };
  } catch (error) {
    return { value: null, error: error instanceof Error ? error.message : String(error), nodeCount: 0, nodeCountCapped: false };
  }
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

function isContainer(value: JsonValue): boolean {
  return Array.isArray(value) || (typeof value === "object" && value !== null);
}

function entriesOf(value: JsonValue, limit = Number.MAX_SAFE_INTEGER): Array<[string, JsonValue]> {
  if (Array.isArray(value)) {
    const result: Array<[string, JsonValue]> = [];
    const end = Math.min(value.length, limit);
    for (let index = 0; index < end; index += 1) result.push([String(index), value[index]]);
    return result;
  }
  if (typeof value === "object" && value !== null) return Object.entries(value).slice(0, limit);
  return [];
}

function containerSize(value: JsonValue): number {
  if (Array.isArray(value)) return value.length;
  if (typeof value === "object" && value !== null) return Object.keys(value).length;
  return 0;
}

function countNodes(value: JsonValue): number {
  const stack: JsonValue[] = [value];
  let count = 0;
  while (stack.length > 0 && count < 2_000_000) {
    const current = stack.pop() as JsonValue;
    count += 1;
    if (isContainer(current)) {
      const children = entriesOf(current);
      for (let index = children.length - 1; index >= 0; index -= 1) stack.push(children[index][1]);
    }
  }
  return count;
}

function copyValue(value: JsonValue, pretty = true): string {
  return typeof value === "string" ? value : JSON.stringify(value, null, pretty ? 2 : 0);
}

function sortJson(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map(sortJson);
  if (typeof value !== "object" || value === null) return value;
  return Object.fromEntries(
    Object.entries(value)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, child]) => [key, sortJson(child)]),
  );
}


// ─── 虚拟化结构树 ───
// 将“可见”的树拍平成一行行数据（只包含已展开的容器），再按固定行高做窗口化渲染：
// 无论 JSON 有多大，DOM 中始终只存在视口附近的几十行，从根上避免超大文档卡死。

const TREE_ROW_HEIGHT = 22;
const TREE_OVERSCAN = 10;
/** 虚拟树最多拍平的行数：超过后截断并提示，避免展开超大数组时一次性占用过多内存。 */
const MAX_FLAT_TREE_ROWS = 200_000;

const SCALAR_VALUE_CLASS: Record<string, string> = {
  string: "text-emerald-300",
  number: "text-amber-300",
  boolean: "text-violet-300",
  null: "text-slate-500",
};

/** 容器子元素数量展示：数组 O(1)，对象最多数 1000 个键，超出显示 "1000+"。 */
function displayChildCount(value: JsonValue): string {
  if (Array.isArray(value)) return String(value.length);
  if (typeof value === "object" && value !== null) {
    let count = 0;
    for (const _key in value) {
      count += 1;
      if (count > 1000) return "1000+";
    }
    return String(count);
  }
  return "0";
}

type FlatTreeRow = {
  path: string;
  name: string;
  value: JsonValue;
  depth: number;
  container: boolean;
  collapsed: boolean;
};

/**
 * 迭代式拍平“当前可见”的树：
 * - 搜索时只保留匹配路径（含祖先），且强制展开；
 * - 折叠的容器整棵跳过；
 * - 大型文档默认折叠（依赖 expandedPaths 逐层展开）；
 * - 总行数超过 MAX_FLAT_TREE_ROWS 时截断（truncated=true）。
 */
function flattenTreeRows(
  value: JsonValue,
  searchMatches: SearchMatches,
  collapsed: Set<string>,
  largeDocument: boolean,
  expandedPaths: Set<string>,
): { rows: FlatTreeRow[]; truncated: boolean } {
  const rows: FlatTreeRow[] = [];
  const searching = Boolean(searchMatches.query);
  const stack: Array<{ path: string; name: string; value: JsonValue; depth: number }> = [
    { path: "root", name: "root", value, depth: 0 },
  ];
  let truncated = false;

  while (stack.length > 0 && !truncated) {
    const current = stack.pop() as (typeof stack)[number];
    if (searching && !searchMatches.paths.has(current.path)) continue;
    const container = isContainer(current.value);
    const collapsedHere =
      container &&
      !searching &&
      (collapsed.has(current.path) ||
        (largeDocument && current.depth > 0 && !expandedPaths.has(current.path)));
    rows.push({
      path: current.path,
      name: current.name,
      value: current.value,
      depth: current.depth,
      container,
      collapsed: collapsedHere,
    });
    if (rows.length >= MAX_FLAT_TREE_ROWS) {
      truncated = true;
      break;
    }
    if (!container || collapsedHere) continue;
    // 只展开当前还有预算的子元素：超大数组/对象不会一次性全部入栈。
    const budget = MAX_FLAT_TREE_ROWS - rows.length;
    const children = childEntriesBounded(current.value, budget + 1);
    if (children.length > budget) truncated = true;
    for (let index = children.length - 1; index >= 0; index -= 1) {
      const [childName, child] = children[index];
      stack.push({
        path: current.path ? `${current.path}.${childName}` : childName,
        name: childName,
        value: child,
        depth: current.depth + 1,
      });
    }
  }
  return { rows, truncated };
}

/** 取容器前 limit 个子元素（数组用下标，对象用 for..in，避免 Object.entries 全量拷贝）。 */
function childEntriesBounded(value: JsonValue, limit: number): Array<[string, JsonValue]> {
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

function VirtualJsonTree({ value, searchMatches, collapsed, largeDocument, expandedPaths, onToggle, onCopy }: {
  value: JsonValue;
  searchMatches: SearchMatches;
  collapsed: Set<string>;
  largeDocument: boolean;
  expandedPaths: Set<string>;
  onToggle: (path: string) => void;
  onCopy: (value: string) => void;
}) {
  const { t } = useTranslation();
  const flat = useMemo(
    () => flattenTreeRows(value, searchMatches, collapsed, largeDocument, expandedPaths),
    [value, searchMatches, collapsed, largeDocument, expandedPaths],
  );
  const rows = flat.rows;
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);

  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const measure = () => setViewportHeight(node.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  // 折叠/搜索变化后滚回顶部，避免停留在已失效的偏移上。
  useEffect(() => {
    setScrollTop(0);
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [value, searchMatches.query, collapsed.size, expandedPaths.size]);

  const start = Math.max(0, Math.floor(scrollTop / TREE_ROW_HEIGHT) - TREE_OVERSCAN);
  const end = Math.min(rows.length, Math.ceil((scrollTop + viewportHeight) / TREE_ROW_HEIGHT) + TREE_OVERSCAN);
  const slice = rows.slice(start, end);

  return (
    <div
      ref={scrollRef}
      className="h-full overflow-auto font-mono"
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
    >
      <div style={{ height: rows.length * TREE_ROW_HEIGHT, position: "relative" }}>
        {slice.map((row, index) => {
          const absoluteIndex = start + index;
          const hit = searchMatches.directPaths.has(row.path);
          const keyClass = hit ? "text-cyan-200 bg-cyan-400/15 rounded px-0.5" : "text-sky-300";
          const type = typeOf(row.value);
          return (
            <div
              key={row.path}
              className="group absolute inset-x-0 flex items-center gap-1 rounded px-1 text-[12px] hover:bg-white/[0.06]"
              style={{ top: absoluteIndex * TREE_ROW_HEIGHT, height: TREE_ROW_HEIGHT, paddingLeft: `${row.depth * 15 + 2}px` }}
            >
              {row.container ? (
                <button
                  type="button"
                  onClick={() => onToggle(row.path)}
                  className="flex h-4 w-4 items-center justify-center text-slate-500 hover:text-slate-200"
                  aria-label={row.collapsed ? t("jsonb.treeExpand") : t("jsonb.treeCollapse")}
                  title={row.collapsed ? t("jsonb.treeExpand") : t("jsonb.treeCollapse")}
                >
                  {row.collapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
                </button>
              ) : (
                <span className="w-4" />
              )}
              {row.name !== "root" && <><span className={keyClass}>{row.name}</span><span className="text-slate-600">:</span></>}
              {row.container ? (
                <span className="text-sky-400">{type === "array" ? `array [${displayChildCount(row.value)}]` : `object {${displayChildCount(row.value)}}`}</span>
              ) : (
                <span className={SCALAR_VALUE_CLASS[type] ?? "text-slate-400"}>{type === "string" ? `"${row.value}"` : String(row.value)}</span>
              )}
              <button
                type="button"
                onClick={() => onCopy(copyValue(row.value))}
                className="ml-1 opacity-0 transition-opacity group-hover:opacity-100 text-slate-500 hover:text-slate-200"
                title={row.container ? t("jsonb.copySubtree") : t("jsonb.copyValue")}
                aria-label={row.container ? t("jsonb.copySubtree") : t("jsonb.copyValue")}
              >
                <Copy className="h-3 w-3" />
              </button>
            </div>
          );
        })}
      </div>
      {flat.truncated && (
        <div className="sticky bottom-0 px-2 py-1 text-[10px] text-amber-200">
          {t("jsonb.truncated", { count: rows.length.toLocaleString() })}
        </div>
      )}
    </div>
  );
}

export default function JsonBrowser() {
  const { t } = useTranslation();
  const [tabs, setTabs] = useState<JsonTab[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("tree");
  const [selectedPath, setSelectedPath] = useState("root");
  const [query, setQuery] = useState("");
  const deferredQuery = useDeferredValue(query);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const [showLeft, setShowLeft] = useState(true);
  const [showRight, setShowRight] = useState(true);
  const [syncScroll, setSyncScroll] = useState(true);
  const [notice, setNotice] = useState("");
  const [collapseAllToken, setCollapseAllToken] = useState(0);
  const [copyState, setCopyState] = useState(false);
  const sourceEditorRef = useRef<any>(null);
  const noticeTimer = useRef<number | null>(null);
  // 挂载恢复期间抑制「无标签则自动新建空白标签」逻辑
  const restoringRef = useRef(false);

  const active = tabs.find((tab) => tab.id === activeId) ?? null;
  const [parsed, setParsed] = useState<ParsedJson>({ value: null, error: null, nodeCount: 0, nodeCountCapped: false });
  const [parsePending, setParsePending] = useState(false);
  const [searchMatches, setSearchMatches] = useState<SearchMatches>(EMPTY_SEARCH_MATCHES);
  const [oversizedDocument, setOversizedDocument] = useState(false);
  const workerRef = useRef<Worker | null>(null);
  const workerReady = useRef(false);
  const parseRequestId = useRef(0);
  const searchRequestId = useRef(0);
  const largeDocument = oversizedDocument || parsed.nodeCount > 10_000 || parsed.nodeCountCapped;

  useEffect(() => {
    const worker = new Worker(new URL("./jsonWorker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    workerReady.current = true;
    worker.onmessage = (event: MessageEvent<{ id: number; type: string; value?: JsonValue | null; error?: string | null; nodeCount?: number; nodeCountCapped?: boolean; query?: string; paths?: string[]; directPaths?: string[]; truncated?: boolean }>) => {
      const response = event.data;
      if (response.type === "parse" && response.id === parseRequestId.current) {
        setParsed({ value: response.value ?? null, error: response.error ?? null, nodeCount: response.nodeCount ?? 0, nodeCountCapped: response.nodeCountCapped ?? false });
        setParsePending(false);
        setCollapsed(response.nodeCount && response.nodeCount > 10_000 ? new Set(["root"]) : new Set());
        setExpandedPaths(new Set());
      }
      if (response.type === "search" && response.id === searchRequestId.current) {
        setSearchMatches({ query: response.query ?? "", paths: new Set(response.paths ?? []), directPaths: new Set(response.directPaths ?? []), truncated: response.truncated });
      }
    };
    return () => {
      workerReady.current = false;
      worker.terminate();
      workerRef.current = null;
    };
  }, []);

  useEffect(() => {
    const requestId = ++parseRequestId.current;
    const text = active?.text ?? "";
    setParsePending(true);
    setSearchMatches(EMPTY_SEARCH_MATCHES);
    setExpandedPaths(new Set());
    const byteLength = new Blob([text]).size;
    const oversized = byteLength > MAX_STRUCTURED_TEXT_BYTES;
    setOversizedDocument(oversized);
    if (oversized) {
      setParsed({ value: null, error: null, nodeCount: 0, nodeCountCapped: true });
      setParsePending(false);
      return;
    }
    const timer = window.setTimeout(() => {
      if (!workerReady.current || !workerRef.current) {
        const fallback = parseJson(text);
        setParsed(fallback);
        setParsePending(false);
        return;
      }
      workerRef.current.postMessage({ id: requestId, type: "parse", text });
    }, 120);
    return () => window.clearTimeout(timer);
  }, [active?.text]);

  useEffect(() => {
    const queryValue = deferredQuery.trim();
    if (!queryValue || !parsed.value || !workerReady.current || !workerRef.current) {
      setSearchMatches(queryValue ? EMPTY_SEARCH_MATCHES : { ...EMPTY_SEARCH_MATCHES, query: "" });
      return;
    }
    const requestId = ++searchRequestId.current;
    workerRef.current.postMessage({ id: requestId, type: "search", query: queryValue });
  }, [deferredQuery, parsed.value]);
  const dirty = Boolean(active && active.savedText !== null && active.text !== active.savedText);

  const flash = useCallback((message: string) => {
    setNotice(message);
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(""), 2400);
  }, []);

  useEffect(() => () => {
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
  }, []);

  const createTab = useCallback((name = t("jsonb.untitled"), text = "") => {
    if (tabs.length >= MAX_TABS) {
      flash(t("jsonb.maxTabs", { max: MAX_TABS }));
      return null;
    }
    const id = `json-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const next: JsonTab = { id, path: null, name, text, savedText: null };
    setTabs((previous) => [...previous, next]);
    setActiveId(id);
    setCollapsed(new Set());
    return id;
  }, [flash, tabs.length]);

  // 挂载时恢复上次打开的 JSON 文件（模块卸载重挂载不丢已打开文档）。
  useEffect(() => {
    let alive = true;
    restoringRef.current = true;
    let paths: string[] = [];
    try {
      const saved = JSON.parse(localStorage.getItem(JSON_RECENT_KEY) || "[]");
      paths = Array.isArray(saved) ? saved.filter((p): p is string => typeof p === "string" && !!p) : [];
    } catch {
      // 忽略
    }
    (async () => {
      try {
        for (const p of paths) {
          if (!alive) return;
          await openPath(p);
        }
      } finally {
        if (alive) restoringRef.current = false;
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 打开/关闭标签后同步持久化文件路径列表（存本地，供挂载恢复）。
  useEffect(() => {
    const paths = tabs.map((t) => t.path).filter((p): p is string => !!p);
    try {
      localStorage.setItem(JSON_RECENT_KEY, JSON.stringify(paths));
    } catch {
      // 忽略写入失败
    }
  }, [tabs]);

  useEffect(() => {
    if (tabs.length === 0 && !restoringRef.current) createTab();
  }, [createTab, tabs.length]);

  useEffect(() => {
    if (!active) return;
    setViewMode("tree");
    setCollapsed(new Set());
    setExpandedPaths(new Set());
  }, [activeId]);

  const updateActive = useCallback((patch: Partial<JsonTab>) => {
    if (!activeId) return;
    setTabs((previous) => previous.map((tab) => tab.id === activeId ? { ...tab, ...patch } : tab));
  }, [activeId]);

  const openPath = useCallback(async (path: string) => {
    try {
      const text = await invoke<string>("read_text_file", { path });
      const existing = tabs.find((tab) => tab.path?.toLowerCase() === path.toLowerCase());
      if (existing) {
        setActiveId(existing.id);
        updateActive({ text });
        flash(t("jsonb.reloaded", { name: existing.name }));
        return;
      }
      if (tabs.length >= MAX_TABS) {
        flash(t("jsonb.maxTabs", { max: MAX_TABS }));
        return;
      }
      const separator = path.includes("\\") ? "\\" : "/";
      const name = path.slice(path.lastIndexOf(separator) + 1) || "document.json";
      const id = `json-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      setTabs((previous) => [...previous, { id, path, name, text, savedText: text }]);
      setActiveId(id);
      flash(t("jsonb.opened", { name }));
    } catch (error) {
      flash(t("jsonb.openFailed", { err: error instanceof Error ? error.message : String(error) }));
    }
  }, [flash, tabs, updateActive]);

  const openFile = useCallback(async () => {
    try {
      const selected = await open({
        multiple: false,
        title: t("jsonb.openTitle"),
        filters: [{ name: t("jsonb.openFilter"), extensions: JSON_EXTENSIONS }],
      });
      if (typeof selected === "string") await openPath(selected);
    } catch (error) {
      flash(t("jsonb.openFailed", { err: error instanceof Error ? error.message : String(error) }));
    }
  }, [flash, openPath]);

  const saveActive = useCallback(async (saveAs = false) => {
    if (!active) return;
    try {
      let path = active.path;
      if (!path || saveAs) {
        const selected = await save({
          title: saveAs ? t("jsonb.saveAsTitle") : t("jsonb.saveTitle"),
          defaultPath: active.name,
          filters: [{ name: t("jsonb.saveFilter"), extensions: ["json"] }],
        });
        if (typeof selected !== "string") return;
        path = selected;
      }
      await invoke("write_text_file", { path, content: active.text });
      const separator = path.includes("\\") ? "\\" : "/";
      updateActive({ path, name: path.slice(path.lastIndexOf(separator) + 1) || active.name, savedText: active.text });
      flash(t("jsonb.saved", { name: path.slice(path.lastIndexOf(separator) + 1) }));
    } catch (error) {
      flash(t("jsonb.saveFailed", { err: error instanceof Error ? error.message : String(error) }));
    }
  }, [active, flash, updateActive]);

  const closeTab = useCallback((id: string) => {
    const target = tabs.find((tab) => tab.id === id);
    if (!target) return;
    if (target.savedText !== null && target.savedText !== target.text && !window.confirm(t("jsonb.unsavedClose", { name: target.name }))) return;
    const index = tabs.findIndex((tab) => tab.id === id);
    const next = tabs.filter((tab) => tab.id !== id);
    if (next.length === 0) {
      const newId = `json-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      setTabs([{ id: newId, path: null, name: t("jsonb.untitled"), text: "", savedText: null }]);
      setActiveId(newId);
      setCollapsed(new Set());
      return;
    }
    setTabs(next);
    if (id === activeId) setActiveId(next[index]?.id ?? next[index - 1]?.id ?? next[0].id);
  }, [activeId, tabs]);

  const applyParsed = useCallback((transform: (value: JsonValue) => JsonValue, pretty = true) => {
    if (parsed.value === null) {
      flash(t("jsonb.invalidJson"));
      return;
    }
    updateActive({ text: JSON.stringify(transform(parsed.value), null, pretty ? 2 : 0) });
  }, [flash, parsed.value, updateActive]);

  const format = () => applyParsed((value) => value, true);
  const minify = () => applyParsed((value) => value, false);
  const sort = () => applyParsed(sortJson, true);

  const escape = () => {
    if (!active?.text.trim()) return;
    updateActive({ text: JSON.stringify(active.text) });
    flash(t("jsonb.escaped"));
  };

  const unescape = () => {
    if (!active?.text.trim()) return;
    try {
      const value = JSON.parse(active.text) as JsonValue;
      updateActive({ text: typeof value === "string" ? value : JSON.stringify(value, null, 2) });
      flash(t("jsonb.unescaped"));
    } catch {
      flash(t("jsonb.unescapeFailed"));
    }
  };

  const copySource = async () => {
    if (!active) return;
    try {
      await navigator.clipboard.writeText(active.text);
      setCopyState(true);
      window.setTimeout(() => setCopyState(false), 1200);
    } catch {
      flash(t("jsonb.copyFailed"));
    }
  };

  const copyValue = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      flash(t("jsonb.copied"));
    } catch {
      flash(t("jsonb.copyFailed"));
    }
  };

  const handleDrop = async (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    const file = event.dataTransfer.files[0];
    if (!file) return;
    try {
      const text = await file.text();
      const id = createTab(file.name, text);
      if (id) setTabs((previous) => previous.map((tab) => tab.id === id ? { ...tab, savedText: text } : tab));
    } catch (error) {
      flash(t("jsonb.dropFailed", { err: error instanceof Error ? error.message : String(error) }));
    }
  };

  const expandAll = () => setCollapsed(new Set());
  const revealPathInEditor = useCallback((path: string) => {
    setSelectedPath(path);
    const target = sourceEditorRef.current;
    if (!target || !active) return;
    const segments = path.split(".").slice(1);
    const needle = segments.length ? `"${segments[segments.length - 1]}"` : "{";
    const lines = active.text.split(/\r?\n/);
    const lineIndex = lines.findIndex((line) => line.includes(needle));
    if (lineIndex >= 0) {
      target.revealLineInCenter(lineIndex + 1);
      target.setPosition({ lineNumber: lineIndex + 1, column: Math.max(1, lines[lineIndex].indexOf(needle) + 1) });
      target.focus();
    }
  }, [active]);

  const collapseAll = () => {
    if (largeDocument) {
      setCollapsed(new Set(["root"]));
      setExpandedPaths(new Set());
    } else {
      const next = new Set<string>();
      const stack: Array<{ value: JsonValue; path: string }> = parsed.value === null ? [] : [{ value: parsed.value, path: "root" }];
      while (stack.length > 0) {
        const current = stack.pop() as (typeof stack)[number];
        if (!isContainer(current.value)) continue;
        next.add(current.path);
        entriesOf(current.value).forEach(([name, child]) => stack.push({ value: child, path: `${current.path}.${name}` }));
      }
      setCollapsed(next);
    }
    setCollapseAllToken((token) => token + 1);
  };

  const scrollEditors = (_source: "left" | "right", _position: unknown) => {
    // 同步滚动已废弃：右侧不再有编辑器
  };

  const topCount = parsed.value !== null && isContainer(parsed.value) ? containerSize(parsed.value) : 0;
  const nodes = parsed.nodeCount;
  const language = "json";
  const buttonClass = "inline-flex h-7 items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 text-[11px] font-medium text-slate-300 transition-colors hover:bg-white/[0.11] hover:text-white disabled:cursor-not-allowed disabled:opacity-40";
  const iconButtonClass = "inline-flex h-7 w-7 items-center justify-center rounded-md border border-white/10 bg-white/[0.05] text-slate-400 transition-colors hover:bg-white/[0.11] hover:text-white disabled:cursor-not-allowed disabled:opacity-40";

  return (
    <div className="flex h-full min-h-0 flex-col bg-slate-950/30 text-slate-200" onDrop={handleDrop} onDragOver={(event) => event.preventDefault()}>
      <div className="flex min-h-11 flex-wrap items-center gap-1.5 border-b border-white/10 bg-slate-950/55 px-2.5 py-1.5">
        <button type="button" className={iconButtonClass} onClick={() => createTab()} title={t("jsonb.newDoc")} aria-label={t("jsonb.newDoc")}><FilePlus2 className="h-3.5 w-3.5" /></button>
        <button type="button" className={buttonClass} onClick={openFile}><FolderOpen className="h-3.5 w-3.5" />{t("jsonb.open")}</button>
        <button type="button" className={buttonClass} onClick={() => void saveActive()} disabled={!active}><Save className="h-3.5 w-3.5" />{t("jsonb.save")}</button>
        <button type="button" className={iconButtonClass} onClick={() => void saveActive(true)} disabled={!active} title={t("jsonb.saveAs")} aria-label={t("jsonb.saveAs")}><Download className="h-3.5 w-3.5" /></button>
        <span className="mx-1 h-4 w-px bg-white/10" />
        <button type="button" className={buttonClass} onClick={format} disabled={!parsed.value}><Indent className="h-3.5 w-3.5" />{t("jsonb.format")}</button>
        <button type="button" className={buttonClass} onClick={minify} disabled={!parsed.value}><Minimize2 className="h-3.5 w-3.5" />{t("jsonb.minify")}</button>
        <button type="button" className={iconButtonClass} onClick={sort} disabled={!parsed.value} title={t("jsonb.sortKeys")} aria-label={t("jsonb.sortKeys")}><Sparkles className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={escape} disabled={!active?.text} title={t("jsonb.escape")} aria-label={t("jsonb.escape")}><Braces className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={unescape} disabled={!active?.text} title={t("jsonb.unescape")} aria-label={t("jsonb.unescape")}><RefreshCw className="h-3.5 w-3.5" /></button>
        <span className="mx-1 h-4 w-px bg-white/10" />
        <button type="button" className={iconButtonClass} onClick={expandAll} disabled={!parsed.value} title={t("jsonb.expandAll")} aria-label={t("jsonb.expandAll")}><ChevronDown className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={collapseAll} disabled={!parsed.value} title={t("jsonb.collapseAll")} aria-label={t("jsonb.collapseAll")}><ChevronRight className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={() => void copySource()} disabled={!active} title={t("jsonb.copySource")} aria-label={t("jsonb.copySource")}>{copyState ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Clipboard className="h-3.5 w-3.5" />}</button>
        <div className="ml-auto flex min-w-[180px] max-w-[260px] flex-1 items-center gap-1 rounded-md border border-white/10 bg-white/[0.04] px-2">
          <Search className="h-3.5 w-3.5 shrink-0 text-slate-500" />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("jsonb.searchPlaceholder")} className="h-7 min-w-0 flex-1 bg-transparent text-[11px] outline-none placeholder:text-slate-600" />
        </div>
      </div>

      <div className="flex min-h-8 shrink-0 items-stretch gap-0.5 overflow-x-auto border-b border-white/10 bg-slate-950/35 px-2 pt-1">
        {tabs.map((tab) => (
          <div key={tab.id} className={`group flex max-w-[220px] shrink-0 items-center gap-1 rounded-t-md border border-b-0 px-2 py-1 text-[11px] ${tab.id === activeId ? "border-white/10 bg-white/[0.08] text-white" : "border-transparent text-slate-500 hover:bg-white/[0.04] hover:text-slate-300"}`}>
            <button type="button" onClick={() => setActiveId(tab.id)} className="min-w-0 truncate" title={tab.path ?? tab.name}>{tab.name}{tab.savedText !== null && tab.savedText !== tab.text ? " •" : ""}</button>
            <button type="button" onClick={() => closeTab(tab.id)} className="shrink-0 text-slate-600 opacity-0 transition-opacity hover:text-red-300 group-hover:opacity-100" title={t("jsonb.closeTab")} aria-label={t("jsonb.closeTabAria", { name: tab.name })}><X className="h-3 w-3" /></button>
          </div>
        ))}
        {tabs.length < MAX_TABS && <button type="button" className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded text-slate-500 hover:bg-white/[0.08] hover:text-white" onClick={() => createTab()} title={t("jsonb.newTab")} aria-label={t("jsonb.newTab")}><FilePlus2 className="h-3 w-3" /></button>}
      </div>

      <div className="flex min-h-0 flex-1">
        {showLeft && (
          <section className={`${showRight ? "w-1/2 border-r" : "w-full"} flex min-h-0 min-w-0 flex-col border-white/10`}>
            <div className="flex h-8 shrink-0 items-center justify-between border-b border-white/10 px-3 text-[10px] text-slate-500">
              <span className="flex items-center gap-1"><FileJson className="h-3 w-3" />{t("jsonb.sourceEditor")}{dirty && <span className="text-amber-400">{t("jsonb.unsaved")}</span>}</span>
              <span>{active ? `${formatBytes(new Blob([active.text]).size)} · ${language}` : ""}</span>
            </div>
            <div className="min-h-0 flex-1" onScroll={(event) => scrollEditors("left", { scrollTop: event.currentTarget.scrollTop, scrollLeft: event.currentTarget.scrollLeft })}>
              <MonacoEditor
                height="100%"
                language="json"
                theme="vs-dark"
                value={active?.text ?? ""}
                onChange={(value) => updateActive({ text: value ?? "" })}
                onMount={(editor) => {
                  sourceEditorRef.current = editor;
                  editor.onDidScrollChange((event: any) => {
                    if (event.scrollTopChanged || event.scrollLeftChanged) {
                      scrollEditors("left", { scrollTop: event.scrollTop, scrollLeft: event.scrollLeft });
                    }
                  });
                }}
                options={{ minimap: { enabled: false }, fontSize: 12, lineNumbers: "on", wordWrap: "on", automaticLayout: true, tabSize: 2, padding: { top: 8, bottom: 8 } }}
              />
            </div>
            {parsed.error && <div className="shrink-0 border-t border-red-500/20 bg-red-500/10 px-3 py-1.5 text-[10px] text-red-300">{t("jsonb.parseError", { err: parsed.error })}</div>}
          </section>
        )}

        {showRight && (
          <section className={`${showLeft ? "w-1/2" : "w-full"} flex min-h-0 min-w-0 flex-col bg-slate-950/20`}>
            <div className="flex h-8 shrink-0 items-center gap-1 border-b border-white/10 px-2 text-[10px] text-slate-500">
              <button type="button" onClick={() => setViewMode("tree")} className={`flex h-6 items-center gap-1 rounded px-2 ${viewMode === "tree" ? "bg-cyan-500/15 text-cyan-300" : "hover:bg-white/[0.06]"}`}><ListTree className="h-3 w-3" />{t("jsonb.tree")}</button>
              <button type="button" onClick={() => setViewMode("graph")} className={`flex h-6 items-center gap-1 rounded px-2 ${viewMode === "graph" ? "bg-cyan-500/15 text-cyan-300" : "hover:bg-white/[0.06]"}`}><MapIcon className="h-3 w-3" />{t("jsonb.graph")}</button>
              <span className="ml-auto flex items-center gap-2">
                {oversizedDocument ? <span className="text-amber-300">{t("jsonb.oversizedPaused")}</span> : parsed.value !== null && <span>{parsePending ? t("jsonb.parsing") : t("jsonb.itemsNodes", { top: topCount, nodes, plus: parsed.nodeCountCapped ? "+" : "" })}</span>}
                <button type="button" onClick={() => setSyncScroll((value) => !value)} className={syncScroll ? "text-cyan-300" : "text-slate-600"} title={syncScroll ? t("jsonb.syncScrollOn") : t("jsonb.syncScrollOff")} aria-label={t("jsonb.syncScrollAria")}><Split className="h-3.5 w-3.5" /></button>
                <button type="button" onClick={() => setShowLeft((value) => !value)} className="hover:text-slate-200" title={t("jsonb.toggleLeft")} aria-label={t("jsonb.toggleLeft")}><PanelLeft className="h-3.5 w-3.5" /></button>
              </span>
            </div>

            {viewMode === "tree" && (
              <div className="min-h-0 flex-1 overflow-hidden p-2 font-mono">
                {oversizedDocument ? <div className="py-10 text-center text-[11px] leading-5 text-amber-200">{t("jsonb.oversizedTree")}</div> : parsed.value === null ? <div className="py-10 text-center text-[11px] text-slate-600">{t("jsonb.treeEmpty")}</div> : <VirtualJsonTree value={parsed.value} searchMatches={searchMatches} collapsed={collapsed} largeDocument={largeDocument} expandedPaths={expandedPaths} onToggle={(path) => { if (largeDocument && path !== "root") { setExpandedPaths((previous) => { const next = new Set(previous); if (next.has(path)) next.delete(path); else next.add(path); return next; }); } else { setCollapsed((previous) => { const next = new Set(previous); if (next.has(path)) next.delete(path); else next.add(path); return next; }); } }} onCopy={(value) => void copyValue(value)} />}
              </div>
            )}
            {viewMode === "graph" && (
              <div className="min-h-0 flex-1">
                {oversizedDocument ? <div className="flex h-full items-center justify-center px-6 text-center text-[11px] leading-5 text-amber-200">{t("jsonb.oversizedGraph")}</div> : parsed.value === null ? <div className="flex h-full items-center justify-center text-[11px] text-slate-600">{t("jsonb.graphEmpty")}</div> : <JsonFlowCanvas value={parsed.value} selectedPath={selectedPath} searchMatches={searchMatches} onSelectPath={revealPathInEditor} onCopy={(value) => void copyValue(value)} collapseAllToken={collapseAllToken} />}
              </div>
            )}
          </section>
        )}

        {!showLeft && !showRight && <div className="flex flex-1 items-center justify-center text-[11px] text-slate-600">{t("jsonb.needPanel")}</div>}
      </div>

      <div className="flex h-7 shrink-0 items-center gap-3 border-t border-white/10 bg-slate-950/55 px-3 text-[10px] text-slate-500">
        <span className="flex items-center gap-1"><Play className="h-3 w-3 text-emerald-400" />{oversizedDocument ? t("jsonb.modeText") : parsePending ? t("jsonb.modeParsing") : parsed.error ? t("jsonb.modeInvalid") : parsed.value === null ? t("jsonb.modeWaiting") : t("jsonb.modeValid")}</span>
        <span>{active ? `${active.name}${active.path ? ` · ${active.path}` : ""}` : t("jsonb.noDoc")}</span>
        <span className="ml-auto">{active ? `${formatBytes(new Blob([active.text]).size)} · ${t("jsonb.lines", { count: active.text.split(/\r?\n/).length })}` : ""}</span>
        <button type="button" className="hover:text-slate-200" onClick={() => setShowRight((value) => !value)} title={showRight ? t("jsonb.hideRight") : t("jsonb.showRight")} aria-label={showRight ? t("jsonb.hideRight") : t("jsonb.showRight")}>{showRight ? <PanelRight className="h-3.5 w-3.5" /> : <PanelRight className="h-3.5 w-3.5 opacity-50" />}</button>
        <button type="button" className="hover:text-slate-200" onClick={() => { setShowLeft(true); setShowRight(true); }} title={t("jsonb.restoreSplit")} aria-label={t("jsonb.restoreSplit")}><Split className="h-3.5 w-3.5" /></button>
      </div>
      {notice && <div className="pointer-events-none absolute bottom-10 left-1/2 z-30 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900/95 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
    </div>
  );
}
