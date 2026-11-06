/**
 * JSON 浏览器 / 图形画布共享的契约类型（AUDIT-051）。
 *
 * 原先定义在展示层组件 `JsonBrowser.tsx` 里，`CanvasFlow.tsx` 与
 * `JsonBrowser.tree.ts` 反向 import 展示层，形成静态环（当前靠 `import type`
 * 擦除才没有运行时问题）。类型下沉到 contracts 后环被彻底消除。
 */

export type JsonValue =
    | null
    | boolean
    | number
    | string
    | JsonValue[]
    | { [key: string]: JsonValue };

export type SearchMatches = {
    query: string;
    /** 命中路径及其祖先，用于自动展开。 */
    paths: Set<string>;
    /** 直接命中（不含祖先）的路径。 */
    directPaths: Set<string>;
    /** 搜索因节点预算被截断。 */
    truncated?: boolean;
};

export type FlatTreeRow = {
    path: string;
    name: string;
    value: JsonValue;
    depth: number;
    container: boolean;
    collapsed: boolean;
};
