import { memo, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
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
import { ChevronDown, ChevronRight } from "lucide-react";
import type { JsonValue, SearchMatches } from "./SystemTools/JsonBrowser";

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

const JSON_EDGE_COLORS = ["#22d3ee", "#a78bfa", "#34d399", "#fbbf24", "#fb7185", "#60a5fa"];

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
  const { t } = useTranslation();
  const columns = arrayColumns(value);
  const objectRows = columns[0] !== "value";
  const displayRows = value.slice(0, 80);
  return (
    <div className="mt-2 overflow-hidden rounded border border-white/10 bg-slate-950/60" onClick={(event) => event.stopPropagation()}>
      <div className="border-b border-white/10 px-2 py-1 text-[9px] font-semibold uppercase tracking-wide text-cyan-300">{t("canvasflow.arrContent", { count: value.length })}</div>
      <div className="max-h-36 overflow-auto">
        <table className="w-full table-fixed border-collapse text-left font-mono text-[9px]">
          <thead><tr>{objectRows && <th className="sticky top-0 w-6 border-b border-white/10 bg-slate-900 px-1.5 py-1 text-slate-600">#</th>}{columns.map((column) => <th key={column} className="sticky top-0 max-w-[100px] border-b border-white/10 bg-slate-900 px-1.5 py-1 text-cyan-300">{column}</th>)}</tr></thead>
          <tbody>{displayRows.map((row, index) => <tr key={`${path}.${index}`} className="hover:bg-white/[0.05]" onClick={() => onSelect(`${path}.${index}`)}>{objectRows && <td className="border-b border-white/5 px-1.5 py-1 text-slate-600">{index + 1}</td>}{columns.map((column) => { const cell = objectRows && typeof row === "object" && row !== null && !Array.isArray(row) ? row[column] ?? null : row; return <td key={column} className="max-w-[100px] truncate border-b border-white/5 px-1.5 py-1 text-slate-300" title={JSON.stringify(cell)}>{compactJsonValue(cell)}</td>; })}</tr>)}</tbody>
        </table>
        {value.length > 80 && <div className="border-t border-white/10 px-2 py-1 text-[9px] text-slate-600">{t("canvasflow.showFirst80")}</div>}
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
  const { t } = useTranslation();
  const { item, selectedPath, searchMatches, collapsed, onSelect, onToggle, onCopy } = data;
  const selected = item.path === selectedPath;
  const chain = searchMatches.paths.has(item.path);
  const directMatch = searchMatches.directPaths.has(item.path);
  const container = isJsonContainer(item.value);
  const color = hashColor(item.id, JSON_EDGE_COLORS);
  return (
    <div className={`w-[250px] rounded-lg border bg-surface-modal px-2.5 py-2 shadow-xl ${selected ? "border-cyan-300 shadow-cyan-500/30" : chain ? "border-cyan-700/80" : "border-white/10"}`} onClick={() => onSelect(item.path)}>
      <Handle type="target" position={Position.Left} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
      <div className="flex items-center gap-1.5">
        {container && <button type="button" className="nodrag nopan inline-flex h-4 w-4 items-center justify-center text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); onToggle(item.path); }} title={collapsed.has(item.path) ? t("canvasflow.expand") : t("canvasflow.collapse")}>{collapsed.has(item.path) ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}</button>}
        <span className={`min-w-0 flex-1 truncate font-mono text-[11px] ${directMatch ? "text-yellow-300" : "text-cyan-200"}`}>{item.name}</span>
        <button type="button" className="nodrag nopan text-slate-600 hover:text-white" onClick={(event) => { event.stopPropagation(); onCopy(copyJsonValue(item.value)); }} title={t("canvasflow.copyNode")}><span className="text-[10px]">⧉</span></button>
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
  return <><defs><linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="0%"><stop offset="0%" stopColor={color} /><stop offset="100%" stopColor="#f8fafc" /></linearGradient></defs><path d={path} fill="none" stroke={color} strokeWidth={selected ? 5 : 3} opacity={selected ? 0.2 : 0.12} /><path d={path} fill="none" stroke={`url(#${gradientId})`} strokeWidth={selected ? 2.2 : 1.4} strokeLinecap="round" markerEnd={`url(#arrow-${gradientId})`} /><marker id={`arrow-${gradientId}`} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#f8fafc" /></marker></>;
});

function JsonFlowInner({ value, selectedPath, searchMatches, onSelectPath, onCopy, collapseAllToken }: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void; collapseAllToken: number }) {
  const { t } = useTranslation();
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
  return <div className="relative h-full min-h-0"><ReactFlow nodes={nodes} edges={edges} nodeTypes={{ jsonNode: JsonFlowNode }} edgeTypes={{ color: ColorEdge }} onNodesChange={(changes) => setNodes((cur) => applyNodeChanges(changes, cur))} fitView minZoom={0.15} maxZoom={2.2} nodesDraggable nodesConnectable={false} elementsSelectable proOptions={{ hideAttribution: true }}><Background color="#1e293b" gap={24} size={1} /><MiniMap style={{ backgroundColor: "var(--color-surface-deep)", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95" nodeColor={(node) => hashColor(String(node.id), JSON_EDGE_COLORS)} nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2, 6, 23, 0.72)" pannable zoomable /><Controls className="canvas-flow-controls" showInteractive={false} /></ReactFlow>{graphTruncated && <div className="pointer-events-none absolute left-3 top-3 z-10 rounded border border-amber-400/20 bg-slate-900/90 px-2 py-1 text-[10px] text-amber-200">{t("canvasflow.graphTruncated", { count: MAX_JSON_FLOW_ITEMS })}</div>}</div>;
}

export function JsonFlowCanvas(props: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void; collapseAllToken: number }) {
  return <div className="h-full min-h-0 bg-slate-950"><ReactFlowProvider><JsonFlowInner {...props} /></ReactFlowProvider></div>;
}
