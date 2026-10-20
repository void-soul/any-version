import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  useReactFlow,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeChange,
  type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ChevronDown, ChevronRight, LayoutGrid, Rows3 } from "lucide-react";
import { EDGE_STYLES, edgePath, useEdgeStyle, type EdgeStyle } from "../utils/graphEdgeStyle";
import {
  DIM_MAX, EDGE_FLOW_CLASS, EDGE_GLOW_CLASS, HL_BOOST_MAX,
  dimmedEdgeStroke, dimmedEdgeWidth, dimmedOpacity, edgeGlow, hasVisibleHighlight,
  headerTint, highlightColor, neonRing, neonTextGlow, relatedEdgeWidth,
  useGraphDepthPref, useGraphHighlightPrefs,
} from "../utils/graphHighlight";
import { graphScope, hopFade } from "../utils/graphScope";
import { treeLayout } from "../utils/treeLayout";
import EdgeStyleSelect from "./shared/EdgeStyleSelect";
import GraphLookPrefs from "./shared/GraphLookPrefs";
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
  /** 与选中项相连（层级由「高亮层级」偏好决定，默认直接父 / 直接子）：点亮但比选中弱一档 */
  related?: boolean;
  /** 选中了别人而自己不在其关联范围：压暗 */
  dimmed?: boolean;
  /** 高亮强度倍率（画布外观浮层）：环 / 辉光 / 表头底色按它缩放 */
  hlBoost?: number;
  /** 无关项透明度（按「淡化强度」算出来，不再写死 opacity-35） */
  dimOpacity?: number;
  onSelect: (path: string) => void;
  onToggle: (path: string) => void;
  onCopy: (value: string) => void;
};

const JSON_EDGE_COLORS = ["#22d3ee", "#a78bfa", "#34d399", "#fbbf24", "#fb7185", "#60a5fa"];

/** 卡片尺寸：与下面 JsonFlowNode 的 w-[250px] 对齐；高度是折叠态的估计值 */
const JSON_CARD_W = 250;
const JSON_CARD_H = 90;
/** 层距 = 卡宽 + 走廊（与思维导图同一套取法，只是卡片更宽） */
const JSON_LEVEL_GAP = 56;
const JSON_STACK_GAP = 22;

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
      <div className="border-b border-white/10 px-2 py-1 text-micro font-semibold uppercase tracking-wide text-cyan-300">{t("canvasflow.arrContent", { count: value.length })}</div>
      <div className="max-h-36 overflow-auto">
        <table className="w-full table-fixed border-collapse text-left font-mono text-micro">
          <thead><tr>{objectRows && <th className="sticky top-0 w-6 border-b border-white/10 bg-slate-900 px-1.5 py-1 text-slate-600">#</th>}{columns.map((column) => <th key={column} className="sticky top-0 max-w-[100px] border-b border-white/10 bg-slate-900 px-1.5 py-1 text-cyan-300">{column}</th>)}</tr></thead>
          <tbody>{displayRows.map((row, index) => <tr key={`${path}.${index}`} className="hover:bg-white/[0.05]" onClick={() => onSelect(`${path}.${index}`)}>{objectRows && <td className="border-b border-white/5 px-1.5 py-1 text-slate-600">{index + 1}</td>}{columns.map((column) => { const cell = objectRows && typeof row === "object" && row !== null && !Array.isArray(row) ? row[column] ?? null : row; return <td key={column} className="max-w-[100px] truncate border-b border-white/5 px-1.5 py-1 text-slate-300" title={JSON.stringify(cell)}>{compactJsonValue(cell)}</td>; })}</tr>)}</tbody>
        </table>
        {value.length > 80 && <div className="border-t border-white/10 px-2 py-1 text-micro text-slate-600">{t("canvasflow.showFirst80")}</div>}
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
  const { item, selectedPath, searchMatches, collapsed, related, dimmed, hlBoost, dimOpacity, onSelect, onToggle, onCopy } = data;
  const selected = item.path === selectedPath;
  const chain = searchMatches.paths.has(item.path);
  const directMatch = searchMatches.directPaths.has(item.path);
  const container = isJsonContainer(item.value);
  const color = hashColor(item.id, JSON_EDGE_COLORS);
  /**
   * 高亮：跟随节点自己的颜色（JSON 节点色由路径哈希得来，天然有色），
   * 与 ER 设计、思维导图共用 graphHighlight —— 选中更亮、关联稍弱、其余压暗。
   * 搜索命中的那条链（chain）优先级低于高亮：两者抢同一个「亮」通道时，
   * 先看选中关系，否则用户选完节点就被搜索结果盖掉了。
   */
  const hlLevel = selected ? "select" : related ? "related" : null;
  const hlColor = hlLevel ? highlightColor(color, hlLevel) : "";
  const hlRing = hlLevel ? neonRing(hlColor, hlLevel, hlBoost) : "";
  const hlGlow = hlLevel ? neonTextGlow(hlColor, hlLevel, hlBoost) : "";
  const hlHeader = hlLevel ? headerTint(hlColor, hlLevel, hlBoost) : "";
  /** 无关项压暗：强度 0 时完全不让步（等同不淡化），拉到 200% 才彻底看不见 */
  const dimStyle = dimmed && (dimOpacity ?? 1) < 0.999 ? { opacity: dimOpacity } : undefined;
  return (
    <div className={`w-[250px] rounded-ctl border bg-surface-modal px-2.5 py-2 shadow-xl ${hlLevel ? "" : chain ? "border-cyan-700/80" : "border-white/10"}`}
      style={{ ...dimStyle, ...(hlLevel ? { borderColor: hlColor, boxShadow: hlRing } : null) }}
      onClick={() => onSelect(item.path)}>
      <Handle type="target" position={Position.Left} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
      <div className="flex items-center gap-1.5">
        {container && <button type="button" className="nodrag nopan inline-flex h-4 w-4 items-center justify-center text-slate-500 hover:text-white" onClick={(event) => { event.stopPropagation(); onToggle(item.path); }} title={collapsed.has(item.path) ? t("canvasflow.expand") : t("canvasflow.collapse")}>{collapsed.has(item.path) ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}</button>}
        <span className={`min-w-0 flex-1 truncate font-mono text-caption ${directMatch && !hlLevel ? "text-yellow-300" : "text-cyan-200"}`} style={{ textShadow: hlGlow || undefined }}>{item.name}</span>
        <button type="button" className="nodrag nopan text-slate-600 hover:text-white" onClick={(event) => { event.stopPropagation(); onCopy(copyJsonValue(item.value)); }} title={t("canvasflow.copyNode")}><span className="text-tiny">⧉</span></button>
      </div>
      <div className="mt-1 truncate font-mono text-tiny" style={{ color, backgroundColor: hlHeader || undefined, borderRadius: hlHeader ? 3 : undefined }}>{jsonSummary(item.value)}</div>
      {Array.isArray(item.value) && !collapsed.has(item.path) && <JsonArrayTable value={item.value} path={item.path} onSelect={onSelect} />}
      <Handle type="source" position={Position.Right} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
    </div>
  );
});

/** 连线的高亮外观（由下面 `edges` 那处统一算好后经 data 传进来）：
 *  `rel` 关联 · `dim` 无关 · `stroke` 无关时的线色（按淡化强度算）·
 *  `width` 线宽（关联时已按跳数退过亮度）· `glow` 关联时的同色辉光 ·
 *  `arrow` 强度为 0 时为 false，箭头与线宽一起收掉。 */
function edgeLook(data: Record<string, unknown> | undefined) {
  return {
    rel: !!(data?.rel as boolean | undefined),
    dim: !!(data?.dim as boolean | undefined),
    stroke: data?.stroke as string | undefined,
    width: data?.width as number | undefined,
    glow: data?.glow as string | undefined,
    arrow: data?.arrow !== false,
  };
}

const ColorEdge = memo(function ColorEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps) {
  // 自定义 edge 拿不到组件 state，样式由每条边的 `data.style` 传进来（缺失 = 曲线）
  const style = (data?.style as EdgeStyle | undefined);
  const [path] = edgePath(style && EDGE_STYLES.includes(style) ? style : "bezier", { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  const color = data?.color as string | undefined ?? "#22d3ee";
  /** 关联边：加粗 + 同色辉光 + 流转动画；无关边：整条压暗（光晕一起淡，否则像有鬼影） */
  const { rel, dim, stroke, width, glow, arrow } = edgeLook(data);
  const gradientId = `flow-edge-${id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const lineWidth = width ?? (rel ? 2.2 : 1.4);
  return <><defs><linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="0%"><stop offset="0%" stopColor={color} /><stop offset="100%" stopColor="#f8fafc" /></linearGradient></defs><path d={path} fill="none" stroke={color} strokeWidth={rel ? lineWidth * 2.2 : 3} opacity={dim ? 0.03 : rel ? 0.22 : 0.12} className={EDGE_GLOW_CLASS} style={glow ? { filter: glow } : undefined} /><path d={path} fill="none" stroke={dim && stroke ? stroke : `url(#${gradientId})`} strokeWidth={lineWidth} strokeLinecap="round" markerEnd={arrow ? `url(#arrow-${gradientId})` : undefined} opacity={dim ? 0.4 : 1} className={EDGE_FLOW_CLASS} /><marker id={`arrow-${gradientId}`} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#f8fafc" /></marker></>;
});

function JsonFlowInner({ value, selectedPath, searchMatches, onSelectPath, onCopy, collapseAllToken }: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void; collapseAllToken: number }) {
  const { t } = useTranslation();
  const { fitView } = useReactFlow();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [nodes, setNodes] = useState<Node<JsonFlowNodeData>[]>([]);
  /** 用户手动排过版之后不再被自动布局覆盖 —— 否则点一次「自动布局」，
   *  下一个节点进来就把整张图推倒重来，手工整理白做。 */
  const manualPositions = useRef(new Set<string>());
  const [autoLayoutToken, setAutoLayoutToken] = useState(0);
  /** 节点实测尺寸（ReactFlow 测完才给）。单独存一份，让自动布局不必依赖 `nodes` ——
   *  依赖 `nodes` 会形成 `nodes → 布局 → computedNodes → nodes` 的渲染回环。 */
  const [measuredSizes, setMeasuredSizes] = useState<Map<string, { width: number; height: number }>>(
    () => new Map(),
  );
  const handleNodesChange = useCallback((changes: NodeChange<Node<JsonFlowNodeData>>[]) => {
    setNodes((current) => applyNodeChanges(changes, current));
    const dims = changes.filter((change) => change.type === "dimensions");
    if (dims.length === 0) return;
    setMeasuredSizes((prev) => {
      const next = new Map(prev);
      let changed = false;
      for (const change of dims) {
        if (change.type !== "dimensions" || !change.dimensions) continue;
        const old = next.get(change.id);
        if (!old || old.width !== change.dimensions.width || old.height !== change.dimensions.height) {
          next.set(change.id, { width: change.dimensions.width, height: change.dimensions.height });
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, []);
  // 连线样式（曲线 / 直角折线 / 直线）：与 ER 设计、思维导图共用一份偏好
  const [edgeStyle, setEdgeStyle] = useEdgeStyle();
  // 画布外观三滑块：与另两个画布共用算法与面板，偏好按 `jsonflow` 隔离
  const { prefs: lookPrefs, setHighlight: setHlBoost, setDim: setDimStrength, reset: resetLook } = useGraphHighlightPrefs("jsonflow");
  const { depth: hlDepth, setDepth: setHlDepth } = useGraphDepthPref("jsonflow");
  const allItems = useMemo(() => buildJsonItems(value), [value]);
  const graphTruncated = allItems.length >= MAX_JSON_FLOW_ITEMS;
  const previousCollapseToken = useRef(0);
  useEffect(() => {
    if (collapseAllToken === previousCollapseToken.current) return;
    setCollapsed(new Set(allItems.filter((item) => isJsonContainer(item.value)).map((item) => item.path)));
    previousCollapseToken.current = collapseAllToken;
  }, [allItems, collapseAllToken]);
  const visibleItems = useMemo(() => allItems.filter((item) => { let current = item.parentId; while (current) { if (collapsed.has(current)) return false; current = allItems.find((candidate) => candidate.id === current)?.parentId ?? null; } return true; }), [allItems, collapsed]);
  /**
   * 关联集合 + 关联连线集合：**一次 BFS 同时得出**（共享层 `utils/graphScope`），
   * 与数据库设计器、思维导图同一口径。JSON 画布是纯父子树，所以「走远」就是沿
   * 祖先链 / 后代链往外扩；层级默认 1 跳（直接父 + 直接子），可调到 5 跳。
   *
   * 边也必须是同一次遍历的产物：早先按「两端有一端在集合内」判定，会点亮同跳层
   * 内部的横向连线，产出「线亮着、节点却暗着」。
   */
  const scope = useMemo(() => {
    const scopeEdges = allItems.flatMap((item) => (item.parentId ? [{ id: `json-edge-${item.id}`, source: item.parentId, target: item.id }] : []));
    return graphScope(allItems.map((item) => item.id), scopeEdges, selectedPath || null, hlDepth);
  }, [allItems, selectedPath, hlDepth]);
  const relatedPaths = useMemo(() => new Set(scope.hops.keys()), [scope]);
  /** 每条连线离选中项几跳：远的那端画得更暗，否则层级调到 3 跳与 1 跳画出来一样 */
  const hopOfEdge = useCallback(
    (a: string, b: string) => {
      if (!selectedPath) return undefined;
      const hopA = scope.hops.get(a);
      const hopB = scope.hops.get(b);
      return hopA === undefined ? hopB : hopB === undefined ? hopA : Math.min(hopA, hopB);
    },
    [scope, selectedPath],
  );
  /** 自动布局：与思维导图共用一套层级树算法（父节点居中于自己的子节点）。
   *  早先这里是 `x = depth * 260` / `y = 同深度第几个 * 92` 的硬编码网格：父节点与
   *  它的子节点在堆叠轴上毫无关系，键一多整张图就散成一片竖条，看不出谁是谁的父。
   *
   *  **不能依赖 `nodes`**：这个 memo 的结果会被 `computedNodes` 消费，而
   *  `computedNodes` 又要写回 `nodes` —— 依赖 `nodes` 就是 `nodes → 布局 → nodes`
   *  的渲染回环（打开画布即无限重渲染）。实测尺寸单独放在 `measuredSizes` 里，只有
   *  尺寸真的变化时才更新。 */
  const autoLayout = useMemo(() => {
    return treeLayout(visibleItems.map((item) => ({ id: item.id, parentId: item.parentId })), "lr", measuredSizes,
      { cardW: JSON_CARD_W, cardH: JSON_CARD_H, levelGap: JSON_LEVEL_GAP, stackGap: JSON_STACK_GAP });
  }, [visibleItems, measuredSizes]);
  /** 当前各节点位置：查表 O(1)。早先把 `nodes.find` 放进逐节点的 map 里，1800 节点就是 O(n²) */
  const posById = useMemo(() => new Map(nodes.map((n) => [n.id, n.position])), [nodes]);
  /** 位置来源：用户手动拖过的沿用，否则取自动布局（首次进入画布就是自动布局的结果） */
  const positionOf = useCallback(
    (id: string) => (manualPositions.current.has(id) ? posById.get(id) : undefined) ?? autoLayout.get(id) ?? { x: 0, y: 0 },
    [autoLayout, posById],
  );
  const computedNodes = useMemo<Node<JsonFlowNodeData>[]>(() => visibleItems.map((item) => ({
    id: item.id, type: "jsonNode", position: positionOf(item.id),
    data: {
      item, selectedPath, searchMatches, collapsed,
      related: relatedPaths.has(item.path),
      dimmed: !!selectedPath && !relatedPaths.has(item.path),
      hlBoost: lookPrefs.highlight,
      dimOpacity: dimmedOpacity(lookPrefs.dim),
      onSelect: onSelectPath,
      onToggle: (path) => setCollapsed((current) => { const next = new Set(current); next.has(path) ? next.delete(path) : next.add(path); return next; }),
      onCopy,
    },
    sourcePosition: Position.Right, targetPosition: Position.Left,
  })), [onCopy, onSelectPath, positionOf, searchMatches, selectedPath, visibleItems, collapsed, relatedPaths, lookPrefs]);
  const edges = useMemo<Edge[]>(() => {
    const hl = lookPrefs.highlight;
    const dim = lookPrefs.dim;
    return visibleItems.flatMap((item) => {
      if (!item.parentId || !visibleItems.some((candidate) => candidate.id === item.parentId)) return [];
      const id = `json-edge-${item.id}`;
      // 判据只认 BFS 实际走过的边；亮度随跳数退（否则层级调到 3 跳与 1 跳画出来一样）
      const rel = !!selectedPath && scope.edgeIds.has(id);
      const fade = rel ? hopFade(hopOfEdge(item.parentId as string, item.id)) : 1;
      return [{
        id, source: item.parentId, target: item.id, type: "color", animated: rel, zIndex: rel ? 10 : 0,
        data: {
          color: hashColor(item.id, JSON_EDGE_COLORS), style: edgeStyle,
          rel, dim: !!selectedPath && !rel,
          stroke: !rel ? dimmedEdgeStroke(dim) : undefined,
          width: rel ? relatedEdgeWidth(hl) * fade : dimmedEdgeWidth(dim),
          glow: rel ? edgeGlow(hashColor(item.parentId as string, JSON_EDGE_COLORS), hl * fade) : undefined,
          // 箭头与线宽走同一判据：强度 0 时线已看不见，满屏实心三角反而成了「只亮线」
          arrow: !(rel && !hasVisibleHighlight(hl)),
        },
        markerEnd: rel && !hasVisibleHighlight(hl) ? undefined : { type: MarkerType.ArrowClosed, color: "#f8fafc" },
      }];
    });
  }, [visibleItems, edgeStyle, selectedPath, scope, hopOfEdge, lookPrefs]);
  // 保持拖放位置：computed nodes 更新时，已有位置的节点保持当前位置。
  // **内容与位置都没变时必须返回原数组引用**：否则 setNodes 每轮都产出新数组 →
  // nodes 变 → posById/computedNodes 重算 → 再次 setNodes，形成无限渲染。
  // data 按字段逐个比对后复用旧引用 —— `computedNodes` 每次都会新建 data 对象，
  // 直接比引用永远不相等，回环照旧。
  useEffect(() => {
    setNodes((current) => {
      const currentById = new Map(current.map((n) => [n.id, n]));
      const rebuilt = computedNodes.map((next) => {
        const existing = currentById.get(next.id);
        if (!existing) return next;
        const a = existing.data;
        const b = next.data;
        const dataSame = a.item === b.item && a.selectedPath === b.selectedPath
          && a.searchMatches === b.searchMatches && a.collapsed === b.collapsed
          && a.related === b.related && a.dimmed === b.dimmed
          && a.hlBoost === b.hlBoost && a.dimOpacity === b.dimOpacity
          && a.onSelect === b.onSelect && a.onCopy === b.onCopy;
        return { ...next, data: dataSame ? a : b, position: existing.position };
      });
      const same = rebuilt.length === current.length && rebuilt.every((next, index) => {
        const old = current[index];
        return !!old && old.id === next.id && old.data === next.data
          && old.position.x === next.position.x && old.position.y === next.position.y;
      });
      return same ? current : rebuilt;
    });
  }, [computedNodes]);
  // JSON 内容变化时重置位置（整张图换了，用户手拖过的坐标也一并作废）
  useEffect(() => { setNodes(computedNodes); manualPositions.current.clear(); }, [value]);
  useEffect(() => { const timer = window.setTimeout(() => fitView({ padding: 0.2, duration: 240 }), 0); return () => window.clearTimeout(timer); }, [fitView, value, collapsed, autoLayoutToken]);
  /** 自动布局：清掉手动位置，所有节点回到层级树算出来的坐标。 */
  const resetToAutoLayout = useCallback(() => {
    manualPositions.current.clear();
    setNodes((current) => current.map((n) => ({ ...n, position: autoLayout.get(n.id) ?? n.position })));
    setAutoLayoutToken((token) => token + 1);
  }, [autoLayout]);
  /** 「整理选中」：只重排高亮范围，其余节点一概不动（与另两个画布同一语义）。
   *  集合直接复用高亮作用域 —— 标签、提示与实现说的必须是同一件事。
   *  **只在点按钮时执行**：早先写成「选中即自动重排」的 useEffect，用户一选节点整块图就跳走。 */
  const arrangeSelected = useCallback(() => {
    if (!selectedPath || relatedPaths.size < 2) return;
    const ids = new Set(relatedPaths);
    const subset = visibleItems.filter((item) => ids.has(item.id));
    const laid = treeLayout(subset.map((item) => ({ id: item.id, parentId: item.parentId })), "lr", measuredSizes,
      { cardW: JSON_CARD_W, cardH: JSON_CARD_H, levelGap: JSON_LEVEL_GAP, stackGap: JSON_STACK_GAP });
    // 整体平移回原区域左上角：不这么做，这批卡片会跳到画布原点，用户以为整张图被重排了
    let oldX = Infinity; let oldY = Infinity; let newX = Infinity; let newY = Infinity;
    for (const item of subset) {
      const now = posById.get(item.id); const next = laid.get(item.id);
      if (!now || !next) continue;
      oldX = Math.min(oldX, now.x); oldY = Math.min(oldY, now.y);
      newX = Math.min(newX, next.x); newY = Math.min(newY, next.y);
    }
    if (!Number.isFinite(oldX) || !Number.isFinite(newX)) return;
    const dx = oldX - newX; const dy = oldY - newY;
    const arranged = new Map(subset.map((item) => [item.id, { x: (laid.get(item.id)?.x ?? 0) + dx, y: (laid.get(item.id)?.y ?? 0) + dy }]));
    setNodes((current) => current.map((n) => (arranged.has(n.id) ? { ...n, position: arranged.get(n.id)! } : n)));
    for (const id of arranged.keys()) manualPositions.current.add(id);
    setAutoLayoutToken((token) => token + 1);
  }, [selectedPath, relatedPaths, visibleItems, measuredSizes, posById]);
  const arrangeDisabled = !selectedPath || relatedPaths.size < 2;
  return <div className="relative h-full min-h-0"><ReactFlow nodes={nodes} edges={edges} nodeTypes={{ jsonNode: JsonFlowNode }} edgeTypes={{ color: ColorEdge }} onNodesChange={handleNodesChange} onNodeDrag={(_e, n) => { manualPositions.current.add(n.id); setNodes((cur) => cur.map((x) => (x.id === n.id ? { ...x, position: n.position } : x))); }} onPaneClick={() => onSelectPath("")} fitView minZoom={0.15} maxZoom={2.2} nodesDraggable nodesConnectable={false} elementsSelectable proOptions={{ hideAttribution: true }}><Background color="#1e293b" gap={24} size={1} /><MiniMap style={{ backgroundColor: "var(--color-surface-deep)", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95" nodeColor={(node) => hashColor(String(node.id), JSON_EDGE_COLORS)} nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2, 6, 23, 0.72)" pannable zoomable /><Controls className="canvas-flow-controls" showInteractive={false} /></ReactFlow>{graphTruncated && <div className="pointer-events-none absolute left-3 top-3 z-10 rounded border border-amber-400/20 bg-slate-900/90 px-2 py-1 text-tiny text-amber-200">{t("canvasflow.graphTruncated", { count: MAX_JSON_FLOW_ITEMS })}</div>}{/* 连线样式：与 ER 设计、思维导图共用一份偏好。
      位置刻意**贴着左下角的缩放按钮**并与之同宽对齐 —— 原来它在右下角，
      和 Controls 隔了整个画布宽，两个「画布控件」散在对角，找起来要来回扫。
      left-0 / w-[30px] 对齐 RF 的 `.react-flow__panel.bottom.left`（Controls 落在 left:0，
      按钮 30px 见 App.css）；bottom-[104px] 是量出来的 —— Controls 三颗钮 30px + 上下边框 ≈ 92px，
      留 12px 间隙不压住它。 */}<div className="absolute bottom-[104px] left-0 z-10 flex w-[30px] justify-center"><EdgeStyleSelect value={edgeStyle} onChange={setEdgeStyle} className="rounded border border-white/10 bg-slate-900/80 px-1 py-1 text-tiny text-slate-300 shadow backdrop-blur outline-none hover:bg-slate-900" /></div>{/* 画布外观浮层：三滑块 + 两个「把画面收拾一下」的动作。
      它比连线样式选择器再往上一格（bottom-[152px]）：下面那颗选择器占掉 104~134，
      浮层面板宽 220px、展开后往右长，不会与它重叠。 */}
    <div className="absolute bottom-[152px] left-0 z-10 flex w-[30px] justify-center"><GraphLookPrefs highlight={lookPrefs.highlight} dim={lookPrefs.dim} onHighlight={setHlBoost} onDim={setDimStrength} depth={hlDepth} onDepth={setHlDepth} onReset={resetLook} maxHighlight={HL_BOOST_MAX} maxDim={DIM_MAX} extra={<div className="grid grid-cols-2 gap-1"><button type="button" onClick={resetToAutoLayout} title={t("canvasflow.autoLayoutHint")} className="flex cursor-pointer flex-col items-center gap-1 rounded-ctl bg-white/5 py-1.5 text-micro text-slate-300 transition hover:bg-white/10 hover:text-slate-100"><LayoutGrid className="h-3.5 w-3.5" /><span className="leading-tight">{t("canvasflow.autoLayout")}</span></button><button type="button" onClick={arrangeSelected} disabled={arrangeDisabled} title={t("canvasflow.arrangeHint")} className="flex cursor-pointer flex-col items-center gap-1 rounded-ctl bg-white/5 py-1.5 text-micro text-slate-300 transition hover:bg-white/10 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-40"><Rows3 className="h-3.5 w-3.5" /><span className="leading-tight">{t("canvasflow.arrangeSelected")}</span></button></div>} /></div></div>;
}

export function JsonFlowCanvas(props: { value: JsonValue; selectedPath: string; searchMatches: SearchMatches; onSelectPath: (path: string) => void; onCopy: (value: string) => void; collapseAllToken: number }) {
  return <div className="h-full min-h-0 bg-slate-950"><ReactFlowProvider><JsonFlowInner {...props} /></ReactFlowProvider></div>;
}
