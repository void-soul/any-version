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

// ─── 「高亮 / 整理选中」的唯一口径 ───

/**
 * 选中表 + 与它直接有外键关系的表（**无向**，一跳）。
 *
 * 这就是界面上高亮的那批表，也就是「整理选中」要整理的那批表 —— 两者必须
 * **同一个定义**：曾经高亮用一份内联写法、整理用 `groupByForeignKey` 的整个连通
 * 分量，两边各写各的，于是一份「所有表外键连通」的业务设计里，
 * 高亮看着 6 张、实际整理了 22 张，提示条如实报「已整理 22 张表」——
 * 标签、提示与实现说的不是同一件事。
 *
 * 刻意**不是**连通分量：连通分量会把「同系列但你没选中的表」一起卷进来，
 * 而用户要的是「忽略其他所有节点，只对齐选中的」。
 */
export function relatedTableIds(doc: DbDesignDocument, selectedId: string | null): Set<string> {
  const set = new Set<string>();
  if (!selectedId) return set;
  set.add(selectedId);
  for (const r of doc.relations) {
    if (r.from.node === selectedId) set.add(r.to.node);
    else if (r.to.node === selectedId) set.add(r.from.node);
  }
  return set;
}

/**
 * 取整理用的**子文档**：只含 `ids` 里的表，以及两端都在集合内的关联。
 *
 * 关联必须双向过滤：指向集合外的关联会让分层算法把一个不存在的父节点算进
 * 层号，结果是这批表被推到错误的列上。n-n 不参与分层（同 `layoutGroup` 的处理）。
 *
 * 父表不在集合内的节点会自然成为这个子布局的根 —— 这正是「忽略其它节点」的
 * 预期结果，不是缺陷。
 */
export function buildArrangeSubDoc(
  doc: DbDesignDocument,
  ids: Set<string>,
): DbDesignDocument {
  return {
    ...doc,
    nodes: doc.nodes.filter((n) => n.kind === "table" && ids.has(n.id)),
    relations: doc.relations.filter(
      (r) => r.kind !== "n-n" && ids.has(r.from.node) && ids.has(r.to.node),
    ),
  };
}
