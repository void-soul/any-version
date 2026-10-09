// 画布自动布局：分层（有关系的表按「父 → 子」分层）+ 网格回退（没关系的表按网格排）。
//
// 逆向出来的设计最常见的问题就是「所有表叠在左上角密密麻麻」—— 反推只写结构、不摆位置。
// 这里给一个**可预期**的摆法：有关联的表按层级从左到右排，同一层的表按网格往下铺；
// 完全没有关系的表自然全落在第 0 层，也就是纯网格。
//
// 为什么不引 dagre / elkjs：布局结果会**写进设计文件**（node.x/y），必须稳定可复现 ——
// 「同一份设计，布局两次要长得一样」。力导向每次结果都不同，不适合当默认摆法。
// 想看漂亮的力导向图，用户可以自己拖。

import type { DbDesignDocument, DbDesignNode } from "./types";

/** 节点卡片宽度（与 DesignNodeCard 的 w-[288px] 对齐；改那边记得改这里） */
const CARD_W = 288;
const GAP_X = 88;
const GAP_Y = 44;
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

/**
 * 算每个节点的落点（node id → 坐标）。
 *
 * 分层：`父表（关系的 to 端）` 在左，`子表（from 端）` 在右，深度用 BFS 首次到达的层数。
 * 用「首次到达」而不是「最长链」是有意的：后者遇到 A→B→A 这种环会死循环 / 需要额外收敛，
 * 而 BFS 天然只访问一次，行为可预期。
 * 多对多（n-n）不参与分层 —— 画布上它表现为一张中间表，参与分层只会把布局搅乱。
 */
export function computeLayout(
  doc: DbDesignDocument,
  opts: LayoutOptions,
): Record<string, { x: number; y: number }> {
  const nodes = doc.nodes.filter((n) => n.kind === "table");
  const out: Record<string, { x: number; y: number }> = {};
  if (nodes.length === 0) return out;

  const idSet = new Set(nodes.map((n) => n.id));
  const children = new Map<string, string[]>();
  const isChild = new Set<string>();
  for (const r of doc.relations) {
    if (r.kind === "n-n") continue;
    if (!idSet.has(r.to.node) || !idSet.has(r.from.node) || r.to.node === r.from.node) continue;
    const list = children.get(r.to.node) ?? [];
    if (!list.includes(r.from.node)) list.push(r.from.node);
    children.set(r.to.node, list);
    isChild.add(r.from.node);
  }

  // BFS 分层：根 = 不在任何关系里当子表的那张
  const depth = new Map<string, number>();
  const queue: string[] = [];
  for (const n of nodes) {
    if (!isChild.has(n.id)) {
      depth.set(n.id, 0);
      queue.push(n.id);
    }
  }
  // 全都是子表（关系成环）时，从第一张起算，保证每个节点都有层
  if (queue.length === 0) {
    depth.set(nodes[0].id, 0);
    queue.push(nodes[0].id);
  }
  while (queue.length > 0) {
    const id = queue.shift()!;
    const d = depth.get(id) ?? 0;
    for (const child of children.get(id) ?? []) {
      if (depth.has(child)) continue; // 只认第一次到达，环不会死循环
      depth.set(child, d + 1);
      queue.push(child);
    }
  }
  for (const n of nodes) {
    if (!depth.has(n.id)) depth.set(n.id, 0); // 兜底：不在任何可达链路上的孤立表
  }

  // 层内网格：列数按表数开方（略偏宽），行高取该层最高的卡片，保证不重叠
  const layers = new Map<number, DbDesignNode[]>();
  for (const n of nodes) {
    const d = depth.get(n.id) ?? 0;
    const list = layers.get(d) ?? [];
    list.push(n);
    layers.set(d, list);
  }
  const depths = [...layers.keys()].sort((a, b) => a - b);

  depths.forEach((d, layerIndex) => {
    const list = layers.get(d)!;
    const cols = Math.max(1, Math.ceil(Math.sqrt(list.length * 1.4)));
    const rowH = Math.max(...list.map((n) => estimateHeight(n, opts)));
    const layerX = PAD + layerIndex * (CARD_W + GAP_X);
    list.forEach((n, i) => {
      const col = i % cols;
      const row = Math.floor(i / cols);
      out[n.id] = { x: layerX + col * (CARD_W + GAP_X), y: PAD + row * (rowH + GAP_Y) };
    });
  });

  return out;
}
