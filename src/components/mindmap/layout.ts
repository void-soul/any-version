import type { MindmapNode } from "./types";

// ════════════ 自动布局（纯函数，可独立测试）════════════

/** 布局方向：lr=左→右（默认，根在左） rl=右→左 tb=上→下 bt=下→上 */
export type LayoutDir = "lr" | "rl" | "tb" | "bt";

export const isLayoutDir = (v: string): v is LayoutDir =>
  v === "lr" || v === "rl" || v === "tb" || v === "bt";

/** 节点卡片宽度，与 MindmapNodeCard 的 w-[200px] 一致。 */
const CARD_W = 200;
/** 节点卡片典型高度（标题栏 + 详情区，与 ReactFlow measured 的兜底值同为 90）。 */
const CARD_H = 90;

/**
 * 树形自动布局：沿 `dir` 方向按深度推进，另一轴按子树堆叠。
 *
 * 堆叠槽位用**后序叶子计数**：叶子依次占一个槽位，父节点居中于首末子节点之间。
 * 这样每个子树在堆叠轴上连续、父节点对齐自己的子节点。
 *
 * 早先是「同一深度里的第几个」全局计数（depthIndex），父节点的槽位和它的子节点
 * 毫无关系，越深的分支离自己的父节点漂移越远，整张图被拉成巨大的 Z 字形 ——
 * 要看全貌只能缩得很小。现在包围盒贴近树的真实体量。
 */
export function layoutTree(
  nodes: MindmapNode[],
  dir: LayoutDir = "lr",
): Map<string, { x: number; y: number }> {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const children = new Map<string, string[]>();
  const roots: string[] = [];
  for (const n of nodes) {
    if (n.parentId && byId.has(n.parentId) && n.parentId !== n.id) {
      const l = children.get(n.parentId) ?? [];
      l.push(n.id);
      children.set(n.parentId, l);
    } else {
      roots.push(n.id);
    }
  }

  // 深度：带 visited 防环（AI 生成的节点若存在循环引用 A→B→A，无保护会卡死）
  const depth = new Map<string, number>();
  const visited = new Set<string>();
  const dfs = (id: string, d: number) => {
    if (visited.has(id)) return;
    visited.add(id);
    depth.set(id, d);
    for (const c of children.get(id) ?? []) dfs(c, d + 1);
  };
  for (const r of roots) dfs(r, 0);
  // 未被根遍历到的节点（环内）兜底放入布局，避免遗漏
  for (const n of nodes) if (!visited.has(n.id)) dfs(n.id, 0);

  // 堆叠槽位（后序）：叶子依次占位，父节点居中于首末子节点
  const slot = new Map<string, number>();
  let cursor = 0;
  const visiting = new Set<string>();
  const place = (id: string): number => {
    const done = slot.get(id);
    if (done !== undefined) return done;
    if (visiting.has(id)) {
      // 成环：就地占一个槽位断开递归（脏数据也要能画出图，不能卡死）
      const s = cursor++;
      slot.set(id, s);
      return s;
    }
    visiting.add(id);
    const kids = children.get(id) ?? [];
    let s: number;
    if (kids.length === 0) {
      s = cursor++;
    } else {
      const kidSlots = kids.map(place);
      s = (Math.min(...kidSlots) + Math.max(...kidSlots)) / 2;
    }
    visiting.delete(id);
    slot.set(id, s);
    return s;
  };
  for (const r of roots) place(r);
  for (const n of nodes) if (!slot.has(n.id)) place(n.id);

  // 沿展开方向的层间距：卡片宽/高 + 连线空隙（早先 260/200 偏松，缩到 24/86）
  const depthStep = dir === "tb" || dir === "bt" ? CARD_H + 86 : CARD_W + 24;
  // 堆叠方向的间距：卡片实测 90~110px 高，+30 才不会上下叠在一起（+10 会互相盖住）
  const stackStep = dir === "tb" || dir === "bt" ? CARD_W + 24 : CARD_H + 30;

  const pos = new Map<string, { x: number; y: number }>();
  for (const n of nodes) {
    const along = (depth.get(n.id) ?? 0) * depthStep;
    const across = (slot.get(n.id) ?? 0) * stackStep;
    switch (dir) {
      case "rl": pos.set(n.id, { x: -along, y: across }); break;
      case "tb": pos.set(n.id, { x: across, y: along }); break;
      case "bt": pos.set(n.id, { x: across, y: -along }); break;
      default:  pos.set(n.id, { x: along, y: across });
    }
  }
  return pos;
}
