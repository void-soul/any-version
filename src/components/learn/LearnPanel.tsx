import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
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
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  FileText,
  FolderOpen,
  LayoutGrid,
  Lightbulb,
  Loader2,
  Network,
  ScrollText,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import type { AiConfig } from "../ai/types";
import {
  LearnGraph,
  LearnMeta,
  LearnNode,
  kindColor,
  learnApi,
} from "./types";
import { moduleAccent } from "../../utils/theme";

const LEARN_ACCENT = moduleAccent();

const button =
  "inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/[0.05] px-2 py-1.5 text-[10px] text-slate-300 transition hover:bg-white/[0.1] hover:text-white disabled:opacity-40";
const selectClass =
  "h-8 min-w-[110px] rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60";

type LearnFlowNodeData = {
  node: LearnNode;
  selected: boolean;
  hasChildren: boolean;
  collapsed: boolean;
  onSelect: () => void;
  onOpenDetail: () => void;
  onToggle: () => void;
};

const LearnFlowNode = memo(function LearnFlowNode({ data }: NodeProps<Node<LearnFlowNodeData>>) {
  const { node, selected, hasChildren, collapsed, onSelect, onOpenDetail, onToggle } = data;
  const color = kindColor(node.kind);
  return (
    <article
      className={`w-[220px] rounded-lg border bg-[#101827] px-2.5 py-2 shadow-xl transition ${
        selected ? "shadow-cyan-500/30" : ""
      }`}
      style={{ borderColor: selected ? color : `${color}66` }}
      onClick={onSelect}
      onDoubleClick={(e) => {
        e.stopPropagation();
        onOpenDetail();
      }}
    >
      <Handle type="target" position={Position.Left} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
      <div className="flex items-center gap-1.5">
        {hasChildren && (
          <button
            type="button"
            className="nodrag nopan inline-flex h-4 w-4 items-center justify-center text-slate-500 hover:text-white"
            onClick={(e) => {
              e.stopPropagation();
              onToggle();
            }}
            title={collapsed ? "展开" : "折叠"}
          >
            {collapsed ? "▸" : "▾"}
          </button>
        )}
        <span className="min-w-0 flex-1 truncate text-[11px] font-semibold" style={{ color }}>
          {node.name}
        </span>
      </div>
      {node.description && (
        <div className="mt-1 line-clamp-2 text-[9px] leading-4 text-slate-400">{node.description}</div>
      )}
      <div className="mt-1.5 inline-flex items-center rounded border px-1 py-0.5 text-[8px] uppercase tracking-wide" style={{ borderColor: `${color}44`, color }}>
        {node.kind}
      </div>
      <Handle type="source" position={Position.Right} isConnectable={false} className="!h-2.5 !w-2.5 !border-2 !border-slate-950" style={{ background: color }} />
    </article>
  );
});

const LearnColorEdge = memo(function LearnColorEdge({ id, sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, data }: EdgeProps) {
  const [path] = getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition, curvature: 0.28 });
  const color = (data?.color as string | undefined) ?? "#22d3ee";
  const gradientId = `learn-edge-${id.replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  return (
    <>
      <defs>
        <linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="0%">
          <stop offset="0%" stopColor={color} />
          <stop offset="100%" stopColor="#f8fafc" />
        </linearGradient>
      </defs>
      <path d={path} fill="none" stroke={color} strokeWidth={3} opacity={0.12} />
      <path d={path} fill="none" stroke={`url(#${gradientId})`} strokeWidth={1.5} strokeLinecap="round" markerEnd={`url(#arrow-${gradientId})`} />
      <marker id={`arrow-${gradientId}`} markerWidth="6" markerHeight="6" refX="5" refY="3" orient="auto">
        <path d="M0,0 L6,3 L0,6 z" fill="#f8fafc" />
      </marker>
    </>
  );
});

function layoutTree(nodes: LearnNode[]): Map<string, { x: number; y: number }> {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const n of nodes) {
    if (n.parentId && byId.has(n.parentId)) {
      const list = children.get(n.parentId) ?? [];
      list.push(n.id);
      children.set(n.parentId, list);
    } else {
      roots.push(n.id);
    }
  }
  const depth = new Map<string, number>();
  const order: string[] = [];
  const dfs = (id: string, d: number) => {
    depth.set(id, d);
    order.push(id);
    for (const c of children.get(id) ?? []) dfs(c, d + 1);
  };
  for (const r of roots) dfs(r, 0);
  const pos = new Map<string, { x: number; y: number }>();
  order.forEach((id, i) => pos.set(id, { x: (depth.get(id) ?? 0) * 260, y: i * 88 }));
  return pos;
}

function LearnDetailModal({ graph, node, onClose }: { graph: LearnGraph; node: LearnNode; onClose: () => void }) {
  const color = kindColor(node.kind);
  return createPortal(
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/70 p-6 backdrop-blur-[3px]" onClick={onClose}>
      <div
        className="flex h-[80vh] w-[min(92vw,720px)] flex-col overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex h-11 shrink-0 items-center gap-2 border-b border-white/10 px-3" style={{ backgroundColor: `${color}1f` }}>
          <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color, boxShadow: `0 0 9px ${color}` }} />
          <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-slate-100">
            {graph.projectName} · {node.name}
          </span>
          <span className="rounded border px-1.5 py-0.5 text-[9px] uppercase" style={{ borderColor: `${color}44`, color }}>
            {node.kind}
          </span>
          <button type="button" className="nodrag nopan ml-1 text-slate-500 hover:text-white" onClick={onClose} title="关闭">
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {node.description && (
            <p className="mb-3 rounded border border-white/10 bg-white/[0.03] px-3 py-2 text-[11px] leading-5 text-slate-300">{node.description}</p>
          )}
          <div className="text-[11px] leading-relaxed text-slate-200 break-words">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{node.detail || node.description || "暂无详细说明"}</ReactMarkdown>
          </div>
        </div>
      </div>
    </div>,
    document.body
  );
}

function LearnCanvasInner({ graph, accent, onRegenerateNode, regeneratingNodeId, projectPath }: { graph: LearnGraph; accent: string; onRegenerateNode: (nodeId: string) => void; regeneratingNodeId: string | null; projectPath: string }) {
  const { fitView } = useReactFlow();
  const [nodes, setNodes] = useState<Node<LearnFlowNodeData>[]>([]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detailNode, setDetailNode] = useState<LearnNode | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; nodeId: string } | null>(null);

  const byId = useMemo(() => new Map(graph.nodes.map((n) => [n.id, n])), [graph.nodes]);
  const layout = useMemo(() => layoutTree(graph.nodes), [graph.nodes]);
  const childrenCount = useMemo(() => {
    const m = new Map<string, number>();
    for (const n of graph.nodes) {
      if (n.parentId) m.set(n.parentId, (m.get(n.parentId) ?? 0) + 1);
    }
    return m;
  }, [graph.nodes]);

  const visibleNodes = useMemo(() => {
    const result: LearnNode[] = [];
    const visit = (n: LearnNode) => {
      result.push(n);
      if (!collapsed.has(n.id)) {
        graph.nodes.filter((c) => c.parentId === n.id).forEach(visit);
      }
    };
    graph.nodes.filter((n) => !n.parentId || !byId.has(n.parentId)).forEach(visit);
    return result;
  }, [graph.nodes, collapsed, byId]);

  const flowNodes = useMemo<Node<LearnFlowNodeData>[]>(() => visibleNodes.map((n) => {
    // 有已保存坐标（非零）时优先使用，否则回退到自动布局
    const hasSaved = n.positionX !== 0 || n.positionY !== 0;
    const p = hasSaved ? { x: n.positionX, y: n.positionY } : layout.get(n.id) ?? { x: 0, y: 0 };
    return {
      id: n.id,
      type: "learnNode",
      position: p,
      data: {
        node: n,
        selected: selectedId === n.id,
        hasChildren: (childrenCount.get(n.id) ?? 0) > 0,
        collapsed: collapsed.has(n.id),
        onSelect: () => setSelectedId(n.id),
        onOpenDetail: () => setDetailNode(n),
        onToggle: () => setCollapsed((cur) => { const next = new Set(cur); next.has(n.id) ? next.delete(n.id) : next.add(n.id); return next; }),
      },
      sourcePosition: Position.Right,
      targetPosition: Position.Left,
    };
  }), [visibleNodes, layout, selectedId, collapsed, childrenCount]);

  const edges = useMemo<Edge[]>(() => visibleNodes.flatMap((n) => {
    if (!n.parentId || !visibleNodes.some((p) => p.id === n.parentId)) return [];
    return [{
      id: `learn-edge-${n.id}`,
      source: n.parentId,
      target: n.id,
      type: "color",
      data: { color: kindColor(n.kind) },
      markerEnd: { type: MarkerType.ArrowClosed, color: "#f8fafc" },
    } as Edge];
  }), [visibleNodes]);

  useEffect(() => {
    setNodes((current) => {
      const curById = new Map(current.map((n) => [n.id, n]));
      return flowNodes.map((next) => {
        const existing = curById.get(next.id);
        return existing ? { ...next, position: existing.position } : next;
      });
    });
  }, [flowNodes]);

  const relayout = useCallback(() => {
    setNodes(flowNodes.map((n) => ({ ...n, position: layout.get(n.id) ?? { x: 0, y: 0 } })));
    // 自动布局后清空已保存坐标
    void learnApi.updatePositions(projectPath, flowNodes.map((n) => ({ nodeId: n.id, x: 0, y: 0 })));
    window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 30);
  }, [flowNodes, layout, fitView, projectPath]);

  const onNodeDragStop = useCallback(
    (_event: MouseEvent | TouchEvent, node: Node<LearnFlowNodeData>) => {
      const pos = { nodeId: node.id, x: node.position.x, y: node.position.y };
      void learnApi.updatePositions(projectPath, [pos]);
    },
    [projectPath]
  );

  useEffect(() => {
    const timer = window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 0);
    return () => window.clearTimeout(timer);
  }, [fitView, graph.projectPath, collapsed]);

  const onNodeContextMenu = useCallback(
    (event: React.MouseEvent, node: Node) => {
      event.preventDefault();
      const id = node.id;
      if (!id.startsWith("sticker-") && byId.has(id)) {
        setSelectedId(id);
        setCtxMenu({ x: event.clientX, y: event.clientY, nodeId: id });
      }
    },
    [byId]
  );

  useEffect(() => {
    const close = () => setCtxMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, []);

  return (
    <div className="relative h-full min-h-0">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={{ learnNode: LearnFlowNode }}
        edgeTypes={{ color: LearnColorEdge }}
        onNodesChange={(changes) => setNodes((cur) => applyNodeChanges(changes, cur))}
        onNodeDragStop={onNodeDragStop}
        onNodeContextMenu={(event, node) => onNodeContextMenu(event, node as Node)}
        fitView
        minZoom={0.15}
        maxZoom={2}
        nodesConnectable={false}
        proOptions={{ hideAttribution: true }}
      >
        <Background color="#1e293b" gap={24} size={1} />
        <MiniMap
          style={{ backgroundColor: "#080f1c", border: "1px solid rgba(255,255,255,.12)" }}
          className="!bg-slate-950/95"
          nodeColor={(n) => kindColor((n.data as LearnFlowNodeData).node.kind)}
          nodeStrokeColor="#0f172a"
          nodeBorderRadius={2}
          maskColor="rgba(2, 6, 23, 0.72)"
          pannable
          zoomable
        />
        <Controls className="canvas-flow-controls" showInteractive={false} />
      </ReactFlow>
      <button
        type="button"
        className="absolute right-4 top-4 z-10 inline-flex items-center gap-1 rounded-md border border-white/10 bg-slate-900/90 px-2 py-1.5 text-[10px] text-slate-300 shadow hover:bg-slate-800 hover:text-white"
        onClick={relayout}
        title="重新自动布局并适配视口"
        style={{ borderColor: `${accent}55` }}
      >
        <LayoutGrid className="h-3 w-3" />自动布局
      </button>

      {/* 右键菜单 */}
      {ctxMenu && (
        <div
          className="fixed z-50 min-w-[180px] rounded-lg border border-white/10 bg-[#101827] py-1 shadow-2xl"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="border-b border-white/10 px-3 py-1.5 text-[10px] font-semibold text-slate-400">
            {byId.get(ctxMenu.nodeId)?.name ?? ctxMenu.nodeId}
          </div>
          <button
            type="button"
            className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
            onClick={() => {
              const n = byId.get(ctxMenu.nodeId);
              if (n) setDetailNode(n);
              setCtxMenu(null);
            }}
          >
            <Sparkles className="h-3.5 w-3.5" />查看详情
          </button>
          <button
            type="button"
            className="flex w-full items-center gap-2 px-3 py-2 text-[11px] text-cyan-300 transition hover:bg-white/[0.08] hover:text-cyan-100 disabled:opacity-30"
            disabled={regeneratingNodeId !== null}
            onClick={() => {
              onRegenerateNode(ctxMenu.nodeId);
              setCtxMenu(null);
            }}
          >
            {regeneratingNodeId === ctxMenu.nodeId ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Network className="h-3.5 w-3.5" />
            )}
            重新分析此子树
          </button>
        </div>
      )}

      {/* 正在重新分析的节点蒙层 */}
      {regeneratingNodeId && (
        <div className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center bg-black/20 backdrop-blur-[1px]">
          <div className="rounded-lg border border-cyan-400/30 bg-slate-900/95 px-4 py-2 text-[11px] text-cyan-200 shadow-xl">
            <Loader2 className="mr-2 inline-block h-3.5 w-3.5 animate-spin" />
            正在重新分析「{byId.get(regeneratingNodeId)?.name ?? regeneratingNodeId}」的子结构…
          </div>
        </div>
      )}
      {detailNode && <LearnDetailModal graph={graph} node={detailNode} onClose={() => setDetailNode(null)} />}
    </div>
  );
}

function LearnCanvas({ graph, accent, onRegenerateNode, regeneratingNodeId }: { graph: LearnGraph; accent: string; onRegenerateNode: (nodeId: string) => void; regeneratingNodeId: string | null }) {
  return (
    <div className="h-full min-h-0 bg-[#080f1c]">
      <ReactFlowProvider>
        <LearnCanvasInner graph={graph} accent={accent} onRegenerateNode={onRegenerateNode} regeneratingNodeId={regeneratingNodeId} projectPath={graph.projectPath} />
      </ReactFlowProvider>
    </div>
  );
}

function formatTime(value: string): string {
  if (!value) return "";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(d).replace(/\//g, "-");
}

export default function LearnPanel() {
  const [metas, setMetas] = useState<LearnMeta[]>([]);
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [projectPath, setProjectPath] = useState("");
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");
  const [current, setCurrent] = useState<LearnGraph | null>(null);
  const [generating, setGenerating] = useState(false);
  const [regeneratingNodeId, setRegeneratingNodeId] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const loadedRef = useRef(false);

  const [activeMode, setActiveMode] = useState<"project" | "text">("project");
  const [textInput, setTextInput] = useState("");
  const [textTitle, setTextTitle] = useState("");

  const flash = useCallback((msg: string) => {
    setNotice(msg);
    window.setTimeout(() => setNotice(""), 2600);
  }, []);

  const loadMetas = useCallback(async () => {
    try {
      setMetas(await learnApi.list());
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    if (loadedRef.current) return;
    loadedRef.current = true;
    void loadMetas();
    void invoke<AiConfig>("get_ai_config")
      .then((cfg) => {
        setConfig(cfg);
        const provider = cfg.providers.find((p) => !p.api_key && !p.openai_url ? false : !p.openai_url || !p.api_key ? false : true) ?? cfg.providers[0];
        if (provider) {
          setProviderId(provider.id);
          setModelId(provider.active_model_id ?? provider.models[0]?.id ?? "");
        }
      })
      .catch(() => setError("加载 AI 配置失败，请先在 AI 模块配置供应商与模型"));
  }, [loadMetas]);

  const providers = useMemo(() => (config?.providers ?? []).filter((p) => !p.openai_url.trim() || !p.api_key.trim() ? false : true), [config]);
  const activeProvider = useMemo(() => providers.find((p) => p.id === providerId) ?? null, [providers, providerId]);

  const pickDirectory = useCallback(async () => {
    const selected = await openDialog({ directory: true, multiple: false, title: "选择要学习的项目目录" });
    if (typeof selected === "string") setProjectPath(selected);
  }, []);

  const generateFromText = useCallback(async () => {
    if (!textInput.trim()) { flash("请输入需求文本"); return; }
    if (!providerId) { flash("请选择 AI 供应商"); return; }
    setGenerating(true);
    setError("");
    try {
      const title = textTitle.trim() || "需求分析";
      const graph = await learnApi.generateFromText({ text: textInput, title, providerId: providerId || null, modelId: modelId || null });
      setCurrent(graph);
      setMetas((prev) => {
        const meta: LearnMeta = { projectPath: graph.projectPath, projectName: graph.projectName, generatedAt: graph.generatedAt, nodeCount: graph.nodes.length };
        const rest = prev.filter((m) => m.projectPath !== graph.projectPath);
        return [meta, ...rest];
      });
      flash(`已提取 ${graph.nodes.length} 个节点的需求结构`);
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
    }
  }, [textInput, textTitle, providerId, modelId, flash]);

  const generate = useCallback(async () => {
    if (!projectPath.trim()) { flash("请先选择项目目录"); return; }
    if (!providerId) { flash("请选择 AI 供应商"); return; }
    setGenerating(true);
    setError("");
    try {
      const graph = await learnApi.generate({ projectPath, providerId: providerId || null, modelId: modelId || null });
      setCurrent(graph);
      setMetas((prev) => {
        const meta: LearnMeta = { projectPath: graph.projectPath, projectName: graph.projectName, generatedAt: graph.generatedAt, nodeCount: graph.nodes.length };
        const rest = prev.filter((m) => m.projectPath !== graph.projectPath);
        return [meta, ...rest];
      });
      flash(`已生成 ${graph.nodes.length} 个节点的结构`);
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
    }
  }, [projectPath, providerId, modelId, flash]);

  const regenerateNode = useCallback(async (nodeId: string) => {
    if (!current || !providerId) return;
    setRegeneratingNodeId(nodeId);
    setError("");
    try {
      const graph = await learnApi.regenerateNode({ projectPath: current.projectPath, nodeId, providerId: providerId || null, modelId: modelId || null });
      setCurrent(graph);
      flash(`已重新分析子树，新增 ${graph.nodes.length} 个节点`);
    } catch (e) {
      setError(String(e));
    } finally {
      setRegeneratingNodeId(null);
    }
  }, [current, providerId, modelId, flash]);

  const loadGraph = useCallback(async (path: string) => {
    setError("");
    try {
      const graph = await learnApi.load(path);
      if (graph) {
        setCurrent(graph);
        setProjectPath(path);
      } else {
        flash("该记录不存在，请重新生成");
      }
    } catch (e) {
      setError(String(e));
    }
  }, [flash]);

  const exportMarkdown = useCallback(async () => {
    if (!current) return;
    try {
      const md = await learnApi.exportMarkdown(current.projectPath);
      const filePath = await save({ defaultPath: `${current.projectName}.md`, filters: [{ name: "Markdown", extensions: ["md"] }] });
      if (!filePath) return;
      await invoke("write_text_file", { path: filePath, content: md });
      flash(`已导出到 ${filePath}`);
    } catch (e) {
      flash(`导出失败：${String(e)}`);
    }
  }, [current, flash]);

  const removeGraph = useCallback(async (meta: LearnMeta) => {
    try {
      await learnApi.remove(meta.projectPath);
      setMetas((prev) => prev.filter((m) => m.projectPath !== meta.projectPath));
      if (current?.projectPath === meta.projectPath) setCurrent(null);
    } catch (e) {
      flash(`删除失败：${String(e)}`);
    }
  }, [current, flash]);

  return (
    <div className="relative flex h-full min-h-0 flex-col bg-slate-950/25 text-slate-200">
      <header className="flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-white/10 px-3">
        <Lightbulb className="h-4 w-4" style={{ color: LEARN_ACCENT }} />
        <span className="text-sm font-semibold text-white">需求分析</span>

        {/* 模式切换 */}
        <div className="flex rounded-lg border border-white/10 bg-slate-950/60 p-0.5">
          <button
            type="button"
            className={`rounded-md px-2.5 py-1 text-[10px] transition ${activeMode === "project" ? "bg-white/10 text-white font-semibold" : "text-slate-400 hover:text-white"}`}
            onClick={() => setActiveMode("project")}
          >
            <FolderOpen className="inline h-3 w-3 mr-1" />项目
          </button>
          <button
            type="button"
            className={`rounded-md px-2.5 py-1 text-[10px] transition ${activeMode === "text" ? "bg-white/10 text-white font-semibold" : "text-slate-400 hover:text-white"}`}
            onClick={() => setActiveMode("text")}
          >
            <FileText className="inline h-3 w-3 mr-1" />文本
          </button>
        </div>

        {activeMode === "project" && (
          <button type="button" className={button} onClick={() => void pickDirectory()} title="选择项目目录">
            <FolderOpen className="h-3 w-3" />{projectPath ? projectPath.split(/[\\/]/).pop() || "已选择目录" : "选择目录"}
          </button>
        )}
        <select className={selectClass} value={providerId} onChange={(e) => { setProviderId(e.target.value); setModelId(""); }} title="AI 供应商">
          <option value="">选择供应商</option>
          {providers.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
        <select className={selectClass} value={modelId} onChange={(e) => setModelId(e.target.value)} title="模型">
          <option value="">默认模型</option>
          {activeProvider?.models.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
        </select>
        <button
          type="button"
          className="inline-flex items-center gap-1 rounded-md px-3 py-1.5 text-[10px] font-semibold text-white disabled:opacity-40"
          style={{ backgroundColor: LEARN_ACCENT }}
          disabled={generating || regeneratingNodeId !== null || !providerId || (activeMode === "project" && !projectPath) || (activeMode === "text" && !textInput.trim())}
          onClick={() => void (activeMode === "text" ? generateFromText() : generate())}
        >
          {generating ? <Loader2 className="h-3 w-3 animate-spin" /> : <Sparkles className="h-3 w-3" />}
          {generating ? "分析中…" : activeMode === "text" ? "提取需求" : "生成结构"}
        </button>
        <div className="ml-auto flex items-center gap-1.5 text-[10px] text-slate-500">
          {current && <button type="button" className={button} onClick={() => void exportMarkdown()} title="导出为 Markdown 文档"><ScrollText className="h-3 w-3" />导出文档</button>}
          <span>{current ? `${current.projectName} · ${current.nodes.length} 节点` : ""}</span>
        </div>
      </header>
      {activeMode === "project" && projectPath && <div className="truncate border-b border-white/5 px-3 py-1 font-mono text-[9px] text-slate-600">{projectPath}</div>}
      <div className="flex min-h-0 flex-1">
        <aside className="flex w-[230px] shrink-0 flex-col border-r border-white/10 bg-slate-950/30">
          <div className="border-b border-white/10 px-3 py-2 text-[10px] font-semibold text-slate-400">分析历史</div>
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {metas.length === 0 && <div className="py-8 text-center text-[10px] text-slate-600">暂无分析记录</div>}
            {metas.map((meta) => (
              <div
                key={meta.projectPath}
                className={`mb-1.5 rounded-md border px-2.5 py-2 transition ${current?.projectPath === meta.projectPath ? "border-cyan-400/40 bg-cyan-400/10" : "border-white/10 bg-white/[0.02] hover:bg-white/[0.06]"}`}
              >
                <button type="button" className="w-full text-left" onClick={() => void loadGraph(meta.projectPath)}>
                  <div className="truncate text-[11px] text-slate-200">{meta.projectName}</div>
                  <div className="mt-0.5 text-[9px] text-slate-500">{meta.nodeCount} 节点 · {formatTime(meta.generatedAt)}</div>
                </button>
                <div className="mt-1.5 flex justify-end border-t border-white/[0.06] pt-1">
                  <button type="button" className="inline-flex h-5 w-5 items-center justify-center rounded text-red-300/60 transition hover:bg-red-400/10 hover:text-red-200" onClick={() => void removeGraph(meta)} title="删除记录">
                    <Trash2 className="h-3 w-3" />
                  </button>
                </div>
              </div>
            ))}
          </div>
        </aside>
        <main className="relative min-w-0 flex-1">
          {current ? (
            <LearnCanvas graph={current} accent={LEARN_ACCENT} onRegenerateNode={regenerateNode} regeneratingNodeId={regeneratingNodeId} />
          ) : activeMode === "text" ? (
            <div className="flex h-full flex-col items-center p-6">
              <div className="flex w-full max-w-[700px] flex-1 flex-col gap-3">
                <div className="flex items-center gap-2">
                  <Lightbulb className="h-5 w-5" style={{ color: LEARN_ACCENT }} />
                  <span className="text-sm text-white font-semibold">需求提取</span>
                  <span className="text-[10px] text-slate-500">粘贴会议纪要、需求文档、聊天记录等任意文本，AI 自动提取为结构化需求树</span>
                </div>
                <input
                  type="text"
                  value={textTitle}
                  onChange={(e) => setTextTitle(e.target.value)}
                  placeholder="需求标题（如「用户中心重构」「支付模块 V2」）"
                  className="h-9 w-full rounded-xl bg-slate-900 border border-white/10 px-3 text-xs text-white placeholder:text-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
                />
                <textarea
                  value={textInput}
                  onChange={(e) => setTextInput(e.target.value)}
                  placeholder={`粘贴需求文本…

示例：
「用户可以在个人中心查看订单列表、修改密码、绑定手机号。
管理员可以在后台查看所有订单并导出 Excel。
系统需要支持短信验证码登录，密码必须至少 8 位包含大小写字母和数字。
接口响应时间 P99 不能超过 200ms，需要支持 1000 QPS 的并发。」`}
                  rows={14}
                  className="flex-1 w-full min-h-[300px] rounded-xl bg-slate-900 border border-white/10 px-3 py-3 text-xs text-white placeholder:text-slate-500 focus:outline-none focus:border-[var(--module-accent)] resize-none"
                />
                <div className="flex items-center justify-between">
                  <span className="text-[9px] text-slate-600">{textInput.length} 字符</span>
                  <button
                    type="button"
                    className="inline-flex items-center gap-1 rounded-md px-4 py-2 text-[11px] font-semibold text-white disabled:opacity-40"
                    style={{ backgroundColor: LEARN_ACCENT }}
                    disabled={generating || !textInput.trim() || !providerId}
                    onClick={() => void generateFromText()}
                  >
                    {generating ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Sparkles className="h-3.5 w-3.5" />}
                    {generating ? "AI 提取中…" : "AI 提取需求"}
                  </button>
                </div>
                {error && <p className="rounded border border-red-400/20 bg-red-400/5 px-3 py-2 text-[11px] text-red-300">{error}</p>}
              </div>
            </div>
          ) : (
            <div className="flex h-full flex-col items-center justify-center gap-3 text-slate-500">
              <Lightbulb className="h-10 w-10" style={{ color: LEARN_ACCENT }} />
              <p className="text-[12px]">选择项目目录分析代码结构，或粘贴文本提取需求</p>
              {error && <p className="max-w-md rounded border border-red-400/20 bg-red-400/5 px-3 py-2 text-[11px] text-red-300">{error}</p>}
            </div>
          )}
        </main>
      </div>
      {notice && <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 rounded-md border border-white/10 bg-slate-900 px-3 py-2 text-[11px] text-slate-200 shadow-xl">{notice}</div>}
      {error && current && (
        <div className="absolute bottom-8 left-1/2 z-40 -translate-x-1/2 max-w-md rounded-md border border-red-400/20 bg-slate-900 px-3 py-2 text-[11px] text-red-300 shadow-xl">
          {error}
          <button type="button" className="ml-2 text-slate-400 hover:text-white" onClick={() => setError("")}>✕</button>
        </div>
      )}
    </div>
  );
}