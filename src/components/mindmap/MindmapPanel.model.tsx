import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BaseEdge, EdgeLabelRenderer, Handle, Position, type EdgeProps, type Node, type NodeProps } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { EDGE_STYLES, edgePath, type EdgeStyle } from "../../utils/graphEdgeStyle";
import { EDGE_FLOW_CLASS, EDGE_GLOW_CLASS, headerTint, highlightColor, neonRing, neonTextGlow } from "../../utils/graphHighlight";
import { Brain, File, Image, Trash2, X, Plus, Eye, ChevronDown, ChevronRight, ChevronsRight, Code2, FileText, ListTree, RotateCcw, RotateCw, ArrowLeft, ArrowRight, ArrowUp, ArrowDown } from "lucide-react";
import { DocumentFull, MindmapFolder, MindmapNode, MindmapSticker, kindColor } from "./types";
import { type LayoutDir } from "./layout";
import { moduleAccent } from "../../utils/theme";
import { VEX_CYBER_CYAN } from "../../utils/brand";
import { openSourceFile } from "./agentShared";
export const ACCENT = moduleAccent();

// Agent ops 事件已处理标记（模块级：面板重挂后事件缓冲会重放，不能重复应用）
export const handledAgentOpsAt = new Set<number>();

export const MM_LAST_DOC_KEY = "any_version_mindmap_last_doc";

// 外观走通用层 ui-btn，这里只保留本模块的尺寸（第二栏底栏按钮）
export const button = "ui-btn px-2 py-1.5 disabled:opacity-40";

export const DOC_SOURCE_ICONS: Record<string, (cls: string) => React.ReactNode> = {
  manual: (c) => <ListTree className={c} />,
  ai_project: (c) => <Code2 className={c} />,
  ai_text: (c) => <FileText className={c} />,
  task: (c) => <Brain className={c} />,
};

export const STICKER_PALETTE = ["#fef3c7", "#d4f5d4", "#dbeafe", "#fce7f3", "#e9d5ff", "#fef9c3", "#ccfbf1", "#ffe4e6"];

export const STICKER_ROTATIONS = [-4.5, 2.8, -1.8, 5.2, -3.2, 1.4, 4.1, -2.4];

export function stickerRotation(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) | 0;
  return STICKER_ROTATIONS[Math.abs(hash) % STICKER_ROTATIONS.length];
}

export function stickerSpawnPosition(index: number): { x: number; y: number } {
  // 用黄金角度把贴纸铺成轻微散开的不规则簇，避免整齐网格感。
  const angle = index * 2.39996;
  const radius = 45 + (index % 4) * 42;
  return {
    x: 170 + Math.cos(angle) * radius + (index % 2) * 24,
    y: 120 + Math.sin(angle) * radius + (index % 3) * 18,
  };
}

export function stickerWidth(id: string): number {
  let hash = 0;
  for (let i = 0; i < id.length; i++) hash = (hash * 17 + id.charCodeAt(i)) | 0;
  return 170 + (Math.abs(hash) % 4) * 12;
}

/** 贴纸「展开/收缩」状态（模块级，节点重挂载不丢）：默认收缩只显示 3 行，
 *  展开后显示并编辑全文。图片贴纸不受影响（本身无文字行限制）。 */
export const stickerExpandedSet = new Set<string>();

export function isStickerExpanded(id: string) {
  return stickerExpandedSet.has(id);
}

export function setStickerExpanded(id: string, v: boolean) {
  if (v) stickerExpandedSet.add(id);
  else stickerExpandedSet.delete(id);
}

export function normalizeHexColor(value: string | null | undefined): string | null {
  const raw = value?.trim() ?? "";
  if (/^#[0-9a-fA-F]{6}$/.test(raw)) return raw;
  if (/^#[0-9a-fA-F]{3}$/.test(raw)) {
    return `#${raw.slice(1).split("").map((c) => c + c).join("")}`;
  }
  return null;
}

export const effectiveNodeColor = (node: MindmapNode) => normalizeHexColor(node.color) ?? kindColor(node.kind);

/** 新增根节点/独立节点的随机取色盘（高饱和，与节点类型配色同亮度风格）。 */
export const NODE_RANDOM_COLORS = ["#22d3ee", "#34d399", "#fbbf24", "#60a5fa", "#fb7185", "#a78bfa", "#f97316", "#4ade80", "#f472b6", "#38bdf8"];

export const randomNodeColor = () => NODE_RANDOM_COLORS[Math.floor(Math.random() * NODE_RANDOM_COLORS.length)];

/** 缩小到这个缩放比以下时，节点文字全部隐藏（ComfyUI 式缩略）。 */
export const ZOOM_HIDE_TEXT = 0.45;

// ════════════ 节点 ════════════

/** 节点卡片数据：标题 + 描述区（文件列表可点击跳转）。
 *  连线不再带文字标签（旧 PortChip 类别胶囊已移除，仅保留连接点色块）。 */
export type FlowNodeData = { node: MindmapNode; selected: boolean; /** 与选中节点相连（层级由「高亮层级」偏好决定，默认直接父 / 子）：点亮但比选中弱一档 */ related?: boolean; /** 选中了别人而自己不在其关联范围：压暗，把注意力收到选中项上 */ dimmed?: boolean; /** 高亮强度倍率（画布外观浮层）：环 / 辉光 / 表头底色按它缩放 */ hlBoost?: number; /** 无关项的透明度（按「淡化强度」算出来，不再写死 opacity-35） */ dimOpacity?: number; hasChildren: boolean; collapsed: boolean; hiddenCount: number; hideText: boolean; parentColor: string | null; targetPosition: Position; sourcePosition: Position; projectRoot: string | null; onSelect: () => void; onOpenDetail: () => void; onToggle: () => void; onAddChild: () => void; onPreview: (e: React.MouseEvent) => void; onPreviewEnd: () => void; onDelete: () => void; onContextMenu: (e: React.MouseEvent) => void; };

export const FlowNode = memo(function FlowNode({ data }: NodeProps<Node<FlowNodeData>>) {
  const { t } = useTranslation();
  const { node, selected, related, dimmed, hlBoost, dimOpacity, hasChildren, collapsed, hiddenCount, hideText, parentColor, targetPosition, sourcePosition, projectRoot, onSelect, onOpenDetail, onToggle, onAddChild, onPreview, onPreviewEnd, onDelete, onContextMenu } = data;
  const c = effectiveNodeColor(node);
  /**
   * 高亮：跟随节点自己的颜色（思维导图节点天生有色，不像 ER 图还有「没设色」的情况），
   * 档位与三个画布共用 utils/graphHighlight —— 选中更亮、关联稍弱、其余压暗。
   *
   * `hlBoost` / `dimOpacity` 来自画布外观浮层：两个滑块一动，这里的环、辉光、
   * 表头底色与透明度要立刻跟着变，所以它们必须进 data（memo 靠引用比较）。
   */
  const hlLevel = selected ? "select" : related ? "related" : null;
  const hlColor = hlLevel ? highlightColor(c, hlLevel) : "";
  const hlRing = hlLevel ? neonRing(hlColor, hlLevel, hlBoost) : "";
  const hlGlow = hlLevel ? neonTextGlow(hlColor, hlLevel, hlBoost) : "";
  const hlHeader = hlLevel ? headerTint(hlColor, hlLevel, hlBoost) : "";
  /** 无关项压暗：强度 0 时完全不让步（等同不淡化），拉到 200% 才彻底看不见 */
  const dimStyle = dimmed && (dimOpacity ?? 1) < 0.999 ? { opacity: dimOpacity } : undefined;
  // 折叠态视觉：折叠了子节点的节点要「看起来特殊」——更醒目的边框/辉光 + 琥珀色「折叠徽标」。
  const isFolded = collapsed && hiddenCount > 0;
  // 大幅缩小 → 隐藏全部文字，只留色条与连接点（ComfyUI 式缩略）。
  if (hideText) {
    return (
      // 缩略态也要有高亮/压暗：大幅缩小后一屏几十个节点，不压暗根本看不出选中的是谁
      <article className="group relative h-[26px] w-[96px] cursor-pointer rounded-md border shadow-lg transition-shadow"
        style={{ ...dimStyle, borderColor: hlLevel ? hlColor : `${c}99`, backgroundColor: "var(--color-surface-panel)", boxShadow: hlRing || undefined }} onClick={onSelect} onDoubleClick={(e) => { e.stopPropagation(); onOpenDetail(); }}
        onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e); }}>
        <Handle id="in" type="target" position={targetPosition} isConnectable className="!h-3 !w-3 !border-2 !border-surface-panel" style={{ background: parentColor ?? "#64748b" }} />
        <div className="absolute inset-x-1 bottom-0.5 top-0.5 rounded-sm" style={{ background: `linear-gradient(100deg, ${c}66, ${c}14)` }} />
        <Handle id="out" type="source" position={sourcePosition} isConnectable className="!h-3 !w-3 !border-2 !border-surface-panel" style={{ background: c }} />
      </article>
    );
  }

  return (
    <article
      className={`group relative w-[200px] rounded-card border shadow-lg transition-all cursor-pointer ${hlLevel ? "" : isFolded ? "" : "hover:shadow-xl"}`}
      style={{
        ...dimStyle,
        borderColor: hlLevel ? hlColor : isFolded ? c : `${c}55`,
        borderWidth: hlLevel === "select" || isFolded ? 2 : 1,
        backgroundColor: "var(--color-surface-panel)",
        // 折叠态：琥珀描边辉光，让「折叠了子节点」的节点一眼可辨；
        // 高亮时两者叠加（琥珀圈 + 霓虹环），别让其中一个把另一个吃掉
        boxShadow: isFolded
          ? `${hlRing ? `${hlRing}, ` : ""}0 0 0 1.5px ${c}88, 0 0 16px ${c}66, 0 0 0 3px rgba(251,191,36,0.18)`
          : hlRing || undefined,
      }}
      onClick={onSelect} onDoubleClick={(e) => { e.stopPropagation(); onOpenDetail(); }} onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onContextMenu(e); }}>
      <Handle id="in" type="target" position={targetPosition} isConnectable className="!h-3 !w-3 !border-2 !border-surface-panel" style={{ background: parentColor ?? "#64748b" }} />
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
        style={{ backgroundColor: "var(--color-surface-panel)", borderColor: `${c}66`, color: c, boxShadow: `0 0 8px ${c}44` }}
        onClick={(e) => { e.stopPropagation(); onAddChild(); }} title={t("mindmap.addChild")}>
        <Plus className="h-3.5 w-3.5" />
      </button>
      {/* 标题栏与描述区使用不同背景，节点信息层次保持稳定。折叠态：琥珀箭头 + 折叠徽标 */}
      <div className="flex items-center gap-1.5 rounded-t-[11px] border-b px-2.5 py-2 pr-8" style={{ borderColor: isFolded ? `${c}55` : `${c}35`, backgroundColor: hlHeader || (isFolded ? `${c}33` : `${c}20`) }}>
        {hasChildren && (
          <button type="button"
            className={`nodrag nopan inline-flex h-4 w-4 items-center justify-center rounded transition ${isFolded ? "bg-amber-400/20 text-amber-300 hover:bg-amber-400/30" : "text-slate-400 hover:bg-white/10 hover:text-white"}`}
            onClick={(e) => { e.stopPropagation(); onToggle(); }} title={collapsed ? t("mindmap.expand") : t("mindmap.collapse")}>
            {collapsed ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
          </button>
        )}
        <span className="min-w-0 flex-1 truncate text-caption font-semibold" style={{ color: c, textShadow: hlGlow || undefined }}>{node.name}</span>
        {/* 折叠徽标：一眼看出该节点折叠隐藏了多少子节点（点击箭头可展开） */}
        {isFolded && (
          <span className="nodrag nopan shrink-0 inline-flex items-center gap-0.5 rounded-full border border-amber-400/50 bg-amber-400/15 px-1.5 py-0.5 text-[8px] font-bold text-amber-300 shadow-[0_0_8px_rgba(251,191,36,0.35)]"
            title={t("mindmap.collapsedHint", { count: hiddenCount })}>
            <ChevronsRight className="h-2.5 w-2.5" />+{hiddenCount}
          </span>
        )}
      </div>
      {/* 描述区：展示节点的文件（原证据文件），点击直接用编辑器打开/定位（最多 3 个，超出折叠） */}
      <div className="space-y-0.5 px-2.5 py-1.5">
        {(node.sources?.length ?? 0) > 0 ? (
          <>
            {node.sources!.slice(0, 3).map((f) => (
              <button key={f} type="button"
                onClick={(e) => { e.stopPropagation(); if (projectRoot) openSourceFile(projectRoot, f); }}
                className={`nodrag nopan flex w-full items-center gap-1 rounded border border-cyan-400/20 bg-cyan-400/[0.06] px-1 py-px text-left text-[8px] font-mono text-cyan-200/90 ${projectRoot ? "cursor-pointer transition hover:border-cyan-400/60 hover:bg-cyan-400/15" : "opacity-60"}`}
                title={projectRoot ? f : undefined}>
                <File className="h-2.5 w-2.5 shrink-0" />
                <span className="min-w-0 truncate">{f.split(/[\\/]/).pop()}</span>
              </button>
            ))}
            {(node.sources!.length > 3) && (
              <div className="px-1 text-[8px] text-slate-500" title={node.sources!.slice(3).join("、")}>{t("mindmap.moreFiles", { count: node.sources!.length - 3 })}</div>
            )}
          </>
        ) : (
          <div className="truncate px-0.5 text-[8px] text-slate-600">{t("mindmap.nodeNoFile")}</div>
        )}
      </div>
      <Handle id="out" type="source" position={sourcePosition} isConnectable className="!h-3 !w-3 !border-2 !border-surface-panel" style={{ background: c }} />
    </article>
  );
});

// ════════════ 连线 ════════════

/** 自定义 edge 拿不到组件 state，样式由每条边的 `data.style` 传进来（缺失 = 曲线）。 */
export function edgeStyleOf(data: Record<string, unknown> | undefined): EdgeStyle {
  const style = data?.style as EdgeStyle | undefined;
  return style && EDGE_STYLES.includes(style) ? style : "bezier";
}

/** 连线的高亮外观（由 `edges` 那处统一算好后经 data 传进来）：
 *  `rel` 关联 · `dim` 无关 · `stroke` 无关时的线色（按淡化强度算）·
 *  `width` 线宽（关联时已按跳数退过亮度）· `glow` 关联时的同色辉光 ·
 *  `arrow` 强度为 0 时为 false，箭头与线宽一起收掉。 */
export function edgeLook(data: Record<string, unknown> | undefined) {
  return {
    rel: !!(data?.rel as boolean | undefined),
    dim: !!(data?.dim as boolean | undefined),
    stroke: data?.stroke as string | undefined,
    width: data?.width as number | undefined,
    glow: data?.glow as string | undefined,
    arrow: data?.arrow !== false,
  };
}

export const ColorEdge = memo(function ColorEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps) {
  const [path] = edgePath(edgeStyleOf(data), { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  const color = (data?.color as string | undefined) ?? VEX_CYBER_CYAN;
  /** 关联边：加粗 + 同色辉光；无关边：整条压暗（连光晕一起淡，否则线淡了光晕还在，像有鬼影） */
  const { rel, dim, stroke, width, glow, arrow } = edgeLook(data);
  const gid = `mm-edge-${id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  return (<>
    <defs><linearGradient id={gid} x1="0%" y1="0%" x2="100%" y2="0%"><stop offset="0%" stopColor={color} /><stop offset="100%" stopColor="#f8fafc" /></linearGradient></defs>
    {/* 光晕层：必须显式关掉 dasharray —— React Flow 的 `.animated path` 是「所有 path」，
        不关的话这道光晕会被一起打成流动虚线，观感变脏 */}
    <path d={path} fill="none" stroke={color} strokeWidth={rel ? (width ?? 2.2) * 2.2 : 3} opacity={dim ? 0.03 : rel ? 0.22 : 0.12} className={EDGE_GLOW_CLASS} style={glow ? { filter: glow } : undefined} />
    {/* 内层实线：流转动画挂在这条上（仅当这条边被标记 animated 时才真的动） */}
    <path d={path} fill="none" stroke={dim && stroke ? stroke : `url(#${gid})`} strokeWidth={width ?? 1.5} strokeLinecap="round" markerEnd={arrow ? `url(#arrow-${gid})` : undefined} opacity={dim ? 0.4 : 1} className={EDGE_FLOW_CLASS} />
    <marker id={`arrow-${gid}`} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto"><path d="M0,0 L6,3 L0,6 z" fill="#f8fafc" /></marker>
  </>);
});

/** 自由关系线：与父子树无关的额外连线（虚线 + 中点说明文字）。
 *  标签本身即操作入口：点击改文字，悬停显示 ✕ 删除。 */
export const RelationEdge = memo(function RelationEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps) {
  const { t } = useTranslation();
  const [path, labelX, labelY] = edgePath(edgeStyleOf(data), { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition }, 0.35);
  const color = (data?.color as string | undefined) ?? VEX_CYBER_CYAN;
  const label = (data?.label as string | undefined) ?? "";
  const onEdit = data?.onEdit as ((e: React.MouseEvent) => void) | undefined;
  const onDelete = data?.onDelete as ((e: React.MouseEvent) => void) | undefined;
  const gid = `mm-link-${id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  /** 关联边：加粗 + 走流转动画（平时是静态虚线；style 里不写 dasharray，交给 CSS） */
  const flow = !!(data?.flow as boolean | undefined);
  /** 无关的关系线连标签一起压暗 —— 只淡线不淡标签，会留下一排浮着的孤零零文字 */
  const { dim, stroke, width, glow, arrow } = edgeLook(data);
  return (<>
    <defs><marker id={`arrow-${gid}`} markerWidth="8" markerHeight="8" refX="7" refY="3.5" orient="auto"><path d="M0,0 L7,3.5 L0,7 z" fill={color} /></marker></defs>
    <BaseEdge id={id} path={path} interactionWidth={22} markerEnd={arrow ? `url(#arrow-${gid})` : undefined}
      className={flow ? EDGE_FLOW_CLASS : undefined}
      style={{ stroke: dim && stroke ? stroke : color, strokeWidth: width ?? (flow ? 2.2 : 1.6), strokeDasharray: flow ? undefined : "6 4", opacity: dim ? 0.4 : 1, filter: flow ? glow : undefined }} />
    <EdgeLabelRenderer>
      <div className="nodrag nopan" style={{ position: "absolute", transform: `translate(-50%,-50%) translate(${labelX}px,${labelY}px)`, pointerEvents: "all", opacity: dim ? 0.4 : 1 }}>
        <div className="group/link flex items-center gap-0.5 rounded-full border px-1.5 py-0.5 text-micro shadow backdrop-blur"
          style={{ borderColor: `${color}77`, backgroundColor: "color-mix(in srgb, var(--color-surface-panel) 92%, transparent)", color }}>
          <button type="button" className="max-w-[140px] cursor-pointer truncate hover:underline" title={t("mindmap.linkEditHint")}
            onClick={(e) => { e.stopPropagation(); onEdit?.(e); }}>{label || t("mindmap.linkMode")}</button>
          <button type="button" className="hidden rounded-full p-0.5 text-slate-500 transition group-hover/link:block hover:bg-white/10 hover:text-red-400"
            title={t("mindmap.delete")} onClick={(e) => { e.stopPropagation(); onDelete?.(e); }}><X className="h-2.5 w-2.5" /></button>
        </div>
      </div>
    </EdgeLabelRenderer>
  </>);
});

// ════════════ 贴纸节点 ════════════

export type StickerNodeData = {
  sticker: MindmapSticker;
  onUpdate: (patch: { content?: string; color?: string; imageData?: string; rotation?: number }) => void;
  onRotate: (delta: number) => void;
  onReplaceImage: () => void;
  onDelete: () => void;
};

export const StickerFlowNode = memo(function StickerFlowNode({ data }: NodeProps<Node<StickerNodeData>>) {
  const { sticker, onUpdate, onRotate, onReplaceImage, onDelete } = data;
  const bg = sticker.color || "#fef3c7";
  const { t } = useTranslation();
  const rotation = sticker.rotation ?? stickerRotation(sticker.id);
  const isImage = Boolean(sticker.imageData);
  // 展开/收缩：默认只显示 3 行文字（收缩态）；展开后显示并编辑全文。
  // 状态存模块级集合，ReactFlow 节点重挂载后仍保留展开状态。
  const [expanded, setExpanded] = useState(() => isStickerExpanded(sticker.id));
  const taRef = useRef<HTMLTextAreaElement>(null);
  const toggleExpand = useCallback(() => {
    setExpanded((prev) => { const next = !prev; setStickerExpanded(sticker.id, next); return next; });
  }, [sticker.id]);
  // 展开时把输入框高度贴合内容（自动增高，不出现滚动条）
  useEffect(() => {
    const el = taRef.current;
    if (el && expanded) { el.style.height = "auto"; el.style.height = `${el.scrollHeight}px`; }
  }, [expanded, sticker.content]);
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
      {!isImage && <button type="button" className="rounded p-1 text-slate-500 hover:bg-black/10 hover:text-slate-800" onClick={(e) => { e.stopPropagation(); toggleExpand(); }} title={expanded ? t("mindmap.stickerCollapseHint") : t("mindmap.stickerExpandHint")}><ChevronDown className={`h-3 w-3 transition-transform ${expanded ? "rotate-180" : ""}`} /></button>}
      <button type="button" className="rounded p-1 text-slate-400 hover:bg-black/10 hover:text-red-500" onClick={(e) => { e.stopPropagation(); onDelete(); }} title={t("mindmap.deleteSticker")}><X className="h-3 w-3" /></button>
    </div>
    {isImage ? (
      <>
        <div className="flex min-h-[80px] items-center justify-center overflow-hidden border border-black/10 bg-white/35">
          <img src={sticker.imageData} alt={sticker.content || t("mindmap.imageStickerAlt")} className="block max-h-[180px] max-w-full object-contain" draggable={false} />
        </div>
        <textarea className="nodrag nowheel mt-2 w-full resize-none bg-transparent text-tiny leading-4 text-slate-800 outline-none" value={sticker.content}
          onChange={(e) => onUpdate({ content: e.target.value })} rows={2} placeholder={t("mindmap.imageCaptionPh")} />
      </>
    ) : (
      <div
        className="relative min-h-[50px] cursor-pointer"
        onDoubleClick={(e) => { e.stopPropagation(); if (!expanded) toggleExpand(); }}
        title={expanded ? t("mindmap.stickerCollapseHint") : t("mindmap.stickerExpandHint")}
      >
        {expanded ? (
          <textarea ref={taRef} className="nodrag nowheel w-full resize-none bg-transparent text-tiny leading-4 text-slate-800 outline-none" value={sticker.content}
            onChange={(e) => onUpdate({ content: e.target.value })}
            rows={Math.max(3, sticker.content.split("\n").length)} placeholder={t("mindmap.stickerPh")} />
        ) : (
          <div className="line-clamp-3 whitespace-pre-wrap break-words text-tiny leading-4 text-slate-800">
            {sticker.content || <span className="text-slate-500">{t("mindmap.stickerPh")}</span>}
          </div>
        )}
      </div>
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

// ════════════ 自动布局 ════════════

/** 布局方向按钮组的文案 key（方向与紧凑排布的实现都在 ./layout） */
export const LAYOUT_DIR_KEYS: Record<LayoutDir, string> = { lr: "mindmap.dirLr", rl: "mindmap.dirRl", tb: "mindmap.dirTb", bt: "mindmap.dirBt" };

/** 布局方向按钮组的图标：下拉框改成按钮组后语义由图标 + tooltip 承载（写法同 DOC_SOURCE_ICONS） */
export const LAYOUT_DIR_ICONS: Record<LayoutDir, (cls: string) => React.ReactNode> = {
  lr: (c) => <ArrowRight className={c} />,
  rl: (c) => <ArrowLeft className={c} />,
  tb: (c) => <ArrowDown className={c} />,
  bt: (c) => <ArrowUp className={c} />,
};

// ════════════ 拖拽改上级：目标节点命中判定 ════════════

/** 点 (px,py) 是否落在以 (sx,sy) 为左上角、宽 w 高 h 的矩形内（拖拽改上级的落点判定）。 */
export function pointInRect(px: number, py: number, sx: number, sy: number, w: number, h: number): boolean {
  return px >= sx && px <= sx + w && py >= sy && py <= sy + h;
}

/** 收集 id 及其全部后代 id（拖到自己的子孙上会成环，必须排除）。 */
export function withDescendants(id: string, nodes: MindmapNode[]): Set<string> {
  const out = new Set<string>([id]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const n of nodes) {
      if (n.parentId && out.has(n.parentId) && !out.has(n.id)) { out.add(n.id); changed = true; }
    }
  }
  return out;
}

// ════════════ 画布 ════════════

// nodeTypes/edgeTypes 必须在组件外定义为常量：若在 JSX 内联新建，每次渲染都会
// 产生新对象，React Flow 会因此反复重渲染（官方文档明确警告的卡顿/卡死源）。
export const mmNodeTypes = { mmNode: FlowNode, stickerNode: StickerFlowNode };

export const mmEdgeTypes = { colorE: ColorEdge, relationE: RelationEdge };

export type PosOverride = { x: number; y: number };

// 节点对象缓存条目：用于拖拽时复用未变化节点对象，避免全部节点每帧重渲染导致闪烁
export type NodeCacheEntry = {
  node: MindmapNode | MindmapSticker;
  full: DocumentFull;
  px: number;
  py: number;
  selected: boolean;
  /** 高亮三态必须进缓存键：漏了就会「点了不亮/亮的是上一次的那个」 */
  related: boolean;
  dimmed: boolean;
  /** 外观滑块也要进缓存键：漏了就是「拖滑块、节点没反应」（与三态同级） */
  hlBoost: number;
  dimOpacity: number;
  hasChildren: boolean;
  collapsed: boolean;
  hiddenCount: number;
  hideText: boolean;
  parentColor: string | null;
  obj: Node;
};

// ════════════ 主面板 ════════════

export function formatTime(v: string): string {
  if (!v) return ""; const d = new Date(v); if (Number.isNaN(d.getTime())) return v;
  return new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(d).replace(/\//g, "-");
}

// ─── 文件夹树：扁平列表 → 层级（支持拖拽整理） ───

export interface FolderNode {
  folder: MindmapFolder;
  children: FolderNode[];
}

export function buildChildren(folders: MindmapFolder[], parentId: string | null): FolderNode[] {
  return folders
    .filter((f) => (f.parentId ?? null) === parentId)
    .map((f) => ({ folder: f, children: buildChildren(folders, f.id) }));
}

/** candidateId 是否位于 id 的子树（后代）中 */
export function isFolderDescendant(folders: MindmapFolder[], id: string, candidateId: string): boolean {
  const children = folders.filter((f) => f.parentId === id);
  return children.some((c) => c.id === candidateId || isFolderDescendant(folders, c.id, candidateId));
}

/** 从根到 activeId 的路径（不含「全部文档」占位） */
export function getFolderPath(folders: MindmapFolder[], activeId: string | null): MindmapFolder[] {
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
