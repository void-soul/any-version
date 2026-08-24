import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  CalendarDays,
  ChevronDown,
  ChevronRight,
  CirclePlus,
  Eye,
  FilePlus2,
  Image as ImageIcon,
  Pencil,
  Trash2,
  Type,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { JsonValue, SearchMatches } from "./SystemTools/JsonBrowser";
import { TaskItem, TaskStatus, STATUS_META, UpdateTaskInput, deriveStatus } from "./tasks/types";
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
const TASK_NODE_HEIGHT = 470;
const taskInputClass = "w-full rounded-md border border-white/10 bg-slate-950/85 px-2.5 py-2 text-[10px] leading-relaxed text-slate-100 outline-none placeholder:text-slate-600 focus:border-cyan-400/60 focus:bg-slate-950";
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
  const allItems = useMemo(() => buildJsonItems(value), [value]);
  const graphTruncated = allItems.length >= MAX_JSON_FLOW_ITEMS;
  const previousCollapseToken = useRef(0);
  useEffect(() => {
    if (collapseAllToken === previousCollapseToken.current) return;
    setCollapsed(new Set(allItems.filter((item) => isJsonContainer(item.value)).map((item) => item.path)));
    previousCollapseToken.current = collapseAllToken;
  }, [allItems, collapseAllToken]);
  const visibleItems = useMemo(() => allItems.filter((item) => { let current = item.parentId; while (current) { if (collapsed.has(current)) return false; current = allItems.find((candidate) => candidate.id === current)?.parentId ?? null; } return true; }), [allItems, collapsed]);
  const nodes = useMemo<Node<JsonFlowNodeData>[]>(() => visibleItems.map((item) => ({ id: item.id, type: "jsonNode", position: { x: item.depth * 260, y: item.depth === 0 ? 0 : visibleItems.filter((candidate) => candidate.depth === item.depth && candidate.id <= item.id).length * 92 }, data: { item, selectedPath, searchMatches, collapsed, onSelect: onSelectPath, onToggle: (path) => setCollapsed((current) => { const next = new Set(current); next.has(path) ? next.delete(path) : next.add(path); return next; }), onCopy }, sourcePosition: Position.Right, targetPosition: Position.Left })), [onCopy, onSelectPath, searchMatches, selectedPath, visibleItems, collapsed]);
  const edges = useMemo<Edge[]>(() => visibleItems.flatMap((item) => !item.parentId || !visibleItems.some((candidate) => candidate.id === item.parentId) ? [] : [{ id: `json-edge-${item.id}`, source: item.parentId, target: item.id, type: "color", data: { color: hashColor(item.id, JSON_EDGE_COLORS) }, markerEnd: { type: MarkerType.ArrowClosed, color: "#f8fafc" } }]), [visibleItems]);
  useEffect(() => { const timer = window.setTimeout(() => fitView({ padding: 0.2, duration: 240 }), 0); return () => window.clearTimeout(timer); }, [fitView, value, collapsed]);
  return <div className="relative h-full min-h-0"><ReactFlow nodes={nodes} edges={edges} nodeTypes={{ jsonNode: JsonFlowNode }} edgeTypes={{ color: ColorEdge }} fitView minZoom={0.15} maxZoom={2.2} nodesDraggable={false} nodesConnectable={false} elementsSelectable proOptions={{ hideAttribution: true }}><Background color="#1e293b" gap={24} size={1} /><MiniMap style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95" nodeColor={(node) => hashColor(String(node.id), JSON_EDGE_COLORS)} nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2, 6, 23, 0.72)" pannable zoomable /><Controls className="canvas-flow-controls" showInteractive={false} /></ReactFlow>{graphTruncated && <div className="pointer-events-none absolute left-3 top-3 z-10 rounded border border-amber-400/20 bg-slate-900/90 px-2 py-1 text-[10px] text-amber-200">图形树仅显示前 {MAX_JSON_FLOW_ITEMS} 个节点</div>}</div>;
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
function isImagePath(value: string): boolean { return /\.(?:png|jpe?g|gif|webp|bmp|svg)$/i.test(value.split(/[?#]/)[0]); }
function localImageSrc(path: string): string { try { return convertFileSrc(path); } catch { return path; } }
function appendMarkdownLine(content: string, line: string): string { return content.trimEnd() ? `${content.trimEnd()}\n\n${line}` : line; }

function TaskMarkdown({ content, onOpenFile }: { content: string; onOpenFile: (path: string) => void }) {
  return <div className="task-markdown text-[10px] leading-relaxed text-slate-200 break-words">
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
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
        return <a href={local ? undefined : target} target={local ? undefined : "_blank"} rel={local ? undefined : "noopener noreferrer"} onClick={(event) => { if (local) { event.preventDefault(); onOpenFile(target); } }} className="text-cyan-300 underline decoration-cyan-400/40 underline-offset-2 hover:text-cyan-100">{children}</a>;
      },
      img: ({ src, alt }) => {
        const target = typeof src === "string" ? src : "";
        return <button type="button" className="my-2 block max-w-full text-left" onClick={() => isLocalFilePath(target) && onOpenFile(target)} title="打开图片"><img src={isLocalFilePath(target) ? localImageSrc(target) : target} alt={alt ?? ""} className="max-h-48 max-w-full rounded border border-white/10 object-contain" /></button>;
      },
      table: ({ children }) => <div className="my-2 overflow-x-auto rounded border border-white/10"><table className="min-w-full text-[9px]">{children}</table></div>,
      thead: ({ children }) => <thead className="bg-slate-800/80">{children}</thead>,
      th: ({ children }) => <th className="border-b border-white/10 px-1.5 py-1 text-left text-cyan-200">{children}</th>,
      td: ({ children }) => <td className="border-b border-white/5 px-1.5 py-1 text-slate-300">{children}</td>,
      hr: () => <hr className="my-2 border-white/10" />,
    }}>{content || "暂无内容"}</ReactMarkdown>
  </div>;
}

const TaskFlowNode = memo(function TaskFlowNode({ data }: NodeProps<Node<TaskFlowNodeData>>) {
  const { task, selected, collapsed, hasChildren } = data;
  const [editingTitle, setEditingTitle] = useState(false);
  const [markdownDraft, setMarkdownDraft] = useState(task.description);
  const [markdownMode, setMarkdownMode] = useState<"edit" | "preview">("edit");
  const color = taskColor(task);
  useEffect(() => { setMarkdownDraft(task.description); }, [task.description]);
  useEffect(() => { if (!selected) setMarkdownMode("edit"); }, [selected]);
  const [titleDraft, setTitleDraft] = useState(task.title);
  useEffect(() => { setTitleDraft(task.title); }, [task.title]);
  const saveTitle = () => { const title = titleDraft.trim(); if (title && title !== task.title) data.onUpdate({ title }); setEditingTitle(false); };
  const saveMarkdown = () => { if (markdownDraft !== task.description) data.onUpdate({ description: markdownDraft }); };
  const insertWith = async (handler: (content: string) => Promise<string | undefined>) => { const next = await handler(markdownDraft); if (next !== undefined) setMarkdownDraft(next); };
  const insertDate = () => { const now = new Date(); const date = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`; const next = appendMarkdownLine(markdownDraft, `**日期：${date}**`); setMarkdownDraft(next); data.onUpdate({ description: next }); };
  return <article className={`relative w-[320px] overflow-hidden rounded-lg border bg-[#101827] shadow-2xl`} style={{ height: TASK_NODE_HEIGHT, borderColor: selected ? color : `${color}88`, boxShadow: selected ? `0 0 22px ${hexToRgba(color, 0.35)}` : "0 18px 40px rgba(0,0,0,.5)" }} onClick={data.onSelect}>
    <Handle type="target" position={Position.Left} isConnectable className="!h-3 !w-3 !border-2 !border-slate-950" style={{ background: color }} />
    <header className="flex h-10 cursor-grab items-center gap-1.5 border-b border-white/10 px-2.5" style={{ backgroundColor: hexToRgba(color, 0.12) }}>
      <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color, boxShadow: `0 0 9px ${color}` }} />
      {editingTitle ? <input autoFocus value={titleDraft} onChange={(event) => setTitleDraft(event.target.value)} onBlur={saveTitle} onKeyDown={(event) => { if (event.key === "Enter") saveTitle(); if (event.key === "Escape") { setTitleDraft(task.title); setEditingTitle(false); } }} className="nodrag nopan min-w-0 flex-1 rounded border border-white/20 bg-slate-950 px-1.5 py-1 text-[11px] text-white outline-none" /> : <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-slate-100">{task.title}</span>}
      <input type="color" value={color} onChange={(event) => data.onUpdate({ color: event.target.value })} className="nodrag nopan h-5 w-5 cursor-pointer rounded border-0 bg-transparent p-0" title="设置节点颜色" />
      <button type="button" className="nodrag nopan text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); setEditingTitle(true); }} title="编辑标题"><Pencil className="h-3 w-3" /></button>
      {hasChildren && <button type="button" className="nodrag nopan text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); data.onToggle(); }} title={collapsed ? "展开子任务" : "折叠子任务"}>{collapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}</button>}
    </header>      <div className="nodrag nopan h-[430px] overflow-y-auto p-2.5">
      <div className="mb-2 flex items-center justify-between text-[9px] text-slate-500"><span className="font-medium text-slate-300" style={{ color }}>{STATUS_META[taskStatus(task)].label}</span><span className="font-mono font-semibold" style={{ color }}>{task.progress}%</span></div>
      <div className="mb-2 h-1.5 rounded-full bg-white/10"><div className="h-full rounded-full transition-[width]" style={{ width: `${task.progress}%`, backgroundColor: color, boxShadow: `0 0 8px ${hexToRgba(color, 0.6)}` }} /></div>
      <div className="mb-3 grid grid-cols-5 gap-1">{([0, 25, 50, 75, 100] as const).map((progress) => <button type="button" key={progress} onClick={() => data.onProgress(progress)} className={`rounded py-1 text-[9px] ${task.progress === progress ? "text-slate-950" : "bg-white/[0.05] text-slate-500 hover:bg-white/[0.1]"}`} style={task.progress === progress ? { backgroundColor: color } : undefined}>{progress}%</button>)}</div>
      <div className="mb-2 flex items-center justify-between border-t border-white/10 pt-2"><span className="flex items-center gap-1 text-[10px] font-semibold text-slate-300"><Type className="h-3.5 w-3.5" style={{ color }} />任务内容</span><div className="flex items-center gap-1"><button type="button" className="nodrag nopan inline-flex items-center gap-1 rounded px-1.5 py-1 text-[9px] text-slate-500 hover:bg-white/[0.06] hover:text-white" style={{ color: `${color}cc` }} onClick={() => setMarkdownMode("edit")} title="编辑 Markdown"><Pencil className="h-3 w-3" />编辑</button><button type="button" className="nodrag nopan inline-flex items-center gap-1 rounded px-1.5 py-1 text-[9px] text-slate-500 hover:bg-white/[0.06] hover:text-white" style={{ color: `${color}cc` }} onClick={() => { saveMarkdown(); setMarkdownMode("preview"); }} title="渲染 Markdown"><Eye className="h-3 w-3" />渲染</button></div></div>
      {markdownMode === "edit" ? <><textarea value={markdownDraft} onChange={(event) => setMarkdownDraft(event.target.value)} onBlur={saveMarkdown} rows={13} placeholder="使用 Markdown 记录任务的全部内容…" className={`${taskInputClass} nodrag nopan min-h-[250px] resize-y font-mono`} /><div className="mt-1.5 flex flex-wrap items-center gap-1"><button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => void insertWith(data.onInsertFile)} title="插入文件路径"><FilePlus2 className="h-3 w-3" />文件</button><button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => void insertWith(data.onInsertImage)} title="插入图片"><ImageIcon className="h-3 w-3" />图片</button><button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={() => void insertWith(data.onInsertScreenshot)} title="插入剪贴板截图"><ImageIcon className="h-3 w-3" />截图</button><button type="button" className={`${taskIconButton} nodrag nopan`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={insertDate} title="插入当前日期"><CalendarDays className="h-3 w-3" />日期</button></div></> : <div className="min-h-[280px] rounded-md border border-white/10 bg-slate-950/60 px-2.5 py-2"><TaskMarkdown content={markdownDraft} onOpenFile={data.onOpenFile} /></div>}
      <div className="mt-2 flex gap-1 border-t border-white/10 pt-2"><button type="button" className={`${taskButton} nodrag nopan flex-1 justify-center`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={data.onAddChild}><CirclePlus className="h-3 w-3" />子任务</button>{selected && <button type="button" className={`${taskButton} nodrag nopan text-red-300`} style={{ borderColor: `${color}55`, color: `${color}ee` }} onClick={data.onDelete}><Trash2 className="h-3 w-3" />删除</button>}</div>
    </div>
    <Handle type="source" position={Position.Right} isConnectable className="!h-3 !w-3 !border-2 !border-slate-950" style={{ background: color }} />
  </article>;
});

const taskNodeTypes = { taskNode: TaskFlowNode };
const edgeTypes = { color: ColorEdge };

function TaskFlowInner(props: TaskFlowProps) {
  const handleConnect = useCallback((connection: { source: string | null; target: string | null }) => { if (connection.source && connection.target && connection.source !== connection.target) props.onConnect(connection.source, connection.target); }, [props.onConnect]);
  const { fitView } = useReactFlow();
  const [nodes, setNodes] = useState<Node<TaskFlowNodeData>[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const taskMap = useMemo(() => new globalThis.Map(props.tasks.map((task) => [task.id, task])), [props.tasks]);
  const visibleTasks = useMemo(() => { const result: TaskItem[] = []; const visit = (task: TaskItem) => { result.push(task); if (!collapsed.has(task.id)) props.tasks.filter((child) => child.parentId === task.id).sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt.localeCompare(b.createdAt)).forEach(visit); }; const root = props.selectedSeries ? taskMap.get(props.selectedSeries) : undefined; if (root) visit(root); return result; }, [collapsed, props.selectedSeries, props.tasks, taskMap]);
  const taskNodes = useMemo(() => visibleTasks.map((task, index) => ({ id: task.id, type: "taskNode", position: { x: task.positionX !== 0 || task.positionY !== 0 ? task.positionX : task.parentId ? 420 : 60, y: task.positionX !== 0 || task.positionY !== 0 ? task.positionY : index * 90 }, data: { task, selected: props.selectedTaskId === task.id, collapsed: collapsed.has(task.id), hasChildren: props.tasks.some((child) => child.parentId === task.id), onSelect: () => props.onSelect(task.id), onToggle: () => setCollapsed((current) => { const next = new Set(current); next.has(task.id) ? next.delete(task.id) : next.add(task.id); return next; }), onAddChild: () => props.onAddChild(task.id), onDelete: () => props.onDelete(task), onProgress: (progress: number) => props.onProgress(task, progress), onUpdate: (patch: UpdateTaskInput) => props.onUpdate(task.id, patch), onInsertFile: (content: string) => props.onInsertFile(task.id, content), onInsertImage: (content: string) => props.onInsertImage(task.id, content), onInsertScreenshot: (content: string) => props.onInsertScreenshot(task.id, content), onOpenFile: props.onOpenFile } })), [collapsed, props]);
  const taskEdges = useMemo<Edge[]>(() => visibleTasks.flatMap((task) => task.parentId && visibleTasks.some((parent) => parent.id === task.parentId) ? [{ id: `task-edge-${task.id}`, source: task.parentId, target: task.id, type: "color", data: { color: taskColor(task) }, animated: props.selectedTaskId === task.id } as Edge] : []), [props.selectedTaskId, visibleTasks]);
  useEffect(() => { setNodes((current) => { const currentById = new globalThis.Map(current.map((node) => [node.id, node])); return taskNodes.map((next) => { const existing = currentById.get(next.id); return existing ? { ...next, position: existing.position, dragging: existing.dragging } : next; }); }); }, [taskNodes]);
  useEffect(() => { const timer = window.setTimeout(() => fitView({ padding: 0.18, duration: 260 }), 0); return () => window.clearTimeout(timer); }, [fitView, props.selectedSeries, collapsed]);
  return <ReactFlow nodes={nodes} edges={taskEdges} nodeTypes={taskNodeTypes} edgeTypes={edgeTypes} onNodesChange={(changes) => setNodes((current) => applyNodeChanges(changes, current))} onNodeDragStop={(_, node) => props.onPositionChange(node.id, node.position)} onConnect={handleConnect} fitView minZoom={0.1} maxZoom={2} nodesConnectable connectionRadius={28} proOptions={{ hideAttribution: true }}><Background color="#1e293b" gap={24} size={1} /><MiniMap style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95" nodeColor={(node) => taskColor((node.data as TaskFlowNodeData).task)} nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2, 6, 23, 0.72)" pannable zoomable /><Controls className="canvas-flow-controls" showInteractive={false} /></ReactFlow>;
}

export type TaskFlowProps = {
  tasks: TaskItem[];
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
};

export function TaskFlowCanvas(props: TaskFlowProps) {
  return <div className="h-full min-h-0 bg-[#080f1c]"><ReactFlowProvider><TaskFlowInner {...props} /></ReactFlowProvider></div>;
}
