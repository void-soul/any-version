import { describe, it, expect } from "vitest";

import { DEFAULT_LAYOUT_MODE, LAYOUT_MODES, computeLayout } from "../layout";
import { CARD_W, CORRIDOR, MIN_CARD_H } from "../layoutCore";
import type { DbDesignDocument, DbDesignNode, DbDesignRelation } from "../types";

const OPTS = { fieldRows: "all" as const };

function t(id: string, name: string, rows = 2): DbDesignNode {
  return {
    id,
    kind: "table",
    name,
    color: "",
    x: 0,
    y: 0,
    table: {
      fields: Array.from({ length: rows }, (_, i) => ({
        name: "c" + i,
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
  } as DbDesignNode;
}

/** from 引用 to：from 是子表、to 是父表 */
function rel(from: string, to: string): DbDesignRelation {
  return {
    id: from + "-" + to,
    name: "",
    from: { node: from, field: to + "_id" },
    to: { node: to, field: "id" },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  } as DbDesignRelation;
}

function docOf(nodes: DbDesignNode[], relations: DbDesignRelation[] = []): DbDesignDocument {
  return { nodes, relations } as DbDesignDocument;
}

function overlap(pos: Record<string, { x: number; y: number }>, ids: string[]): string | null {
  for (let i = 0; i < ids.length; i++) {
    for (let j = i + 1; j < ids.length; j++) {
      const p = pos[ids[i]];
      const q = pos[ids[j]];
      const xHit = p.x < q.x + CARD_W && q.x < p.x + CARD_W;
      const yHit = p.y < q.y + MIN_CARD_H && q.y < p.y + MIN_CARD_H;
      if (xHit && yHit) return ids[i] + " x " + ids[j];
    }
  }
  return null;
}

/**
 * 核心表左右分列：**被引用的主表在左、引用别人的明细表在右**，两侧上下均分。
 *
 * 方向以用户给的期望图为准（order_item 在中，order / product / product_sku 竖排在左，
 * after_sale 在右）。关系里 `from` 引用 `to`，所以「被引用的那一端」是 `to`。
 *
 * 与放射算法共用「系列」口径，这里只测块内几何。
 */
describe("layout coreLeftRight", () => {
  it("被引用的主表在核心左侧、引用别人的明细表在右侧", () => {
    // core 被 t2 引用（t2 是明细表 → 右）；core 引用 t3（t3 是主表 → 左）
    const doc = docOf(
      [t("t1", "core"), t("t2", "detail"), t("t3", "master")],
      [rel("t2", "t1"), rel("t1", "t3")],
    );
    const pos = computeLayout(doc, OPTS, "coreLeftRight");
    expect(pos.t3.x).toBeLessThan(pos.t1.x); // 主表在左
    expect(pos.t2.x).toBeGreaterThan(pos.t1.x); // 明细表在右
    expect(pos.t1.x - pos.t3.x).toBe(CARD_W + CORRIDOR);
    expect(pos.t2.x - pos.t1.x).toBe(CARD_W + CORRIDOR);
  });

  /**
   * 真实数据回归（用户期望图）：order_item 引用 order / product / product_sku，
   * after_sale 引用 order_item。期望 order_item 在中间，被引用的三张在左一列竖排，
   * after_sale 在右。
   */
  it("真实关系：order_item 在中，order/product/product_sku 在左列，after_sale 在右", () => {
    const doc = docOf(
      [t("t0", "order_item"), t("t1", "order"), t("t2", "product"), t("t3", "product_sku"), t("t4", "after_sale")],
      [
        rel("t0", "t1"), // order_item -> order
        rel("t0", "t2"), // order_item -> product
        rel("t0", "t3"), // order_item -> product_sku
        rel("t4", "t0"), // after_sale -> order_item
      ],
    );
    const pos = computeLayout(doc, OPTS, "coreLeftRight");
    // 核心 = 连接最多的 order_item
    // 三张被引用的表同一列（都在核心左边一列）
    expect(pos.t1.x).toBe(pos.t2.x);
    expect(pos.t2.x).toBe(pos.t3.x);
    expect(pos.t1.x).toBeLessThan(pos.t0.x);
    // after_sale 引用 order_item → 右侧
    expect(pos.t4.x).toBeGreaterThan(pos.t0.x);
    // 三张主表竖排不重叠
    expect(overlap(pos, ["t1", "t2", "t3"])).toBeNull();
  });

  it("同一侧多张主表上下平行均分（间距一致、以核心表为中心对称）", () => {
    const doc = docOf(
      [t("t0", "core"), t("t1", "master_a"), t("t2", "master_b"), t("t3", "master_c")],
      [rel("t0", "t1"), rel("t0", "t2"), rel("t0", "t3")],
    );
    const pos = computeLayout(doc, OPTS, "coreLeftRight");
    expect(pos.t1.x).toBe(pos.t2.x);
    expect(pos.t2.x).toBe(pos.t3.x);
    expect(pos.t2.y - pos.t1.y).toBe(pos.t3.y - pos.t2.y);
    const mid = (k: string) => pos[k].y + MIN_CARD_H / 2;
    expect(Math.abs(mid("t1") + mid("t3") - 2 * mid("t0"))).toBeLessThan(1);
  });

  it("两跳及更远沿同一方向继续往外加列（跳数越深离核心越远）", () => {
    // core 引用 t2（t2 是主表 → 左一列），t2 再引用 t3（t3 两跳 → 左二列）
    const doc = docOf(
      [t("t1", "core"), t("t2", "master"), t("t3", "master_grand")],
      [rel("t1", "t2"), rel("t2", "t3")],
    );
    const pos = computeLayout(doc, OPTS, "coreLeftRight");
    expect(pos.t3.x).toBeLessThan(pos.t2.x);
    expect(pos.t2.x).toBeLessThan(pos.t1.x);
  });

  it("两侧都深时列不会互相压住", () => {
    const doc = docOf(
      [
        t("t0", "core"),
        t("t1", "master_a"),
        t("t2", "master_b"),
        t("t3", "master_grand"),
        t("t4", "detail_a"),
        t("t5", "detail_b"),
      ],
      [rel("t0", "t1"), rel("t0", "t2"), rel("t1", "t3"), rel("t4", "t0"), rel("t5", "t0")],
    );
    const pos = computeLayout(doc, OPTS, "coreLeftRight");
    const ids = ["t0", "t1", "t2", "t3", "t4", "t5"];
    expect(overlap(pos, ids)).toBeNull();
    // 列序（从左到右）：两跳主表 | 一跳主表 | 核心 | 明细表
    expect(pos.t3.x).toBeLessThan(pos.t1.x); // 两跳的主表更靠左
    expect(pos.t1.x).toBeLessThan(pos.t0.x); // 一跳主表在核心左边
    expect(pos.t4.x).toBeGreaterThan(pos.t0.x); // 明细表在核心右边
  });

  it("可复现：同一份设计算两次坐标完全一致（坐标会写进设计文件）", () => {
    const doc = docOf(
      [t("t0", "core"), t("t1", "master_a"), t("t2", "master_b"), t("t3", "detail_a")],
      [rel("t0", "t1"), rel("t0", "t2"), rel("t3", "t0")],
    );
    expect(computeLayout(doc, OPTS, "coreLeftRight")).toEqual(
      computeLayout(doc, OPTS, "coreLeftRight"),
    );
  });

  it("成环时不崩、每张表都有落点", () => {
    const doc = docOf(
      [t("t1", "a"), t("t2", "b"), t("t3", "c")],
      [rel("t1", "t2"), rel("t2", "t3"), rel("t3", "t1")],
    );
    const pos = computeLayout(doc, OPTS, "coreLeftRight");
    expect(Object.keys(pos).sort()).toEqual(["t1", "t2", "t3"]);
  });

  it("孤立表仍铺成网格（左右分列对无关系表没有意义）", () => {
    const pos = computeLayout(docOf([t("t1", "a"), t("t2", "b"), t("t3", "c")]), OPTS, "coreLeftRight");
    expect(pos.t1.y).toBe(pos.t2.y);
    expect(pos.t2.x).toBeGreaterThan(pos.t1.x);
  });

  it("两种模式产出不同坐标，但都无重叠（切换算法不会坏图）", () => {
    const doc = docOf(
      [
        t("t0", "core"),
        t("t1", "master_a"),
        t("t2", "master_b"),
        t("t3", "detail_a"),
        t("t4", "detail_b"),
      ],
      [rel("t0", "t1"), rel("t0", "t2"), rel("t3", "t0"), rel("t4", "t0")],
    );
    const radial = computeLayout(doc, OPTS, "radial");
    const core = computeLayout(doc, OPTS, "coreLeftRight");
    expect(core).not.toEqual(radial);
    const ids = Object.keys(radial);
    expect(overlap(core, ids)).toBeNull();
    expect(overlap(radial, ids)).toBeNull();
  });

  /**
   * 缺省模式 = `DEFAULT_LAYOUT_MODE`（当前是核心左右）。
   *
   * 这条钉住的是「不传 mode 时与常量一致」而不是某个字面量 ——
   * 断言字面量的话，以后改默认就会以「测试失败」的形式提醒，但看不出该改哪里；
   * 断言与常量一致，则调用点漏改默认时能立刻抓到（默认曾经散在三处，正是这么漏的）。
   */
  it("不传 mode 时等于 DEFAULT_LAYOUT_MODE", () => {
    const doc = docOf(
      [t("t0", "core"), t("t1", "master_a"), t("t2", "detail_a")],
      [rel("t0", "t1"), rel("t2", "t0")],
    );
    expect(computeLayout(doc, OPTS)).toEqual(computeLayout(doc, OPTS, DEFAULT_LAYOUT_MODE));
    // 顺带钉住默认值本身：用户明确要求默认用「核心表左右」
    expect(DEFAULT_LAYOUT_MODE).toBe("coreLeftRight");
    // 且它与实际默认行为一致（不只是常量自己）
    expect(computeLayout(doc, OPTS)).toEqual(computeLayout(doc, OPTS, "coreLeftRight"));
    expect(computeLayout(doc, OPTS)).not.toEqual(computeLayout(doc, OPTS, "radial"));
  });

  /** 下拉首项 = 默认项：否则用户打开面板看到的高亮项与实际排版不是一回事。 */
  it("LAYOUT_MODES 的首项就是默认模式", () => {
    expect(LAYOUT_MODES[0].value).toBe(DEFAULT_LAYOUT_MODE);
    // 两种模式都必须在列表里
    expect(LAYOUT_MODES.map((m) => m.value).sort()).toEqual(["coreLeftRight", "radial"]);
  });
});
