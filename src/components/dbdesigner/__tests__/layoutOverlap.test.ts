import { describe, expect, it } from "vitest";

import { computeLayout } from "../layout";
import type { DbDesignDocument, DbDesignNode, DbDesignRelation } from "../types";

const OPTS = { alwaysExpand: false, fieldRows: "all" as const };

function n(id: string, rows = 3): DbDesignNode {
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
        pk: i === 0,
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

// 与 layout.ts 的估算保持一致：HEADER_H + 行数 * ROW_H + 12
function cardH(rows: number) {
  return 58 + rows * 18 + 12;
}

/**
 * 布局结果会**写进设计文件**（node.x / node.y），所以「任意两张卡片都不压叠」
 * 是硬底线 —— 一旦破线，用户打开设计就是一坨互相盖住的卡片，而且已经存进文件了。
 * 这里用一份「链 + 扇出 + 环 + 孤立表」齐全的 schema 兜住它。
 */
describe("computeLayout（卡片互不压叠）", () => {
  it("复杂 schema 里任意两张卡片都不重叠", () => {
    const nodes = [
      n("users"),
      n("orders", 5),
      n("items", 4),
      n("sku", 6),
      n("logs"),
      n("audit"),
      n("solo1"),
      n("solo2"),
      n("solo3"),
      n("a"),
      n("b"),
    ];
    const doc = {
      nodes,
      relations: [
        rel("orders", "users"),
        rel("items", "orders"),
        rel("items", "sku"),
        rel("audit", "logs"),
        rel("a", "b"),
        rel("b", "a"),
      ],
    } as DbDesignDocument;

    const pos = computeLayout(doc, OPTS);
    expect(Object.keys(pos).length).toBe(nodes.length);

    const boxes = nodes.map((nd) => {
      const p = pos[nd.id];
      const rows = nd.table?.fields.length ?? 1;
      return { id: nd.id, x1: p.x, x2: p.x + 288, y1: p.y, y2: p.y + cardH(rows) };
    });
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const A = boxes[i];
        const B = boxes[j];
        const overlap = A.x1 < B.x2 && B.x1 < A.x2 && A.y1 < B.y2 && B.y1 < A.y2;
        expect(overlap, `${A.id} 与 ${B.id} 压叠: ${JSON.stringify([A, B])}`).toBe(false);
      }
    }
  });

  it("同一份设计算两次完全一致（坐标要能写进文件）", () => {
    const doc = {
      nodes: [n("users"), n("orders", 5), n("items", 4), n("solo")],
      relations: [rel("orders", "users"), rel("items", "orders")],
    } as DbDesignDocument;
    expect(computeLayout(doc, OPTS)).toEqual(computeLayout(doc, OPTS));
  });
});
