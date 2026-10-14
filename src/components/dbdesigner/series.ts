// 「同系列」的唯一判定：**表名前缀 ∪ 外键连通**。
//
// 配色（同一系列同色）与自动布局（同一系列聚在一块）共用这一份实现 ——
// 两处对「系列」的理解一旦分叉，就会出现「颜色成对、位置却分开」这种自相矛盾的画面。
//
// 为什么是「前缀 ∪ 外键」而不是只看其中之一：
//   · 只看外键：`user_paper` / `user_token` 这种还没建外键的同系列表会被拆开，
//     颜色各不同、位置也各在一角 —— 而它们显然是一家人（用户明确要求按表名分组）。
//   · 只看前缀：`billing_invoice` 与 `invoice` 是同一样东西，却会被错归成两个系列
//     （这是早先只用外键连通分量时特意规避的坑，不能又踩回去）。
// 所以两者取并集：名字像的一堆先聚起来，有外键关系的再跨名字合并。
//
// 布局与配色都只调 `groupByPrefix`；`groupByForeignKey` 保留为它的输入之一。

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
 * 系列分组键：表名**第一个下划线之前**的部分。
 *
 * `user_paper` → `user`；`product_category` → `product`；没有下划线的表用整名
 * （`order` → `order`，它自成一组）。空前缀（`_foo`，非法表名）也退化成整名。
 */
export function seriesKeyOf(name: string): string {
  const t = (name ?? "").trim();
  const i = t.indexOf("_");
  return i > 0 ? t.slice(0, i) : t;
}

/**
 * 按「表名前缀 ∪ 外键关系」把表分组。**这是配色与布局唯一的系列口径。**
 *
 * 返回顺序 = 各组首次出现在文档中的顺序：调用方据此取色与摆位，
 * 保证同一份设计每次算出来完全一样（布局结果会写进 `.dbdesign.json`）。
 */
export function groupByPrefix(
  doc: DbDesignDocument,
  nodes: DbDesignNode[] = doc.nodes.filter((n) => n.kind === "table"),
): DbDesignNode[][] {
  // 并查集：先按「前缀键」合并，再按外键合并。两步合成一次遍历即可。
  const parent = new Map<string, string>();
  const find = (x: string): string => {
    let root = x;
    while (parent.get(root) !== root) root = parent.get(root)!;
    let cur = x;
    while (parent.get(cur) !== root) {
      const next = parent.get(cur)!;
      parent.set(cur, root);
      cur = next;
    }
    return root;
  };
  const union = (a: string, b: string) => {
    const ra = find(a);
    const rb = find(b);
    if (ra !== rb) parent.set(ra, rb);
  };
  for (const n of nodes) parent.set(n.id, n.id);
  // 1) 前缀：同前缀的表先归一堆（键用节点 id 的形式参与 union，避免表名重复时串组）
  const byKey = new Map<string, string>();
  for (const n of nodes) {
    const key = seriesKeyOf(n.name).toLowerCase();
    const first = byKey.get(key);
    if (first === undefined) byKey.set(key, n.id);
    else union(first, n.id);
  }
  // 2) 外键：名字不像但有引用关系的也拉到一起（跨前缀合并）
  for (const r of doc.relations) {
    if (r.kind === "n-n") continue;
    if (!parent.has(r.from.node)) continue;
    if (!parent.has(r.to.node)) continue;
    union(r.from.node, r.to.node);
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
