import { describe, it, expect } from "vitest";
import { computeLayout } from "../layout";
import { MIN_CARD_H } from "../layoutCore";
import { buildArrangeSubDoc, relatedTableIds } from "../series";
import type { DbDesignDocument } from "../types";

const CARD_W = 288;

function table(id: string, name: string, cols: string[]) {
  return {
    id,
    kind: "table",
    name,
    color: "",
    x: 0,
    y: 0,
    table: {
      fields: cols.map((c, i) => ({
        name: c,
        type: { base: "int" },
        nullable: false,
        default: null,
        comment: "",
        pk: i === 0,
        autoIncrement: i === 0,
        unique: false,
      })),
      indexes: [],
    },
  };
}

function rel(from: string, to: string, field: string) {
  return {
    id: `r-${from}-${to}`,
    name: "",
    from: { node: from, field },
    to: { node: to, field: `${to}_id` },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  };
}

/**
 * 卡片是否重叠。
 *
 * 高度用算法自己的下界 `MIN_CARD_H`，不要拍一个「宽松上界」：早先这里写 200px，
 * 而这些卡片实际只有 106px 高、行间距 170px，200 > 170 于是把**正常间距判成重叠**
 * —— coreLeftRight 的星型用例因此误报（真实文件里两种模式都是零重叠）。
 * 用算法的常量，既不会误报，也跟实现一起演进。
 */
function anyOverlap(pos: Record<string, { x: number; y: number }>, ids: string[]): string | null {
  for (let i = 0; i < ids.length; i++) {
    for (let j = i + 1; j < ids.length; j++) {
      const p = pos[ids[i]];
      const q = pos[ids[j]];
      if (!p || !q) continue;
      const hitX = p.x < q.x + CARD_W && q.x < p.x + CARD_W;
      const hitY = p.y < q.y + MIN_CARD_H && q.y < p.y + MIN_CARD_H;
      if (hitX && hitY) return `${ids[i]}×${ids[j]}`;
    }
  }
  return null;
}

const CHAIN: DbDesignDocument = {
  id: "d",
  name: "d",
  dialect: "mysql",
  nodes: [
    table("t1", "user", ["user_id", "user_name"]),
    table("t2", "article", ["article_id", "user_id", "article_title"]),
    table("t3", "comment", ["comment_id", "article_id", "content"]),
  ],
  relations: [rel("t2", "t1", "user_id"), rel("t3", "t2", "article_id")],
} as any;

/**
 * 方向语义：2026-10 起布局从「父左子右分层树」换成「枢纽放射」，
 * 所以这里**不再断言 x 方向**（放射图里父在子的左边/右边取决于谁是枢纽，
 * 强行断言只会把实现锁死在旧算法上）。
 *
 * 真正该守住的是三条与方向无关的不变量：
 *   1. 卡片互不重叠（用户抱怨「表压在一起」）
 *   2. 可复现（坐标会写进设计文件，两次必须一样）
 *   3. 「整理选中」与整份布局口径一致，且只动选中的那几张表
 *
 * 这三条**必须两种模式都成立**，所以下面用 `MODES` 遍历，而不是依赖 `DEFAULT_LAYOUT_MODE`：
 * 默认值一改就集体失效，既看不出是算法坏了还是默认值变了，���等于这些不变量根本没被测到。
 */
/** 两种布局算法：不变量对两者都必须成立 */
const MODES = ["radial", "coreLeftRight"] as const;

describe("layout（两种算法都要满足的不变量）", () => {
  it("链式结构：卡片互不重叠", () => {
    for (const mode of MODES) {
      const pos = computeLayout(CHAIN, { fieldRows: "all" }, mode);
      expect(anyOverlap(pos, ["t1", "t2", "t3"]), `${mode} 下重叠`).toBeNull();
    }
  });

  it("放射：连接最多的表落在圆心（链式里中段的 article 度数 2，是事实上的枢纽）", () => {
    // 显式传 radial：这条测的是「圆心」这个放射特有的形态，
    // 核心左右算法里枢纽在中间列而不是圆心，用隐式默认会随默认值变化而失效
    const pos = computeLayout(CHAIN, { fieldRows: "all" }, "radial");
    // user 只被引用 1 次、comment 被引用 0 次，article 被引用 1 次且引用了 user → 度数 2 最高
    const avg = (id: string) =>
      (["t1", "t2", "t3"] as const)
        .filter((k) => k !== id)
        .reduce((s, k) => s + Math.hypot(pos[id].x - pos[k].x, pos[id].y - pos[k].y), 0) / 2;
    const center = (["t1", "t2", "t3"] as const).reduce((a, b) => (avg(a) <= avg(b) ? a : b));
    expect(center).toBe("t2");
  });

  it("可复现：同一份设计算两次坐标完全一致（坐标会写进设计文件）", () => {
    for (const mode of MODES) {
      expect(computeLayout(CHAIN, { fieldRows: "all" }, mode)).toEqual(
        computeLayout(CHAIN, { fieldRows: "all" }, mode),
      );
    }
  });

  it("星型：六张表都围着枢纽，互不重叠", () => {
    const nodes = [table("t0", "order_item", ["item_id"])];
    const relations = [];
    for (let i = 1; i <= 6; i++) {
      nodes.push(table(`t${i}`, `leaf_${i}`, [`leaf_${i}_id`]) as any);
      relations.push(rel(`t${i}`, "t0", `order_id`) as any);
    }
    const doc = { id: "d", name: "d", dialect: "mysql", nodes, relations } as any;
    for (const mode of MODES) {
      const pos = computeLayout(doc, { fieldRows: "all" }, mode);
      const ids = Object.keys(pos);
      expect(ids).toHaveLength(7);
      expect(anyOverlap(pos, ids), `${mode} 下重叠`).toBeNull();
    }
  });

  it("整理选中：只排选中的那几张，且与整份布局用同一个算法（不会自成一套）", () => {
    const selectedId = "t2";
    const ids = relatedTableIds(CHAIN, selectedId);
    expect([...ids].sort()).toEqual(["t1", "t2", "t3"]);

    const sub = buildArrangeSubDoc(CHAIN, ids);
    for (const mode of MODES) {
      const subPos = computeLayout(sub, { fieldRows: "all" }, mode);
      // 只含选中的三张
      expect(Object.keys(subPos).sort()).toEqual(["t1", "t2", "t3"]);
      expect(anyOverlap(subPos, ["t1", "t2", "t3"]), `${mode} 下重叠`).toBeNull();
      const fullPos = computeLayout(CHAIN, { fieldRows: "all" }, mode);
      const avg = (pos: Record<string, { x: number; y: number }>, id: string) =>
        (["t1", "t2", "t3"] as const)
          .filter((k) => k !== id)
          .reduce((s, k) => s + Math.hypot(pos[id].x - pos[k].x, pos[id].y - pos[k].y), 0) / 2;
      expect(avg(subPos, "t1")).toBeLessThanOrEqual(avg(fullPos, "t1"));
    }
  });

  it("选中集合里的表落在互不相干的系列时也不会压在一起", () => {
    const doc: DbDesignDocument = {
      id: "d",
      name: "d",
      dialect: "mysql",
      nodes: [
        table("t1", "user", ["user_id"]),
        table("t2", "order", ["order_id", "user_id"]),
        table("t3", "product", ["product_id"]),
      ],
      relations: [rel("t2", "t1", "user_id")],
    } as any;
    for (const mode of MODES) {
      const pos = computeLayout(doc, { fieldRows: "all" }, mode);
      expect(anyOverlap(pos, ["t1", "t2", "t3"]), `${mode} 下重叠`).toBeNull();
    }
  });
});
