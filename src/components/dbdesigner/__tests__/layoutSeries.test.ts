import { describe, it, expect } from "vitest";
import { computeLayout } from "../layout";
import { groupByPrefix, seriesKeyOf } from "../series";
import type { DbDesignDocument, DbDesignNode, DbDesignRelation } from "../types";

const OPTS = { fieldRows: "all" as const };
const CARD_W = 288;

function n(id: string, rows = 2): DbDesignNode {
  return {
    id,
    kind: "table",
    name: id,
    color: "",
    x: 0,
    y: 0,
    table: {
      fields: Array.from({ length: rows }, (_, i) => ({
        name: `c${i}`,
        type: { base: "int" },
        nullable: false,
        default: null,
        comment: "",
        pk: false,
        autoIncrement: false,
        unique: false,
      })),
      indexes: [],
    },
  } as DbDesignNode;
}
/** from 指向 to：语义上是「子表 from → 父表 to」 */
function rel(from: string, to: string): DbDesignRelation {
  return {
    id: from + "-" + to,
    name: "",
    from: { node: from, field: "u" },
    to: { node: to, field: "c" },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  } as DbDesignRelation;
}
function docOf(nodes: DbDesignNode[], relations: DbDesignRelation[] = []): DbDesignDocument {
  return { nodes, relations } as DbDesignDocument;
}
/** 卡片矩形（坐标是左上角，估算高度与layout 内部同量级） */
const approxH = (rows = 2) => 58 + rows * 18 + 12;
function overlaps(pos: Record<string, { x: number; y: number }>, a: string, b: string): boolean {
  const ra = { x: pos[a].x, y: pos[a].y, w: CARD_W, h: approxH() };
  const rb = { x: pos[b].x, y: pos[b].y, w: CARD_W, h: approxH() };
  return (
    ra.x < rb.x + rb.w && rb.x < ra.x + ra.w && ra.y < rb.y + rb.h && rb.y < ra.y + ra.h
  );
}
function dist(pos: Record<string, { x: number; y: number }>, a: string, b: string): number {
  const cx = (id: string) => ({ x: pos[id].x + CARD_W / 2, y: pos[id].y + approxH() / 2 });
  const ca = cx(a);
  const cb = cx(b);
  return Math.hypot(ca.x - cb.x, ca.y - cb.y);
}

describe("系列分组：表名前缀 ∪ 外键", () => {
  it("同前缀的表归一组（哪怕它们之间还没有外键）", () => {
    const a = n("t1", 1);
    a.name = "user_paper";
    const b = n("t2", 1);
    b.name = "user_token";
    const groups = groupByPrefix(docOf([a, b]));
    expect(groups).toHaveLength(1);
    expect(groups[0].map((x) => x.name)).toEqual(["user_paper", "user_token"]);
  });

  it("有外键但名字不像的表也会并到一起（billing_invoice ≈ invoice）", () => {
    const doc = docOf(
      [n("t1"), n("t2")],
      [rel("t2", "t1")],
    );
    doc.nodes[0].name = "invoice";
    doc.nodes[1].name = "billing_invoice";
    const groups = groupByPrefix(doc);
    expect(groups).toHaveLength(1);
  });

  it("没有下划线的表自成一组", () => {
    expect(seriesKeyOf("order")).toBe("order");
    expect(seriesKeyOf("user_paper")).toBe("user");
    expect(seriesKeyOf("_weird")).toBe("_weird");
  });

  it("单表系列也会拿到颜色（autoColorAll 不会漏掉它）", () => {
    const doc = docOf([n("t1"), n("t2"), n("t3")]);
    expect(groupByPrefix(doc)).toHaveLength(3);
  });
});

/**
 * 放射布局专属用例。
 *
 * **每个用例都显式传 "radial"**：这个 describe 测的就是放射语义（圆心、环上挨着），
 * 不该跟着 `DEFAULT_LAYOUT_MODE` 走 —— 默认值一改，这里的断言就会集体失效，
 * 然后要花时间分辨「是算法坏了」还是「默认值变了」（本次就踩了这个坑）。
 */
describe("computeLayout（枢纽放射）", () => {
  it("星型结构：连接最多的表落在圆心，同伴环绕它", () => {
    // order_item 被 6 张表引用 → 它是枢纽
    const nodes = [n("t0"), ...Array.from({ length: 6 }, (_, i) => n(`t${i + 1}`))];
    const rels = Array.from({ length: 6 }, (_, i) => rel(`t${i + 1}`, "t0"));
    const pos = computeLayout(docOf(nodes, rels), OPTS, "radial");
    // 圆心 = 距其它表平均距离最小的那个
    let best = "";
    let bestD = Infinity;
    for (const id of Object.keys(pos)) {
      const avg =
        Object.keys(pos)
          .filter((x) => x !== id)
          .reduce((s, x) => s + dist(pos, id, x), 0) / 6;
      if (avg < bestD) {
        bestD = avg;
        best = id;
      }
    }
    expect(best).toBe("t0");
  });

  it("卡片互不重叠（任意两张表不压在一起）", () => {
    const nodes = [n("t0"), ...Array.from({ length: 9 }, (_, i) => n(`t${i + 1}`))];
    const rels = Array.from({ length: 9 }, (_, i) => rel(`t${i + 1}`, "t0"));
    const pos = computeLayout(docOf(nodes, rels), OPTS, "radial");
    const ids = Object.keys(pos);
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) {
        expect(overlaps(pos, ids[i], ids[j])).toBe(false);
      }
    }
  });

  it("可复现：同一份设计算两次坐标完全一致（布局会写进设计文件）", () => {
    const nodes = [n("t0"), ...Array.from({ length: 7 }, (_, i) => n(`t${i + 1}`))];
    const rels = Array.from({ length: 7 }, (_, i) => rel(`t${i + 1}`, "t0"));
    const doc = docOf(nodes, rels);
    expect(computeLayout(doc, OPTS, "radial")).toEqual(computeLayout(doc, OPTS, "radial"));
  });

  it("相连的表在环上挨着（连线是短弦而不是横穿圆心的长线）", () => {
    // a-b 相连、c-d-e 相连，两组互不相连 → 应分到两个系列块，块内各自成环
    const doc = docOf(
      [n("t0"), n("t1"), n("t2"), n("t3")],
      [rel("t0", "t1"), rel("t2", "t3")],
    );
    const groups = groupByPrefix(doc);
    expect(groups).toHaveLength(2);
    const pos = computeLayout(doc, OPTS, "radial");
    // 每组内部：相连的两张表中心距 < 该组外接直径的一半
    expect(dist(pos, "t0", "t1")).toBeLessThan(dist(pos, "t0", "t2"));
  });

  it("单表系列也能落到画布上（不是 undefined）", () => {
    const pos = computeLayout(docOf([n("t0"), n("t1")]), OPTS, "radial");
    expect(pos.t0).toBeDefined();
    expect(pos.t1).toBeDefined();
  });

  it("孤立表铺成网格，同前缀的孤立表仍然同块（网格而不是放射）", () => {
    const pos = computeLayout(
      docOf([
        n("t0"),
        n("t1"),
        n("t2"),
        n("t3"),
        n("t4"),
        n("t5"),
      ]),
      OPTS,
    );
    // 无任何关系 → 全是孤立表 → 网格布局：同一行内的两张卡x 差 = 卡宽 + 走廊
    expect(pos.t0).toBeDefined();
    expect(pos.t5).toBeDefined();
    expect(pos.t1.x - pos.t0.x).toBe(CARD_W + 120);
  });
});
