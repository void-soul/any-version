import type { MindmapNode } from "./types";

// ════════════ 自动布局（纯函数，可独立测试）════════════

/** 布局方向：lr=左→右（默认，根在左） rl=右→左 tb=上→下 bt=下→上 */
export type LayoutDir = "lr" | "rl" | "tb" | "bt";

export const isLayoutDir = (v: string): v is LayoutDir =>
  v === "lr" || v === "rl" || v === "tb" || v === "bt";

/** 节点卡片宽度，与 MindmapNodeCard 的 w-[200px] 一致。 */
const CARD_W = 200;
/** 节点卡片高度的**兜底**值（React Flow 还没测到时的估计）。 */
const CARD_H = 90;

/**
 * 层间距：沿展开方向，卡片之间留出的「连线走廊」。
 *
 * 24 太窄 —— 卡片宽 200，走廊只剩 24px，连线的箭头几乎贴着下一张卡片的边框。
 */
const LEVEL_GAP = 56;

/**
 * 兄弟间距：堆叠方向上卡片之间的呼吸空间。
 *
 * 早先固定 30（配合固定行高 90 → 步长 120）。但卡片高度是**内容驱动**的
 * （标题栏 33 + 0~3 行证据文件，实测 56~110px），一律按 90 留白的结果是
 * 矮卡片之间空一大截、而超过 90 的卡片反而会互相压住。
 * 现在行高由**该行卡片的实测高度**决定，间隙只需保证不粘连。
 */
const STACK_GAP = 22;

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
  /**
   * React Flow 实测的节点尺寸。给了就按**真实尺寸**排行高 —— 卡片高度是内容驱动的
   * （标题栏 + 0~3 行证据文件），用固定 90 估会白留空隙、超高的卡片还会压住邻居。
   * 缺省时退回 CARD_W / CARD_H（首帧 RF 还没测量时就是这条路）。
   */
  measured?: Map<string, { width: number; height: number }>,
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

  // 卡片在两个轴上的「长度」：展开方向决定层距，堆叠方向决定行高。
  // 横向布局（lr/rl）展开方向看宽度、堆叠方向看高度；纵向（tb/bt）恰好相反。
  const vertical = dir === "tb" || dir === "bt";
  const alongSize = (id: string) =>
    vertical ? (measured?.get(id)?.height ?? CARD_H) : (measured?.get(id)?.width ?? CARD_W);
  const acrossSize = (id: string) =>
    vertical ? (measured?.get(id)?.width ?? CARD_W) : (measured?.get(id)?.height ?? CARD_H);

  // 层距 = **最长的**卡片 + 走廊：短卡片不白留，长卡片也不能压到下一层
  let depthExtent = vertical ? CARD_H : CARD_W;
  for (const n of nodes) depthExtent = Math.max(depthExtent, alongSize(n.id));
  const depthStep = depthExtent + LEVEL_GAP;

  // 堆叠位置（后序）：叶子依次占一行，**行高按该行卡片的实测长度**累计；
  // 父节点居中于自己的首末子节点。早先按固定 slot 序数乘以步长 —— 卡片比估计值矮就白留一大截，
  // 比估计值高就互相压住。
  const across = new Map<string, number>();
  let cursor = 0;
  const visiting = new Set<string>();
  const place = (id: string): number => {
    const done = across.get(id);
    if (done !== undefined) return done;
    const kids = children.get(id) ?? [];
    let side: number;
    if (kids.length === 0 || visiting.has(id)) {
      // 叶子；或成环的脏数据 —— 就地占一行断开递归（脏数据也要能画出图，不能卡死）
      side = cursor;
      cursor += acrossSize(id) + STACK_GAP;
    } else {
      visiting.add(id);
      const sides = kids.map(place);
      visiting.delete(id);
      // 居中于首末子节点：与「位置值取平均」的既有约定一致（等行高时就是几何中心）
      side = (Math.min(...sides) + Math.max(...sides)) / 2;
    }
    across.set(id, side);
    return side;
  };
  for (const r of roots) place(r);
  for (const n of nodes) if (!across.has(n.id)) place(n.id);

  const pos = new Map<string, { x: number; y: number }>();
  for (const n of nodes) {
    const along = (depth.get(n.id) ?? 0) * depthStep;
    const side = across.get(n.id) ?? 0;
    switch (dir) {
      case "rl": pos.set(n.id, { x: -along, y: side }); break;
      case "tb": pos.set(n.id, { x: side, y: along }); break;
      case "bt": pos.set(n.id, { x: side, y: -along }); break;
      default:  pos.set(n.id, { x: along, y: side });
    }
  }
  return pos;
}
