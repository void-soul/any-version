import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Editor from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeftRight,
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
  GitCompareArrows,
  Indent,
  ListTree,
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

type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
type ViewMode = "tree" | "graph" | "text" | "compare";

type GraphItem = {
  id: string;
  name: string;
  path: string;
  value: JsonValue;
  depth: number;
  x: number;
  y: number;
  width: number;
  height: number;
  parentId: string | null;
};

type JsonTab = {
  id: string;
  path: string | null;
  name: string;
  text: string;
  savedText: string | null;
};

type TreeNodeProps = {
  name: string;
  value: JsonValue;
  path: string;
  depth: number;
  query: string;
  collapsed: Set<string>;
  onToggle: (path: string) => void;
  onCopy: (value: string) => void;
};

const MAX_TABS = 9;
const JSON_EXTENSIONS = ["json", "jsonc", "json5", "ndjson"];

function typeOf(value: JsonValue): "null" | "array" | "object" | "string" | "number" | "boolean" {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value as "object" | "string" | "number" | "boolean";
}

function parseJson(text: string): { value: JsonValue | null; error: string | null } {
  if (!text.trim()) return { value: null, error: null };
  try {
    return { value: JSON.parse(text) as JsonValue, error: null };
  } catch (error) {
    return { value: null, error: error instanceof Error ? error.message : String(error) };
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

function entriesOf(value: JsonValue): Array<[string, JsonValue]> {
  if (Array.isArray(value)) return value.map((item, index) => [String(index), item]);
  if (typeof value === "object" && value !== null) return Object.entries(value);
  return [];
}

function countNodes(value: JsonValue): number {
  return entriesOf(value).reduce((total, [, child]) => total + countNodes(child), 1);
}

function containsQuery(name: string, value: JsonValue, path: string, query: string): boolean {
  if (!query) return true;
  const normalized = query.toLowerCase();
  if (name.toLowerCase().includes(normalized) || path.toLowerCase().includes(normalized)) return true;
  if (!isContainer(value)) return String(value).toLowerCase().includes(normalized);
  return entriesOf(value).some(([childName, child]) =>
    containsQuery(childName, child, path ? `${path}.${childName}` : childName, query),
  );
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

function normalizeJsonText(value: string): string {
  return value.replace(/\r\n?/g, "\n");
}

function TreeNode({ name, value, path, depth, query, collapsed, onToggle, onCopy }: TreeNodeProps) {
  if (query && !containsQuery(name, value, path, query)) return null;
  const type = typeOf(value);
  const container = isContainer(value);
  const isCollapsed = collapsed.has(path) && !query;
  const hit = Boolean(query && (name.toLowerCase().includes(query.toLowerCase()) || path.toLowerCase().includes(query.toLowerCase())));
  const keyClass = hit ? "text-cyan-200 bg-cyan-400/15 rounded px-0.5" : "text-sky-300";
  const valueClass: Record<string, string> = {
    string: "text-emerald-300",
    number: "text-amber-300",
    boolean: "text-violet-300",
    null: "text-slate-500",
  };
  const children = entriesOf(value);

  return (
    <div>
      <div
        className="group flex min-h-[22px] items-center gap-1 rounded px-1 font-mono text-[12px] hover:bg-white/[0.06]"
        style={{ paddingLeft: `${depth * 15 + 2}px` }}
      >
        {container ? (
          <button
            type="button"
            onClick={() => onToggle(path)}
            className="flex h-4 w-4 items-center justify-center text-slate-500 hover:text-slate-200"
            aria-label={isCollapsed ? "展开" : "折叠"}
            title={isCollapsed ? "展开" : "折叠"}
          >
            {isCollapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
          </button>
        ) : (
          <span className="w-4" />
        )}
        {name !== "root" && <><span className={keyClass}>{name}</span><span className="text-slate-600">:</span></>}
        {container ? (
          <span className="text-sky-400">{type === "array" ? `array [${children.length}]` : `object {${children.length}}`}</span>
        ) : (
          <span className={valueClass[type] ?? "text-slate-400"}>{type === "string" ? `"${value}"` : String(value)}</span>
        )}
        <button
          type="button"
          onClick={() => onCopy(copyValue(value))}
          className="ml-1 opacity-0 transition-opacity group-hover:opacity-100 text-slate-500 hover:text-slate-200"
          title={container ? "复制子树" : "复制值"}
          aria-label={container ? "复制子树" : "复制值"}
        >
          <Copy className="h-3 w-3" />
        </button>
      </div>
      {container && !isCollapsed && (
        <div>
          {children.map(([childName, child]) => (
            <TreeNode
              key={`${path}.${childName}`}
              name={childName}
              value={child}
              path={path ? `${path}.${childName}` : childName}
              depth={depth + 1}
              query={query}
              collapsed={collapsed}
              onToggle={onToggle}
              onCopy={onCopy}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function buildGraphItems(value: JsonValue): GraphItem[] {
  const items: GraphItem[] = [];
  const rowByDepth = new Map<number, number>();
  const maxDepth = 0;
  const visit = (name: string, current: JsonValue, path: string, depth: number, parentId: string | null) => {
    const id = path || "root";
    const row = rowByDepth.get(depth) ?? 0;
    rowByDepth.set(depth, row + 1);
    const label = name === "root" ? "root" : name;
    const type = typeOf(current);
    const summary = isContainer(current)
      ? type === "array" ? `[${entriesOf(current).length}]` : `{${entriesOf(current).length}}`
      : type === "string" ? `"${String(current).slice(0, 28)}${String(current).length > 28 ? "…" : ""}"` : String(current);
    items.push({
      id,
      name: label,
      path: id,
      value: current,
      depth,
      x: depth * 235,
      y: row * 76,
      width: Math.max(150, Math.min(245, 42 + label.length * 7 + summary.length * 5)),
      height: 48,
      parentId,
    });
    if (isContainer(current)) {
      entriesOf(current).forEach(([childName, child]) => visit(childName, child, path ? `${path}.${childName}` : childName, depth + 1, id));
    }
  };
  visit("root", value, "root", maxDepth, null);
  const maxRows = Math.max(...Array.from(rowByDepth.values()), 1);
  return items.map((item) => ({ ...item, y: (item.y - (maxRows - 1) * 38) }));
}

function GraphCanvas({ value, selectedPath, onSelectPath }: { value: JsonValue; selectedPath: string; onSelectPath: (path: string) => void }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const transformRef = useRef({ x: 36, y: 0, scale: 1 });
  const dragRef = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const items = useMemo(() => buildGraphItems(value), [value]);
  const itemById = useMemo(() => new Map(items.map((item) => [item.id, item])), [items]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const viewport = viewportRef.current;
    if (!canvas || !viewport) return;
    const rect = viewport.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.floor(rect.width * ratio));
    canvas.height = Math.max(1, Math.floor(rect.height * ratio));
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${rect.height}px`;
    const context = canvas.getContext("2d");
    if (!context) return;
    const transform = transformRef.current;
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, rect.width, rect.height);
    context.fillStyle = "#080f1c";
    context.fillRect(0, 0, rect.width, rect.height);
    context.save();
    context.translate(transform.x, transform.y);
    context.scale(transform.scale, transform.scale);
    context.lineWidth = 1.4 / transform.scale;
    items.forEach((item) => {
      if (!item.parentId) return;
      const parent = itemById.get(item.parentId);
      if (!parent) return;
      context.beginPath();
      context.moveTo(parent.x + parent.width, parent.y + parent.height / 2);
      context.bezierCurveTo(parent.x + parent.width + 55, parent.y + parent.height / 2, item.x - 55, item.y + item.height / 2, item.x, item.y + item.height / 2);
      context.strokeStyle = "rgba(100, 116, 139, .65)";
      context.stroke();
    });
    items.forEach((item) => {
      const selected = item.path === selectedPath;
      const container = isContainer(item.value);
      context.fillStyle = selected ? "#164e63" : container ? "#111f35" : "#101827";
      context.strokeStyle = selected ? "#22d3ee" : container ? "#2d6081" : "#334155";
      context.lineWidth = (selected ? 2 : 1) / transform.scale;
      context.beginPath();
      context.roundRect(item.x, item.y, item.width, item.height, 6);
      context.fill();
      context.stroke();
      context.font = "600 12px ui-monospace, SFMono-Regular, Consolas, monospace";
      context.fillStyle = selected ? "#cffafe" : "#bae6fd";
      context.fillText(item.name.length > 25 ? `${item.name.slice(0, 22)}...` : item.name, item.x + 10, item.y + 19);
      const type = typeOf(item.value);
      const summary = isContainer(item.value)
        ? type === "array" ? `[${entriesOf(item.value).length}]` : `{${entriesOf(item.value).length}}`
        : type === "string" ? `"${String(item.value).slice(0, 28)}${String(item.value).length > 28 ? "..." : ""}"` : String(item.value);
      context.font = "11px ui-monospace, SFMono-Regular, Consolas, monospace";
      context.fillStyle = type === "string" ? "#86efac" : type === "number" ? "#fcd34d" : "#94a3b8";
      context.fillText(summary, item.x + 10, item.y + 36);
    });
    context.restore();
  }, [itemById, items, selectedPath]);

  useEffect(() => {
    draw();
    const observer = new ResizeObserver(draw);
    if (viewportRef.current) observer.observe(viewportRef.current);
    return () => observer.disconnect();
  }, [draw]);

  const hitTest = (clientX: number, clientY: number) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return null;
    const transform = transformRef.current;
    const x = (clientX - rect.left - transform.x) / transform.scale;
    const y = (clientY - rect.top - transform.y) / transform.scale;
    return [...items].reverse().find((item) => x >= item.x && x <= item.x + item.width && y >= item.y && y <= item.y + item.height) ?? null;
  };

  return (
    <div ref={viewportRef} className="relative h-full min-h-0 overflow-hidden bg-slate-950" onWheel={(event) => {
      event.preventDefault();
      const rect = canvasRef.current?.getBoundingClientRect();
      if (!rect) return;
      const oldScale = transformRef.current.scale;
      const nextScale = Math.max(0.35, Math.min(2.5, oldScale * (event.deltaY < 0 ? 1.1 : 0.9)));
      const cursorX = event.clientX - rect.left;
      const cursorY = event.clientY - rect.top;
      transformRef.current = { x: cursorX - (cursorX - transformRef.current.x) * nextScale / oldScale, y: cursorY - (cursorY - transformRef.current.y) * nextScale / oldScale, scale: nextScale };
      draw();
    }} onPointerDown={(event) => {
      event.currentTarget.setPointerCapture(event.pointerId);
      dragRef.current = { x: event.clientX, y: event.clientY, moved: false };
    }} onPointerMove={(event) => {
      const drag = dragRef.current;
      if (!drag) return;
      const dx = event.clientX - drag.x;
      const dy = event.clientY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
      transformRef.current.x += dx;
      transformRef.current.y += dy;
      drag.x = event.clientX;
      drag.y = event.clientY;
      draw();
    }} onPointerUp={(event) => {
      const drag = dragRef.current;
      dragRef.current = null;
      if (!drag?.moved) {
        const item = hitTest(event.clientX, event.clientY);
        if (item) onSelectPath(item.path);
      }
    }}>
      <canvas ref={canvasRef} className="absolute inset-0 cursor-grab active:cursor-grabbing" />
      <div className="pointer-events-none absolute bottom-3 left-3 rounded border border-white/10 bg-slate-900/80 px-2 py-1 text-[10px] text-slate-500">滚轮缩放 · 拖动画布 · 点击节点定位</div>
    </div>
  );
}

function CompareSummary({ left, right }: { left: string; right: string }) {
  const leftLines = normalizeJsonText(left).split("\n");
  const rightLines = normalizeJsonText(right).split("\n");
  const max = Math.max(leftLines.length, rightLines.length);
  let changed = 0;
  for (let index = 0; index < max; index += 1) {
    if (leftLines[index] !== rightLines[index]) changed += 1;
  }
  const equal = changed === 0;
  return (
    <div className={`flex items-center gap-2 border-b px-3 py-2 text-[11px] ${equal ? "border-emerald-500/20 bg-emerald-500/10 text-emerald-300" : "border-amber-500/20 bg-amber-500/10 text-amber-200"}`}>
      <GitCompareArrows className="h-3.5 w-3.5" />
      {equal ? "两份文档内容一致" : `发现 ${changed} 行差异 · 左侧 ${leftLines.length} 行 · 右侧 ${rightLines.length} 行`}
    </div>
  );
}

export default function JsonBrowser() {
  const [tabs, setTabs] = useState<JsonTab[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("tree");
  const [selectedPath, setSelectedPath] = useState("root");
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [showLeft, setShowLeft] = useState(true);
  const [showRight, setShowRight] = useState(true);
  const [syncScroll, setSyncScroll] = useState(true);
  const [notice, setNotice] = useState("");
  const [rightText, setRightText] = useState("");
  const [compareText, setCompareText] = useState("");
  const [copyState, setCopyState] = useState(false);
  const sourceEditorRef = useRef<any>(null);
  const rightEditorRef = useRef<any>(null);
  const noticeTimer = useRef<number | null>(null);
  const syncLock = useRef<"left" | "right" | null>(null);

  const active = tabs.find((tab) => tab.id === activeId) ?? null;
  const parsed = useMemo(() => parseJson(active?.text ?? ""), [active?.text]);
  const compareParsed = useMemo(() => parseJson(compareText), [compareText]);
  const dirty = Boolean(active && active.savedText !== null && active.text !== active.savedText);

  const flash = useCallback((message: string) => {
    setNotice(message);
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(""), 2400);
  }, []);

  useEffect(() => () => {
    if (noticeTimer.current !== null) window.clearTimeout(noticeTimer.current);
  }, []);

  const createTab = useCallback((name = "未命名.json", text = "") => {
    if (tabs.length >= MAX_TABS) {
      flash(`最多打开 ${MAX_TABS} 个文档`);
      return null;
    }
    const id = `json-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const next: JsonTab = { id, path: null, name, text, savedText: null };
    setTabs((previous) => [...previous, next]);
    setActiveId(id);
    setCollapsed(new Set());
    return id;
  }, [flash, tabs.length]);

  useEffect(() => {
    if (tabs.length === 0) createTab();
  }, [createTab, tabs.length]);

  useEffect(() => {
    if (!active) return;
    setRightText(active.text);
    setCompareText("");
    setViewMode("tree");
    setCollapsed(new Set());
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
        flash(`已重新加载 ${existing.name}`);
        return;
      }
      if (tabs.length >= MAX_TABS) {
        flash(`最多打开 ${MAX_TABS} 个文档`);
        return;
      }
      const separator = path.includes("\\") ? "\\" : "/";
      const name = path.slice(path.lastIndexOf(separator) + 1) || "document.json";
      const id = `json-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      setTabs((previous) => [...previous, { id, path, name, text, savedText: text }]);
      setActiveId(id);
      flash(`已打开 ${name}`);
    } catch (error) {
      flash(`打开失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }, [flash, tabs, updateActive]);

  const openFile = useCallback(async () => {
    try {
      const selected = await open({
        multiple: false,
        title: "打开 JSON 文档",
        filters: [{ name: "JSON / Text", extensions: JSON_EXTENSIONS }],
      });
      if (typeof selected === "string") await openPath(selected);
    } catch (error) {
      flash(`打开失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }, [flash, openPath]);

  const saveActive = useCallback(async (saveAs = false) => {
    if (!active) return;
    try {
      let path = active.path;
      if (!path || saveAs) {
        const selected = await save({
          title: saveAs ? "JSON 文档另存为" : "保存 JSON 文档",
          defaultPath: active.name,
          filters: [{ name: "JSON", extensions: ["json"] }],
        });
        if (typeof selected !== "string") return;
        path = selected;
      }
      await invoke("write_text_file", { path, content: active.text });
      const separator = path.includes("\\") ? "\\" : "/";
      updateActive({ path, name: path.slice(path.lastIndexOf(separator) + 1) || active.name, savedText: active.text });
      flash(`已保存 ${path.slice(path.lastIndexOf(separator) + 1)}`);
    } catch (error) {
      flash(`保存失败：${error instanceof Error ? error.message : String(error)}`);
    }
  }, [active, flash, updateActive]);

  const closeTab = useCallback((id: string) => {
    const target = tabs.find((tab) => tab.id === id);
    if (!target) return;
    if (target.savedText !== null && target.savedText !== target.text && !window.confirm(`“${target.name}”有未保存修改，仍要关闭吗？`)) return;
    const index = tabs.findIndex((tab) => tab.id === id);
    const next = tabs.filter((tab) => tab.id !== id);
    if (next.length === 0) {
      const newId = `json-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
      setTabs([{ id: newId, path: null, name: "未命名.json", text: "", savedText: null }]);
      setActiveId(newId);
      setCollapsed(new Set());
      return;
    }
    setTabs(next);
    if (id === activeId) setActiveId(next[index]?.id ?? next[index - 1]?.id ?? next[0].id);
  }, [activeId, tabs]);

  const applyParsed = useCallback((transform: (value: JsonValue) => JsonValue, pretty = true) => {
    if (parsed.value === null) {
      flash("当前 JSON 无法解析");
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
    flash("已转义为 JSON 字符串");
  };

  const unescape = () => {
    if (!active?.text.trim()) return;
    try {
      const value = JSON.parse(active.text) as JsonValue;
      updateActive({ text: typeof value === "string" ? value : JSON.stringify(value, null, 2) });
      flash("已取消转义");
    } catch {
      flash("取消转义失败：当前内容不是有效 JSON");
    }
  };

  const copySource = async () => {
    if (!active) return;
    try {
      await navigator.clipboard.writeText(active.text);
      setCopyState(true);
      window.setTimeout(() => setCopyState(false), 1200);
    } catch {
      flash("复制失败");
    }
  };

  const copyValue = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      flash("已复制");
    } catch {
      flash("复制失败");
    }
  };

  const loadCompareFile = async () => {
    try {
      const selected = await open({ multiple: false, title: "选择对比文档", filters: [{ name: "JSON", extensions: JSON_EXTENSIONS }] });
      if (typeof selected !== "string") return;
      const text = await invoke<string>("read_text_file", { path: selected });
      setCompareText(text);
      setViewMode("compare");
      flash("已载入右侧对比文档");
    } catch (error) {
      flash(`载入对比文档失败：${error instanceof Error ? error.message : String(error)}`);
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
      flash(`读取拖放文件失败：${error instanceof Error ? error.message : String(error)}`);
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
    const next = new Set<string>();
    const visit = (value: JsonValue, path: string) => {
      if (!isContainer(value)) return;
      next.add(path);
      entriesOf(value).forEach(([name, child]) => visit(child, path ? `${path}.${name}` : name));
    };
    if (parsed.value !== null) visit(parsed.value, "root");
    setCollapsed(next);
  };

  const scrollEditors = (source: "left" | "right", position: { scrollTop: number; scrollLeft: number }) => {
    if (!syncScroll || syncLock.current === source) return;
    const target = source === "left" ? rightEditorRef.current : sourceEditorRef.current;
    if (!target) return;
    syncLock.current = source;
    target.setScrollPosition(position);
    window.requestAnimationFrame(() => {
      if (syncLock.current === source) syncLock.current = null;
    });
  };

  const topCount = parsed.value !== null && isContainer(parsed.value) ? entriesOf(parsed.value).length : 0;
  const nodes = parsed.value !== null ? countNodes(parsed.value) : 0;
  const language = "json";
  const buttonClass = "inline-flex h-7 items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 text-[11px] font-medium text-slate-300 transition-colors hover:bg-white/[0.11] hover:text-white disabled:cursor-not-allowed disabled:opacity-40";
  const iconButtonClass = "inline-flex h-7 w-7 items-center justify-center rounded-md border border-white/10 bg-white/[0.05] text-slate-400 transition-colors hover:bg-white/[0.11] hover:text-white disabled:cursor-not-allowed disabled:opacity-40";

  return (
    <div className="flex h-full min-h-0 flex-col bg-slate-950/30 text-slate-200" onDrop={handleDrop} onDragOver={(event) => event.preventDefault()}>
      <div className="flex min-h-11 flex-wrap items-center gap-1.5 border-b border-white/10 bg-slate-950/55 px-2.5 py-1.5">
        <button type="button" className={iconButtonClass} onClick={() => createTab()} title="新建文档" aria-label="新建文档"><FilePlus2 className="h-3.5 w-3.5" /></button>
        <button type="button" className={buttonClass} onClick={openFile}><FolderOpen className="h-3.5 w-3.5" />打开</button>
        <button type="button" className={buttonClass} onClick={() => void saveActive()} disabled={!active}><Save className="h-3.5 w-3.5" />保存</button>
        <button type="button" className={iconButtonClass} onClick={() => void saveActive(true)} disabled={!active} title="另存为" aria-label="另存为"><Download className="h-3.5 w-3.5" /></button>
        <span className="mx-1 h-4 w-px bg-white/10" />
        <button type="button" className={buttonClass} onClick={format} disabled={!parsed.value}><Indent className="h-3.5 w-3.5" />格式化</button>
        <button type="button" className={buttonClass} onClick={minify} disabled={!parsed.value}><Minimize2 className="h-3.5 w-3.5" />压缩</button>
        <button type="button" className={iconButtonClass} onClick={sort} disabled={!parsed.value} title="按键名排序" aria-label="按键名排序"><Sparkles className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={escape} disabled={!active?.text} title="转义" aria-label="转义"><Braces className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={unescape} disabled={!active?.text} title="取消转义" aria-label="取消转义"><RefreshCw className="h-3.5 w-3.5" /></button>
        <span className="mx-1 h-4 w-px bg-white/10" />
        <button type="button" className={iconButtonClass} onClick={expandAll} disabled={!parsed.value} title="展开全部" aria-label="展开全部"><ChevronDown className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={collapseAll} disabled={!parsed.value} title="折叠全部" aria-label="折叠全部"><ChevronRight className="h-3.5 w-3.5" /></button>
        <button type="button" className={iconButtonClass} onClick={() => void copySource()} disabled={!active} title="复制源文档" aria-label="复制源文档">{copyState ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Clipboard className="h-3.5 w-3.5" />}</button>
        <div className="ml-auto flex min-w-[180px] max-w-[260px] flex-1 items-center gap-1 rounded-md border border-white/10 bg-white/[0.04] px-2">
          <Search className="h-3.5 w-3.5 shrink-0 text-slate-500" />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索键名 / 路径 / 值" className="h-7 min-w-0 flex-1 bg-transparent text-[11px] outline-none placeholder:text-slate-600" />
        </div>
      </div>

      <div className="flex min-h-8 shrink-0 items-stretch gap-0.5 overflow-x-auto border-b border-white/10 bg-slate-950/35 px-2 pt-1">
        {tabs.map((tab) => (
          <div key={tab.id} className={`group flex max-w-[220px] shrink-0 items-center gap-1 rounded-t-md border border-b-0 px-2 py-1 text-[11px] ${tab.id === activeId ? "border-white/10 bg-white/[0.08] text-white" : "border-transparent text-slate-500 hover:bg-white/[0.04] hover:text-slate-300"}`}>
            <button type="button" onClick={() => setActiveId(tab.id)} className="min-w-0 truncate" title={tab.path ?? tab.name}>{tab.name}{tab.savedText !== null && tab.savedText !== tab.text ? " •" : ""}</button>
            <button type="button" onClick={() => closeTab(tab.id)} className="shrink-0 text-slate-600 opacity-0 transition-opacity hover:text-red-300 group-hover:opacity-100" title="关闭标签" aria-label={`关闭 ${tab.name}`}><X className="h-3 w-3" /></button>
          </div>
        ))}
        {tabs.length < MAX_TABS && <button type="button" className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded text-slate-500 hover:bg-white/[0.08] hover:text-white" onClick={() => createTab()} title="新建标签" aria-label="新建标签"><FilePlus2 className="h-3 w-3" /></button>}
      </div>

      <div className="flex min-h-0 flex-1">
        {showLeft && (
          <section className={`${showRight ? "w-1/2 border-r" : "w-full"} flex min-h-0 min-w-0 flex-col border-white/10`}>
            <div className="flex h-8 shrink-0 items-center justify-between border-b border-white/10 px-3 text-[10px] text-slate-500">
              <span className="flex items-center gap-1"><FileJson className="h-3 w-3" />源编辑器{dirty && <span className="text-amber-400">· 未保存</span>}</span>
              <span>{active ? `${formatBytes(new Blob([active.text]).size)} · ${language}` : ""}</span>
            </div>
            <div className="min-h-0 flex-1" onScroll={(event) => scrollEditors("left", { scrollTop: event.currentTarget.scrollTop, scrollLeft: event.currentTarget.scrollLeft })}>
              <Editor
                height="100%"
                language="json"
                theme="vs-dark"
                value={active?.text ?? ""}
                onChange={(value) => updateActive({ text: value ?? "" })}
                onMount={(editor) => {
                  sourceEditorRef.current = editor;
                  editor.onDidScrollChange((event) => {
                    if (event.scrollTopChanged || event.scrollLeftChanged) {
                      scrollEditors("left", { scrollTop: event.scrollTop, scrollLeft: event.scrollLeft });
                    }
                  });
                }}
                options={{ minimap: { enabled: false }, fontSize: 12, lineNumbers: "on", wordWrap: "on", automaticLayout: true, tabSize: 2, padding: { top: 8, bottom: 8 } }}
              />
            </div>
            {parsed.error && <div className="shrink-0 border-t border-red-500/20 bg-red-500/10 px-3 py-1.5 text-[10px] text-red-300">解析错误：{parsed.error}</div>}
          </section>
        )}

        {showRight && (
          <section className={`${showLeft ? "w-1/2" : "w-full"} flex min-h-0 min-w-0 flex-col bg-slate-950/20`}>
            <div className="flex h-8 shrink-0 items-center gap-1 border-b border-white/10 px-2 text-[10px] text-slate-500">
              <button type="button" onClick={() => setViewMode("tree")} className={`flex h-6 items-center gap-1 rounded px-2 ${viewMode === "tree" ? "bg-cyan-500/15 text-cyan-300" : "hover:bg-white/[0.06]"}`}><ListTree className="h-3 w-3" />结构</button>
              <button type="button" onClick={() => setViewMode("graph")} className={`flex h-6 items-center gap-1 rounded px-2 ${viewMode === "graph" ? "bg-cyan-500/15 text-cyan-300" : "hover:bg-white/[0.06]"}`}><GitCompareArrows className="h-3 w-3" />图形树</button>
              <button type="button" onClick={() => { setRightText(active?.text ?? ""); setViewMode("text"); }} className={`flex h-6 items-center gap-1 rounded px-2 ${viewMode === "text" ? "bg-cyan-500/15 text-cyan-300" : "hover:bg-white/[0.06]"}`}><Braces className="h-3 w-3" />预览</button>
              <button type="button" onClick={() => setViewMode("compare")} className={`flex h-6 items-center gap-1 rounded px-2 ${viewMode === "compare" ? "bg-cyan-500/15 text-cyan-300" : "hover:bg-white/[0.06]"}`}><GitCompareArrows className="h-3 w-3" />对比</button>
              <span className="ml-auto flex items-center gap-2">
                {parsed.value !== null && <span>顶层 {topCount} · 节点 {nodes}</span>}
                <button type="button" onClick={() => setSyncScroll((value) => !value)} className={syncScroll ? "text-cyan-300" : "text-slate-600"} title={syncScroll ? "关闭同步滚动" : "开启同步滚动"} aria-label="同步滚动"><Split className="h-3.5 w-3.5" /></button>
                <button type="button" onClick={() => setShowLeft((value) => !value)} className="hover:text-slate-200" title="切换左侧编辑器" aria-label="切换左侧编辑器"><PanelLeft className="h-3.5 w-3.5" /></button>
              </span>
            </div>

            {viewMode === "tree" && (
              <div className="min-h-0 flex-1 overflow-auto p-2 font-mono">
                {parsed.value === null ? <div className="py-10 text-center text-[11px] text-slate-600">输入有效 JSON 后显示结构树</div> : <TreeNode name="root" value={parsed.value} path="root" depth={0} query={query} collapsed={collapsed} onToggle={(path) => setCollapsed((previous) => { const next = new Set(previous); if (next.has(path)) next.delete(path); else next.add(path); return next; })} onCopy={(value) => void copyValue(value)} />}
              </div>
            )}
            {viewMode === "graph" && (
              <div className="min-h-0 flex-1">
                {parsed.value === null ? <div className="flex h-full items-center justify-center text-[11px] text-slate-600">输入有效 JSON 后显示图形树</div> : <GraphCanvas value={parsed.value} selectedPath={selectedPath} onSelectPath={revealPathInEditor} />}
              </div>
            )}
            {viewMode === "text" && (
              <div className="min-h-0 flex-1">
                <Editor
                  height="100%"
                  language="json"
                  theme="vs-dark"
                  value={rightText}
                  onChange={(value) => setRightText(value ?? "")}
                  onMount={(editor) => {
                    rightEditorRef.current = editor;
                    editor.onDidScrollChange((event) => {
                      if (event.scrollTopChanged || event.scrollLeftChanged) {
                        scrollEditors("right", { scrollTop: event.scrollTop, scrollLeft: event.scrollLeft });
                      }
                    });
                  }}
                  options={{ readOnly: true, minimap: { enabled: false }, fontSize: 12, lineNumbers: "on", wordWrap: "on", automaticLayout: true, padding: { top: 8, bottom: 8 } }}
                />
              </div>
            )}
            {viewMode === "compare" && (
              <div className="flex min-h-0 flex-1 flex-col">
                <div className="flex shrink-0 items-center gap-1 border-b border-white/10 px-2 py-1.5">
                  <button type="button" className={buttonClass} onClick={() => void loadCompareFile()}><FolderOpen className="h-3.5 w-3.5" />载入对比文件</button>
                  <button type="button" className={buttonClass} onClick={() => setCompareText(active?.text ?? "")}><ArrowLeftRight className="h-3.5 w-3.5" />复制当前内容</button>
                  {compareParsed.error && <span className="text-[10px] text-red-300">右侧 JSON 无效</span>}
                </div>
                <CompareSummary left={active?.text ?? ""} right={compareText} />
                <div className="min-h-0 flex-1"><Editor height="100%" language="json" theme="vs-dark" value={compareText} onChange={(value) => setCompareText(value ?? "")} options={{ minimap: { enabled: false }, fontSize: 12, lineNumbers: "on", wordWrap: "on", automaticLayout: true, padding: { top: 8, bottom: 8 } }} /></div>
              </div>
            )}
          </section>
        )}

        {!showLeft && !showRight && <div className="flex flex-1 items-center justify-center text-[11px] text-slate-600">至少打开一个面板</div>}
      </div>

      <div className="flex h-7 shrink-0 items-center gap-3 border-t border-white/10 bg-slate-950/55 px-3 text-[10px] text-slate-500">
        <span className="flex items-center gap-1"><Play className="h-3 w-3 text-emerald-400" />{parsed.error ? "JSON 无效" : parsed.value === null ? "等待输入" : "JSON 有效"}</span>
        <span>{active ? `${active.name}${active.path ? ` · ${active.path}` : ""}` : "无文档"}</span>
        <span className="ml-auto">{active ? `${formatBytes(new Blob([active.text]).size)} · ${active.text.split(/\r?\n/).length} 行` : ""}</span>
        <button type="button" className="hover:text-slate-200" onClick={() => setShowRight((value) => !value)} title={showRight ? "隐藏右侧面板" : "显示右侧面板"} aria-label={showRight ? "隐藏右侧面板" : "显示右侧面板"}>{showRight ? <PanelRight className="h-3.5 w-3.5" /> : <PanelRight className="h-3.5 w-3.5 opacity-50" />}</button>
        <button type="button" className="hover:text-slate-200" onClick={() => { setShowLeft(true); setShowRight(true); }} title="恢复双栏" aria-label="恢复双栏"><Split className="h-3.5 w-3.5" /></button>
      </div>
      {notice && <div className="pointer-events-none absolute bottom-10 left-1/2 z-30 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900/95 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
    </div>
  );
}
