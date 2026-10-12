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

  const children = new Map<string, string[]>();
  const isChild = new Set<string>();
  for (const r of doc.relations) {
    // n-n 不参与分层：它在画布上表现为一张中间表，参与只会把布局搅乱
    if (r.kind === "n-n") continue;
    if (!idSet.has(r.to.node) || !idSet.has(r.from.node) || r.to.node === r.from.node) continue;
    const list = children.get(r.to.node) ?? [];
    if (!list.includes(r.from.node)) list.push(r.from.node);
    children.set(r.to.node, list);
    isChild.add(r.from.node);
  }

  // 层号 = 从根出发的最长路径长度。逐轮松弛：有更深的来路就下沉，
  // 用「本轮是否还在变」收敛，成环时最多多跑一轮就停（不会死循环）。
  const depth = new Map<string, number>();
  const parentOf = new Map<string, string | null>();
  const queue: string[] = [];
  for (const n of group) {
    if (!isChild.has(n.id)) {
      depth.set(n.id, 0);
      parentOf.set(n.id, null);
      queue.push(n.id);
    }
  }
  // 关系成环（全是子表）时从第一张起算，保证每个节点都有层
  if (queue.length === 0) {
    depth.set(group[0].id, 0);
    parentOf.set(group[0].id, null);
    queue.push(group[0].id);
  }
  for (const n of group) {
    if (!depth.has(n.id)) {
      depth.set(n.id, 0);
      parentOf.set(n.id, null);
    }
  }
  for (let round = 0; round < group.length; round++) {
    let changed = false;
    for (const n of group) {
      const d = depth.get(n.id) ?? 0;
      for (const child of children.get(n.id) ?? []) {
        // 子节点必须比父节点深一层：比现在更深才更新
        if ((depth.get(child) ?? 0) <= d) {
          if ((depth.get(child) ?? 0) < d + 1) {
            depth.set(child, d + 1);
            parentOf.set(child, n.id);
            changed = true;
          }
        }
      }
    }
    if (!changed) break;
  }
  for (const n of group) {
    if (!depth.has(n.id)) {
      depth.set(n.id, 0);
      parentOf.set(n.id, null);
    }
  }

  const layers = new Map<number, DbDesignNode[]>();
  for (const n of group) {
    const d = depth.get(n.id) ?? 0;
    const list = layers.get(d) ?? [];
    list.push(n);
    layers.set(d, list);
  }
  const depths = [...layers.keys()].sort((a, b) => a - b);

  // 兄弟相邻：层内按「父节点在本层的落位次序」排序。
  // 父在上一层，所以从上往下逐层处理，排完一层就把序号回填给它的子节点用。
  const orderOf = new Map<string, number>();
  let maxY = 0;

  depths.forEach((d, layerIndex) => {
    const list = layers.get(d)!;
    // 排序键 = 父节点的序号；无父（根层）用自身在文档中的序号兜底，保持稳定
    list.sort((a, b) => {
      const pa = parentOf.get(a.id);
      const pb = parentOf.get(b.id);
      const ka = pa ? orderOf.get(pa) ?? Number.MAX_SAFE_INTEGER : originalIndex(a);
      const kb = pb ? orderOf.get(pb) ?? Number.MAX_SAFE_INTEGER : originalIndex(b);
      return ka - kb || originalIndex(a) - originalIndex(b);
    });

    const cols = Math.max(1, Math.ceil(Math.sqrt(list.length * 1.4)));
    const rowH = Math.max(...list.map((n) => estimateHeight(n, opts)));
    const layerX = PAD + layerIndex * (CARD_W + GAP_X);
    list.forEach((n, i) => {
      const col = i % cols;
      const row = Math.floor(i / cols);
      positions[n.id] = { x: layerX + col * (CARD_W + GAP_X), y: PAD + row * (rowH + GAP_Y) };
      orderOf.set(n.id, i);
      maxY = Math.max(maxY, PAD + row * (rowH + GAP_Y) + rowH);
    });
  });

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
