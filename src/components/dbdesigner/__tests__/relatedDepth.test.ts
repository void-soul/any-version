import { describe, it, expect } from "vitest";

import {
  DEFAULT_HIGHLIGHT_DEPTH,
  HIGHLIGHT_DEPTHS,
  clampHighlightDepth,
  relatedTableIds,
} from "../series";
import type { DbDesignDocument, DbDesignNode, DbDesignRelation } from "../types";

function t(id: string): DbDesignNode {
  return { id, kind: "table", name: id, color: "", x: 0, y: 0, table: { fields: [], indexes: [] } } as DbDesignNode;
}
function rel(from: string, to: string): DbDesignRelation {
  return {
    id: from + "-" + to,
    name: "",
    from: { node: from, field: "id" },
    to: { node: to, field: "id" },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  } as DbDesignRelation;
}
function docOf(nodes: string[], relations: DbDesignRelation[]): DbDesignDocument {
  return { nodes: nodes.map(t), relations } as DbDesignDocument;
}

/**
 * 链式：a—b—c—d—e—f（**无向**，每条关系都双向连边）。
 *
 * 拉到 6 个是为了让各层级可区分：选中 c 时一跳到两侧，
 * 就是 b 与 d；若链只有 5 节点，第 2 跳就已经到两端、各层级结果一样。
 */
const CHAIN = docOf(
  ["a", "b", "c", "d", "e", "f"],
  [rel("b", "a"), rel("c", "b"), rel("d", "c"), rel("e", "d"), rel("f", "e")],
);

describe("relatedTableIds（高亮层级）", () => {
  it("默认 1 跳 = 选中表 + 直接上级/下级（既有行为不能变）", () => {
    const got = relatedTableIds(CHAIN, "c");
    expect([...got].sort()).toEqual(["b", "c", "d"]);
    expect(DEFAULT_HIGHLIGHT_DEPTH).toBe(1);
  });

  it("层级=2 / 3 逐跳向外扩", () => {
    // 无向链：c 的第 1 跳是 {b, d}，第 2 跳是 {a, e}，第 3 跳才得到 f
    expect([...relatedTableIds(CHAIN, "c", 2)].sort()).toEqual(["a", "b", "c", "d", "e"]);
    expect([...relatedTableIds(CHAIN, "c", 3)].sort()).toEqual(["a", "b", "c", "d", "e", "f"]);
  });

  it("层级=5 也不越界：只含真实存在的表", () => {
    const got = relatedTableIds(CHAIN, "c", 5);
    expect([...got].sort()).toEqual(["a", "b", "c", "d", "e", "f"]);
    for (const id of got) expect(["a", "b", "c", "d", "e", "f"]).toContain(id);
  });

  it("成环不死循环（a↔b↔c 环 + 环外挂一张）", () => {
    const doc = docOf(["a", "b", "c", "x"], [rel("b", "a"), rel("c", "b"), rel("a", "c"), rel("x", "a")]);
    const got = relatedTableIds(doc, "a", 5);
    expect([...got].sort()).toEqual(["a", "b", "c", "x"]);
  });

  it("悬空关系不炸：指向不存在的节点", () => {
    const doc = docOf(["a", "b"], [rel("b", "a"), rel("zz", "a"), rel("b", "yy")]);
    expect(() => relatedTableIds(doc, "a", 5)).not.toThrow();
    expect([...relatedTableIds(doc, "a", 5)].sort()).toEqual(["a", "b"]);
  });

  it("没有选中时是空集（画布无高亮 = 什么都不点亮）", () => {
    expect(relatedTableIds(CHAIN, null, 5).size).toBe(0);
  });

  it("非法层级收敛到 1：0 / 负数 / 99 / 字符串都不生效", () => {
    const base = [...relatedTableIds(CHAIN, "c", 1)].sort();
    for (const bad of [0, -3, 99]) {
      // 0 与负数按「至少 1 跳」处理，而不是返回空集 —— 传 0 不该让高亮整个消失
      expect([...relatedTableIds(CHAIN, "c", bad)].sort()).toEqual(base);
    }
  });

  it("clampHighlightDepth：只认 1-5，其余回落默认", () => {
    expect(HIGHLIGHT_DEPTHS).toEqual([1, 2, 3, 4, 5]);
    for (const ok of [1, 2, 3, 4, 5]) expect(clampHighlightDepth(ok)).toBe(ok);
    // localStorage 里存的是字符串，必须同样认
    expect(clampHighlightDepth("3")).toBe(3);
    expect(clampHighlightDepth("9")).toBe(1);
    expect(clampHighlightDepth(null)).toBe(1);
    expect(clampHighlightDepth(undefined)).toBe(1);
    expect(clampHighlightDepth("abc")).toBe(1);
  });
});
