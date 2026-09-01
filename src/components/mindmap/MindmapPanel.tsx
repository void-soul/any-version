import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, ReactFlowProvider, getBezierPath, useReactFlow,
  type Connection, type Edge, type EdgeProps, type Node, type NodeChange, type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { MindmapMarkdown } from "./MindmapMarkdown";
import { MarkdownFieldEditor } from "./MarkdownFieldEditor";
import VexEmptyState from "../VexEmptyState";
import {
  AlertTriangle, Brain, File, Folder, FolderOpen, LayoutGrid, Lightbulb, Loader2,
  ScrollText, Sparkles, StickyNote, Image, Trash2, X, Plus, Pencil, Eye,
  ChevronDown, ChevronRight, ChevronLeft, FolderPlus, Search, Maximize2, Minimize2, Code2, FileText, ListTree, Palette, RotateCcw, RotateCw, Calendar,
} from "lucide-react";
import type { AiConfig } from "../ai/types";
import { AiImportResult, DocumentFull, MindmapDocument, MindmapFolder, MindmapNode, MindmapSticker, PlannedOccurrence, PositionInput, kindColor, mmApi } from "./types";
import { moduleAccent } from "../../utils/theme";

const ACCENT = moduleAccent();
const MM_LAST_DOC_KEY = "any_version_mindmap_last_doc";
const button = "inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";
const selectClass = "h-8 min-w-[110px] rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60";
const DOC_SOURCE_ICONS: Record<string, (cls: string) => React.ReactNode> = {
  manual: (c) => <ListTree className={c} />,
  ai_project: (c) => <Code2 className={c} />,
  ai_text: (c) => <FileText className={c} />,
  task: (c) => <Brain className={c} />,
};
const STICKER_PALETTE = ["#fef3c7", "#d4f5d4", "#dbeafe", "#fce7f3", "#e9d5ff", "#fef9c3", "#ccfbf1", "#ffe4e6"];
const STICKER_ROTATIONS = [-4.5, 2.8, -1.8, 5.2, -3.2, 1.4, 4.1, -2.4];

function stickerRotation(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) | 0;
  return STICKER_ROTATIONS[Math.abs(hash) % STICKER_ROTATIONS.length];
}

function stickerSpawnPosition(index: number): { x: number; y: number } {
  // 用黄金角度把贴纸铺成轻微散开的不规则簇，避免整齐网格感。
  const angle = index * 2.39996;
  const radius = 45 + (index % 4) * 42;
  return {
    x: 170 + Math.cos(angle) * radius + (index % 2) * 24,
    y: 120 + Math.sin(angle) * radius + (index % 3) * 18,
  };
}

function stickerWidth(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i++) hash = (hash * 17 + id.charCodeAt(i)) | 0;
  return 170 + (Math.abs(hash) % 4) * 12;
}

function normalizeHexColor(value: string | null | undefined): string | null {
  const raw = value?.trim() ?? "";
  if (/^#[0-9a-fA-F]{6}$/.test(raw)) return raw;
  if (/^#[0-9a-fA-F]{3}$/.test(raw)) {
    return `#${raw.slice(1).split("").map((c) => c + c).join("")}`;
  }
  return null;
}

const effectiveNodeColor = (node: MindmapNode) => normalizeHexColor(node.color) ?? kindColor(node.kind);

/** 计划时间徽标的短文本：ISO 串 → MM-DD HH:MM */
function planShort(value: string): string {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  return `${mm}-${dd} ${hh}:${mi}`;
}

// ════════════ 节点 ════════════

type FlowNodeData = { node: MindmapNode; selected: boolean; hasChildren: boolean; collapsed: boolean; targetPosition: Position; sourcePosition: Position; onSelect: () => void; onOpenDetail: () => void; onToggle: () => void; onAddChild: () => void; onPreview: (e: React.MouseEvent) => void; onPreviewEnd: () => void; onDelete: () => void; onContextMenu: (e: React.MouseEvent) => void; };

const FlowNode = memo(function FlowNode({ data }: NodeProps<Node<FlowNodeData>>) {
  const { t } = useTranslation();
  const { node, selected, hasChildren, collapsed, targetPosition, sourcePosition, onSelect, onOpenDetail, onToggle, onAddChild, onPreview, onPreviewEnd, onDelete, onContextMenu } = data;
  const c = effectiveNodeColor(node);
  return (
    <article className={`group relative w-[200px] rounded-xl border shadow-lg transition-shadow cursor-pointer ${selected ? "shadow-cyan-500/30 ring-1 ring-cyan-400/40" : "hover:shadow-xl"}`}
      style={{ borderColor: selected ? c : `${c}55`, backgroundColor: "#0d1524" }} onClick={onSelect} onDoubleClick={(e) => { e.stopPropagation(); onOpenDetail(); }} onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e); }}>
      <Handle type="target" position={targetPosition} isConnectable className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: c }} />
      {/* 节点右上角悬浮按钮：预览（气泡）+ 删除 */}
      <div className="nodrag nopan absolute right-1 top-1 z-10 hidden items-center gap-0.5 group-hover:flex">
        <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-cyan-300"
          onMouseEnter={onPreview} onMouseMove={onPreview} onMouseLeave={onPreviewEnd} title={t("mindmap.previewOnly")}>
          <Eye className="h-3 w-3" />
        </button>
        <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-red-400"
          onClick={(e) => { e.stopPropagation(); onDelete(); }} title={t("mindmap.deleteNode")}>
          <Trash2 className="h-3 w-3" />
        </button>
      </div>
      {/* 节点右侧悬浮 + 按钮：给当前节点直接添加子节点 */}
      <button type="button" className="nodrag nopan absolute -right-2.5 top-1/2 z-10 hidden h-5 w-5 -translate-y-1/2 items-center justify-center rounded-full border transition group-hover:flex hover:scale-110"
        style={{ backgroundColor: "#0d1524", borderColor: `${c}66`, color: c, boxShadow: `0 0 8px ${c}44` }}
        onClick={(e) => { e.stopPropagation(); onAddChild(); }} title={t("mindmap.addChild")}>
        <Plus className="h-3.5 w-3.5" />
      </button>
      {/* 标题栏与描述区使用不同背景，节点信息层次保持稳定。 */}
      <div className="flex items-center gap-1.5 rounded-t-[11px] border-b px-2.5 py-2 pr-8" style={{ borderColor: `${c}35`, backgroundColor: `${c}20` }}>
        {hasChildren && <button type="button" className="nodrag nopan inline-flex h-4 w-4 items-center justify-center text-slate-400 hover:text-white" onClick={(e) => { e.stopPropagation(); onToggle(); }} title={collapsed ? t("mindmap.expand") : t("mindmap.collapse")}>
          {collapsed ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
        </button>}
        <span className="min-w-0 flex-1 truncate text-[11px] font-semibold" style={{ color: c }}>{node.name}</span>
      </div>
      <div className="mx-1.5 mt-1.5 min-h-[38px] rounded-md border border-white/[0.06] bg-slate-900/80 px-2 py-1.5 text-[9px] leading-4 text-slate-300">
        <span className={node.description ? "line-clamp-2" : "italic text-slate-600"}>{node.description || t("mindmap.noDesc")}</span>
      </div>
      <div className="flex items-center gap-1.5 px-2.5 pt-1.5">
        {node.progress > 0 && <span className="text-[8px] text-slate-500">{node.progress}%</span>}
        {node.planAt && <span className="text-[8px] text-slate-400 font-mono">{t("mindmap.planAt", { time: planShort(node.planAt) })}</span>}
        {(node.sources?.length ?? 0) > 0 && <span className="inline-flex items-center gap-0.5 text-[8px] text-cyan-300/70" title={t("mindmap.evidenceTitle", { count: node.sources!.length, names: node.sources!.join("、") })}><File className="h-2.5 w-2.5" />{node.sources!.length}</span>}
      </div>
      {node.progress > 0 && <div className="mx-2.5 mb-2 mt-1 h-1 overflow-hidden rounded-full bg-slate-800/80"><div className="h-full rounded-full transition-all" style={{ width: `${node.progress}%`, backgroundColor: c, boxShadow: `0 0 6px ${c}66` }} /></div>}
      <Handle type="source" position={sourcePosition} isConnectable className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: c }} />
    </article>
  );
});

// ════════════ 连线 ════════════

const ColorEdge = memo(function ColorEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, curvature: 0.28 });
  const color = (data?.color as string | undefined) ?? "#22d3ee";
  const gid = `mm-edge-${id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  return (<>
    <defs><linearGradient id={gid} x1="0%" y1="0%" x2="100%" y2="0%"><stop offset="0%" stopColor={color} /><stop offset="100%" stopColor="#f8fafc" /></linearGradient></defs>
    <path d={path} fill="none" stroke={color} strokeWidth={3} opacity={0.12} />
    <path d={path} fill="none" stroke={`url(#${gid})`} strokeWidth={1.5} strokeLinecap="round" markerEnd={`url(#arrow-${gid})`} />
    <marker id={`arrow-${gid}`} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#f8fafc" /></marker>
  </>);
});

// ════════════ 贴纸节点 ════════════

type StickerNodeData = {
  sticker: MindmapSticker;
  onUpdate: (patch: { content?: string; color?: string; imageData?: string; rotation?: number }) => void;
  onRotate: (delta: number) => void;
  onReplaceImage: () => void;
  onDelete: () => void;
};

const StickerFlowNode = memo(function StickerFlowNode({ data }: NodeProps<Node<StickerNodeData>>) {
  const { sticker, onUpdate, onRotate, onReplaceImage, onDelete } = data;
  const bg = sticker.color || "#fef3c7";
  const { t } = useTranslation();
  const rotation = sticker.rotation ?? stickerRotation(sticker.id);
  const isImage = Boolean(sticker.imageData);
  return (<div className="group relative border border-black/10 p-3 transition-shadow hover:z-10 hover:shadow-2xl" style={{
    width: isImage ? Math.max(stickerWidth(sticker.id), 220) : stickerWidth(sticker.id),
    backgroundColor: bg,
    transform: `rotate(${rotation}deg)`,
    borderRadius: "2px 7px 3px 1px",
    boxShadow: "2px 4px 10px rgba(0,0,0,.28), inset 0 0 0 1px rgba(255,255,255,.22)",
  }}>
    <span className="pointer-events-none absolute -left-1.5 -top-1.5 h-4 w-4 rotate-[-18deg] rounded-sm bg-white/75 shadow-sm" aria-hidden="true" />
    <span className="pointer-events-none absolute -right-1.5 -top-1 h-4 w-4 rotate-[14deg] rounded-sm bg-white/65 shadow-sm" aria-hidden="true" />
    <div className="nodrag nopan absolute right-1 top-1 z-10 hidden items-center gap-0.5 group-hover:flex">
      <button type="button" className="rounded p-1 text-slate-500 hover:bg-black/10 hover:text-slate-800" onClick={(e) => { e.stopPropagation(); onRotate(-5); }} title={t("mindmap.rotateCcw")}><RotateCcw className="h-3 w-3" /></button>
      <button type="button" className="rounded p-1 text-slate-500 hover:bg-black/10 hover:text-slate-800" onClick={(e) => { e.stopPropagation(); onRotate(5); }} title={t("mindmap.rotateCw")}><RotateCw className="h-3 w-3" /></button>
      {isImage && <button type="button" className="rounded p-1 text-slate-500 hover:bg-black/10 hover:text-slate-800" onClick={(e) => { e.stopPropagation(); onReplaceImage(); }} title={t("mindmap.replaceImage")}><Image className="h-3 w-3" /></button>}
      <button type="button" className="rounded p-1 text-slate-400 hover:bg-black/10 hover:text-red-500" onClick={(e) => { e.stopPropagation(); onDelete(); }} title={t("mindmap.deleteSticker")}><X className="h-3 w-3" /></button>
    </div>
    {isImage ? (
      <>
        <div className="flex min-h-[80px] items-center justify-center overflow-hidden border border-black/10 bg-white/35">
          <img src={sticker.imageData} alt={sticker.content || t("mindmap.imageStickerAlt")} className="block max-h-[180px] max-w-full object-contain" draggable={false} />
        </div>
        <textarea className="nodrag nowheel mt-2 w-full resize-none bg-transparent text-[10px] leading-4 text-slate-800 outline-none" value={sticker.content}
          onChange={(e) => onUpdate({ content: e.target.value })} rows={2} placeholder={t("mindmap.imageCaptionPh")} />
      </>
    ) : (
      <textarea className="nodrag nowheel w-full resize-none bg-transparent text-[10px] leading-4 text-slate-800 outline-none" value={sticker.content}
        onChange={(e) => onUpdate({ content: e.target.value })} rows={3} placeholder={t("mindmap.stickerPh")} style={{ minHeight: 50 }} />
    )}
    {/* 贴纸颜色：预设淡色 + 自定义取色器（与节点同一套机制，颜色更浅） */}
    <div className="mt-1.5 flex items-center gap-1 opacity-60 transition group-hover:opacity-100">
      {STICKER_PALETTE.slice(0, 6).map(cl => <button key={cl} type="button" className="h-3.5 w-3.5 rounded-full border border-black/20" style={{ backgroundColor: cl, boxShadow: bg === cl ? `0 0 0 1.5px rgba(0,0,0,.45)` : "none" }}
        onClick={(e) => { e.stopPropagation(); onUpdate({ color: cl }); }} />)}
      <label className="relative inline-flex h-3.5 w-3.5 cursor-pointer items-center justify-center overflow-hidden rounded-full border border-black/25" title={t("mindmap.customColor")}>
        <input type="color" value={/^#[0-9a-fA-F]{6}$/.test(bg) ? bg : "#fef3c7"} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
          onChange={(e) => { e.stopPropagation(); onUpdate({ color: e.target.value }); }} />
        <span className="h-2 w-2 rounded-full" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
      </label>
    </div>
  </div>);
});

// ════════════ 通用确认弹窗（强调/标题色由调用方按模块主题传入，不写死固定色）════════════

function ConfirmModal({ title, message, accent, confirmText = "mindmap.confirmDeleteBtn", onConfirm, onClose }: { title: string; message: string; accent: string; confirmText?: string; onConfirm: () => void; onClose: () => void }) {
  const { t } = useTranslation();
  const confirmLabel = String(confirmText).includes(".") ? (t as any)(confirmText) : confirmText;
  return createPortal(
    <div className="fixed inset-0 z-[210] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
      <div className="w-[360px] overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-2 border-b border-white/10 px-4 py-3">
          <AlertTriangle className="h-4 w-4 shrink-0" style={{ color: accent }} />
          <h3 className="text-sm font-semibold text-white">{title}</h3>
        </div>
        <div className="px-4 py-3 text-[11px] leading-5 text-slate-300">{message}</div>
        <div className="flex justify-end gap-2 px-4 pb-4">
          <button type="button" className="rounded-md border border-white/10 bg-white/[0.05] px-4 py-1.5 text-[11px] text-slate-300 hover:bg-white/10 hover:text-white" onClick={onClose}>{t("mindmap.cancel")}</button>
          <button type="button" className="rounded-md px-4 py-1.5 text-[11px] font-semibold text-white" style={{ backgroundColor: "#ef4444", boxShadow: `0 0 14px ${accent}66` }} onClick={onConfirm}>{confirmLabel}</button>
        </div>
      </div>
    </div>, document.body);
}

// ════════════ 自动布局 ════════════

/** 布局方向：lr=左→右（默认，根在左） rl=右→左 tb=上→下 bt=下→上 */
type LayoutDir = "lr" | "rl" | "tb" | "bt";
const LAYOUT_DIR_KEYS: Record<LayoutDir, string> = { lr: "mindmap.dirLr", rl: "mindmap.dirRl", tb: "mindmap.dirTb", bt: "mindmap.dirBt" };
const isLayoutDir = (v: string): v is LayoutDir => v === "lr" || v === "rl" || v === "tb" || v === "bt";

type BackgroundTexture = "none" | "grid" | "dots" | "diagonal" | "cross" | "paper";
const BACKGROUND_TEXTURE_KEYS: Record<BackgroundTexture, string> = {
  none: "mindmap.texNone",
  grid: "mindmap.texGrid",
  dots: "mindmap.texDots",
  diagonal: "mindmap.texDiagonal",
  cross: "mindmap.texCross",
  paper: "mindmap.texPaper",
};

const BACKGROUND_TEXTURE_STYLES: Record<BackgroundTexture, React.CSSProperties> = {
  none: { backgroundColor: "#080f1c" },
  grid: {
    backgroundColor: "#080f1c",
    backgroundImage: "linear-gradient(rgba(100,116,139,.12) 1px, transparent 1px), linear-gradient(90deg, rgba(100,116,139,.12) 1px, transparent 1px)",
    backgroundSize: "24px 24px",
  },
  dots: {
    backgroundColor: "#080f1c",
    backgroundImage: "radial-gradient(rgba(100,116,139,.28) 1px, transparent 1px)",
    backgroundSize: "24px 24px",
  },
  diagonal: {
    backgroundColor: "#080f1c",
    backgroundImage: "repeating-linear-gradient(135deg, rgba(100,116,139,.10) 0 1px, transparent 1px 14px)",
    backgroundSize: "14px 14px",
  },
  cross: {
    backgroundColor: "#080f1c",
    backgroundImage: "linear-gradient(rgba(100,116,139,.12) 1px, transparent 1px), linear-gradient(90deg, rgba(100,116,139,.12) 1px, transparent 1px)",
    backgroundSize: "12px 12px",
  },
  paper: {
    backgroundColor: "#111827",
    backgroundImage: "radial-gradient(rgba(148,163,184,.10) .7px, transparent .8px), radial-gradient(rgba(15,23,42,.18) .7px, transparent .8px)",
    backgroundPosition: "0 0, 7px 8px",
    backgroundSize: "15px 15px",
  },
};

function layoutTree(nodes: MindmapNode[], dir: LayoutDir = "lr"): Map<string, { x: number; y: number }> {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const n of nodes) {
    if (n.parentId && byId.has(n.parentId) && n.parentId !== n.id) {
      const l = children.get(n.parentId) ?? []; l.push(n.id); children.set(n.parentId, l);
    } else { roots.push(n.id); }
  }
  const depth = new Map<string, number>();
  const order: string[] = [];
  // 带 visited 防环：AI 生成的节点若存在循环引用（A→B→A），
  // 无保护会无限递归导致画布打开时程序卡死。
  const visited = new Set<string>();
  const dfs = (id: string, d: number) => {
    if (visited.has(id)) return;
    visited.add(id); depth.set(id, d); order.push(id);
    for (const c of children.get(id) ?? []) dfs(c, d + 1);
  };
  for (const r of roots) dfs(r, 0);
  // 未被根遍历到的节点（环内）兜底放入布局，避免遗漏
  for (const n of nodes) { if (!visited.has(n.id)) dfs(n.id, 0); }
  const pos = new Map<string, { x: number; y: number }>();
  // 沿深度方向推进的间距（X 或 Y），以及同深度节点堆叠的间距
  const depthStep = dir === "tb" || dir === "bt" ? 200 : 260;
  // 同深度节点堆叠间距：节点卡片约 90~110px 高，80px 会让兄弟节点上下叠在一起，
  // 看起来像「后添加的节点把前一个的内容盖掉」。调大到 120px 保证每张卡片完整可见。
  const stackStep = dir === "tb" || dir === "bt" ? 240 : 120;
  const depthIndex = new Map<number, number>();
  for (const id of order) {
    const d = depth.get(id) ?? 0;
    const i = depthIndex.get(d) ?? 0;
    depthIndex.set(d, i + 1);
    const along = d * depthStep;   // 沿展开方向的偏移
    const across = i * stackStep;  // 同深度堆叠的偏移
    switch (dir) {
      case "rl": pos.set(id, { x: -along, y: across }); break;
      case "tb": pos.set(id, { x: across, y: along }); break;
      case "bt": pos.set(id, { x: across, y: -along }); break;
      default:  pos.set(id, { x: along, y: across });
    }
  }
  return pos;
}

// ════════════ 祖先链（选中节点的高亮路径） ════════════

function ancestorChain(nodeId: string, nodes: MindmapNode[]): string[] {
  const byId = new Map(nodes.map(n => [n.id, n]));
  const chain: string[] = [nodeId];
  const seen = new Set<string>([nodeId]);
  let cur = byId.get(nodeId);
  while (cur?.parentId && byId.has(cur.parentId) && cur.parentId !== cur.id && !seen.has(cur.parentId)) {
    seen.add(cur.parentId);
    chain.push(cur.parentId);
    cur = byId.get(cur.parentId);
  }
  return chain;
}

// ════════════ 详细弹窗（拖拽分隔条 + 全屏） ════════════

function DetailModal({ node, onUpdate, onClose, projectRoot }: { node: MindmapNode; accent?: string; onUpdate: (patch: Partial<MindmapNode>) => void; onClose: () => void; projectRoot?: string }) {
  const { t } = useTranslation();
  // 证据文件点击：在资源管理器中定位（项目根路径来自文档 sourceDesc）
  const openSource = (src: string) => {
    if (!projectRoot) return;
    const p = `${projectRoot.replace(/\\/g, "/")}/${src}`;
    void invoke("launcher_open_file_location", { path: p }).catch(() => {});
  };
  const sources = node.sources ?? [];
  // 双击节点进入详情后直接可编辑；预览/分栏交给下方 MarkdownFieldEditor 自带工具栏。
  const [detail, setDetail] = useState(node.detail);
  const [description, setDescription] = useState(node.description);
  const [name, setName] = useState(node.name);
  const [progress, setProgress] = useState(node.progress);
  const [planAt, setPlanAt] = useState(node.planAt ?? "");
  const [repeat, setRepeat] = useState(node.repeat || "none");
  const [color, setColor] = useState(node.color);
  const [fullscreen, setFullscreen] = useState(false);
  const c = normalizeHexColor(color) ?? kindColor(node.kind);
  const COLORS = ["#f8fafc","#22d3ee","#34d399","#fbbf24","#60a5fa","#fb7185","#a78bfa","#f97316","#f59e0b","#94a3b8"];

  const save = useCallback(() => {
    onUpdate({ name, description, color, progress, detail, planAt: planAt.trim() ? planAt.trim() : null, repeat });
  }, [name, description, color, progress, detail, planAt, repeat, onUpdate]);

  // Auto-save on unmount（用 ref 保存最新 save，避免 save 身份变化时
  // cleanup 反复触发 save → 父级 setState → 新 save → 无限循环卡死）
  const saveRef = useRef(save);
  useEffect(() => { saveRef.current = save; });
  useEffect(() => () => { saveRef.current(); }, []);

  // 文本字段输入防抖自动保存（800ms 无输入后落盘），不再只依赖 blur/关闭。
  // 依赖仅文本值，saveRef 取最新 save，不会因 save 身份变化陷入循环。
  useEffect(() => {
    const t = window.setTimeout(() => { saveRef.current(); }, 800);
    return () => window.clearTimeout(t);
  }, [name, description, detail]);

  return createPortal(
    <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]">
      <div className={`flex ${fullscreen ? "h-full w-full" : "h-[80vh] w-[min(92vw,680px)]"} flex-col overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl`} onClick={(e) => e.stopPropagation()}>
        <div className="flex h-11 shrink-0 items-center gap-2 border-b border-white/10 px-3" style={{ backgroundColor: `${c}1f` }}>
          <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: c, boxShadow: `0 0 9px ${c}` }} />
          <input className="min-w-0 flex-1 bg-transparent text-[12px] font-semibold text-slate-100 outline-none" value={name} onChange={(e) => setName(e.target.value)} onBlur={save} />
          <div className="flex items-center gap-1 ml-1">
            <button type="button" className={`nodrag nopan rounded p-1 text-[10px] ${fullscreen ? "bg-white/10 text-white" : "text-slate-400"} hover:bg-white/10 hover:text-white`} onClick={() => setFullscreen(!fullscreen)} title={fullscreen ? t("mindmap.fullscreenExit") : t("mindmap.fullscreen")}>{fullscreen ? <Minimize2 className="h-3.5 w-3.5" /> : <Maximize2 className="h-3.5 w-3.5" />}</button>
            <button type="button" className="nodrag nopan rounded p-1 text-slate-400 hover:text-white" onClick={onClose}><X className="h-4 w-4" /></button>
          </div>
        </div>
        {sources.length > 0 && (
          <div className="border-b border-white/5 bg-white/[0.02] px-4 py-2">
            <div className="mb-1 flex items-center gap-1 text-[9px] text-slate-500"><File className="h-2.5 w-2.5" />{t("mindmap.evidence", { count: sources.length })}{t("mindmap.evidenceHint")}</div>
            <div className="flex flex-wrap gap-1">
              {sources.map(s => (
                <button key={s} type="button" onClick={() => openSource(s)}
                  className={`nodrag nopan inline-flex max-w-[220px] cursor-pointer items-center gap-1 truncate rounded border border-cyan-400/25 bg-cyan-400/[0.07] px-1.5 py-0.5 font-mono text-[8px] text-cyan-200 transition hover:border-cyan-400/60 hover:bg-cyan-400/15 ${projectRoot ? "" : "cursor-default opacity-70"}`}
                  title={s}>
                  <File className="h-2.5 w-2.5 shrink-0" />
                  <span className="truncate">{s}</span>
                </button>
              ))}
            </div>
          </div>
        )}
        {/* 表单字段区（紧凑、可滚动）：描述 / 进度·计划时间·重复（同一行） / 颜色 */}
        <div className="shrink-0 space-y-3 overflow-y-auto p-4" style={{ maxHeight: "42%" }}>
          <div>
            <label className="text-[9px] text-slate-500 block mb-1">{t("mindmap.descLabel")}</label>
            <textarea className="w-full rounded bg-slate-900 border border-white/10 px-3 py-2 text-[11px] text-white outline-none resize-none" value={description} onChange={(e) => setDescription(e.target.value)} rows={2} onBlur={save} />
          </div>
          <div className="flex flex-wrap items-end gap-x-4 gap-y-2">
            <div>
              <label className="text-[9px] text-slate-500 block mb-1">{t("mindmap.progressLabel")}</label>
              <div className="flex items-center gap-1">
                <input type="range" min={0} max={100} value={progress} onChange={(e) => { const v = Number(e.target.value); setProgress(v); onUpdate({ progress: v }); }} className="w-24 h-6" />
                <span className="text-[10px] text-slate-400">{progress}%</span>
              </div>
            </div>
            <div>
              <label className="text-[9px] text-slate-500 block mb-1">{t("mindmap.planTimeLabel")}</label>
              <PlanDateTimePicker value={planAt} onChange={(iso) => { setPlanAt(iso ?? ""); onUpdate({ planAt: iso }); }} />
            </div>
            <div>
              <label className="text-[9px] text-slate-500 block mb-1">{t("mindmap.planRepeat")}</label>
              <select value={repeat} onChange={(e) => { const v = e.target.value; setRepeat(v); onUpdate({ repeat: v }); }}
                className="h-8 cursor-pointer rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60">
                <option value="none">{t("mindmap.noRepeat")}</option>
                <option value="daily">{t("mindmap.daily")}</option>
                <option value="weekly">{t("mindmap.weekly")}</option>
              </select>
            </div>
          </div>
          <div>
            <label className="text-[9px] text-slate-500 block mb-1">{t("mindmap.colorLabel")}</label>
            <div className="flex gap-1.5 flex-wrap items-center">
              {COLORS.map(cl => <button key={cl} type="button" className="h-5 w-5 rounded-full border border-white/20" style={{ backgroundColor: cl, boxShadow: color === cl ? `0 0 6px ${cl}` : "none" }}
                onClick={() => { setColor(cl); onUpdate({ color: cl }); }} />)}
              {/* 自定义任意颜色：原生取色器 + hex 输入 + 恢复默认 */}
              <label className="relative inline-flex h-5 w-5 cursor-pointer items-center justify-center overflow-hidden rounded-full border border-white/30" title={t("mindmap.customColor")}>
                <input type="color" value={/^#[0-9a-fA-F]{6}$/.test(color) ? color : "#22d3ee"} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
                  onChange={(e) => { const v = e.target.value; setColor(v); onUpdate({ color: v }); }} />
                <span className="h-3 w-3 rounded-full" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
              </label>
              <input value={color || ""} onChange={(e) => { const v = e.target.value; setColor(v); onUpdate({ color: v || "" }); }} placeholder="#RRGGBB"
                className="h-5 w-[74px] rounded border border-white/15 bg-slate-900 px-1.5 text-[9px] text-slate-300 outline-none focus:border-cyan-400/60" />
              <button type="button" className="rounded border border-white/15 px-1.5 py-0.5 text-[9px] text-slate-400 hover:text-white" onClick={() => { setColor(""); onUpdate({ color: "" }); }} title={t("mindmap.autoColor")}>{t("mindmap.auto")}</button>
            </div>
          </div>
        </div>
        {/* 详细内容：完整 Markdown 编辑器，占满剩余高度（不再留大片空白） */}
        <div className="flex min-h-0 flex-1 flex-col px-4 pb-4">
          <label className="mb-1 block text-[9px] text-slate-500">{t("mindmap.detailLabel")}</label>
          <div className="flex min-h-0 flex-1 flex-col">
            <MarkdownFieldEditor value={detail} onChange={(v) => setDetail(v)} />
          </div>
        </div>
      </div>
    </div>, document.body);
}

// ════════════ 创建文档弹窗 ════════════

function CreateDocModal({ onClose, onCreate, folderId }: { onClose: () => void; onCreate: (name: string, desc: string, folderId: string | null) => void; folderId: string | null }) {
  const { t } = useTranslation();
  const [name, setName] = useState(""); const [desc, setDesc] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => { inputRef.current?.focus(); }, []);

  return createPortal(
    <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
      <div className="w-[380px] rounded-xl border border-white/10 bg-[#0d1524] p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-semibold text-white">{t("mindmap.newMapTitle")}</h3>
          <button type="button" className="text-slate-500 hover:text-white" onClick={onClose}><X className="h-4 w-4" /></button>
        </div>
        <div className="space-y-3">
          <div><label className="text-[10px] text-slate-400 block mb-1">{t("mindmap.nameLabel")}</label><input ref={inputRef} className="w-full h-9 rounded-lg bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none" value={name} onChange={(e) => setName(e.target.value)} placeholder={t("mindmap.mapNamePh")} onKeyDown={(e) => { if (e.key === "Enter" && name.trim()) onCreate(name.trim(), desc, folderId); }} /></div>
          <div><label className="text-[10px] text-slate-400 block mb-1">{t("mindmap.descLabel")}</label><textarea className="w-full h-16 rounded-lg bg-slate-900 border border-white/10 px-3 py-2 text-xs text-white outline-none resize-none" value={desc} onChange={(e) => setDesc(e.target.value)} placeholder={t("mindmap.descPh")} /></div>
          <button type="button" className="w-full rounded-lg py-2 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }}
            disabled={!name.trim()} onClick={() => { if (name.trim()) onCreate(name.trim(), desc, folderId); }}>{t("mindmap.create")}</button>
        </div>
      </div>
    </div>, document.body);
}

// ════════════ 计划日历 ════════════

const WEEKDAY_KEYS = ["w1", "w2", "w3", "w4", "w5", "w6", "w7"];

/** Date → YYYY-MM-DD（本地时区） */
function toYMD(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${dd}`;
}

function monthDays(y: number, m: number): number { return new Date(y, m + 1, 0).getDate(); }
// 该月 1 号是周几（周一 = 0）
function monthOffset(y: number, m: number): number { return (new Date(y, m, 1).getDay() + 6) % 7; }

/** 纯月份网格：供「计划时间选择器」与「计划日历」复用；onDropDay 存在时单元格可作为拖拽目标 */
function MiniCalendar({ year, month, selected, marked, onSelect, onDropDay }: {
  year: number; month: number;
  selected?: string | null;
  marked?: Set<string>;
  onSelect: (ymd: string) => void;
  onDropDay?: (ymd: string) => void;
}) {
  const { t } = useTranslation();
  const today = toYMD(new Date());
  const [dropYmd, setDropYmd] = useState<string | null>(null);
  const dim = monthDays(year, month);
  const off = monthOffset(year, month);
  const cells: (string | null)[] = [];
  for (let i = 0; i < off; i++) cells.push(null);
  for (let d = 1; d <= dim; d++) cells.push(toYMD(new Date(year, month, d)));
  while (cells.length % 7 !== 0) cells.push(null);
  return (
    <div className="w-full">
      <div className="grid grid-cols-7 gap-0.5 mb-1">
        {WEEKDAY_KEYS.map((wk, i) => (
          <div key={i} className={`py-1 text-center text-[9px] font-semibold ${i >= 5 ? "text-slate-500" : "text-slate-400"}`}>{t("mindmap." + wk)}</div>
        ))}
      </div>
      <div className="grid grid-cols-7 gap-0.5">
        {cells.map((ymd, i) => {
          if (!ymd) return <div key={i} className="h-8" />;
          const dNum = Number(ymd.slice(8, 10));
          const isSel = ymd === selected;
          const isToday = ymd === today;
          const hasMark = marked?.has(ymd) ?? false;
          const isDrop = dropYmd === ymd;
          return (
            <button key={i} type="button"
              onClick={() => onSelect(ymd)}
              onDragOver={(e) => { if (!onDropDay) return; e.preventDefault(); e.dataTransfer.dropEffect = "move"; setDropYmd(ymd); }}
              onDragLeave={() => setDropYmd((v) => (v === ymd ? null : v))}
              onDrop={(e) => { if (!onDropDay) return; e.preventDefault(); setDropYmd(null); onDropDay(ymd); }}
              className={`relative flex h-8 items-center justify-center rounded-md text-[10px] transition cursor-pointer ${isDrop ? "bg-cyan-400/25 ring-2 ring-inset ring-cyan-300 text-cyan-100" : isSel ? "bg-cyan-400 text-slate-950 font-bold" : isToday ? "text-cyan-300 ring-1 ring-inset ring-cyan-400/50 hover:bg-white/[0.06]" : "text-slate-300 hover:bg-white/[0.06]"}`}>
              {dNum}
              {hasMark && <span className={`absolute bottom-0.5 left-1/2 h-1 w-1 -translate-x-1/2 rounded-full ${isSel ? "bg-slate-900" : isDrop ? "bg-cyan-200" : "bg-cyan-400"}`} />}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** 计划时间选择器：日历选日期 + 时间输入，弹层用 portal 避免被弹窗裁剪 */
export function PlanDateTimePicker({ value, onChange }: { value: string; onChange: (iso: string | null) => void }) {
  const { t } = useTranslation();
  const btnRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const init = value ? new Date(value) : new Date();
  const [ym, setYm] = useState({ y: init.getFullYear(), m: init.getMonth() });
  const [dateStr, setDateStr] = useState(() => (value ? toYMD(new Date(value)) : toYMD(new Date())));
  const [timeStr, setTimeStr] = useState(() => {
    if (!value) return "09:00";
    const d = new Date(value);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  });

  const toggleOpen = () => {
    if (open) { setOpen(false); return; }
    const r = btnRef.current?.getBoundingClientRect();
    if (r) {
      const popW = 280;
      const left = Math.min(r.left, Math.max(8, window.innerWidth - popW - 8));
      setPos({ left, top: r.bottom + 6 });
    }
    setOpen(true);
  };

  const confirm = () => {
    const [y, m, d] = dateStr.split("-").map(Number);
    const [hh, mi] = timeStr.split(":").map(Number);
    const dt = new Date(y, m - 1, d, hh, mi);
    if (Number.isNaN(dt.getTime())) return;
    onChange(dt.toISOString());
    setOpen(false);
  };

  return (
    <>
      <div ref={btnRef} className="flex items-center gap-2">
        <button type="button" onClick={toggleOpen}
          className={`flex h-8 items-center gap-1.5 rounded-md border px-2.5 text-xs transition cursor-pointer ${value ? "border-cyan-400/40 bg-slate-950/70 text-slate-200 hover:border-cyan-400/70" : "border-dashed border-white/20 bg-transparent text-slate-500 hover:text-slate-300"}`}>
          <Calendar className="h-3.5 w-3.5" />
          {value ? planShort(value) : t("mindmap.pickPlanTime")}
        </button>
        {value && <button type="button" className="rounded border border-white/15 px-1.5 py-0.5 text-[9px] text-slate-400 hover:text-white" onClick={() => { onChange(null); setOpen(false); }}>{t("mindmap.clear")}</button>}
      </div>
      {open && createPortal(
        <>
          <div className="fixed inset-0 z-[220]" onClick={() => setOpen(false)} />
          <div className="fixed z-[221] w-[280px] rounded-lg border border-white/10 bg-[#0d1524] p-3 shadow-2xl" style={pos ?? { left: 8, top: 8 }}>
            <div className="mb-2 flex items-center justify-between">
              <button type="button" className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => setYm(({ y, m }) => (m === 0 ? { y: y - 1, m: 11 } : { y, m: m - 1 }))} title={t("mindmap.prevMonth")}><ChevronLeft className="h-3.5 w-3.5" /></button>
              <span className="text-[11px] font-semibold text-slate-200">{t("mindmap.yearMonth", { year: ym.y, month: ym.m + 1 })}</span>
              <button type="button" className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => setYm(({ y, m }) => (m === 11 ? { y: y + 1, m: 0 } : { y, m: m + 1 }))} title={t("mindmap.nextMonth")}><ChevronRight className="h-3.5 w-3.5" /></button>
            </div>
            <MiniCalendar year={ym.y} month={ym.m} selected={dateStr} onSelect={(ymd) => setDateStr(ymd)} />
            <div className="mt-2 flex items-center gap-1.5">
              <span className="text-[9px] text-slate-500">{t("mindmap.time")}</span>
              <input type="time" value={timeStr} onChange={(e) => setTimeStr(e.target.value)}
                className="h-7 flex-1 rounded-md border border-white/10 bg-slate-900 px-1.5 text-[10px] text-slate-200 outline-none focus:border-cyan-400/60" />
            </div>
            <div className="mt-2 flex items-center justify-between">
              <button type="button" className="rounded border border-white/15 px-2 py-1 text-[9px] text-slate-300 hover:text-white" onClick={() => { const now = new Date(); setDateStr(toYMD(now)); setYm({ y: now.getFullYear(), m: now.getMonth() }); }}>{t("mindmap.today")}</button>
              <div className="flex gap-1.5">
                <button type="button" className="rounded border border-white/15 px-2.5 py-1 text-[9px] text-slate-400 hover:text-white" onClick={() => { onChange(null); setOpen(false); }}>{t("mindmap.clear")}</button>
                <button type="button" className="rounded bg-cyan-500 px-2.5 py-1 text-[9px] font-semibold text-slate-950 hover:bg-cyan-400" onClick={confirm}>{t("mindmap.confirm")}</button>
              </div>
            </div>
          </div>
        </>, document.body)}
    </>
  );
}

/** 按后端返回的 occurDay 把范围查询结果分组（重复计划已在后端展开为具体发生记录） */
function buildOccurMap(occ: PlannedOccurrence[]): Map<string, PlannedOccurrence[]> {
  const m = new Map<string, PlannedOccurrence[]>();
  for (const p of occ) {
    const arr = m.get(p.occurDay) ?? [];
    arr.push(p);
    m.set(p.occurDay, arr);
  }
  for (const arr of m.values()) arr.sort((a, b) => a.occurAt.localeCompare(b.occurAt));
  return m;
}

/** 计划日历：月/周视图（有计划的日期打点，今日高亮，过期计划置灰）+ 选中日期计划列表，跨全部文档聚合 */
function PlanCalendarModal({ onPick, onClose, onAddPlan, onMoveOccurrence }: {
  onPick: (p: PlannedOccurrence) => void;
  onClose: () => void;
  onAddPlan: (ymd: string) => void;
  onMoveOccurrence: (fromDay: string, toDay: string, nodeId: string) => Promise<boolean>;
}) {
  const { t } = useTranslation();
  const now = new Date();
  const [view, setView] = useState<"month" | "week">("month");
  const [ym, setYm] = useState({ y: now.getFullYear(), m: now.getMonth() });
  const [selDay, setSelDay] = useState<string>(toYMD(now));
  const [occ, setOcc] = useState<PlannedOccurrence[]>([]);
  const [loading, setLoading] = useState(false);
  // 拖拽改期：dragItem=正在拖的条目，pendingMove=待确认的移动，refreshKey=移动成功后重新拉取
  const [dragItem, setDragItem] = useState<{ nodeId: string; fromDay: string; name: string } | null>(null);
  const [pendingMove, setPendingMove] = useState<{ nodeId: string; name: string; fromDay: string; toDay: string } | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [moving, setMoving] = useState(false);

  const pickDay = (ymd: string) => {
    setSelDay(ymd);
    const [y, m] = ymd.split("-").map(Number);
    setYm({ y, m: m - 1 });
  };

  const shift = (delta: number) => {
    if (view === "month") {
      setYm(({ y, m }) => { const d = new Date(y, m + delta, 1); return { y: d.getFullYear(), m: d.getMonth() }; });
    } else {
      const d = new Date(`${selDay}T00:00:00`);
      d.setDate(d.getDate() + delta * 7);
      const ymd = toYMD(d);
      setSelDay(ymd);
      setYm({ y: d.getFullYear(), m: d.getMonth() });
    }
  };

  const goToday = () => { const t = new Date(); setSelDay(toYMD(t)); setYm({ y: t.getFullYear(), m: t.getMonth() }); };

  // 周视图：selDay 所在周的 7 天（周一为一周起点）
  const weekStart = useMemo(() => {
    const d = new Date(`${selDay}T00:00:00`);
    d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
    return d;
  }, [selDay]);
  const weekDays = useMemo(() => Array.from({ length: 7 }, (_, i) => { const d = new Date(weekStart); d.setDate(weekStart.getDate() + i); return d; }), [weekStart]);

  // 当前可见范围（月=当月 1 日~月末；周=周一~周日），切换视图/翻页时重新拉取，
  // 重复计划由后端在 SQL 中展开，前端只按 occurDay 分组。
  const range = useMemo(() => {
    if (view === "month") {
      return { start: toYMD(new Date(ym.y, ym.m, 1)), end: toYMD(new Date(ym.y, ym.m + 1, 0)) };
    }
    return { start: toYMD(weekStart), end: toYMD(weekDays[6]) };
  }, [view, ym, weekStart, weekDays]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    mmApi.plannedOccurrences(range.start, range.end)
      .then(list => { if (alive) setOcc(list); })
      .catch(() => { if (alive) setOcc([]); })
      .finally(() => { if (alive) setLoading(false); });
    return () => { alive = false; };
  }, [range.start, range.end, refreshKey]);

  // 拖拽改期：把条目拖到某天 → 弹出确认栏（不改 plan_at 本身的时间，只平移日期）
  const dayDiff = (a: string, b: string) => Math.round((new Date(`${b}T00:00:00`).getTime() - new Date(`${a}T00:00:00`).getTime()) / 86400000);
  const handleDayDrop = (ymd: string) => {
    if (!dragItem) return;
    setDragItem(null);
    if (ymd === dragItem.fromDay) return;
    setPendingMove({ nodeId: dragItem.nodeId, name: dragItem.name, fromDay: dragItem.fromDay, toDay: ymd });
  };
  const confirmMove = async () => {
    if (!pendingMove || moving) return;
    setMoving(true);
    try {
      const ok = await onMoveOccurrence(pendingMove.fromDay, pendingMove.toDay, pendingMove.nodeId);
      if (ok) { setPendingMove(null); setRefreshKey(k => k + 1); }
    } finally { setMoving(false); }
  };

  const byDay = useMemo(() => buildOccurMap(occ), [occ]);
  const marked = useMemo(() => new Set(byDay.keys()), [byDay]);
  const dayPlans = byDay.get(selDay) ?? [];

  const label = view === "month"
    ? t("mindmap.monthTitle", { y: ym.y, m: ym.m + 1 })
    : t("mindmap.rangeTitle", { m1: weekStart.getMonth() + 1, d1: weekStart.getDate(), m2: weekDays[6].getMonth() + 1, d2: weekDays[6].getDate() });

  const selLabel = (() => {
    const d = new Date(`${selDay}T00:00:00`);
    if (Number.isNaN(d.getTime())) return selDay;
    return t("mindmap.dayTitle", { m: d.getMonth() + 1, d: d.getDate() });
  })();

  const todayYmd = toYMD(now);
  const nowMs = now.getTime();

  const renderPlan = (p: PlannedOccurrence) => {
    const occMs = new Date(p.occurAt).getTime();
    const past = !Number.isNaN(occMs) && occMs < nowMs;
    return (
      <button key={`${p.documentId}-${p.id}`} type="button"
        draggable
        onDragStart={(e) => { setDragItem({ nodeId: p.id, fromDay: p.occurDay, name: p.name }); e.dataTransfer.setData("text/plain", p.id); e.dataTransfer.effectAllowed = "move"; }}
        onDragEnd={() => setDragItem(null)}
        onClick={() => onPick(p)}
        className={`flex w-full cursor-grab items-center gap-2 rounded-md border border-white/10 bg-white/[0.03] px-2.5 py-2 text-left transition hover:bg-white/[0.08] active:cursor-grabbing ${past ? "opacity-45" : ""} ${dragItem?.nodeId === p.id ? "opacity-40" : ""}`}
        title={`${t("mindmap.openNodeInDoc", { name: p.documentName })}${past ? t("mindmap.pastMark") : ""}`}>
        <span className={`shrink-0 font-mono text-[9px] ${past ? "text-slate-500 line-through" : "text-slate-400"}`}>{new Date(p.occurAt).toTimeString().slice(0, 5)}</span>
        <span className={`min-w-0 flex-1 truncate text-[10px] ${past ? "text-slate-500 line-through" : "text-slate-200"}`}>{p.name}</span>
        {p.repeat && p.repeat !== "none" && <span className="shrink-0 text-[8px] text-cyan-300/80">{p.repeat === "daily" ? t("mindmap.daily") : t("mindmap.weekly")}</span>}
        <span className="shrink-0 text-[9px] text-slate-500">{p.documentName}</span>
      </button>
    );
  };

  return createPortal(
    <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]">
      <div className="w-[min(94vw,760px)] rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between border-b border-white/10 px-4 py-3">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-white"><Calendar className="h-4 w-4 text-cyan-400" />{t("mindmap.planCalendar")}</h3>
          <div className="flex items-center gap-1.5">
            <div className="flex rounded-md border border-white/10 bg-slate-950/60 p-0.5">
              <button type="button" onClick={() => setView("month")} className={`rounded px-2 py-1 text-[9px] font-medium transition cursor-pointer ${view === "month" ? "bg-cyan-500/20 text-cyan-300" : "text-slate-400 hover:text-white"}`}>{t("mindmap.monthView")}</button>
              <button type="button" onClick={() => setView("week")} className={`rounded px-2 py-1 text-[9px] font-medium transition cursor-pointer ${view === "week" ? "bg-cyan-500/20 text-cyan-300" : "text-slate-400 hover:text-white"}`}>{t("mindmap.weekView")}</button>
            </div>
            <button type="button" className="rounded p-1 text-slate-400 hover:text-white" onClick={onClose} title={t("mindmap.close")}><X className="h-4 w-4" /></button>
          </div>
        </div>
        <div className="flex min-h-[380px] flex-col gap-4 p-4 lg:flex-row">
          {/* 网格区：月历 / 周历 */}
          <div className="shrink-0 lg:w-[340px]">
            <div className="mb-2 flex items-center justify-between">
              <button type="button" className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => shift(-1)} title={t("mindmap.prevPage")}><ChevronLeft className="h-4 w-4" /></button>
              <span className="text-[11px] font-semibold text-slate-200">{label}</span>
              <button type="button" className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => shift(1)} title={t("mindmap.nextPage")}><ChevronRight className="h-4 w-4" /></button>
            </div>
            {view === "month" ? (
              <MiniCalendar year={ym.y} month={ym.m} selected={selDay} marked={marked} onSelect={pickDay} onDropDay={handleDayDrop} />
            ) : (
              <div className="flex gap-1">
                {weekDays.map((d, i) => {
                  const ymd = toYMD(d);
                  const isToday = ymd === todayYmd;
                  const isSel = ymd === selDay;
                  const dayPlansW = byDay.get(ymd) ?? [];
                  return (
                    <div key={i}
                      onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = "move"; }}
                      onDrop={(e) => { e.preventDefault(); handleDayDrop(ymd); }}
                      className={`flex-1 rounded-md border px-1 pb-1 ${isSel ? "border-cyan-400/60 bg-cyan-400/[0.06]" : isToday ? "border-cyan-400/30 bg-white/[0.02]" : "border-white/5"}`}>
                      <button type="button" onClick={() => pickDay(ymd)} className={`w-full py-1 text-center text-[9px] transition cursor-pointer ${isSel ? "font-bold text-cyan-300" : isToday ? "text-cyan-300" : "text-slate-400 hover:text-white"}`}>
                        <div className="mb-0.5 text-[8px] text-slate-500">{t("mindmap." + WEEKDAY_KEYS[i])}</div>
                        <div>{d.getDate()}</div>
                      </button>
                      <div className="space-y-0.5">
                        {dayPlansW.slice(0, 3).map(p => <div key={`${p.documentId}-${p.id}`} className="mx-auto h-1 w-1 rounded-full" style={{ backgroundColor: normalizeHexColor(p.color) ?? kindColor(p.kind) }} title={p.name} />)}
                        {dayPlansW.length > 3 && <div className="text-center text-[7px] text-slate-600">+{dayPlansW.length - 3}</div>}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
            <div className="mt-2 flex items-center justify-between">
              <span className="flex items-center gap-1 text-[9px] text-slate-500"><span className="h-1.5 w-1.5 rounded-full bg-cyan-400" />{t("mindmap.hasPlanDays")}</span>
              <button type="button" className="rounded border border-white/15 px-2 py-1 text-[9px] text-slate-300 hover:text-white" onClick={goToday}>{t("mindmap.today")}</button>
            </div>
          </div>
          {/* 当日计划列表 */}
          <div className="flex min-h-0 min-w-0 flex-1 flex-col rounded-lg border border-white/10 bg-slate-950/40">
            <div className="flex items-center justify-between gap-2 border-b border-white/10 px-3 py-2">
              <span className="text-[11px] font-semibold text-slate-200">{t("mindmap.dayPlans", { label: selLabel, count: dayPlans.length })}<span className="ml-1 text-[8px] font-normal text-slate-500">{t("mindmap.dragToReschedule")}</span></span>
              <button type="button" onClick={() => onAddPlan(selDay)}
                className="flex shrink-0 cursor-pointer items-center gap-1 rounded border border-cyan-400/40 px-1.5 py-0.5 text-[9px] text-cyan-300 transition hover:bg-cyan-400/10"
                title={t("mindmap.addPlanTitle")}>{t("mindmap.addPlan")}</button>
            </div>
            <div className="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
              {loading ? (
                <div className="flex h-full items-center justify-center text-[10px] text-slate-500"><Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />{t("mindmap.loading")}</div>
              ) : dayPlans.length === 0 ? (
                <VexEmptyState title={t("mindmap.dayEmptyTitle")} desc={t("mindmap.dayEmptyDesc")} tick={t("mindmap.dayEmptyTick")} avatarSize={34} className="!py-6" />
              ) : dayPlans.map(renderPlan)}
            </div>
            {occ.length === 0 && !loading && (
              <div className="border-t border-white/10 px-3 py-2 text-center text-[9px] text-slate-600">{t("mindmap.noPlanNodes")}</div>
            )}
          </div>
        </div>
        {pendingMove && (
          <div className="flex items-center justify-between gap-3 border-t border-white/10 bg-cyan-400/[0.06] px-4 py-2.5">
            <div className="min-w-0 text-[10px] text-slate-200">
              <span className="text-cyan-300">{pendingMove.name}</span>」：{pendingMove.fromDay} → {pendingMove.toDay}
              <span className="ml-1.5 text-slate-500">{dayDiff(pendingMove.fromDay, pendingMove.toDay) > 0 ? t("mindmap.moveLater", { count: dayDiff(pendingMove.fromDay, pendingMove.toDay) }) : t("mindmap.moveEarlier", { count: Math.abs(dayDiff(pendingMove.fromDay, pendingMove.toDay)) })}</span>
            </div>
            <div className="flex shrink-0 gap-1.5">
              <button type="button" className="cursor-pointer rounded border border-white/15 px-2 py-1 text-[9px] text-slate-400 hover:text-white" onClick={() => setPendingMove(null)}>{t("mindmap.cancel")}</button>
              <button type="button" disabled={moving} className="cursor-pointer rounded bg-cyan-500 px-2.5 py-1 text-[9px] font-semibold text-slate-950 hover:bg-cyan-400 disabled:opacity-50" onClick={() => void confirmMove()}>
                {moving ? t("mindmap.moving") : t("mindmap.confirmMove")}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>, document.body);
}

// ════════════ AI 导入校验报告 ════════════

const VIEW_LABEL_KEYS: Record<string, string> = {
  architecture: "mindmap.viewArchitecture",
  workflow: "mindmap.viewWorkflow",
  dataflow: "mindmap.viewDataflow",
  sequence: "mindmap.viewSequence",
  lifecycle: "mindmap.viewLifecycle",
};
const viewLabel = (t: (k: string, o?: any) => string, v: string) => t(VIEW_LABEL_KEYS[v] ?? v);

/** 导入完成弹窗：逐视图展示节点数、修复轮数与残留校验诊断；点击条目跳转到对应文档 */
function AiImportReportModal({ result, onClose, onOpenDoc }: {
  result: AiImportResult;
  onClose: () => void;
  onOpenDoc: (id: string) => void;
}) {
  const { t } = useTranslation();
  const allOk = result.reports.length > 0 && result.reports.every(r => r.diagnostics.length === 0) && result.failures.length === 0;
  return createPortal(
    <div className="fixed inset-0 z-[210] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]" onClick={onClose}>
      <div className="w-[min(94vw,560px)] rounded-xl border border-white/10 bg-[#0d1524] p-4 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-white"><Sparkles className="h-4 w-4 text-cyan-400" />{t("mindmap.aiReportTitle")}</h3>
          <button type="button" className="cursor-pointer rounded p-1 text-slate-400 hover:text-white" onClick={onClose} title={t("mindmap.close")}><X className="h-4 w-4" /></button>
        </div>
        <div className="max-h-[60vh] space-y-2 overflow-y-auto pr-1">
          {result.reports.length === 0 && (
            <p className="py-4 text-center text-[10px] text-slate-500">{t("mindmap.noViewsGenerated")}</p>
          )}
          {result.reports.map(r => {
            const doc = result.documents.find(d => d.document.id === r.documentId);
            const ok = r.diagnostics.length === 0;
            return (
              <button key={r.documentId} type="button"
                onClick={() => onOpenDoc(r.documentId)}
                className="block w-full cursor-pointer rounded-lg border border-white/10 bg-white/[0.03] px-3 py-2 text-left transition hover:bg-white/[0.08]"
                title={t("mindmap.openDoc")}>
                <div className="flex items-center justify-between gap-2">
                  <span className="flex min-w-0 items-center gap-1.5 text-[11px] font-semibold text-slate-200">
                    <span className="shrink-0 rounded border border-cyan-400/40 bg-cyan-400/10 px-1.5 py-0.5 text-[9px] text-cyan-300">{viewLabel(t, r.view)}</span>
                    <span className="truncate">{doc?.document.name ?? t("mindmap.docPlaceholder")}</span>
                  </span>
                  <span className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] ${ok ? "bg-emerald-500/15 text-emerald-300" : "bg-amber-500/15 text-amber-300"}`}>{ok ? t("mindmap.validationOk") : t("mindmap.validationError")}</span>
                </div>
                <div className="mt-1 text-[9px] text-slate-500">
                  {t("mindmap.nodeCount", { count: r.nodeCount })} · {r.repairRounds === 1 ? t("mindmap.repairFirstPass") : t("mindmap.repairRound", { count: r.repairRounds })}
                </div>
                <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[9px]">
                  <span className="text-slate-500">{t("mindmap.evidenceStats", { nodes: r.evidenceNodes, count: r.evidenceCount })}</span>
                  {r.nodeCount - r.evidenceNodes > 0 && (
                    <span className="text-slate-600">{t("mindmap.aiInferred", { count: r.nodeCount - r.evidenceNodes })}</span>
                  )}
                  {r.evidenceVerified && r.evidenceCount > 0 && (
                    <span className={r.evidenceHitCount === r.evidenceCount ? "text-emerald-300" : "text-amber-300"}>
                      {t("mindmap.evidenceHit", { hit: r.evidenceHitCount, total: r.evidenceCount, pct: Math.round((r.evidenceHitCount / r.evidenceCount) * 100) })}
                    </span>
                  )}
                </div>
                {r.diagnostics.length > 0 && (
                  <div className="mt-1.5 rounded border border-red-500/20 bg-red-500/10 px-2 py-1 text-[9px] leading-relaxed text-red-300">{r.diagnostics.join("；")}</div>
                )}
              </button>
            );
          })}
          {result.failures.length > 0 && (
            <div className="rounded-lg border border-amber-500/20 bg-amber-500/[0.06] px-3 py-2">
              <div className="text-[10px] font-semibold text-amber-300">{t("mindmap.failedViews")}</div>
              {result.failures.map((f, i) => (
                <div key={i} className="mt-1 text-[9px] leading-relaxed text-amber-200/80">「{viewLabel(t, f.view)}」：{f.reason}</div>
              ))}
            </div>
          )}
        </div>
        <div className="mt-3 flex items-center justify-between gap-2">
          <span className={`text-[9px] ${allOk ? "text-emerald-300" : "text-slate-500"}`}>{allOk ? t("mindmap.allViewsOk") : t("mindmap.someViewsError")}</span>
          <button type="button" className="cursor-pointer rounded bg-cyan-500 px-3 py-1.5 text-[10px] font-semibold text-slate-950 hover:bg-cyan-400" onClick={onClose}>{t("mindmap.closeBtn")}</button>
        </div>
      </div>
    </div>, document.body);
}

// ════════════ 画布 ════════════

// nodeTypes/edgeTypes 必须在组件外定义为常量：若在 JSX 内联新建，每次渲染都会
// 产生新对象，React Flow 会因此反复重渲染（官方文档明确警告的卡顿/卡死源）。
const mmNodeTypes = { mmNode: FlowNode, stickerNode: StickerFlowNode };
const mmEdgeTypes = { colorE: ColorEdge };

type PosOverride = { x: number; y: number };

// 节点对象缓存条目：用于拖拽时复用未变化节点对象，避免全部节点每帧重渲染导致闪烁
type NodeCacheEntry = {
  node: MindmapNode | MindmapSticker;
  full: DocumentFull;
  px: number;
  py: number;
  selected: boolean;
  hasChildren: boolean;
  collapsed: boolean;
  obj: Node;
};

function CanvasInner({ full, accent, onDocumentUpdate, onHistoryPush, historyVersion, onAiProject, onAiText, onError, onOpenCalendar, focusRequest, onFocusHandled }: { full: DocumentFull; accent: string; onDocumentUpdate: (d: DocumentFull) => void; onHistoryPush: () => void; historyVersion: number; onAiProject: () => void; onAiText: () => void; onError: (message: string) => void; onOpenCalendar: () => void; focusRequest: { nodeId: string; ts: number } | null; onFocusHandled: () => void }) {
  const { t } = useTranslation();
  const { fitView } = useReactFlow();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detailNode, setDetailNode] = useState<MindmapNode | null>(null);
  const [preview, setPreview] = useState<{ node: MindmapNode; x: number; y: number } | null>(null);
  // 预览气泡悬停状态：鼠标移入气泡后保持打开（此时可滚动查看长内容），移出才关闭。
  // 关闭加 250ms 延迟，否则鼠标从节点移向气泡的瞬间气泡就消失了。
  const previewHoverRef = useRef(false);
  const previewCloseTimer = useRef<number | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; nodeId: string } | null>(null);
  const [lastSaved, setLastSaved] = useState<number | null>(null);
  // 用户拖放产生的位置覆盖（本地状态）。节点位置 = posOverrides ?? (已保存坐标 ?? 自动布局)。
  // 关键：拖放不再依赖 nodes state + useEffect 同步（那会在 WebView2/React19 下形成
  // onNodesChange → setNodes → 重建节点 → 重新测量 → onNodesChange 的死循环），
  // 而是直接驱动本地 posOverrides，一次性 setState，物理上杜绝反馈循环。
  const [posOverrides, setPosOverrides] = useState<Record<string, PosOverride>>({});
  // RF 实测的节点尺寸（dimensions 变更写入）。adoptUserNodes 只认 userNode.measured，
  // 缺了它 RF 会在拖动中每帧重测 → 节点尺寸塌陷/恢复 → 闪烁。
  const [measuredMap, setMeasuredMap] = useState<Record<string, { width: number; height: number }>>({});
  // 切换文档时清空位置覆盖（新文档用其自身已保存坐标或自动布局）
  useEffect(() => { setPosOverrides({}); }, [full.document.id]);
  // 切换文档时恢复该文档保存的布局方向（避免沿用上一份导图的方向）
  useEffect(() => {
    const d = full.document.layoutDir;
    setDir(isLayoutDir(d) ? d : "lr");
  }, [full.document.id]);
  // 撤销/重做恢复快照后清空本地拖拽覆盖，让还原的坐标生效
  useEffect(() => { setPosOverrides({}); }, [historyVersion]);

  // 自动保存：拖放坐标先写入本地 pending，防抖批量 flush 到后端。
  // fullRef 始终指向最新 full，避免防抖回调读到陈旧闭包。
  const fullRef = useRef(full);
  useEffect(() => { fullRef.current = full; }, [full]);
  const pendingPos = useRef<PositionInput[]>([]);
  const pendingStickers = useRef<Map<string, { x: number; y: number }>>(new Map());
  const flushTimer = useRef<number | null>(null);

  const graphNodes = full.nodes;
  const stickers = full.stickers;
  const backgroundTexture = (full.document.backgroundTexture in BACKGROUND_TEXTURE_KEYS
    ? full.document.backgroundTexture
    : "dots") as BackgroundTexture;
  const byId = useMemo(() => new Map(graphNodes.map((n) => [n.id, n])), [graphNodes]);
  // focusRequest effect 需要读取最新节点但又不依赖 byId 变化（避免弹窗被重开），用 ref 镜像
  const byIdRef = useRef(byId);
  useEffect(() => { byIdRef.current = byId; }, [byId]);
  // 布局方向（画布级设置）：切换后自动重排并持久化；初始值取该文档保存的方向
  const [dir, setDir] = useState<LayoutDir>(() => (isLayoutDir(full.document.layoutDir) ? full.document.layoutDir : "lr"));
  const layout = useMemo(() => layoutTree(graphNodes, dir), [graphNodes, dir]);
  const endpointPositions = dir === "tb"
    ? { target: Position.Top, source: Position.Bottom }
    : dir === "bt"
      ? { target: Position.Bottom, source: Position.Top }
      : dir === "rl"
        ? { target: Position.Right, source: Position.Left }
        : { target: Position.Left, source: Position.Right };
  const childrenCount = useMemo(() => { const m = new Map<string, number>(); for (const n of graphNodes) { if (n.parentId) m.set(n.parentId, (m.get(n.parentId) ?? 0) + 1); } return m; }, [graphNodes]);

  // Ancestor chain for selected node
  const highlightChain = useMemo(() => selectedId ? ancestorChain(selectedId, graphNodes) : [], [selectedId, graphNodes]);

  const visibleNodes = useMemo(() => {
    const result: MindmapNode[] = [];
    const visited = new Set<string>();
    const visit = (n: MindmapNode) => {
      if (visited.has(n.id)) return;
      visited.add(n.id); result.push(n);
      if (!collapsed.has(n.id)) graphNodes.filter(c => c.parentId === n.id && c.parentId !== c.id && !visited.has(c.id)).forEach(visit);
    };
    graphNodes.filter(n => !n.parentId || !byId.has(n.parentId) || n.parentId === n.id).forEach(visit);
    // 环内/孤立节点兜底
    graphNodes.forEach(n => { if (!visited.has(n.id)) visit(n); });
    return result;
  }, [graphNodes, collapsed, byId]);

  const addNode = useCallback((parentId: string | null) => {
    onHistoryPush();
    const now = new Date().toISOString();
    // 新增子节点继承父节点的可见颜色（父节点未手动配色时取类型默认色），保持树视觉连续
    const parent = parentId ? byId.get(parentId) : null;
    const n: MindmapNode = { id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, documentId: full.document.id, parentId, name: parentId ? t("mindmap.newNode") : t("mindmap.newRoot"), description: "", detail: "", kind: parentId ? "other" : "root", color: parent ? effectiveNodeColor(parent) : "", progress: 0, planAt: null, repeat: "none", positionX: 0, positionY: 0, createdAt: now, updatedAt: now };
    void mmApi.upsertNode({ documentId: full.document.id, node: n });
    onDocumentUpdate({ ...full, nodes: [...full.nodes, n] });
    setSelectedId(n.id);
  }, [full, byId, onDocumentUpdate, onHistoryPush]);

  const addChildNode = useCallback((parentIdOverride?: string) => {
    addNode(parentIdOverride ?? selectedId ?? null);
  }, [addNode, selectedId]);

  const makeRoot = useCallback((nodeId: string) => {
    const node = byId.get(nodeId);
    if (!node || node.parentId === null) return;
    onHistoryPush();
    const updated = { ...node, parentId: null, kind: node.kind === "root" ? node.kind : "root", updatedAt: new Date().toISOString() };
    void mmApi.upsertNode({ documentId: full.document.id, node: updated });
    onDocumentUpdate({ ...full, nodes: full.nodes.map(n => n.id === nodeId ? updated : n) });
  }, [byId, full, onDocumentUpdate, onHistoryPush]);

  const onConnect = useCallback((connection: Connection) => {
    const source = connection.source;
    const target = connection.target;
    if (!source || !target || source === target || !byId.has(source) || !byId.has(target)) return;
    // 不能把节点连到自己的后代，否则会形成环，布局和折叠都会失效。
    let cursor: string | null = source;
    const seen = new Set<string>();
    while (cursor && !seen.has(cursor)) {
      if (cursor === target) return;
      seen.add(cursor);
      cursor = byId.get(cursor)?.parentId ?? null;
    }
    const child = byId.get(target)!;
    onHistoryPush();
    const updated = { ...child, parentId: source, kind: child.kind === "root" ? "other" : child.kind, updatedAt: new Date().toISOString() };
    void mmApi.upsertNode({ documentId: full.document.id, node: updated });
    onDocumentUpdate({ ...full, nodes: full.nodes.map(n => n.id === target ? updated : n) });
  }, [byId, full, onDocumentUpdate, onHistoryPush]);

  const deleteNode = useCallback((nodeId: string) => {
    onHistoryPush();
    void mmApi.deleteNode({ documentId: full.document.id, nodeId });
    const removed = new Set<string>([nodeId]);
    let changed = true;
    while (changed) {
      changed = false;
      for (const n of full.nodes) {
        if (n.parentId && removed.has(n.parentId) && !removed.has(n.id)) { removed.add(n.id); changed = true; }
      }
    }
    onDocumentUpdate({ ...full, nodes: full.nodes.filter(n => !removed.has(n.id)) });
    if (selectedId === nodeId) setSelectedId(null);
    if (detailNode?.id === nodeId) setDetailNode(null);
    setPreview(null);
  }, [selectedId, detailNode, full, onDocumentUpdate, onHistoryPush]);

  // 打开详情弹窗前先记录历史快照：整个编辑会话（改名称/描述/颜色/进度/detail）算一步撤销
  const openDetail = useCallback((n: MindmapNode) => {
    onHistoryPush();
    setDetailNode(n);
  }, [onHistoryPush]);

  // 节点对象缓存：拖拽时 `onNodesChange` 每个 mousemove 都会 setPosOverrides → 本 useMemo 重算。
  // 若每次重建全部节点对象，每个节点 data 都是新引用，memo(FlowNode) 无法跳过 → 全部节点每帧重渲染 → 闪烁。
  // 这里按 id 缓存：只有被拖节点（位置变化）重建对象，其余节点复用旧对象，memo 跳过 → 无闪烁。
  // 缓存键含 `full` 引用：full 变化（拖放结束/编辑保存）时全部失效重建，避免闭包捕获过期数据。
  const nodeObjCache = useRef(new Map<string, NodeCacheEntry>());

  const flowNodes = useMemo<Node<FlowNodeData | StickerNodeData>[]>(() => {
    const cache = nodeObjCache.current;
    const alive = new Set<string>();
    const main: Node<FlowNodeData>[] = [];
    for (const n of visibleNodes) {
      alive.add(n.id);
      const hasSaved = n.positionX !== 0 || n.positionY !== 0;
      const base = hasSaved ? { x: n.positionX, y: n.positionY } : layout.get(n.id) ?? { x: 0, y: 0 };
      const p = posOverrides[n.id] ?? base;
      const selected = selectedId === n.id || highlightChain.includes(n.id);
      const hasChildren = (childrenCount.get(n.id) ?? 0) > 0;
      const isCollapsed = collapsed.has(n.id);
      const prev = cache.get(n.id);
      if (prev && prev.full === full && prev.node === n && prev.px === p.x && prev.py === p.y && prev.selected === selected && prev.hasChildren === hasChildren && prev.collapsed === isCollapsed) {
        main.push(prev.obj as Node<FlowNodeData>);
        continue;
      }
      // data 仅在节点内容/状态变化时重建；纯位置变化（拖动中）复用旧 data 引用，
      // 避免 memo(FlowNode) 因 data 每帧新引用而重渲染整个节点子树（WebView2 下闪烁）。
      // 注意：data 里的回调（onAddChild/onOpenDetail/onDelete…）闭包捕获了 full/selectedId 等状态，
      // 因此 full 一旦变化就必须重建 data，否则复用旧 data 会让回调闭包停留在旧状态——
      // 例如连续点「+」加子节点时，第 3 次会用第 1 次渲染时的旧 full，把第 2 个子节点覆盖掉。
      const prevData = prev ? (prev.obj as Node<FlowNodeData>).data : null;
      const dataChanged = !prevData || prev!.full !== full || prev!.node !== n || prev!.selected !== selected || prev!.hasChildren !== hasChildren || prev!.collapsed !== isCollapsed || prevData.targetPosition !== endpointPositions.target || prevData.sourcePosition !== endpointPositions.source;
      const data: FlowNodeData = dataChanged          ? { node: n, selected, hasChildren, collapsed: isCollapsed, targetPosition: endpointPositions.target, sourcePosition: endpointPositions.source,
            onSelect: () => setSelectedId(n.id), onOpenDetail: () => openDetail(n),
            onToggle: () => setCollapsed(cur => { const nx = new Set(cur); nx.has(n.id) ? nx.delete(n.id) : nx.add(n.id); return nx; }),
            onAddChild: () => addChildNode(n.id),
            onPreview: (e) => { if (previewCloseTimer.current) window.clearTimeout(previewCloseTimer.current); setPreview({ node: n, x: e.clientX, y: e.clientY }); },
            onPreviewEnd: () => { if (previewCloseTimer.current) window.clearTimeout(previewCloseTimer.current); previewCloseTimer.current = window.setTimeout(() => { if (!previewHoverRef.current) setPreview(null); }, 250); },
            onDelete: () => deleteNode(n.id),
            onContextMenu: (e) => { setSelectedId(n.id); setCtxMenu({ x: e.clientX, y: e.clientY, nodeId: n.id }); } }
        : prevData;
      const prevObj = (prev?.obj ?? null) as Node<FlowNodeData> | null;
      const measured = measuredMap[n.id] ?? prevObj?.measured;
      const obj: Node<FlowNodeData> = prevObj
        ? { ...prevObj, position: p, data, measured, sourcePosition: endpointPositions.source, targetPosition: endpointPositions.target }
        : { id: n.id, type: "mmNode", position: p, data, measured, sourcePosition: endpointPositions.source, targetPosition: endpointPositions.target };
      cache.set(n.id, { node: n, full, px: p.x, py: p.y, selected, hasChildren, collapsed: isCollapsed, obj });
      main.push(obj);
    }
    const stickerNodes: Node<StickerNodeData>[] = [];
    const legacyStickerCluster = stickers.length > 1 && stickers.every(s => s.positionX === 100 && s.positionY === 100);
    for (const [index, s] of stickers.entries()) {
      const sid = `sticker-${s.id}`;
      alive.add(sid);
      const p = posOverrides[sid] ?? (legacyStickerCluster ? stickerSpawnPosition(index) : { x: s.positionX, y: s.positionY });
      const prev = cache.get(sid);
      if (prev && prev.full === full && prev.node === s && prev.px === p.x && prev.py === p.y) {
        stickerNodes.push(prev.obj as Node<StickerNodeData>);
        continue;
      }
      // 与节点同理：纯位置变化（拖动中）复用旧 data，避免贴纸每帧重渲染
      const prevData = prev ? (prev.obj as Node<StickerNodeData>).data : null;
      // 与节点同理：full 变化时贴纸的 onUpdate/onRotate/onDelete 闭包会过期，必须重建 data
      const dataChanged = !prevData || prev!.full !== full || prev!.node !== s;
      const data: StickerNodeData = dataChanged
        ? { sticker: s, onUpdate: (patch: { content?: string; color?: string; imageData?: string; rotation?: number }) => {
            const next = { ...s, ...patch };
            void mmApi.upsertSticker({ documentId: full.document.id, sticker: next });
            onDocumentUpdate({ ...full, stickers: full.stickers.map(x => x.id === s.id ? next : x) });
          }, onRotate: (delta: number) => {
            const current = s.rotation ?? stickerRotation(s.id);
            const nextRotation = Math.max(-180, Math.min(180, current + delta));
            const next = { ...s, rotation: nextRotation };
            void mmApi.upsertSticker({ documentId: full.document.id, sticker: next });
            onDocumentUpdate({ ...full, stickers: full.stickers.map(x => x.id === s.id ? next : x) });
          }, onReplaceImage: async () => {
            const selected = await openDialog({ multiple: false, title: t("mindmap.replaceStickerImg"), filters: [{ name: t("mindmap.image"), extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico"] }] });
            if (typeof selected !== "string") return;
            try {
              const imageData = await invoke<string>("image_to_base64", { filePath: selected });
              const next = { ...s, imageData };
              void mmApi.upsertSticker({ documentId: full.document.id, sticker: next });
              onDocumentUpdate({ ...full, stickers: full.stickers.map(x => x.id === s.id ? next : x) });
            } catch (e) { console.error("[mindmap] 读取贴纸图片失败:", e); }
          }, onDelete: () => {
            void mmApi.deleteSticker({ documentId: full.document.id, stickerId: s.id });
            onDocumentUpdate({ ...full, stickers: full.stickers.filter(x => x.id !== s.id) });
          } }
        : prevData;
      const prevObj = (prev?.obj ?? null) as Node<StickerNodeData> | null;
      const measured = measuredMap[sid] ?? prevObj?.measured;
      const obj: Node<StickerNodeData> = prevObj
        ? { ...prevObj, position: p, data, measured }
        : { id: sid, type: "stickerNode", position: p, data, measured };
      cache.set(sid, { node: s, full, px: p.x, py: p.y, selected: false, hasChildren: false, collapsed: false, obj });
      stickerNodes.push(obj);
    }
    // 清理不再显示的缓存项
    for (const k of cache.keys()) if (!alive.has(k)) cache.delete(k);
    return [...main, ...stickerNodes];
  }, [visibleNodes, layout, selectedId, collapsed, childrenCount, stickers, full, onDocumentUpdate, highlightChain, posOverrides, measuredMap, addChildNode, deleteNode, openDetail, endpointPositions]);

  const edges = useMemo<Edge[]>(() => visibleNodes.flatMap(n => {
    if (!n.parentId || !visibleNodes.some(p => p.id === n.parentId)) return [];
    const isOnChain = highlightChain.includes(n.id) && highlightChain.includes(n.parentId);
    return [{ id: `mm-e-${n.id}`, source: n.parentId, target: n.id, type: "colorE", style: isOnChain ? { strokeWidth: 2, opacity: 0.9 } : {},
      data: { color: effectiveNodeColor(n) }, markerEnd: { type: MarkerType.ArrowClosed, color: "#f8fafc" } } as Edge];
  }), [visibleNodes, highlightChain]);

  // 受控节点：React Flow 拖放时把位置写入 posOverrides。仅处理 position 变更，
  // 选择由 selectedId 管理。一次 setState → 一次渲染 → 收敛，无反馈循环。
  // 关键：dimensions 变更（RF 实测的节点尺寸）必须记录下来并挂回节点对象——
  // adoptUserNodes 只认 userNode.measured，节点对象没有 measured 时 RF 会判定
  // 未测量并重新测量，导致拖动中节点尺寸逐帧塌陷/恢复 → 闪烁（JSON 画布用
  // applyNodeChanges 写回 measured 所以不闪）。
  const onNodesChange = useCallback((changes: NodeChange[]) => {
    setPosOverrides(prev => {
      let next: Record<string, PosOverride> | null = null;
      for (const ch of changes) {
        if (ch.type === "position" && ch.position) {
          next = next ?? { ...prev };
          next[ch.id] = { x: ch.position.x, y: ch.position.y };
        }
      }
      return next ?? prev;
    });
    for (const ch of changes) {
      if (ch.type === "dimensions" && ch.id && ch.dimensions) {
        const d = ch.dimensions;
        const dims = { width: d.width ?? 0, height: d.height ?? 0 };
        setMeasuredMap(prev => {
          const old = prev[ch.id];
          if (old && old.width === dims.width && old.height === dims.height) return prev;
          return { ...prev, [ch.id]: dims };
        });
      }
    }
  }, []);

  useEffect(() => { const t = window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 0); return () => window.clearTimeout(t); }, [fitView, full.document.id, collapsed]);

  // 计划日历点击节点 → 选中并打开详情，同时把视口聚焦到该节点（重复点击用 ts 区分）。
  // 只用 ref 读取节点、不依赖 byId：处理完立即回调清除 focusRequest，避免 byId 变化时
  // effect 重跑 → 把用户刚关闭的详情弹窗又打开（「弹窗关不掉」bug）。
  useEffect(() => {
    if (!focusRequest) return;
    const n = byIdRef.current.get(focusRequest.nodeId);
    onFocusHandled();
    if (!n) return;
    setSelectedId(n.id);
    setDetailNode(n);
    const t = window.setTimeout(() => fitView({ nodes: [{ id: n.id }], padding: 0.4, duration: 300, maxZoom: 1.2 }), 60);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusRequest]);

  const updateNode = useCallback((patch: Partial<MindmapNode>) => {
    if (!detailNode) return;
    const updated = { ...detailNode, ...patch, planAt: patch.planAt !== undefined ? patch.planAt : detailNode.planAt ?? null, updatedAt: new Date().toISOString() };
    // 注意：不能 setDetailNode(updated) —— 弹窗关闭时的 unmount 自动保存会再次
    // 调用 updateNode → setDetailNode → 把刚关闭的弹窗重新打开（无法关闭的 bug）。
    // 弹窗内部用本地 state 渲染编辑，父级 detailNode 仅用于挂载，无需同步。
    void mmApi.upsertNode({ documentId: full.document.id, node: updated });
    onDocumentUpdate({ ...full, nodes: full.nodes.map(n => n.id === updated.id ? updated : n) });
  }, [detailNode, full, onDocumentUpdate]);

  const scheduleFlush = useCallback(() => {
    if (flushTimer.current) window.clearTimeout(flushTimer.current);
    flushTimer.current = window.setTimeout(() => {
      flushTimer.current = null;
      const pos = pendingPos.current;
      const stMap = pendingStickers.current;
      pendingPos.current = [];
      pendingStickers.current = new Map();
      const cur = fullRef.current;
      if (pos.length) void mmApi.updatePositions(cur.document.id, pos);
      stMap.forEach((p, sid) => {
        const s = cur.stickers.find(x => x.id === sid);
        if (s) void mmApi.upsertSticker({ documentId: cur.document.id, sticker: { ...s, positionX: p.x, positionY: p.y } });
      });
      setLastSaved(Date.now());
    }, 300);
  }, []);

  // 卸载时把未刷新的坐标立即写入后端，避免快速切换文档丢坐标
  useEffect(() => () => {
    if (flushTimer.current) window.clearTimeout(flushTimer.current);
    const pos = pendingPos.current;
    const stMap = pendingStickers.current;
    if (pos.length || stMap.size) {
      const cur = fullRef.current;
      if (pos.length) void mmApi.updatePositions(cur.document.id, pos);
      stMap.forEach((p, sid) => {
        const s = cur.stickers.find(x => x.id === sid);
        if (s) void mmApi.upsertSticker({ documentId: cur.document.id, sticker: { ...s, positionX: p.x, positionY: p.y } });
      });
    }
  }, []);

  const onNodeDragStop = useCallback((_e: MouseEvent | TouchEvent, node: Node) => {
    onHistoryPush(); // 一次拖放 = 一步撤销
    const cur = fullRef.current;
    const now = new Date().toISOString();
    if (node.id.startsWith("sticker-")) {
      const sid = node.id.replace("sticker-", "");
      pendingStickers.current.set(sid, { x: node.position.x, y: node.position.y });
      onDocumentUpdate({ ...cur, document: { ...cur.document, updatedAt: now }, stickers: cur.stickers.map(s => s.id === sid ? { ...s, positionX: node.position.x, positionY: node.position.y, updatedAt: now } : s) });
    } else {
      pendingPos.current.push({ nodeId: node.id, x: node.position.x, y: node.position.y });
      onDocumentUpdate({ ...cur, document: { ...cur.document, updatedAt: now }, nodes: cur.nodes.map(n => n.id === node.id ? { ...n, positionX: node.position.x, positionY: node.position.y, updatedAt: now } : n) });
    }
    scheduleFlush();
  }, [onDocumentUpdate, scheduleFlush, onHistoryPush]);

  const addSticker = useCallback((imageData = "") => {
    onHistoryPush();
    const id = `s${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    const spawn = stickerSpawnPosition(full.stickers.length);
    const now = new Date().toISOString();
    const s: MindmapSticker = { id, documentId: full.document.id, content: "", imageData, rotation: stickerRotation(id), color: STICKER_PALETTE[Math.floor(Math.random() * STICKER_PALETTE.length)], positionX: spawn.x, positionY: spawn.y, createdAt: now, updatedAt: now };
    void mmApi.upsertSticker({ documentId: full.document.id, sticker: s });
    onDocumentUpdate({ ...full, stickers: [...full.stickers, s] });
  }, [full, onDocumentUpdate, onHistoryPush]);

  const addImageSticker = useCallback(async () => {
    const selected = await openDialog({ multiple: false, title: t("mindmap.pickImageSticker"), filters: [{ name: t("mindmap.image"), extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico"] }] });
    if (typeof selected !== "string") return;
    try {
      const imageData = await invoke<string>("image_to_base64", { filePath: selected });
      addSticker(imageData);
    } catch (e) {
      onError(t("mindmap.readImageFail", { err: String(e) }));
    }
  }, [addSticker]);

  const deleteSelected = useCallback(() => {
    if (!selectedId) return;
    deleteNode(selectedId);
  }, [selectedId, deleteNode]);

  const relayout = useCallback(() => {
    const lp = layoutTree(full.nodes, dir);
    const n2 = flowNodes.filter(n => !n.id.startsWith("sticker-")).map(n => ({ nodeId: n.id, x: lp.get(n.id)?.x ?? 0, y: lp.get(n.id)?.y ?? 0 }));
    void mmApi.updatePositions(full.document.id, n2);
    setPosOverrides(prev => {
      if (n2.length === 0) return prev;
      const next = { ...prev };
      for (const p of n2) next[p.nodeId] = { x: p.x, y: p.y };
      return next;
    });
    window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 30);
  }, [flowNodes, full, dir, fitView]);

  // 切换布局方向：立即按新方向重排（覆盖已保存坐标）并适配视口，同时持久化方向
  const changeDir = useCallback((d: LayoutDir) => {
    if (d === dir) return;
    setDir(d);
    void mmApi.updateLayoutDir(full.document.id, d);
    const lp = layoutTree(full.nodes, d);
    const n2 = flowNodes.filter(n => !n.id.startsWith("sticker-")).map(n => ({ nodeId: n.id, x: lp.get(n.id)?.x ?? 0, y: lp.get(n.id)?.y ?? 0 }));
    void mmApi.updatePositions(full.document.id, n2);
    setPosOverrides(prev => {
      const next = { ...prev };
      for (const p of n2) next[p.nodeId] = { x: p.x, y: p.y };
      return next;
    });
    window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 30);
  }, [dir, flowNodes, full, fitView]);

  const changeBackgroundTexture = useCallback((texture: BackgroundTexture) => {
    if (texture === backgroundTexture) return;
    onHistoryPush();
    const now = new Date().toISOString();
    const document = { ...full.document, backgroundTexture: texture, updatedAt: now };
    void mmApi.updateBackgroundTexture(full.document.id, texture);
    onDocumentUpdate({ ...full, document });
  }, [backgroundTexture, full, onDocumentUpdate, onHistoryPush]);

  const onNodeContextMenu = useCallback((event: React.MouseEvent, node: Node) => {
    event.preventDefault();
    if (!node.id.startsWith("sticker-") && byId.has(node.id)) { setSelectedId(node.id); setCtxMenu({ x: event.clientX, y: event.clientY, nodeId: node.id }); }
  }, [byId]);

  useEffect(() => { const close = () => setCtxMenu(null); window.addEventListener("click", close); return () => window.removeEventListener("click", close); }, []);

  // ── 键盘快捷键 ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (detailNode) return; // Detail modal open - let it handle keys
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement) return;
      if (e.key === "Delete" || e.key === "Backspace") { if (selectedId) { e.preventDefault(); deleteSelected(); } }
      else if (e.key === "Tab") { e.preventDefault(); addChildNode(); }
      else if (e.key === "Enter") { if (selectedId) { e.preventDefault(); const n = byId.get(selectedId); if (n) setDetailNode(n); } }
      else if (e.key === "Escape") { setSelectedId(null); setCtxMenu(null); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedId, detailNode, deleteSelected, addChildNode, byId]);

  return (
    <div className="relative h-full min-h-0" style={BACKGROUND_TEXTURE_STYLES[backgroundTexture]}>
      <ReactFlow nodes={flowNodes} edges={edges} nodeTypes={mmNodeTypes} edgeTypes={mmEdgeTypes}
        onNodesChange={onNodesChange} onNodeDragStop={onNodeDragStop} onConnect={onConnect}
        onNodeContextMenu={(e, n) => onNodeContextMenu(e, n as Node)} minZoom={0.1} maxZoom={2.5} nodesConnectable
        proOptions={{ hideAttribution: true }}>
        <MiniMap style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95"
          nodeColor={(n) => { const d = n.data as FlowNodeData | StickerNodeData; return 'node' in d ? effectiveNodeColor(d.node) : "#fef3c7"; }}
          nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2,6,23,0.72)" pannable zoomable />
        <Controls className="canvas-flow-controls" showInteractive={false} />
      </ReactFlow>

      {/* Compact floating toolbar */}
      <div className="absolute right-4 top-4 z-10 flex flex-col gap-1">
        <div className="rounded-lg border border-white/10 bg-slate-900/95 p-1 shadow-lg flex flex-col gap-0.5">
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={relayout} title={t("mindmap.autoLayout")}><LayoutGrid className="h-3 w-3" />{t("mindmap.layout")}</button>
          <select className="w-full rounded border border-white/10 bg-slate-900/95 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-400/60" value={dir} onChange={(e) => changeDir(e.target.value as LayoutDir)} title={t("mindmap.layoutDir")}>
            {Object.entries(LAYOUT_DIR_KEYS).map(([k, v]) => <option key={k} value={k}>{t(v)}</option>)}
          </select>
          <label className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300" title={t("mindmap.bgTexture")}>
            <Palette className="h-3 w-3 shrink-0 text-slate-400" />
            <span className="shrink-0">{t("mindmap.bg")}</span>
            <select className="min-w-0 flex-1 rounded border border-white/10 bg-slate-900 px-1 py-0.5 text-[10px] text-slate-300 outline-none focus:border-cyan-400/60" value={backgroundTexture} onChange={(e) => changeBackgroundTexture(e.target.value as BackgroundTexture)}>
              {Object.entries(BACKGROUND_TEXTURE_KEYS).map(([k, v]) => <option key={k} value={k}>{t(v)}</option>)}
            </select>
          </label>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={onOpenCalendar} title={t("mindmap.planCalendarBtn")}><Calendar className="h-3 w-3" />{t("mindmap.planCalendar")}</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={() => addChildNode()} title={t("mindmap.addChild")}><Plus className="h-3 w-3" />{t("mindmap.childNode")}</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-cyan-300 hover:bg-cyan-400/10 hover:text-cyan-200" onClick={() => addNode(null)} title={t("mindmap.newRootNode")}><ListTree className="h-3 w-3" />{t("mindmap.newRootNode")}</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-cyan-300 hover:bg-cyan-400/10 hover:text-cyan-200" onClick={onAiProject} title={t("mindmap.aiProject")}><Code2 className="h-3 w-3" />{t("mindmap.aiProject")}</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-cyan-300 hover:bg-cyan-400/10 hover:text-cyan-200" onClick={onAiText} title={t("mindmap.aiParseDoc")}><FileText className="h-3 w-3" />{t("mindmap.aiParseDoc")}</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={() => addSticker()} title={t("mindmap.textSticker")}><StickyNote className="h-3 w-3" />{t("mindmap.textSticker")}</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={() => void addImageSticker()} title={t("mindmap.imageSticker")}><Image className="h-3 w-3" />{t("mindmap.imageSticker")}</button>
        </div>
        {/* 自动保存指示 */}
        {lastSaved && (
          <div className="rounded-lg border border-emerald-400/20 bg-emerald-950/60 px-2 py-1 text-[8px] text-emerald-300 shadow-lg">
            {t("mindmap.autoSaved", { time: new Date(lastSaved).toLocaleTimeString("zh-CN", { hour12: false }) })}
          </div>
        )}
        {/* Keyboard hints */}
        {selectedId && (
          <div className="rounded-lg border border-white/10 bg-slate-900/95 p-1.5 shadow-lg text-[8px] text-slate-600 leading-relaxed">
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Tab</kbd> {t("mindmap.kbdChild")}</div>
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Enter</kbd> {t("mindmap.kbdDetail")}</div>
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Del</kbd> {t("mindmap.kbdDelete")}</div>
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Esc</kbd> {t("mindmap.kbdCancel")}</div>
          </div>
        )}
      </div>

      {ctxMenu && (
        <div className="fixed z-50 min-w-[160px] rounded-lg border border-white/10 bg-[#101827] py-1 shadow-2xl" style={{ left: ctxMenu.x, top: ctxMenu.y }} onClick={e => e.stopPropagation()}>
          <div className="border-b border-white/10 px-3 py-1.5 text-[10px] font-semibold text-slate-400">{byId.get(ctxMenu.nodeId)?.name ?? ctxMenu.nodeId}</div>
          <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
            onClick={() => { const n = byId.get(ctxMenu.nodeId); if (n) setDetailNode(n); setCtxMenu(null); }}><Sparkles className="h-3.5 w-3.5" />{t("mindmap.viewDetail")}</button>
          <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
            onClick={() => { addChildNode(ctxMenu.nodeId); setCtxMenu(null); }}><Plus className="h-3.5 w-3.5" />{t("mindmap.addChild")}</button>
          <button type="button" disabled={byId.get(ctxMenu.nodeId)?.parentId === null} className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white disabled:opacity-40"
            onClick={() => { makeRoot(ctxMenu.nodeId); setCtxMenu(null); }}><ListTree className="h-3.5 w-3.5" />{t("mindmap.makeRoot")}</button>
          {(byId.get(ctxMenu.nodeId)?.sources?.length ?? 0) > 0 && (() => {
            const n = byId.get(ctxMenu.nodeId)!;
            const root = full.document.sourceDesc || "";
            const srcs = n.sources ?? [];
            return (<>
              <div className="border-t border-white/10" />
              {srcs.length === 1 && root && (() => { const p = `${root.replace(/\\/g, "/")}/${srcs[0]}`; return (
                <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
                  onClick={() => { void invoke("launcher_open_file_location", { path: p }).catch(() => {}); setCtxMenu(null); }}><Folder className="h-3.5 w-3.5" />{t("mindmap.openFolder")}</button>
              ); })()}
              {srcs.map((s, i) => (
                <button key={i} type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
                  onClick={() => {
                    if (root) { const p = `${root.replace(/\\/g, "/")}/${s}`; void openPath(p).catch(() => {}); }
                    setCtxMenu(null);
                  }} title={s}><FileText className="h-3.5 w-3.5" />{t("mindmap.openFile", { name: s.length > 20 ? s.slice(0, 18) + "…" : s })}</button>
              ))}
            </>);
          })()}
          <div className="border-t border-white/10" />
          <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-red-300 transition hover:bg-white/[0.08] hover:text-red-100"
            onClick={() => { deleteNode(ctxMenu.nodeId); setCtxMenu(null); }}><Trash2 className="h-3.5 w-3.5" />{t("mindmap.delete")}</button>
        </div>)}
      {/* 悬浮只读预览气泡（跟随鼠标；鼠标移入气泡后保持打开，可滚动查看长内容） */}
      {preview && (() => {
        const pc = effectiveNodeColor(preview.node);
        const left = Math.min(preview.x + 14, window.innerWidth - 380);
        const top = Math.min(preview.y + 14, window.innerHeight - 340);
        return (
          <div className="fixed z-[300]" style={{ left, top }}
            onMouseEnter={() => { previewHoverRef.current = true; if (previewCloseTimer.current) { window.clearTimeout(previewCloseTimer.current); previewCloseTimer.current = null; } }}
            onMouseLeave={() => { previewHoverRef.current = false; setPreview(null); }}
            onWheel={(e) => e.stopPropagation()}>
            <div className="w-[350px] rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl">
              <div className="flex items-center gap-1.5 border-b border-white/10 px-3 py-2">
                <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: pc, boxShadow: `0 0 6px ${pc}` }} />
                <span className="min-w-0 truncate text-[11px] font-semibold" style={{ color: pc }}>{preview.node.name}</span>

              </div>
              <div className="max-h-[300px] overflow-y-auto p-3">
                <MindmapMarkdown content={preview.node.detail || preview.node.description || t("mindmap.missingView")} />
              </div>
            </div>
          </div>
        );
      })()}
      {detailNode && <DetailModal node={detailNode} accent={accent} projectRoot={full.document.sourceDesc} onUpdate={updateNode} onClose={() => setDetailNode(null)} />}
    </div>
  );
}

function Canvas({ full, accent, onDocumentUpdate, onHistoryPush, historyVersion, onAiProject, onAiText, onError, onOpenCalendar, focusRequest, onFocusHandled }: { full: DocumentFull; accent: string; onDocumentUpdate: (d: DocumentFull) => void; onHistoryPush: () => void; historyVersion: number; onAiProject: () => void; onAiText: () => void; onError: (message: string) => void; onOpenCalendar: () => void; focusRequest: { nodeId: string; ts: number } | null; onFocusHandled: () => void }) {
  return <div className="h-full min-h-0 bg-[#080f1c]"><ReactFlowProvider><CanvasInner full={full} accent={accent} onDocumentUpdate={onDocumentUpdate} onHistoryPush={onHistoryPush} historyVersion={historyVersion} onAiProject={onAiProject} onAiText={onAiText} onError={onError} onOpenCalendar={onOpenCalendar} focusRequest={focusRequest} onFocusHandled={onFocusHandled} /></ReactFlowProvider></div>;
}

// ════════════ 主面板 ════════════

function formatTime(v: string): string {
  if (!v) return ""; const d = new Date(v); if (Number.isNaN(d.getTime())) return v;
  return new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(d).replace(/\//g, "-");
}

// ─── 文件夹树：扁平列表 → 层级（支持拖拽整理） ───

interface FolderNode {
  folder: MindmapFolder;
  children: FolderNode[];
}

function buildChildren(folders: MindmapFolder[], parentId: string | null): FolderNode[] {
  return folders
    .filter((f) => (f.parentId ?? null) === parentId)
    .map((f) => ({ folder: f, children: buildChildren(folders, f.id) }));
}

/** candidateId 是否位于 id 的子树（后代）中 */
function isFolderDescendant(folders: MindmapFolder[], id: string, candidateId: string): boolean {
  const children = folders.filter((f) => f.parentId === id);
  return children.some((c) => c.id === candidateId || isFolderDescendant(folders, c.id, candidateId));
}

/** 从根到 activeId 的路径（不含「全部文档」占位） */
function getFolderPath(folders: MindmapFolder[], activeId: string | null): MindmapFolder[] {
  if (!activeId) return [];
  const chain: MindmapFolder[] = [];
  let cur = folders.find((f) => f.id === activeId) ?? null;
  while (cur) {
    const c = cur;
    chain.unshift(c);
    cur = c.parentId ? (folders.find((f) => f.id === c.parentId) ?? null) : null;
  }
  return chain;
}

export default function MindmapPanel() {
  const { t } = useTranslation();
  const [docs, setDocs] = useState<MindmapDocument[]>([]);
  const [folders, setFolders] = useState<MindmapFolder[]>([]);
  const [activeFolderId, setActiveFolderId] = useState<string | null>(null);
  const [dragOverFolderId, setDragOverFolderId] = useState<string | null>(null);
  const [collapsedFolders, setCollapsedFolders] = useState<Set<string>>(new Set());
  // 跟随鼠标的拖拽浮层状态（自定义 pointer 拖拽；绕开 HTML5 DnD 在 WebView2 起拖/光标问题）
  const [ghost, setGhost] = useState<{ kind: "doc" | "folder"; name: string; x: number; y: number; ok: boolean } | null>(null);
  // 拖拽会话：按下即记录候选，移动超过阈值才确认为拖动
  const dragHandleRef = useRef<{ kind: "doc" | "folder"; id: string; name: string; startX: number; startY: number; active: boolean } | null>(null);
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");
  const [full, setFull] = useState<DocumentFull | null>(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [showCreate, setShowCreate] = useState(false);
  const [showFolderCreate, setShowFolderCreate] = useState(false);
  const [editingFolder, setEditingFolder] = useState<MindmapFolder | null>(null);
  const [folderName, setFolderName] = useState("");
  const [renamingDocId, setRenamingDocId] = useState<string | null>(null);
  const [renameDocName, setRenameDocName] = useState("");
  const [showAi, setShowAi] = useState<"project" | "text" | null>(null);
  const [textInput, setTextInput] = useState("");
  const [textTitle, setTextTitle] = useState("");
  const [projectPath, setProjectPath] = useState("");
  const [aiLoading, setAiLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarW, setSidebarW] = useState(260);
  const [confirmState, setConfirmState] = useState<{ title: string; message: string; action: () => void } | null>(null);
  // 计划日历：跨文档按日查看计划（具体发生记录由日历弹窗按可见范围向后端拉取）
  const [showCalendar, setShowCalendar] = useState(false);
  // AI 导入校验报告弹窗（导入完成后展示节点数/修复轮数/残留诊断）
  const [aiReport, setAiReport] = useState<AiImportResult | null>(null);
  // 日历点击节点后：通知画布选中并打开详情（ts 用于重复点击同一节点也触发）
  const [calFocus, setCalFocus] = useState<{ nodeId: string; ts: number } | null>(null);
  // 侧栏宽度拖拽期间是否发生了位移（用于区分「点击收起」与「拖动调宽」）
  const sbResizeRef = useRef<{ moved: boolean }>({ moved: false });

  const flash = useCallback((m: string) => { setNotice(m); window.setTimeout(() => setNotice(""), 2600); }, []);

  // 稳定引用：避免每次父组件渲染都新建函数，导致 CanvasInner 的
  // flowNodes useMemo 反复重建、setNodes 连环触发造成卡顿。
  const onDocumentUpdated = useCallback((d: DocumentFull) => {
    setFull(d);
    setDocs(prev => prev.map(x => x.id === d.document.id ? d.document : x));
  }, []);

  // ── 撤销/重做（Ctrl+Z / Ctrl+Y）──
  // 快照式历史：每次离散操作前把当前 DocumentFull 压入撤销栈；撤销/重做时恢复快照并写回后端。
  const undoStack = useRef<DocumentFull[]>([]);
  const redoStack = useRef<DocumentFull[]>([]);
  const currentRef = useRef<DocumentFull | null>(null);
  const [historyVersion, setHistoryVersion] = useState(0);
  useEffect(() => { currentRef.current = full; }, [full]);

  const commitHistory = useCallback(() => {
    const cur = currentRef.current;
    if (cur) {
      undoStack.current.push(cur);
      if (undoStack.current.length > 50) undoStack.current.shift();
      redoStack.current = [];
    }
  }, []);

  // 恢复快照：本地 state 立即回退，并把节点/贴纸差异写回后端（持久化）
  const restoreSnapshot = useCallback((snap: DocumentFull) => {
    const cur = currentRef.current;
    setFull(snap);
    setDocs(ds => ds.map(x => x.id === snap.document.id ? snap.document : x));
    setHistoryVersion(v => v + 1);
    if (!cur) return;
    void (async () => {
      // 逐条写回，任一失败就停止并提示（不再静默吞错，避免画布显示已恢复但未持久化）
      try {
        const snapIds = new Set(snap.nodes.map(n => n.id));
        for (const n of cur.nodes) if (!snapIds.has(n.id)) { await mmApi.deleteNode({ documentId: snap.document.id, nodeId: n.id }); }
        for (const n of snap.nodes) { await mmApi.upsertNode({ documentId: snap.document.id, node: n }); }
        const snapStickers = new Set(snap.stickers.map(s => s.id));
        for (const s of cur.stickers) if (!snapStickers.has(s.id)) { await mmApi.deleteSticker({ documentId: snap.document.id, stickerId: s.id }); }
        for (const s of snap.stickers) { await mmApi.upsertSticker({ documentId: snap.document.id, sticker: s }); }
        if (snap.document.backgroundTexture !== cur.document.backgroundTexture) {
          await mmApi.updateBackgroundTexture(snap.document.id, snap.document.backgroundTexture || "dots");
        }
      } catch (e) {
        // 本地已恢复，但持久化失败：明确告知用户，避免数据丢失后无从排查
        console.error("[mindmap] 撤销/重做持久化失败:", e);
        setError(t("mindmap.undoRedoFail", { err: String(e) }));
      }
    })();
  }, []);

  const undo = useCallback(() => {
    const prev = undoStack.current.pop();
    if (!prev) return;
    const cur = currentRef.current;
    if (cur) redoStack.current.push(cur);
    restoreSnapshot(prev);
  }, [restoreSnapshot]);

  const redo = useCallback(() => {
    const next = redoStack.current.pop();
    if (!next) return;
    const cur = currentRef.current;
    if (cur) undoStack.current.push(cur);
    restoreSnapshot(next);
  }, [restoreSnapshot]);

  // 全局 Ctrl+Z / Ctrl+Y（输入框内不拦截，保留浏览器原生文本撤销）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) return;
      const k = e.key.toLowerCase();
      if (k === "z") { e.preventDefault(); if (e.shiftKey) redo(); else undo(); }
      else if (k === "y") { e.preventDefault(); redo(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo]);

  const refreshFolders = useCallback(async () => {
    try { setFolders(await mmApi.listFolders()); } catch {}
  }, []);

  useEffect(() => {
    void mmApi.init().then(async () => {
      const [ld, lf] = await Promise.all([mmApi.list(), mmApi.listFolders()]);
      setDocs(ld); setFolders(lf);
      // 挂载时恢复上次打开的思维导图（模块卸载重挂载后仍停留在原文档）
      try {
        const last = localStorage.getItem(MM_LAST_DOC_KEY);
        if (last && ld.some(d => d.id === last)) {
          const f = await mmApi.load(last);
          if (f) setFull(f);
        }
      } catch { /* 忽略恢复失败 */ }
    }).catch(() => {});
    // 今日计划提醒：挂载时统计今天（含重复计划）的发生记录数
    const today = toYMD(new Date());
    void mmApi.plannedOccurrences(today, today).then(list => {
      if (list.length > 0) flash(t("mindmap.todayPlanFlash", { count: list.length }));
    }).catch(() => {});
    // 同步后端：更新托盘小红点（今天有计划时点亮），系统通知同一天只弹一次
    void invoke("mm_refresh_plan_badge").catch(() => {});
    void invoke<AiConfig>("get_ai_config").then(cfg => {
      setConfig(cfg);
      const p = cfg.providers.find(x => x.api_key && x.openai_url) ?? cfg.providers[0];
      if (p) { setProviderId(p.id); setModelId(p.active_model_id ?? p.models[0]?.id ?? ""); }
    }).catch(() => setError(t("mindmap.aiCfgFail")));
  }, []);

  // 当前文档变化时持久化 id，供挂载恢复
  useEffect(() => {
    if (full?.document.id) {
      try { localStorage.setItem(MM_LAST_DOC_KEY, full.document.id); } catch { /* 忽略 */ }
    }
  }, [full?.document.id]);

  const providers = useMemo(() => (config?.providers ?? []).filter(p => p.api_key && p.openai_url), [config]);

  // AI 导入入口：无当前文档时先自动新建空白文档承载导入结果（可导入并新建）
  const openAiImport = useCallback(async (kind: "project" | "text") => {
    if (!full) {
      try {
        const doc = await mmApi.create({ name: kind === "project" ? t("mindmap.aiProjectDoc") : t("mindmap.aiTextDoc"), description: "", sourceType: kind === "project" ? "ai_project" : "ai_text", folderId: null });
        setDocs(prev => [doc, ...prev]);
        const f = await mmApi.load(doc.id);
        if (f) setFull(f);
        void refreshFolders();
      } catch (e) { flash(String(e)); return; }
    }
    setShowAi(kind);
  }, [full, flash, refreshFolders]);

  const loadDocument = useCallback(async (id: string) => {
    setError("");
    try {
      const f = await mmApi.load(id);
      if (f) { setFull(f); } else { flash(t("mindmap.docNotFound")); }
    } catch (e) { setError(String(e)); }
  }, [flash]);

  // 从校验报告弹窗跳转到对应视图文档
  const openReportDoc = useCallback(async (id: string) => {
    setAiReport(null);
    try {
      const f = await mmApi.load(id);
      if (f) setFull(f); else flash(t("mindmap.docNotFound"));
    } catch (e) { setError(String(e)); }
  }, [flash]);

  // 打开计划日历（弹窗内部按当前可见月份/周自动拉取该范围内的发生记录）
  const openCalendar = useCallback(() => setShowCalendar(true), []);

  // 日历中点击计划：若属于其它文档先切换过去，再让画布定位到该节点
  const openPlannedNode = useCallback(async (p: PlannedOccurrence) => {
    setShowCalendar(false);
    if (!full || full.document.id !== p.documentId) {
      try {
        const f = await mmApi.load(p.documentId);
        if (!f) { flash(t("mindmap.docNotFound")); return; }
        setFull(f);
      } catch (e) { setError(String(e)); return; }
    }
    setCalFocus({ nodeId: p.id, ts: Date.now() });
  }, [full, flash]);

  // 日历拖拽改期：按 from→to 的天数差改写 plan_at（daily/weekly 整条顺延，钟点不变），成功刷新托盘角标
  const movePlanOccurrence = useCallback(async (fromDay: string, toDay: string, nodeId: string): Promise<boolean> => {
    try {
      await mmApi.movePlanOccurrence({ nodeId, fromDay, toDay });
      flash(t("mindmap.planMoved"));
      void invoke("mm_refresh_plan_badge").catch(() => {});
      return true;
    } catch (e) { setError(String(e)); return false; }
  }, [flash]);

  // 日历「添加计划」：在（当前或新建的）文档中创建带该日期（09:00）的计划节点，并打开详情
  const addPlanNode = useCallback(async (ymd: string) => {
    const nowIso = new Date().toISOString();
    let docId = full?.document.id ?? null;
    if (!docId) {
      try {
        const doc = await mmApi.create({ name: t("mindmap.planDoc"), description: "", sourceType: "manual", folderId: null });
        setDocs(prev => [doc, ...prev]);
        const f = await mmApi.load(doc.id);
        if (!f) { flash(t("mindmap.createDocFail")); return; }
        setFull(f);
        docId = doc.id;
        void refreshFolders();
      } catch (e) { flash(String(e)); return; }
    }
    const planIso = new Date(`${ymd}T09:00:00`).toISOString();
    const n: MindmapNode = { id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, documentId: docId, parentId: null, name: t("mindmap.newPlan"), description: "", detail: "", kind: "task", color: "", progress: 0, planAt: planIso, repeat: "none", positionX: 0, positionY: 0, createdAt: nowIso, updatedAt: nowIso };
    try { await mmApi.upsertNode({ documentId: docId, node: n }); }
    catch (e) { setError(String(e)); return; }
    setFull(prev => (prev && prev.document.id === docId) ? { ...prev, nodes: [...prev.nodes, n] } : prev);
    setDocs(prev => prev.map(d => d.id === docId ? { ...d, nodeCount: d.nodeCount + 1, updatedAt: nowIso } : d));
    setShowCalendar(false);
    setCalFocus({ nodeId: n.id, ts: Date.now() });
  }, [full, flash, refreshFolders]);

  const createDoc = useCallback(async (name: string, desc: string, folderId: string | null) => {
    try {
      const doc = await mmApi.create({ name, description: desc, sourceType: "manual", folderId });
      setDocs(prev => [doc, ...prev]);
      if (!folderId) setActiveFolderId(null);
      setShowCreate(false);
      const f = await mmApi.load(doc.id);
      if (f) setFull(f);
      void refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [flash, refreshFolders]);

  const removeDoc = useCallback((id: string, name: string) => {
    setConfirmState({ title: t("mindmap.delMapTitle"), message: t("mindmap.delMapMsg", { name }), action: () => void executeRemoveDoc(id) });
  }, []);

  const executeRemoveDoc = useCallback(async (id: string) => {
    setConfirmState(null);
    try { await mmApi.remove(id); setDocs(prev => prev.filter(d => d.id !== id)); if (full?.document.id === id) setFull(null); refreshFolders(); } catch (e) { flash(String(e)); }
  }, [full, flash]);

  const createFolder = useCallback(async () => {
    if (!folderName.trim()) return;
    try {
      await mmApi.createFolder({ name: folderName.trim(), parentId: activeFolderId });
      setShowFolderCreate(false); setFolderName("");
      await refreshFolders();
      setCollapsedFolders((prev) => {
        if (!activeFolderId) return prev;
        const next = new Set(prev);
        next.delete(activeFolderId); // 展开父目录，让新文件夹可见
        return next;
      });
    } catch (e) { flash(String(e)); }
  }, [folderName, activeFolderId, flash, refreshFolders]);

  const updateFolder = useCallback(async () => {
    if (!editingFolder || !folderName.trim()) return;
    try {
      await mmApi.updateFolder({ id: editingFolder.id, name: folderName.trim() });
      setEditingFolder(null); setFolderName("");
      await refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [editingFolder, folderName, flash, refreshFolders]);

  const loadDocs = useCallback(async () => {
    try { setDocs(await mmApi.list(activeFolderId)); } catch {}
  }, [activeFolderId]);

  const deleteFolder = useCallback((id: string, name: string) => {
    setConfirmState({ title: t("mindmap.delFolderTitle"), message: t("mindmap.delFolderMsg", { name }), action: () => void executeDeleteFolder(id) });
  }, []);

  const executeDeleteFolder = useCallback(async (id: string) => {
    setConfirmState(null);
    try { await mmApi.deleteFolder(id); if (activeFolderId === id) setActiveFolderId(null); await refreshFolders(); await loadDocs(); } catch (e) { flash(String(e)); }
  }, [activeFolderId, flash, refreshFolders, loadDocs]);

  const startRenameDoc = useCallback((d: MindmapDocument) => { setRenamingDocId(d.id); setRenameDocName(d.name); }, []);
  const saveRenameDoc = useCallback(async (d: MindmapDocument) => {
    setRenamingDocId(null);
    const name = renameDocName.trim();
    if (!name || name === d.name) return;
    try {
      await mmApi.update({ id: d.id, name });
      setDocs(prev => prev.map(x => x.id === d.id ? { ...x, name } : x));
      flash(t("mindmap.renamed"));
    } catch (e) { flash(String(e)); }
  }, [renameDocName, flash]);

  const moveDoc = useCallback(async (docId: string, fid: string | null) => {
    try {
      await mmApi.moveDocument({ documentId: docId, folderId: fid });
      await Promise.all([loadDocs(), refreshFolders()]);
    } catch (e) { flash(String(e)); }
  }, [flash, loadDocs]);

  const moveFolder = useCallback(async (fid: string, toParent: string | null) => {
    try {
      await mmApi.moveFolder({ folderId: fid, parentId: toParent });
      await Promise.all([loadDocs(), refreshFolders()]);
    } catch (e) { flash(String(e)); }
  }, [flash, loadDocs]);

  // ── 自定义拖拽（pointer events，绕开 HTML5 DnD）──
  // mousedown 只记录候选；窗口级 mousemove 移动超 5px 激活拖拽并绘制浮层、
  // 用 elementFromPoint 命中检测落点（文件夹行 / 根落点）；mouseup 执行移动。
  // 落点判定：文件夹不允许落入自身或自身后代（后端亦有防环兜底）。
  useEffect(() => {
    const onMove = (ev: MouseEvent) => {
      const d = dragHandleRef.current;
      if (!d) return;
      if (!d.active) {
        if (Math.hypot(ev.clientX - d.startX, ev.clientY - d.startY) < 5) return;
        d.active = true;
      }
      ev.preventDefault();
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      const hitFolder = el?.closest?.("[data-drop-folder]") as HTMLElement | null;
      const hitRoot = el?.closest?.("[data-drop-root]") as HTMLElement | null;
      let ok = false;
      let target: string | null = null;
      if (hitFolder) {
        const tid = hitFolder.getAttribute("data-drop-folder")!;
        if (d.kind === "folder" && (tid === d.id || isFolderDescendant(folders, d.id, tid))) {
          ok = false;
        } else {
          target = tid; ok = true;
        }
      } else if (hitRoot) {
        ok = true;
      }
      setDragOverFolderId(ok ? (target ?? "__root") : null);
      setGhost({ kind: d.kind, name: d.name, x: ev.clientX, y: ev.clientY, ok });
    };
    const onUp = (ev: MouseEvent) => {
      const d = dragHandleRef.current;
      if (!d) return;
      dragHandleRef.current = null;
      if (!d.active) return;
      ev.preventDefault();
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      const hitFolder = el?.closest?.("[data-drop-folder]") as HTMLElement | null;
      const hitRoot = el?.closest?.("[data-drop-root]") as HTMLElement | null;
      let target: string | null = null;
      let moved = false;
      if (hitFolder) {
        const tid = hitFolder.getAttribute("data-drop-folder")!;
        if (d.kind === "folder" && (tid === d.id || isFolderDescendant(folders, d.id, tid))) {
          moved = false; // 文件夹不能落入自身/自身后代
        } else {
          target = tid; moved = true;
        }
      } else if (hitRoot) {
        moved = true; // target 保持 null = 根目录
      }
      if (moved) {
        if (d.kind === "doc") void moveDoc(d.id, target);
        else void moveFolder(d.id, target);
      }
      setDragOverFolderId(null);
      setGhost(null);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
  }, [folders, moveDoc, moveFolder]);

  // 按下时记录拖拽候选（阻止文本选择与滚动起手，保证拖动语义稳定）；
  // 普通点击不受影响（click 事件仍正常派发）。
  const startDragHandle = useCallback((e: React.MouseEvent, kind: "doc" | "folder", id: string, name: string) => {
    if (e.button !== 0) return;
    e.preventDefault();
    dragHandleRef.current = { kind, id, name, startX: e.clientX, startY: e.clientY, active: false };
  }, []);

  useEffect(() => { void loadDocs(); }, [activeFolderId, loadDocs]);

  const toggleFolderCollapse = useCallback((id: string) => {
    setCollapsedFolders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  // 递归渲染文件夹树：每个文件夹既是拖源（可整理到任意目录/根），也是落点（接收文档与文件夹）
  const renderFolderNodes = (nodes: FolderNode[], depth: number): React.ReactNode => {
    return nodes.map((node) => {
      const f = node.folder;
      const over = dragOverFolderId === f.id;
      const open = !collapsedFolders.has(f.id);
      const hasChildren = node.children.length > 0;
      return (
        <div key={f.id}>
<div
            className={`group mb-0.5 flex items-center gap-1 rounded-md py-1.5 pr-1 transition select-none ${over ? "ring-1 ring-cyan-400/70 bg-cyan-400/10" : "hover:bg-white/[0.05]"}`}
            style={{ paddingLeft: 6 + depth * 14 }}
            data-drop-folder={f.id}
            title={depth > 0 ? t("mindmap.dragHintRoot") : t("mindmap.dragHint")}
            onMouseDown={(e) => startDragHandle(e, "folder", f.id, f.name)}
          >
            {hasChildren ? (
              <button type="button" className="flex h-4 w-4 shrink-0 items-center justify-center rounded text-slate-500 hover:text-white cursor-pointer"
                onClick={(e) => { e.stopPropagation(); toggleFolderCollapse(f.id); }} title={open ? t("mindmap.collapse") : t("mindmap.expand")}>
                {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
              </button>
            ) : (
              <span className="w-4 shrink-0" />
            )}
            <button type="button" className="flex min-w-0 flex-1 items-center gap-1.5 text-left cursor-pointer" onClick={() => setActiveFolderId(f.id)} title={t("mindmap.enterFolder", { name: f.name })}>
              <Folder className={`h-3.5 w-3.5 shrink-0 ${depth === 0 ? "text-amber-400" : "text-amber-400/60"}`} />
              <span className="truncate text-[10px] text-slate-300">{f.name}</span>
              <span className="shrink-0 text-[9px] text-slate-400">{f.documentCount}</span>
            </button>
            <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-slate-300 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-white/10 hover:text-white"
              onClick={() => { setEditingFolder(f); setFolderName(f.name); }} title={t("mindmap.rename")}><Pencil className="h-3 w-3" /></button>
            <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-red-400/50 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-red-400/10 hover:text-red-300"
              onClick={() => deleteFolder(f.id, f.name)} title={t("mindmap.deleteFolder")}><Trash2 className="h-3 w-3" /></button>
          </div>
          {open && renderFolderNodes(node.children, depth + 1)}
        </div>
      );
    });
  };

  // 应用 AI 类型路由导入结果：切到主文档，把新生成的视图文档并入侧栏列表
  const applyAiImport = useCallback((r: AiImportResult) => {
    const primary = r.documents.find(d => d.document.id === r.primaryId) ?? r.documents[0];
    if (primary) {
      setFull(primary);
      setDocs(prev => {
        const existing = new Set(prev.map(d => d.id));
        return [...r.documents.map(d => d.document), ...prev.filter(d => !existing.has(d.id))];
      });
    }
    const names = r.documents.map(d => d.document.name).join("、");
    const failTxt = r.failures.length ? t("mindmap.viewFailures", { count: r.failures.length, names: r.failures.map(f => f.view).join("、") }) : "";
    flash(t("mindmap.viewsGenerated", { count: r.documents.length, names: names ? `：${names}` : "", failures: failTxt }));
    setShowAi(null);
    setAiReport(r);
    void refreshFolders();
  }, [flash, refreshFolders]);

  const runAiProject = useCallback(async () => {
    if (!full || !projectPath || !providerId || !modelId) return;
    setAiLoading(true); setError("");
    try {
      const r = await mmApi.aiFromProject({ documentId: full.document.id, projectPath, providerId: providerId || null, modelId: modelId || null });
      applyAiImport(r);
    } catch (e) { setError(String(e)); } finally { setAiLoading(false); }
  }, [full, projectPath, providerId, modelId, applyAiImport]);

  const runAiText = useCallback(async () => {
    if (!full || !textInput.trim() || !providerId || !modelId) return;
    setAiLoading(true); setError("");
    try {
      const r = await mmApi.aiFromText({ documentId: full.document.id, text: textInput, title: textTitle || full.document.name, providerId: providerId || null, modelId: modelId || null });
      applyAiImport(r);
    } catch (e) { setError(String(e)); } finally { setAiLoading(false); }
  }, [full, textInput, textTitle, providerId, modelId, applyAiImport]);

  const exportMd = useCallback(async () => {
    if (!full) return;
    try {
      const md = await mmApi.exportMd(full.document.id);
      const fp = await save({ defaultPath: `${full.document.name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
      if (!fp) return;
      await invoke("write_text_file", { path: fp, content: md });
      flash(t("mindmap.exportedTo", { path: fp }));
    } catch (e) { flash(String(e)); }
  }, [full, flash]);

  // Filtered documents
  const filteredDocs = useMemo(() => {
    if (!search.trim()) return docs;
    const q = search.toLowerCase();
    return docs.filter(d => d.name.toLowerCase().includes(q) || (d.description && d.description.toLowerCase().includes(q)));
  }, [docs, search]);

  // 当前视图的文件夹树：根视图 = 顶层目录；进入目录 = 该目录直接子目录
  const treeRoots = activeFolderId ? buildChildren(folders, activeFolderId) : buildChildren(folders, null);
  const folderPath = getFolderPath(folders, activeFolderId);

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-slate-950/25 text-slate-200">
      <div className="flex min-h-0 flex-1">
        {!sidebarCollapsed && (
          <aside className="group/sb relative flex shrink-0 flex-col border-r border-white/10 bg-slate-950/30" style={{ width: sidebarW }}>
            {/* Search */}
            <div className="border-b border-white/10 px-2 py-1.5 flex items-center gap-1.5">
              <Search className="h-3 w-3 shrink-0 text-slate-600" />
              <input className="min-w-0 flex-1 bg-transparent text-[10px] text-slate-300 outline-none placeholder:text-slate-700" value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("mindmap.searchPh")} />
              {search && <button type="button" className="text-slate-600 hover:text-white" onClick={() => setSearch("")}><X className="h-3 w-3" /></button>}
            </div>
            {!activeFolderId && folders.length === 0 && (
              <div className="px-3 pt-1 text-[9px] text-slate-500">{t("mindmap.noFolders")}</div>
            )}
            <div className="min-h-0 flex-1 overflow-y-auto p-1.5 select-none" data-drop-root>
              {/* 面包屑：路径上级均可点击进入；也是移回相应目录的投放目标 */}
              {activeFolderId && (
                <div className="mb-1 space-y-0.5">
                  <button type="button" className={`mb-0.5 flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-[10px] select-none ${dragOverFolderId === "__root" ? "ring-1 ring-cyan-400/70 bg-cyan-400/10 text-white" : "text-slate-400 hover:bg-white/[0.05] hover:text-white"}`}
                    onClick={() => setActiveFolderId(null)}
                    data-drop-root>
                    <ChevronRight className="h-3 w-3 -rotate-180" />{t("mindmap.allDocs")} <span className="text-[8px] text-slate-500">{t("mindmap.backToRoot")}</span>
                  </button>
                  {folderPath.map((f, i) => {
                    const isLast = i === folderPath.length - 1;
                    const over = dragOverFolderId === f.id;
                    if (isLast) {
                      return (
                        <div key={f.id} className={`mb-0.5 flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-[10px] ${over ? "ring-1 ring-cyan-400/70 bg-cyan-400/10 text-white" : "text-slate-400"}`}>
                          <Folder className="h-3 w-3 shrink-0 text-amber-400/70" />
                          <span className="truncate">{f.name}</span>
                          <span className="shrink-0 text-[9px] text-slate-500">{f.documentCount}</span>
                        </div>
                      );
                    }
                    return (
                      <button key={f.id} type="button" className={`mb-0.5 flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-[10px] cursor-pointer select-none ${over ? "ring-1 ring-cyan-400/70 bg-cyan-400/10 text-white" : "text-slate-400 hover:bg-white/[0.05] hover:text-white"}`}
                        onClick={() => setActiveFolderId(f.id)}
                        data-drop-folder={f.id}>
                        <Folder className="h-3 w-3 shrink-0 text-amber-400/70" />
                        <span className="truncate">{f.name}</span>
                        <span className="shrink-0 text-[9px] text-slate-500">{f.documentCount}</span>
                        <ChevronRight className="h-2.5 w-2.5 shrink-0 text-slate-600" />
                      </button>
                    );
                  })}
                </div>
              )}
              {/* 文件夹树 —— 可拖拽整理：文件↔目录、目录↔目录/根 */}
              {treeRoots.length > 0 && <div className="mb-1.5">{renderFolderNodes(treeRoots, 0)}</div>}
              {/* Documents */}
              {filteredDocs.length === 0 && <div className="py-8 text-center text-[10px] text-slate-500">{search ? t("mindmap.noMatchDocs") : t("mindmap.noDocs")}</div>}
              {filteredDocs.map(d => {
                const IconFn = DOC_SOURCE_ICONS[d.sourceType] ?? DOC_SOURCE_ICONS.manual;
                return (
                <div key={d.id} className={`group mb-1 flex items-center gap-1.5 rounded-md border px-2.5 py-2 transition select-none ${full?.document.id === d.id ? "border-cyan-400/40 bg-cyan-400/10" : "border-white/10 bg-white/[0.02] hover:bg-white/[0.06]"}`}
                  title={t("mindmap.docHint")}
                  data-drop-root
                  onMouseDown={(e) => { if (renamingDocId === d.id) return; startDragHandle(e, "doc", d.id, d.name); }}
                  onDoubleClick={(e) => { if (renamingDocId === d.id) { e.stopPropagation(); return; } e.preventDefault(); e.stopPropagation(); void loadDocument(d.id); }}>
                  <button type="button" className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => { if (renamingDocId === d.id) return; void loadDocument(d.id); }}>
                    {IconFn("h-3.5 w-3.5 shrink-0 text-slate-500")}
                    <span className="flex min-w-0 flex-col">
                      {renamingDocId === d.id ? (
                        <input autoFocus value={renameDocName} onChange={e => setRenameDocName(e.target.value)}
                          onClick={e => e.stopPropagation()} onDoubleClick={e => e.stopPropagation()}
                          onKeyDown={e => { e.stopPropagation(); if (e.key === "Enter") void saveRenameDoc(d); else if (e.key === "Escape") setRenamingDocId(null); }}
                          onBlur={() => void saveRenameDoc(d)}
                          className="w-full rounded border border-cyan-400/50 bg-slate-900 px-1.5 py-0.5 text-[11px] text-white outline-none" />
                      ) : (
                        <span className="truncate text-[11px] text-slate-200">{d.name}</span>
                      )}
                      <span className="text-[9px] text-slate-400">{formatTime(d.updatedAt)}</span>
                    </span>
                  </button>
                  {activeFolderId && <button type="button" className="shrink-0 text-[8px] text-slate-300 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:text-white" onClick={() => void moveDoc(d.id, null)}>{t("mindmap.moveOut")}</button>}
                  <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-slate-400 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-white/10 hover:text-white" onClick={() => startRenameDoc(d)} title={t("mindmap.rename")}><Pencil className="h-3 w-3" /></button>
                  <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-red-300/60 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-red-400/10 hover:text-red-200" onClick={() => removeDoc(d.id, d.name)} title={t("mindmap.delete")}><Trash2 className="h-3 w-3" /></button>
                </div>
              );})}
            </div>
            {/* 底部工具区：新建文件夹/文档 + AI 导入入口 */}
            <div className="shrink-0 space-y-1.5 border-t border-white/10 p-1.5">
              <div className="grid grid-cols-2 gap-1.5">
                <button type="button" className={button} onClick={() => { setShowFolderCreate(true); setFolderName(""); }} title={t("mindmap.newFolder")}><FolderPlus className="h-3 w-3" />{t("mindmap.folder")}</button>
                <button type="button" className={button} onClick={() => setShowCreate(true)} title={t("mindmap.newDoc")}><Plus className="h-3 w-3" />{t("mindmap.doc")}</button>
              </div>
              <div className="grid grid-cols-2 gap-1.5">
                <button type="button" className={`${button} hover:bg-white/10`} style={{ color: ACCENT, borderColor: `${ACCENT}55` }} onClick={() => void openAiImport("project")} title={t("mindmap.aiProject")}><FolderOpen className="h-3 w-3" />{t("mindmap.aiProjectBtn")}</button>
                <button type="button" className={`${button} hover:bg-white/10`} style={{ color: ACCENT, borderColor: `${ACCENT}55` }} onClick={() => void openAiImport("text")} title={t("mindmap.aiParseDoc")}><Sparkles className="h-3 w-3" />{t("mindmap.aiTextBtn")}</button>
              </div>
              <button type="button" className={button} onClick={() => void openCalendar()} title={t("mindmap.planCalendarBtn")}><Calendar className="h-3 w-3" />{t("mindmap.planCalendar")}</button>
              {full && <div className="flex items-center justify-between px-0.5 text-[9px] text-slate-500"><span className="truncate">{t("mindmap.nodesCount", { name: full.document.name, count: full.nodes.length })}</span><button type="button" className={button} onClick={exportMd} title={t("mindmap.exportMd")}><ScrollText className="h-3 w-3" /></button></div>}
            </div>
            {/* 宽度拖拽把手 + 收起按钮（侧边栏右侧） */}
            <div className="absolute -right-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]" title={t("mindmap.dragResize")}
              onMouseDown={(e) => {
                if (e.button !== 0) return;
                e.preventDefault();
                sbResizeRef.current.moved = false;
                const startX = e.clientX; const startW = sidebarW;
                const onMove = (ev: MouseEvent) => { if (Math.abs(ev.clientX - startX) > 2) sbResizeRef.current.moved = true; setSidebarW(Math.min(460, Math.max(170, startW + (ev.clientX - startX)))); };
                const onUp = () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
                window.addEventListener("mousemove", onMove);
                window.addEventListener("mouseup", onUp);
              }}>
              <button type="button" className="flex h-6 w-2.5 items-center justify-center rounded-l bg-slate-800/80 text-slate-400 opacity-0 transition group-hover/sb:opacity-100 pointer-events-none group-hover/sb:pointer-events-auto hover:text-white" title={t("mindmap.collapseSidebar")}
                onClick={(e) => { e.stopPropagation(); if (!sbResizeRef.current.moved) setSidebarCollapsed(true); }}>
                <ChevronLeft className="h-3 w-3" />
              </button>
            </div>
          </aside>
        )}
        {sidebarCollapsed && (
          <button type="button" className="absolute left-2 top-2 z-20 inline-flex h-8 w-8 items-center justify-center rounded-md border border-white/10 bg-slate-900/90 text-slate-300 shadow-lg transition hover:text-white" onClick={() => setSidebarCollapsed(false)} title={t("mindmap.expandSidebar")}>
            <ChevronRight className="h-4 w-4" />
          </button>
        )}
        <main className="relative min-w-0 flex-1">
          {showAi === "project" ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 p-6">
              <div className="flex w-full max-w-[500px] flex-col gap-3">
                <div className="flex items-center gap-2"><Lightbulb className="h-5 w-5" style={{ color: ACCENT }} /><span className="text-sm text-white font-semibold">{t("mindmap.aiProjectTitle")}</span></div>
                <div className="flex gap-2">
                  <button type="button" className={button} onClick={async () => { const d = await openDialog({ directory: true, multiple: false, title: t("mindmap.pickDir") }); if (typeof d === "string") setProjectPath(d); }}><FolderOpen className="h-3 w-3" />{projectPath ? projectPath.split(/[\\\\/]/).pop() : t("mindmap.pickDir")}</button>
                  <select className={`${selectClass} flex-1`} value={providerId} onChange={e => { const p = providers.find(x => x.id === e.target.value); setProviderId(e.target.value); setModelId(p?.active_model_id ?? p?.models[0]?.id ?? ""); }}>
                    <option value="">{t("mindmap.pickProvider")}</option>{providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
                  </select>
                </div>
                <select className={selectClass} value={modelId} onChange={e => setModelId(e.target.value)} disabled={!providerId}>
                  <option value="">{t("mindmap.pickModel")}</option>{(providers.find(p => p.id === providerId)?.models ?? []).map(m => <option key={m.id} value={m.id}>{m.name || m.id}</option>)}
                </select>
                <p className="text-[10px] text-slate-500">{t("mindmap.aiAppendHint")}</p>
                <button type="button" className="w-full rounded-lg py-2 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!projectPath || !providerId || !modelId || aiLoading} onClick={() => void runAiProject()}>{aiLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin inline mr-1" /> : <Sparkles className="h-3.5 w-3.5 inline mr-1" />}{aiLoading ? t("mindmap.aiAnalyzing") : t("mindmap.importProject")}</button>
                <button type="button" className="text-[10px] text-slate-500 hover:text-white" onClick={() => setShowAi(null)}>{t("mindmap.backToCanvas")}</button>
              </div>
            </div>
          ) : showAi === "text" ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 p-6">
              <div className="flex w-full max-w-[500px] flex-col gap-3">
                <div className="flex items-center gap-2"><Lightbulb className="h-5 w-5" style={{ color: ACCENT }} /><span className="text-sm text-white font-semibold">{t("mindmap.aiTextTitle")}</span></div>
                <input className="h-9 w-full rounded-xl bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none" value={textTitle} onChange={e => setTextTitle(e.target.value)} placeholder={t("mindmap.reqTitlePh")} />
                <textarea className="w-full h-40 rounded-xl bg-slate-900 border border-white/10 px-3 py-2 text-xs text-white outline-none resize-none" value={textInput} onChange={e => setTextInput(e.target.value)} placeholder={t("mindmap.reqTextPh")} />
                <select className={selectClass} value={providerId} onChange={e => { const p = providers.find(x => x.id === e.target.value); setProviderId(e.target.value); setModelId(p?.active_model_id ?? p?.models[0]?.id ?? ""); }}>
                  <option value="">{t("mindmap.pickProvider")}</option>{providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
                </select>
                <select className={selectClass} value={modelId} onChange={e => setModelId(e.target.value)} disabled={!providerId}>
                  <option value="">{t("mindmap.pickModel")}</option>{(providers.find(p => p.id === providerId)?.models ?? []).map(m => <option key={m.id} value={m.id}>{m.name || m.id}</option>)}
                </select>
                <p className="text-[10px] text-slate-500">{t("mindmap.aiTextAppendHint")}</p>
                <button type="button" className="w-full rounded-lg py-2 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }}
                  disabled={!textInput.trim() || !providerId || !modelId || aiLoading} onClick={() => void runAiText()}>
                  {aiLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin inline mr-1" /> : <Sparkles className="h-3.5 w-3.5 inline mr-1" />}
                  {aiLoading ? t("mindmap.aiExtracting") : t("mindmap.aiExtract")}
                </button>
                <button type="button" className="text-[10px] text-slate-500 hover:text-white" onClick={() => setShowAi(null)}>{t("mindmap.skipManual")}</button>
              </div>
            </div>
          ) : full ? (
            <Canvas full={full} accent={ACCENT} onDocumentUpdate={onDocumentUpdated} onHistoryPush={commitHistory} historyVersion={historyVersion} onAiProject={() => setShowAi("project")} onAiText={() => setShowAi("text")} onError={setError} onOpenCalendar={openCalendar} focusRequest={calFocus} onFocusHandled={() => setCalFocus(null)} />
          ) : (
            <VexEmptyState
              title={t("mindmap.emptyTitle")}
              desc={t("mindmap.emptyDesc")}
              tick={t("mindmap.emptyTick")}
              avatarSize={56}
              className="h-full"
            />
          )}
          {error && !showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-[11px] text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
        </main>
      </div>
      {showCreate && <CreateDocModal onClose={() => setShowCreate(false)} onCreate={(n,d,fid) => { void createDoc(n,d,fid); }} folderId={activeFolderId} />}
      {error && showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-[11px] text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
      {notice && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
      {confirmState && <ConfirmModal title={confirmState.title} message={confirmState.message} accent={ACCENT} onConfirm={confirmState.action} onClose={() => setConfirmState(null)} />}
      {showCalendar && <PlanCalendarModal onPick={(p) => void openPlannedNode(p)} onClose={() => setShowCalendar(false)} onAddPlan={(ymd) => void addPlanNode(ymd)} onMoveOccurrence={movePlanOccurrence} />}
      {aiReport && <AiImportReportModal result={aiReport} onClose={() => setAiReport(null)} onOpenDoc={(id) => void openReportDoc(id)} />}
      {/* 拖拽浮层：跟随鼠标，提示当前落点是否有效 */}
      {ghost && (
        <div className="pointer-events-none fixed z-[9998] flex items-center gap-1.5 rounded-md border border-cyan-400/50 bg-slate-900/95 px-2 py-1 text-[10px] text-slate-200 shadow-2xl"
          style={{ left: ghost.x + 14, top: ghost.y + 16 }}>
          {ghost.kind === "folder" ? <Folder className="h-3 w-3 shrink-0 text-amber-400" /> : <FileText className="h-3 w-3 shrink-0 text-slate-400" />}
          <span className="max-w-[150px] truncate font-medium">{ghost.name}</span>
          <span className={`${ghost.ok ? "text-cyan-300" : "text-slate-500"}`}>{ghost.ok ? t("mindmap.dropOk") : t("mindmap.dropTarget")}</span>
        </div>
      )}
      {/* Folder create/edit modal */}
      {showFolderCreate && createPortal(
        <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
          <div className="w-[340px] rounded-xl border border-white/10 bg-[#0d1524] p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-4 text-sm font-semibold text-white">{t("mindmap.newFolderTitle")}</h3>
            <input className="w-full h-9 rounded-lg bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none mb-4" value={folderName} onChange={(e) => setFolderName(e.target.value)} placeholder={t("mindmap.folderNamePh")} autoFocus onKeyDown={(e) => e.key === "Enter" && createFolder()} />
            <div className="flex justify-end gap-2">
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] text-slate-400 hover:text-white" onClick={() => setShowFolderCreate(false)}>{t("mindmap.cancel")}</button>
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!folderName.trim()} onClick={createFolder}>{t("mindmap.create")}</button>
            </div>
          </div>
        </div>, document.body)}
      {editingFolder && createPortal(
        <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]">
          <div className="w-[340px] rounded-xl border border-white/10 bg-[#0d1524] p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-4 text-sm font-semibold text-white">{t("mindmap.renameFolderTitle")}</h3>
            <input className="w-full h-9 rounded-lg bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none mb-4" value={folderName} onChange={(e) => setFolderName(e.target.value)} autoFocus onKeyDown={(e) => e.key === "Enter" && updateFolder()} />
            <div className="flex justify-end gap-2">
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] text-slate-400 hover:text-white" onClick={() => setEditingFolder(null)}>{t("mindmap.cancel")}</button>
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!folderName.trim()} onClick={updateFolder}>{t("mindmap.save")}</button>
            </div>
          </div>
        </div>, document.body)}
    </div>
  );
}