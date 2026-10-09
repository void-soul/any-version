import { describe, expect, it } from "vitest";

import { computeLayout } from "../layout";
import type { DbDesignDocument, DbDesignNode, DbField } from "../types";

function field(name: string, pk = false): DbField {
  return {
    name,
    type: { base: "bigint" },
    nullable: false,
    pk,
    autoIncrement: false,
    unique: false,
  };
}

function table(id: string, fields: DbField[]): DbDesignNode {
  return {
    id,
    kind: "table",
    name: id,
    table: { fields, indexes: [] },
    view: null,
  };
}

function docOf(nodes: DbDesignNode[], relations: DbDesignDocument["relations"] = []): DbDesignDocument {
  return {
    id: "d",
    name: "d",
    dialect: "mysql",
    nodes,
    relations,
  };
}

const opts = { alwaysExpand: false, fieldRows: "all" as const };

describe("computeLayout", () => {
  it("没有任何关系时全部落在第 0 层并排成网格（逆向 dump 的常见情况）", () => {
    const doc = docOf([table("a", [field("id", true)]), table("b", [field("id", true)]), table("c", [field("id", true)])]);
    const pos = computeLayout(doc, opts);
    expect(Object.keys(pos).sort()).toEqual(["a", "b", "c"]);
    // 3 个表：列数 = ceil(sqrt(3*1.4)) = 3 → 同一排，x 依次拉开
    expect(pos.a.y).toBe(pos.b.y);
    expect(pos.b.y).toBe(pos.c.y);
    expect(pos.b.x).toBeGreaterThan(pos.a.x);
    expect(pos.c.x).toBeGreaterThan(pos.b.x);
  });

  it("表多到一行放不下时自动换行，不叠在一起", () => {
    const many = Array.from({ length: 9 }, (_, i) => table(`t${i}`, [field("id", true)]));
    const pos = computeLayout(docOf(many), opts);
    // 9 个表：列数 = ceil(sqrt(12.6)) = 4 → 4 + 4 + 1 三行
    const rows = new Set(Object.values(pos).map((p) => p.y));
    expect(rows.size).toBe(3);
    // 同一行内 x 不重合
    expect(pos.t1.x).toBeGreaterThan(pos.t0.x);
    expect(pos.t4.x).toBe(pos.t0.x);
    expect(pos.t4.y).toBeGreaterThan(pos.t0.y);
  });

  it("有父子关系时子表被放到父表右边一层", () => {
    const doc = docOf(
      [table("users", [field("id", true)]), table("orders", [field("id", true), field("user_id")])],
      [{ id: "r1", from: { node: "orders", field: "user_id" }, to: { node: "users", field: "id" }, kind: "1-n" }],
    );
    const pos = computeLayout(doc, opts);
    expect(pos.users.x).toBeLessThan(pos.orders.x);
    expect(pos.users.y).toBe(pos.orders.y);
  });

  it("多级链路一层一层往右排", () => {
    const doc = docOf(
      [table("a", [field("id", true)]), table("b", [field("id", true)]), table("c", [field("id", true)])],
      [
        { id: "r1", from: { node: "b", field: "a_id" }, to: { node: "a", field: "id" }, kind: "1-n" },
        { id: "r2", from: { node: "c", field: "b_id" }, to: { node: "b", field: "id" }, kind: "1-n" },
      ],
    );
    const pos = computeLayout(doc, opts);
    expect(pos.a.x).toBeLessThan(pos.b.x);
    expect(pos.b.x).toBeLessThan(pos.c.x);
  });

  it("关系成环时不死循环，且每个节点都有落点", () => {
    const doc = docOf(
      [table("a", [field("id", true)]), table("b", [field("id", true)])],
      [
        { id: "r1", from: { node: "b", field: "a_id" }, to: { node: "a", field: "id" }, kind: "1-n" },
        { id: "r2", from: { node: "a", field: "b_id" }, to: { node: "b", field: "id" }, kind: "1-n" },
      ],
    );
    const pos = computeLayout(doc, opts);
    expect(Object.keys(pos).sort()).toEqual(["a", "b"]);
  });

  it("多对多不参与分层（否则布局会被中间表搅乱）", () => {
    const doc = docOf(
      [table("a", [field("id", true)]), table("b", [field("id", true)])],
      [{ id: "r1", from: { node: "b", field: "a_id" }, to: { node: "a", field: "id" }, kind: "n-n" }],
    );
    const pos = computeLayout(doc, opts);
    // 同一层 → y 相同
    expect(pos.a.y).toBe(pos.b.y);
  });

  it("字段多的表估得更高，同层里不会互相压住（行高取该层最高）", () => {
    const fat = table("fat", Array.from({ length: 12 }, (_, i) => field(`c${i}`)));
    const thin = table("thin", [field("id", true)]);
    const doc = docOf([fat, thin]);
    const pos = computeLayout(doc, { alwaysExpand: true, fieldRows: "all" });
    // 两个都在第 0 层、同一行 → y 相同；行高由更高的 fat 决定
    expect(pos.fat.y).toBe(pos.thin.y);
  });

  it("空文档返回空结果，不报错", () => {
    expect(computeLayout(docOf([]), opts)).toEqual({});
  });
});
