import { describe, it, expect } from "vitest";

import {
  DEFAULT_HIGHLIGHT_DEPTH,
  HIGHLIGHT_DEPTHS,
  clampHighlightDepth,
  relatedEdgeIds,
  relatedTableHops,
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

describe("relatedTableHops（连线分级亮度的数据源）", () => {
  it("记录每张表相差多少跳，选中表自己是 0", () => {
    // 链式 a-b-c-d-e-f，选中 c → b/d 是 1 跳、a/e 是 2 跳、f 是 3 跳
    const hops = relatedTableHops(CHAIN, "c", 3);
    expect(hops.get("c")).toBe(0);
    expect(hops.get("b")).toBe(1);
    expect(hops.get("d")).toBe(1);
    expect(hops.get("a")).toBe(2);
    expect(hops.get("f")).toBe(3);
  });

  it("跳数与集合版一致（连线与表卡必须看同一个层级）", () => {
    // 这个一致性很关键：表卡用 Set、连线用 Map，两者不一致就会出现
    // 「表亮了但线没活」（这正是用户报的那个 bug）。
    for (const depth of [1, 2, 3, 5]) {
      expect(relatedTableHops(CHAIN, "c", depth).size).toBe(relatedTableIds(CHAIN, "c", depth).size);
    }
  });

  it("远的跳数到达上限后不再扩展", () => {
    expect(relatedTableHops(CHAIN, "c", 2).has("f")).toBe(false);
    expect(relatedTableHops(CHAIN, "c", 3).has("f")).toBe(true);
    expect(relatedTableHops(CHAIN, "c", 3).get("f")).toBe(3);
  });

  it("无选中、选中不存在、层级非法 → 都不得出错", () => {
    expect(relatedTableHops(CHAIN, null, 3).size).toBe(0);
    expect(relatedTableHops(CHAIN, "nope", 3).size).toBe(0);
    // 非法值回落到默认 1 跳（与集合版行为一致，见上方用例）
    for (const bad of [0, -3, 99, "3"]) {
      expect(relatedTableHops(CHAIN, "c", bad as number).size).toBe(
        relatedTableIds(CHAIN, "c", bad as number).size,
      );
    }
  });

  it("悬空关系不建边（不得出现画布上没有的表）", () => {
    const doc = docOf(["a", "b"], [rel("b", "a")]);
    doc.relations.push({
      id: "rx",
      name: "",
      from: { node: "a", field: "id" },
      to: { node: "ghost", field: "id" },
      kind: "1-n",
      onDelete: "",
      onUpdate: "",
      mirror: false,
    } as DbDesignRelation);
    expect(relatedTableHops(doc, "a", 3).has("ghost")).toBe(false);
    expect(relatedTableHops(doc, "a", 3).size).toBe(2);
  });
});

/**
 * 连线（边）的高亮口径 —— **层级指的是「从选中表走出去能走多远」**，
 * 不是「两端只要有一端落在集合里就算」。
 *
 * 这两者不等价，差在**同一跳层内部的横向连线**：
 * 选中 x，一跳邻居 a；a 下面挂着 c、d（都是 2 跳），而 c 与 d 之间也有一条线。
 * 集合口径会点亮 c–d —— 但从 x 出发要走 x→a→c→d 第 3 步才到，
 * 层级=2 时它根本不该亮。两端只是「远房亲戚」的线亮着、表却暗着，
 * 屏幕上就是**只亮线、不亮表**（用户实测）。
 *
 * 所以连线的高亮必须取自 BFS **实际走过**的关系集合。
 */
describe("relatedEdgeIds（连线的高亮口径）", () => {
  /**
   * 带「同层横向边」的拓扑，专门用来区分两种口径：
   *   x ──1跳── a ──2跳── c ──┐
   *   │          └──2跳── d ──┘   （c 与 d 之间还有一条线，同属第 2 层内部）
   *   └──1跳── b
   */
  const LATERAL = docOf(
    ["x", "a", "b", "c", "d"],
    [rel("a", "x"), rel("b", "x"), rel("c", "a"), rel("d", "a"), rel("d", "c")],
  );
  /** 每个 fixture 的选中项（CHAIN 选中间的 c，LATERAL 选根 x） */
  const CASES: Array<[string, DbDesignDocument]> = [
    ["CHAIN", CHAIN],
    ["LATERAL", LATERAL],
  ];
  const sel = (doc: DbDesignDocument) => (doc === CHAIN ? "c" : "x");
  const sorted = (s: Set<string>) => [...s].sort();
  /** 某条关系的两端 id */
  const ends = (doc: DbDesignDocument, edgeId: string) => {
    const r = doc.relations.find((x) => x.id === edgeId)!;
    return [r.from.node, r.to.node] as const;
  };

  it("层级=d：点亮的连线 = 从选中表出发 d 步以内实际走过的那些", () => {
    // 链 a-b-c-d-e-f，选中 c
    expect(sorted(relatedEdgeIds(CHAIN, "c", 1))).toEqual(["c-b", "d-c"]);
    expect(sorted(relatedEdgeIds(CHAIN, "c", 2))).toEqual(["b-a", "c-b", "d-c", "e-d"]);
    expect(sorted(relatedEdgeIds(CHAIN, "c", 3))).toEqual(["b-a", "c-b", "d-c", "e-d", "f-e"]);
  });

  it("同一跳层内部的横向连线不该亮（那不是走得到的路）", () => {
    expect(sorted(relatedTableIds(LATERAL, "x", 2))).toEqual(["a", "b", "c", "d", "x"]);
    expect(sorted(relatedEdgeIds(LATERAL, "x", 2))).toEqual(["a-x", "b-x", "c-a", "d-a"]);
    expect(relatedEdgeIds(LATERAL, "x", 2).has("d-c")).toBe(false);
    // 层级放到 3，才真的走得到 c-d
    expect(relatedEdgeIds(LATERAL, "x", 3).has("d-c")).toBe(true);
  });

  it("层级=d 时，亮线的两端必须都在 d 跳集合内（表不许比线暗）", () => {
    // 用户抱怨的那句「线亮了表却没亮」，在这条不变量下不可能再出现。
    for (const [name, doc] of CASES) {
      for (const d of [1, 2, 3, 4, 5]) {
        const ids = relatedTableIds(doc, sel(doc), d);
        for (const eid of relatedEdgeIds(doc, sel(doc), d)) {
          for (const end of ends(doc, eid)) {
            expect(`${name}/${d}/${eid}/${end}: ${ids.has(end)}`).toBe(
              `${name}/${d}/${eid}/${end}: true`,
            );
          }
        }
      }
    }
  });

  it("层级滑块真的往外扩：层数越大，亮线与亮表集合只增不减", () => {
    for (const [, doc] of CASES) {
      for (let d = 2; d <= 5; d++) {
        const prevEdges = relatedEdgeIds(doc, sel(doc), d - 1);
        const nowEdges = relatedEdgeIds(doc, sel(doc), d);
        for (const eid of prevEdges) expect(nowEdges.has(eid)).toBe(true);
        const prevNodes = relatedTableIds(doc, sel(doc), d - 1);
        const nowNodes = relatedTableIds(doc, sel(doc), d);
        for (const nid of prevNodes) expect(nowNodes.has(nid)).toBe(true);
      }
    }
  });

  it("与表卡口径一致：亮线的端点集合恰好等于亮表集合（补上旧测试只比 size 的漏洞）", () => {
    for (const [, doc] of CASES) {
      for (const d of [1, 2, 3, 4, 5]) {
        const ids = relatedTableIds(doc, sel(doc), d);
        const touched = new Set<string>();
        for (const eid of relatedEdgeIds(doc, sel(doc), d)) {
          for (const end of ends(doc, eid)) touched.add(end);
        }
        for (const n of touched) expect(ids.has(n)).toBe(true);
        // 反向：被点亮的表（选中表自己除外）至少要连着一条亮线，
        // 否则就出现「表亮着、却没有任何线交代它为什么亮」
        for (const n of ids) {
          if (n === sel(doc)) continue;
          expect(`${n}@${d}`).toBe(`${[...touched].includes(n) ? n : "MISS:" + n}@${d}`);
        }
      }
    }
  });

  it("无选中 / 选中不存在 / 悬空关系：空集且不抛", () => {
    const ghost = docOf(["a", "b"], [rel("b", "a"), rel("b", "zz"), rel("qq", "a")]);
    expect(relatedEdgeIds(CHAIN, null, 5).size).toBe(0);
    expect(relatedEdgeIds(CHAIN, "nope", 5).size).toBe(0);
    expect(sorted(relatedEdgeIds(ghost, "a", 5))).toEqual(["b-a"]);
  });
});
