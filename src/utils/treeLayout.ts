/**
 * 层级树自动布局：**思维导图与 JSON 浏览共用一套算法**。
 *
 * 为什么抽到共享层：两处画的都是「按层级从根往叶展开、父节点居中于自己的子节点」，
 * 但早先各写一份 —— JSON 浏览那份是 `depth * 260` 的硬编码网格：父节点与它的子节点
 * 在堆叠轴上毫无关系，键一多整张图就散成一片竖条，看不出谁是谁的父。
 * 与其把「后序叶子计数」那份再抄一遍，不如两边共用一份。
 *
 * 堆叠槽位用**后序叶子计数**：叶子依次占一个槽位，父节点居中于首末子节点之间。
 * 这样每个子树在堆叠轴上连续、父节点对齐自己的子节点 —— 早先按「同一深度里的第几个」
 * 全局计数（depthIndex），父节点的槽位和它的子节点毫无关系，越深的分支离自己的
 * 父节点漂移越远，整张图被拉成巨大的 Z 字形，要看全貌只能缩得很小。
 *
 * 用法上刻意**只认 id / parentId**（外加可选实测尺寸），所以任何「父指针 + 一堆兄弟」
 * 的数据都能直接喂进来，不必先转换成某个模块的节点类型。
 */

/** 布局方向：lr=左→右（根在左） rl=右→左 tb=上→下 bt=下→上 */
export type TreeLayoutDir = "lr" | "rl" | "tb" | "bt";

export const isTreeLayoutDir = (v: string): v is TreeLayoutDir =>
  v === "lr" || v === "rl" || v === "tb" || v === "bt";

/** 算法只需要这些：树结构（id / parentId）+ 实测尺寸 */
export interface LayoutNode {
  id: string;
  parentId?: string | null;
}

export interface TreeLayoutSizes {
  /** 卡片宽（未实测时的兜底值） */
  cardW: number;
  /** 卡片高（未实测时的兜底值） */
  cardH: number;
  /** 展开方向的「连线走廊」：层间距 = 最长的卡片 + 它 */
  levelGap: number;
  /** 堆叠方向的呼吸空间 */
  stackGap: number;
}

export function treeLayout(
  nodes: LayoutNode[],
  dir: TreeLayoutDir = "lr",
  /**
   * React Flow 实测的节点尺寸。给了就按**真实尺寸**排行高 —— 卡片高度是内容驱动的
   * （标题栏 + 若干行内容），用固定值估会白留空隙、超高的卡片还会压住邻居。
   */
  measured?: Map<string, { width: number; height: number }>,
  sizes: TreeLayoutSizes = { cardW: 200, cardH: 90, levelGap: 56, stackGap: 22 },
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
    vertical ? (measured?.get(id)?.height ?? sizes.cardH) : (measured?.get(id)?.width ?? sizes.cardW);
  const acrossSize = (id: string) =>
    vertical ? (measured?.get(id)?.width ?? sizes.cardW) : (measured?.get(id)?.height ?? sizes.cardH);

  // 层距 = **最长的**卡片 + 走廊：短卡片不白留，长卡片也不能压到下一层
  let depthExtent = vertical ? sizes.cardH : sizes.cardW;
  for (const n of nodes) depthExtent = Math.max(depthExtent, alongSize(n.id));
  const depthStep = depthExtent + sizes.levelGap;

  // 堆叠位置（后序）：叶子依次占一行，**行高按该行卡片的实测长度**累计；
  // 父节点居中于自己的首末子节点。
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
      cursor += acrossSize(id) + sizes.stackGap;
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
