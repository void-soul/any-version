/**
 * 树遍历的公共边界（AUDIT-044 / 050 单一真源）。
 *
 * 用户数据（文件夹树 / 分类树 / 拓扑树）来自 SQLite，外部导入或手工改库可能
 * 形成环或超深链；React 递归组件会直接栈溢出导致整个 WebView 白屏。
 * 所有树渲染/拉平必须带深度上限与路径去重。
 */

export const MAX_TREE_DEPTH = 64;

/** 从任意节点取子节点；返回空数组表示叶子。 */
export type ChildrenOf<T> = (node: T) => readonly T[];

/**
 * 迭代式前序拉平（不会栈溢出）。
 * - `seen` 沿当前路径去重，环会被断开而不是无限展开；
 * - 超过 `maxDepth` 的节点不再下钻，结果中附带 `truncated` 标记。
 */
export function flattenTree<T>(
    roots: readonly T[],
    childrenOf: ChildrenOf<T>,
    options: { idOf?: (node: T) => string; maxDepth?: number } = {},
): { rows: Array<{ node: T; depth: number }>; truncated: boolean } {
    const maxDepth = options.maxDepth ?? MAX_TREE_DEPTH;
    const rows: Array<{ node: T; depth: number }> = [];
    let truncated = false;
    // 栈元素带「当前路径上的祖先 id 集合」，避免全局去重误杀共享子节点。
    const stack: Array<{ node: T; depth: number; path: Set<string> }> = [];
    // 反序入栈以保持原有展示顺序（roots 与 children 都适用）。
    for (let i = roots.length - 1; i >= 0; i--) {
        stack.push({ node: roots[i], depth: 0, path: new Set() });
    }

    while (stack.length) {
        const item = stack.pop()!;
        const id = options.idOf?.(item.node);
        if (id !== undefined && item.path.has(id)) {
            truncated = true; // 检测到环
            continue;
        }
        rows.push({ node: item.node, depth: item.depth });
        if (item.depth >= maxDepth) {
            if (childrenOf(item.node).length > 0) truncated = true;
            continue;
        }
        const nextPath = new Set(item.path);
        if (id !== undefined) nextPath.add(id);
        const children = childrenOf(item.node);
        // 反序入栈以保持原有展示顺序
        for (let i = children.length - 1; i >= 0; i--) {
            stack.push({ node: children[i], depth: item.depth + 1, path: nextPath });
        }
    }
    return { rows, truncated };
}
