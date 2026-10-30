import { withDescendants, pointInRect, type PosOverride } from "./MindmapPanel.model";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Position, type Node, type FitView } from "@xyflow/react";
import { graphScope } from "../../utils/graphScope";
import { DocumentFull, MindmapNode } from "./types";
import { isLayoutDir, layoutTree, type LayoutDir } from "./layout";

/** Owns the MindmapCanvasLayout feature, independently of page rendering. */
export function useMindmapCanvasLayout({ full, measuredMap, hlDepth, fitView, treeOpen, posOverrides, flowToScreenPosition }: {
  posOverrides: Record<string, PosOverride>;
  flowToScreenPosition: (position: { x: number; y: number }) => { x: number; y: number };
  full: DocumentFull;
  measuredMap: Record<string, { width: number; height: number; }>;
  hlDepth: 1 | 2 | 3 | 4 | 5;
  fitView: FitView<Node>;
  treeOpen: boolean;
}) {

  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const [selectedId, setSelectedId] = useState<string | null>(null);

  const treeListRef = useRef<HTMLDivElement | null>(null);

  const graphNodes = full.nodes;

  const stickers = full.stickers;

  // 自由关系线（旧文档可能没有该字段）
  const links = useMemo(() => full.links ?? [], [full.links]);

  const byId = useMemo(() => new Map(graphNodes.map((n) => [n.id, n])), [graphNodes]);

  // 布局方向（画布级设置）：切换后自动重排并持久化；初始值取该文档保存的方向
  const [dir, setDir] = useState<LayoutDir>(() => (isLayoutDir(full.document.layoutDir) ? full.document.layoutDir : "lr"));

  // 把实测尺寸喂给布局：行高按真实卡片高度累计。
  // 不这么做就只能拿固定 90 当卡片高 —— 矮卡片之间白留一大截、超过 90 的卡片还会互相压住。
  const measuredSizes = useMemo(
    () => new Map(Object.entries(measuredMap)),
    [measuredMap],
  );

  const layout = useMemo(() => layoutTree(graphNodes, dir, measuredSizes), [graphNodes, dir, measuredSizes]);

  const endpointPositions = dir === "tb"
    ? { target: Position.Top, source: Position.Bottom }
    : dir === "bt"
      ? { target: Position.Bottom, source: Position.Top }
      : dir === "rl"
        ? { target: Position.Right, source: Position.Left }
        : { target: Position.Left, source: Position.Right };

  const childrenCount = useMemo(() => { const m = new Map<string, number>(); for (const n of graphNodes) { if (n.parentId) m.set(n.parentId, (m.get(n.parentId) ?? 0) + 1); } return m; }, [graphNodes]);

  // 每个节点的全部后代数（折叠徽标用：折叠后隐藏了多少个节点）
  const descendantCount = useMemo(() => {
    const children = new Map<string, string[]>();
    for (const n of graphNodes) { if (n.parentId) { const l = children.get(n.parentId) ?? []; l.push(n.id); children.set(n.parentId, l); } }
    const memo = new Map<string, number>();
    const count = (id: string, seen: Set<string>): number => {
      const cached = memo.get(id); if (cached !== undefined) return cached;
      seen.add(id);
      let total = 0;
      for (const c of children.get(id) ?? []) { if (!seen.has(c)) total += 1 + count(c, seen); }
      memo.set(id, total);
      return total;
    };
    const m = new Map<string, number>();
    for (const n of graphNodes) m.set(n.id, count(n.id, new Set()));
    return m;
  }, [graphNodes]);

  /**
   * 关联节点集合 + 关联连线集合：**一次 BFS 同时得出**（共享层 `utils/graphScope`），
   * 与数据库设计器同一口径。
   *
   * 层级 = 从选中项出发能走多远，默认 1 跳（直接父 / 子 / 关系线两端），可调到 5 跳。
   * 早先这里是手写的一跳集合，用户没有「看远一点」的开关 —— 而大导图上
   * 「选中一个模块、看它两跳内的邻居」是常见需求。
   *
   * 为什么不用祖先链当「关联」：祖先链会把「亮一条到根的路」当成高亮，选中一个深处
   * 的节点时亮起一大片，注意力反而收不回来。`highlightChain` 仍然保留 ——
   * 它服务于滚动同步与视口聚焦，只是不再参与高亮。
   *
   * 边也必须是同一次遍历的产物：早先连线用「两端有一端在集合内」判定，会点亮同跳层
   * 内部的横向连线，产出「线亮着、两端的节点却暗着」。
   */
  const scope = useMemo(() => {
    // 边 id 必须与下面 `edges` 里用的一致，否则连线查不到自己那条的高亮状态
    const scopeEdges: { id: string; source: string; target: string }[] = [];
    for (const n of graphNodes) {
      if (n.parentId && n.parentId !== n.id) scopeEdges.push({ id: `mm-e-${n.id}`, source: n.parentId, target: n.id });
    }
    for (const l of links) scopeEdges.push({ id: `mm-l-${l.id}`, source: l.sourceId, target: l.targetId });
    return graphScope(graphNodes.map((n) => n.id), scopeEdges, selectedId, hlDepth);
  }, [graphNodes, links, selectedId, hlDepth]);

  const relatedIds = useMemo(() => new Set(scope.hops.keys()), [scope]);

  /** 连线离选中项几跳：远的那端画得更暗，否则层级调到 3 跳与 1 跳画出来一样 */
  const hopOfEdge = useCallback(
    (a: string, b: string) => {
      if (selectedId === null) return undefined;
      const hopA = scope.hops.get(a);
      const hopB = scope.hops.get(b);
      return hopA === undefined ? hopB : hopB === undefined ? hopA : Math.min(hopA, hopB);
    },
    [scope, selectedId],
  );

  const visibleNodes = useMemo(() => {
    // 一个节点可见 ⟺ 它的所有祖先都未被折叠（节点自身的折叠状态只隐藏其后代，不隐藏自己）。
    // 用记忆化 DFS 判定，O(N)。
    //
    // 旧实现用「从根 visit + 环/孤立节点兜底 forEach visit」，兜底循环会把「被折叠节点
    // 的子节点」也重新 visit 加回可见集（因为它们此时尚未 visited），导致折叠只高亮、
    // 子节点仍显示。这里改为独立判定每个节点的可见性，从根上消除该 bug。
    const visMemo = new Map<string, boolean>();
    const isVisible = (n: MindmapNode): boolean => {
      const cached = visMemo.get(n.id);
      if (cached !== undefined) return cached;
      // 先占位 true 防环（思维导图本应是树；万一有环按可见处理，避免无限递归）
      visMemo.set(n.id, true);
      let visible = true;
      if (n.parentId && n.parentId !== n.id) {
        const parent = byId.get(n.parentId);
        if (parent) {
          // 父节点被折叠 → 本节点（及其整棵子树）隐藏；否则继承父节点可见性
          visible = collapsed.has(parent.id) ? false : isVisible(parent);
        }
        // 父节点不存在（孤儿节点）→ 可见
      }
      visMemo.set(n.id, visible);
      return visible;
    };
    return graphNodes.filter((n) => isVisible(n));
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
    // 环/孤立节点兜底：仅访问「未被折叠祖先隐藏」的节点。
    // 旧实现无此判断，会把「被折叠节点的子节点」也 visit 回来（它们此时尚未 visited），
    // 导致折叠后右侧树里子节点仍显示。
    const hiddenByCollapsedAncestor = (n: MindmapNode): boolean => {
      let cur = n;
      const seen = new Set<string>();
      while (cur.parentId && cur.parentId !== cur.id && !seen.has(cur.parentId)) {
        seen.add(cur.id);
        if (collapsed.has(cur.parentId)) return true;
        const parent = byId.get(cur.parentId);
        if (!parent) return false; // 孤儿（父不存在）→ 不被折叠隐藏
        cur = parent;
      }
      return false;
    };
    graphNodes.forEach(n => { if (!visited.has(n.id) && !hiddenByCollapsedAncestor(n)) visit(n, 0); });
    return out;
  }, [graphNodes, collapsed, byId]);

  // 点击树节点：展开其全部祖先（折叠集合移除）→ 选中 → 视口聚焦到该节点。
  // 注意：不能把聚焦定时器的 cleanup 返回给 onClick 立即调用——那会把 fitView 取消掉
  // （点击树节点选中但不移动视口的 bug）。聚焦用 60ms 延时，等布局稳定后再移动视口。
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

  // 节点树行首箭头：复用画布节点用的同一份 `collapsed` 集合，
  // 于是树上折叠 / 展开与画布是同一状态，不会出现「树折了画布还展开」。
  const toggleTreeCollapse = useCallback((id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  /** 全部展开：清空折叠集合（画布同步展开，因为两者共用同一份状态） */
  const expandAllTree = useCallback(() => setCollapsed(new Set()), []);

  /** 全部折叠：折叠所有「有子节点」的节点，只留顶层节点（叶子节点折叠无意义） */
  const collapseAllTree = useCallback(() => {
    setCollapsed(() => new Set(childrenCount.keys()));
  }, [childrenCount]);

  // 聚焦当前节点：保留当前缩放级别上限，避免单节点定位后突然放大到难以回看全局。
  const focusSelected = useCallback(() => {
    if (!selectedId || !byId.has(selectedId)) return;
    fitView({ nodes: [{ id: selectedId }], padding: 0.4, duration: 300, maxZoom: 1.2 });
  }, [selectedId, byId, fitView]);

  // 当前选中节点变化（画布点选/键盘导航）→ 树面板滚动到对应行，保持两侧行为同步。
  const selectedTreeIdx = navTree.findIndex(x => x.node.id === selectedId);

  useEffect(() => {
    if (selectedTreeIdx < 0 || !treeOpen) return;
    const list = treeListRef.current;
    if (!list) return;
    const row = list.querySelector<HTMLElement>(`[data-tree-idx="${selectedTreeIdx}"]`);
    row?.scrollIntoView({ block: "nearest" });
  }, [selectedTreeIdx, treeOpen]);

  // ── 自由关系线：新建 / 编辑 / 删除 ──

  /** 节点卡片中心的画布坐标（关系线弹层定位用）。 */
  const nodeCenterOf = useCallback((id: string): { x: number; y: number } | null => {
    const n = byId.get(id);
    if (!n) return null;
    const hasSaved = n.positionX !== 0 || n.positionY !== 0;
    const base = hasSaved ? { x: n.positionX, y: n.positionY } : layout.get(n.id) ?? { x: 0, y: 0 };
    const p = posOverrides[n.id] ?? base;
    const s = measuredMap[n.id] ?? { width: 200, height: 90 };
    return { x: p.x + s.width / 2, y: p.y + s.height / 2 };
  }, [byId, layout, posOverrides, measuredMap]);

  /** 两节点连线中点的屏幕坐标（新建关系线时给弹层定位；坐标缺失则退回屏幕中心）。 */
  const linkAnchorOnScreen = useCallback((sourceId: string, targetId: string): { x: number; y: number } => {
    const a = nodeCenterOf(sourceId);
    const b = nodeCenterOf(targetId);
    if (!a || !b) return { x: Math.round(window.innerWidth / 2), y: Math.round(window.innerHeight / 2) };
    return flowToScreenPosition({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 });
  }, [nodeCenterOf, flowToScreenPosition]);

  /** 拖拽中的节点中心落在哪个节点卡片里（拖到某节点上＝成为它的子节点）。 */
  const findDropTarget = useCallback((node: Node): string | null => {
    if (node.id.startsWith("sticker-")) return null;
    const self = byId.get(node.id);
    if (!self) return null;
    const ownSize = measuredMap[node.id] ?? node.measured ?? { width: 200, height: 90 };
    const cx = node.position.x + (ownSize.width ?? 0) / 2;
    const cy = node.position.y + (ownSize.height ?? 0) / 2;
    // 排除自己与自己的全部后代（否则成环），也排除当前上级（那是原地重排，不是改关系）
    const forbidden = withDescendants(node.id, graphNodes);
    for (let i = visibleNodes.length - 1; i >= 0; i--) {
      const target = visibleNodes[i];
      if (forbidden.has(target.id) || target.id === self.parentId) continue;
      const size = measuredMap[target.id];
      if (!size) continue;
      const pos = posOverrides[target.id] ?? (target.positionX !== 0 || target.positionY !== 0
        ? { x: target.positionX, y: target.positionY }
        : layout.get(target.id) ?? { x: 0, y: 0 });
      if (pointInRect(cx, cy, pos.x, pos.y, size.width, size.height)) return target.id;
    }
    return null;
  }, [byId, graphNodes, measuredMap, visibleNodes, posOverrides, layout]);
  return {
    linkAnchorOnScreen,
    findDropTarget,
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
  };
}
