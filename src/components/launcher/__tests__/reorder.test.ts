import { describe, expect, it } from "vitest";
import { buildReorderOrders } from "../reorder";
import type { Item } from "../types";

function item(id: number, classificationId: number): Item {
  return {
    id,
    classificationId,
    name: `item-${id}`,
    itemType: 0,
    data: {},
    shortcutKey: null,
    globalShortcutKey: false,
    order: id,
  };
}

describe("buildReorderOrders", () => {
  it("同分类：完全按传入顺序（界面所见顺序）重新编号 0..n-1", () => {
    const list = [item(3, 1), item(1, 1), item(2, 1)];

    expect(buildReorderOrders(list, [1])).toEqual([
      {
        categoryId: 1,
        orders: [
          [3, 0],
          [1, 1],
          [2, 2],
        ],
      },
    ]);
  });

  it("跨分类：每个分类独立编号，未涉及分类不产生指令", () => {
    const list = [item(1, 1), item(4, 2), item(2, 1), item(5, 2), item(9, 3)];

    const result = buildReorderOrders(list, [1, 2]);

    expect(result).toEqual([
      {
        categoryId: 1,
        orders: [
          [1, 0],
          [2, 1],
        ],
      },
      {
        categoryId: 2,
        orders: [
          [4, 0],
          [5, 1],
        ],
      },
    ]);
  });

  it("空分类返回空指令，调用方据此跳过写库", () => {
    const result = buildReorderOrders([item(1, 1)], [2]);

    expect(result).toEqual([{ categoryId: 2, orders: [] }]);
  });

  it("被「只显示有效项目」隐藏的项只要在完整列表里就一起编号，不与可见项序号冲突", () => {
    // id=1 为隐藏项（exists=false），完整列表顺序为 1,2,3
    const list = [item(1, 1), item(2, 1), item(3, 1)];

    const [group] = buildReorderOrders(list, [1]);

    expect(group.orders.map(([, order]) => order)).toEqual([0, 1, 2]);
    expect(group.orders.some(([id]) => id === 1)).toBe(true);
  });
});
