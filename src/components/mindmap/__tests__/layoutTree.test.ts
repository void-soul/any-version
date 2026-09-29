import { describe, expect, it } from "vitest";
import { layoutTree } from "../layout";
import type { MindmapNode } from "../types";

function node(id: string, parentId: string | null): MindmapNode {
  return {
    id, documentId: "doc", parentId, name: id, detail: "", kind: "other",
    color: "#f59e0b", positionX: 0, positionY: 0,
  };
}

/** 根 → A/B 两个分支，每个分支 5 个叶子（AI 生成的导图典型形态）。 */
function wideTree(): MindmapNode[] {
  const out: MindmapNode[] = [node("R", null), node("A", "R"), node("B", "R")];
  for (const p of ["A", "B"]) {
    for (let i = 1; i <= 5; i++) out.push(node(`${p}${i}`, p));
  }
  return out;
}

describe("layoutTree", () => {
  it("父节点居中于自己的子节点（不漂移）", () => {
    const pos = layoutTree(wideTree());
    const y = (id: string) => pos.get(id)!.y;
    // A 居中于 a1..a5，B 居中于 b1..b5 —— 早先按「同深度第几个」全局取槽位，
    // B 会被放到第 2 行、却拖着 600~1080 的一段子节点，整张图被拉成 Z 字形。
    expect(y("A")).toBe((y("A1") + y("A5")) / 2);
    expect(y("B")).toBe((y("B1") + y("B5")) / 2);
    expect(y("R")).toBe((y("A") + y("B")) / 2);
  });

  it("各子树在堆叠轴上连续、互不交错", () => {
    const pos = layoutTree(wideTree());
    const aYs = ["A1", "A2", "A3", "A4", "A5"].map((id) => pos.get(id)!.y);
    const bYs = ["B1", "B2", "B3", "B4", "B5"].map((id) => pos.get(id)!.y);
    expect(Math.max(...aYs)).toBeLessThan(Math.min(...bYs));
    // 子树内部叶子等距排列：未测量时按兜底卡片高 90 + 兄弟间距 22 = 112
    expect(aYs[1] - aYs[0]).toBe(112);
  });

  it("按深度沿展开方向推进，层间距 256（卡宽 200 + 走廊 56）", () => {
    const pos = layoutTree(wideTree());
    expect(pos.get("R")!.x).toBe(0);
    expect(pos.get("A")!.x).toBe(256);
    expect(pos.get("A1")!.x).toBe(512);
  });

  it("按实测高度排行高：矮卡片不再被固定行高白留一大截", () => {
    const nodes = wideTree();
    // 每个节点只有 60px 高（未测量时会按兜底 90 排成 112 的行距）
    const measured = new Map(
      nodes.map(
        (n) => [n.id, { width: 200, height: 60 }] as const,
      ),
    );
    const pos = layoutTree(nodes, "lr", new Map(measured));
    const aYs = ["A1", "A2", "A3", "A4", "A5"].map((id) => pos.get(id)!.y);
    // 60 + 22
    expect(aYs[1] - aYs[0]).toBe(82);
    // 父节点仍居中于首末子节点
    expect(pos.get("A")!.y).toBe((aYs[0] + aYs[4]) / 2);
  });

  it("超高的卡片会撑开层距，不会被下一层压住", () => {
    const nodes = wideTree();
    const measured = new Map(
      nodes.map((n) => [n.id, { width: 200, height: 140 }] as const),
    );
    // 纵向布局的层距看「高」：最长的卡片 140 + 走廊 56
    const tb = layoutTree(nodes, "tb", new Map(measured));
    expect(tb.get("A")!.y).toBe(196);
    // 横向布局的层距看「宽」，不受高度影响
    const lr = layoutTree(nodes, "lr", new Map(measured));
    expect(lr.get("A")!.x).toBe(256);
    // 行高按 140 累计
    expect(lr.get("A2")!.y - lr.get("A1")!.y).toBe(162);
  });

  it("四个方向都把深度映射到正确的轴", () => {
    const nodes = wideTree();
    const lr = layoutTree(nodes, "lr");
    const rl = layoutTree(nodes, "rl");
    const tb = layoutTree(nodes, "tb");
    const bt = layoutTree(nodes, "bt");
    // lr/rl：深度走 X（rl 取负轴），堆叠走 Y
    expect(rl.get("A")!.x).toBe(-lr.get("A")!.x);
    expect(rl.get("A")!.y).toBe(lr.get("A")!.y);
    // tb/bt：深度走 Y（bt 取负轴），层距 90 + 56 = 146；堆叠改走 X，行序不变、步长 200 + 22 = 222
    expect(tb.get("R")!.y).toBe(0);
    expect(tb.get("A")!.y).toBe(146);
    expect(bt.get("A")!.y).toBe(-146);
    expect(tb.get("A")!.x).toBe((lr.get("A")!.y / 112) * 222);
  });

  it("成环的脏数据不卡死，且每个节点都拿到坐标", () => {
    // A→B→A：既没有根，也会让朴素递归无限下钻（画布打开即卡死）
    const cyclic = [node("A", "B"), node("B", "A")];
    const pos = layoutTree(cyclic);
    expect(pos.size).toBe(2);
    expect(Number.isFinite(pos.get("A")!.x)).toBe(true);
    expect(Number.isFinite(pos.get("B")!.y)).toBe(true);
  });

  it("自引用节点当作根处理", () => {
    const pos = layoutTree([node("S", "S")]);
    expect(pos.get("S")!.x).toBe(0);
    expect(pos.get("S")!.y).toBe(0);
  });

  it("空导图返回空布局", () => {
    expect(layoutTree([]).size).toBe(0);
  });
});
