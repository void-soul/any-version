// 布局算法之二：**核心表左右分列**（用户 2026-10 追加的需求）。
//
// 形态：找到核心表 → 它的子表在**左侧**、父表在**右侧**，两侧各自**上下平行均分**；
// 更远的表沿同一方向继续往外加列（子方向往左、父方向往右），跳数越深离核心越远。
//
// 与 `layout.ts` 的放射算法分工：
//   · 放射：边都是辐射短线，星型结构最漂亮，但「谁引用谁」要靠看线的箭头分辨；
//   · 左右分列：**引用方向直接编码在坐标里**（主表一定在左、明细表一定在右），
//     代价是同层表格之间的连线会斜着走一点。
// 两者共用同一份「系列」口径（见 series.ts），所以切换算法不会改变配色。
//
// 仍然是**确定算法**（坐标会写进 `.dbdesign.json`，同一份设计布局两次必须一样）：
// BFS 队列按文档序、并列取文档序、垂直均分不引入任何随机量。

import type { DbDesignDocument, DbDesignNode } from "./types";
import { fkFieldsOf, relationFieldsOf, type FieldRowMode } from "./types";

/** 卡片宽度（与 layout.ts / DesignNodeCard 对齐） */
export const CARD_W = 288;
/** 卡片之间的走廊（横向净空）：线只走走廊，所以不穿卡 */
export const CORRIDOR = 120;
/** 同列内上下两张卡的净空 */
export const ROW_GAP = 64;
/** 表头 + 两行字段的典型高度下限（估算一律不小于它） */
export const MIN_CARD_H = 106;
const ROW_H = 18;
const HEADER_H = 58;

export interface CoreLayoutOptions {
  fieldRows: FieldRowMode;
}

/** 块内布局结果：局部坐标（从 0,0 起）+ 占位尺寸 */
export interface BlockLayout {
  positions: Record<string, { x: number; y: number }>;
  width: number;
  height: number;
}

/** 节点在文档中的原始次序：所有排序的稳定兜底 */
export function originalIndex(n: DbDesignNode): number {
  const m = n.id.match(/(\d+)/);
  return m ? Number(m[1]) : 0;
}

function visibleFieldCount(doc: DbDesignDocument, node: DbDesignNode, mode: FieldRowMode): number {
  const fields = node.table?.fields ?? [];
  if (mode === "all") return fields.length;
  const fk = fkFieldsOf(doc, node.id);
  const rel = relationFieldsOf(doc, node.id);
  return fields.filter((f) => f.pk || fk.includes(f.name) || rel.includes(f.name)).length;
}

export function estimateHeight(doc: DbDesignDocument, node: DbDesignNode, mode: FieldRowMode): number {
  const rows =
    node.kind === "view"
      ? mode === "all"
        ? 3
        : 1
      : Math.max(1, visibleFieldCount(doc, node, mode));
  return Math.max(MIN_CARD_H, HEADER_H + rows * ROW_H + (node.comment ? 18 : 0) + 12);
}

/** 块内邻接（不含 n-n：它在画布上是自动生成的中间表，参与只会把布局搅乱） */
export function adjacency(doc: DbDesignDocument, group: DbDesignNode[]): {
  adj: Map<string, Set<string>>;
  /** 子方向：`from → to`（from 引用 to，所以 from 是子表） */
  children: Map<string, string[]>;
  parents: Map<string, string[]>;
} {
  const ids = new Set(group.map((n) => n.id));
  const adj = new Map<string, Set<string>>();
  const children = new Map<string, string[]>();
  const parents = new Map<string, string[]>();
  for (const n of group) {
    adj.set(n.id, new Set());
    children.set(n.id, []);
    parents.set(n.id, []);
  }
  for (const r of doc.relations) {
    if (r.kind === "n-n") continue;
    if (!ids.has(r.from.node) || !ids.has(r.to.node)) continue;
    if (r.from.node === r.to.node) continue;
    adj.get(r.from.node)!.add(r.to.node);
    adj.get(r.to.node)!.add(r.from.node);
    children.get(r.to.node)!.push(r.from.node);
    parents.get(r.from.node)!.push(r.to.node);
  }
  return { adj, children, parents };
}

/**
 * 一个系列按「核心表左右分列」布局。
 *
 * 步骤：
 * 1. 核心 = 连接数最多的表（并列取文档序）；
 * 2. BFS 分列：从核心出发，第一次到达某表时记下列号。沿子方向走 → 左侧，
 *    沿父方向走 → 右侧，每深一跳离核心远一列；
 * 3. 每列按估计高度**上下均分**，整列以核心表的 y 为中心对称展开；
 * 4. BFS 到不了的（成环的另一支）兜底进最外侧一列。
 */
export function layoutGroupCore(
  group: DbDesignNode[],
  doc: DbDesignDocument,
  opts: CoreLayoutOptions,
): BlockLayout {
  const mode = opts.fieldRows;
  const heights: Record<string, number> = {};
  for (const n of group) heights[n.id] = estimateHeight(doc, n, mode);
  const { adj, children, parents } = adjacency(doc, group);

  // ── 1) 选核心：连接数最多者 ──
  const hub = [...group].sort((a, b) => {
    const d = adj.get(b.id)!.size - adj.get(a.id)!.size;
    return d !== 0 ? d : originalIndex(a) - originalIndex(b);
  })[0];

  // ── 2) BFS 分列 ──
  const columnOf = new Map<string, number>();
  columnOf.set(hub.id, 0);
  type QItem = { id: string; col: number };
  const queue: QItem[] = [{ id: hub.id, col: 0 }];
  const seen = new Set<string>([hub.id]);
  while (queue.length > 0) {
    const cur = queue.shift()!;
    const push = (list: string[], next: number) => {
      for (const id of list) {
        if (seen.has(id)) continue;
        seen.add(id);
        columnOf.set(id, next);
        queue.push({ id, col: next });
      }
    };
    // 方向：**被引用的（主表）在左，引用别人的（明细表）在右**。
    //
    // 别和「谁引用谁」搞反：关系里 `from` 引用 `to`，所以 `to` 才是被引用的那一端。
    // 用户给的期望图（order_item 在中，order / product / product_sku 竖排在左，after_sale 在右）
    // 就是这个方向 —— 第一版做反了，整幅图左右镜像。
    push(parents.get(cur.id) ?? [], cur.col - 1); // 主表在左：越远越左
    push(children.get(cur.id) ?? [], cur.col + 1); // 明细表在右：越远越右
  }

  // ── 3) 分列收集（BFS 没到不了的兜底到最右一列）──
  const cols = new Map<number, DbDesignNode[]>();
  for (const n of group) {
    const col = columnOf.get(n.id) ?? Number.MAX_SAFE_INTEGER;
    const list = cols.get(col) ?? [];
    list.push(n);
    cols.set(col, list);
  }
  const colKeys = [...cols.keys()].sort((a, b) => a - b);
  const reachable = colKeys.filter((k) => k !== Number.MAX_SAFE_INTEGER);
  const minCol = reachable.length > 0 ? Math.min(...reachable) : 0;
  const maxCol = reachable.length > 0 ? Math.max(...reachable) : 0;

  // 核心列只有一张卡，它的高度就是整幅图的纵向中心线
  const centerY = heights[hub.id] / 2;

  const positions: Record<string, { x: number; y: number }> = {};
  for (const key of colKeys) {
    const list = [...cols.get(key)!].sort((a, b) => originalIndex(a) - originalIndex(b));
    const col = key === Number.MAX_SAFE_INTEGER ? maxCol + 1 : key;
    const x = (col - minCol) * (CARD_W + CORRIDOR);
    const tallest = Math.max(...list.map((n) => heights[n.id]));
    const total = list.length * tallest + (list.length - 1) * ROW_GAP;
    // 整列以核心表为中心对称展开（上下「平等」）
    let y = centerY - total / 2;
    for (const n of list) {
      positions[n.id] = { x, y: y + (tallest - heights[n.id]) / 2 };
      y += tallest + ROW_GAP;
    }
  }

  // ── 4) 外接尺寸（供编排层居中与堆叠）──
  //
  // 归一化到「左上角为原点」时**只平移一次**：minY 先一次性算完再统一减。
  // 早先写成「边遍历边改 y」，第二张卡读到的 minY 已经是平移后的值，
  // 于是同列的 y 被压回同一水平线 —— 两张表直接叠在一起（测试抓到 t4 x t5 重叠）。
  let minY = Infinity;
  let maxX = 0;
  let maxY = -Infinity;
  for (const n of group) {
    const p = positions[n.id];
    if (p.y < minY) minY = p.y;
    if (p.x + CARD_W > maxX) maxX = p.x + CARD_W;
    if (p.y + heights[n.id] > maxY) maxY = p.y + heights[n.id];
  }
  if (minY === Infinity) minY = 0;
  for (const n of group) {
    positions[n.id] = { x: positions[n.id].x, y: positions[n.id].y - minY };
  }
  return { positions, width: maxX, height: maxY - minY };
}

/** 无关联的表铺成网格（放射与左右分列都共用这一步）。 */
export function layoutLooseGrid(
  loose: DbDesignNode[],
  doc: DbDesignDocument,
  opts: CoreLayoutOptions,
  pad: number,
): BlockLayout {
  const cols = Math.max(1, Math.ceil(Math.sqrt(loose.length * 1.4)));
  const positions: Record<string, { x: number; y: number }> = {};
  const rowH = Math.max(...loose.map((n) => estimateHeight(doc, n, opts.fieldRows)));
  loose.forEach((n, i) => {
    positions[n.id] = {
      x: pad + (i % cols) * (CARD_W + CORRIDOR),
      y: pad + Math.floor(i / cols) * (rowH + ROW_GAP),
    };
  });
  const rowCount = Math.ceil(loose.length / cols);
  return {
    positions,
    width: pad * 2 + cols * CARD_W + (cols - 1) * CORRIDOR,
    height: pad * 2 + rowCount * rowH + (rowCount - 1) * ROW_GAP,
  };
}
