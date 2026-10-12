// 「同系列」的唯一判定：外键关系的连通分量。
//
// 配色（同一系列同色）与自动布局（同一系列聚在一块）共用这一份实现 ——
// 两处对「系列」的理解一旦分叉，就会出现「颜色成对、位置却分开」这种自相矛盾的画面。
//
// 刻意不用表名前缀：billing_invoice 与 invoice 是同一样东西，前缀法会把它们错归成两个系列。

import type { DbDesignDocument, DbDesignNode } from "./types";

/**
 * 按外键连通分量把表分组，返回的每个组内节点必然互相引用得到。
 *
 * 分组顺序与颜色稳定性：按节点在文档中的出现顺序返回，调用方据此分配颜色/
 * 坐标位置，保证同一份设计每次算出来都一样（力导向那种「每次都不一样」
 * 不能用 —— 布局结果会写进设计文件）。
 */
export function groupByForeignKey(
  doc: DbDesignDocument,
  nodes: DbDesignNode[] = doc.nodes.filter((n) => n.kind === "table"),
): DbDesignNode[][] {
  const parent = new Map<string, string>();
  const find = (x: string): string => {
    let root = x;
    while (parent.get(root) !== root) root = parent.get(root)!;
    // 路径压缩：避免长链反复遍历
    let cur = x;
    while (parent.get(cur) !== root) {
      const next = parent.get(cur)!;
      parent.set(cur, root);
      cur = next;
    }
    return root;
  };
  for (const n of nodes) parent.set(n.id, n.id);
  for (const r of doc.relations) {
    // 两端都得是本次参与布局的节点；悬空关系直接跳过，
    // 否则 find() 会顺着不存在的 key 一路找下去
    if (!parent.has(r.from.node) || !parent.has(r.to.node)) continue;
    const a = find(r.from.node);
    const b = find(r.to.node);
    if (a !== b) parent.set(a, b);
  }
  const byRoot = new Map<string, DbDesignNode[]>();
  for (const n of nodes) {
    const root = find(n.id);
    const list = byRoot.get(root);
    if (list) list.push(n);
    else byRoot.set(root, [n]);
  }
  return [...byRoot.values()];
}
