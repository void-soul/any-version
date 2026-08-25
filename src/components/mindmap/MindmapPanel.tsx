import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
import {
  Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, ReactFlowProvider, getBezierPath, useReactFlow,
  type Edge, type EdgeProps, type Node, type NodeChange, type NodeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { MindmapMarkdown } from "./MindmapMarkdown";
import {
  AlertTriangle, Brain, Circle, File, Folder, FolderOpen, LayoutGrid, Lightbulb, Loader2, Lock, Puzzle,
  Route, ScrollText, Server, Settings, Sparkles, StickyNote, Trash2, X, Plus, Pencil, Eye, Columns,
  ChevronDown, ChevronRight, FolderPlus, Search, Maximize2, Minimize2, GripVertical, Code2, FileText, ListTree,
} from "lucide-react";
import type { AiConfig } from "../ai/types";
import { DocumentFull, MindmapDocument, MindmapFolder, MindmapNode, MindmapSticker, PositionInput, kindColor, mmApi } from "./types";
import { moduleAccent } from "../../utils/theme";

const ACCENT = moduleAccent();
const button = "inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";
const selectClass = "h-8 min-w-[110px] rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60";
const DOC_SOURCE_LABELS: Record<string, string> = { manual: "手动画", ai_project: "AI 读项目", ai_text: "AI 析需求", task: "任务" };
const DOC_SOURCE_ICONS: Record<string, (cls: string) => React.ReactNode> = {
  manual: (c) => <ListTree className={c} />,
  ai_project: (c) => <Code2 className={c} />,
  ai_text: (c) => <FileText className={c} />,
  task: (c) => <Brain className={c} />,
};
const STICKER_PALETTE = ["#fef3c7", "#d4f5d4", "#dbeafe", "#fce7f3", "#e9d5ff", "#fef9c3", "#ccfbf1", "#ffe4e6"];

// 节点类型（kind）→ 图标（与文档列表同风格的 lucide 图标）+ 中文名
const KIND_ICONS: Record<string, (cls: string) => React.ReactNode> = {
  root: (c) => <ListTree className={c} />,
  module: (c) => <Folder className={c} />,
  task: (c) => <Brain className={c} />,
  requirement: (c) => <FileText className={c} />,
  constraint: (c) => <Lock className={c} />,
  risk: (c) => <AlertTriangle className={c} />,
  component: (c) => <Puzzle className={c} />,
  service: (c) => <Server className={c} />,
  route: (c) => <Route className={c} />,
  config: (c) => <Settings className={c} />,
  file: (c) => <File className={c} />,
  other: (c) => <Circle className={c} />,
};
const KIND_LABELS: Record<string, string> = { root: "根节点", module: "模块", task: "任务", requirement: "需求", constraint: "约束", risk: "风险", component: "组件", service: "服务", route: "路由", config: "配置", file: "文件", other: "其他" };

// ════════════ 节点 ════════════

type FlowNodeData = { node: MindmapNode; selected: boolean; hasChildren: boolean; collapsed: boolean; onSelect: () => void; onOpenDetail: () => void; onToggle: () => void; onAddChild: () => void; onPreview: (e: React.MouseEvent) => void; onPreviewEnd: () => void; onDelete: () => void; };

const FlowNode = memo(function FlowNode({ data }: NodeProps<Node<FlowNodeData>>) {
  const { node, selected, hasChildren, collapsed, onSelect, onOpenDetail, onToggle, onAddChild, onPreview, onPreviewEnd, onDelete } = data;
  const c = node.color && node.color !== "#f59e0b" ? node.color : kindColor(node.kind);
  return (
    <article className={`group relative w-[200px] rounded-xl border px-2.5 py-2 shadow-lg transition-shadow cursor-pointer ${selected ? "shadow-cyan-500/30 ring-1 ring-cyan-400/40" : "hover:shadow-xl"}`}
      style={{ borderColor: selected ? c : `${c}55`, background: "linear-gradient(135deg, #111c2e 0%, #0d1524 100%)" }} onClick={onSelect} onDoubleClick={(e) => { e.stopPropagation(); onOpenDetail(); }}>
      <Handle type="target" position={Position.Left} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: c }} />
      {/* 节点右上角悬浮按钮：预览（气泡）+ 删除 */}
      <div className="nodrag nopan absolute right-1 top-1 z-10 hidden items-center gap-0.5 group-hover:flex">
        <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-cyan-300"
          onMouseEnter={onPreview} onMouseMove={onPreview} onMouseLeave={onPreviewEnd} title="预览（只读）">
          <Eye className="h-3 w-3" />
        </button>
        <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-red-400"
          onClick={(e) => { e.stopPropagation(); onDelete(); }} title="删除节点">
          <Trash2 className="h-3 w-3" />
        </button>
      </div>
      {/* 节点右侧悬浮 + 按钮：给当前节点直接添加子节点 */}
      <button type="button" className="nodrag nopan absolute -right-2.5 top-1/2 z-10 hidden h-5 w-5 -translate-y-1/2 items-center justify-center rounded-full border transition group-hover:flex hover:scale-110"
        style={{ backgroundColor: "#0d1524", borderColor: `${c}66`, color: c, boxShadow: `0 0 8px ${c}44` }}
        onClick={(e) => { e.stopPropagation(); onAddChild(); }} title="添加子节点">
        <Plus className="h-3.5 w-3.5" />
      </button>
      <div className="flex items-center gap-1.5 pr-8">
        {hasChildren && <button type="button" className="nodrag nopan inline-flex h-4 w-4 items-center justify-center text-slate-500 hover:text-white" onClick={(e) => { e.stopPropagation(); onToggle(); }} title={collapsed ? "展开" : "折叠"}>
          {collapsed ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
        </button>}
        <span className="min-w-0 flex-1 truncate text-[11px] font-semibold" style={{ color: c }}>{node.name}</span>
      </div>
      {node.description && <div className="mt-1 line-clamp-2 text-[9px] leading-4 text-slate-400">{node.description}</div>}
      <div className="mt-1.5 flex items-center gap-1.5">
        <span title={KIND_LABELS[node.kind] ?? node.kind} className="inline-flex items-center rounded-md border px-1 py-0.5" style={{ borderColor: `${c}44`, color: c, backgroundColor: `${c}14` }}>{KIND_ICONS[node.kind]?.( "h-3 w-3") ?? <Circle className="h-3 w-3" />}</span>
        {node.progress > 0 && <span className="text-[8px] text-slate-500">{node.progress}%</span>}
      </div>
      {node.progress > 0 && <div className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-slate-800/80"><div className="h-full rounded-full transition-all" style={{ width: `${node.progress}%`, backgroundColor: c, boxShadow: `0 0 6px ${c}66` }} /></div>}
      <Handle type="source" position={Position.Right} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: c }} />
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

type StickerNodeData = { sticker: MindmapSticker; onUpdate: (patch: { content?: string; color?: string }) => void; onDelete: () => void };

const StickerFlowNode = memo(function StickerFlowNode({ data }: NodeProps<Node<StickerNodeData>>) {
  const { sticker, onUpdate, onDelete } = data;
  const bg = sticker.color || "#fef3c7";
  return (<div className="group relative w-[190px] rounded-lg border border-white/10 p-3 shadow-xl" style={{ backgroundColor: bg }}>
    <button type="button" className="nodrag nopan absolute right-1 top-1 hidden h-5 w-5 items-center justify-center rounded text-slate-400 group-hover:flex hover:bg-black/10 hover:text-red-500" onClick={(e) => { e.stopPropagation(); onDelete(); }}><X className="h-3 w-3" /></button>
    <textarea className="nodrag nowheel w-full resize-none bg-transparent text-[10px] text-slate-800 outline-none" value={sticker.content}
      onChange={(e) => onUpdate({ content: e.target.value })} rows={3} placeholder="写点什么..." style={{ minHeight: 50 }} />
    {/* 贴纸颜色：预设淡色 + 自定义取色器（与节点同一套机制，颜色更浅） */}
    <div className="mt-1.5 flex items-center gap-1 opacity-60 transition group-hover:opacity-100">
      {STICKER_PALETTE.slice(0, 6).map(cl => <button key={cl} type="button" className="h-3.5 w-3.5 rounded-full border border-black/20" style={{ backgroundColor: cl, boxShadow: bg === cl ? `0 0 0 1.5px rgba(0,0,0,.45)` : "none" }}
        onClick={(e) => { e.stopPropagation(); onUpdate({ color: cl }); }} />)}
      <label className="relative inline-flex h-3.5 w-3.5 cursor-pointer items-center justify-center overflow-hidden rounded-full border border-black/25" title="自定义颜色">
        <input type="color" value={/^#[0-9a-fA-F]{6}$/.test(bg) ? bg : "#fef3c7"} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
          onChange={(e) => { e.stopPropagation(); onUpdate({ color: e.target.value }); }} />
        <span className="h-2 w-2 rounded-full" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
      </label>
    </div>
  </div>);
});

// ════════════ 自动布局 ════════════

function layoutTree(nodes: MindmapNode[]): Map<string, { x: number; y: number }> {
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
  const yByDepth = new Map<number, number>();
  for (const id of order) {
    const d = depth.get(id) ?? 0;
    const y = yByDepth.get(d) ?? 0;
    pos.set(id, { x: d * 260, y });
    yByDepth.set(d, y + 80);
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

function DetailModal({ node, onUpdate, onClose }: { node: MindmapNode; accent?: string; onUpdate: (patch: Partial<MindmapNode>) => void; onClose: () => void }) {
  const [tab, setTab] = useState<"view" | "edit">("view");
  const [detail, setDetail] = useState(node.detail);
  const [description, setDescription] = useState(node.description);
  const [name, setName] = useState(node.name);
  const [kind, setKind] = useState(node.kind);
  const [progress, setProgress] = useState(node.progress);
  const [color, setColor] = useState(node.color);
  const [split, setSplit] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  const [splitRatio, setSplitRatio] = useState(50);
  const dragging = useRef(false);
  const c = color && color !== "#f59e0b" ? color : kindColor(node.kind);
  const kinds = ["root", "module", "task", "requirement", "constraint", "risk", "component", "service", "route", "config", "file", "other"];
  const COLORS = ["#f8fafc","#22d3ee","#34d399","#fbbf24","#60a5fa","#fb7185","#a78bfa","#f97316","#f59e0b","#94a3b8"];

  const save = useCallback(() => {
    onUpdate({ name, description, color, kind, progress, detail });
  }, [name, description, color, kind, progress, detail, onUpdate]);

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

  // Draggable separator
  const onMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    dragging.current = true;
    const startX = e.clientX;
    const startRatio = splitRatio;
    const onMove = (ev: MouseEvent) => {
      if (!dragging.current) return;
      const dx = ev.clientX - startX;
      const container = (ev.target as HTMLElement).closest(".detail-body") as HTMLElement | null;
      const w = container?.clientWidth ?? 1;
      setSplitRatio(Math.max(20, Math.min(80, startRatio + (dx / w) * 100)));
    };
    const onUp = () => { dragging.current = false; window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, [splitRatio]);

  return createPortal(
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]" onClick={onClose}>
      <div className={`flex ${fullscreen ? "h-full w-full" : split ? "h-[85vh] w-[min(95vw,1100px)]" : "h-[80vh] w-[min(92vw,680px)]"} flex-col overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl`} onClick={(e) => e.stopPropagation()}>
        <div className="flex h-11 shrink-0 items-center gap-2 border-b border-white/10 px-3" style={{ backgroundColor: `${c}1f` }}>
          <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: c, boxShadow: `0 0 9px ${c}` }} />
          <input className="min-w-0 flex-1 bg-transparent text-[12px] font-semibold text-slate-100 outline-none" value={name} onChange={(e) => setName(e.target.value)} onBlur={save} />
          <span className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[9px] uppercase" style={{ borderColor: `${c}44`, color: c }}>{KIND_ICONS[node.kind]?.( "h-3 w-3")}{KIND_LABELS[node.kind] ?? node.kind}</span>
          <div className="flex items-center gap-1 ml-1">
            <button type="button" className="nodrag nopan rounded p-1 text-[10px] text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => setTab(tab === "view" ? "edit" : "view")} title={tab === "view" ? "编辑" : "预览"}>{tab === "view" ? <Pencil className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}</button>
            <button type="button" className={`nodrag nopan rounded p-1 text-[10px] ${split ? "bg-white/10 text-white" : "text-slate-400"} hover:bg-white/10 hover:text-white`} onClick={() => setSplit(!split)} title="分栏"><Columns className="h-3.5 w-3.5" /></button>
            <button type="button" className={`nodrag nopan rounded p-1 text-[10px] ${fullscreen ? "bg-white/10 text-white" : "text-slate-400"} hover:bg-white/10 hover:text-white`} onClick={() => setFullscreen(!fullscreen)} title={fullscreen ? "退出全屏" : "全屏"}>{fullscreen ? <Minimize2 className="h-3.5 w-3.5" /> : <Maximize2 className="h-3.5 w-3.5" />}</button>
            <button type="button" className="nodrag nopan rounded p-1 text-slate-400 hover:text-white" onClick={onClose}><X className="h-4 w-4" /></button>
          </div>
        </div>
        {tab === "edit" ? (
          <div className="min-h-0 flex-1 overflow-y-auto p-4 space-y-3">
            <div>
              <label className="text-[9px] text-slate-500 block mb-1">描述</label>
              <textarea className="w-full rounded bg-slate-900 border border-white/10 px-3 py-2 text-[11px] text-white outline-none resize-none" value={description} onChange={(e) => setDescription(e.target.value)} rows={2} onBlur={save} />
            </div>
            <div className="flex gap-3">
              <div className="flex-1"><label className="text-[9px] text-slate-500 block mb-1">类型</label>
                <select className={`${selectClass} w-full`} value={kind} onChange={(e) => { setKind(e.target.value); onUpdate({ kind: e.target.value }); }}>
                  {kinds.map(k => <option key={k} value={k}>{KIND_LABELS[k] ?? k}</option>)}
                </select>
              </div>
              <div><label className="text-[9px] text-slate-500 block mb-1">进度</label>
                <input type="range" min={0} max={100} value={progress} onChange={(e) => { const v = Number(e.target.value); setProgress(v); onUpdate({ progress: v }); }} className="w-20 h-6" />
                <span className="text-[10px] text-slate-400 ml-1">{progress}%</span>
              </div>
            </div>
            <div>
              <label className="text-[9px] text-slate-500 block mb-1">颜色</label>
              <div className="flex gap-1.5 flex-wrap items-center">
                {COLORS.map(cl => <button key={cl} type="button" className="h-5 w-5 rounded-full border border-white/20" style={{ backgroundColor: cl, boxShadow: color === cl ? `0 0 6px ${cl}` : "none" }}
                  onClick={() => { setColor(cl); onUpdate({ color: cl }); }} />)}
                {/* 自定义任意颜色：原生取色器 + hex 输入 + 恢复默认 */}
                <label className="relative inline-flex h-5 w-5 cursor-pointer items-center justify-center overflow-hidden rounded-full border border-white/30" title="自定义颜色">
                  <input type="color" value={/^#[0-9a-fA-F]{6}$/.test(color) ? color : "#22d3ee"} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
                    onChange={(e) => { const v = e.target.value; setColor(v); onUpdate({ color: v }); }} />
                  <span className="h-3 w-3 rounded-full" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
                </label>
                <input value={color || ""} onChange={(e) => { const v = e.target.value; setColor(v); onUpdate({ color: v || "" }); }} placeholder="#RRGGBB"
                  className="h-5 w-[74px] rounded border border-white/15 bg-slate-900 px-1.5 text-[9px] text-slate-300 outline-none focus:border-cyan-400/60" />
                <button type="button" className="rounded border border-white/15 px-1.5 py-0.5 text-[9px] text-slate-400 hover:text-white" onClick={() => { setColor(""); onUpdate({ color: "" }); }} title="恢复为按类型自动配色">自动</button>
              </div>
            </div>
            <div><label className="text-[9px] text-slate-500 block mb-1">详细内容 (Markdown)</label>
              <textarea className="w-full min-h-[200px] rounded bg-slate-900 border border-white/10 px-3 py-2 text-[11px] text-white font-mono outline-none resize-none" value={detail}
                onChange={(e) => setDetail(e.target.value)} onBlur={save} placeholder="支持 Markdown..." />
            </div>
          </div>
        ) : (
          <div className={`detail-body flex min-h-0 flex-1 ${split ? "flex-row" : "flex-col"}`}>
            <div className={`${split ? "" : ""} overflow-y-auto p-4`} style={split ? { width: `${splitRatio}%` } : {}}>
              <MindmapMarkdown content={node.detail || node.description || "暂无详细说明"} />
            </div>
            {split && (
              <>
                <div className="flex w-[5px] shrink-0 cursor-col-resize items-center justify-center border-x border-white/10 bg-slate-900/50 hover:bg-cyan-400/20" onMouseDown={onMouseDown}>
                  <GripVertical className="h-3 w-3 text-slate-600" />
                </div>
                <div className="overflow-y-auto p-1" style={{ width: `${100 - splitRatio}%` }}>
                  <textarea className="w-full h-full min-h-[400px] rounded bg-slate-900 border border-white/10 px-3 py-2 text-[11px] text-white font-mono outline-none resize-none"
                    value={detail} onChange={(e) => setDetail(e.target.value)} onBlur={save} />
                </div>
              </>
            )}
          </div>
        )}
      </div>
    </div>, document.body);
}

// ════════════ 创建文档弹窗 ════════════

function CreateDocModal({ onClose, onCreate, folderId }: { onClose: () => void; onCreate: (name: string, desc: string, sourceType: string, folderId: string | null) => void; folderId: string | null }) {
  const [name, setName] = useState(""); const [desc, setDesc] = useState(""); const [st, setSt] = useState("manual");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => { inputRef.current?.focus(); }, []);

  return createPortal(
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]" onClick={onClose}>
      <div className="w-[380px] rounded-xl border border-white/10 bg-[#0d1524] p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-semibold text-white">新建思维导图</h3>
          <button type="button" className="text-slate-500 hover:text-white" onClick={onClose}><X className="h-4 w-4" /></button>
        </div>
        <div className="space-y-3">
          <div><label className="text-[10px] text-slate-400 block mb-1">名称</label><input ref={inputRef} className="w-full h-9 rounded-lg bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none" value={name} onChange={(e) => setName(e.target.value)} placeholder="思维导图名称" onKeyDown={(e) => { if (e.key === "Enter" && name.trim()) onCreate(name.trim(), desc, st, folderId); }} /></div>
          <div><label className="text-[10px] text-slate-400 block mb-1">描述</label><textarea className="w-full h-16 rounded-lg bg-slate-900 border border-white/10 px-3 py-2 text-xs text-white outline-none resize-none" value={desc} onChange={(e) => setDesc(e.target.value)} placeholder="可选描述" /></div>
          <div><label className="text-[10px] text-slate-400 block mb-1">来源</label>
            <div className="grid grid-cols-2 gap-1.5">
              {Object.entries(DOC_SOURCE_LABELS).map(([k, v]) => (
                <button key={k} type="button" className={`rounded-lg border px-3 py-2 text-[10px] transition ${st === k ? "border-cyan-400/40 bg-cyan-400/10 text-white" : "border-white/10 text-slate-400 hover:bg-white/5"}`}
                  onClick={() => setSt(k)}>{v}</button>
              ))}
            </div>
          </div>
          <button type="button" className="w-full rounded-lg py-2 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }}
            disabled={!name.trim()} onClick={() => { if (name.trim()) onCreate(name.trim(), desc, st, folderId); }}>创建</button>
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

function CanvasInner({ full, accent, onDocumentUpdate, onHistoryPush, historyVersion }: { full: DocumentFull; accent: string; onDocumentUpdate: (d: DocumentFull) => void; onHistoryPush: () => void; historyVersion: number }) {
  const { fitView } = useReactFlow();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detailNode, setDetailNode] = useState<MindmapNode | null>(null);
  const [preview, setPreview] = useState<{ node: MindmapNode; x: number; y: number } | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; nodeId: string } | null>(null);
  const [lastSaved, setLastSaved] = useState<number | null>(null);
  // 用户拖放产生的位置覆盖（本地状态）。节点位置 = posOverrides ?? (已保存坐标 ?? 自动布局)。
  // 关键：拖放不再依赖 nodes state + useEffect 同步（那会在 WebView2/React19 下形成
  // onNodesChange → setNodes → 重建节点 → 重新测量 → onNodesChange 的死循环），
  // 而是直接驱动本地 posOverrides，一次性 setState，物理上杜绝反馈循环。
  const [posOverrides, setPosOverrides] = useState<Record<string, PosOverride>>({});
  // 切换文档时清空位置覆盖（新文档用其自身已保存坐标或自动布局）
  useEffect(() => { setPosOverrides({}); }, [full.document.id]);
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
  const layout = useMemo(() => layoutTree(graphNodes), [graphNodes]);
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

  const addChildNode = useCallback((parentIdOverride?: string) => {
    onHistoryPush();
    const parentId = parentIdOverride ?? selectedId ?? graphNodes.find(n => !n.parentId)?.id ?? "root";
    const n: MindmapNode = { id: `n${Date.now()}`, documentId: full.document.id, parentId, name: "新节点", description: "", detail: "", kind: "other", color: "#f59e0b", progress: 0, positionX: 0, positionY: 0, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
    void mmApi.upsertNode({ documentId: full.document.id, node: n });
    onDocumentUpdate({ ...full, nodes: [...full.nodes, n] });
  }, [selectedId, graphNodes, full, onDocumentUpdate, onHistoryPush]);

  const deleteNode = useCallback((nodeId: string) => {
    onHistoryPush();
    void mmApi.deleteNode({ documentId: full.document.id, nodeId });
    onDocumentUpdate({ ...full, nodes: full.nodes.filter(n => n.id !== nodeId && n.parentId !== nodeId) });
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
      const prevObj = (prev?.obj ?? null) as Node<FlowNodeData> | null;
      const obj: Node<FlowNodeData> = prevObj
        // 复用旧对象并保留 React Flow 内部字段（measured/dragging/selected 等）：
        // 拖拽时若重建全新对象会丢失 measured → React Flow 每帧重新测量 → 被拖节点闪烁。
        ? { ...prevObj, position: p, data: { node: n, selected, hasChildren, collapsed: isCollapsed,
            onSelect: () => setSelectedId(n.id), onOpenDetail: () => openDetail(n),
            onToggle: () => setCollapsed(cur => { const nx = new Set(cur); nx.has(n.id) ? nx.delete(n.id) : nx.add(n.id); return nx; }),
            onAddChild: () => addChildNode(n.id),
            onPreview: (e) => setPreview({ node: n, x: e.clientX, y: e.clientY }),
            onPreviewEnd: () => setPreview(null),
            onDelete: () => deleteNode(n.id) } }
        : { id: n.id, type: "mmNode", position: p,
            data: { node: n, selected, hasChildren, collapsed: isCollapsed,
              onSelect: () => setSelectedId(n.id), onOpenDetail: () => openDetail(n),
              onToggle: () => setCollapsed(cur => { const nx = new Set(cur); nx.has(n.id) ? nx.delete(n.id) : nx.add(n.id); return nx; }),
              onAddChild: () => addChildNode(n.id),
              onPreview: (e) => setPreview({ node: n, x: e.clientX, y: e.clientY }),
              onPreviewEnd: () => setPreview(null),
              onDelete: () => deleteNode(n.id) },
            sourcePosition: Position.Right, targetPosition: Position.Left };
      cache.set(n.id, { node: n, full, px: p.x, py: p.y, selected, hasChildren, collapsed: isCollapsed, obj });
      main.push(obj);
    }
    const stickerNodes: Node<StickerNodeData>[] = [];
    for (const s of stickers) {
      const sid = `sticker-${s.id}`;
      alive.add(sid);
      const p = posOverrides[sid] ?? { x: s.positionX, y: s.positionY };
      const prev = cache.get(sid);
      if (prev && prev.full === full && prev.node === s && prev.px === p.x && prev.py === p.y) {
        stickerNodes.push(prev.obj as Node<StickerNodeData>);
        continue;
      }
      const updateSticker = (patch: { content?: string; color?: string }) => {
        const next = { ...s, ...patch };
        void mmApi.upsertSticker({ documentId: full.document.id, sticker: next });
        onDocumentUpdate({ ...full, stickers: full.stickers.map(x => x.id === s.id ? next : x) });
      };
      const deleteSticker = () => {
        void mmApi.deleteSticker({ documentId: full.document.id, stickerId: s.id });
        onDocumentUpdate({ ...full, stickers: full.stickers.filter(x => x.id !== s.id) });
      };
      const prevObj = (prev?.obj ?? null) as Node<StickerNodeData> | null;
      const obj: Node<StickerNodeData> = prevObj
        ? { ...prevObj, position: p, data: { sticker: s, onUpdate: updateSticker, onDelete: deleteSticker } }
        : { id: sid, type: "stickerNode", position: p,
            data: { sticker: s, onUpdate: updateSticker, onDelete: deleteSticker } };
      cache.set(sid, { node: s, full, px: p.x, py: p.y, selected: false, hasChildren: false, collapsed: false, obj });
      stickerNodes.push(obj);
    }
    // 清理不再显示的缓存项
    for (const k of cache.keys()) if (!alive.has(k)) cache.delete(k);
    return [...main, ...stickerNodes];
  }, [visibleNodes, layout, selectedId, collapsed, childrenCount, stickers, full, onDocumentUpdate, highlightChain, posOverrides, addChildNode, deleteNode, openDetail]);

  const edges = useMemo<Edge[]>(() => visibleNodes.flatMap(n => {
    if (!n.parentId || !visibleNodes.some(p => p.id === n.parentId)) return [];
    const isOnChain = highlightChain.includes(n.id) && highlightChain.includes(n.parentId);
    return [{ id: `mm-e-${n.id}`, source: n.parentId, target: n.id, type: "colorE", style: isOnChain ? { strokeWidth: 2, opacity: 0.9 } : {},
      data: { color: n.color && n.color !== "#f59e0b" ? n.color : kindColor(n.kind) }, markerEnd: { type: MarkerType.ArrowClosed, color: "#f8fafc" } } as Edge];
  }), [visibleNodes, highlightChain]);

  // 受控节点：React Flow 拖放时把位置写入 posOverrides。仅处理 position 变更，
  // 忽略 dimensions/select 等（选择由 selectedId 管理，尺寸由 React Flow 内部测量）。
  // 一次 setState → 一次渲染 → 收敛，无反馈循环。
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
  }, []);

  useEffect(() => { const t = window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 0); return () => window.clearTimeout(t); }, [fitView, full.document.id, collapsed]);

  const updateNode = useCallback((patch: Partial<MindmapNode>) => {
    if (!detailNode) return;
    const updated = { ...detailNode, ...patch, updatedAt: new Date().toISOString() };
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

  const addSticker = useCallback(() => {
    onHistoryPush();
    const s: MindmapSticker = { id: `s${Date.now()}`, documentId: full.document.id, content: "", color: STICKER_PALETTE[Math.floor(Math.random() * STICKER_PALETTE.length)], positionX: 100, positionY: 100, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
    void mmApi.upsertSticker({ documentId: full.document.id, sticker: s });
    onDocumentUpdate({ ...full, stickers: [...full.stickers, s] });
  }, [full, onDocumentUpdate, onHistoryPush]);

  const deleteSelected = useCallback(() => {
    if (!selectedId) return;
    onHistoryPush();
    void mmApi.deleteNode({ documentId: full.document.id, nodeId: selectedId });
    onDocumentUpdate({ ...full, nodes: full.nodes.filter(n => n.id !== selectedId && n.parentId !== selectedId) });
    setSelectedId(null);
  }, [selectedId, full, onDocumentUpdate, onHistoryPush]);

  const relayout = useCallback(() => {
    const lp = layoutTree(full.nodes);
    const n2 = flowNodes.filter(n => !n.id.startsWith("sticker-")).map(n => ({ nodeId: n.id, x: lp.get(n.id)?.x ?? 0, y: lp.get(n.id)?.y ?? 0 }));
    void mmApi.updatePositions(full.document.id, n2);
    setPosOverrides(prev => {
      if (n2.length === 0) return prev;
      const next = { ...prev };
      for (const p of n2) next[p.nodeId] = { x: p.x, y: p.y };
      return next;
    });
    window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 30);
  }, [flowNodes, full, fitView]);

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
        onNodesChange={onNodesChange} onNodeDragStop={onNodeDragStop}
        onNodeContextMenu={(e, n) => onNodeContextMenu(e, n as Node)} minZoom={0.1} maxZoom={2.5} nodesConnectable={false}
        proOptions={{ hideAttribution: true }}>
        <Background color="#1e293b" gap={24} size={1} />
        <MiniMap style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95"
          nodeColor={(n) => { const d = n.data as FlowNodeData | StickerNodeData; return 'node' in d ? (d.node.color && d.node.color !== "#f59e0b" ? d.node.color : kindColor(d.node.kind)) : "#fef3c7"; }}
          nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2,6,23,0.72)" pannable zoomable />
        <Controls className="canvas-flow-controls" showInteractive={false} />
      </ReactFlow>

      {/* Compact floating toolbar */}
      <div className="absolute right-4 top-4 z-10 flex flex-col gap-1">
        <div className="rounded-lg border border-white/10 bg-slate-900/95 p-1 shadow-lg flex flex-col gap-0.5">
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={relayout} title="自动布局 (Ctrl+L)"><LayoutGrid className="h-3 w-3" />布局</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={() => addChildNode()} title="添加子节点 (Tab)"><Plus className="h-3 w-3" />子节点</button>
          <button type="button" className="inline-flex items-center gap-1.5 rounded px-2 py-1.5 text-[10px] text-slate-300 hover:bg-white/[0.08] hover:text-white" onClick={addSticker} title="添加贴纸"><StickyNote className="h-3 w-3" />贴纸</button>
        </div>
        {/* 自动保存指示 */}
        {lastSaved && (
          <div className="rounded-lg border border-emerald-400/20 bg-emerald-950/60 px-2 py-1 text-[8px] text-emerald-300 shadow-lg">
            ✓ 已自动保存 {new Date(lastSaved).toLocaleTimeString("zh-CN", { hour12: false })}
          </div>
        )}
        {/* Keyboard hints */}
        {selectedId && (
          <div className="rounded-lg border border-white/10 bg-slate-900/95 p-1.5 shadow-lg text-[8px] text-slate-600 leading-relaxed">
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Tab</kbd> 子节点</div>
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Enter</kbd> 详情</div>
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Del</kbd> 删除</div>
            <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Esc</kbd> 取消</div>
          </div>
        )}
      </div>

      {ctxMenu && (
        <div className="fixed z-50 min-w-[160px] rounded-lg border border-white/10 bg-[#101827] py-1 shadow-2xl" style={{ left: ctxMenu.x, top: ctxMenu.y }} onClick={e => e.stopPropagation()}>
          <div className="border-b border-white/10 px-3 py-1.5 text-[10px] font-semibold text-slate-400">{byId.get(ctxMenu.nodeId)?.name ?? ctxMenu.nodeId}</div>
          <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
            onClick={() => { const n = byId.get(ctxMenu.nodeId); if (n) setDetailNode(n); setCtxMenu(null); }}><Sparkles className="h-3.5 w-3.5" />查看详情</button>
          <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
            onClick={() => { addChildNode(ctxMenu.nodeId); setCtxMenu(null); }}><Plus className="h-3.5 w-3.5" />添加子节点</button>
          <div className="border-t border-white/10" />
          <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-red-300 transition hover:bg-white/[0.08] hover:text-red-100"
            onClick={() => { void mmApi.deleteNode({ documentId: full.document.id, nodeId: ctxMenu.nodeId });
              onDocumentUpdate({ ...full, nodes: full.nodes.filter(n => n.id !== ctxMenu.nodeId && n.parentId !== ctxMenu.nodeId) }); setCtxMenu(null); }}><Trash2 className="h-3.5 w-3.5" />删除</button>
        </div>)}
      {/* 悬浮只读预览气泡（跟随鼠标，pointer-events-none 不拦截交互） */}
      {preview && (() => {
        const pc = preview.node.color && preview.node.color !== "#f59e0b" ? preview.node.color : kindColor(preview.node.kind);
        const left = Math.min(preview.x + 14, window.innerWidth - 380);
        const top = Math.min(preview.y + 14, window.innerHeight - 340);
        return (
          <div className="pointer-events-none fixed z-[300]" style={{ left, top }}>
            <div className="w-[350px] rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl">
              <div className="flex items-center gap-1.5 border-b border-white/10 px-3 py-2">
                <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: pc, boxShadow: `0 0 6px ${pc}` }} />
                <span className="min-w-0 truncate text-[11px] font-semibold" style={{ color: pc }}>{preview.node.name}</span>
                <span className="ml-auto shrink-0 rounded border px-1 py-0.5 text-[8px] uppercase tracking-wide" style={{ borderColor: `${pc}44`, color: pc }}>{preview.node.kind}</span>
              </div>
              <div className="max-h-[260px] overflow-y-auto p-3">
                <MindmapMarkdown content={preview.node.detail || preview.node.description || "暂无内容"} />
              </div>
            </div>
          </div>
        );
      })()}
      {detailNode && <DetailModal node={detailNode} accent={accent} onUpdate={updateNode} onClose={() => setDetailNode(null)} />}
    </div>
  );
}

function Canvas({ full, accent, onDocumentUpdate, onHistoryPush, historyVersion }: { full: DocumentFull; accent: string; onDocumentUpdate: (d: DocumentFull) => void; onHistoryPush: () => void; historyVersion: number }) {
  return <div className="h-full min-h-0 bg-[#080f1c]"><ReactFlowProvider><CanvasInner full={full} accent={accent} onDocumentUpdate={onDocumentUpdate} onHistoryPush={onHistoryPush} historyVersion={historyVersion} /></ReactFlowProvider></div>;
}

// ════════════ 主面板 ════════════

function formatTime(v: string): string {
  if (!v) return ""; const d = new Date(v); if (Number.isNaN(d.getTime())) return v;
  return new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(d).replace(/\//g, "-");
}

export default function MindmapPanel() {
  const [docs, setDocs] = useState<MindmapDocument[]>([]);
  const [folders, setFolders] = useState<MindmapFolder[]>([]);
  const [activeFolderId, setActiveFolderId] = useState<string | null>(null);
  const [dragOverFolderId, setDragOverFolderId] = useState<string | null>(null);
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
  const [showAi, setShowAi] = useState<"project" | "text" | null>(null);
  const [textInput, setTextInput] = useState("");
  const [textTitle, setTextTitle] = useState("");
  const [projectPath, setProjectPath] = useState("");
  const [aiLoading, setAiLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

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
      try {
        const snapIds = new Set(snap.nodes.map(n => n.id));
        for (const n of cur.nodes) if (!snapIds.has(n.id)) { await mmApi.deleteNode({ documentId: snap.document.id, nodeId: n.id }).catch(() => {}); }
        for (const n of snap.nodes) { await mmApi.upsertNode({ documentId: snap.document.id, node: n }).catch(() => {}); }
        const snapStickers = new Set(snap.stickers.map(s => s.id));
        for (const s of cur.stickers) if (!snapStickers.has(s.id)) { await mmApi.deleteSticker({ documentId: snap.document.id, stickerId: s.id }).catch(() => {}); }
        for (const s of snap.stickers) { await mmApi.upsertSticker({ documentId: snap.document.id, sticker: s }).catch(() => {}); }
      } catch { /* 忽略持久化失败，本地已恢复 */ }
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
    }).catch(() => {});
    void invoke<AiConfig>("get_ai_config").then(cfg => {
      setConfig(cfg);
      const p = cfg.providers.find(x => x.api_key && x.openai_url) ?? cfg.providers[0];
      if (p) { setProviderId(p.id); setModelId(p.active_model_id ?? p.models[0]?.id ?? ""); }
    }).catch(() => setError("加载 AI 配置失败"));
  }, []);

  const providers = useMemo(() => (config?.providers ?? []).filter(p => p.api_key && p.openai_url), [config]);

  const loadDocument = useCallback(async (id: string) => {
    setError("");
    try {
      const f = await mmApi.load(id);
      if (f) { setFull(f); } else { flash("文档不存在"); }
    } catch (e) { setError(String(e)); }
  }, [flash]);

  const createDoc = useCallback(async (name: string, desc: string, sourceType: string, folderId: string | null) => {
    try {
      const doc = await mmApi.create({ name, description: desc, sourceType, folderId });
      setDocs(prev => [doc, ...prev]);
      if (!folderId) setActiveFolderId(null);
      setShowCreate(false);
      if (sourceType === "ai_project") { setShowAi("project"); setFull(null); }
      else if (sourceType === "ai_text") { setShowAi("text"); setFull(null); }
      else {
        const f = await mmApi.load(doc.id);
        if (f) setFull(f);
      }
      void refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [flash, refreshFolders]);

  const removeDoc = useCallback(async (id: string) => {
    if (!confirm("确定删除此思维导图？")) return;
    try { await mmApi.remove(id); setDocs(prev => prev.filter(d => d.id !== id)); if (full?.document.id === id) setFull(null); refreshFolders(); } catch (e) { flash(String(e)); }
  }, [full, flash]);

  const createFolder = useCallback(async () => {
    if (!folderName.trim()) return;
    try {
      await mmApi.createFolder({ name: folderName.trim() });
      setShowFolderCreate(false); setFolderName("");
      await refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [folderName, flash, refreshFolders]);

  const updateFolder = useCallback(async () => {
    if (!editingFolder || !folderName.trim()) return;
    try {
      await mmApi.updateFolder({ id: editingFolder.id, name: folderName.trim() });
      setEditingFolder(null); setFolderName("");
      await refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [editingFolder, folderName, flash, refreshFolders]);

  const deleteFolder = useCallback(async (id: string) => {
    if (!confirm("确定删除此文件夹？（文档不会被删除，将移至根目录）")) return;
    try { await mmApi.deleteFolder(id); if (activeFolderId === id) setActiveFolderId(null); await refreshFolders(); await loadDocs(); } catch (e) { flash(String(e)); }
  }, [activeFolderId, flash, refreshFolders]);

  const moveDoc = useCallback(async (docId: string, fid: string | null) => {
    try {
      await mmApi.moveDocument({ documentId: docId, folderId: fid });
      await Promise.all([loadDocs(), refreshFolders()]);
    } catch (e) { flash(String(e)); }
  }, [flash]);

  const loadDocs = useCallback(async () => {
    try { setDocs(await mmApi.list(activeFolderId)); } catch {}
  }, [activeFolderId]);

  useEffect(() => { void loadDocs(); }, [activeFolderId, loadDocs]);

  const runAiProject = useCallback(async () => {
    if (!full || !projectPath || !providerId) return;
    setAiLoading(true); setError("");
    try {
      const f = await mmApi.aiFromProject({ documentId: full.document.id, projectPath, providerId: providerId || null, modelId: modelId || null });
      setFull(f); setDocs(prev => prev.map(d => d.id === f.document.id ? f.document : d));
      setShowAi(null); flash(`已生成 ${f.nodes.length} 个节点`);
    } catch (e) { setError(String(e)); } finally { setAiLoading(false); }
  }, [full, projectPath, providerId, modelId, flash]);

  const runAiText = useCallback(async () => {
    if (!full || !textInput.trim() || !providerId) return;
    setAiLoading(true); setError("");
    try {
      const f = await mmApi.aiFromText({ documentId: full.document.id, text: textInput, title: textTitle || full.document.name, providerId: providerId || null, modelId: modelId || null });
      setFull(f); setDocs(prev => prev.map(d => d.id === f.document.id ? f.document : d));
      setShowAi(null); flash(`已提取 ${f.nodes.length} 个节点`);
    } catch (e) { setError(String(e)); } finally { setAiLoading(false); }
  }, [full, textInput, textTitle, providerId, modelId, flash]);

  const exportMd = useCallback(async () => {
    if (!full) return;
    try {
      const md = await mmApi.exportMd(full.document.id);
      const fp = await save({ defaultPath: `${full.document.name}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
      if (!fp) return;
      await invoke("write_text_file", { path: fp, content: md });
      flash(`已导出到 ${fp}`);
    } catch (e) { flash(String(e)); }
  }, [full, flash]);

  // Filtered documents
  const filteredDocs = useMemo(() => {
    if (!search.trim()) return docs;
    const q = search.toLowerCase();
    return docs.filter(d => d.name.toLowerCase().includes(q) || (d.description && d.description.toLowerCase().includes(q)));
  }, [docs, search]);

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-slate-950/25 text-slate-200">
      <header className="flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-white/10 px-3">
        <button type="button" className="inline-flex h-7 w-7 items-center justify-center rounded text-slate-400 hover:bg-white/10 hover:text-white"
          onClick={() => setSidebarCollapsed(!sidebarCollapsed)} title={sidebarCollapsed ? "展开侧栏" : "收起侧栏"}>
          {sidebarCollapsed ? <ChevronRight className="h-4 w-4" /> : <ChevronDown className="h-4 w-4 -rotate-90" />}
        </button>
        <Brain className="h-4 w-4" style={{ color: ACCENT }} />
        <span className="text-sm font-semibold text-white">思维导图</span>
        <button type="button" className={`${button} border-cyan-400/30 text-cyan-300 hover:bg-cyan-400/10 hover:text-cyan-200`} onClick={() => setShowCreate(true)}><Plus className="h-3 w-3" />新建</button>
        <div className="ml-auto flex items-center gap-1.5">
          {full && <button type="button" className={button} onClick={exportMd}><ScrollText className="h-3 w-3" />导出</button>}
          <span className="text-[10px] text-slate-500">{full ? `${full.document.name} · ${full.nodes.length} 节点` : "选择一个文档"}</span>
        </div>
      </header>
      <div className="flex min-h-0 flex-1">
        {!sidebarCollapsed && (
          <aside className="flex w-[250px] shrink-0 flex-col border-r border-white/10 bg-slate-950/30">
            {/* Search */}
            <div className="border-b border-white/10 px-2 py-1.5 flex items-center gap-1.5">
              <Search className="h-3 w-3 shrink-0 text-slate-600" />
              <input className="min-w-0 flex-1 bg-transparent text-[10px] text-slate-300 outline-none placeholder:text-slate-700" value={search} onChange={(e) => setSearch(e.target.value)} placeholder="搜索文档..." />
              {search && <button type="button" className="text-slate-600 hover:text-white" onClick={() => setSearch("")}><X className="h-3 w-3" /></button>}
            </div>
            {/* Folders —— 始终显示，保证随时可新建/重命名文件夹 */}
            {!activeFolderId && (
              <div className="border-b border-white/10 px-2 py-1.5 flex items-center justify-between">
                <span className="text-[9px] font-semibold text-slate-500 uppercase tracking-wider">文件夹</span>
                <button type="button" className="inline-flex h-5 w-5 items-center justify-center rounded text-slate-600 hover:bg-white/10 hover:text-white" onClick={() => { setShowFolderCreate(true); setFolderName(""); }} title="新建文件夹"><FolderPlus className="h-3 w-3" /></button>
              </div>
            )}
            {!activeFolderId && folders.length === 0 && (
              <div className="px-3 py-1 text-[9px] text-slate-700">暂无文件夹，点击右上角 + 新建</div>
            )}
            <div className="min-h-0 flex-1 overflow-y-auto p-1.5">
              {/* Folder list —— 可拖拽文档到文件夹归类 */}
              {!activeFolderId && folders.map(f => {
                const over = dragOverFolderId === f.id;
                return (
                <div key={f.id} className={`group mb-0.5 flex items-center gap-1.5 rounded-md px-2 py-1.5 transition ${over ? "ring-1 ring-cyan-400/70 bg-cyan-400/10" : "hover:bg-white/[0.05]"}`}
                  onDragOver={e => { e.preventDefault(); e.dataTransfer.dropEffect = "move"; }}
                  onDragEnter={() => setDragOverFolderId(f.id)}
                  onDragLeave={e => { if (!e.currentTarget.contains(e.relatedTarget as unknown as globalThis.Node)) setDragOverFolderId(null); }}
                  onDrop={e => { e.preventDefault(); setDragOverFolderId(null); const id = e.dataTransfer.getData("text/x-doc-id"); if (id) void moveDoc(id, f.id); }}>
                  <button type="button" className="flex min-w-0 flex-1 items-center gap-1.5 text-left" onClick={() => setActiveFolderId(f.id)}>
                    <Folder className="h-3.5 w-3.5 shrink-0 text-amber-400" />
                    <span className="truncate text-[10px] text-slate-300">{f.name}</span>
                    <span className="shrink-0 text-[9px] text-slate-600">{f.documentCount}</span>
                  </button>
                  <button type="button" className="hidden h-5 w-5 items-center justify-center rounded text-slate-600 group-hover:flex hover:bg-white/10 hover:text-white"
                    onClick={() => { setEditingFolder(f); setFolderName(f.name); }}><Pencil className="h-3 w-3" /></button>
                  <button type="button" className="hidden h-5 w-5 items-center justify-center rounded text-red-400/50 group-hover:flex hover:bg-red-400/10 hover:text-red-300"
                    onClick={() => void deleteFolder(f.id)}><Trash2 className="h-3 w-3" /></button>
                </div>
              );})}
              {/* Breadcrumb —— 也可作为拖回根目录的投放目标 */}
              {activeFolderId && (
                <button type="button" className={`mb-1 flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-[10px] ${dragOverFolderId === "__root" ? "ring-1 ring-cyan-400/70 bg-cyan-400/10 text-white" : "text-slate-400 hover:bg-white/[0.05] hover:text-white"}`}
                  onClick={() => setActiveFolderId(null)}
                  onDragOver={e => { e.preventDefault(); e.dataTransfer.dropEffect = "move"; }}
                  onDragEnter={() => setDragOverFolderId("__root")}
                  onDragLeave={e => { if (!e.currentTarget.contains(e.relatedTarget as unknown as globalThis.Node)) setDragOverFolderId(null); }}
                  onDrop={e => { e.preventDefault(); setDragOverFolderId(null); const id = e.dataTransfer.getData("text/x-doc-id"); if (id) void moveDoc(id, null); }}>
                  <ChevronRight className="h-3 w-3 -rotate-180" />← 全部文档（拖到此处移回根目录）
                </button>
              )}
              {/* Documents */}
              {filteredDocs.length === 0 && <div className="py-8 text-center text-[10px] text-slate-600">{search ? "无匹配文档" : "暂无文档"}</div>}
              {filteredDocs.map(d => {
                const IconFn = DOC_SOURCE_ICONS[d.sourceType] ?? DOC_SOURCE_ICONS.manual;
                return (
                <div key={d.id} className={`group mb-1 flex items-center gap-1.5 rounded-md border px-2.5 py-2 transition ${full?.document.id === d.id ? "border-cyan-400/40 bg-cyan-400/10" : "border-white/10 bg-white/[0.02] hover:bg-white/[0.06]"}`}
                  draggable
                  onDragStart={e => { e.dataTransfer.setData("text/x-doc-id", d.id); e.dataTransfer.effectAllowed = "move"; }}
                  onDragEnd={() => setDragOverFolderId(null)}>
                  <button type="button" className="flex min-w-0 flex-1 items-center gap-2 text-left" onClick={() => void loadDocument(d.id)}>
                    {IconFn("h-3.5 w-3.5 shrink-0 text-slate-500")}
                    <span className="flex min-w-0 flex-col">
                      <span className="truncate text-[11px] text-slate-200">{d.name}</span>
                      <span className="text-[9px] text-slate-600">{formatTime(d.updatedAt)}</span>
                    </span>
                  </button>
                  {activeFolderId && <button type="button" className="hidden shrink-0 text-[8px] text-slate-600 group-hover:block hover:text-slate-400" onClick={() => moveDoc(d.id, null)}>移出</button>}
                  <button type="button" className="hidden h-5 w-5 shrink-0 items-center justify-center rounded text-red-300/60 transition group-hover:flex hover:bg-red-400/10 hover:text-red-200" onClick={() => void removeDoc(d.id)} title="删除"><Trash2 className="h-3 w-3" /></button>
                </div>
              );})}
            </div>
          </aside>
        )}
        <main className="relative min-w-0 flex-1">
          {full ? (
            <Canvas full={full} accent={ACCENT} onDocumentUpdate={onDocumentUpdated} onHistoryPush={commitHistory} historyVersion={historyVersion} />
          ) : showAi === "project" ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 p-6">
              <div className="flex w-full max-w-[500px] flex-col gap-3">
                <div className="flex items-center gap-2"><Lightbulb className="h-5 w-5" style={{ color: ACCENT }} /><span className="text-sm text-white font-semibold">从项目学习</span></div>
                <div className="flex gap-2">
                  <button type="button" className={button} onClick={async () => { const d = await openDialog({ directory: true, multiple: false, title: "选择项目目录" }); if (typeof d === "string") setProjectPath(d); }}>
                    <FolderOpen className="h-3 w-3" />{projectPath ? projectPath.split(/[\\\\/]/).pop() : "选择目录"}
                  </button>
                  <select className={selectClass} value={providerId} onChange={e => { setProviderId(e.target.value); setModelId(""); }}>
                    {providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
                  </select>
                </div>
                <button type="button" className="w-full rounded-lg py-2 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }}
                  disabled={!projectPath || !providerId || aiLoading} onClick={() => void runAiProject()}>
                  {aiLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin inline mr-1" /> : <Sparkles className="h-3.5 w-3.5 inline mr-1" />}
                  {aiLoading ? "AI 分析中…" : "AI 分析项目结构"}
                </button>
                <button type="button" className="text-[10px] text-slate-500 hover:text-white" onClick={() => setShowAi(null)}>跳过，手动编辑</button>
              </div>
            </div>
          ) : showAi === "text" ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 p-6">
              <div className="flex w-full max-w-[500px] flex-col gap-3">
                <div className="flex items-center gap-2"><Lightbulb className="h-5 w-5" style={{ color: ACCENT }} /><span className="text-sm text-white font-semibold">AI 析需求</span></div>
                <input className="h-9 w-full rounded-xl bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none" value={textTitle} onChange={e => setTextTitle(e.target.value)} placeholder="需求标题" />
                <textarea className="w-full h-40 rounded-xl bg-slate-900 border border-white/10 px-3 py-2 text-xs text-white outline-none resize-none" value={textInput} onChange={e => setTextInput(e.target.value)} placeholder="粘贴需求文本…" />
                <select className={selectClass} value={providerId} onChange={e => { setProviderId(e.target.value); setModelId(""); }}>
                  {providers.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}
                </select>
                <button type="button" className="w-full rounded-lg py-2 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }}
                  disabled={!textInput.trim() || !providerId || aiLoading} onClick={() => void runAiText()}>
                  {aiLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin inline mr-1" /> : <Sparkles className="h-3.5 w-3.5 inline mr-1" />}
                  {aiLoading ? "AI 提取中…" : "AI 提取需求"}
                </button>
                <button type="button" className="text-[10px] text-slate-500 hover:text-white" onClick={() => setShowAi(null)}>跳过，手动编辑</button>
              </div>
            </div>
          ) : (
            <div className="flex h-full flex-col items-center justify-center gap-4 text-slate-500">
              <div className="rounded-full border border-white/10 p-4"><Brain className="h-10 w-10" style={{ color: ACCENT }} /></div>
              <p className="text-[12px] font-medium text-slate-400">新建或选择一个思维导图</p>
              <p className="text-[10px] text-slate-600 max-w-xs text-center">点击左侧 + 新建空白导图，或通过 AI 从项目 / 需求文本自动生成</p>
            </div>
          )}
          {error && !showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-[11px] text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
        </main>
      </div>
      {showCreate && <CreateDocModal onClose={() => setShowCreate(false)} onCreate={(n,d,st,fid) => { void createDoc(n,d,st,fid); }} folderId={activeFolderId} />}
      {error && showAi && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-[11px] text-red-300 shadow-xl">{error}<button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button></div>}
      {notice && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
      {/* Folder create/edit modal */}
      {showFolderCreate && createPortal(
        <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]" onClick={() => setShowFolderCreate(false)}>
          <div className="w-[340px] rounded-xl border border-white/10 bg-[#0d1524] p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-4 text-sm font-semibold text-white">新建文件夹</h3>
            <input className="w-full h-9 rounded-lg bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none mb-4" value={folderName} onChange={(e) => setFolderName(e.target.value)} placeholder="文件夹名称" autoFocus onKeyDown={(e) => e.key === "Enter" && createFolder()} />
            <div className="flex justify-end gap-2">
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] text-slate-400 hover:text-white" onClick={() => setShowFolderCreate(false)}>取消</button>
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!folderName.trim()} onClick={createFolder}>创建</button>
            </div>
          </div>
        </div>, document.body)}
      {editingFolder && createPortal(
        <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]" onClick={() => setEditingFolder(null)}>
          <div className="w-[340px] rounded-xl border border-white/10 bg-[#0d1524] p-5 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-4 text-sm font-semibold text-white">重命名文件夹</h3>
            <input className="w-full h-9 rounded-lg bg-slate-900 border border-white/10 px-3 text-xs text-white outline-none mb-4" value={folderName} onChange={(e) => setFolderName(e.target.value)} autoFocus onKeyDown={(e) => e.key === "Enter" && updateFolder()} />
            <div className="flex justify-end gap-2">
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] text-slate-400 hover:text-white" onClick={() => setEditingFolder(null)}>取消</button>
              <button type="button" className="rounded-md px-4 py-1.5 text-[11px] font-semibold text-white disabled:opacity-40" style={{ backgroundColor: ACCENT }} disabled={!folderName.trim()} onClick={updateFolder}>保存</button>
            </div>
          </div>
        </div>, document.body)}
    </div>
  );
}