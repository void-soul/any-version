import { describe, expect, it } from "vitest";
import {
  DEFAULT_GRAPH_DEPTH,
  GRAPH_DEPTHS,
  clampGraphDepth,
  graphScope,
  hopFade,
  scopeEdgeIds,
  scopeNodeIds,
  type ScopeEdge,
} from "../graphScope";

/**
 * a 为根：b(1) 往下挂 c、e(2)，c 往下 d(3)，d 往下 f(4)；
 * 另外 a 直接挂 x、y(1)，x—y 之间还有一条横向边 —— 用它复现「只亮线、不亮节点」。
 */
function fixture(): { nodes: string[]; edges: ScopeEdge[] } {
  return {
    nodes: ["a", "b", "c", "d", "e", "f", "x", "y"],
    edges: [
      { id: "ab", source: "a", target: "b" },
      { id: "bc", source: "b", target: "c" },
      { id: "cd", source: "c", target: "d" },
      { id: "be", source: "b", target: "e" },
      { id: "df", source: "d", target: "f" },
      { id: "ax", source: "a", target: "x" },
      { id: "ay", source: "a", target: "y" },
      { id: "xy", source: "x", target: "y" },
    ],
  };
}

describe("clampGraphDepth", () => {
  it("合法值原样返回，非法值一律回落 1 跳", () => {
    for (const d of GRAPH_DEPTHS) expect(clampGraphDepth(d)).toBe(d);
    // 传 0 不该让高亮整个消失；传 99 的「全都要」不该由一个非法输入默默决定
    for (const bad of [0, -1, 99, "3x", null, undefined, NaN]) expect(clampGraphDepth(bad)).toBe(DEFAULT_GRAPH_DEPTH);
  });
});

describe("graphScope", () => {
  const { nodes, edges } = fixture();

  it("未选中 / 选中不存在的节点 → 空作用域（调用方无需特判）", () => {
    expect(scopeNodeIds(graphScope(nodes, edges, null)).size).toBe(0);
    expect(scopeNodeIds(graphScope(nodes, edges, "zz")).size).toBe(0);
  });

  it("默认 1 跳 = 根 + 直接邻居", () => {
    const scope = graphScope(nodes, edges, "b");
    expect([...scopeNodeIds(scope)].sort()).toEqual(["a", "b", "c", "e"]);
  });

  it("层级可调：d 是 2 跳、f 是 3 跳，层级卡在哪就停在哪", () => {
    const two = graphScope(nodes, edges, "b", 2);
    expect(two.hops.get("d")).toBe(2);
    expect(two.hops.has("f")).toBe(false);
    expect(graphScope(nodes, edges, "b", 3).hops.get("f")).toBe(3);
    // 图是连通的，5 跳 = 全图
    expect(graphScope(nodes, edges, "b", 5).hops.size).toBe(nodes.length);
  });

  it("边的范围是「BFS 实际走过的那批」，不是「两端有一端在集合内」", () => {
    // 回归「只亮线、不亮节点」：x 与 y 都是 a 的 1 跳邻居，集合口径会点亮 x—y，
    // 可从 a 出发要走 a→x→y 两步才到 y —— 层级=1 时它不该亮。
    // 两端都在亮着的集合里、线却亮着，屏幕上就是「线亮着、节点却暗着」。
    const one = graphScope(nodes, edges, "a", 1);
    expect(one.hops.has("x")).toBe(true);
    expect(one.hops.has("y")).toBe(true);
    expect(scopeEdgeIds(one).has("xy")).toBe(false);
    expect([...scopeEdgeIds(one)].sort()).toEqual(["ab", "ax", "ay"]);
    // 层级放到 2，BFS 才真的走过 x—y
    expect(scopeEdgeIds(graphScope(nodes, edges, "a", 2)).has("xy")).toBe(true);
  });

  it("已走过的边不因对端已访问而漏记", () => {
    const scope = graphScope(nodes, edges, "a", 2);
    // a→b→c→d：cd 是 3 跳，不该在 2 跳内；但 ab 必须在
    expect(scope.edgeIds.has("ab")).toBe(true);
    expect(scope.edgeIds.has("cd")).toBe(false);
  });

  it("悬空边不参与：既不点亮自己，也不把那个 id 加进节点集", () => {
    const withDangling = [...edges, { id: "zz", source: "a", target: "gone" }];
    const scope = graphScope(nodes, withDangling, "a", 1);
    expect(scope.edgeIds.has("zz")).toBe(false);
    expect(scope.hops.has("gone")).toBe(false);
  });

  it("成环不卡死（BFS 按跳数分层，天然收敛）", () => {
    const cyclic = {
      nodes: ["x", "y", "z"],
      edges: [
        { id: "xy", source: "x", target: "y" },
        { id: "yz", source: "y", target: "z" },
        { id: "zx", source: "z", target: "x" },
      ],
    };
    expect(scopeNodeIds(graphScope(cyclic.nodes, cyclic.edges, "x", 5)).size).toBe(3);
  });

  it("非法层级当作 1 跳，而不是「最深」", () => {
    expect([...scopeNodeIds(graphScope(nodes, edges, "a", 99))].sort()).toEqual(["a", "b", "x", "y"]);
  });
});

describe("hopFade", () => {
  it("1 跳最亮，逐跳退亮度，5 跳封底 0.25", () => {
    expect(hopFade(1)).toBe(1);
    expect(hopFade(2)).toBe(0.75);
    expect(hopFade(3)).toBe(0.5);
    expect(hopFade(4)).toBe(0.25);
    // 再远也留 0.25：归零等于把五跳画成四跳
    expect(hopFade(5)).toBe(0.25);
    expect(hopFade(9)).toBe(0.25);
  });

  it("未高亮 / 根自己 → 1（不打折）", () => {
    expect(hopFade(undefined)).toBe(1);
    expect(hopFade(0)).toBe(1);
  });
});
