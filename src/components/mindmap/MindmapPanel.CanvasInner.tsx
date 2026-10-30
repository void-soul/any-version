import { MindmapNodeContextMenu } from "./MindmapNodeContextMenu";
import { useMindmapPositionWriter } from "./useMindmapPositionWriter";
import { useMindmapCanvasLayout } from "./useMindmapCanvasLayout";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Controls, MarkerType, MiniMap, ReactFlow, useReactFlow, type Connection, type Edge, type Node, type NodeChange, type Viewport } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { CONNECTION_LINE_BY_STYLE, useEdgeStyle } from "../../utils/graphEdgeStyle";
import EdgeStyleSelect from "../shared/EdgeStyleSelect";
import { DIM_MAX, HL_BOOST_MAX, dimmedEdgeStroke, dimmedEdgeWidth, dimmedOpacity, edgeGlow, hasVisibleHighlight, relatedEdgeWidth, useGraphDepthPref, useGraphHighlightPrefs } from "../../utils/graphHighlight";
import { hopFade } from "../../utils/graphScope";
import GraphLookPrefs from "../shared/GraphLookPrefs";
import { DocumentMarkdown } from "../shared/markdown/DocumentMarkdown";
import { HoverPreview, useHoverPreview } from "../shared/HoverPreview";
import { Brain, LayoutGrid, ScrollText, StickyNote, Image, Plus, Pencil, Eye, ChevronDown, ChevronRight, ListTree, Link2, LocateFixed } from "lucide-react";
import { DocumentFull, MindmapLink, MindmapNode, MindmapSticker, mmApi } from "./types";
import { isLayoutDir, layoutTree, type LayoutDir } from "./layout";
import { VEX_CYBER_CYAN } from "../../utils/brand";
import { usePaneWidth } from "./paneWidths";
import { ACCENT, LAYOUT_DIR_ICONS, LAYOUT_DIR_KEYS, STICKER_PALETTE, ZOOM_HIDE_TEXT, button, effectiveNodeColor, mmEdgeTypes, mmNodeTypes, randomNodeColor, stickerRotation, stickerSpawnPosition, type FlowNodeData, type NodeCacheEntry, type PosOverride, type StickerNodeData } from "./MindmapPanel.model";
import { ArrangeHighlightedButton } from "./MindmapPanel.ArrangeHighlightedButton";
import { DetailModal } from "./MindmapPanel.DetailModal";

export function CanvasInner({ full, accent, onDocumentUpdate, onHistoryPush, historyVersion, onAiProject, onExportMd, onError, aiPill, treeOpen, onTreeOpenChange }: { full: DocumentFull; accent: string; onDocumentUpdate: (d: DocumentFull) => void; onHistoryPush: () => void; historyVersion: number; onAiProject: () => void; onExportMd: () => void; onError: (message: string) => void; aiPill?: React.ReactNode; treeOpen: boolean; onTreeOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const { fitView, flowToScreenPosition } = useReactFlow();
  // 连线样式（曲线 / 直角折线 / 直线）：与 ER 设计、JSON 图共用一份偏好，存在 localStorage
  const [edgeStyle, setEdgeStyle] = useEdgeStyle();
  // 画布外观三滑块（高亮强度 / 淡化强度 / 高亮层级）：与数据库设计器、JSON 浏览
  // 共用同一套算法与面板，但偏好**按模块隔离**（`mindmap` 前缀）——
  // 导图几十个节点、设计图几百张表，能压多淡并不一样，共用一个滑块
  // 等于逼用户「在 A 里调顺手，到 B 全都不对」。
  const { prefs: lookPrefs, setHighlight: setHlBoost, setDim: setDimStrength, reset: resetLook } = useGraphHighlightPrefs("mindmap");
  const { depth: hlDepth, setDepth: setHlDepth } = useGraphDepthPref("mindmap");
  // 节点树导航面板：点击树节点 = 选中 + 展开祖先 + 视口聚焦（与悬浮窗树形选择同一交互直觉）。
  // 显隐状态提升到主面板（与第一栏联动，见 MindmapPanel 的 setSidePanesOpen），
  // 这里保留 setTreeOpen 这个别名，让下方既有调用点无需改动。
  const setTreeOpen = onTreeOpenChange;
  // 第二栏（节点树）宽度：与第一栏/第四栏同一套拖拽把手逻辑 + 同一份持久化
  const [treeW, setTreeW] = usePaneWidth("tree");
  const [detailNode, setDetailNode] = useState<MindmapNode | null>(null);
  // 预览气泡：定位 / 250ms 延迟关闭 / 移入本体不关，都在共享 hook 里。
  // 通用实现见 components/shared/HoverPreview（数据库设计的表列表共用同一份）。
  const hp = useHoverPreview();
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; nodeId: string } | null>(null);
  const {
    lastSaved,
    fullRef,
    pendingPos,
    pendingStickers,
    scheduleFlush,
  } = useMindmapPositionWriter({
    full,
  });
  // ── 自由关系线 ──
  // 关系线模式：此模式下拖拽连线只加「额外关系线」，不改父子关系（按住 Shift 拖线等效）。
  const [linkMode, setLinkMode] = useState(false);
  // 连线时是否按住了 Shift（onConnectStart 记录，onConnect 消费）
  const shiftConnectRef = useRef(false);
  // 关系线编辑弹层：新建草稿（isNew=true）或编辑已有关系线
  const [linkDraft, setLinkDraft] = useState<{ id: string; sourceId: string; targetId: string; label: string; x: number; y: number; isNew: boolean } | null>(null);
  // 画布内的轻提示（改上级 / 建关系线后的反馈）
  const [canvasNotice, setCanvasNotice] = useState<string>("");
  const noticeTimer = useRef<number | null>(null);
  const flashCanvas = useCallback((text: string) => {
    setCanvasNotice(text);
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setCanvasNotice(""), 2600);
  }, []);
  // 拖拽改上级：拖拽过程中命中的目标节点（松手时生效）
  const dropTargetRef = useRef<string | null>(null);
  // 点击式连线：关系线模式下第一次点选的节点＝关系线起点，再点一个节点即完成
  const [linkFrom, setLinkFrom] = useState<string | null>(null);
  // 节点 data 里的回调会被 React Flow 缓存，不能闭包捕获会变的状态 → 用 ref 镜像
  const linkFromRef = useRef<string | null>(null);
  const linkModeRef = useRef(false);
  useEffect(() => { linkFromRef.current = linkFrom; }, [linkFrom]);
  useEffect(() => { linkModeRef.current = linkMode; }, [linkMode]);
  // 本次拖线的起点节点 / onConnect 是否已受理（onConnectEnd 兜底用，见下方 onConnectEnd）
  const connectStartRef = useRef<string | null>(null);
  const connectHandledRef = useRef(false);
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
  const {
    collapsed,
    setCollapsed,
    selectedId,
    setSelectedId,
    treeListRef,
    graphNodes,
    stickers,
    links,
    byId,
    dir,
    setDir,
    layout,
    endpointPositions,
    childrenCount,
    descendantCount,
    scope,
    relatedIds,
    hopOfEdge,
    visibleNodes,
    navTree,
    navToNode,
    toggleTreeCollapse,
    expandAllTree,
    collapseAllTree,
    focusSelected,
    linkAnchorOnScreen,
    findDropTarget,
  } = useMindmapCanvasLayout({
    full,
    measuredMap,
    posOverrides,
    flowToScreenPosition,
    hlDepth,
    fitView,
    treeOpen,
  });

  const zoomHideRef = useRef(false);
  const [zoomHide, setZoomHide] = useState(false);
  const onViewportMove = useCallback((_e: unknown, vp: Viewport) => {
    const hide = vp.zoom < ZOOM_HIDE_TEXT;
    if (hide !== zoomHideRef.current) { zoomHideRef.current = hide; setZoomHide(hide); }
  }, []);
  // 切换文档时清空位置覆盖（新文档用其自身已保存坐标或自动布局）
  useEffect(() => { setPosOverrides({}); }, [full.document.id]);
  // 切换文档时丢弃「关系线起点」的待完成状态
  useEffect(() => { setLinkFrom(null); }, [full.document.id]);
  // 切换文档时恢复该文档保存的布局方向（避免沿用上一份导图的方向）
  useEffect(() => {
    const d = full.document.layoutDir;
    setDir(isLayoutDir(d) ? d : "lr");
  }, [full.document.id]);
  // 撤销/重做恢复快照后清空本地拖拽覆盖，让还原的坐标生效
  useEffect(() => { setPosOverrides({}); }, [historyVersion]);

  const addNode = useCallback((parentId: string | null) => {
    onHistoryPush();
    // 新增子节点继承父节点的可见颜色（父节点未手动配色时取类型默认色），保持树视觉连续；
    // 新增根节点/独立节点取随机色（每根各不相同）。
    const parent = parentId ? byId.get(parentId) : null;
    const n: MindmapNode = { id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, documentId: full.document.id, parentId, name: parentId ? t("mindmap.newNode") : t("mindmap.newRoot"), detail: "", kind: parentId ? "other" : "root", color: parent ? effectiveNodeColor(parent) : randomNodeColor(), positionX: 0, positionY: 0 };
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
    const updated = { ...node, parentId: null, kind: node.kind === "root" ? node.kind : "root" };
    void mmApi.upsertNode({ documentId: full.document.id, node: updated });
    onDocumentUpdate({ ...full, nodes: full.nodes.map(n => n.id === nodeId ? updated : n) });
  }, [byId, full, onDocumentUpdate, onHistoryPush]);

  // ── 自由关系线：新建 / 编辑 / 删除 ──

  /** 节点卡片中心的画布坐标（关系线弹层定位用）。 */

  /** 两节点连线中点的屏幕坐标（新建关系线时给弹层定位；坐标缺失则退回屏幕中心）。 */

  /** 打开「新建关系线」弹层；同向关系线已存在时直接打开它编辑（不重复建线）。 */
  const openLinkCreator = useCallback((sourceId: string, targetId: string) => {
    const existing = links.find(l => l.sourceId === sourceId && l.targetId === targetId);
    const p = linkAnchorOnScreen(sourceId, targetId);
    if (existing) { setLinkDraft({ id: existing.id, sourceId, targetId, label: existing.label, x: p.x, y: p.y, isNew: false }); return; }
    setLinkDraft({ id: `lk${Date.now()}-${Math.random().toString(36).slice(2, 6)}`, sourceId, targetId, label: "", x: p.x, y: p.y, isNew: true });
  }, [links, linkAnchorOnScreen]);

  /** 点击关系线标签 → 就地编辑（弹层贴着鼠标位置）。 */
  const openLinkEditor = useCallback((link: MindmapLink, e: React.MouseEvent) => {
    setLinkDraft({ id: link.id, sourceId: link.sourceId, targetId: link.targetId, label: link.label, x: e.clientX, y: e.clientY, isNew: false });
  }, []);

  /** 点选节点：关系线模式下退化为「点起点 → 点终点」的点击式连线（不必去够连接点）。
   *  注意：本函数会存进节点 data 被 React Flow 缓存，因此只读 ref，不闭包捕获 linkMode/linkFrom。 */
  const selectNode = useCallback((id: string) => {
    setSelectedId(id);
    if (!linkModeRef.current) return;
    const from = linkFromRef.current;
    if (!from) { setLinkFrom(id); flashCanvas(t("mindmap.linkPickFrom", { name: byId.get(id)?.name ?? id })); return; }
    if (from === id) { setLinkFrom(null); return; } // 再点起点＝取消
    setLinkFrom(null);
    openLinkCreator(from, id);
  }, [byId, openLinkCreator, flashCanvas, t]);

  /** 保存关系线（新建或改文字）。 */
  const saveLinkDraft = useCallback(() => {
    if (!linkDraft) return;
    const now = new Date().toISOString();
    const old = links.find(l => l.id === linkDraft.id);
    const link: MindmapLink = {
      id: linkDraft.id, documentId: full.document.id, sourceId: linkDraft.sourceId, targetId: linkDraft.targetId,
      label: linkDraft.label.trim(), createdAt: old?.createdAt ?? now, updatedAt: now,
    };
    onHistoryPush();
    void mmApi.upsertLink({ documentId: full.document.id, link });
    const next = old ? links.map(l => (l.id === link.id ? link : l)) : [...links, link];
    onDocumentUpdate({ ...full, links: next });
    setLinkDraft(null);
    flashCanvas(t("mindmap.linkSaved", { label: link.label || t("mindmap.linkMode") }));
  }, [linkDraft, links, full, onDocumentUpdate, onHistoryPush, flashCanvas, t]);

  /** 删除关系线。 */
  const removeLink = useCallback((linkId: string) => {
    onHistoryPush();
    void mmApi.deleteLink({ documentId: full.document.id, linkId });
    onDocumentUpdate({ ...full, links: links.filter(l => l.id !== linkId) });
    setLinkDraft(prev => (prev?.id === linkId ? null : prev));
  }, [links, full, onDocumentUpdate, onHistoryPush]);

  const onConnect = useCallback((connection: Connection) => {
    // 只要连接点之间连上了（哪怕后面被防环等规则拒绝），就不该再走 onConnectEnd 的兜底
    connectHandledRef.current = true;
    const source = connection.source;
    const target = connection.target;
    if (!source || !target || source === target || !byId.has(source) || !byId.has(target)) return;
    // 关系线模式（或按住 Shift 拖线）：只加一条带文字的额外关系线，不改父子关系。
    if (linkMode || shiftConnectRef.current) { openLinkCreator(source, target); return; }
    // 不能连到自己的后代，否则成环。
    let cursor: string | null = source;
    const seen = new Set<string>();
    while (cursor && !seen.has(cursor)) {
      if (cursor === target) return;
      seen.add(cursor);
      cursor = byId.get(cursor)?.parentId ?? null;
    }
    const child = byId.get(target)!;
    onHistoryPush();
    // 目标已有上级 → 改挂到新上级（其子树一并跟随）；没有上级 → 设置上级。
    const updated = { ...child, parentId: source, kind: child.kind === "root" ? "other" : child.kind };
    void mmApi.upsertNode({ documentId: full.document.id, node: updated });
    onDocumentUpdate({ ...full, nodes: full.nodes.map(n => n.id === target ? updated : n) });
    if (child.parentId && child.parentId !== source) {
      flashCanvas(t("mindmap.reparented", { name: child.name, parent: byId.get(source)?.name ?? source }));
    }
  }, [byId, full, onDocumentUpdate, onHistoryPush, linkMode, openLinkCreator, flashCanvas, t]);

  /** 记录拖线起点与是否按住 Shift（松手时 onConnect / onConnectEnd 据此分流）。 */
  const onConnectStart = useCallback((event: MouseEvent | TouchEvent, params: { nodeId?: string | null }) => {
    shiftConnectRef.current = "shiftKey" in event ? Boolean(event.shiftKey) : false;
    connectStartRef.current = params?.nodeId ?? null;
    connectHandledRef.current = false;
  }, []);

  /** 拖线兜底：关系线模式下可以「拖到目标节点的任意位置」松手（不必精确命中 12px 的连接点）。
   *  语义＝从起点节点连到落点节点；只有连接点之间没连上（onConnect 未触发）时才生效。 */
  const onConnectEnd = useCallback((event: MouseEvent | TouchEvent) => {
    const fromId = connectStartRef.current;
    const handled = connectHandledRef.current;
    connectStartRef.current = null;
    connectHandledRef.current = false;
    if (handled || !fromId) return;
    if (!linkMode && !shiftConnectRef.current) return;
    const point = "changedTouches" in event ? event.changedTouches[0] : event;
    if (!point) return;
    const el = document.elementFromPoint(point.clientX, point.clientY);
    const toId = el?.closest(".react-flow__node")?.getAttribute("data-id") ?? null;
    if (!toId || toId === fromId || !byId.has(toId)) return;
    openLinkCreator(fromId, toId);
  }, [linkMode, byId, openLinkCreator]);

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
    onDocumentUpdate({ ...full, nodes: full.nodes.filter(n => !removed.has(n.id)), links: links.filter(l => !removed.has(l.sourceId) && !removed.has(l.targetId)) });
    if (selectedId === nodeId) setSelectedId(null);
    if (detailNode?.id === nodeId) setDetailNode(null);
    hp.hideNow();
    setLinkDraft(prev => (prev && (removed.has(prev.sourceId) || removed.has(prev.targetId)) ? null : prev));
  }, [selectedId, detailNode, full, links, onDocumentUpdate, onHistoryPush]);

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
      // 选中只认自己；祖先链不再当高亮（否则选中深处节点会亮一大片），改由 relatedIds 承担
      const selected = selectedId === n.id;
      const related = relatedIds.has(n.id);
      const dimmed = selectedId !== null && !selected && !related;
      // 「淡化强度」只在真的选中了别人时才算：没选中时整图都该是常态亮度
      const dimOpacity = dimmed ? dimmedOpacity(lookPrefs.dim) : 1;
      const hlBoost = lookPrefs.highlight;
      const hasChildren = (childrenCount.get(n.id) ?? 0) > 0;
      const isCollapsed = collapsed.has(n.id);
      const hiddenCount = descendantCount.get(n.id) ?? 0;
      // ComfyUI 式连接点：输入口取父节点类别色，输出口取本节点类别色
      const parent = n.parentId ? byId.get(n.parentId) : undefined;
      const parentColor = parent ? effectiveNodeColor(parent) : null;
      const projectRoot = full.document.sourceDesc || null;
      const prev = cache.get(n.id);
      if (prev && prev.full === full && prev.node === n && prev.px === p.x && prev.py === p.y && prev.selected === selected && prev.related === related && prev.dimmed === dimmed && prev.hlBoost === hlBoost && prev.dimOpacity === dimOpacity && prev.hasChildren === hasChildren && prev.collapsed === isCollapsed && prev.hiddenCount === hiddenCount && prev.hideText === zoomHide && prev.parentColor === parentColor) {
        main.push(prev.obj as Node<FlowNodeData>);
        continue;
      }
      // data 仅在节点内容/状态变化时重建；纯位置变化（拖动中）复用旧 data 引用，
      // 避免 memo(FlowNode) 因 data 每帧新引用而重渲染整个节点子树（WebView2 下闪烁）。
      // 注意：data 里的回调（onAddChild/onOpenDetail/onDelete…）闭包捕获了 full/selectedId 等状态，
      // 因此 full 一旦变化就必须重建 data，否则复用旧 data 会让回调闭包停留在旧状态——
      // 例如连续点「+」加子节点时，第 3 次会用第 1 次渲染时的旧 full，把第 2 个子节点覆盖掉。
      const prevData = prev ? (prev.obj as Node<FlowNodeData>).data : null;
      const dataChanged = !prevData || prev!.full !== full || prev!.node !== n || prev!.selected !== selected || prev!.related !== related || prev!.dimmed !== dimmed || prev!.hlBoost !== hlBoost || prev!.dimOpacity !== dimOpacity || prev!.hasChildren !== hasChildren || prev!.collapsed !== isCollapsed || prev!.hiddenCount !== hiddenCount || prevData.targetPosition !== endpointPositions.target || prevData.sourcePosition !== endpointPositions.source || prevData.hideText !== zoomHide || prevData.parentColor !== parentColor;
      const data: FlowNodeData = dataChanged ? { node: n, selected, related, dimmed, hlBoost, dimOpacity, hasChildren, collapsed: isCollapsed, hiddenCount, hideText: zoomHide, parentColor, projectRoot, targetPosition: endpointPositions.target, sourcePosition: endpointPositions.source,
            onSelect: () => selectNode(n.id), onOpenDetail: () => openDetail(n),
            onToggle: () => setCollapsed(cur => { const nx = new Set(cur); nx.has(n.id) ? nx.delete(n.id) : nx.add(n.id); return nx; }),
            onAddChild: () => addChildNode(n.id),
            onPreview: (e) => hp.show({ title: n.name, x: e.clientX, y: e.clientY, accent: effectiveNodeColor(n), content: <DocumentMarkdown content={n.detail || t("mindmap.missingView")} /> }),
            onPreviewEnd: () => hp.scheduleHide(),
            onDelete: () => deleteNode(n.id),
            onContextMenu: (e) => { setSelectedId(n.id); setCtxMenu({ x: e.clientX, y: e.clientY, nodeId: n.id }); } }
        : prevData;
      const prevObj = (prev?.obj ?? null) as Node<FlowNodeData> | null;
      const measured = measuredMap[n.id] ?? prevObj?.measured;
      const obj: Node<FlowNodeData> = prevObj
        ? { ...prevObj, position: p, data, measured, sourcePosition: endpointPositions.source, targetPosition: endpointPositions.target }
        : { id: n.id, type: "mmNode", position: p, data, measured, sourcePosition: endpointPositions.source, targetPosition: endpointPositions.target };
      cache.set(n.id, { node: n, full, px: p.x, py: p.y, selected, related, dimmed, hlBoost, dimOpacity, hasChildren, collapsed: isCollapsed, hiddenCount, hideText: zoomHide, parentColor, obj });
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
      cache.set(sid, { node: s, full, px: p.x, py: p.y, selected: false, related: false, dimmed: false, hlBoost: lookPrefs.highlight, dimOpacity: 1, hasChildren: false, collapsed: false, hiddenCount: 0, hideText: false, parentColor: null, obj });
      stickerNodes.push(obj);
    }
    // 清理不再显示的缓存项
    for (const k of cache.keys()) if (!alive.has(k)) cache.delete(k);
    return [...main, ...stickerNodes];
  }, [visibleNodes, layout, selectedId, collapsed, childrenCount, stickers, full, onDocumentUpdate, relatedIds, posOverrides, measuredMap, addChildNode, deleteNode, openDetail, endpointPositions, zoomHide, byId, selectNode, lookPrefs.dim, lookPrefs.highlight]);

  const edges = useMemo<Edge[]>(() => {
    const visible = new Set(visibleNodes.map(n => n.id));
    const out: Edge[] = [];
    /* ── 高亮口径：整块连线**只在这里定义一次**，下面两个循环共用 ──
     *
     * 判据是 `scope.edgeIds`（BFS 实际走过的边），不是「两端有一端在节点集合内」：
     * 后者会点亮同跳层内部的横向连线，产出「线亮着、两端的节点却暗着」。
     * 节点那边的三态用的是同一次遍历的 hops，两边口径一旦分家就会出现
     * 「节点亮了、连着它的线还暗着」。
     *
     * `hasSel` 那层判断不能省：作用域在未选中时是**空的**，只看它的话
     * 「什么都没选」会被算成「全都不相关」，整图连线一起压暗。 */
    const hasSel = selectedId !== null;
    const hl = lookPrefs.highlight;
    const dim = lookPrefs.dim;
    /** 一条连线的完整外观：亮度随跳数退、压暗随「淡化强度」退、箭头随强度一起收掉 */
    const look = (edgeId: string, a: string, b: string, color: string) => {
      const rel = hasSel && scope.edgeIds.has(edgeId);
      const fade = rel ? hopFade(hopOfEdge(a, b)) : 1;
      return {
        rel,
        dim: hasSel && !rel,
        // 线宽与辉光按跳数退亮度（否则层级调到 3 跳与 1 跳画出来一样）
        glow: edgeGlow(color, hl * fade),
        width: relatedEdgeWidth(hl) * fade,
        // 箭头与线宽走同一判据：强度 0 时线已看不见，满屏实心三角反而成了「只亮线」
        arrow: !(rel && !hasVisibleHighlight(hl)),
        stroke: dimmedEdgeStroke(dim),
        dimWidth: dimmedEdgeWidth(dim),
      };
    };
    // 树形主连线（实线，走主输入/输出口）
    for (const n of visibleNodes) {
      if (!n.parentId || !visible.has(n.parentId)) continue;
      // 样式经 `data` 传给自定义 edge（ColorEdge 不吃 edge.style —— 它自己画 path）。
      const lk = look(`mm-e-${n.id}`, n.parentId, n.id, effectiveNodeColor(n));
      out.push({ id: `mm-e-${n.id}`, source: n.parentId, target: n.id, sourceHandle: "out", targetHandle: "in", type: "colorE",
        animated: lk.rel, zIndex: lk.rel ? 10 : 0,
        data: { color: effectiveNodeColor(n), style: edgeStyle, rel: lk.rel, dim: lk.dim, stroke: lk.dim ? lk.stroke : undefined, width: lk.rel ? lk.width : lk.dimWidth, glow: lk.rel ? lk.glow : undefined, arrow: lk.arrow },
        markerEnd: lk.arrow ? { type: MarkerType.ArrowClosed, color: "#f8fafc" } : undefined } as Edge);
    }
    // 自由关系线（虚线 + 文字标签）：两端节点都可见时才画（折叠隐藏的子树不画）
    for (const l of links) {
      if (!visible.has(l.sourceId) || !visible.has(l.targetId)) continue;
      const src = byId.get(l.sourceId);
      const lk = look(`mm-l-${l.id}`, l.sourceId, l.targetId, src ? effectiveNodeColor(src) : VEX_CYBER_CYAN);
      out.push({
        id: `mm-l-${l.id}`, source: l.sourceId, target: l.targetId, sourceHandle: "out", targetHandle: "in",
        type: "relationE",
        // 关联的关系线也走流转动画：它本来就是「非树形的额外关联」，与高亮语义一致
        animated: lk.rel, zIndex: lk.rel ? 10 : 0,
        data: {
          color: src ? effectiveNodeColor(src) : VEX_CYBER_CYAN,
          style: edgeStyle,
          flow: lk.rel,
          dim: lk.dim,
          stroke: lk.dim ? lk.stroke : undefined,
          width: lk.rel ? lk.width : lk.dimWidth,
          glow: lk.rel ? lk.glow : undefined,
          arrow: lk.arrow,
          label: l.label,
          onEdit: (e: React.MouseEvent) => openLinkEditor(l, e),
          onDelete: () => removeLink(l.id),
        },
      } as Edge);
    }
    return out;
  }, [visibleNodes, byId, selectedId, scope, hopOfEdge, links, openLinkEditor, removeLink, edgeStyle, lookPrefs]);

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

  // 视口初始对齐：仅在切换文档时 fitView。
  // 注意：不能把 collapsed 放进依赖——点击节点折叠/展开按钮时 collapsed 变化会触发
  // 全图 fitView（整屏缩放），用户感知为「点折叠按钮却缩放全屏」；
  // 折叠本应只是局部收缩子节点，不应扰动当前视口。
  useEffect(() => { const t = window.setTimeout(() => fitView({ padding: 0.2, duration: 260 }), 0); return () => window.clearTimeout(t); }, [fitView, full.document.id]);

  const updateNode = useCallback((patch: Partial<MindmapNode>) => {
    if (!detailNode) return;
    const updated = { ...detailNode, ...patch };
    // 注意：不能 setDetailNode(updated) —— 弹窗关闭时的 unmount 自动保存会再次
    // 调用 updateNode → setDetailNode → 把刚关闭的弹窗重新打开（无法关闭的 bug）。
    // 弹窗内部用本地 state 渲染编辑，父级 detailNode 仅用于挂载，无需同步。
    void mmApi.upsertNode({ documentId: full.document.id, node: updated });
    onDocumentUpdate({ ...full, nodes: full.nodes.map(n => n.id === updated.id ? updated : n) });
  }, [detailNode, full, onDocumentUpdate]);

  /** 拖拽中的节点中心落在哪个节点卡片里（拖到某节点上＝成为它的子节点）。 */

  const onNodeDrag = useCallback((_e: MouseEvent | TouchEvent, node: Node) => {
    dropTargetRef.current = findDropTarget(node);
  }, [findDropTarget]);

  const onNodeDragStop = useCallback((_e: MouseEvent | TouchEvent, node: Node) => {
    const dropTarget = dropTargetRef.current;
    dropTargetRef.current = null;
    const cur = fullRef.current;
    const now = new Date().toISOString();
    const dragged = node.id.startsWith("sticker-") ? null : byId.get(node.id);
    const target = dropTarget ? byId.get(dropTarget) : null;
    // 拖到另一个节点卡片上松手 → 改上级（落位交给自动布局：清掉已保存坐标，排到新上级的子节点序列里）
    if (dragged && target && dragged.id !== target.id && dragged.parentId !== target.id) {
      onHistoryPush(); // 一次拖放 = 一步撤销
      const updated: MindmapNode = { ...dragged, parentId: target.id, kind: dragged.kind === "root" ? "other" : dragged.kind, positionX: 0, positionY: 0 };
      void mmApi.upsertNode({ documentId: cur.document.id, node: updated });
      onDocumentUpdate({ ...cur, document: { ...cur.document, updatedAt: now }, nodes: cur.nodes.map(n => n.id === updated.id ? updated : n) });
      setPosOverrides(prev => { if (!(updated.id in prev)) return prev; const next = { ...prev }; delete next[updated.id]; return next; });
      // 上一次拖拽可能还有未落盘的坐标，必须一并丢弃，否则会把刚清掉的坐标写回去
      pendingPos.current = pendingPos.current.filter(p => p.nodeId !== updated.id);
      flashCanvas(t("mindmap.reparented", { name: dragged.name, parent: target.name }));
      return;
    }
    onHistoryPush(); // 一次拖放 = 一步撤销
    if (node.id.startsWith("sticker-")) {
      const sid = node.id.replace("sticker-", "");
      pendingStickers.current.set(sid, { x: node.position.x, y: node.position.y });
      onDocumentUpdate({ ...cur, document: { ...cur.document, updatedAt: now }, stickers: cur.stickers.map(s => s.id === sid ? { ...s, positionX: node.position.x, positionY: node.position.y, updatedAt: now } : s) });
    } else {
      pendingPos.current.push({ nodeId: node.id, x: node.position.x, y: node.position.y });
      onDocumentUpdate({ ...cur, document: { ...cur.document, updatedAt: now }, nodes: cur.nodes.map(n => n.id === node.id ? { ...n, positionX: node.position.x, positionY: node.position.y } : n) });
    }
    scheduleFlush();
  }, [onDocumentUpdate, scheduleFlush, onHistoryPush, byId, flashCanvas, t]);

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

  // 键盘方向键按树关系移动：上=父节点，下=首个子节点，左右=前后同级节点。
  // 这比按画布几何位置猜测更稳定，尤其适用于自动布局和手动拖动混用的导图。
  const navigateByKey = useCallback((key: string) => {
    if (!selectedId) return;
    const current = byId.get(selectedId);
    if (!current) return;
    let nextId: string | null = null;
    if (key === "ArrowUp") {
      nextId = current.parentId && byId.has(current.parentId) ? current.parentId : null;
    } else if (key === "ArrowDown") {
      nextId = graphNodes.find(n => n.parentId === current.id)?.id ?? null;
    } else if (key === "ArrowLeft" || key === "ArrowRight") {
      const siblings = graphNodes.filter(n => (n.parentId ?? null) === (current.parentId ?? null));
      const index = siblings.findIndex(n => n.id === current.id);
      if (index >= 0) {
        const offset = key === "ArrowLeft" ? -1 : 1;
        nextId = siblings[index + offset]?.id ?? null;
      }
    }
    if (nextId) navToNode(nextId);
  }, [selectedId, byId, graphNodes, navToNode]);

  /**
   * 「整理选中」：**只**排布高亮范围里的节点，其余一律不动（与数据库设计器同一语义）。
   *
   * 集合必须复用高亮作用域（`relatedIds`），不能另写一份：设计器那边曾经高亮用一份
   * 内联写法、整理用整个连通分量，两边对不上 —— 提示条如实报「已整理 22 张」，
   * 高亮却只有 6 张。标签、提示与实现说的必须是同一件事。
   *
   * 排完**整体平移回原高亮区域左上角**：否则这批卡片会跳到画布原点，用户以为整张图被重排了。
   *
   * 不弹确认：它只改坐标，Ctrl+Z 可回退，点一次「自动布局」也能回来。
   */
  const arrangeHighlighted = useCallback(() => {
    const ids = new Set(relatedIds);
    const subset = full.nodes.filter((n) => ids.has(n.id));
    if (subset.length < 2) return;
    const laid = layoutTree(subset, dir);
    // 平移量 = 旧包围盒左上 − 新包围盒左上（用当前画布坐标，保住用户手拖过的位置）
    const cur = new Map(flowNodes.map((n) => [n.id, n.position]));
    let oldX = Infinity; let oldY = Infinity; let newX = Infinity; let newY = Infinity;
    for (const n of subset) {
      const now = cur.get(n.id); const next = laid.get(n.id);
      if (!now || !next) continue;
      oldX = Math.min(oldX, now.x); oldY = Math.min(oldY, now.y);
      newX = Math.min(newX, next.x); newY = Math.min(newY, next.y);
    }
    if (!Number.isFinite(oldX) || !Number.isFinite(newX)) return;
    const dx = oldX - newX;
    const dy = oldY - newY;
    const n2 = subset.map((n) => ({ nodeId: n.id, x: (laid.get(n.id)?.x ?? 0) + dx, y: (laid.get(n.id)?.y ?? 0) + dy }));
    void mmApi.updatePositions(full.document.id, n2);
    setPosOverrides(prev => {
      const next = { ...prev };
      for (const p of n2) next[p.nodeId] = { x: p.x, y: p.y };
      return next;
    });
    flashCanvas(t("mindmap.arrangeDone", { count: n2.length }));
    window.setTimeout(() => fitView({ nodes: n2.map((p) => ({ id: p.nodeId })), padding: 0.25, duration: 260 }), 30);
  }, [relatedIds, full, dir, flowNodes, fitView, flashCanvas, t]);

  const arrangeDisabled = selectedId === null || relatedIds.size < 2;

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

  // 空白处按下即收起（用 mousedown 而非 click：拖线建立关系线是 mouseup 触发，
  // 紧随其后的 click 会把刚打开的弹层立刻关掉）。弹层内部各自 stopPropagation。
  useEffect(() => { const close = () => { setCtxMenu(null); setLinkDraft(null); }; window.addEventListener("mousedown", close); return () => window.removeEventListener("mousedown", close); }, []);

  // ── 键盘快捷键 ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (detailNode) return; // Detail modal open - let it handle keys
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement) return;
      if (e.key === "Delete" || e.key === "Backspace") { if (selectedId) { e.preventDefault(); deleteSelected(); } }
      else if (e.key === "Tab") { e.preventDefault(); addChildNode(); }
      else if (e.key === "Enter") { if (selectedId) { e.preventDefault(); const n = byId.get(selectedId); if (n) setDetailNode(n); } }
      else if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(e.key)) { e.preventDefault(); navigateByKey(e.key); }
      else if (e.key === "Escape") { setSelectedId(null); setCtxMenu(null); hp.hideNow(); setLinkDraft(null); setLinkFrom(null); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedId, detailNode, deleteSelected, addChildNode, byId, navigateByKey]);

  return (
    // 四栏布局的第三栏（画布）：第二栏「节点树 + 底部选项」由下方 order-first 的
    // 原右上角覆盖层承担，视觉上排在画布左侧；absolute 覆盖层（提示/上下文菜单）
    // 仍以本容器为定位父级。
    <div className="relative flex h-full min-h-0">
      <ReactFlow className="min-w-0 flex-1" nodes={flowNodes} edges={edges} nodeTypes={mmNodeTypes} edgeTypes={mmEdgeTypes}
        onNodesChange={onNodesChange} onNodeDrag={onNodeDrag} onNodeDragStop={onNodeDragStop} onConnect={onConnect} onConnectStart={onConnectStart} onConnectEnd={onConnectEnd} onMove={onViewportMove}
        onPaneClick={() => { setSelectedId(null); setCtxMenu(null); hp.hideNow(); setLinkFrom(null); }}
        onNodeContextMenu={(e, n) => onNodeContextMenu(e, n as Node)}
        onEdgeClick={(e, ed) => { const l = links.find(x => `mm-l-${x.id}` === ed.id); if (l) openLinkEditor(l, e); }}
        connectionLineType={CONNECTION_LINE_BY_STYLE[edgeStyle]}
        minZoom={0.1} maxZoom={2.5} nodesConnectable
        proOptions={{ hideAttribution: true }}>
        <MiniMap style={{ backgroundColor: "var(--color-surface-deep)", border: "1px solid rgba(255,255,255,.12)" }} className="!bg-slate-950/95"
          nodeColor={(n) => { const d = n.data as FlowNodeData | StickerNodeData; return 'node' in d ? effectiveNodeColor(d.node) : "#fef3c7"; }}
          nodeStrokeColor="#0f172a" nodeBorderRadius={2} maskColor="rgba(2,6,23,0.72)" pannable zoomable />
        <Controls className="canvas-flow-controls" showInteractive={false} />
      </ReactFlow>

      {/* 连线样式原先是这里的右下角浮层，已挪到左侧设置区（与「布局」并排一行）*/}

      {/* 画布外观浮层（高亮强度 / 淡化强度 / 高亮层级 + 整理选中）：
          与数据库设计器、JSON 浏览同一块面板（shared/GraphLookPrefs），只是偏好各自隔离。
          位置量着左下角 Controls 算：Controls 三颗钮 30px + 上下边框 ≈ 92px，
          bottom-[104px] 留 12px 间隙不压住它（与 JSON 画布同一套定位法）。 */}
      <div className="absolute bottom-[104px] left-0 z-10 flex w-[30px] justify-center">
        <GraphLookPrefs
          highlight={lookPrefs.highlight}
          dim={lookPrefs.dim}
          onHighlight={setHlBoost}
          onDim={setDimStrength}
          depth={hlDepth}
          onDepth={setHlDepth}
          onReset={resetLook}
          maxHighlight={HL_BOOST_MAX}
          maxDim={DIM_MAX}
          extra={<ArrangeHighlightedButton onArrange={arrangeHighlighted} disabled={arrangeDisabled} />}
        />
      </div>

      {/* 连线规则提示：拖线＝改上级（拖到节点卡片上松手同样改上级）；关系线模式下拖线＝加一条带文字的关系线 */}
      <div className="pointer-events-none absolute bottom-2 left-1/2 z-10 -translate-x-1/2">
        <span className={`inline-flex items-center gap-1 whitespace-nowrap rounded-full border px-2.5 py-1 text-micro shadow backdrop-blur ${linkMode ? "border-cyan-400/40 bg-cyan-950/80 text-cyan-200" : "border-white/10 bg-slate-900/80 text-slate-400"}`}>
          <Link2 className="h-2.5 w-2.5 text-cyan-300/80" />
          {linkFrom
            ? t("mindmap.linkFromHint", { name: byId.get(linkFrom)?.name ?? linkFrom })
            : linkMode ? t("mindmap.linkModeOn") : t("mindmap.connectHint")}
        </span>
      </div>

      {/* 画布轻提示：改上级 / 建关系线的即时反馈 */}
      {canvasNotice && (
        <div className="pointer-events-none absolute left-1/2 top-3 z-30 -translate-x-1/2 rounded-full border border-cyan-400/30 bg-slate-900/95 px-3 py-1.5 text-tiny text-cyan-200 shadow-xl">
          {canvasNotice}
        </div>
      )}

      {/* 第二栏：节点树 + 底部选项（原画布右上角覆盖层）。
          用 order-first 让它排在画布左侧，避免大段 JSX 搬移；收起时收成窄条。
          外层 pointer-events-none 让空白区不挡交互，各交互块自身 pointer-events-auto。 */}
      <div className="relative order-first pointer-events-none flex shrink-0 flex-col border-r border-white/5 bg-surface-panel/70"
        style={{ width: treeOpen ? treeW : 36 }}>
        {aiPill && <div className={`pointer-events-auto ${treeOpen ? "p-1.5" : "hidden"}`}>{aiPill}</div>}
        <div className="flex min-h-0 flex-1 flex-col">
          {/* 节点树导航：与悬浮窗树形选择同交互 —— 点击即导航到该节点 */}
          <div className="pointer-events-auto flex min-h-0 flex-1 flex-col">
            {!treeOpen && (
              <button type="button" className="mx-auto mt-1 rounded p-1 text-slate-400 transition hover:bg-white/10 hover:text-white"
                onClick={() => setTreeOpen(true)} title={t("mindmap.treeNavShow")}>
                <ListTree className="h-3.5 w-3.5" />
              </button>
            )}
            {treeOpen && (
              <div className="flex min-h-0 flex-1 flex-col border-b border-white/10">
            <div className="flex items-center gap-1.5 border-b border-white/10 px-2 py-1.5">
              <ListTree className="h-3 w-3 shrink-0 text-slate-500" />
              <span className="min-w-0 flex-1 truncate text-tiny font-semibold uppercase tracking-wide text-slate-400">{t("mindmap.treeNavTitle")}</span>
              {/* 全部展开 / 全部折叠：树上折完想一眼恢复时用，避免逐个点箭头 */}
              <button type="button" className="shrink-0 rounded border border-white/10 px-1 py-px text-micro text-slate-500 transition hover:bg-white/10 hover:text-white"
                onClick={expandAllTree} title={t("mindmap.treeExpandAll")}>{t("mindmap.treeExpandAllShort")}</button>
              <button type="button" className="shrink-0 rounded border border-white/10 px-1 py-px text-micro text-slate-500 transition hover:bg-white/10 hover:text-white"
                onClick={collapseAllTree} title={t("mindmap.treeCollapseAll")}>{t("mindmap.treeCollapseAllShort")}</button>
              <button type="button" className="ui-icon-btn p-0.5" onClick={() => setTreeOpen(false)} title={t("mindmap.treeNavHide")}>
                <ChevronRight className="h-3 w-3" />
              </button>
            </div>
            <div ref={treeListRef} className="min-h-0 flex-1 overflow-y-auto py-0.5">
              {navTree.map((x, i) => {
                const on = selectedId === x.node.id;
                const rowColor = effectiveNodeColor(x.node);
                return (
                  <div key={x.node.id} data-tree-idx={i} role="button" tabIndex={-1}
                    className={`group/row flex w-full items-center gap-1 py-1 pr-1.5 text-left text-tiny transition hover:bg-white/[0.06] ${on ? "text-white" : "text-slate-400"}`}
                    style={{ paddingLeft: 2 + x.depth * 12 }}>
                    {/* 行首折叠箭头：与文件夹树同一套写法（无子节点时给等宽占位，保证缩进对齐） */}
                    {childrenCount.get(x.node.id) ? (
                      <button type="button"
                        className="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded text-slate-500 transition hover:bg-white/10 hover:text-white"
                        onClick={(e) => { e.stopPropagation(); toggleTreeCollapse(x.node.id); }}
                        title={collapsed.has(x.node.id) ? t("mindmap.expand") : t("mindmap.collapse")}>
                        {collapsed.has(x.node.id) ? <ChevronRight className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
                      </button>
                    ) : (
                      <span className="w-3.5 shrink-0" />
                    )}
                    {/* 主体：点击 = 导航（展开祖先 + 选中 + 视口聚焦） */}
                    <button type="button"
                      className={`flex min-w-0 flex-1 items-center gap-1 text-left ${on ? "text-white" : "text-slate-400"}`}
                      onClick={() => navToNode(x.node.id)}
                      title={x.node.name}>
                      <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: rowColor, boxShadow: on ? `0 0 5px ${rowColor}` : undefined }} />
                      <span className={`min-w-0 flex-1 truncate ${on ? "font-semibold" : ""}`}
                        style={on ? { color: rowColor } : undefined}>{x.node.name}</span>
                      {collapsed.has(x.node.id) && descendantCount.get(x.node.id) ? (
                        <span className="shrink-0 rounded bg-slate-800 px-1 text-[8px] text-slate-500"
                          title={t("mindmap.collapsedHint", { count: descendantCount.get(x.node.id) ?? 0 })}>
                          +{descendantCount.get(x.node.id)}
                        </span>
                      ) : null}
                    </button>
                    {/* 行尾悬浮操作：预览详细内容 / 打开编辑弹框（与日历定位行为一致） */}
                    <span className="hidden shrink-0 items-center gap-0.5 group-hover/row:flex">
                      <button type="button" className="rounded p-0.5 text-slate-500 transition hover:bg-white/10 hover:text-cyan-300"
                        onMouseEnter={(e) => hp.show({ title: x.node.name, x: e.clientX, y: e.clientY, accent: effectiveNodeColor(x.node), content: <DocumentMarkdown content={x.node.detail || t("mindmap.missingView")} /> })}
                        onMouseMove={(e) => { if (!hp.preview || hp.preview.title !== x.node.name) hp.show({ title: x.node.name, x: e.clientX, y: e.clientY, accent: effectiveNodeColor(x.node), content: <DocumentMarkdown content={x.node.detail || t("mindmap.missingView")} /> }); }}
                        onMouseLeave={() => hp.scheduleHide()}
                        onClick={(e) => e.stopPropagation()}
                        title={t("mindmap.previewOnly")}>
                        <Eye className="h-3 w-3" />
                      </button>
                      <button type="button" className="ui-icon-btn p-0.5"
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
          {/* 第二栏底部选项（原浮动工具栏）：全部改用与第一栏底部同一套按钮外观（`button` 常量），
              两栏底栏视觉一致；计划日历入口不在这里 —— 它展示的是跨全部文档的聚合计划（全局视图），
              入口统一放第一栏（文档栏）底部。收起成窄条时整块隐藏，避免挤在 36px 宽里 */}
          <div className={`pointer-events-auto flex shrink-0 flex-col gap-1 p-1.5 pt-0 ${treeOpen ? "" : "hidden"}`}>
            <div className="rounded-ctl border border-white/10 bg-slate-900/95 p-1 shadow-lg flex flex-col gap-0.5">
              {/* 「布局」与「连线样式」并排一行：两者都是画布级的观感设置，
                  原来连线样式是画布右下角的浮层，和这里隔了一整个画布。
                  外观沿用 button 常量，与下面这些按钮同一套。 */}
              <div className="grid grid-cols-2 gap-0.5">
                <button type="button" className={`${button} justify-start`} onClick={relayout} title={t("mindmap.autoLayout")}><LayoutGrid className="h-3 w-3" />{t("mindmap.layout")}</button>
                <EdgeStyleSelect
                  value={edgeStyle}
                  onChange={setEdgeStyle}
                  className={`${button} justify-start appearance-none px-1.5`}
                />
              </div>
              {/* 布局方式：原下拉框改为按钮组 —— 与同类操作按钮同一外观，且一步直达（少一次展开） */}
              <div className="grid grid-cols-4 gap-0.5" title={t("mindmap.layoutDir")}>
                {(Object.keys(LAYOUT_DIR_KEYS) as LayoutDir[]).map((k) => (
                  <button key={k} type="button" onClick={() => changeDir(k)} title={t(LAYOUT_DIR_KEYS[k])}
                    className={`${button} justify-center px-0 ${dir === k ? "border-white/25 bg-white/[0.12] text-white" : ""}`}>
                    {LAYOUT_DIR_ICONS[k]("h-3 w-3")}
                  </button>
                ))}
              </div>
              {/* 关系线模式：拖线在任意两节点之间加「额外关系线」（不改父子关系） */}
              <button type="button" className={`${button} w-full justify-start ${linkMode ? "border-cyan-400/40 bg-cyan-400/15 text-cyan-200" : ""}`}
                onClick={() => setLinkMode(v => { if (v) setLinkFrom(null); return !v; })} title={t("mindmap.linkModeHint")}><Link2 className="h-3 w-3" />{t("mindmap.linkMode")}</button>
              <button type="button" disabled={!selectedId} className={`${button} w-full justify-start`} onClick={focusSelected} title={t("mindmap.focusSelected")}><LocateFixed className="h-3 w-3" />{t("mindmap.focusSelected")}</button>
              <button type="button" className={`${button} w-full justify-start`} onClick={() => addChildNode()} title={t("mindmap.addChild")}><Plus className="h-3 w-3" />{t("mindmap.childNode")}</button>
              <button type="button" className={`${button} w-full justify-start`} onClick={() => addNode(null)} title={t("mindmap.newRootNode")}><ListTree className="h-3 w-3" />{t("mindmap.newRootNode")}</button>
              {/* AI 入口：与第一栏底部的「AI 分析」同一强调色，两栏同一件事不该有两种颜色 */}
              <button type="button" className={`${button} w-full justify-start`} style={{ color: ACCENT, borderColor: `${ACCENT}55` }} onClick={onAiProject} title={t("mindmap.aiUnifiedTitle")}><Brain className="h-3 w-3" />{t("mindmap.aiUnifiedBtn")}</button>
              <button type="button" className={`${button} w-full justify-start`} onClick={() => addSticker()} title={t("mindmap.textSticker")}><StickyNote className="h-3 w-3" />{t("mindmap.textSticker")}</button>
              <button type="button" className={`${button} w-full justify-start`} onClick={() => void addImageSticker()} title={t("mindmap.imageSticker")}><Image className="h-3 w-3" />{t("mindmap.imageSticker")}</button>
              {/* 导出 Markdown：针对当前这份文档，所以放在画布侧而不是文档列表栏 */}
              <button type="button" className={`${button} w-full justify-start`} onClick={onExportMd} title={t("mindmap.exportMd")}><ScrollText className="h-3 w-3" />{t("mindmap.exportMd")}</button>
            </div>
            {/* 自动保存指示 */}
            {lastSaved && (
              <div className="rounded-ctl border border-emerald-400/20 bg-emerald-950/60 px-2 py-1 text-[8px] text-emerald-300 shadow-lg">
                {t("mindmap.autoSaved", { time: new Date(lastSaved).toLocaleTimeString("zh-CN", { hour12: false }) })}
              </div>
            )}
            {/* Keyboard hints */}
            {selectedId && (
              <div className="rounded-ctl border border-white/10 bg-slate-900/95 p-1.5 shadow-lg text-[8px] text-slate-600 leading-relaxed">
                <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Tab</kbd> {t("mindmap.kbdChild")}</div>
                <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">↑↓←→</kbd> {t("mindmap.kbdNavigate")}</div>
                <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Enter</kbd> {t("mindmap.kbdDetail")}</div>
                <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Del</kbd> {t("mindmap.kbdDelete")}</div>
                <div><kbd className="rounded border border-white/15 px-1 py-0.5 text-[7px] text-slate-400">Esc</kbd> {t("mindmap.kbdCancel")}</div>
              </div>
            )}
          </div>
        </div>
        {/* 右缘拖宽把手：与第一栏、第四栏同一套逻辑 */}
        {treeOpen && (
          <div
            className="pointer-events-auto absolute -right-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]"
            title={t("mindmap.dragResize")}
            onMouseDown={(e) => {
              if (e.button !== 0) return;
              e.preventDefault();
              const startX = e.clientX; const startW = treeW;
              const onMove = (ev: MouseEvent) => setTreeW(startW + (ev.clientX - startX));
              const onUp = () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
              window.addEventListener("mousemove", onMove);
              window.addEventListener("mouseup", onUp);
            }} />
        )}
      </div>

      {/* 关系线弹层：新建时输入说明文字，编辑时改文字或删除 */}
      {linkDraft && (() => {
        const fromName = byId.get(linkDraft.sourceId)?.name ?? linkDraft.sourceId;
        const toName = byId.get(linkDraft.targetId)?.name ?? linkDraft.targetId;
        return (
          <div className="fixed z-[220] w-[248px] rounded-ctl border border-cyan-400/25 bg-surface-modal p-2 shadow-2xl"
            style={{ left: Math.max(8, Math.min(linkDraft.x, window.innerWidth - 256)), top: Math.max(8, Math.min(linkDraft.y, window.innerHeight - 132)) }}
            onClick={e => e.stopPropagation()} onMouseDown={e => e.stopPropagation()}>
            <div className="mb-1.5 flex items-center gap-1 text-tiny text-slate-400">
              <Link2 className="h-3 w-3 shrink-0 text-cyan-300" />
              <span className="min-w-0 truncate" title={`${fromName} → ${toName}`}>{t("mindmap.linkTitle", { from: fromName, to: toName })}</span>
            </div>
            <input autoFocus value={linkDraft.label} placeholder={t("mindmap.linkLabelPh")}
              onChange={(e) => setLinkDraft(prev => prev ? { ...prev, label: e.target.value } : prev)}
              onKeyDown={(e) => {
                if (e.key === "Enter") { e.preventDefault(); saveLinkDraft(); }
                else if (e.key === "Escape") { e.preventDefault(); setLinkDraft(null); }
              }}
              className="w-full rounded border border-white/10 bg-slate-900/80 px-2 py-1 text-caption text-slate-200 outline-none focus:border-cyan-400/60" />
            <div className="mt-2 flex items-center justify-between gap-1">
              <span className="text-[8px] text-slate-600">{t("mindmap.linkHint")}</span>
              <div className="flex shrink-0 items-center gap-1">
                {!linkDraft.isNew && (
                  <button type="button" className="rounded border border-red-500/30 bg-red-500/10 px-2 py-1 text-tiny text-red-300 hover:bg-red-500/20"
                    onClick={() => removeLink(linkDraft.id)}>{t("mindmap.delete")}</button>
                )}
                <button type="button" className="rounded border border-white/10 bg-white/[0.05] px-2 py-1 text-tiny text-slate-300 hover:bg-white/[0.1] hover:text-white"
                  onClick={() => setLinkDraft(null)}>{t("mindmap.cancel")}</button>
                <button type="button" className="rounded bg-cyan-500 px-2 py-1 text-tiny font-semibold text-slate-950 hover:bg-cyan-400"
                  onClick={saveLinkDraft}>{t("mindmap.save")}</button>
              </div>
            </div>
          </div>
        );
      })()}

      {ctxMenu && (
        <MindmapNodeContextMenu
          ctxMenu={ctxMenu}
          byId={byId}
          setDetailNode={setDetailNode}
          setCtxMenu={setCtxMenu}
          addChildNode={addChildNode}
          makeRoot={makeRoot}
          full={full}
          deleteNode={deleteNode}
        />)}
      {/* 悬浮只读预览气泡（跟随鼠标；鼠标移入气泡后保持打开，可滚动查看长内容） */}
      {hp.preview && (
        <HoverPreview
          preview={hp.preview}
          hoverGuard={hp.hoverGuard}
        />
      )}
      {detailNode && <DetailModal node={detailNode} accent={accent} projectRoot={full.document.sourceDesc} onUpdate={updateNode} onClose={() => setDetailNode(null)} />}
    </div>
  );
}
