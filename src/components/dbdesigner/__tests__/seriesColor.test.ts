import { describe, it, expect } from "vitest";
import type { DbDesignDocument, DbDesignNode, DbDesignRelation } from "./types";

/** 与 DbDesignerPanel 中 assignSeriesColors 同构的副本：这里只测算法本身 */
function assignSeriesColors(doc: DbDesignDocument, palette: string[]): Map<string, string> {
  const parent = new Map<string, string>();
  const find = (x: string): string => {
    let root = x;
    while (parent.get(root) !== root) root = parent.get(root)!;
    let cur = x;
    while (parent.get(cur) !== root) {
      const next = parent.get(cur)!;
      parent.set(cur, root);
      cur = next;
    }
    return root;
  };
  for (const n of doc.nodes) parent.set(n.id, n.id);
  for (const r of doc.relations) {
    if (!parent.has(r.from.node) || !parent.has(r.to.node)) continue;
    const a = find(r.from.node);
    const b = find(r.to.node);
    if (a !== b) parent.set(a, b);
  }
  const colorOfRoot = new Map<string, string>();
  const byNode = new Map<string, string>();
  let idx = 0;
  for (const n of doc.nodes) {
    const root = find(n.id);
    if (!colorOfRoot.has(root)) colorOfRoot.set(root, palette[idx++ % palette.length]);
    byNode.set(n.id, colorOfRoot.get(root)!);
  }
  return byNode;
}

const PALETTE = ["#c1", "#c2", "#c3"];

function node(id: string): DbDesignNode {
  return { id, kind: "table", name: id, color: "", x: 0, y: 0 } as DbDesignNode;
}
function rel(from: string, to: string): DbDesignRelation {
  return {
    id: from + "-" + to,
    name: "",
    from: { node: from, field: "x" },
    to: { node: to, field: "y" },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  } as DbDesignRelation;
}
function docOf(nodes: string[], relations: DbDesignRelation[]): DbDesignDocument {
  return { nodes: nodes.map(node), relations } as DbDesignDocument;
}

describe("assignSeriesColors（按外键连通分量分组）", () => {
  it("互不引用的表各自分组、颜色不同", () => {
    const m = assignSeriesColors(docOf(["a", "b"], []), PALETTE);
    expect(m.get("a")).toBe("#c1");
    expect(m.get("b")).toBe("#c2");
  });

  it("有引用关系的表归为同一系列、同色", () => {
    const m = assignSeriesColors(docOf(["users", "orders", "tags"], [rel("orders", "users")]), PALETTE);
    expect(m.get("orders")).toBe(m.get("users"));
    expect(m.get("tags")).not.toBe(m.get("users"));
  });

  it("传递闭包也算同系列：a-b-c 三张表同色", () => {
    const m = assignSeriesColors(docOf(["a", "b", "c", "d"], [rel("a", "b"), rel("b", "c")]), PALETTE);
    expect(m.get("a")).toBe(m.get("b"));
    expect(m.get("b")).toBe(m.get("c"));
    expect(m.get("d")).not.toBe(m.get("a"));
  });

  it("环形引用不会死循环，且仍归一组", () => {
    const m = assignSeriesColors(docOf(["a", "b"], [rel("a", "b"), rel("b", "a")]), PALETTE);
    expect(m.get("a")).toBe(m.get("b"));
  });

  it("指向不存在节点的悬空关系被跳过，不影响其余分组", () => {
    const m = assignSeriesColors(docOf(["a", "b"], [rel("a", "ghost")]), PALETTE);
    expect(m.get("a")).toBe("#c1");
    expect(m.get("b")).toBe("#c2");
  });

  it("色板用尽后循环复用，规模过大时仍能上色", () => {
    const m = assignSeriesColors(docOf(["a", "b", "c", "d"], []), PALETTE);
    expect(m.size).toBe(4);
    expect(new Set(m.values()).size).toBe(PALETTE.length);
  });

  it("重复计算结果稳定（不会每次都换色）", () => {
    const d = docOf(["users", "orders", "tags"], [rel("orders", "users")]);
    const a = assignSeriesColors(d, PALETTE);
    const b = assignSeriesColors(d, PALETTE);
    expect([...a.entries()]).toEqual([...b.entries()]);
  });
});
