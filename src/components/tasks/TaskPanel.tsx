import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { TaskFlowCanvas } from "../CanvasFlow";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import {
  Check,
  ChevronDown,
  ChevronRight,
  CirclePlus,
  FileText,
  FolderOpen,
  Image as ImageIcon,
  Link2,
  Loader2,
  Map as MapIcon,
  Maximize2,
  Minus,
  Network,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import {
  TaskItem,
  TaskLog,
  TaskLogReferenceInput,
  TaskStatus,
  STATUS_META,
  UpdateTaskInput,
  tasksApi,
  deriveStatus,
} from "./types";

type PickyBookmark = { id: string; title: string; url?: string | null };
type CanvasNode = TaskItem & { x: number; y: number; depth: number; childrenCount: number };
type LogDraft = { content: string; minutes: string; references: TaskLogReferenceInput[] };
type Transform = { x: number; y: number; scale: number };
type Bounds = { minX: number; minY: number; maxX: number; maxY: number; width: number; height: number };
type Position = { x: number; y: number };

type TaskNodeProps = {
  task: CanvasNode;
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
  onDragStart: (event: React.PointerEvent<HTMLElement>) => void;
};

const NODE_WIDTH = 320;
const NODE_HEIGHT = 410;
const COLUMN_GAP = 92;
const ROW_GAP = 34;
const CANVAS_PADDING = 56;
const defaultNodeColor = "#f59e0b";
const inputClass = "w-full rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-amber-400/60";
const iconButton = "inline-flex h-7 w-7 items-center justify-center rounded-md border border-white/10 bg-white/[0.04] text-slate-400 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";
const button = "inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";

function statusOf(task: TaskItem): TaskStatus {
  return deriveStatus(task.progress);
}

function fallbackStatusColor(task: TaskItem): string {
  const status = statusOf(task);
  return status === "done" ? "#34d399" : status === "inProgress" ? "#fbbf24" : "#64748b";
}

function nodeColor(task: TaskItem): string {
  return /^#[0-9a-f]{6}$/i.test(task.color) ? task.color : fallbackStatusColor(task);
}

const TASK_EDGE_COLORS = [
  ["#22d3ee", "#3b82f6"],
  ["#a78bfa", "#ec4899"],
  ["#34d399", "#14b8a6"],
  ["#fbbf24", "#f97316"],
  ["#fb7185", "#f43f5e"],
  ["#60a5fa", "#818cf8"],
] as const;

function taskEdgeColors(id: string): readonly [string, string] {
  let hash = 0;
  for (let index = 0; index < id.length; index += 1) hash = (hash * 31 + id.charCodeAt(index)) >>> 0;
  return TASK_EDGE_COLORS[hash % TASK_EDGE_COLORS.length];
}

function getBounds(nodes: CanvasNode[]): Bounds {
  if (nodes.length === 0) return { minX: 0, minY: 0, maxX: 1, maxY: 1, width: 1, height: 1 };
  const minX = Math.min(...nodes.map((node) => node.x));
  const minY = Math.min(...nodes.map((node) => node.y));
  const maxX = Math.max(...nodes.map((node) => node.x + NODE_WIDTH));
  const maxY = Math.max(...nodes.map((node) => node.y + NODE_HEIGHT));
  return { minX, minY, maxX, maxY, width: Math.max(1, maxX - minX), height: Math.max(1, maxY - minY) };
}

function autoLayout(tasks: TaskItem[], rootId: string, collapsed: Set<string>): CanvasNode[] {
  const taskIds = new Set(tasks.map((task) => task.id));
  const byParent = new globalThis.Map<string, TaskItem[]>();
  tasks.forEach((task) => {
    if (!task.parentId || !taskIds.has(task.parentId)) return;
    const children = byParent.get(task.parentId) ?? [];
    children.push(task);
    byParent.set(task.parentId, children);
  });
  byParent.forEach((children) => children.sort((a, b) => a.sortOrder - b.sortOrder || a.createdAt.localeCompare(b.createdAt)));

  const nodes = new globalThis.Map<string, CanvasNode>();
  let leafIndex = 0;
  const visit = (task: TaskItem, depth: number): number => {
    const children = collapsed.has(task.id) ? [] : (byParent.get(task.id) ?? []);
    const childYs = children.map((child) => visit(child, depth + 1));
    const y = childYs.length > 0 ? (childYs[0] + childYs[childYs.length - 1]) / 2 : leafIndex++ * (NODE_HEIGHT + ROW_GAP);
    nodes.set(task.id, {
      ...task,
      x: CANVAS_PADDING + depth * (NODE_WIDTH + COLUMN_GAP),
      y: CANVAS_PADDING + y,
      depth,
      childrenCount: byParent.get(task.id)?.length ?? 0,
    });
    return y;
  };
  const root = tasks.find((task) => task.id === rootId);
  if (root) visit(root, 0);
  return [...nodes.values()].sort((a, b) => a.depth - b.depth || a.y - b.y);
}

function LogItem({ log, onOpenReference }: { log: TaskLog; onOpenReference: (reference: TaskLog["references"][number]) => void }) {
  return (
    <article className="rounded border border-white/10 bg-black/20 p-2">
      <div className="flex items-center justify-between text-[9px] text-slate-500">
        <span>{log.logDate}</span>
        <span>{log.minutesSpent > 0 ? `${log.minutesSpent} 分钟` : ""}</span>
      </div>
      {log.content && <p className="mt-1 whitespace-pre-wrap text-[10px] leading-relaxed text-slate-300">{log.content}</p>}
      {log.references.length > 0 && (
        <div className="mt-2 space-y-1">
          {log.references.map((reference) => (
            <button type="button" key={reference.id} onClick={() => onOpenReference(reference)} className="flex w-full items-center gap-1.5 rounded bg-white/[0.04] px-2 py-1 text-left text-[9px] text-cyan-300 hover:bg-cyan-400/10">
              <Link2 className="h-3 w-3 shrink-0" />
              <span className="truncate">{reference.label || reference.target}</span>
            </button>
          ))}
        </div>
      )}
    </article>
  );
}

function TaskNode({
  task,
  selected,
  collapsed,
  hasChildren,
  logs,
  picky,
  draftLog,
  busy,
  onSelect,
  onToggle,
  onAddChild,
  onDelete,
  onProgress,
  onUpdate,
  onAddLog,
  onAddReference,
  onAddPickyReference,
  onRemoveDraftReference,
  onDraftLogChange,
  onOpenReference,
  onDragStart,
}: TaskNodeProps) {
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState(task.title);
  const [descriptionDraft, setDescriptionDraft] = useState(task.description);
  const color = nodeColor(task);

  useEffect(() => {
    setTitleDraft(task.title);
    setDescriptionDraft(task.description);
  }, [task.description, task.title]);

  const saveTitle = () => {
    const title = titleDraft.trim();
    if (title && title !== task.title) onUpdate({ title });
    setEditingTitle(false);
  };

  const saveDescription = () => {
    if (descriptionDraft !== task.description) onUpdate({ description: descriptionDraft });
  };

  return (
    <article
      className={`task-canvas-node absolute overflow-hidden rounded-lg border bg-[#101827] shadow-2xl transition-shadow ${selected ? "shadow-cyan-500/25" : "shadow-black/30"}`}
      style={{ left: task.x, top: task.y, width: NODE_WIDTH, height: NODE_HEIGHT, borderColor: selected ? "#22d3ee" : `${color}88` }}
      onClick={onSelect}
    >
      <header className="flex h-10 cursor-grab items-center gap-1.5 border-b border-white/10 px-2.5 active:cursor-grabbing" style={{ backgroundColor: `${color}18` }} onPointerDown={onDragStart}>
        <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color, boxShadow: `0 0 9px ${color}` }} />
        {editingTitle ? (
          <input
            autoFocus
            value={titleDraft}
            onChange={(event) => setTitleDraft(event.target.value)}
            onPointerDown={(event) => event.stopPropagation()}
            onBlur={saveTitle}
            onKeyDown={(event) => { if (event.key === "Enter") saveTitle(); if (event.key === "Escape") { setTitleDraft(task.title); setEditingTitle(false); } }}
            className="min-w-0 flex-1 rounded border border-white/20 bg-slate-950 px-1.5 py-1 text-[11px] text-white outline-none"
          />
        ) : (
          <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-slate-100">{task.title}</span>
        )}
        <input type="color" value={color} onChange={(event) => onUpdate({ color: event.target.value })} onPointerDown={(event) => event.stopPropagation()} className="h-5 w-5 cursor-pointer rounded border-0 bg-transparent p-0" title="设置节点颜色" />
        <button type="button" className="text-slate-500 hover:text-white" onPointerDown={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); setEditingTitle(true); }} title="编辑标题" aria-label="编辑标题"><Pencil className="h-3 w-3" /></button>
        {hasChildren && <button type="button" className="text-slate-500 hover:text-white" onPointerDown={(event) => event.stopPropagation()} onClick={(event) => { event.stopPropagation(); onToggle(); }} title={collapsed ? "展开子任务" : "折叠子任务"} aria-label={collapsed ? "展开子任务" : "折叠子任务"}>{collapsed ? <ChevronRight className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}</button>}
      </header>

      <div className="h-[370px] overflow-y-auto p-2.5" onPointerDown={(event) => event.stopPropagation()}>
        <div className="mb-2 flex items-center justify-between text-[9px] text-slate-500">
          <span>{STATUS_META[statusOf(task)].label}</span>
          <span className="font-mono" style={{ color }}>{task.progress}%</span>
        </div>
        <div className="mb-2 h-1 rounded-full bg-white/10"><div className="h-full rounded-full" style={{ width: `${task.progress}%`, backgroundColor: color }} /></div>
        <div className="mb-2 grid grid-cols-5 gap-1">
          {([0, 25, 50, 75, 100] as const).map((progress) => <button type="button" key={progress} onClick={() => onProgress(progress)} className={`rounded py-1 text-[9px] ${task.progress === progress ? "text-slate-950" : "bg-white/[0.05] text-slate-500 hover:bg-white/[0.1]"}`} style={task.progress === progress ? { backgroundColor: color } : undefined}>{progress}%</button>)}
        </div>
        <textarea value={descriptionDraft} onChange={(event) => setDescriptionDraft(event.target.value)} onBlur={saveDescription} rows={2} placeholder="任务描述" className={`${inputClass} mb-2 resize-none`} />
        <div className="mb-2 flex items-center gap-1 text-[9px] text-slate-500">
          <span>{hasChildren ? `${task.childrenCount} 个子任务` : "叶子任务"}</span>
          <span className="ml-auto">{task.positionX !== 0 || task.positionY !== 0 ? "自定义位置" : "自动布局"}</span>
        </div>

        {selected && (
          <>
            <div className="mb-2 flex items-center justify-between border-t border-white/10 pt-2"><span className="text-[10px] font-semibold text-amber-300">日志（{logs.length}）</span><FileText className="h-3.5 w-3.5 text-slate-600" /></div>
            <div className="space-y-1.5">
              {logs.map((log) => <LogItem key={log.id} log={log} onOpenReference={onOpenReference} />)}
              {logs.length === 0 && <p className="text-[9px] text-slate-600">还没有日志</p>}
            </div>
            <div className="mt-2 border-t border-white/10 pt-2">
              <textarea value={draftLog.content} onChange={(event) => onDraftLogChange({ content: event.target.value })} rows={2} placeholder="记录这次工作的进展…" className={`${inputClass} resize-none`} />
              <div className="mt-1 flex gap-1">
                <input type="number" min={0} value={draftLog.minutes} onChange={(event) => onDraftLogChange({ minutes: event.target.value })} placeholder="分钟" className={`${inputClass} w-16`} />
                <button type="button" className={iconButton} onClick={() => onAddReference("file")} title="引用电脑文件" aria-label="引用电脑文件"><FolderOpen className="h-3.5 w-3.5" /></button>
                <button type="button" className={iconButton} onClick={() => onAddReference("image")} title="引用图片" aria-label="引用图片"><ImageIcon className="h-3.5 w-3.5" /></button>
                <select className={`${inputClass} min-w-0 flex-1`} value="" onChange={(event) => { const bookmark = picky.find((item) => item.id === event.target.value); if (bookmark) onAddPickyReference(bookmark); }}><option value="">Picky 引用</option>{picky.map((bookmark) => <option key={bookmark.id} value={bookmark.id}>{bookmark.title}</option>)}</select>
              </div>
              {draftLog.references.length > 0 && <div className="mt-1 space-y-1">{draftLog.references.map((reference, index) => <div key={`${reference.kind}-${reference.target}-${index}`} className="flex items-center gap-1 rounded bg-white/[0.04] px-2 py-1 text-[9px] text-slate-400"><Link2 className="h-3 w-3 shrink-0 text-cyan-300" /><span className="min-w-0 flex-1 truncate">{reference.label || reference.target}</span><button type="button" className="text-slate-600 hover:text-red-300" onClick={() => onRemoveDraftReference(index)} title="移除引用" aria-label="移除引用"><X className="h-3 w-3" /></button></div>)}</div>}
              <button type="button" className={`${button} mt-1 w-full justify-center bg-amber-400 text-slate-950 hover:bg-amber-300`} disabled={busy || (!draftLog.content.trim() && !draftLog.references.length && !draftLog.minutes.trim())} onClick={onAddLog}><Check className="h-3 w-3" />保存日志</button>
            </div>
          </>
        )}

        <div className="mt-2 flex gap-1 border-t border-white/10 pt-2">
          <button type="button" className={`${button} flex-1 justify-center`} onClick={onAddChild}><CirclePlus className="h-3 w-3" />子任务</button>
          {selected && <button type="button" className={`${button} text-red-300`} onClick={onDelete}><Trash2 className="h-3 w-3" />删除</button>}
        </div>
      </div>
    </article>
  );
}

function TaskCanvas({
  tasks,
  selectedSeries,
  selectedTaskId,
  collapsed,
  logs,
  picky,
  draftLog,
  busy,
  onSelect,
  onToggle,
  onAddChild,
  onExpandAll,
  onAutoLayout,
  onDelete,
  onProgress,
  onUpdate,
  onAddLog,
  onAddReference,
  onAddPickyReference,
  onRemoveDraftReference,
  onDraftLogChange,
  onOpenReference,
  onPositionChange,
}: {
  tasks: TaskItem[];
  selectedSeries: string | null;
  selectedTaskId: string | null;
  collapsed: Set<string>;
  logs: TaskLog[];
  picky: PickyBookmark[];
  draftLog: LogDraft;
  busy: boolean;
  onSelect: (id: string) => void;
  onToggle: (id: string) => void;
  onAddChild: (id: string) => void;
  onExpandAll: () => void;
  onAutoLayout: () => void;
  onDelete: (task: TaskItem) => void;
  onProgress: (task: TaskItem, progress: number) => void;
  onUpdate: (id: string, patch: UpdateTaskInput) => void;
  onAddLog: () => void;
  onAddReference: (kind: "file" | "image") => void;
  onAddPickyReference: (bookmark: PickyBookmark) => void;
  onRemoveDraftReference: (index: number) => void;
  onDraftLogChange: (patch: Partial<LogDraft>) => void;
  onOpenReference: (reference: TaskLog["references"][number]) => void;
  onPositionChange: (id: string, position: Position) => void;
}) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const minimapRef = useRef<HTMLCanvasElement>(null);
  const transformRef = useRef<Transform>({ x: 0, y: 0, scale: 1 });
  const dragRef = useRef<{ id: string; startX: number; startY: number; origin: Position } | null>(null);
  const panRef = useRef<{ x: number; y: number } | null>(null);
  const [transform, setTransform] = useState<Transform>({ x: 0, y: 0, scale: 1 });
  const [localPositions, setLocalPositions] = useState<Record<string, Position>>({});
  const autoNodes = useMemo(() => selectedSeries ? autoLayout(tasks, selectedSeries, collapsed) : [], [collapsed, selectedSeries, tasks]);
  const nodes = useMemo(() => autoNodes.map((node) => ({ ...node, ...(localPositions[node.id] ?? ((node.positionX !== 0 || node.positionY !== 0) ? { x: node.positionX, y: node.positionY } : {})) })), [autoNodes, localPositions]);
  const nodeById = useMemo(() => new globalThis.Map(nodes.map((node) => [node.id, node])), [nodes]);
  const bounds = useMemo(() => getBounds(nodes), [nodes]);

  useEffect(() => {
    setLocalPositions((previous) => {
      const next = { ...previous };
      tasks.forEach((task) => {
        if (!(task.id in next) && (task.positionX !== 0 || task.positionY !== 0)) next[task.id] = { x: task.positionX, y: task.positionY };
      });
      Object.keys(next).forEach((id) => { if (!tasks.some((task) => task.id === id)) delete next[id]; });
      return next;
    });
  }, [tasks]);

  const selectedChain = useMemo(() => {
    const ids = new Set<string>();
    let current = selectedTaskId ? nodeById.get(selectedTaskId) : null;
    while (current) {
      ids.add(current.id);
      current = current.parentId ? nodeById.get(current.parentId) : undefined;
    }
    return ids;
  }, [nodeById, selectedTaskId]);

  const applyTransform = (next: Transform) => {
    transformRef.current = next;
    setTransform(next);
  };

  const fitView = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport || nodes.length === 0) return;
    const rect = viewport.getBoundingClientRect();
    const scale = Math.max(0.28, Math.min(1.2, Math.min((rect.width - 72) / bounds.width, (rect.height - 72) / bounds.height)));
    applyTransform({ scale, x: (rect.width - bounds.width * scale) / 2 - bounds.minX * scale, y: (rect.height - bounds.height * scale) / 2 - bounds.minY * scale });
  }, [bounds, nodes.length]);

  useEffect(() => {
    const frame = window.requestAnimationFrame(fitView);
    return () => window.cancelAnimationFrame(frame);
  }, [collapsed, fitView, selectedSeries]);

  useEffect(() => {
    const observer = new ResizeObserver(fitView);
    if (viewportRef.current) observer.observe(viewportRef.current);
    return () => observer.disconnect();
  }, [fitView]);

  const drawMinimap = useCallback(() => {
    const canvas = minimapRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    canvas.width = Math.max(1, Math.floor(rect.width * ratio));
    canvas.height = Math.max(1, Math.floor(rect.height * ratio));
    const context = canvas.getContext("2d");
    if (!context) return;
    const padding = 8;
    const scale = Math.min((rect.width - padding * 2) / bounds.width, (rect.height - padding * 2) / bounds.height);
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, rect.width, rect.height);
    context.fillStyle = "rgba(8, 15, 28, .95)";
    context.fillRect(0, 0, rect.width, rect.height);
    nodes.forEach((node) => {
      context.fillStyle = node.id === selectedTaskId ? "#22d3ee" : selectedChain.has(node.id) ? "#0891b2" : nodeColor(node);
      context.globalAlpha = node.id === selectedTaskId || selectedChain.has(node.id) ? 1 : .72;
      context.fillRect(padding + (node.x - bounds.minX) * scale, padding + (node.y - bounds.minY) * scale, Math.max(3, NODE_WIDTH * scale), Math.max(2, NODE_HEIGHT * scale));
    });
    const viewport = viewportRef.current?.getBoundingClientRect();
    if (viewport) {
      context.globalAlpha = 1;
      context.strokeStyle = "#f8fafc";
      context.strokeRect(padding + (-transformRef.current.x / transformRef.current.scale - bounds.minX) * scale, padding + (-transformRef.current.y / transformRef.current.scale - bounds.minY) * scale, viewport.width / transformRef.current.scale * scale, viewport.height / transformRef.current.scale * scale);
    }
    context.globalAlpha = 1;
  }, [bounds, nodes, selectedChain, selectedTaskId, transform]);

  useEffect(() => { drawMinimap(); }, [drawMinimap]);

  const zoomAt = (clientX: number, clientY: number, factor: number) => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const rect = viewport.getBoundingClientRect();
    const oldScale = transformRef.current.scale;
    const scale = Math.max(0.28, Math.min(2.5, oldScale * factor));
    const cursorX = clientX - rect.left;
    const cursorY = clientY - rect.top;
    applyTransform({ scale, x: cursorX - (cursorX - transformRef.current.x) * scale / oldScale, y: cursorY - (cursorY - transformRef.current.y) * scale / oldScale });
  };

  const handleNodeDragStart = (id: string, event: React.PointerEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).closest("button, input, textarea, select")) return;
    event.stopPropagation();
    const node = nodeById.get(id);
    if (!node) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { id, startX: event.clientX, startY: event.clientY, origin: { x: node.x, y: node.y } };
    onSelect(id);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    const position = { x: drag.origin.x + (event.clientX - drag.startX) / transformRef.current.scale, y: drag.origin.y + (event.clientY - drag.startY) / transformRef.current.scale };
    setLocalPositions((previous) => ({ ...previous, [drag.id]: position }));
  };

  const finishNodeDrag = () => {
    const drag = dragRef.current;
    dragRef.current = null;
    if (!drag) return;
    const node = nodeById.get(drag.id);
    const position = localPositions[drag.id] ?? (node ? { x: node.x, y: node.y } : null);
    if (position) onPositionChange(drag.id, position);
  };

  const handleCanvasPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if ((event.target as HTMLElement).closest("button, input, textarea, select, .task-canvas-node, canvas")) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    panRef.current = { x: event.clientX, y: event.clientY };
  };

  const handleCanvasPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const pan = panRef.current;
    if (!pan || dragRef.current) return;
    applyTransform({ ...transformRef.current, x: transformRef.current.x + event.clientX - pan.x, y: transformRef.current.y + event.clientY - pan.y });
    panRef.current = { x: event.clientX, y: event.clientY };
  };

  const handleCanvasPointerUp = () => {
    panRef.current = null;
    finishNodeDrag();
  };

  const handleAutoLayout = () => {
    if (!selectedSeries) return;
    const layout = autoLayout(tasks, selectedSeries, new Set());
    setLocalPositions(Object.fromEntries(layout.map((node) => [node.id, { x: node.x, y: node.y }])));
    onExpandAll();
    onAutoLayout();
    window.requestAnimationFrame(fitView);
  };

  const hitMinimap = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const padding = 8;
    const scale = Math.min((rect.width - padding * 2) / bounds.width, (rect.height - padding * 2) / bounds.height);
    const worldX = bounds.minX + (event.clientX - rect.left - padding) / scale;
    const worldY = bounds.minY + (event.clientY - rect.top - padding) / scale;
    const viewport = viewportRef.current?.getBoundingClientRect();
    if (!viewport) return;
    applyTransform({ ...transformRef.current, x: viewport.width / 2 - worldX * transformRef.current.scale, y: viewport.height / 2 - worldY * transformRef.current.scale });
  };

  const worldWidth = bounds.maxX + CANVAS_PADDING;
  const worldHeight = Math.max(bounds.maxY + CANVAS_PADDING, 600);

  return (
    <div ref={viewportRef} className="relative h-full min-h-0 overflow-hidden bg-[#080f1c]" onWheel={(event) => { event.preventDefault(); zoomAt(event.clientX, event.clientY, event.deltaY < 0 ? 1.1 : 0.9); }} onPointerDown={handleCanvasPointerDown} onPointerMove={(event) => { handlePointerMove(event); handleCanvasPointerMove(event); }} onPointerUp={handleCanvasPointerUp} onPointerCancel={handleCanvasPointerUp}>
      <div className="absolute left-3 top-3 z-30 flex items-center gap-1 rounded-md border border-white/10 bg-slate-900/90 p-1.5 shadow-lg">
        <button type="button" className={iconButton} onClick={() => { const rect = viewportRef.current?.getBoundingClientRect(); if (rect) zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, .85); }} title="缩小" aria-label="缩小"><Minus className="h-3 w-3" /></button>
        <span className="w-10 text-center font-mono text-[9px] text-slate-500">{Math.round(transform.scale * 100)}%</span>
        <button type="button" className={iconButton} onClick={() => { const rect = viewportRef.current?.getBoundingClientRect(); if (rect) zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, 1.18); }} title="放大" aria-label="放大"><Plus className="h-3 w-3" /></button>
        <button type="button" className={iconButton} onClick={fitView} title="适配视口" aria-label="适配视口"><Maximize2 className="h-3 w-3" /></button>
        <button type="button" className={iconButton} onClick={handleAutoLayout} title="自动布局" aria-label="自动布局"><MapIcon className="h-3 w-3" /></button>
      </div>
      <canvas ref={minimapRef} className="absolute bottom-3 right-3 z-30 h-[108px] w-[180px] rounded-md border border-white/10 shadow-lg" onPointerDown={(event) => { event.stopPropagation(); hitMinimap(event); }} />
      {!selectedSeries && <div className="flex h-full items-center justify-center text-[11px] text-slate-600">选择左侧任务系列后开始编辑节点</div>}
      {selectedSeries && nodes.length === 0 && <div className="flex h-full items-center justify-center text-[11px] text-slate-600">当前系列没有可显示的任务节点</div>}
      {selectedSeries && nodes.length > 0 && <div className="absolute left-0 top-0" style={{ width: worldWidth, height: worldHeight, transform: `translate(${transform.x}px, ${transform.y}px) scale(${transform.scale})`, transformOrigin: "0 0" }}>
        <svg className="pointer-events-none absolute left-0 top-0 overflow-visible" width={worldWidth} height={worldHeight}>
          <defs>
            <filter id="task-edge-glow" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="3" result="blur" /><feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge></filter>
            {nodes.map((node) => {
              if (!node.parentId) return null;
              const parent = nodeById.get(node.parentId);
              if (!parent) return null;
              const edgeId = `task-edge-${node.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
              const [startColor, endColor] = taskEdgeColors(node.id);
              return <linearGradient key={edgeId} id={edgeId} x1="0%" y1="0%" x2="100%" y2="0%"><stop offset="0%" stopColor={startColor} /><stop offset="52%" stopColor={nodeColor(node)} /><stop offset="100%" stopColor={endColor} /></linearGradient>;
            })}
            {nodes.map((node) => {
              if (!node.parentId || !nodeById.has(node.parentId)) return null;
              const edgeId = `task-edge-${node.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
              const markerId = `${edgeId}-arrow`;
              return <marker key={markerId} id={markerId} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill={`url(#${edgeId})`} /></marker>;
            })}
          </defs>
          {nodes.map((node) => {
            if (!node.parentId) return null;
            const parent = nodeById.get(node.parentId);
            if (!parent) return null;
            const highlighted = selectedChain.has(node.id) && selectedChain.has(parent.id);
            const edgeId = `task-edge-${node.id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
            const markerId = `${edgeId}-arrow`;
            const path = `M ${parent.x + NODE_WIDTH} ${parent.y + NODE_HEIGHT / 2} C ${parent.x + NODE_WIDTH + COLUMN_GAP / 2} ${parent.y + NODE_HEIGHT / 2}, ${node.x - COLUMN_GAP / 2} ${node.y + NODE_HEIGHT / 2}, ${node.x} ${node.y + NODE_HEIGHT / 2}`;
            return <g key={node.id} opacity={highlighted ? 1 : 0.82}>
              <path d={path} fill="none" stroke={`url(#${edgeId})`} strokeWidth={highlighted ? 8 : 6} opacity={highlighted ? 0.28 : 0.16} filter="url(#task-edge-glow)" />
              <path d={path} fill="none" stroke={`url(#${edgeId})`} strokeWidth={highlighted ? 4.5 : 2.8} strokeLinecap="round" markerEnd={`url(#${markerId})`} />
            </g>;
          })}
        </svg>
        {nodes.map((node) => <TaskNode key={node.id} task={node} selected={selectedTaskId === node.id} collapsed={collapsed.has(node.id)} hasChildren={node.childrenCount > 0} logs={selectedTaskId === node.id ? logs : []} picky={picky} draftLog={draftLog} busy={busy} onSelect={() => onSelect(node.id)} onToggle={() => onToggle(node.id)} onAddChild={() => onAddChild(node.id)} onDelete={() => onDelete(node)} onProgress={(progress) => onProgress(node, progress)} onUpdate={(patch) => onUpdate(node.id, patch)} onAddLog={onAddLog} onAddReference={onAddReference} onAddPickyReference={onAddPickyReference} onRemoveDraftReference={onRemoveDraftReference} onDraftLogChange={onDraftLogChange} onOpenReference={onOpenReference} onDragStart={(event) => handleNodeDragStart(node.id, event)} />)}
      </div>}
    </div>
  );
}

export default function TaskPanel() {
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [selectedSeries, setSelectedSeries] = useState<string | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [search, setSearch] = useState("");
  const [statusFilter, setStatusFilter] = useState<TaskStatus | "all">("all");
  const [logs, setLogs] = useState<TaskLog[]>([]);
  const [picky, setPicky] = useState<PickyBookmark[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [showCreate, setShowCreate] = useState(false);
  const [createParentId, setCreateParentId] = useState<string | null>(null);
  const [editing, setEditing] = useState<TaskItem | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [draftLog, setDraftLog] = useState<LogDraft>({ content: "", minutes: "", references: [] });
  const [notice, setNotice] = useState("");

  const loadTasks = useCallback(async () => {
    setLoading(true);
    try {
      const result = await tasksApi.search("", false);
      setTasks(result);
      setSelectedSeries((current) => current && result.some((task) => task.id === current) ? current : null);
      setSelectedTaskId((current) => current && result.some((task) => task.id === current) ? current : null);
    } catch (error) {
      setNotice(`加载任务失败：${String(error)}`);
    } finally {
      setLoading(false);
    }
  }, []);

  const loadLogs = useCallback(async (taskId: string | null) => {
    if (!taskId) { setLogs([]); return; }
    try { setLogs(await tasksApi.listLogs(taskId)); } catch (error) { setNotice(`加载日志失败：${String(error)}`); }
  }, []);

  useEffect(() => {
    void loadTasks();
    invoke<{ bookmarks?: PickyBookmark[] }>("picky_get_state").then((state) => setPicky(state.bookmarks ?? [])).catch(() => {});
  }, [loadTasks]);

  useEffect(() => { void loadLogs(selectedTaskId); }, [loadLogs, selectedTaskId]);

  const filteredTasks = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    return tasks.filter((task) => (!keyword || `${task.title} ${task.description}`.toLowerCase().includes(keyword)) && (statusFilter === "all" || statusOf(task) === statusFilter));
  }, [search, statusFilter, tasks]);
  const series = useMemo(() => filteredTasks.filter((task) => !task.parentId), [filteredTasks]);
  const selectedSeriesTask = tasks.find((task) => task.id === selectedSeries) ?? null;
  const selectedTask = selectedTaskId ? tasks.find((task) => task.id === selectedTaskId) ?? null : null;

  const flash = useCallback((message: string) => {
    setNotice(message);
    window.setTimeout(() => setNotice(""), 2200);
  }, []);

  const updateTask = useCallback(async (id: string, patch: UpdateTaskInput) => {
    try {
      const updated = await tasksApi.update(id, patch);
      setTasks((current) => current.map((task) => task.id === id ? updated : task));
    } catch (error) {
      flash(`更新任务失败：${String(error)}`);
    }
  }, [flash]);

  const savePosition = useCallback((id: string, position: Position) => {
    void updateTask(id, { positionX: Math.round(position.x), positionY: Math.round(position.y) });
  }, [updateTask]);

  const autoLayoutTasks = useCallback(() => {
    if (!selectedSeries) return;
    const layout = autoLayout(tasks, selectedSeries, new Set());
    void Promise.all(layout.map((node) => updateTask(node.id, { positionX: Math.round(node.x), positionY: Math.round(node.y) })));
  }, [selectedSeries, tasks, updateTask]);

  const saveTask = async () => {
    const title = draftTitle.trim();
    if (!title) return;
    setBusy(true);
    try {
      if (editing) {
        const updated = await tasksApi.update(editing.id, { title, parentId: editing.parentId });
        setTasks((current) => current.map((task) => task.id === updated.id ? updated : task));
      } else {
        const created = await tasksApi.create({ title, parentId: createParentId, color: defaultNodeColor });
        setTasks((current) => [...current, created]);
        setSelectedSeries(createParentId ? selectedSeries : created.id);
        setSelectedTaskId(created.id);
      }
      setShowCreate(false);
      setEditing(null);
      setDraftTitle("");
    } catch (error) {
      flash(`保存任务失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  };

  const setProgress = useCallback(async (task: TaskItem, progress: number) => {
    try {
      const updated = await tasksApi.setProgress(task.id, { progress });
      setTasks((current) => current.map((item) => item.id === updated.id ? updated : item));
    } catch (error) { flash(`更新状态失败：${String(error)}`); }
  }, [flash]);

  const deleteTask = useCallback(async (task: TaskItem) => {
    if (!window.confirm(`删除「${task.title}」？子任务会提升为顶层任务。`)) return;
    try {
      await tasksApi.remove(task.id);
      setTasks((current) => current.filter((item) => item.id !== task.id).map((item) => item.parentId === task.id ? { ...item, parentId: null } : item));
      if (selectedTaskId === task.id) setSelectedTaskId(null);
      if (selectedSeries === task.id) setSelectedSeries(null);
      setLogs([]);
    } catch (error) { flash(`删除失败：${String(error)}`); }
  }, [flash, selectedSeries, selectedTaskId]);

  const addReference = useCallback(async (kind: "file" | "image") => {
    const selected = await openDialog({ multiple: false, directory: false, title: kind === "image" ? "选择图片引用" : "选择文件引用", filters: kind === "image" ? [{ name: "图片", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }] : undefined });
    if (typeof selected === "string") setDraftLog((draft) => ({ ...draft, references: [...draft.references, { kind, target: selected, label: selected.split(/[\\/]/).pop() ?? selected }] }));
  }, []);

  const addPickyReference = useCallback((bookmark: PickyBookmark) => setDraftLog((draft) => ({ ...draft, references: [...draft.references, { kind: "picky", target: bookmark.id, label: bookmark.title }] })), []);

  const addLog = useCallback(async () => {
    if (!selectedTask || (!draftLog.content.trim() && !draftLog.references.length && !draftLog.minutes.trim())) return;
    setBusy(true);
    try {
      await tasksApi.addLog(selectedTask.id, new Date().toISOString().slice(0, 10), draftLog.content, Number(draftLog.minutes) || 0, draftLog.references);
      setDraftLog({ content: "", minutes: "", references: [] });
      await loadLogs(selectedTask.id);
      flash("日志已记录");
    } catch (error) { flash(`写入日志失败：${String(error)}`); } finally { setBusy(false); }
  }, [draftLog, flash, loadLogs, selectedTask]);

  const openReference = useCallback((reference: TaskLog["references"][number]) => {
    if (reference.kind === "file" || reference.kind === "image") void openPath(reference.target);
    else void openUrl(picky.find((bookmark) => bookmark.id === reference.target)?.url ?? reference.target);
  }, [picky]);

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-slate-950/25 text-slate-200">
      <header className="flex min-h-12 shrink-0 items-center gap-2 border-b border-white/10 px-3">
        <Network className="h-4 w-4 text-amber-300" /><span className="text-sm font-semibold text-white">任务画布</span><span className="text-[10px] text-slate-500">{tasks.length} 个节点 · {series.length} 个系列</span>
        <div className="ml-auto flex items-center gap-1.5"><div className="flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.04] px-2"><Search className="h-3 w-3 text-slate-500" /><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder="搜索任务" className="h-7 w-36 bg-transparent text-[11px] outline-none placeholder:text-slate-600" /></div><button type="button" className={button} onClick={() => { setEditing(null); setCreateParentId(null); setDraftTitle(""); setShowCreate(true); }}><Plus className="h-3 w-3" />新建系列</button></div>
      </header>
      <div className="flex min-h-0 flex-1">
        <aside className="flex w-[220px] shrink-0 flex-col border-r border-white/10 bg-slate-950/30">
          <div className="flex items-center gap-1 border-b border-white/10 p-2">{(["all", "todo", "inProgress", "done"] as const).map((filter) => <button key={filter} type="button" onClick={() => setStatusFilter(filter)} className={`flex-1 rounded px-1 py-1.5 text-[9px] ${statusFilter === filter ? "bg-amber-400 text-slate-950" : "text-slate-500 hover:bg-white/[0.06]"}`}>{filter === "all" ? "全部" : STATUS_META[filter].label}</button>)}</div>
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {loading ? <div className="flex justify-center py-10 text-slate-500"><Loader2 className="h-4 w-4 animate-spin" /></div> : series.map((task) => <button type="button" key={task.id} onClick={() => { setSelectedSeries(task.id); setSelectedTaskId(task.id); }} className={`mb-1.5 w-full rounded-md border px-2.5 py-2 text-left ${selectedSeries === task.id ? "border-amber-300/50 bg-amber-300/10" : "border-white/10 bg-white/[0.02] hover:bg-white/[0.06]"}`}><span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full" style={{ backgroundColor: nodeColor(task) }} /><span className="min-w-0 flex-1 truncate text-[11px] text-slate-200">{task.title}</span><span className="font-mono text-[9px] text-slate-500">{task.progress}%</span></span><span className="mt-1 block text-[9px] text-slate-600">{tasks.filter((child) => child.parentId === task.id).length} 个直接子任务</span></button>)}
            {!loading && series.length === 0 && <div className="py-10 text-center text-[10px] text-slate-600">没有匹配的系列</div>}
          </div>
        </aside>
        <main className="relative min-w-0 flex-1">
          <TaskFlowCanvas tasks={tasks} selectedSeries={selectedSeries} selectedTaskId={selectedTaskId} logs={logs} picky={picky} draftLog={draftLog} busy={busy} onSelect={setSelectedTaskId} onAddChild={(id) => { setEditing(null); setCreateParentId(id); setDraftTitle(""); setShowCreate(true); }} onDelete={(task) => void deleteTask(task)} onProgress={(task, progress) => void setProgress(task, progress)} onUpdate={(id, patch) => void updateTask(id, patch)} onAddLog={() => void addLog()} onAddReference={(kind) => void addReference(kind)} onAddPickyReference={addPickyReference} onRemoveDraftReference={(index) => setDraftLog((draft) => ({ ...draft, references: draft.references.filter((_, itemIndex) => itemIndex !== index) }))} onDraftLogChange={(patch) => setDraftLog((draft) => ({ ...draft, ...patch }))} onOpenReference={openReference} onPositionChange={savePosition} />
        </main>
      </div>
      {notice && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
      {showCreate && <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4" onClick={() => setShowCreate(false)}><div className="w-[360px] rounded-lg border border-white/10 bg-[#101827] p-4 shadow-2xl" onClick={(event) => event.stopPropagation()}><div className="mb-3 flex items-center justify-between"><h3 className="text-sm font-semibold text-white">{editing ? "编辑任务" : createParentId ? "添加子任务" : "新建系列"}</h3><button type="button" className={iconButton} onClick={() => setShowCreate(false)}><X className="h-3.5 w-3.5" /></button></div><input autoFocus value={draftTitle} onChange={(event) => setDraftTitle(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void saveTask(); }} placeholder="任务名称" className={inputClass} /><div className="mt-3 flex justify-end gap-2"><button type="button" className={button} onClick={() => setShowCreate(false)}>取消</button><button type="button" className="inline-flex items-center gap-1 rounded-md bg-amber-400 px-3 py-1.5 text-[10px] font-semibold text-slate-950" disabled={busy || !draftTitle.trim()} onClick={() => void saveTask()}>{busy && <Loader2 className="h-3 w-3 animate-spin" />}保存</button></div></div></div>}
    </div>
  );
}
