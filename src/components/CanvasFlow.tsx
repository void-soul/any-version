import React, { memo, useCallback, useEffect, useMemo, useState } from "react";
import {
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
import {
  Check,
  ChevronDown,
  ChevronRight,
  CirclePlus,
  FileText,
  FolderOpen,
  Image as ImageIcon,
  Link2,
  Pencil,
  Trash2,
  X,
} from "lucide-react";
import type { JsonValue, SearchMatches } from "./SystemTools/JsonBrowser";
import {
  TaskItem,
  TaskLog,
  TaskLogReferenceInput,
  TaskStatus,
  STATUS_META,
  UpdateTaskInput,
  deriveStatus,
} from "./tasks/types";

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
  logs: TaskLog[];
  picky: PickyBookmark[];
  draftLog: LogDraft;
  busy: boolean;
  onSelect: () => void;
  onToggle: () => void;
  onAddChild: () => void;
  onDelete: () => void;
  onProgress: (progress: number) => void;
  onUpdate: (patch: UpdateTaskInput) => void;
  onAddLog: () => void;
  onAddReference: (kind: "file" | "image") => void;
  onAddPickyReference: (bookmark: PickyBookmark) => void;
  onRemoveDraftReference: (index: number) => void;
  onDraftLogChange: (patch: Partial<LogDraft>) => void;
  onOpenReference: (reference: TaskLog["references"][number]) => void;
};

type PickyBookmark = { id: string; title: string; url?: string | null };
type LogDraft = { content: string; minutes: string; references: TaskLogReferenceInput[] };
type TaskFlowNode = TaskItem & { x: number; y: number; childrenCount: number };

const JSON_EDGE_COLORS = ["#22d3ee", "#a78bfa", "#34d399", "#fbbf24", "#fb7185", "#60a5fa"];
const TASK_EDGE_COLORS = ["#22d3ee", "#a78bfa", "#34d399", "#fbbf24", "#fb7185", "#60a5fa"];
const TASK_NODE_WIDTH = 320;
const TASK_NODE_HEIGHT = 410;
const taskInputClass = "w-full rounded border border-white/10 bg-slate-950/70 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-amber-400/60";
const taskButton = "inline-flex items-center gap-1 rounded border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";
const taskIconButton = "inline-flex h-7 w-7 items-center justify-center rounded border border-white/10 bg-white/[0.04] text-slate-400 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";

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

function jsonSummary(value: JsonValue): string {
  const type = jsonType(value);
  if (Array.isArray(value)) return `array [${value.length}]`;
  if (typeof value === "object" && value !== null) return `object {${Object.keys(value).length}}`;
  if (typeof value === "string") return `"${value.length > 32 ? `${value.slice(0, 29)}...` : value}"`;
  return `${type}: ${String(value)}`;
}

function buildJsonItems(value: JsonValue): JsonGraphItem[] {
  const result: JsonGraphItem[] = [];
  const visit = (name: string, current: JsonValue, path: string, parentId: string | null, depth: number) => {
    const id = path || "root";
    result.push({ id, name, path: id, value: current, parentId, depth });
    if (isJsonContainer(current)) {
      jsonEntries(current).forEach(([childName, child]) => visit(childName, child, `${path}.${childName}`, id, depth + 1));
    }
  };
  visit("root", value, "root", null, 0);
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
    <div className={`min-w-[170px] max-w-[260px] rounded-lg border bg-[#101827] px-2.5 py-2 shadow-xl ${selected ? "border-cyan-300 shadow-cyan-500/30" : chain ? "border-cyan-700/80" : "border-white/10"}`} onClick={() => onSelect(item.path)}>
      <Handle type="target" position={Position.Left} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
      <div className="flex items-center gap-1.5">
        {container && <button type="button" className="nodrag nopan inline-flex h-4 w-4 items-center justify-center text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); onToggle(item.path); }} title={collapsed.has(item.path) ? "展开" : "折叠"}>{collapsed.has(item.path) ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}</button>}
        <span className={`min-w-0 flex-1 truncate font-mono text-[11px] ${directMatch ? "text-yellow-300" : "text-cyan-200"}`}>{item.name}</span>
        <button type="button" className="nodrag nopan text-slate-600 hover:text-white" onClick={(event) => { event.stopPropagation(); onCopy(copyJsonValue(item.value)); }} title="复制节点内容"><span className="text-[10px]">⧉</span></button>
      </div>
      <div className="mt-1 truncate font-mono text-[10px]" style={{ color }}>{jsonSummary(item.value)}</div>
      <Handle type="source" position={Position.Right} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
    </div>
  );
});

const ColorEdge = memo(function ColorEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, selected, data }: EdgeProps) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, curvature: 0.28 });
  const color = data?.color as string | undefined ?? "#22d3ee";
  const gradientId = `flow-edge-${id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  return (
    <>
      <defs><linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="0%"><stop offset="0%" stopColor={color} /><stop offset="100%" stopColor="#f8fafc" /></linearGradient></defs>
      <path d={path} fill="none" stroke={color} strokeWidth={selected ? 8 : 5} opacity={selected ? 0.2 : 0.12} />
      <path d={path} fill="none" stroke={`url(#${gradientId})`} strokeWidth={selected ? 3.5 : 2.2} strokeLinecap="round" markerEnd="url(#arrow-${gradientId})" />
      <marker id={`arrow-${gradientId}`} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#f8fafc" /></marker>
    </>
  );
});

function JsonFlowInner({ value, selectedPath, searchMatches, onSelectPath, onCopy }: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void }) {
  const { fitView } = useReactFlow();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const allItems = useMemo(() => buildJsonItems(value), [value]);
  const visibleItems = useMemo(() => allItems.filter((item) => {
    let current = item.parentId;
    while (current) {
      if (collapsed.has(current)) return false;
      current = allItems.find((candidate) => candidate.id === current)?.parentId ?? null;
    }
    return true;
  }), [allItems, collapsed]);
  const nodes = useMemo<Node<JsonFlowNodeData>[]>(() => visibleItems.map((item) => ({ id: item.id, type: "jsonNode", position: { x: item.depth * 260, y: item.depth === 0 ? 0 : visibleItems.filter((candidate) => candidate.depth === item.depth && candidate.id <= item.id).length * 92 }, data: { item, selectedPath, searchMatches, collapsed, onSelect: onSelectPath, onToggle: (path) => setCollapsed((current) => { const next = new Set(current); next.has(path) ? next.delete(path) : next.add(path); return next; }), onCopy }, sourcePosition: Position.Right, targetPosition: Position.Left, hidden: false })), [onCopy, onSelectPath, searchMatches, selectedPath, visibleItems, collapsed]);
  const edges = useMemo<Edge[]>(() => visibleItems.flatMap((item) => {
    if (!item.parentId || !visibleItems.some((candidate) => candidate.id === item.parentId)) return [];
    return [{ id: `json-edge-${item.id}`, source: item.parentId, target: item.id, type: "color", data: { color: hashColor(item.id, JSON_EDGE_COLORS) }, markerEnd: { type: MarkerType.ArrowClosed, color: "#f8fafc" } }];
  }), [visibleItems]);
  useEffect(() => { const timer = window.setTimeout(() => fitView({ padding: 0.2, duration: 240 }), 0); return () => window.clearTimeout(timer); }, [fitView, value, collapsed]);
  return <ReactFlow nodes={nodes} edges={edges} nodeTypes={{ jsonNode: JsonFlowNode }} edgeTypes={{ color: ColorEdge }} fitView minZoom={0.15} maxZoom={2.2} nodesDraggable={false} proOptions={{ hideAttribution: true }}><Background color="#1e293b" gap={24} size={1} /><MiniMap nodeColor={(node) => hashColor(String(node.id), JSON_EDGE_COLORS)} pannable zoomable /><Controls showInteractive={false} /></ReactFlow>;
}

export function JsonFlowCanvas(props: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void }) {
  return <div className="h-full min-h-0 bg-slate-950"><ReactFlowProvider><JsonFlowInner {...props} /></ReactFlowProvider></div>;
}

function taskStatus(task: TaskItem): TaskStatus { return deriveStatus(task.progress); }
function taskColor(task: TaskItem): string { return /^#[0-9a-f]{6}$/i.test(task.color) ? task.color : taskStatus(task) === "done" ? "#34d399" : taskStatus(task) === "inProgress" ? "#fbbf24" : "#64748b"; }

function LogItem({ log, onOpenReference }: { log: TaskLog; onOpenReference: (reference: TaskLog["references"][number]) => void }) {
  return <article className="rounded border border-white/10 bg-black/20 p-2"><div className="flex items-center justify-between text-[9px] text-slate-500"><span>{log.logDate}</span><span>{log.minutesSpent > 0 ? `${log.minutesSpent} 分钟` : ""}</span></div>{log.content && <p className="mt-1 whitespace-pre-wrap text-[10px] leading-relaxed text-slate-300">{log.content}</p>}{log.references.length > 0 && <div className="mt-2 space-y-1">{log.references.map((reference) => <button type="button" key={reference.id} onClick={() => onOpenReference(reference)} className="nodrag nopan flex w-full items-center gap-1.5 rounded bg-white/[0.04] px-2 py-1 text-left text-[9px] text-cyan-300 hover:bg-cyan-400/10"><Link2 className="h-3 w-3 shrink-0" /><span className="truncate">{reference.label || reference.target}</span></button>)}</div>}</article>;
}

const TaskFlowNode = memo(function TaskFlowNode({ data }: NodeProps<Node<TaskFlowNodeData>>) {
  const { task, selected, collapsed, hasChildren, logs, picky, draftLog, busy } = data;
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(task.title);
  const [descriptionDraft, setDescriptionDraft] = useState(task.description);
  const color = taskColor(task);
  useEffect(() => { setTitleDraft(task.title); setDescriptionDraft(task.description); }, [task.description, task.title]);
  const saveTitle = () => { const title = titleDraft.trim(); if (title && title !== task.title) data.onUpdate({ title }); setEditingTitle(false); };
  return <article className={`relative w-[320px] overflow-hidden rounded-lg border bg-[#101827] shadow-2xl ${selected ? "shadow-cyan-500/25" : "shadow-black/30"}`} style={{ height: TASK_NODE_HEIGHT, borderColor: selected ? "#22d3ee" : `${color}88` }} onClick={data.onSelect}>
    <Handle type="target" position={Position.Left} className="!h-3 !w-3 !border-2 !border-slate-950" style={{ background: color }} />
    <header className="flex h-10 cursor-grab items-center gap-1.5 border-b border-white/10 px-2.5" style={{ backgroundColor: `${color}18` }}>
      <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color, boxShadow: `0 0 9px ${color}` }} />
      {editingTitle ? <input autoFocus value={titleDraft} onChange={(event) => setTitleDraft(event.target.value)} onBlur={saveTitle} onKeyDown={(event) => { if (event.key === "Enter") saveTitle(); if (event.key === "Escape") { setTitleDraft(task.title); setEditingTitle(false); } }} className="nodrag nopan min-w-0 flex-1 rounded border border-white/20 bg-slate-950 px-1.5 py-1 text-[11px] text-white outline-none" /> : <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-slate-100">{task.title}</span>}
      <input type="color" value={color} onChange={(event) => data.onUpdate({ color: event.target.value })} className="nodrag nopan h-5 w-5 cursor-pointer rounded border-0 bg-transparent p-0" title="设置节点颜色" />
      <button type="button" className="nodrag nopan text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); setEditingTitle(true); }} title="编辑标题"><Pencil className="h-3 w-3" /></button>
      {hasChildren && <button type="button" className="nodrag nopan text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); data.onToggle(); }} title={collapsed ? "展开子任务" : "折叠子任务"}>{collapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}</button>}
    </header>
    <div className="nodrag nopan h-[370px] overflow-y-auto p-2.5">
      <div className="mb-2 flex items-center justify-between text-[9px] text-slate-500"><span>{STATUS_META[taskStatus(task)].label}</span><span className="font-mono" style={{ color }}>{task.progress}%</span></div>
      <div className="mb-2 h-1 rounded-full bg-white/10"><div className="h-full rounded-full" style={{ width: `${task.progress}%`, backgroundColor: color }} /></div>
      <div className="mb-2 grid grid-cols-5 gap-1">{([0, 25, 50, 75, 100] as const).map((progress) => <button type="button" key={progress} onClick={() => data.onProgress(progress)} className={`rounded py-1 text-[9px] ${task.progress === progress ? "text-slate-950" : "bg-white/[0.05] text-slate-500 hover:bg-white/[0.1]"}`} style={task.progress === progress ? { backgroundColor: color } : undefined}>{progress}%</button>)}</div>
      <textarea value={descriptionDraft} onChange={(event) => setDescriptionDraft(event.target.value)} onBlur={() => descriptionDraft !== task.description && data.onUpdate({ description: descriptionDraft })} rows={2} placeholder="任务描述" className={`${taskInputClass} nodrag nopan mb-2 resize-none`} />
      {selected && <><div className="mb-2 flex items-center justify-between border-t border-white/10 pt-2"><span className="text-[10px] font-semibold text-amber-300">日志（{logs.length}）</span><FileText className="h-3.5 w-3.5 text-slate-600" /></div><div className="space-y-1.5">{logs.map((log) => <LogItem key={log.id} log={log} onOpenReference={data.onOpenReference} />)}{logs.length === 0 && <p className="text-[9px] text-slate-600">还没有日志</p>}</div><div className="mt-2 border-t border-white/10 pt-2"><textarea value={draftLog.content} onChange={(event) => data.onDraftLogChange({ content: event.target.value })} rows={2} placeholder="记录这次工作的进展…" className={`${taskInputClass} nodrag nopan resize-none`} /><div className="mt-1 flex gap-1"><input type="number" min={0} value={draftLog.minutes} onChange={(event) => data.onDraftLogChange({ minutes: event.target.value })} placeholder="分钟" className={`${taskInputClass} nodrag nopan w-16`} /><button type="button" className={`${taskIconButton} nodrag nopan`} onClick={() => data.onAddReference("file")} title="引用电脑文件"><FolderOpen className="h-3.5 w-3.5" /></button><button type="button" className={`${taskIconButton} nodrag nopan`} onClick={() => data.onAddReference("image")} title="引用图片"><ImageIcon className="h-3.5 w-3.5" /></button><select className={`${taskInputClass} nodrag nopan min-w-0 flex-1`} value="" onChange={(event) => { const bookmark = picky.find((item) => item.id === event.target.value); if (bookmark) data.onAddPickyReference(bookmark); }}><option value="">Picky 引用</option>{picky.map((bookmark) => <option key={bookmark.id} value={bookmark.id}>{bookmark.title}</option>)}</select></div>{draftLog.references.length > 0 && <div className="mt-1 space-y-1">{draftLog.references.map((reference, index) => <div key={`${reference.kind}-${reference.target}-${index}`} className="flex items-center gap-1 rounded bg-white/[0.04] px-2 py-1 text-[9px] text-slate-400"><Link2 className="h-3 w-3 shrink-0 text-cyan-300" /><span className="min-w-0 flex-1 truncate">{reference.label || reference.target}</span><button type="button" className="nodrag nopan text-slate-600 hover:text-red-300" onClick={() => data.onRemoveDraftReference(index)} title="移除引用"><X className="h-3 w-3" /></button></div>)}</div>}<button type="button" className={`${taskButton} nodrag nopan mt-1 w-full justify-center bg-amber-400 text-slate-950 hover:bg-amber-300`} disabled={busy || (!draftLog.content.trim() && !draftLog.references.length && !draftLog.minutes.trim())} onClick={data.onAddLog}><Check className="h-3 w-3" />保存日志</button></div></>}
      <div className="mt-2 flex gap-1 border-t border-white/10 pt-2"><button type="button" className={`${taskButton} nodrag nopan flex-1 justify-center`} onClick={data.onAddChild}><CirclePlus className="h-3 w-3" />子任务</button>{selected && <button type="button" className={`${taskButton} nodrag nopan text-red-300`} onClick={data.onDelete}><Trash2 className="h-3 w-3" />删除</button>}</div>
    </div>
    <Handle type="source" position={Position.Right} className="!h-3 !w-3 !border-2 !border-slate-950" style={{ background: color }} />
  </article>;
});

const taskNodeTypes = { taskNode: TaskFlowNode };
const jsonNodeTypes = { jsonNode: JsonFlowNode };
const edgeTypes = { color: ColorEdge };

function TaskFlowInner(props: TaskFlowProps) {
  const { fitView } = useReactFlow();
  const [nodes, setNodes] = useState<Node<TaskFlowNodeData>[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const taskMap = useMemo(() => new globalThis.Map(props.tasks.map((task) => [task.id, task])), [props.tasks]);
  const visibleTasks = useMemo(() => {
    const result: TaskItem[] = [];
    const visit = (task: TaskItem) => {
      result.push(task);
      if (!collapsed.has(task.id)) props.tasks.filter((child) => child.parentId === task.id).sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt.localeCompare(b.createdAt)).forEach(visit);
    };
    const root = props.selectedSeries ? taskMap.get(props.selectedSeries) : undefined;
    if (root) visit(root);
    return result;
  }, [collapsed, props.selectedSeries, props.tasks, taskMap]);
  const taskNodes = useMemo(() => visibleTasks.map((task, index) => ({ id: task.id, type: "taskNode", position: { x: task.positionX || (task.parentId ? 420 : 60), y: task.positionY || index * 90 }, data: { task, selected: props.selectedTaskId === task.id, collapsed: collapsed.has(task.id), hasChildren: props.tasks.some((child) => child.parentId === task.id), logs: props.selectedTaskId === task.id ? props.logs : [], picky: props.picky, draftLog: props.draftLog, busy: props.busy, onSelect: () => props.onSelect(task.id), onToggle: () => setCollapsed((current) => { const next = new Set(current); next.has(task.id) ? next.delete(task.id) : next.add(task.id); return next; }), onAddChild: () => props.onAddChild(task.id), onDelete: () => props.onDelete(task), onProgress: (progress: number) => props.onProgress(task, progress), onUpdate: (patch: UpdateTaskInput) => props.onUpdate(task.id, patch), onAddLog: props.onAddLog, onAddReference: props.onAddReference, onAddPickyReference: props.onAddPickyReference, onRemoveDraftReference: props.onRemoveDraftReference, onDraftLogChange: props.onDraftLogChange, onOpenReference: props.onOpenReference } })), [collapsed, props]);
  const taskEdges = useMemo<Edge[]>(() => visibleTasks.flatMap((task) => task.parentId && visibleTasks.some((parent) => parent.id === task.parentId) ? [{ id: `task-edge-${task.id}`, source: task.parentId, target: task.id, type: "color", data: { color: hashColor(task.id, TASK_EDGE_COLORS) }, animated: props.selectedTaskId === task.id } as Edge] : []), [props.selectedTaskId, visibleTasks]);
  useEffect(() => { setNodes(taskNodes); }, [taskNodes, setNodes]);
  useEffect(() => { const timer = window.setTimeout(() => fitView({ padding: 0.18, duration: 260 }), 0); return () => window.clearTimeout(timer); }, [fitView, props.selectedSeries, collapsed]);
  return <ReactFlow nodes={nodes} edges={taskEdges} nodeTypes={taskNodeTypes} edgeTypes={edgeTypes} onNodesChange={(changes) => setNodes((current) => current.map((node) => { const change = changes.find((item) => "id" in item && item.id === node.id && item.type === "position"); if (!change || change.type !== "position" || !change.position) return node; return { ...node, position: change.position }; }))} onNodeDragStop={(_, node) => props.onPositionChange(node.id, node.position)} fitView minZoom={0.1} maxZoom={2} nodesConnectable={false} proOptions={{ hideAttribution: true }}><Background color="#1e293b" gap={24} size={1} /><MiniMap nodeColor={(node) => taskColor((node.data as TaskFlowNodeData).task)} pannable zoomable /><Controls showInteractive={false} /></ReactFlow>;
}

export type TaskFlowProps = {
  tasks: TaskItem[];
  selectedSeries: string | null;
  selectedTaskId: string | null;
  logs: TaskLog[];
  picky: PickyBookmark[];
  draftLog: LogDraft;
  busy: boolean;
  onSelect: (id: string) => void;
  onAddChild: (id: string) => void;
  onDelete: (task: TaskItem) => void;
  onProgress: (task: TaskItem, progress: number) => void;
  onUpdate: (id: string, patch: UpdateTaskInput) => void;
  onAddLog: () => void;
  onAddReference: (kind: "file" | "image") => void;
  onAddPickyReference: (bookmark: PickyBookmark) => void;
  onRemoveDraftReference: (index: number) => void;
  onDraftLogChange: (patch: Partial<LogDraft>) => void;
  onOpenReference: (reference: TaskLog["references"][number]) => void;
  onPositionChange: (id: string, position: { x: number; y: number }) => void;
};

export function TaskFlowCanvas(props: TaskFlowProps) {
  return <div className="h-full min-h-0 bg-[#080f1c]"><ReactFlowProvider><TaskFlowInner {...props} /></ReactFlowProvider></div>;
}
