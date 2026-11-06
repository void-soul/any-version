import { describe, expect, it } from "vitest";
import { MAX_TREE_DEPTH, flattenTree } from "../treeTraversal";

interface Node { id: string; children: Node[] }
const kids = (n: Node) => n.children;
const idOf = (n: Node) => n.id;

function chain(depth: number): Node[] {
    let node: Node = { id: `n${depth}`, children: [] };
    for (let i = depth - 1; i >= 0; i--) node = { id: `n${i}`, children: [node] };
    return [node];
}

describe("flattenTree 边界", () => {
    it("保持前序与深度", () => {
        const roots: Node[] = [
            { id: "a", children: [{ id: "a1", children: [] }, { id: "a2", children: [] }] },
            { id: "b", children: [] },
        ];
        const { rows, truncated } = flattenTree(roots, kids, { idOf });
        expect(rows.map((r) => `${r.node.id}@${r.depth}`)).toEqual([
            "a@0", "a1@1", "a2@1", "b@0",
        ]);
        expect(truncated).toBe(false);
    });

    it("断开环而不是无限展开", () => {
        const a: Node = { id: "a", children: [] };
        const b: Node = { id: "b", children: [a] };
        a.children.push(b);
        const { rows, truncated } = flattenTree([a], kids, { idOf });
        expect(rows.length).toBeLessThan(5);
        expect(truncated).toBe(true);
    });

    it("超深链在深度上限处截断且栈不溢出", () => {
        const { rows, truncated } = flattenTree(chain(MAX_TREE_DEPTH + 500), kids, { idOf });
        expect(rows.length).toBe(MAX_TREE_DEPTH + 1);
        expect(truncated).toBe(true);
    });

    it("共享子节点在不同分支下都保留", () => {
        const shared: Node = { id: "shared", children: [] };
        const roots: Node[] = [
            { id: "a", children: [shared] },
            { id: "b", children: [shared] },
        ];
        const { rows } = flattenTree(roots, kids, { idOf });
        expect(rows.filter((r) => r.node.id === "shared")).toHaveLength(2);
    });
});
