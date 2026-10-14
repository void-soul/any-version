// 画布自动布局：**按系列分块 + 块内枢纽放射**。
//
// 逆向出来的设计最常见的问题是「所有表叠在左上角密密麻麻」—— 反推只写结构、不摆位置。
//
// 为什么是「枢纽放射」而不是分层树（2026-10 改版）：
// 用户给的目标图是「星型结构以枢纽为中心、其余环绕、线呈辐射状不交叉」。
// 原来的分层树（最长路径分层 + 主树后序）在**一个大连通分量的星型结构**上必然退化：
// 分层要求父子各占一层/一列，于是 order_item 与它的 6 张同伴被摊到 7 个不同列里，
// 连线变成横穿整幅图的长斜线 —— 而这正是「线不重叠、先不穿表」治不好的根因。
// 放射布局让所有边都从中心往外辐射，天然没有长斜线。
//
// 为什么不用 dagre / elkjs / 力导向：
//   · 布局结果会**写进设计文件**（node.x/y），必须稳定可复现 ——「同一份设计布局两次要长得一样」。
//     力导向每次结果都不同，不能当默认摆法；dagre 是纯函数但只提供分层树语义。
//   · 放射布局的每个量都由「度数 + 文档序」算出，完全确定，且**不需要新依赖**。
//
// 为什么按系列分块（连线不再打结的第二道保险）：不同系列物理隔离，
// 而「系列」的口径与配色完全同一份（见 series.ts）—— 颜色成对、位置也一定成对。

import type { DbDesignDocument, DbDesignNode } from "./types";
import { fkFieldsOf, relationFieldsOf, type FieldRowMode } from "./types";
import { groupByPrefix } from "./series";

/** 节点卡片宽度（与 DesignNodeCard 的 w-[288px] 对齐；改那边记得改这里） */
const CARD_W = 288;
/**
 * 卡片之间的走廊宽度（切向净空）。
 *
 * 它是「线不穿表」的主要保证：环上相邻两张卡的弧距必须 ≥ 卡宽 + 走廊，
 * 于是连线只走在这条走廊里，不可能横穿卡片。
 */
const CORRIDOR = 120;
/**
 * 圈与圈之间的**最小径向间距**。
 *
 * 数值不是拍出来的：两张卡片（宽 w、高 h）不重叠的充要条件是中心距
 * ≥ √(w² + h²)（否则存在 dx < w 且 dy < h 的组合）。相邻两圈里两张卡的中心距
 * 最小值出现在「角度相同」时 = 径向间距，所以只要径向间距 ≥ √(w²+h²) ≈ 307，
 * **无论两张卡错开多少角度都不会压住**。
 *
 * 早先按「卡片高度 + 80」取（≈186），实测就在这里翻车：外圈卡与内圈卡斜着错开、
 * 中心距只有 186 而水平错开仅 161 —— 两张卡压在一起（截图级 bug，靠测试抓出来）。
 */
const RING_MIN_GAP = CARD_W + 60;
/** 第一圈半径：8 张卡刚好围一圈，视觉上最接近目标图那种「环抱中心」 */
const R0 = 430;
/** 每圈至少放几张（否则圈会小到卡片挤在一起） */
const MIN_PER_RING = 3;
/** 环向排序的迭代轮数（barycenter 平滑；3 轮足够，再多收益递减且拖慢布局） */
const ORDER_SWEEPS = 3;
/** 系列块之间的纵向间距 */
const BLOCK_GAP = 160;
const PAD = 40;
/** 卡片高度估算：字段一行 ≈ 18px，表头 ≈ 46px（与卡片实际样式同量级即可，宁可高估） */
const ROW_H = 18;
const HEADER_H = 58;

export interface LayoutOptions {
  /** 卡片里显示哪些字段行：keys = 主键 + 外键 + 关联字段，all = 全部字段 */
  fieldRows: FieldRowMode;
}

/**
 * 这张表在当前显示模式下会渲染几行字段。
 *
 * 口径必须与 `DesignNodeCard` 里 `shown` 的筛选**逐字一致**：布局坐标要写进设计
 * 文件，估矮了卡片就会互相压住。所以这里直接复用卡片自己用的两个 helper，
 * 而不是再写一遍「哪些算外键」—— 两边各写一份迟早会漂。
 */
function visibleFieldCount(doc: DbDesignDocument, node: DbDesignNode, mode: FieldRowMode): number {
  const fields = node.table?.fields ?? [];
  if (mode === "all") return fields.length;
  const fk = fkFieldsOf(doc, node.id);
  const rel = relationFieldsOf(doc, node.id);
  return fields.filter((f) => f.pk || fk.includes(f.name) || rel.includes(f.name)).length;
}

function estimateHeight(doc: DbDesignDocument, node: DbDesignNode, mode: FieldRowMode): number {
  const rows =
    node.kind === "view"
      ? mode === "all"
        ? 3
        : 1
      : Math.max(1, visibleFieldCount(doc, node, mode));
  return HEADER_H + rows * ROW_H + (node.comment ? 18 : 0) + 12;
}

/** 一个系列的布局结果：局部坐标 + 该块占位尺寸（用于把各块摆开、并居中） */
interface GroupLayout {
  positions: Record<string, { x: number; y: number }>;
  width: number;
  height: number;
}

/** 节点在文档中的原始次序：所有排序的稳定兜底，保证同一份设计每次算出来一样 */
function originalIndex(n: DbDesignNode): number {
  const m = n.id.match(/(\d+)/);
  return m ? Number(m[1]) : 0;
}

/** 块内邻接表（不含 n-n：它在画布上是一张自动生成的中间表，参与只会把布局搅乱） */
function adjacency(doc: DbDesignDocument, group: DbDesignNode[]): Map<string, Set<string>> {
  const ids = new Set(group.map((n) => n.id));
  const adj = new Map<string, Set<string>>();
  for (const n of group) adj.set(n.id, new Set());
  for (const r of doc.relations) {
    if (r.kind === "n-n") continue;
    if (!ids.has(r.from.node) || !ids.has(r.to.node)) continue;
    if (r.from.node === r.to.node) continue;
    adj.get(r.from.node)!.add(r.to.node);
    adj.get(r.to.node)!.add(r.from.node);
  }
  return adj;
}

/**
 * 第 ring 圈的半径：按「这一圈能放几张卡」反解，而不是拍一个固定值。
 *
 * 一圈 m 张卡均分时，相邻卡的弧距是 `2πr/m`，要 ≥ 卡宽 + 走廊，
 * 所以 `r = m × (卡宽 + 走廊) / 2π`；圈与圈之间再加一条径向净空。
 */
function ringRadius(ring: number): number {
  return R0 + ring * RING_MIN_GAP;
}

/** 给定半径，一圈最多塞几张卡（弧距够放下一张卡片为止） */
function ringCapacity(r: number): number {
  return Math.max(MIN_PER_RING, Math.floor((2 * Math.PI * r) / (CARD_W + CORRIDOR)));
}

/**
 * 单个系列：**枢纽 + 放射圈**。
 *
 * 1. 枢纽 = 系列内连接数最多的表（并列时取文档序靠前）—— 星型结构里它就是事实上的中心。
 * 2. 其余成员按度数降序分配到各圈：连接多的先放内圈（线更短、更少交叉）。
 * 3. 圈内在角度上均分，但**排序**用 barycenter（邻居角度的圆周均值）迭代 3 轮，
 *    让「互相关联的表挨在一起」—— 同圈的连线于是只连相邻卡（短弦，不穿圆心），
 *    这才是「线和线不重叠」的真正来源。
 * 4. 角度起点固定正上方；排序并列时一律按文档序 → 完全可复现。
 */
function layoutGroup(group: DbDesignNode[], doc: DbDesignDocument, opts: LayoutOptions): GroupLayout {
  const mode = opts.fieldRows;
  const heights: Record<string, number> = {};
  for (const n of group) heights[n.id] = estimateHeight(doc, n, mode);
  const adj = adjacency(doc, group);
  const positions: Record<string, { x: number; y: number }> = {};

  // 中心放枢纽：连接数最多者（并列取文档序靠前），单表系列它就是自己
  const sorted = [...group].sort((a, b) => {
    const d = adj.get(b.id)!.size - adj.get(a.id)!.size;
    return d !== 0 ? d : originalIndex(a) - originalIndex(b);
  });
  const hub = sorted[0];
  const rest = sorted.slice(1);
  positions[hub.id] = { x: -CARD_W / 2, y: -heights[hub.id] / 2 };

  if (rest.length > 0) {
    const rings: DbDesignNode[][] = [];
    for (let ring = 0, i = 0; i < rest.length; ring++) {
      const cap = ringCapacity(ringRadius(ring));
      rings.push(rest.slice(i, i + cap));
      i += cap;
    }
    const angleOf = new Map<string, number>();
    const START = -Math.PI / 2; // 正上方 12 点方向，顺时针铺开
    const placeRing = (members: DbDesignNode[], count: number) => {
      const step = 2 * Math.PI / Math.max(1, count);
      members.forEach((n, i) => angleOf.set(n.id, START + i * step));
    };
    for (const ring of rings) placeRing(ring, ring.length);

    // barycenter 平滑：把每张表排到「邻居平均角度」附近，连线因此不交叉。
    // 圆周均值用单位向量求和再 atan2 —— 直接对角度取算术平均会在 ±π 处翻车。
    for (let sweep = 0; sweep < ORDER_SWEEPS; sweep++) {
      for (const ring of rings) {
        const scored = ring.map((n) => {
          let sx = 0;
          let sy = 0;
          let cnt = 0;
          for (const m of adj.get(n.id)!) {
            const a = angleOf.get(m);
            if (a === undefined) continue;
            sx += Math.cos(a);
            sy += Math.sin(a);
            cnt++;
          }
          // 没有已放置邻居（只剩枢纽这一个邻居）→ 用文档序兜底，保持轮次间稳定
          return { n, key: cnt > 0 ? Math.atan2(sy, sx) : Number.MAX_SAFE_INTEGER, doc: originalIndex(n) };
        });
        scored.sort((a, b) => (a.key !== b.key ? a.key - b.key : a.doc - b.doc));
        placeRing(
          scored.map((s) => s.n),
          scored.length,
        );
      }
    }

    // 落坐标：卡片**中心**落在圆上，所以要减掉半个卡宽 / 卡高
    rings.forEach((ring, ringIdx) => {
      const r = ringRadius(ringIdx);
      for (const n of ring) {
        const a = angleOf.get(n.id)!;
        positions[n.id] = {
          x: r * Math.cos(a) - CARD_W / 2,
          y: r * Math.sin(a) - heights[n.id] / 2,
        };
      }
    });
  }

  // 块的外接尺寸：给 computeLayout 做水平居中与纵向堆叠
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const n of group) {
    const p = positions[n.id];
    minX = Math.min(minX, p.x);
    minY = Math.min(minY, p.y);
    maxX = Math.max(maxX, p.x + CARD_W);
    maxY = Math.max(maxY, p.y + heights[n.id]);
  }
  for (const n of group) {
    positions[n.id] = { x: positions[n.id].x - minX, y: positions[n.id].y - minY };
  }
  return { positions, width: maxX - minX, height: maxY - minY };
}

/**
 * 算每个节点的落点（node id → 坐标）。
 *
 * 两步：**先按业务系列分块**（与配色同一份口径），块内各自枢纽放射，
 * 各块垂直堆叠并水平居中。这样「不相干的表不会互相挡道」「同一系列的连线是辐射短线」，
 * 而不是让所有表挤进同一套分层里、连线横穿整幅图。
 */
export function computeLayout(
  doc: DbDesignDocument,
  opts: LayoutOptions,
): Record<string, { x: number; y: number }> {
  const nodes = doc.nodes.filter((n) => n.kind === "table");
  if (nodes.length === 0) return {};

  const groups = groupByPrefix(doc, nodes);

  // 孤立表（与任何表都没关系）各自成块会被顺次堆成一条线 —— 它们无缘线，
  // 应该聚成一块铺网格（逆向 dump 里「一堆无关联表」很常见）。
  const linked = new Set<string>();
  for (const r of doc.relations) {
    if (r.kind === "n-n") continue;
    linked.add(r.from.node);
    linked.add(r.to.node);
  }
  const isIsolated = (g: DbDesignNode[]) => g.length === 1 && !linked.has(g[0].id);
  const loose = groups.filter(isIsolated).flatMap((g) => g);
  const blocks = groups.filter((g) => !isIsolated(g));

  const laid: GroupLayout[] = [];
  for (const g of blocks) laid.push(layoutGroup(g, doc, opts));

  if (loose.length > 0) {
    const cols = Math.max(1, Math.ceil(Math.sqrt(loose.length * 1.4)));
    const positions: Record<string, { x: number; y: number }> = {};
    const rows = loose.map((n) => estimateHeight(doc, n, opts.fieldRows));
    const rowH = Math.max(...rows);
    loose.forEach((n, i) => {
      positions[n.id] = {
        x: PAD + (i % cols) * (CARD_W + CORRIDOR),
        y: PAD + Math.floor(i / cols) * (rowH + CORRIDOR),
      };
    });
    const rowCount = Math.ceil(loose.length / cols);
    laid.push({
      positions,
      width: PAD * 2 + cols * CARD_W + (cols - 1) * CORRIDOR,
      height: PAD * 2 + rowCount * rowH + (rowCount - 1) * CORRIDOR,
    });
  }

  // 各块水平居中：放射块接近圆形、宽度不一，不居中会参差不齐
  const maxW = Math.max(...laid.map((b) => b.width));
  const out: Record<string, { x: number; y: number }> = {};
  let cursorY = PAD;
  for (const b of laid) {
    const dx = PAD + (maxW - b.width) / 2;
    for (const [id, p] of Object.entries(b.positions)) {
      out[id] = { x: p.x + dx, y: p.y + cursorY };
    }
    cursorY += b.height + BLOCK_GAP;
  }
  return out;
}
