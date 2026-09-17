import type { Item } from "./types";

/** 一个分类的重排序指令：`[项目 id, 新序号]` 列表。 */
export interface CategoryReorder {
  categoryId: number;
  orders: [number, number][];
}

/**
 * 按「界面当前顺序」生成各分类的排序指令。
 *
 * 传入的 items 必须是拖拽让位结束后前端维护的完整列表（含被「只显示有效项目」隐藏的项），
 * 顺序即用户所见的最终顺序；函数只负责按分类切分并重新编号 0..n-1。
 *
 * 之所以不接收拖拽结束事件的 over 位置：松手瞬间指针可能落在卡片间隙（over 为容器）
 * 或容器之外（over 为 null），按 over 推导会漏保存或把项目挪到末尾，
 * 表现为「拖完看着生效，重新打开又变回原样」。
 */
export function buildReorderOrders(
  items: readonly Item[],
  categoryIds: readonly number[],
): CategoryReorder[] {
  return categoryIds.map((categoryId) => {
    const grouped = items.filter((item) => item.classificationId === categoryId);
    return {
      categoryId,
      orders: grouped.map((item, index) => [item.id, index] as [number, number]),
    };
  });
}
