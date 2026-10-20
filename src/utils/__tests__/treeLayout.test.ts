import { describe, expect, it } from "vitest";
import { isTreeLayoutDir, treeLayout, type LayoutNode } from "../treeLayout";

/** 根 → A/B 两个分支，每个分支 3 个叶子 */
function wideTree(): LayoutNode[] {
  const out: LayoutNode[] = [
    { id: "R", parentId: null },
    { id: "A", parentId: "R" },
    { id: "B", parentId: "R" },
  ];
  for (const p of ["A", "B"]) for (let i = 1; i <= 3; i += 1) out.push({ id: `${p}${i}`, parentId: p });
  return out;
}

/** JSON 画布的卡片更宽（250），层距与行距按它算 */
const JSON_SIZES = { cardW: 250, cardH: 90, levelGap: 56, stackGap: 22 };

describe("treeLayout", () => {
  it("父节点居中于自己的子节点，各子树在堆叠轴上互不交错", () => {
    const pos = treeLayout(wideTree(), "lr", undefined, JSON_SIZES);
    const y = (id: string) => pos.get(id)!.y;
    expect(y("A")).toBe((y("A1") + y("A3")) / 2);
    expect(y("R")).toBe((y("A") + y("B")) / 2);
    const aYs = ["A1", "A2", "A3"].map(y);
    const bYs = ["B1", "B2", "B3"].map(y);
    expect(Math.max(...aYs)).toBeLessThan(Math.min(...bYs));
  });

  it("尺寸参数真的生效：卡宽 250 + 走廊 56 = 层距 306（JSON 画布口径）", () => {
    const pos = treeLayout(wideTree(), "lr", undefined, JSON_SIZES);
    expect(pos.get("R")!.x).toBe(0);
    expect(pos.get("A")!.x).toBe(306);
    expect(pos.get("A1")!.x).toBe(612);
    // 行距按兜底卡高 90 + 兄弟间距 22
    expect(pos.get("A2")!.y - pos.get("A1")!.y).toBe(112);
  });

  it("实测尺寸优先于兜底值；横向布局的层距看宽、纵向看高", () => {
    const nodes = wideTree();
    const measured = new Map(nodes.map((n) => [n.id, { width: 250, height: 260 }] as const));
    // lr：沿 X 展开，层距看**宽度** → 250 + 56
    expect(treeLayout(nodes, "lr", new Map(measured), JSON_SIZES).get("A")!.x).toBe(306);
    // tb：沿 Y 展开，层距看**高度** → 260 + 56
    expect(treeLayout(nodes, "tb", new Map(measured), JSON_SIZES).get("A")!.y).toBe(316);
    // 行距按实测高度累计 → 260 + 22
    expect(treeLayout(nodes, "lr", new Map(measured), JSON_SIZES).get("A2")!.y).toBe(282);
  });

  it("成环的脏数据不卡死，且每个节点都拿到有限坐标", () => {
    const cyclic: LayoutNode[] = [{ id: "A", parentId: "B" }, { id: "B", parentId: "A" }];
    const pos = treeLayout(cyclic, "lr", undefined, JSON_SIZES);
    expect(pos.size).toBe(2);
    expect(Number.isFinite(pos.get("A")!.x)).toBe(true);
    expect(Number.isFinite(pos.get("B")!.y)).toBe(true);
  });

  it("自引用节点当根处理，空输入返回空布局", () => {
    expect(treeLayout([{ id: "S", parentId: "S" }]).get("S")).toEqual({ x: 0, y: 0 });
    expect(treeLayout([]).size).toBe(0);
  });
});

describe("isTreeLayoutDir", () => {
  it("只认四个方向", () => {
    for (const d of ["lr", "rl", "tb", "bt"]) expect(isTreeLayoutDir(d)).toBe(true);
    for (const d of ["LR", "", "radial", "up"]) expect(isTreeLayoutDir(d)).toBe(false);
  });
});
