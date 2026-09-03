import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
import {
  Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, ReactFlowProvider, getBezierPath, useReactFlow,
  type Connection, type Edge, type EdgeProps, type Node, type NodeChange, type NodeProps, type Viewport,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { MindmapMarkdown } from "./MindmapMarkdown";
import { MarkdownFieldEditor } from "./MarkdownFieldEditor";
import { NodeFormFields } from "./NodeFormFields";
import VexEmptyState from "../VexEmptyState";
import {
  AlertTriangle, BarChart3, Brain, Coins, File, Folder, FolderOpen, LayoutGrid, Loader2, Terminal,
  ScrollText, Sparkles, StickyNote, Image, Trash2, X, Plus, Pencil, Eye,
  ChevronDown, ChevronRight, ChevronLeft, FolderPlus, Search, Maximize2, Minimize2, Code2, FileText, ListTree, RotateCcw, RotateCw, Calendar, Link2, Ban, Square,
} from "lucide-react";
import type { AiConfig } from "../ai/types";
import { AiImportResult, DocumentFull, MindmapDocument, MindmapFolder, MindmapNode, MindmapSticker, PlannedOccurrence, PositionInput, kindColor, mmApi } from "./types";
import { moduleAccent } from "../../utils/theme";
import { VEX_CYBER_CYAN } from "../../utils/brand";
import { createEventBuffer, useEventBufferSnapshot, type EventBuffer } from "../../utils/eventBuffer";
import { SharedModal } from "../shared/Modal";
import { ConfirmDialog } from "../shared/ConfirmDialog";

const ACCENT = moduleAccent();
const MM_LAST_DOC_KEY = "any_version_mindmap_last_doc";

// AI 导入进度事件缓冲（模块级，App 生命周期内常驻）：思维导图面板切走/隐藏后，
// 组件内的 mm-ai-progress 订阅随 Effects 销毁，后端即发即弃的事件会丢；
// 由缓冲在模块作用域统一订阅并保存载荷，面板重新可见时完整重放（长任务不断流）。
const mmAiProgressBuffer: EventBuffer<AiProgressEntry> = createEventBuffer<AiProgressEntry>(
  "mm-ai-progress",
  {
    transform: (p) => ({ ...(p as Omit<AiProgressEntry, "at">), at: Date.now() }),
    limit: 500,
  }
);
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

/** 在资源管理器中定位项目内文件（证据文件 / AI 探索读取的文件共用）。
 *  统一走 launcher_reveal_file：explorer /select 直启动，不经降权代理，
 *  避免被已运行 explorer 误判参数而打开「我的文档」。 */
function openSourceFile(projectRoot: string, src: string) {
  const p = `${projectRoot.replace(/\\/g, "/")}/${src}`;
  void invoke("launcher_reveal_file", { path: p }).catch((e) => console.error("定位文件失败:", p, e));
}

// 节点类别 → 本地化 key（ComfyUI 式端口标签用）
const KIND_KEYS: Record<string, string> = {
  root: "mindmap.kindRoot",
  module: "mindmap.kindModule",
  component: "mindmap.kindComponent",
  service: "mindmap.kindService",
  route: "mindmap.kindRoute",
  config: "mindmap.kindConfig",
  file: "mindmap.kindFile",
  requirement: "mindmap.kindRequirement",
  task: "mindmap.kindTask",
  constraint: "mindmap.kindConstraint",
  risk: "mindmap.kindRisk",
  other: "mindmap.kindOther",
};

/** 缩小到这个缩放比以下时，节点文字全部隐藏（ComfyUI 式缩略）。 */
const ZOOM_HIDE_TEXT = 0.45;

/** 端口标签小胶囊：依附在节点边缘的输入/输出连接点上，带类别色点。 */
function PortChip({ label, color, side }: { label: string; color: string; side: "l" | "r" | "t" | "b" }) {
  const inner = (<>
    <span aria-hidden className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: color }} />
    <span className="whitespace-nowrap leading-none">{label}</span>
  </>);
  const pos = side === "l"
    ? "left-0.5 top-1/2 -translate-y-1/2"
    : side === "r"
      ? "right-0.5 top-1/2 -translate-y-1/2"
      : side === "t"
        ? "top-0.5 left-1/2 -translate-x-1/2 flex-col"
        : "bottom-0.5 left-1/2 -translate-x-1/2 flex-col-reverse";
  return (
    <span className={`pointer-events-none absolute z-[5] inline-flex items-center gap-1 rounded-sm bg-[#0d1524]/90 px-1 py-0.5 text-[8px] text-slate-400 shadow ring-1 ring-white/10 ${pos}`}>
      {inner}
    </span>
  );
}

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

/** 额外连线端口：「谁连进来 / 连向谁」+ 输入端名称 + 端口色 */
type FlowNodeData = { node: MindmapNode; selected: boolean; hasChildren: boolean; collapsed: boolean; hideText: boolean; parentColor: string | null; parentKind: string | null; targetPosition: Position; sourcePosition: Position; onSelect: () => void; onOpenDetail: () => void; onToggle: () => void; onAddChild: () => void; onPreview: (e: React.MouseEvent) => void; onPreviewEnd: () => void; onDelete: () => void; onContextMenu: (e: React.MouseEvent) => void; };

const FlowNode = memo(function FlowNode({ data }: NodeProps<Node<FlowNodeData>>) {
  const { t } = useTranslation();
  const { node, selected, hasChildren, collapsed, hideText, parentColor, parentKind, targetPosition, sourcePosition, onSelect, onOpenDetail, onToggle, onAddChild, onPreview, onPreviewEnd, onDelete, onContextMenu } = data;
  const c = effectiveNodeColor(node);
  const kindL = (k: string | null | undefined) => (k ? t(KIND_KEYS[k] ?? KIND_KEYS.other) : t(KIND_KEYS.other));
  // 大幅缩小 → 隐藏全部文字，只留色条与连接点（ComfyUI 式缩略）。
  if (hideText) {
    return (
      <article className="group relative h-[26px] w-[96px] cursor-pointer rounded-md border shadow-lg transition-shadow"
        style={{ borderColor: `${c}99`, backgroundColor: "#0d1524" }} onClick={onSelect} onDoubleClick={(e) => { e.stopPropagation(); onOpenDetail(); }}
        onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e); }}>
        <Handle id="in" type="target" position={targetPosition} isConnectable className="!h-3 !w-3 !border-2 !border-[#0d1524]" style={{ background: parentColor ?? "#64748b" }} />
        <div className="absolute inset-x-1 bottom-0.5 top-0.5 rounded-sm" style={{ background: `linear-gradient(100deg, ${c}66, ${c}14)` }} />
        <Handle id="out" type="source" position={sourcePosition} isConnectable className="!h-3 !w-3 !border-2 !border-[#0d1524]" style={{ background: c }} />
      </article>
    );
  }

  return (
    <article className={`group relative w-[200px] rounded-xl border shadow-lg transition-shadow cursor-pointer ${selected ? "shadow-cyan-500/30 ring-1 ring-cyan-400/40" : "hover:shadow-xl"}`}
      style={{ borderColor: selected ? c : `${c}55`, backgroundColor: "#0d1524" }} onClick={onSelect} onDoubleClick={(e) => { e.stopPropagation(); onOpenDetail(); }} onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e); }}>
      <Handle id="in" type="target" position={targetPosition} isConnectable className="!h-3 !w-3 !border-2 !border-[#0d1524]" style={{ background: parentColor ?? "#64748b" }} />
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
      {/* 节点右下角悬浮 + 按钮：给当前节点直接添加子节点（避开右侧输出口） */}
      <button type="button" className="nodrag nopan absolute -right-2.5 bottom-1.5 z-10 hidden h-5 w-5 items-center justify-center rounded-full border transition group-hover:flex hover:scale-110"
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
      {/* 默认端口语义标签：贴在节点左右/上下边缘的连接点旁，标出「接入什么 / 向外输出什么」。
          左右布局时给描述区预留空间，避免标签盖住文字。 */}
      {targetPosition === Position.Left && <PortChip label={kindL(parentKind)} color={parentColor ?? "#64748b"} side="l" />}
      {targetPosition === Position.Right && <PortChip label={kindL(parentKind)} color={parentColor ?? "#64748b"} side="r" />}
      {sourcePosition === Position.Left && <PortChip label={kindL(node.kind)} color={c} side="l" />}
      {sourcePosition === Position.Right && <PortChip label={kindL(node.kind)} color={c} side="r" />}
      {targetPosition === Position.Top && <><PortChip label={kindL(parentKind)} color={parentColor ?? "#64748b"} side="t" /><PortChip label={kindL(node.kind)} color={c} side="b" /></>}
      {targetPosition === Position.Bottom && <><PortChip label={kindL(parentKind)} color={parentColor ?? "#64748b"} side="b" /><PortChip label={kindL(node.kind)} color={c} side="t" /></>}
      <div className="flex items-center gap-1.5 px-2.5 pt-1.5">
        {node.planAt && <span className="text-[8px] text-slate-400 font-mono">{t("mindmap.planAt", { time: planShort(node.planAt) })}</span>}
        {(node.sources?.length ?? 0) > 0 && <span className="inline-flex items-center gap-0.5 text-[8px] text-cyan-300/70" title={t("mindmap.evidenceTitle", { count: node.sources!.length, names: node.sources!.join("、") })}><File className="h-2.5 w-2.5" />{node.sources!.length}</span>}
      </div>
      <Handle id="out" type="source" position={sourcePosition} isConnectable className="!h-3 !w-3 !border-2 !border-[#0d1524]" style={{ background: c }} />
    </article>
  );
});

// ════════════ 连线 ════════════

const ColorEdge = memo(function ColorEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, curvature: 0.28 });
  const color = (data?.color as string | undefined) ?? VEX_CYBER_CYAN;
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
    openSourceFile(projectRoot, src);
  };
  const sources = node.sources ?? [];
  // 双击节点进入详情后直接可编辑；预览/分栏交给下方 MarkdownFieldEditor 自带工具栏。
  const [detail, setDetail] = useState(node.detail);
  const [name, setName] = useState(node.name);
  const [planAt, setPlanAt] = useState(node.planAt ?? "");
  const [repeat, setRepeat] = useState(node.repeat || "none");
  const [color, setColor] = useState(node.color);
  const [fullscreen, setFullscreen] = useState(false);
  const c = normalizeHexColor(color) ?? kindColor(node.kind);

  const save = useCallback(() => {
    onUpdate({ name, color, detail, planAt: planAt.trim() ? planAt.trim() : null, repeat });
  }, [name, color, detail, planAt, repeat, onUpdate]);

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
  }, [name, detail]);

  // 弹窗高度随内容自动伸缩（与速记悬浮窗一致）：先以 height:auto 测量各子块
  // 自然高度（标题栏 + 证据 + 表单 + 详情编辑器），再把弹窗高度设为
  // min(自然高度, 视口可用高度)；超出后由表单区内部滚动兜底。
  const cardRef = useRef<HTMLDivElement>(null);
  const [fitH, setFitH] = useState<number | null>(null);
  useEffect(() => {
    if (fullscreen) { setFitH(null); return; }
    const card = cardRef.current;
    if (!card) return;
    let raf = 0;
    const measure = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const prev = card.style.height;
        card.style.height = "auto";
        const natural = card.scrollHeight;
        card.style.height = prev;
        const maxH = Math.max(420, window.innerHeight - 48); // modal-mask p-4 上下各 16px
        const desired = Math.round(Math.min(maxH, Math.max(420, natural)));
        setFitH((h) => (h === desired ? h : desired));
      });
    };
    const ro = new ResizeObserver(measure);
    ro.observe(card);
    for (const k of card.children) ro.observe(k);
    measure();
    return () => { ro.disconnect(); cancelAnimationFrame(raf); };
  }, [fullscreen, sources.length]);

  return createPortal(
    <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]">
      <div ref={cardRef} className={`flex ${fullscreen ? "h-full w-full" : "w-[min(92vw,680px)]"} flex-col overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl`} style={fullscreen ? undefined : { height: fitH ?? "auto" }} onClick={(e) => e.stopPropagation()}>
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
        {/* 表单字段区（与速记悬浮窗一致的压缩布局）：进度/计划时间/重复/颜色单行卡片 */}
        <div className="shrink-0 space-y-2.5 overflow-y-auto p-3" style={{ maxHeight: "42%" }}>
          <NodeFormFields ns="mindmap"
            planAt={planAt} repeat={repeat} color={color}
            onPlanAt={(iso) => { setPlanAt(iso ?? ""); onUpdate({ planAt: iso }); }}
            onRepeat={(v) => { setRepeat(v); onUpdate({ repeat: v }); }}
            onColor={(v) => { setColor(v); onUpdate({ color: v }); }}
          />
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

// ════════════ AI 工作过程日志（实时） ════════════

/** 后端推送的进度事件（mm-ai-progress）：step 标记阶段，其余字段按步骤类型取用 */
interface AiProgressEntry {
  step: "scan" | "explore" | "read" | "route" | "view" | "view_done" | "repair" | "fail" | "usage" | "stream" | "cancel" | string;
  // 视图落库完成（step=view_done）：文档 id，前端据此增量拉取渲染
  // （后端 emit 原样发 doc_id，事件缓冲不做 key 转换，故两种命名都要兼容）
  docId?: string;
  doc_id?: string;
  index?: number;
  round?: number;
  total?: number;
  reason?: string;
  done?: boolean;
  files?: string[];
  views?: string[];
  view?: string;
  count?: number;
  rounds?: number;
  detail?: string;
  // token 用量（step=usage）
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
  model?: string;
  // 流式输出（step=stream）：累计字符数与末尾预览
  length?: number;
  text?: string;
  // 断流重连（step=reconnect）：第几次续写/重发、上限、已收到的字符数、是否携带断点续写；
  // send=true 表示「连接阶段」重试（请求未成功发出/未拿到响应头），否则为断点续写
  attempt?: number;
  max?: number;
  resume?: boolean;
  send?: boolean;
  // 无效点单回执（step=reject）：AI 本轮请求了目录结构里不存在的路径
  paths?: string[];
  // 前端收到时打的时间戳（ms）
  at?: number;
}

/** AI 弹窗最小化后的画布悬浮胶囊：显示后台任务仍在进行（最新进度 + 用时），点击恢复弹窗。 */
function AiRunningPill({ onRestore, onStop }: { onRestore: () => void; onStop: () => void }) {
  const { t } = useTranslation();
  const entries = useEventBufferSnapshot(mmAiProgressBuffer);
  const [now, setNow] = useState(() => Date.now());
  // 秒表：进度事件稀疏（长视图生成中）时用时也要跳动
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  const last = entries[entries.length - 1];
  const elapsed = entries.length ? now - (entries[0].at ?? now) : 0;
  return (
    <div className="absolute right-4 top-4 z-50 flex items-center gap-2 rounded-full border border-cyan-400/25 bg-slate-900/90 px-3 py-1.5 shadow-xl backdrop-blur-sm">
      <Brain className="h-3.5 w-3.5 shrink-0 animate-pulse text-cyan-300" />
      <div className="min-w-0 text-left">
        <div className="text-[10px] font-semibold leading-tight text-white">{t("aiMinimized.running")}</div>
        <div className="max-w-[240px] truncate text-[9px] leading-tight text-slate-400" title={last ? progressText(last, t) : ""}>
          {last ? progressText(last, t) : "…"} · {t("aiMinimized.elapsed")} {fmtDur(elapsed)}
        </div>
      </div>
      <button type="button" onClick={onRestore} title={t("aiMinimized.restore")}
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-slate-300 transition hover:bg-white/10 hover:text-white">
        <Maximize2 className="h-3 w-3" />
      </button>
      <button type="button" onClick={onStop} title={t("mindmap.aiStop")}
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-red-300/80 transition hover:bg-red-400/10 hover:text-red-300">
        <Square className="h-2.5 w-2.5 fill-current" />
      </button>
    </div>
  );
}

const STEP_ICONS: Record<string, React.ReactNode> = {
  scan: <Search className="h-3 w-3 text-cyan-300" />,
  explore: <Brain className="h-3 w-3 text-violet-300" />,
  read: <File className="h-3 w-3 text-emerald-300" />,
  route: <LayoutGrid className="h-3 w-3 text-amber-300" />,
  reconnect: <RotateCcw className="h-3 w-3 text-orange-300" />,
  view: <Sparkles className="h-3 w-3 text-cyan-300" />,
  view_done: <Sparkles className="h-3 w-3 text-emerald-300" />,
  repair: <RotateCcw className="h-3 w-3 text-amber-300" />,
  reject: <AlertTriangle className="h-3 w-3 text-yellow-300" />,
  fail: <AlertTriangle className="h-3 w-3 text-red-300" />,
  usage: <Coins className="h-3 w-3 text-emerald-300" />,
  stream: <Terminal className="h-3 w-3 text-emerald-300" />,
  cancel: <Ban className="h-3 w-3 text-red-300" />,
};

const fmtNum = (n: number) => n.toLocaleString();
const fmtDur = (ms: number) => {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
};
const fmtClock = (ts: number) => {
  const d = new Date(ts);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
};

/** 把结构化进度事件转为一行可读文本（i18n） */
function progressText(e: AiProgressEntry, t: (k: string, o?: any) => string): string {
  switch (e.step) {
    case "scan":
      return e.done ? t("mindmap.aiStepScanDone") : t("mindmap.aiStepScan");
    case "explore":
      return t("mindmap.aiStepExplore", {
        n: e.round,
        total: e.total,
        reason: e.reason || "",
        done: e.done ? ` — ${t("mindmap.aiExploreDone")}` : "",
      });
    case "read":
      return t("mindmap.aiStepReading", { files: (e.files ?? []).join(", ") });
    case "route":
      return e.views
        ? t("mindmap.aiStepViews", { views: e.views.map(v => viewLabel(t, v)).join(", ") })
        : t("mindmap.aiStepRouting");
    case "view":
      return t("mindmap.aiStepView", { view: viewLabel(t, e.view ?? "") });
    case "view_done":
      return t("mindmap.aiStepViewDone", { view: viewLabel(t, e.view ?? "") });
    case "reconnect":
      return e.send
        ? t("mindmap.aiStepReconnectSend", { n: e.attempt ?? 0, max: e.max ?? 0, msg: e.detail ?? "" })
        : t("mindmap.aiStepReconnectResume", { n: e.attempt ?? 0, max: e.max ?? 0, chars: e.length ?? 0 });
    case "repair":
      return t("mindmap.aiStepRepair", { count: e.count ?? 0, rounds: e.rounds ?? 0 });
    case "reject":
      return t("mindmap.aiStepReject", { paths: (e.paths ?? []).join(", ") });
    case "fail":
      return t("mindmap.aiStepFail", { msg: e.detail ?? "" });
    case "usage":
      return t("mindmap.aiStepUsage", { model: e.model ?? "" });
    case "stream":
      return e.done ? t("mindmap.aiStepStreamDone", { n: e.length ?? 0 }) : t("mindmap.aiStepStream");
    case "cancel":
      return t("mindmap.aiCancelled");
    default:
      return e.detail ?? e.step;
  }
}

/** IDE 式实时过程面板：AI 导入期间的每一步（扫描、每轮探索、读取的文件、视图生成、
 *  修复轮次、每次请求的 token 用量）+ 实时累计统计（请求数/输入/输出/总 token/用时）。
 *  后端通过 Tauri 事件 "mm-ai-progress" 推送结构化步骤，token 事件来自响应里的 usage 字段。
 *  事件由模块级缓冲（mmAiProgressBuffer）统一接收：面板切走再切回时历史完整重放，
 *  不会因隐藏期间订阅失效而丢失任何进度。 */
function AiProgressLog({ projectRoot, onStop }: { projectRoot?: string | null; onStop?: () => void }) {
  const { t } = useTranslation();
  const entries = useEventBufferSnapshot(mmAiProgressBuffer);
  const [totals, setTotals] = useState({ req: 0, in: 0, out: 0, total: 0 });
  const [model, setModel] = useState("");
  const [streamInfo, setStreamInfo] = useState<{ length: number; text: string } | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const boxRef = useRef<HTMLDivElement | null>(null);

  // 从缓冲重放：初始化 totals/model/stream/cancelled 状态（组件挂载或切回时跑一次，
  // 覆盖隐藏期间到达的事件，保证统计与最终状态与实际任务进度一致）。
  // 依赖 buffer.version：缓冲内容变化（含 clear）时重新演算一遍。
  const replayRef = useRef(-1);
  useEffect(() => {
    if (replayRef.current === mmAiProgressBuffer.version()) return;
    replayRef.current = mmAiProgressBuffer.version();
    const list = mmAiProgressBuffer.snapshot();
    setTotals({ req: 0, in: 0, out: 0, total: 0 });
    setModel("");
    setCancelled(false);
    setStreamInfo(null);
    for (const p of list) {
      if (p.step === "cancel") {
        setCancelled(true);
        setStreamInfo(null);
        continue;
      }
      if (p.step === "stream") {
        if (p.done) {
          setStreamInfo(null);
          continue;
        }
        setStreamInfo({ length: p.length ?? 0, text: p.text ?? "" });
        continue;
      }
      setStreamInfo(null);
      if (p.step === "usage") {
        const inT = p.prompt_tokens ?? 0;
        const outT = p.completion_tokens ?? 0;
        setTotals(prev => ({ req: prev.req + 1, in: prev.in + inT, out: prev.out + outT, total: prev.total + (p.total_tokens ?? inT + outT) }));
        if (p.model) setModel(p.model);
      }
    }
  }, [entries]);

  // 用时秒表：每秒刷新当前时间，驱动总耗时跳动
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    if (boxRef.current) boxRef.current.scrollTop = boxRef.current.scrollHeight;
  }, [entries]);

  // 时间线视图：把缓冲里的节流 stream 帧折叠为「开始 + 完成」两行（与旧组件内订阅
  // 行为一致），其余步骤原样保留；中间帧只用于实时预览区（streamInfo）。
  const timeline = useMemo(() => {
    const out: AiProgressEntry[] = [];
    for (const p of entries) {
      if (p.step === "stream" && !p.done) {
        const l = out[out.length - 1];
        if (l && l.step === "stream" && !l.done) continue;
      }
      out.push(p);
    }
    return out;
  }, [entries]);

  const last = timeline[timeline.length - 1];
  const startedAt = timeline[0]?.at ?? now;
  const elapsed = timeline.length ? now - startedAt : 0;

  return (
    <div className="overflow-hidden rounded-lg border border-white/10 bg-slate-950/90" title={t("mindmap.aiProgressLog")}>
      {/* 头部：标题 + 模型 + 实时统计 */}
      <div className="flex items-center gap-2 border-b border-white/10 px-2.5 py-1.5">
        <span className="inline-flex shrink-0 items-center gap-1 text-[9px] font-semibold uppercase tracking-wide text-slate-400">
          <Brain className="h-3 w-3 animate-pulse text-cyan-300" />{t("mindmap.aiProgressTitle")}
        </span>
        {model && <span className="min-w-0 truncate rounded bg-white/[0.06] px-1.5 py-px font-mono text-[8px] text-slate-400" title={model}>{model}</span>}
        <span className="ml-auto flex shrink-0 items-center gap-2 font-mono text-[8px] tabular-nums text-slate-400">
          <span>{t("mindmap.aiStatRequests", { count: totals.req })}</span>
          <span className="text-emerald-300/90">↑{fmtNum(totals.in)}</span>
          <span className="text-cyan-300/90">↓{fmtNum(totals.out)}</span>
          <span className="text-slate-200">{fmtNum(totals.total)}</span>
          <span className="text-slate-500">{t("mindmap.aiStatElapsed", { s: fmtDur(elapsed) })}</span>
        </span>
        {onStop && (
          <button type="button" onClick={onStop} disabled={cancelled}
            className="inline-flex shrink-0 cursor-pointer items-center gap-1 rounded border border-red-400/40 bg-red-500/10 px-2 py-0.5 text-[8px] font-semibold text-red-300 transition hover:border-red-400/80 hover:bg-red-500/25 hover:text-red-200 disabled:cursor-default disabled:opacity-40"
            title={t("mindmap.aiStopTitle")}>
            <Square className="h-2.5 w-2.5 fill-current" />{cancelled ? t("mindmap.aiStopped") : t("mindmap.aiStop")}
          </button>
        )}
      </div>
      {/* 当前活动行：流式输出时实时显示 AI 正在写的内容，否则显示最后一步 + 闪烁光标；
          取消后定格在「已取消」状态，不再闪烁 */}
      <div className="flex items-center gap-1.5 border-b border-white/5 px-2.5 py-1 text-[9px] text-slate-300">
        {cancelled ? (<>
          <span className="shrink-0"><Ban className="h-3 w-3 text-red-300" /></span>
          <span className="min-w-0 flex-1 truncate font-semibold text-red-300">{t("mindmap.aiCancelled")}</span>
        </>) : streamInfo ? (<>
          <span className="shrink-0">{STEP_ICONS.stream}</span>
          <span className="min-w-0 flex-1 truncate font-mono text-emerald-300/90">
            <span className="text-slate-500">⏵</span> {streamInfo.text || t("mindmap.aiStepStream")}
            <span className="animate-pulse text-emerald-300">▍</span>
          </span>
          <span className="shrink-0 font-mono text-[8px] tabular-nums text-slate-500">{fmtNum(streamInfo.length)}</span>
        </>) : last ? (<>
          <span className="shrink-0">{STEP_ICONS[last.step] ?? <Sparkles className="h-3 w-3 text-slate-400" />}</span>
          <span className="min-w-0 flex-1 truncate">{progressText(last, t)}</span>
          <span className="animate-pulse text-cyan-300">▍</span>
        </>) : (
          <span className="text-slate-600">{t("mindmap.aiStepScan")}</span>
        )}
      </div>
      {/* 时间线：时间戳 + 步骤图标 + 文本 + 每步耗时 */}
      <div ref={boxRef} className="max-h-44 space-y-0.5 overflow-y-auto px-2.5 py-1.5 font-mono text-[9px] leading-relaxed">
        {timeline.map((e, i) => {
          const prev = timeline[i - 1];
          const dt = prev && e.at !== undefined ? e.at - (prev.at ?? e.at) : 0;
          return (
            <div key={i} className="flex items-baseline gap-1.5">
              <span className="shrink-0 text-slate-600 tabular-nums">{fmtClock(e.at ?? Date.now())}</span>
              <span className="mt-0.5 shrink-0 self-center">{STEP_ICONS[e.step] ?? <Sparkles className="h-3 w-3 text-slate-400" />}</span>
              <span className={`min-w-0 flex-1 break-words ${e.step === "fail" || e.step === "cancel" ? "text-red-300" : e.step === "usage" ? "text-slate-400" : "text-slate-300"}`}>
                {e.step === "usage" ? (
                  <span className="inline-flex items-center gap-1">
                    <span className="text-emerald-300/90">+{fmtNum(e.prompt_tokens ?? 0)}</span>
                    <span className="text-cyan-300/90">−{fmtNum(e.completion_tokens ?? 0)}</span>
                    <span className="text-slate-500">＝{fmtNum(e.total_tokens ?? 0)}</span>
                  </span>
                ) : e.step === "read" && projectRoot && (e.files?.length ?? 0) > 0 ? (
                  // 探索阶段读取的文件：可点击，直接定位到源码（复用证据文件的定位能力）
                  <span className="inline-flex min-w-0 flex-wrap items-center gap-1">
                    <span className="text-slate-500">{t("mindmap.aiStepReadFiles", { count: (e.files ?? []).length })}</span>
                    {(e.files ?? []).map((f, j) => (
                      <button key={j} type="button"
                        onClick={(ev) => { ev.stopPropagation(); openSourceFile(projectRoot, f); }}
                        className="inline-flex max-w-[190px] cursor-pointer items-center gap-0.5 truncate rounded border border-cyan-400/25 bg-cyan-400/[0.07] px-1 py-px font-normal text-cyan-200 transition hover:border-cyan-400/60 hover:bg-cyan-400/15"
                        title={f}>
                        <File className="h-2.5 w-2.5 shrink-0" />
                        <span className="truncate">{f}</span>
                      </button>
                    ))}
                  </span>
                ) : progressText(e, t)}
              </span>
              <span className="shrink-0 text-slate-600 tabular-nums">{dt > 0 ? fmtDur(dt) : ""}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
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
  const [expOpen, setExpOpen] = useState(false);
  const allOk = result.reports.length > 0 && result.reports.every(r => r.diagnostics.length === 0) && result.failures.length === 0;
  return createPortal(
    <div className="fixed inset-0 z-[210] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]" onClick={onClose}>
      <div className="w-[min(94vw,560px)] rounded-xl border border-white/10 bg-[#0d1524] p-4 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-white"><Sparkles className="h-4 w-4 text-cyan-400" />{t("mindmap.aiReportTitle")}</h3>
          <button type="button" className="cursor-pointer rounded p-1 text-slate-400 hover:text-white" onClick={onClose} title={t("mindmap.close")}><X className="h-4 w-4" /></button>
        </div>
        {/* 本次运行总消耗（含路由/探索阶段，来自后端累计） */}
        {result.usage && (result.usage.requests > 0 || result.usage.totalTokens > 0) && (
          <div className="mb-2 flex items-center gap-2 rounded-lg border border-white/10 bg-white/[0.03] px-3 py-1.5 text-[9px]">
            <BarChart3 className="h-3 w-3 shrink-0 text-emerald-300" />
            <span className="shrink-0 font-semibold text-slate-300">{t("mindmap.aiRunUsage")}</span>
            <span className="font-mono tabular-nums text-slate-400">{t("mindmap.aiRunUsageLine", { req: result.usage.requests, in: fmtNum(result.usage.inputTokens), out: fmtNum(result.usage.outputTokens), total: fmtNum(result.usage.totalTokens) })}</span>
          </div>
        )}
        {/* 项目探索过程：每轮 AI 点单的理由 + 实际读取的文件清单（可折叠） */}
        {result.exploration && result.exploration.length > 0 && (() => {
          const expFiles = result.exploration.reduce((n, r) => n + r.files.length, 0);
          return (
            <div className="mb-2 rounded-lg border border-white/10 bg-white/[0.03]">
              <button type="button"
                onClick={() => setExpOpen(v => !v)}
                className="flex w-full cursor-pointer items-center justify-between gap-2 px-3 py-1.5 text-left">
                <span className="flex items-center gap-1.5 text-[10px] font-semibold text-slate-300">
                  <Search className="h-3 w-3 text-cyan-300" />
                  {t("mindmap.aiExploreTitle", { rounds: result.exploration.length, files: expFiles })}
                </span>
                {expOpen ? <ChevronDown className="h-3 w-3 shrink-0 text-slate-400" /> : <ChevronRight className="h-3 w-3 shrink-0 text-slate-400" />}
              </button>
              {expOpen && (
                <div className="space-y-1.5 border-t border-white/5 px-3 py-2">
                  {result.exploration.map(r => (
                    <div key={r.round}>
                      <div className="text-[9px] text-slate-400">
                        <span className="mr-1 rounded border border-cyan-400/30 bg-cyan-400/10 px-1 py-px font-mono text-cyan-300">#{r.round}</span>
                        {r.reason || t("mindmap.aiExploreNoReason")}
                      </div>
                      {(r.files.length > 0 || r.dirs.length > 0) && (
                        <div className="mt-0.5 flex flex-wrap gap-1">
                          {r.files.map(f => (
                            <span key={f} className="rounded bg-white/[0.05] px-1 py-px font-mono text-[8px] text-slate-400" title={f}>{f}</span>
                          ))}
                          {r.dirs.map(d => (
                            <span key={d} className="rounded bg-white/[0.05] px-1 py-px font-mono text-[8px] text-slate-500" title={d}>{d}/</span>
                          ))}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })()}
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
                  {(r.usage?.requests ?? 0) > 0 && (
                    <span className="ml-1 font-mono text-emerald-300/80" title={t("mindmap.aiRunUsage")}>⚡ {fmtNum(r.usage?.totalTokens ?? 0)} tok</span>
                  )}
                  {(doc?.document.aiImports ?? 0) > 0 && (
                    <span className="ml-1 text-slate-600">{t("mindmap.aiDocUsage", { imports: doc!.document.aiImports, tokens: fmtNum((doc!.document.aiInputTokens ?? 0) + (doc!.document.aiOutputTokens ?? 0)) })}</span>
                  )}
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
  hideText: boolean;
  parentColor: string | null;
  parentKind: string | null;
  obj: Node;
};

function CanvasInner({ full, accent, onDocumentUpdate, onHistoryPush, historyVersion, onAiProject, onAiText, onError, onOpenCalendar, focusRequest, onFocusHandled }: { full: DocumentFull; accent: string; onDocumentUpdate: (d: DocumentFull) => void; onHistoryPush: () => void; historyVersion: number; onAiProject: () => void; onAiText: () => void; onError: (message: string) => void; onOpenCalendar: () => void; focusRequest: { nodeId: string; ts: number } | null; onFocusHandled: () => void }) {
  const { t } = useTranslation();
  const { fitView } = useReactFlow();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // 节点树导航面板：点击树节点 = 选中 + 展开祖先 + 视口聚焦（与悬浮窗树形选择同一交互直觉）
  const [treeOpen, setTreeOpen] = useState(true);
  const treeListRef = useRef<HTMLDivElement | null>(null);
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
  // 视口缩放：低于阈值时节点文字整体隐藏（ComfyUI 式缩略）。
  // 只在跨阈值瞬间 setState，避免 onMove 每帧触发整树重染。
  const zoomHideRef = useRef(false);
  const [zoomHide, setZoomHide] = useState(false);
  const onViewportMove = useCallback((_e: unknown, vp: Viewport) => {
    const hide = vp.zoom < ZOOM_HIDE_TEXT;
    if (hide !== zoomHideRef.current) { zoomHideRef.current = hide; setZoomHide(hide); }
  }, []);
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

  // ══════ 节点树导航（右侧面板） ══════
  // 与悬浮窗「选择归属」同一套树形展开算法：根级起深度缩进，当前选中高亮。
  const navTree = useMemo(() => {
    const out: { node: MindmapNode; depth: number }[] = [];
    const visited = new Set<string>();
    const visit = (n: MindmapNode, depth: number) => {
      if (visited.has(n.id)) return;
      visited.add(n.id);
      out.push({ node: n, depth });
      if (!collapsed.has(n.id)) graphNodes.filter(c => c.parentId === n.id && c.parentId !== c.id && !visited.has(c.id)).forEach(c => visit(c, depth + 1));
    };
    graphNodes.filter(n => !n.parentId || !byId.has(n.parentId) || n.parentId === n.id).forEach(n => visit(n, 0));
    graphNodes.forEach(n => { if (!visited.has(n.id)) visit(n, 0); }); // 环/孤立节点兑底
    return out;
  }, [graphNodes, collapsed, byId]);

  // 点击树节点：展开其全部祖先（折叠集合移除）→ 选中 → 视口聚焦到该节点。
  // 注意：不能把聚焦定时器的 cleanup 返回给 onClick 立即调用——那会把 fitView 取消掉
  // （点击树节点选中但不移动视口的 bug）。聚焦用 60ms 延时，与日历 focusRequest 同参数。
  const navToNode = useCallback((id: string) => {
    setCollapsed(prev => {
      let changed = false;
      const next = new Set(prev);
      let cursor = byId.get(id)?.parentId ?? null;
      while (cursor && byId.has(cursor)) {
        if (next.delete(cursor)) changed = true;
        cursor = byId.get(cursor)?.parentId ?? null;
      }
      return changed ? next : prev;
    });
    setSelectedId(id);
    // 等展开生效（节点出现在 visibleNodes/flowNodes 并被 RF 测量）后再聚焦
    window.setTimeout(() => fitView({ nodes: [{ id }], padding: 0.4, duration: 300, maxZoom: 1.2 }), 60);
  }, [byId, fitView]);

  // 当前选中节点变化（画布点选/键盘导航）→ 树面板滚动到对应行，保持两侧行为同步。
  const selectedTreeIdx = navTree.findIndex(x => x.node.id === selectedId);
  useEffect(() => {
    if (selectedTreeIdx < 0 || !treeOpen) return;
    const list = treeListRef.current;
    if (!list) return;
    const row = list.querySelector<HTMLElement>(`[data-tree-idx="${selectedTreeIdx}"]`);
    row?.scrollIntoView({ block: "nearest" });
  }, [selectedTreeIdx, treeOpen]);

  const addNode = useCallback((parentId: string | null) => {
    onHistoryPush();
    const now = new Date().toISOString();
    // 新增子节点继承父节点的可见颜色（父节点未手动配色时取类型默认色），保持树视觉连续
    const parent = parentId ? byId.get(parentId) : null;
    const n: MindmapNode = { id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, documentId: full.document.id, parentId, name: parentId ? t("mindmap.newNode") : t("mindmap.newRoot"), detail: "", kind: parentId ? "other" : "root", color: parent ? effectiveNodeColor(parent) : "", planAt: null, repeat: "none", positionX: 0, positionY: 0, createdAt: now, updatedAt: now };
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
    // 单父约束：一个节点只允许一个入口（树形结构）。目标已有父节点时忽略本次连线。
    const child = byId.get(target)!;
    if (child.parentId) return;
    // 不能连到自己的后代，否则成环。
    let cursor: string | null = source;
    const seen = new Set<string>();
    while (cursor && !seen.has(cursor)) {
      if (cursor === target) return;
      seen.add(cursor);
      cursor = byId.get(cursor)?.parentId ?? null;
    }
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
      // ComfyUI 式连接点：输入口取父节点类别色/类别名，输出口取本节点类别色/类别名
      const parent = n.parentId ? byId.get(n.parentId) : undefined;
      const parentColor = parent ? effectiveNodeColor(parent) : null;
      const parentKind = parent ? parent.kind : null;
      const prev = cache.get(n.id);
      if (prev && prev.full === full && prev.node === n && prev.px === p.x && prev.py === p.y && prev.selected === selected && prev.hasChildren === hasChildren && prev.collapsed === isCollapsed && prev.hideText === zoomHide && prev.parentColor === parentColor && prev.parentKind === parentKind) {
        main.push(prev.obj as Node<FlowNodeData>);
        continue;
      }
      // data 仅在节点内容/状态变化时重建；纯位置变化（拖动中）复用旧 data 引用，
      // 避免 memo(FlowNode) 因 data 每帧新引用而重渲染整个节点子树（WebView2 下闪烁）。
      // 注意：data 里的回调（onAddChild/onOpenDetail/onDelete…）闭包捕获了 full/selectedId 等状态，
      // 因此 full 一旦变化就必须重建 data，否则复用旧 data 会让回调闭包停留在旧状态——
      // 例如连续点「+」加子节点时，第 3 次会用第 1 次渲染时的旧 full，把第 2 个子节点覆盖掉。
      const prevData = prev ? (prev.obj as Node<FlowNodeData>).data : null;
      const dataChanged = !prevData || prev!.full !== full || prev!.node !== n || prev!.selected !== selected || prev!.hasChildren !== hasChildren || prev!.collapsed !== isCollapsed || prevData.targetPosition !== endpointPositions.target || prevData.sourcePosition !== endpointPositions.source || prevData.hideText !== zoomHide || prevData.parentColor !== parentColor || prevData.parentKind !== parentKind;
      const data: FlowNodeData = dataChanged          ? { node: n, selected, hasChildren, collapsed: isCollapsed, hideText: zoomHide, parentColor, parentKind, targetPosition: endpointPositions.target, sourcePosition: endpointPositions.source,
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
      cache.set(n.id, { node: n, full, px: p.x, py: p.y, selected, hasChildren, collapsed: isCollapsed, hideText: zoomHide, parentColor, parentKind, obj });
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
      cache.set(sid, { node: s, full, px: p.x, py: p.y, selected: false, hasChildren: false, collapsed: false, hideText: false, parentColor: null, parentKind: null, obj });
      stickerNodes.push(obj);
    }
    // 清理不再显示的缓存项
    for (const k of cache.keys()) if (!alive.has(k)) cache.delete(k);
    return [...main, ...stickerNodes];
  }, [visibleNodes, layout, selectedId, collapsed, childrenCount, stickers, full, onDocumentUpdate, highlightChain, posOverrides, measuredMap, addChildNode, deleteNode, openDetail, endpointPositions, zoomHide, byId]);

  const edges = useMemo<Edge[]>(() => {
    const visible = new Set(visibleNodes.map(n => n.id));
    const out: Edge[] = [];
    // 树形主连线（实线，走主输入/输出口）
    for (const n of visibleNodes) {
      if (!n.parentId || !visible.has(n.parentId)) continue;
      const isOnChain = highlightChain.includes(n.id) && highlightChain.includes(n.parentId);
      out.push({ id: `mm-e-${n.id}`, source: n.parentId, target: n.id, sourceHandle: "out", targetHandle: "in", type: "colorE", style: isOnChain ? { strokeWidth: 2, opacity: 0.9 } : {},
        data: { color: effectiveNodeColor(n) }, markerEnd: { type: MarkerType.ArrowClosed, color: "#f8fafc" } } as Edge);
    }
    return out;
  }, [visibleNodes, byId, highlightChain]);

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
    <div className="relative h-full min-h-0">
      <ReactFlow nodes={flowNodes} edges={edges} nodeTypes={mmNodeTypes} edgeTypes={mmEdgeTypes}
        onNodesChange={onNodesChange} onNodeDragStop={onNodeDragStop} onConnect={onConnect} onMove={onViewportMove}
        onNodeContextMenu={(e, n) => onNodeContextMenu(e, n as Node)} minZoom={0.1} maxZoom={2.5} nodesConnectable
        proOptions={{ hideAttribution: true }}>
        <MiniMap style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95"
          nodeColor={(n) => { const d = n.data as FlowNodeData | StickerNodeData; return 'node' in d ? effectiveNodeColor(d.node) : "#fef3c7"; }}
          nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2,6,23,0.72)" pannable zoomable />
        <Controls className="canvas-flow-controls" showInteractive={false} />
      </ReactFlow>

      {/* 连线规则提示：拖拽连线 = 设置父节点；一个节点只能有一个父节点 */}
      <div className="pointer-events-none absolute bottom-2 left-1/2 z-10 -translate-x-1/2">
        <span className="inline-flex items-center gap-1 whitespace-nowrap rounded-full border border-white/10 bg-slate-900/80 px-2.5 py-1 text-[9px] text-slate-400 shadow backdrop-blur">
          <Link2 className="h-2.5 w-2.5 text-cyan-300/80" />{t("mindmap.connectHint")}
        </span>
      </div>

      {/* 节点树导航（右侧）：与悬浮窗树形选择同交互 —— 点击即导航到该节点 */}
      <div className="absolute right-4 top-4 z-20 flex flex-col items-end" style={{ maxWidth: "46%" }}>
        {!treeOpen && (
          <button type="button" className="inline-flex items-center gap-1.5 rounded-lg border border-white/10 bg-slate-900/95 px-2 py-1.5 text-[10px] text-slate-300 shadow-lg hover:bg-white/[0.08] hover:text-white"
            onClick={() => setTreeOpen(true)} title={t("mindmap.treeNavShow")}>
            <ListTree className="h-3 w-3" />{t("mindmap.treeNavShow")}
          </button>
        )}
        {treeOpen && (
          <div className="flex max-h-full w-52 flex-col rounded-lg border border-white/10 bg-[#0d1524]/95 shadow-2xl shadow-black/40 backdrop-blur">
            <div className="flex items-center gap-1.5 border-b border-white/10 px-2 py-1.5">
              <ListTree className="h-3 w-3 shrink-0 text-slate-500" />
              <span className="min-w-0 flex-1 truncate text-[10px] font-semibold uppercase tracking-wide text-slate-400">{t("mindmap.treeNavTitle")}</span>
              <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-white" onClick={() => setTreeOpen(false)} title={t("mindmap.treeNavHide")}>
                <ChevronRight className="h-3 w-3" />
              </button>
            </div>
            <div ref={treeListRef} className="max-h-[52vh] min-h-0 overflow-y-auto py-0.5">
              {navTree.map((x, i) => {
                const on = selectedId === x.node.id;
                const rowColor = effectiveNodeColor(x.node);
                return (
                  <div key={x.node.id} data-tree-idx={i} role="button" tabIndex={-1}
                    className={`group/row flex w-full items-center gap-1 py-1 pr-1.5 text-left text-[10px] transition hover:bg-white/[0.06] ${on ? "text-white" : "text-slate-400"}`}
                    style={{ paddingLeft: 8 + x.depth * 12 }}>
                    {/* 主体：点击 = 导航（展开祖先 + 选中 + 视口聚焦） */}
                    <button type="button"
                      className={`flex min-w-0 flex-1 items-center gap-1 text-left ${on ? "text-white" : "text-slate-400"}`}
                      onClick={() => navToNode(x.node.id)}
                      title={x.node.name}>
                      {x.depth > 0 && <span className="shrink-0 text-slate-700">└</span>}
                      <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: rowColor, boxShadow: on ? `0 0 5px ${rowColor}` : undefined }} />
                      <span className={`min-w-0 flex-1 truncate ${on ? "font-semibold" : ""}`}
                        style={on ? { color: rowColor } : undefined}>{x.node.name}</span>
                      {collapsed.has(x.node.id) && childrenCount.get(x.node.id) ? (
                        <span className="shrink-0 rounded bg-slate-800 px-1 text-[8px] text-slate-500">{childrenCount.get(x.node.id)}</span>
                      ) : null}
                    </button>
                    {/* 行尾悬浮操作：预览详细内容 / 打开编辑弹框（与日历定位行为一致） */}
                    <span className="hidden shrink-0 items-center gap-0.5 group-hover/row:flex">
                      <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-cyan-300"
                        onMouseEnter={(e) => { if (previewCloseTimer.current) { window.clearTimeout(previewCloseTimer.current); previewCloseTimer.current = null; } setPreview({ node: x.node, x: e.clientX, y: e.clientY }); }}
                        onMouseMove={(e) => { setPreview(prev => (prev && prev.node.id === x.node.id) ? prev : { node: x.node, x: e.clientX, y: e.clientY }); }}
                        onMouseLeave={() => { previewCloseTimer.current = window.setTimeout(() => { if (!previewHoverRef.current) setPreview(null); }, 250); }}
                        onClick={(e) => e.stopPropagation()}
                        title={t("mindmap.previewOnly")}>
                        <Eye className="h-3 w-3" />
                      </button>
                      <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-white"
                        onClick={() => { navToNode(x.node.id); const n = byId.get(x.node.id); if (n) openDetail(n); }}
                        title={t("mindmap.viewDetail")}>
                        <Pencil className="h-3 w-3" />
                      </button>
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
      {/* Compact floating toolbar */}
      <div className="absolute right-4 top-4 z-10 flex flex-col gap-1">
        <div className="rounded-lg border border-white/10 bg-slate-900/95 p-1 shadow-lg flex flex-col gap-0.5">
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={relayout} title={t("mindmap.autoLayout")}><LayoutGrid className="h-3 w-3" />{t("mindmap.layout")}</button>
          <select className="w-full rounded border border-white/10 bg-slate-900/95 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-400/60" value={dir} onChange={(e) => changeDir(e.target.value as LayoutDir)} title={t("mindmap.layoutDir")}>
            {Object.entries(LAYOUT_DIR_KEYS).map(([k, v]) => <option key={k} value={k}>{t(v)}</option>)}
          </select>
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
                  onClick={() => { void invoke("launcher_reveal_file", { path: p }).catch((e) => console.error("定位文件失败:", p, e)); setCtxMenu(null); }}><Folder className="h-3.5 w-3.5" />{t("mindmap.openFolder")}</button>
              ); })()}
              {srcs.map((s, i) => (
                <button key={i} type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
                  onClick={() => {
                    if (root) openSourceFile(root, s);
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
                <MindmapMarkdown content={preview.node.detail || t("mindmap.missingView")} />
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
  const [aiProjectHint, setAiProjectHint] = useState("");
  // AI 项目导入产物选项：深度（1 最浅清单 → 5 最深业务流）+ 视图开关（空 = AI 自动判断）
  const [aiDepth, setAiDepth] = useState(3);
  const [aiViews, setAiViews] = useState<string[]>([]);
  const [aiLoading, setAiLoading] = useState(false);
  const aiRunIdRef = useRef<string | null>(null); // 当前 AI 导入运行的取消标识
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
  // AI 弹框关闭拦截：任务进行中先询问（确认后停止任务再关闭）
  const [aiCloseGuard, setAiCloseGuard] = useState(false);
  // AI 弹框最小化：任务后台继续跑，画布上只留一个可恢复的悬浮图标。
  // 关闭规则不变（仍只能经 requestCloseAi），最小化只是把弹框从视口挪开。
  const [aiMinimized, setAiMinimized] = useState(false);
  // 订阅模块级 AI 进度缓冲：驱动「边生成边绘制」effect 在每个 view_done 到达时触发
  const aiProgress = useEventBufferSnapshot(mmAiProgressBuffer);

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
    // AI 供应商/模型预填：优先全局默认 AI 模型（全局设置中选择，与翻译/划词翻译共享），
    // 失效或未设置时回退第一个可用供应商（与后端 resolve 的回退规则一致）。
    void Promise.all([
      invoke<AiConfig>("get_ai_config"),
      invoke<{ providerId: string | null; modelId: string | null }>("get_translate_config").catch(() => ({ providerId: null, modelId: null })),
    ]).then(([cfg, gDef]) => {
      setConfig(cfg);
      const usable = cfg.providers.filter(x => x.api_key && x.openai_url);
      const p = (gDef.providerId && cfg.providers.find(x => x.id === gDef.providerId && x.api_key && x.openai_url))
        ?? usable[0]
        ?? cfg.providers[0];
      if (!p) return;
      setProviderId(p.id);
      // 模型：全局默认模型（校验仍属于该供应商）> 供应商激活模型 > 第一个模型
      const mid = (gDef.modelId && p.models.some(m => m.id === gDef.modelId) ? gDef.modelId : null)
        ?? p.active_model_id ?? p.models[0]?.id ?? "";
      setModelId(mid);
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
    const n: MindmapNode = { id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, documentId: docId, parentId: null, name: t("mindmap.newPlan"), detail: "", kind: "task", color: "", planAt: planIso, repeat: "none", positionX: 0, positionY: 0, createdAt: nowIso, updatedAt: nowIso };
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

  // 应用 AI 类型路由导入结果：切到主文档，把新生成的视图文档并入侧栏列表。
  // （运行期间已通过 view_done 事件增量绘制过视图：这里避免把当前画布切回旧文档，
  //   只补齐尚未加载的新文档到侧栏。）
  const applyAiImport = useCallback((r: AiImportResult) => {
    setDocs(prev => {
      const existing = new Set(prev.map(d => d.id));
      const fresh = r.documents.map(d => d.document).filter(d => !existing.has(d.id));
      return [...fresh, ...prev];
    });
    const names = r.documents.map(d => d.document.name).join("、");
    const failTxt = r.failures.length ? t("mindmap.viewFailures", { count: r.failures.length, names: r.failures.map(f => f.view).join("、") }) : "";
    flash(t("mindmap.viewsGenerated", { count: r.documents.length, names: names ? `：${names}` : "", failures: failTxt }));
    setShowAi(null);
    setAiMinimized(false);
    setAiReport(r);
    void refreshFolders();
  }, [flash, refreshFolders]);

  // ── 边生成边绘制 ──
  // 后端每落库一个视图文档就广播 view_done；这里增量拉取并渲染：
  // 首个完成视图直接切到该文档（用户立刻在画布上看到成果），
  // 后续视图并入侧栏列表（不打断用户正在看的文档，仅 toast 提示，点击即跳转）。
  const drawnDocIdsRef = useRef<Set<string>>(new Set());
  const firstViewDrawnRef = useRef(false);
  useEffect(() => {
    if (!aiLoading) return;
    const done = mmAiProgressBuffer.snapshot().filter(e => e.step === "view_done");
    for (const e of done) {
      const docId = e.docId ?? e.doc_id;
      if (!docId || drawnDocIdsRef.current.has(docId)) continue;
      drawnDocIdsRef.current.add(docId);
      void (async () => {
        try {
          const f = await mmApi.load(docId);
          if (!f) return;
          setDocs(prev => (prev.some(d => d.id === docId) ? prev : [f.document, ...prev]));
          if (!firstViewDrawnRef.current) {
            firstViewDrawnRef.current = true;
            setFull(f);
          }
        } catch { /* 拉取失败不致命：最终结果仍会整体应用 */ }
      })();
    }
  }, [aiLoading, aiProgress]);

  // 新一轮导入开始：重置增量绘制标记
  useEffect(() => {
    if (aiLoading) { drawnDocIdsRef.current = new Set(); firstViewDrawnRef.current = false; }
  }, [aiLoading]);

  const runAiProject = useCallback(async () => {
    if (!full || !projectPath || !providerId || !modelId) return;
    const runId = crypto.randomUUID ? crypto.randomUUID() : `run-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    aiRunIdRef.current = runId;
    mmAiProgressBuffer.clear(); // 新一轮导入：清空进度缓冲，日志面板从零开始
    setAiLoading(true); setError("");
    try {
      const r = await mmApi.aiFromProject({ documentId: full.document.id, projectPath, providerId: providerId || null, modelId: modelId || null, userHint: aiProjectHint.trim() || null, depth: aiDepth, views: aiViews, runId });
      applyAiImport(r);
    } catch (e) {
      const msg = String(e);
      // 用户主动停止：不当作错误红字提示，仅日志面板展示
      if (msg.includes("已取消")) flash(t("mindmap.aiCancelled")); else setError(msg);
    } finally { aiRunIdRef.current = null; setAiLoading(false); }
  }, [full, projectPath, providerId, modelId, aiDepth, aiViews, applyAiImport, flash]);

  const runAiText = useCallback(async () => {
    if (!full || !textInput.trim() || !providerId || !modelId) return;
    const runId = crypto.randomUUID ? crypto.randomUUID() : `run-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    aiRunIdRef.current = runId;
    mmAiProgressBuffer.clear(); // 新一轮导入：清空进度缓冲，日志面板从零开始
    setAiLoading(true); setError("");
    try {
      const r = await mmApi.aiFromText({ documentId: full.document.id, text: textInput, title: textTitle || full.document.name, providerId: providerId || null, modelId: modelId || null, runId });
      applyAiImport(r);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("已取消")) flash(t("mindmap.aiCancelled")); else setError(msg);
    } finally { aiRunIdRef.current = null; setAiLoading(false); }
  }, [full, textInput, textTitle, providerId, modelId, applyAiImport, flash]);

  // IDE 式「停止」：给当前导入运行打取消标志，后端各循环/流式块边界随即中断
  const stopAi = useCallback(async () => {
    const rid = aiRunIdRef.current;
    if (!rid) return;
    try { await mmApi.aiCancel(rid); } catch { /* 取消请求本身失败可忽略：运行自然结束时 ref 会被清空 */ }
  }, []);

  // AI 弹框关闭：任务进行中先询问（确认后停止任务），空闲则直接关。
  // 只有右上角 ✕ / footer 按钮会走到这里；弹框本身无 Esc/遮罩关闭。
  const requestCloseAi = useCallback(() => {
    if (aiLoading) { setAiCloseGuard(true); return; }
    setShowAi(null);
    setAiMinimized(false);
  }, [aiLoading]);

  const confirmCloseAi = useCallback(async () => {
    setAiCloseGuard(false);
    await stopAi();
    setShowAi(null);
    setAiMinimized(false);
  }, [stopAi]);

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
          {showAi === "project" && !aiMinimized && (
            <SharedModal open onClose={requestCloseAi} width={560} headerActions={
              <button type="button" onClick={() => setAiMinimized(true)} title={t("aiMinimized.minimize")}
                className="flex h-6 w-6 items-center justify-center rounded-md text-slate-400 transition hover:bg-white/10 hover:text-white">
                <Minimize2 className="w-3.5 h-3.5" />
              </button>
            } title={t("mindmap.aiProjectTitle")}>
              <div className="flex w-full flex-col gap-3">
                <div className="flex gap-2">
                  <button type="button" className={button} onClick={async () => { const d = await openDialog({ directory: true, multiple: false, title: t("mindmap.pickDir") }); if (typeof d === "string") setProjectPath(d); }}><FolderOpen className="h-3 w-3" />{projectPath ? projectPath.split(/[\\\\/]/).pop() : t("mindmap.pickDir")}</button>
                  <select className={`${selectClass} flex-1`} value={providerId} onChange={e => { const p = providers.find(x => x.id === e.target.value); setProviderId(e.target.value); setModelId(p?.active_model_id ?? p?.models[0]?.id ?? ""); }}>
                    <option value="">{t("mindmap.pickProvider")}</option>{providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
                  </select>
                </div>
                <select className={selectClass} value={modelId} onChange={e => setModelId(e.target.value)} disabled={!providerId}>
                  <option value="">{t("mindmap.pickModel")}</option>{(providers.find(p => p.id === providerId)?.models ?? []).map(m => <option key={m.id} value={m.id}>{m.name || m.id}</option>)}
                </select>
                <textarea className="w-full h-20 rounded-xl bg-slate-900 border border-white/10 px-3 py-2 text-xs text-white outline-none resize-none" value={aiProjectHint} onChange={e => setAiProjectHint(e.target.value)} placeholder={t("mindmap.aiProjectHintPh")} />
                <p className="text-[10px] text-slate-500">{t("mindmap.aiAppendHint")}</p>
                {/* 产物深度：1 最浅清单 → 5 最深业务流与判定方式 */}
                <div className="rounded-xl border border-white/10 bg-slate-900/60 px-3 py-2">
                  <div className="mb-1 flex items-center justify-between">
                    <span className="text-[10px] font-semibold uppercase tracking-wide text-slate-400">{t("mindmap.aiDepthTitle")}</span>
                    <span className="text-[10px] font-semibold" style={{ color: ACCENT }}>{t(`mindmap.aiDepth${aiDepth}`)}</span>
                  </div>
                  <input type="range" min={1} max={5} step={1} value={aiDepth} onChange={e => setAiDepth(Number(e.target.value))} className="w-full accent-cyan-400" />
                  <div className="mt-0.5 flex justify-between text-[9px] text-slate-500">
                    <span>{t("mindmap.aiDepthMin")}</span>
                    <span>{t("mindmap.aiDepthMax")}</span>
                  </div>
                  <p className="mt-1 text-[10px] leading-4 text-slate-500">{t(`mindmap.aiDepthDesc${aiDepth}`)}</p>
                </div>
                {/* 生成哪些视图：不勾选 = 交给 AI 自动判断 */}
                <div className="rounded-xl border border-white/10 bg-slate-900/60 px-3 py-2">
                  <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-slate-400">{t("mindmap.aiViewsTitle")}</div>
                  <div className="flex flex-wrap gap-1.5">
                    {["architecture", "workflow", "dataflow"].map(v => {
                      const on = aiViews.includes(v);
                      return (
                        <button key={v} type="button"
                          className={`rounded-md border px-2 py-1 text-[10px] transition ${on ? "text-white" : "border-white/10 bg-slate-950/60 text-slate-400 hover:text-slate-200"}`}
                          style={on ? { borderColor: ACCENT, backgroundColor: `${ACCENT}22` } : undefined}
                          onClick={() => setAiViews(prev => on ? prev.filter(x => x !== v) : [...prev, v])}>
                          {viewLabel(t, v)}
                        </button>
                      );
                    })}
                  </div>
                  <p className="mt-1 text-[10px] leading-4 text-slate-500">{aiViews.length ? t("mindmap.aiViewsPicked", { count: aiViews.length }) : t("mindmap.aiViewsAuto")}</p>
                </div>
                <button type="button" className="w-full rounded-lg py-2 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!projectPath || !providerId || !modelId || aiLoading} onClick={() => void runAiProject()}>{aiLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin inline mr-1" /> : <Sparkles className="h-3.5 w-3.5 inline mr-1" />}{aiLoading ? t("mindmap.aiAnalyzing") : t("mindmap.importProject")}</button>                  {aiLoading && <AiProgressLog projectRoot={projectPath} onStop={stopAi} />}
              </div>
            </SharedModal>
          )}
          {showAi === "text" && !aiMinimized && (
            <SharedModal open onClose={requestCloseAi} width={560} headerActions={
              <button type="button" onClick={() => setAiMinimized(true)} title={t("aiMinimized.minimize")}
                className="flex h-6 w-6 items-center justify-center rounded-md text-slate-400 transition hover:bg-white/10 hover:text-white">
                <Minimize2 className="w-3.5 h-3.5" />
              </button>
            } title={t("mindmap.aiTextTitle")}>
              <div className="flex w-full flex-col gap-3">
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
                </button>                  {aiLoading && <AiProgressLog onStop={stopAi} />}
              </div>
            </SharedModal>
          )}
          {/* 画布常驻：AI 弹框打开/最小化时也保持挂载，边生成边绘制。
              弹框打开期间设置 modal-mask 抑制全局快捷键，画布仅作背景展示。 */}
          {(full ? (
            <Canvas full={full} accent={ACCENT} onDocumentUpdate={onDocumentUpdated} onHistoryPush={commitHistory} historyVersion={historyVersion} onAiProject={() => setShowAi("project")} onAiText={() => setShowAi("text")} onError={setError} onOpenCalendar={openCalendar} focusRequest={calFocus} onFocusHandled={() => setCalFocus(null)} />
          ) : (
            <VexEmptyState
              title={t("mindmap.emptyTitle")}
              desc={t("mindmap.emptyDesc")}
              tick={t("mindmap.emptyTick")}
              avatarSize={56}
              className="h-full"
            />
          ))}
          {/* 最小化后的后台运行指示：画布右上角悬浮胶囊，点击恢复弹窗 */}
          {aiMinimized && aiLoading && showAi && (
            <AiRunningPill onRestore={() => setAiMinimized(false)} onStop={() => void stopAi()} />
          )}
          {error && !showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-[11px] text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
        </main>
      </div>
      {showCreate && <CreateDocModal onClose={() => setShowCreate(false)} onCreate={(n,d,fid) => { void createDoc(n,d,fid); }} folderId={activeFolderId} />}
      {error && showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-[11px] text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
      {notice && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
      {/* AI 弹框关闭确认：任务进行中询问，确认后停止任务再关闭 */}
      <ConfirmDialog
        open={aiCloseGuard}
        onCancel={() => setAiCloseGuard(false)}
        onConfirm={() => void confirmCloseAi()}
        title={t("aiCloseGuard.title")}
        desc={t("aiCloseGuard.desc")}
        confirmText={t("aiCloseGuard.confirm")}
        cancelText={t("aiCloseGuard.cancel")}
        danger
      />
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