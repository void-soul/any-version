import { describe, it, expect } from "vitest";
import { computeLayout } from "../layout";
import type { DbDesignDocument, DbDesignNode, DbDesignRelation } from "../types";

const OPTS = { fieldRows: "all" as const };

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

describe("computeLayout（系列分块 + 兄弟相邻）", () => {
  it("同一系列的表在同一块内，按父子分层左右排开", () => {
    const pos = computeLayout(docOf([n("users"), n("orders"), n("items")], [rel("orders", "users"), rel("items", "orders")]), OPTS);
    // 父在左：x 逐层递增
    expect(pos.orders.x).toBeGreaterThan(pos.users.x);
    expect(pos.items.x).toBeGreaterThan(pos.orders.x);
  });

  it("不同系列被排到互不重叠的纵向区域（连线不会横跨别家）", () => {
    const pos = computeLayout(
      docOf([n("users"), n("orders"), n("logs"), n("audit")], [rel("orders", "users"), rel("audit", "logs")]),
      OPTS,
    );
    const aBottom = pos.orders.y + 100;
    // 第二个系列整体落在第一个系列之下
    expect(pos.logs.y).toBeGreaterThanOrEqual(aBottom);
    expect(pos.audit.y).toBeGreaterThanOrEqual(aBottom);
  });

  it("同父的子表紧邻排列（层内兄弟相邻，连线不再扇形散开）", () => {
    const pos = computeLayout(
      docOf([n("users"), n("o1"), n("o2"), n("o3")], [rel("o1", "users"), rel("o2", "users"), rel("o3", "users")]),
      OPTS,
    );
    const ys = [pos.o1.y, pos.o2.y, pos.o3.y];
    const sorted = [...ys].sort((a, b) => a - b);
    // 三个子表在同一列纵向连续排列，行距一致（不会一个在左上角、一个在右下角）
    const gaps = [sorted[1] - sorted[0], sorted[2] - sorted[1]];
    expect(gaps[0]).toBeCloseTo(gaps[1], 0);
  });

  it("孤立表合并成一个网格块，而不是竖着排成一条线", () => {
    const pos = computeLayout(docOf([n("a"), n("b"), n("c"), n("d")]), OPTS);
    const ys = new Set([pos.a.y, pos.b.y, pos.c.y, pos.d.y]);
    // 四张无关联表会排成 2×2，两行两列
    expect(ys.size).toBe(2);
    expect(pos.c.y).toBe(pos.a.y);
    expect(pos.b.x).not.toBe(pos.a.x);
  });

  it("孤立表与有关联的表不会排到同一块里", () => {
    const pos = computeLayout(
      docOf([n("users"), n("orders"), n("solo1"), n("solo2")], [rel("orders", "users")]),
      OPTS,
    );
    // 有关系的块在上，孤立表块在下
    expect(pos.solo1.y).toBeGreaterThan(pos.orders.y);
    expect(pos.solo2.y).toBeGreaterThan(pos.orders.y);
  });

  it("环形引用不构成死循环，所有节点都有坐标", () => {
    const pos = computeLayout(docOf([n("a"), n("b")], [rel("a", "b"), rel("b", "a")]), OPTS);
    expect(pos.a).toBeDefined();
    expect(pos.b).toBeDefined();
  });

  it("悬空关系被跳过，不影响其余布局", () => {
    const pos = computeLayout(docOf([n("a"), n("b")], [rel("a", "ghost")]), OPTS);
    expect(pos.a).toBeDefined();
    expect(pos.b).toBeDefined();
  });

  it("布局是确定的：同一份文档算两次结果一致（可写进设计文件）", () => {
    const d = docOf([n("users"), n("orders"), n("logs")], [rel("orders", "users")]);
    expect(computeLayout(d, OPTS)).toEqual(computeLayout(d, OPTS));
  });

  it("视图节点不参与布局", () => {
    const view = { id: "v1", kind: "view", name: "v", x: 0, y: 0 } as DbDesignNode;
    const pos = computeLayout(docOf([n("users"), view], []), OPTS);
    expect(pos.users).toBeDefined();
    expect(pos.v1).toBeUndefined();
  });
});
