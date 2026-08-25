import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  applyNodeChanges,
  Background,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  getBezierPath,
  useReactFlow,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { invoke } from "@tauri-apps/api/core";
import Editor from "@monaco-editor/react";
import {
  CalendarDays,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Columns2,
  Eye,
  FilePlus2,
  GripVertical,
  Image as ImageIcon,
  Maximize2,
  Minimize2,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { JsonValue, SearchMatches } from "./SystemTools/JsonBrowser";
import { TaskItem, TaskStatus, STATUS_META, UpdateTaskInput, deriveStatus, TaskSticker } from "./tasks/types";
import { moduleAccent } from "../utils/theme";

type JsonGraphItem = {
  id: string;
  name: string;
  path: string;
  value: JsonValue;
  parentId: string | null;
  depth: number;
};

type JsonFlowNodeData = {
  item: JsonGraphItem;
  selectedPath: string;
  searchMatches: SearchMatches;
  collapsed: Set<string>;
  onSelect: (path: string) => void;
  onToggle: (path: string) => void;
  onCopy: (value: string) => void;
};

type TaskFlowNodeData = {
  task: TaskItem;
  selected: boolean;
  collapsed: boolean;
  hasChildren: boolean;
  onSelect: () => void;
  onToggle: () => void;
  onAddChild: () => void;
  onDelete: () => void;
  onProgress: (progress: number) => void;
  onUpdate: (patch: UpdateTaskInput) => void;
  onInsertFile: (content: string) => Promise<string | undefined>;
  onInsertImage: (content: string) => Promise<string | undefined>;
  onInsertScreenshot: (content: string) => Promise<string | undefined>;
  onOpenFile: (path: string) => void;
};

type TaskFlowNode = TaskItem & { x: number; y: number; childrenCount: number };

const JSON_EDGE_COLORS = ["#22d3ee", "#a78bfa", "#34d399", "#fbbf24", "#fb7185", "#60a5fa"];
const TASK_NODE_HEIGHT = 138;
const taskButton = "inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";
const taskIconButton = "inline-flex h-7 items-center justify-center gap-1 rounded border border-white/10 bg-white/[0.04] px-2 text-[9px] text-slate-400 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";

type JsonValueObject = { [key: string]: JsonValue };

function hashColor(id: string, palette: string[]): string {
  let hash = 0;
  for (let index = 0; index < id.length; index += 1) hash = (hash * 31 + id.charCodeAt(index)) >>> 0;
  return palette[hash % palette.length];
}

function jsonType(value: JsonValue): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  return typeof value;
}

function jsonEntries(value: JsonValue): Array<[string, JsonValue]> {
  if (Array.isArray(value)) return value.map((child, index) => [String(index), child]);
  if (typeof value === "object" && value !== null) return Object.entries(value);
  return [];
}

function isJsonContainer(value: JsonValue): boolean {
  return Array.isArray(value) || (typeof value === "object" && value !== null);
}

function objectKeyCountPreview(value: JsonValueObject): string {
  let count = 0;
  for (const key in value) {
    if (Object.prototype.hasOwnProperty.call(value, key)) count += 1;
    if (count > 1000) return "1000+";
  }
  return String(count);
}

function jsonSummary(value: JsonValue): string {
  const type = jsonType(value);
  if (Array.isArray(value)) return `array [${value.length}]`;
  if (typeof value === "object" && value !== null) return `object {${objectKeyCountPreview(value as JsonValueObject)}}`;
  if (typeof value === "string") return `"${value.length > 32 ? `${value.slice(0, 29)}...` : value}"`;
  return `${type}: ${String(value)}`;
}

function compactJsonValue(value: JsonValue): string {
  if (value === null) return "null";
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  const text = JSON.stringify(value);
  return text.length > 42 ? `${text.slice(0, 39)}...` : text;
}

function arrayColumns(value: JsonValue[]): string[] {
  const rows = value.slice(0, 80).filter((item): item is JsonValueObject => typeof item === "object" && item !== null && !Array.isArray(item));
  if (rows.length === 0) return ["value"];
  return [...new Set(rows.flatMap((row) => Object.keys(row)))].slice(0, 8);
}

function JsonArrayTable({ value, path, onSelect }: { value: JsonValue[]; path: string; onSelect: (path: string) => void }) {
  const columns = arrayColumns(value);
  const objectRows = columns[0] !== "value";
  const displayRows = value.slice(0, 80);
  return (
    <div className="mt-2 overflow-hidden rounded border border-white/10 bg-slate-950/60" onClick={(event) => event.stopPropagation()}>
      <div className="border-b border-white/10 px-2 py-1 text-[9px] font-semibold uppercase tracking-wide text-cyan-300">数组内容 · {value.length} 行</div>
      <div className="max-h-36 overflow-auto">
        <table className="w-full table-fixed border-collapse text-left font-mono text-[9px]">
          <thead><tr>{objectRows && <th className="sticky top-0 w-6 border-b border-white/10 bg-slate-900 px-1.5 py-1 text-slate-600">#</th>}{columns.map((column) => <th key={column} className="sticky top-0 max-w-[100px] border-b border-white/10 bg-slate-900 px-1.5 py-1 text-cyan-300">{column}</th>)}</tr></thead>
          <tbody>{displayRows.map((row, index) => <tr key={`${path}.${index}`} className="hover:bg-white/[0.05]" onClick={() => onSelect(`${path}.${index}`)}>{objectRows && <td className="border-b border-white/5 px-1.5 py-1 text-slate-600">{index + 1}</td>}{columns.map((column) => { const cell = objectRows && typeof row === "object" && row !== null && !Array.isArray(row) ? row[column] ?? null : row; return <td key={column} className="max-w-[100px] truncate border-b border-white/5 px-1.5 py-1 text-slate-300" title={JSON.stringify(cell)}>{compactJsonValue(cell)}</td>; })}</tr>)}</tbody>
        </table>
        {value.length > 80 && <div className="border-t border-white/10 px-2 py-1 text-[9px] text-slate-600">仅显示前 80 行</div>}
      </div>
    </div>
  );
}

const MAX_JSON_FLOW_ITEMS = 1800;

function buildJsonItems(value: JsonValue): JsonGraphItem[] {
  const result: JsonGraphItem[] = [];
  const stack: Array<{ name: string; current: JsonValue; path: string; parentId: string | null; depth: number }> = [{ name: "root", current: value, path: "root", parentId: null, depth: 0 }];
  while (stack.length > 0 && result.length < MAX_JSON_FLOW_ITEMS) {
    const current = stack.pop() as (typeof stack)[number];
    const id = current.path || "root";
    result.push({ id, name: current.name, path: id, value: current.current, parentId: current.parentId, depth: current.depth });
    if (isJsonContainer(current.current)) {
      const children = jsonEntries(current.current);
      for (let index = children.length - 1; index >= 0; index -= 1) {
        const [childName, child] = children[index];
        stack.push({ name: childName, current: child, path: `${current.path}.${childName}`, parentId: id, depth: current.depth + 1 });
      }
    }
  }
  return result;
}

function copyJsonValue(value: JsonValue): string {
  return typeof value === "string" ? value : JSON.stringify(value, null, 2);
}

const JsonFlowNode = memo(function JsonFlowNode({ data }: NodeProps<Node<JsonFlowNodeData>>) {
  const { item, selectedPath, searchMatches, collapsed, onSelect, onToggle, onCopy } = data;
  const selected = item.path === selectedPath;
  const chain = searchMatches.paths.has(item.path);
  const directMatch = searchMatches.directPaths.has(item.path);
  const container = isJsonContainer(item.value);
  const color = hashColor(item.id, JSON_EDGE_COLORS);
  return (
    <div className={`w-[250px] rounded-lg border bg-[#101827] px-2.5 py-2 shadow-xl ${selected ? "border-cyan-300 shadow-cyan-500/30" : chain ? "border-cyan-700/80" : "border-white/10"}`} onClick={() => onSelect(item.path)}>
      <Handle type="target" position={Position.Left} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
      <div className="flex items-center gap-1.5">
        {container && <button type="button" className="nodrag nopan inline-flex h-4 w-4 items-center justify-center text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); onToggle(item.path); }} title={collapsed.has(item.path) ? "展开" : "折叠"}>{collapsed.has(item.path) ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}</button>}
        <span className={`min-w-0 flex-1 truncate font-mono text-[11px] ${directMatch ? "text-yellow-300" : "text-cyan-200"}`}>{item.name}</span>
        <button type="button" className="nodrag nopan text-slate-600 hover:text-white" onClick={(event) => { event.stopPropagation(); onCopy(copyJsonValue(item.value)); }} title="复制节点内容"><span className="text-[10px]">⧉</span></button>
      </div>
      <div className="mt-1 truncate font-mono text-[10px]" style={{ color }}>{jsonSummary(item.value)}</div>
      {Array.isArray(item.value) && !collapsed.has(item.path) && <JsonArrayTable value={item.value} path={item.path} onSelect={onSelect} />}
      <Handle type="source" position={Position.Right} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
    </div>
  );
});

const ColorEdge = memo(function ColorEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, selected, data }: EdgeProps) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, curvature: 0.28 });
  const color = data?.color as string | undefined ?? "#22d3ee";
  const gradientId = `flow-edge-${id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  return <><defs><linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="0%"><stop offset="0%" stopColor={color} /><stop offset="100%" stopColor="#f8fafc" /></linearGradient></defs><path d={path} fill="none" stroke={color} strokeWidth={selected ? 8 : 5} opacity={selected ? 0.2 : 0.12} /><path d={path} fill="none" stroke={`url(#${gradientId})`} strokeWidth={selected ? 3.5 : 2.2} strokeLinecap="round" markerEnd={`url(#arrow-${gradientId})`} /><marker id={`arrow-${gradientId}`} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#f8fafc" /></marker></>;
});

function JsonFlowInner({ value, selectedPath, searchMatches, onSelectPath, onCopy, collapseAllToken }: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void; collapseAllToken: number }) {
  const { fitView } = useReactFlow();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [nodes, setNodes] = useState<Node<JsonFlowNodeData>[]>([]);
  const allItems = useMemo(() => buildJsonItems(value), [value]);
  const graphTruncated = allItems.length >= MAX_JSON_FLOW_ITEMS;
  const previousCollapseToken = useRef(0);
  useEffect(() => {
    if (collapseAllToken === previousCollapseToken.current) return;
    setCollapsed(new Set(allItems.filter((item) => isJsonContainer(item.value)).map((item) => item.path)));
    previousCollapseToken.current = collapseAllToken;
  }, [allItems, collapseAllToken]);
  const visibleItems = useMemo(() => allItems.filter((item) => { let current = item.parentId; while (current) { if (collapsed.has(current)) return false; current = allItems.find((candidate) => candidate.id === current)?.parentId ?? null; } return true; }), [allItems, collapsed]);
  const computedNodes = useMemo<Node<JsonFlowNodeData>[]>(() => visibleItems.map((item) => ({ id: item.id, type: "jsonNode", position: { x: item.depth * 260, y: item.depth === 0 ? 0 : visibleItems.filter((candidate) => candidate.depth === item.depth && candidate.id <= item.id).length * 92 }, data: { item, selectedPath, searchMatches, collapsed, onSelect: onSelectPath, onToggle: (path) => setCollapsed((current) => { const next = new Set(current); next.has(path) ? next.delete(path) : next.add(path); return next; }), onCopy }, sourcePosition: Position.Right, targetPosition: Position.Left })), [onCopy, onSelectPath, searchMatches, selectedPath, visibleItems, collapsed]);
  const edges = useMemo<Edge[]>(() => visibleItems.flatMap((item) => !item.parentId || !visibleItems.some((candidate) => candidate.id === item.parentId) ? [] : [{ id: `json-edge-${item.id}`, source: item.parentId, target: item.id, type: "color", data: { color: hashColor(item.id, JSON_EDGE_COLORS) }, markerEnd: { type: MarkerType.ArrowClosed, color: "#f8fafc" } }]), [visibleItems]);
  // 保持拖放位置：computed nodes 更新时，已有位置的节点保持当前位置
  useEffect(() => {
    setNodes((current) => {
      const currentById = new Map(current.map((n) => [n.id, n]));
      return computedNodes.map((next) => {
        const existing = currentById.get(next.id);
        return existing ? { ...next, position: existing.position } : next;
      });
    });
  }, [computedNodes]);
  // JSON 内容变化时重置位置
  useEffect(() => { setNodes(computedNodes); }, [value]);
  useEffect(() => { const timer = window.setTimeout(() => fitView({ padding: 0.2, duration: 240 }), 0); return () => window.clearTimeout(timer); }, [fitView, value, collapsed]);
  return <div className="relative h-full min-h-0"><ReactFlow nodes={nodes} edges={edges} nodeTypes={{ jsonNode: JsonFlowNode }} edgeTypes={{ color: ColorEdge }} onNodesChange={(changes) => setNodes((cur) => applyNodeChanges(changes, cur))} fitView minZoom={0.15} maxZoom={2.2} nodesDraggable nodesConnectable={false} elementsSelectable proOptions={{ hideAttribution: true }}><Background color="#1e293b" gap={24} size={1} /><MiniMap style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95" nodeColor={(node) => hashColor(String(node.id), JSON_EDGE_COLORS)} nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2, 6, 23, 0.72)" pannable zoomable /><Controls className="canvas-flow-controls" showInteractive={false} /></ReactFlow>{graphTruncated && <div className="pointer-events-none absolute left-3 top-3 z-10 rounded border border-amber-400/20 bg-slate-900/90 px-2 py-1 text-[10px] text-amber-200">图形树仅显示前 {MAX_JSON_FLOW_ITEMS} 个节点</div>}</div>;
}

export function JsonFlowCanvas(props: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void; collapseAllToken: number }) {
  return <div className="h-full min-h-0 bg-slate-950"><ReactFlowProvider><JsonFlowInner {...props} /></ReactFlowProvider></div>;
}

function taskStatus(task: TaskItem): TaskStatus { return deriveStatus(task.progress); }
/** 节点主题色：任务自定义颜色优先，否则回退到任务模块的主题色（--module-accent）。 */
function taskColor(task: TaskItem): string { return /^#[0-9a-f]{6}$/i.test(task.color) ? task.color : moduleAccent(); }
function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}
function isLocalFilePath(value: string): boolean { return /^(?:[A-Za-z]:[\\/]|\\\\|\/)/.test(value.trim()); }
/** 还原 markdown 解析时被百分号编码的路径（如空格 → %20），解码失败时回退原值。 */
function decodeLocalPath(value: string): string {
  try { return decodeURIComponent(value); } catch { return value; }
}
function appendMarkdownLine(content: string, line: string): string { return content.trimEnd() ? `${content.trimEnd()}\n\n${line}` : line; }

/** 本地图片：通过 image_to_base64 读取为 data URL 显示（不依赖 asset 协议作用域）。 */
function LocalTaskImage({ path, alt, onOpenFile }: { path: string; alt: string; onOpenFile: (path: string) => void }) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setSrc(null);
    setFailed(false);
    void invoke<string>("image_to_base64", { filePath: path })
      .then((data) => { if (!cancelled) setSrc(data); })
      .catch(() => { if (!cancelled) setFailed(true); });
    return () => { cancelled = true; };
  }, [path]);
  if (failed) {
    return <button type="button" className="my-2 block max-w-full text-left text-[9px] text-red-300" onClick={() => onOpenFile(path)} title="打开文件">图片加载失败，点击打开文件</button>;
  }
  if (!src) {
    return <div className="my-2 h-16 animate-pulse rounded border border-white/10 bg-slate-900/60" />;
  }
  return <button type="button" className="my-2 block max-w-full text-left" onClick={() => onOpenFile(path)} title="打开图片"><img src={src} alt={alt} className="max-h-48 max-w-full rounded border border-white/10 object-contain" /></button>;
}

/** react-markdown v10 默认 urlTransform 会把 `C:/...` 当作不安全协议清空 href/src，
 *  这里放行本地磁盘路径（盘符/UNC），其余仍按安全协议白名单处理（javascript: 等仍被清空）。 */
function taskUrlTransform(value: string): string {
  if (/^[A-Za-z]:[\\/]/.test(value) || value.startsWith("\\\\")) return value;
  const colon = value.indexOf(":");
  const questionMark = value.indexOf("?");
  const numberSign = value.indexOf("#");
  const slash = value.indexOf("/");
  if (colon === -1 || colon > (questionMark === -1 ? value.length : questionMark) || colon > (numberSign === -1 ? value.length : numberSign) || colon > (slash === -1 ? value.length : slash)) return value;
  const protocol = value.slice(0, colon).toLowerCase();
  return /^(https?|ircs?|mailto|xmpp)$/i.test(protocol) ? value : "";
}

function TaskMarkdown({ content, onOpenFile }: { content: string; onOpenFile: (path: string) => void }) {
  return <div className="task-markdown text-[10px] leading-relaxed text-slate-200 break-words">
    <ReactMarkdown remarkPlugins={[remarkGfm]} urlTransform={taskUrlTransform} components={{
      h1: ({ children }) => <h1 className="mt-2 mb-1 text-sm font-bold text-white">{children}</h1>,
      h2: ({ children }) => <h2 className="mt-2 mb-1 text-[12px] font-bold text-slate-100">{children}</h2>,
      h3: ({ children }) => <h3 className="mt-1.5 mb-1 text-[11px] font-semibold text-slate-200">{children}</h3>,
      p: ({ children }) => <p className="my-1.5 first:mt-0 last:mb-0">{children}</p>,
      ul: ({ children }) => <ul className="my-1.5 list-disc space-y-0.5 pl-4">{children}</ul>,
      ol: ({ children }) => <ol className="my-1.5 list-decimal space-y-0.5 pl-4">{children}</ol>,
      li: ({ children }) => <li>{children}</li>,
      blockquote: ({ children }) => <blockquote className="my-2 border-l-2 border-cyan-400/50 pl-2 text-slate-400">{children}</blockquote>,
      code: ({ children }) => <code className="rounded bg-slate-700/60 px-1 py-0.5 font-mono text-[9px] text-cyan-200">{children}</code>,
      pre: ({ children }) => <pre className="my-2 overflow-x-auto rounded border border-white/10 bg-slate-950 p-2 font-mono text-[9px] text-slate-300">{children}</pre>,
      strong: ({ children }) => <strong className="font-semibold text-white">{children}</strong>,
      del: ({ children }) => <del className="text-slate-500">{children}</del>,
      a: ({ href, children }) => {
        const target = href ?? "";
        const local = isLocalFilePath(target);
        return <a href={local ? undefined : target} target={local ? undefined : "_blank"} rel={local ? undefined : "noopener noreferrer"} onClick={(event) => { if (local) { event.preventDefault(); onOpenFile(decodeLocalPath(target)); } }} className="text-cyan-300 underline decoration-cyan-400/40 underline-offset-2 hover:text-cyan-100">{children}</a>;
      },
      img: ({ src, alt }) => {
        const target = typeof src === "string" ? src : "";
        if (isLocalFilePath(target)) {
          return <LocalTaskImage path={decodeLocalPath(target)} alt={alt ?? ""} onOpenFile={onOpenFile} />;
        }
        return <img src={target} alt={alt ?? ""} className="max-h-48 max-w-full rounded border border-white/10 object-contain" />;
      },
      table: ({ children }) => <div className="my-2 overflow-x-auto rounded border border-white/10"><table className="min-w-full text-[9px]">{children}</table></div>,
      thead: ({ children }) => <thead className="bg-slate-800/80">{children}</thead>,
      th: ({ children }) => <th className="border-b border-white/10 px-1.5 py-1 text-left text-cyan-200">{children}</th>,
      td: ({ children }) => <td className="border-b border-white/5 px-1.5 py-1 text-slate-300">{children}</td>,
      hr: () => <hr className="my-2 border-white/10" />,
    }}>{content || "暂无内容"}</ReactMarkdown>
  </div>;
}

function TaskDetailModal({ task, onClose, onUpdate, onInsertFile, onInsertImage, onInsertScreenshot, onOpenFile }: {
  task: TaskItem;
  onClose: () => void;
  onUpdate: (patch: UpdateTaskInput) => void;
  onInsertFile: (content: string) => Promise<string | undefined>;
  onInsertImage: (content: string) => Promise<string | undefined>;
  onInsertScreenshot: (content: string) => Promise<string | undefined>;
  onOpenFile: (path: string) => void;
}) {
  const color = taskColor(task);
  const [draft, setDraft] = useState(task.detail);
  const [mode, setMode] = useState<"edit" | "preview" | "split">("edit");
  const [fullscreen, setFullscreen] = useState(false);
  const [splitPct, setSplitPct] = useState(50);
  const splitRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);
  useEffect(() => { setDraft(task.detail); }, [task.detail]);
  const startSplitDrag = (event: React.PointerEvent) => {
    event.preventDefault();
    draggingRef.current = true;
    const container = splitRef.current;
    if (!container) return;
    const update = (clientX: number) => {
      const rect = container.getBoundingClientRect();
      if (rect.width <= 0) return;
      const pct = ((clientX - rect.left) / rect.width) * 100;
      setSplitPct(Math.min(85, Math.max(15, pct)));
    };
    update(event.clientX);
    const onMove = (moveEvent: PointerEvent) => update(moveEvent.clientX);
    const onUp = () => {
      draggingRef.current = false;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };
  const save = () => { if (draft !== task.detail) onUpdate({ detail: draft }); onClose(); };
  const insertWith = async (handler: (content: string) => Promise<string | undefined>) => { const next = await handler(draft); if (next !== undefined) setDraft(next); };
  const insertDate = () => { const now = new Date(); const date = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`; setDraft(appendMarkdownLine(draft, `**日期：${date}**`)); };
  const insertBar = (
    <div className="flex flex-wrap items-center gap-1">
      <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => void insertWith(onInsertFile)} title="插入文件路径"><FilePlus2 className="h-3 w-3" />文件</button>
      <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => void insertWith(onInsertImage)} title="插入图片"><ImageIcon className="h-3 w-3" />图片</button>
      <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => void insertWith(onInsertScreenshot)} title="插入剪贴板截图"><ImageIcon className="h-3 w-3" />截图</button>
      <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={insertDate} title="插入当前日期"><CalendarDays className="h-3 w-3" />日期</button>
      <span className="ml-auto font-mono text-[9px] text-slate-600">{draft.length} 字符</span>
    </div>
  );
  const editorPane = (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="relative min-h-0 flex-1">
        <Editor
          height="100%"
          language="markdown"
          theme="vs-dark"
          value={draft}
          onChange={(value) => setDraft(value ?? "")}
          onMount={(editor) => { if (mode === "edit") editor.focus(); }}
          options={{ minimap: { enabled: false }, fontSize: 12, lineNumbers: "on", wordWrap: "on", automaticLayout: true, tabSize: 2, padding: { top: 8, bottom: 8 }, scrollBeyondLastLine: false, renderLineHighlight: "line", overviewRulerLanes: 0, hideCursorInOverviewRuler: true }}
        />
        {!draft && <div className="pointer-events-none absolute left-4 top-2.5 text-[11px] text-slate-600">使用 Markdown 记录任务的详细内容…</div>}
      </div>
      <div className="flex shrink-0 items-center border-t border-white/10 px-3 py-2">{insertBar}</div>
    </div>
  );
  const previewPane = (
    <div className="min-h-0 flex-1 overflow-y-auto p-4"><TaskMarkdown content={draft} onOpenFile={onOpenFile} /></div>
  );
  return createPortal(
    <div className={`fixed inset-0 z-[200] flex items-center justify-center bg-black/70 backdrop-blur-[3px] ${fullscreen ? "p-0" : "p-6"}`} onClick={onClose}>
      <div className={`flex flex-col overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl ${fullscreen ? "h-[100vh] w-[100vw] rounded-none" : "h-[82vh] w-[min(92vw,860px)]"}`} onClick={(event) => event.stopPropagation()}>
        <div className="flex h-11 shrink-0 items-center gap-2 border-b border-white/10 px-3" style={{ backgroundColor: hexToRgba(color, 0.12) }}>
          <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color, boxShadow: `0 0 9px ${color}` }} />
          <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-slate-100">{task.title} · 详细</span>
          <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => setMode("edit")} title="仅编辑"><Pencil className="h-3 w-3" />编辑</button>
          <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => setMode("split")} title="左编辑右预览"><Columns2 className="h-3 w-3" />分栏</button>
          <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => setMode("preview")} title="仅渲染"><Eye className="h-3 w-3" />渲染</button>
          <button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => setFullscreen((value) => !value)} title={fullscreen ? "退出全屏" : "全屏"}>{fullscreen ? <Minimize2 className="h-3 w-3" /> : <Maximize2 className="h-3 w-3" />}</button>
          <button type="button" className="nodrag nopan ml-1 text-slate-500 hover:text-white" onClick={onClose} title="关闭"><X className="h-4 w-4" /></button>
        </div>
        {mode === "split" ? <div ref={splitRef} className="flex min-h-0 flex-1"><div className="flex min-w-0 flex-col" style={{ width: `${splitPct}%` }}>{editorPane}</div><div className="flex w-2 shrink-0 cursor-col-resize touch-none items-center justify-center bg-white/5 transition hover:bg-white/15" onPointerDown={startSplitDrag} title="拖拽调整宽度"><GripVertical className="h-3 w-3 text-slate-500" /></div><div className="min-w-0 flex-1 bg-slate-950/40">{previewPane}</div></div> : mode === "edit" ? editorPane : previewPane}
        <div className="flex shrink-0 justify-end gap-2 border-t border-white/10 px-3 py-2.5">
          <button type="button" className={taskButton} onClick={onClose}>取消</button>
          <button type="button" className="nodrag nopan inline-flex items-center gap-1 rounded-md px-3 py-1.5 text-[10px] font-semibold text-white" style={{ backgroundColor: color }} onClick={save}>保存</button>
        </div>
      </div>
    </div>,
    document.body
  );
}

const TaskFlowNode = memo(function TaskFlowNode({ data }: NodeProps<Node<TaskFlowNodeData>>) {
  const { task, selected, collapsed, hasChildren } = data;
  const [editingTitle, setEditingTitle] = useState(false);
  const [detailOpen, setDetailOpen] = useState(false);
  const [bodyCollapsed, setBodyCollapsed] = useState(false);
  const color = taskColor(task);
  const [titleDraft, setTitleDraft] = useState(task.title);
  useEffect(() => { setTitleDraft(task.title); }, [task.title]);
  const saveTitle = () => { const title = titleDraft.trim(); if (title && title !== task.title) data.onUpdate({ title }); setEditingTitle(false); };
  return <><article className={`relative flex w-[320px] flex-col overflow-hidden rounded-lg border bg-[#101827] shadow-2xl`} style={{ height: bodyCollapsed ? undefined : TASK_NODE_HEIGHT, borderColor: selected ? color : `${color}88`, boxShadow: selected ? `0 0 22px ${hexToRgba(color, 0.35)}` : "0 18px 40px rgba(0,0,0,.5)" }} onClick={data.onSelect} onDoubleClick={(event) => { event.stopPropagation(); setDetailOpen(true); }}>
    <Handle type="target" position={Position.Left} isConnectable className="!h-3 !w-3 !border-2 !border-slate-950" style={{ background: color }} />
    <header className="flex h-10 cursor-grab items-center gap-1.5 border-b border-white/10 px-2.5" style={{ backgroundColor: hexToRgba(color, 0.12) }}>
      <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color, boxShadow: `0 0 9px ${color}` }} />
      {editingTitle ? <input autoFocus value={titleDraft} onChange={(event) => setTitleDraft(event.target.value)} onBlur={saveTitle} onKeyDown={(event) => { if (event.key === "Enter") saveTitle(); if (event.key === "Escape") { setTitleDraft(task.title); setEditingTitle(false); } }} className="nodrag nopan min-w-0 flex-1 rounded border border-white/20 bg-slate-950 px-1.5 py-1 text-[11px] text-white outline-none" /> : <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-slate-100">{task.title}</span>}
      <input type="color" value={color} onChange={(event) => data.onUpdate({ color: event.target.value })} className="nodrag nopan h-5 w-5 cursor-pointer rounded border-0 bg-transparent p-0" title="设置节点颜色" />
      <button type="button" className="nodrag nopan text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); setEditingTitle(true); }} title="编辑标题"><Pencil className="h-3 w-3" /></button>
      {hasChildren && <button type="button" className="nodrag nopan text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); data.onToggle(); }} title={collapsed ? "展开子任务" : "折叠子任务"}>{collapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}</button>}
      <button type="button" className="nodrag nopan text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); setBodyCollapsed((v) => !v); }} title={bodyCollapsed ? "展开节点" : "收起节点"}>{bodyCollapsed ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}</button>
    </header>
    {!bodyCollapsed && <div className="nodrag nopan flex flex-1 flex-col gap-1.5 p-2.5">
      <div className="flex items-center justify-between text-[9px] text-slate-500"><span className="font-medium text-slate-300" style={{ color }}>{STATUS_META[taskStatus(task)].label}</span><span className="font-mono font-semibold" style={{ color }}>{task.progress}%</span></div>
      <input type="range" min={0} max={100} step={5} value={task.progress} onChange={(event) => data.onProgress(Number(event.target.value))} className="nodrag nopan h-1.5 w-full cursor-pointer accent-current" style={{ color }} title="拖动调整进度" />
      <div className="mt-0.5 flex gap-1.5">
        <button type="button" className="nodrag nopan flex flex-1 items-center justify-center gap-1.5 rounded border border-white/10 bg-white/[0.04] py-1.5 text-[10px] text-slate-400 transition hover:bg-white/[0.1] hover:text-white" style={{ borderColor: `${color}55`, color: `${color}cc` }} onClick={(event) => { event.stopPropagation(); setDetailOpen(true); }}><Maximize2 className="h-3.5 w-3.5" />编辑详细内容{task.detail ? `（${task.detail.length} 字符）` : ""}</button>
        {selected && <button type="button" className="nodrag nopan inline-flex h-[26px] w-[34px] shrink-0 items-center justify-center rounded border border-red-400/30 text-red-300 transition hover:bg-red-400/10 hover:text-red-200" onClick={(event) => { event.stopPropagation(); data.onDelete(); }} title="删除任务"><Trash2 className="h-3.5 w-3.5" /></button>}
      </div>
    </div>}
    <Handle type="source" position={Position.Right} isConnectable className="!h-3 !w-3 !border-2 !border-slate-950" style={{ background: color }} />
  </article>
  <button type="button" className="nodrag nopan absolute z-10 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-full border-2 border-dashed text-sm font-bold leading-none transition hover:scale-110 hover:border-solid" style={{ right: -38, top: "50%", borderColor: `${color}88`, color: `${color}cc`, backgroundColor: "#101827", boxShadow: "0 0 6px rgba(0,0,0,.45)" }} onClick={(event) => { event.stopPropagation(); data.onAddChild(); }} title="添加子任务"><Plus className="h-3.5 w-3.5" /></button>
  {detailOpen && <TaskDetailModal task={task} onClose={() => setDetailOpen(false)} onUpdate={data.onUpdate} onInsertFile={data.onInsertFile} onInsertImage={data.onInsertImage} onInsertScreenshot={data.onInsertScreenshot} onOpenFile={data.onOpenFile} />}
</>;
});

// ─── 贴纸节点（白板便签）───

type StickerNodeData = {
  sticker: TaskSticker;
  onUpdate: (patch: { content?: string; color?: string; positionX?: number; positionY?: number }) => void;
  onDelete: () => void;
};

const STICKER_PALETTE = ["#fef3c7", "#d4f5d4", "#dbeafe", "#fce7f3", "#ede9fe", "#ffedd5", "#e0e7ff"];

const StickerFlowNode = memo(function StickerFlowNode({ data }: NodeProps<Node<StickerNodeData>>) {
  const { sticker } = data;
  const [editing, setEditing] = useState(!sticker.content);
  const [draft, setDraft] = useState(sticker.content);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const bg = hexToRgba(sticker.color, 0.92);
  useEffect(() => { setDraft(sticker.content); }, [sticker.content]);
  const save = () => { if (draft !== sticker.content) data.onUpdate({ content: draft }); setEditing(false); };
  return <>
    <div className={`relative min-w-[200px] max-w-[320px] rounded px-3 py-2 shadow-lg`} style={{ backgroundColor: bg, borderColor: sticker.color, borderWidth: 1, color: "#1e1b4b", fontSize: 12, lineHeight: 1.5, fontFamily: "'Segoe UI', system-ui, sans-serif" }} onDoubleClick={() => setEditing(true)}>
      {editing ? <textarea autoFocus value={draft} onChange={(e) => setDraft(e.target.value)} onBlur={save} onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); save(); } if (e.key === "Escape") { setDraft(sticker.content); setEditing(false); } }} placeholder="写点什么…" className="nodrag nopan min-w-0 resize-none border-0 bg-transparent text-[13px] leading-relaxed outline-none placeholder:text-black/25" rows={Math.max(1, draft.split(/\r?\n/).length)} style={{ color: "#1e1b4b", fontFamily: "'Segoe UI', system-ui, sans-serif", width: "100%" }} /> : <div className="min-h-[1.5em] cursor-text whitespace-pre-wrap break-words text-[13px] leading-relaxed">{sticker.content || <span className="italic text-black/30">双击编辑…</span>}</div>}
      <div className="mt-1.5 flex items-center gap-1 border-t border-black/10 pt-1.5">
        <div className="relative">
          <button type="button" className="nodrag nopan h-4 w-4 rounded-full border border-black/20" style={{ backgroundColor: sticker.color }} onClick={(e) => { e.stopPropagation(); setPaletteOpen((v) => !v); }} title="换颜色" />
          {paletteOpen && <div className="nodrag nopan absolute bottom-full left-0 z-20 mb-1 flex gap-1 rounded bg-slate-900 p-1 shadow-xl" onMouseLeave={() => setPaletteOpen(false)}>{STICKER_PALETTE.map((c) => <button key={c} type="button" className="h-5 w-5 rounded-full border border-white/20 transition hover:scale-110" style={{ backgroundColor: c }} onClick={(e) => { e.stopPropagation(); data.onUpdate({ color: c }); setPaletteOpen(false); }} />)}</div>}
        </div>
        <button type="button" className="nodrag nopan ml-auto rounded p-0.5 text-black/30 hover:bg-black/10 hover:text-red-600" onClick={(e) => { e.stopPropagation(); data.onDelete(); }} title="删除贴纸"><Trash2 className="h-3 w-3" /></button>
      </div>
    </div>
    <Handle type="source" position={Position.Right} isConnectable={false} style={{ visibility: "hidden" }} />
    <Handle type="target" position={Position.Left} isConnectable={false} style={{ visibility: "hidden" }} />
  </>;
});

const taskNodeTypes = { taskNode: TaskFlowNode, stickerNode: StickerFlowNode };
const edgeTypes = { color: ColorEdge };

function TaskFlowInner(props: TaskFlowProps) {
  const handleConnect = useCallback((connection: { source: string | null; target: string | null }) => { if (connection.source && connection.target && connection.source !== connection.target) props.onConnect(connection.source, connection.target); }, [props.onConnect]);
  const { fitView } = useReactFlow();
  const [nodes, setNodes] = useState<Node<TaskFlowNodeData | StickerNodeData>[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const taskMap = useMemo(() => new globalThis.Map(props.tasks.map((task) => [task.id, task])), [props.tasks]);
  const visibleTasks = useMemo(() => { const result: TaskItem[] = []; const visit = (task: TaskItem) => { result.push(task); if (!collapsed.has(task.id)) props.tasks.filter((child) => child.parentId === task.id).sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt.localeCompare(b.createdAt)).forEach(visit); }; const root = props.selectedSeries ? taskMap.get(props.selectedSeries) : undefined; if (root) visit(root); return result; }, [collapsed, props.selectedSeries, props.tasks, taskMap]);
  const taskNodes = useMemo(() => visibleTasks.map((task, index) => ({ id: task.id, type: "taskNode", position: { x: task.positionX !== 0 || task.positionY !== 0 ? task.positionX : task.parentId ? 420 : 60, y: task.positionX !== 0 || task.positionY !== 0 ? task.positionY : index * 90 }, data: { task, selected: props.selectedTaskId === task.id, collapsed: collapsed.has(task.id), hasChildren: props.tasks.some((child) => child.parentId === task.id), onSelect: () => props.onSelect(task.id), onToggle: () => setCollapsed((current) => { const next = new Set(current); next.has(task.id) ? next.delete(task.id) : next.add(task.id); return next; }), onAddChild: () => props.onAddChild(task.id), onDelete: () => props.onDelete(task), onProgress: (progress: number) => props.onProgress(task, progress), onUpdate: (patch: UpdateTaskInput) => props.onUpdate(task.id, patch), onInsertFile: (content: string) => props.onInsertFile(task.id, content), onInsertImage: (content: string) => props.onInsertImage(task.id, content), onInsertScreenshot: (content: string) => props.onInsertScreenshot(task.id, content), onOpenFile: props.onOpenFile } })), [collapsed, props]);
  const taskEdges = useMemo<Edge[]>(() => visibleTasks.flatMap((task) => task.parentId && visibleTasks.some((parent) => parent.id === task.parentId) ? [{ id: `task-edge-${task.id}`, source: task.parentId, target: task.id, type: "color", data: { color: taskColor(task) }, animated: props.selectedTaskId === task.id } as Edge] : []), [props.selectedTaskId, visibleTasks]);

  const stickerNodes = useMemo<Node<StickerNodeData>[]>(() => props.stickers.map((s) => ({
    id: `sticker-${s.id}`,
    type: "stickerNode",
    position: { x: s.positionX, y: s.positionY },
    draggable: true,
    data: {
      sticker: s,
      onUpdate: (patch) => props.onUpdateSticker(s.id, patch),
      onDelete: () => props.onDeleteSticker(s.id),
    },
  })), [props.stickers, props.onUpdateSticker, props.onDeleteSticker]);

  useEffect(() => { setNodes((current) => { const currentById = new globalThis.Map(current.map((node) => [node.id, node])); return [
      ...taskNodes.map((next) => { const existing = currentById.get(next.id); return existing ? { ...next, position: existing.position, dragging: existing.dragging } : next; }),
      ...stickerNodes.map((next) => { const existing = currentById.get(next.id); return existing ? { ...next, position: existing.position, dragging: existing.dragging } : next; }),
    ]; }); }, [taskNodes, stickerNodes]);
  useEffect(() => { const timer = window.setTimeout(() => fitView({ padding: 0.18, duration: 260 }), 0); return () => window.clearTimeout(timer); }, [fitView, props.selectedSeries, collapsed]);
  return <ReactFlow nodes={nodes} edges={taskEdges} nodeTypes={taskNodeTypes} edgeTypes={edgeTypes} onNodesChange={(changes) => setNodes((current) => applyNodeChanges(changes, current))} onNodeDragStop={(_, node) => { if (node.id.startsWith("sticker-")) { props.onStickerPositionChange(node.id.slice(8), node.position); } else { props.onPositionChange(node.id, node.position); } }} onConnect={handleConnect} fitView minZoom={0.1} maxZoom={2} nodesConnectable connectionRadius={28} proOptions={{ hideAttribution: true }} onPaneClick={(event) => { if (event.detail === 2) { const bounds = (event.target as HTMLElement).closest(".react-flow__pane")?.getBoundingClientRect(); if (bounds) props.onAddSticker(event.clientX - bounds.left, event.clientY - bounds.top); } }}><Background color="#1e293b" gap={24} size={1} /><MiniMap style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95" nodeColor={(node) => node.id.startsWith("sticker-") ? (node.data as StickerNodeData).sticker.color : taskColor((node.data as TaskFlowNodeData).task)} nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2, 6, 23, 0.72)" pannable zoomable /><Controls className="canvas-flow-controls" showInteractive={false} /></ReactFlow>;
}

export type TaskFlowProps = {
  tasks: TaskItem[];
  stickers: TaskSticker[];
  selectedSeries: string | null;
  selectedTaskId: string | null;
  onSelect: (id: string) => void;
  onAddChild: (id: string) => void;
  onDelete: (task: TaskItem) => void;
  onProgress: (task: TaskItem, progress: number) => void;
  onUpdate: (id: string, patch: UpdateTaskInput) => void;
  onInsertFile: (id: string, content: string) => Promise<string | undefined>;
  onInsertImage: (id: string, content: string) => Promise<string | undefined>;
  onInsertScreenshot: (id: string, content: string) => Promise<string | undefined>;
  onOpenFile: (path: string) => void;
  onPositionChange: (id: string, position: { x: number; y: number }) => void;
  onConnect: (parentId: string, childId: string) => void;
  onAddSticker: (x: number, y: number) => void;
  onUpdateSticker: (id: string, patch: { content?: string; color?: string; positionX?: number; positionY?: number }) => void;
  onDeleteSticker: (id: string) => void;
  onStickerPositionChange: (id: string, position: { x: number; y: number }) => void;
};

export function TaskFlowCanvas(props: TaskFlowProps) {
  return <div className="h-full min-h-0 bg-[#080f1c]"><ReactFlowProvider><TaskFlowInner {...props} /></ReactFlowProvider></div>;
}
