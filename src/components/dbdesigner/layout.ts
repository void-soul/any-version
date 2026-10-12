// 画布自动布局：先按「业务系列」分块，块内再按父子分层。
//
// 逆向出来的设计最常见的问题是「所有表叠在左上角密密麻麻」—— 反推只写结构、不摆位置。
//
// 为什么不引 dagre / elkjs：布局结果会**写进设计文件**（node.x/y），必须稳定可复现 ——
// 「同一份设计，布局两次要长得一样」。力导向每次结果都不同，不适合当默认摆法。
// 想看漂亮的力导向图，用户可以自己拖。
//
// 为什么按系列分块（这是连线不再打结的关键）：早期把所有表丢进同一套分层，
// 于是 orders 系的表和 log 系的表交错排在同一层，连线横穿整个画布，看着就是一团乱麻。
// 现在每个系列独立布局、系列之间垂直堆叠 —— 不相关系列物理隔离，连线永远不跨块。

import type { DbDesignDocument, DbDesignNode } from "./types";
import { groupByForeignKey } from "./series";

/** 节点卡片宽度（与 DesignNodeCard 的 w-[288px] 对齐；改那边记得改这里） */
const CARD_W = 288;
/**
 * 层间距（X 方向）：卡片 288 宽，**留够走线的走廊**。
 * 早先只留 88，连线没有走廊可用，只能贴着卡片绕、或者干脆垂直穿越一堆表 ——
 * 间距不够时，再好的分层算法都会被连线噪声淹没。
 */
const GAP_X = 200;
const GAP_Y = 64;
/** 两个系列块之间的纵向间距：给连线留出走线的余量 */
const GAP_SERIES = 180;
const PAD = 40;
/** 卡片高度估算：字段一行 ≈ 18px，表头 ≈ 46px（与卡片实际样式同量级即可，宁可高估） */
const ROW_H = 18;
const HEADER_H = 58;

export interface LayoutOptions {
  /** 常驻展开时按全部字段算高度，折叠时只按主键算 —— 算矮了卡片会互相压住 */
  alwaysExpand: boolean;
  /** 折叠态显示哪些字段（keys = 只主键+外键） */
  fieldRows: "keys" | "all";
}

/** 这张表在当前显示模式下会渲染几行字段 */
function visibleFieldCount(node: DbDesignNode, opts: LayoutOptions): number {
  const fields = node.table?.fields ?? [];
  if (opts.alwaysExpand || opts.fieldRows === "all") return fields.length;
  return fields.filter((f) => f.pk).length;
}

function estimateHeight(node: DbDesignNode, opts: LayoutOptions): number {
  const rows = Math.max(1, visibleFieldCount(node, opts));
  return HEADER_H + rows * ROW_H + (node.comment ? 18 : 0) + 12;
}

/** 一个系列的布局结果：坐标 + 该块占位尺寸（用于垂直堆叠下一块） */
interface GroupLayout {
  positions: Record<string, { x: number; y: number }>;
  height: number;
}

/**
 * 单个系列内部：BFS 分层 + 层内「兄弟相邻」网格。
 *
 * 层号用**最长路径**而不是 BFS 最短路径：BFS 会把「直连父节点的子表」和
 * 「隔了两层的孙表」放进同一层，于是父子之间出现一条横跨整层的回折线。
 * 最长路径保证「子节点严格比父节点深一层」，每一根连线都只往右走一段，不折返。
 * 成环靠「只认第一次到达 + 迭代收敛」兜底，不会死循环。
 *
 * 层内再按「父节点是谁」排序：同一个父的子表连着排。早期直接按文档顺序铺网格，
 * 一个父表的几个子表被拆到网格不同角落，从父表拉出的线就成了扇形乱麻 ——
 * 这是连线乱的第二个根因，且比换线条类型管用得多。
 */
function layoutGroup(
  group: DbDesignNode[],
  doc: DbDesignDocument,
  opts: LayoutOptions,
): GroupLayout {
  const positions: Record<string, { x: number; y: number }> = {};
  const idSet = new Set(group.map((n) => n.id));

  const byId = (id: string) => group.find((x) => x.id === id)!;
  /** 文档次序（按 id），各种排序的稳定兜底 */
  const indexOfId = (id: string) => originalIndex(byId(id));

  const children = new Map<string, string[]>();
  for (const r of doc.relations) {
    // n-n 不参与分层：它在画布上表现为一张中间表，参与只会把布局搅乱
    if (r.kind === "n-n") continue;
    if (!idSet.has(r.to.node) || !idSet.has(r.from.node) || r.to.node === r.from.node) continue;
    const list = children.get(r.to.node) ?? [];
    if (!list.includes(r.from.node)) list.push(r.from.node);
    children.set(r.to.node, list);
  }

  // 层号 = 从根出发的最长路径长度。逐轮松弛：有更深的来路就下沉，
  // 用「本轮是否还在变」收敛；成环时每轮还会涨，但轮数有上限（不会死循环）。
  const depth = new Map<string, number>();
  for (const n of group) depth.set(n.id, 0);
  for (let round = 0; round < group.length; round++) {
    let changed = false;
    for (const n of group) {
      const d = depth.get(n.id) ?? 0;
      for (const child of children.get(n.id) ?? []) {
        // 子节点必须比父节点深一层：比现在更深才更新
        if ((depth.get(child) ?? 0) < d + 1) {
          depth.set(child, d + 1);
          changed = true;
        }
      }
    }
    if (!changed) break;
  }

  // ── 树布局：后序叶子计数（替换早先的「层内网格」）───────────────────
  //
  // 网格的致命伤：同一层 N 张表被 ceil(sqrt(N*1.4)) 列的格子填充，落在末尾的表
  // （比如 purchase_order）会被推到角落，离它引用的父表隔了大半个网格 —— 截图里
  // 「几张表孤立漂浮在底部」就是这么来的。
  //
  // 正确的做法与思维导图 layoutTree 同源：**后序叶子计数**。叶子依次占一个槽位，
  // 父节点居中于首末子节点之间。每个子树在堆叠轴上连续，父子必然紧凑，
  // 不需要再去猜「这个父表该排第几行」。
  //
  // 但**没有子树的根**（典型是逆向 dump 里一堆互不相干的表）仍然走网格：它们之间
  // 没有关系，纵向排成一条竖线会把画布拉得又高又窄，横向铺开才像一张总览图。
  //
  // ER 图不是树（多对多、有环），所以先提取「主树」：每个节点只保留一个主父节点
  // （引用它的、且恰好浅一层的那个），再按主树布局。其余连线照常画，不参与排位。

  // 1) 提取主树：每个节点的「主父」= 引用它的、且恰好浅一层的节点。
  //    并列时用文档次序兜底，保证同一份设计两次布局长得一样。
  const mainParent = new Map<string, string | null>();
  for (const n of group) {
    const d = depth.get(n.id) ?? 0;
    if (d === 0) {
      mainParent.set(n.id, null);
      continue;
    }
    let best: string | null = null;
    let bestScore = Number.MAX_SAFE_INTEGER;
    for (const r of doc.relations) {
      if (r.from.node !== n.id || !idSet.has(r.to.node) || r.kind === "n-n") continue;
      const pd = depth.get(r.to.node) ?? 0;
      // 主父必须在浅一层（d-1），否则就是「跨层回折线」的来源
      if (pd !== d - 1) continue;
      const score = indexOfId(r.to.node);
      if (score < bestScore) {
        bestScore = score;
        best = r.to.node;
      }
    }
    mainParent.set(n.id, best);
  }

  // 2) 主树的反向映射（父 → 子），兄弟按文档次序排序保证稳定
  const treeChildren = new Map<string, string[]>();
  for (const [child, parent] of mainParent) {
    if (!parent) continue;
    const list = treeChildren.get(parent) ?? [];
    list.push(child);
    treeChildren.set(parent, list);
  }
  for (const list of treeChildren.values()) list.sort((a, b) => indexOfId(a) - indexOfId(b));

  // 有子树的根 → 各占一条纵向带；「光杆根」→ 汇到末尾统一排网格
  const roots = group
    .filter((n) => mainParent.get(n.id) === null)
    .sort((a, b) => originalIndex(a) - originalIndex(b));
  const treeRoots = roots.filter((r) => (treeChildren.get(r.id) ?? []).length > 0);
  const looseRoots = roots.filter((r) => (treeChildren.get(r.id) ?? []).length === 0);

  // 3) 后序遍历：叶子占槽位，父节点居中于首末子节点之间
  let cursor = PAD;
  const place = (id: string, d: number): { top: number; bottom: number } => {
    const n = byId(id);
    const selfH = estimateHeight(n, opts);
    const kids = treeChildren.get(id) ?? [];
    if (kids.length === 0) {
      positions[id] = { x: PAD + d * (CARD_W + GAP_X), y: cursor };
      const bottom = cursor + selfH;
      cursor = bottom + GAP_Y;
      return { top: positions[id].y, bottom };
    }
    const tops: number[] = [];
    const bottoms: number[] = [];
    for (const k of kids) {
      const { top, bottom } = place(k, d + 1);
      tops.push(top);
      bottoms.push(bottom);
    }
    // 父节点居中于首末子节点：父子落在同一视野带里，连线是短横线而不是斜跨
    const top = Math.min(...tops);
    const bottom = Math.max(...bottoms);
    positions[id] = { x: PAD + d * (CARD_W + GAP_X), y: (top + bottom) / 2 - selfH / 2 };
    return { top, bottom };
  };
  // 一个子树 = 一条纵向带，带与带之间再留一道空隙
  for (const r of treeRoots) {
    place(r.id, 0);
    cursor += GAP_Y;
  }

  // 4) 光杆根铺成网格：互不相干的表排成一条竖线，既难看又白费屏高
  if (looseRoots.length > 0) {
    const cols = Math.max(1, Math.ceil(Math.sqrt(looseRoots.length * 1.4)));
    const rowH = Math.max(...looseRoots.map((n) => estimateHeight(n, opts)));
    looseRoots.forEach((n, i) => {
      const col = i % cols;
      const row = Math.floor(i / cols);
      positions[n.id] = { x: PAD + col * (CARD_W + GAP_X), y: cursor + row * (rowH + GAP_Y) };
    });
  }

  let maxY = 0;
  for (const n of group) maxY = Math.max(maxY, positions[n.id].y + estimateHeight(n, opts));
  return { positions, height: maxY };
}

/** 节点在文档中的原始次序：作为排序的稳定兜底，避免同父兄弟之间的顺序飘 */
function originalIndex(n: DbDesignNode): number {
  return parseInt(n.id.replace(/\D+/g, ""), 10) || 0;
}

/**
 * 算每个节点的落点（node id → 坐标）。
 *
 * 分两步：**先按业务系列分块并垂直堆叠**，块内各自分父子层。
 * 这样「不相干的表不会互相挡道」「同一系列父子挨着站」，连线自然就不会打结 ——
 * 比单纯把线条换成正交折线有效得多。
 */
export function computeLayout(
  doc: DbDesignDocument,
  opts: LayoutOptions,
): Record<string, { x: number; y: number }> {
  const nodes = doc.nodes.filter((n) => n.kind === "table");
  if (nodes.length === 0) return {};

  const groups = groupByForeignKey(doc, nodes);

  // 孤立表（与任何表都没关系）若各自成块，会被顺次堆叠成一条线 —— 而它们无关缘线，
  // 本来就应该以网格排并到一块里去（逆向 dump 里“一堆无关联表”很常见）。
  const linked = new Set<string>();
  for (const r of doc.relations) {
    if (r.kind === "n-n") continue;
    linked.add(r.from.node);
    linked.add(r.to.node);
  }
  const isIsolated = (g: DbDesignNode[]) => g.length === 1 && !linked.has(g[0].id);
  const isolated = groups.filter(isIsolated).flat();
  const blocks = groups.filter((g) => !isIsolated(g));
  if (isolated.length > 0) blocks.push(isolated);

  const out: Record<string, { x: number; y: number }> = {};
  let cursorY = 0;
  for (const group of blocks) {
    const { positions, height } = layoutGroup(group, doc, opts);
    for (const [id, p] of Object.entries(positions)) {
      out[id] = { x: p.x, y: p.y + cursorY };
    }
    cursorY += height + GAP_SERIES;
  }
  return out;
}
