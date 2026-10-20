/**
 * 画布高亮的**通用作用域**：一次 BFS 同时得出「亮哪些节点」与「亮哪些连线」。
 *
 * 为什么抽到这里：数据库设计器先有了这套口径（`dbdesigner/series.ts` 的
 * `highlightScope`），而思维导图、JSON 浏览的画布是同一件事 —— 选中一个节点，
 * 把它周围几跳点亮、其余压暗。三处各写一份的话，「亮哪些」迟早会在某个图上
 * 分叉，而分叉的表现是纯观感问题：不报错，只让人以为哪里坏了。
 *
 * 与 `series.ts` 那份的区别只有一点：那份认 `DbDesignDocument`（关系两端是
 * `{node, field}`），这里认最普通的一条边 `{ id, source, target }` ——
 * 于是父子树（思维导图 / JSON 浏览）与外键图（设计器）都能直接喂进来。
 */

/** 高亮层级的合法范围：1 = 只直接相邻，5 = 五跳以内 */
export const GRAPH_DEPTHS = [1, 2, 3, 4, 5] as const;
export type GraphDepth = (typeof GRAPH_DEPTHS)[number];
export const DEFAULT_GRAPH_DEPTH: GraphDepth = 1;

/** 把任意输入（localStorage 字符串等）收敛成合法层级，非法值回落默认 */
export function clampGraphDepth(v: unknown): GraphDepth {
  const n = typeof v === "number" ? v : Number(v);
  return (GRAPH_DEPTHS as readonly number[]).includes(n) ? (n as GraphDepth) : DEFAULT_GRAPH_DEPTH;
}

/** 作用域要的图：节点 id 集合 + 无向边（边 id 是连线高亮的唯一来源） */
export interface ScopeEdge {
  id: string;
  source: string;
  target: string;
}

export interface GraphScope {
  /** 每个节点离选中项几跳（选中项自己 = 0）。不在范围内的 id 查不到。 */
  hops: Map<string, number>;
  /** d 步以内 BFS **实际走过**的边 id —— 连线高亮的唯一来源。 */
  edgeIds: Set<string>;
}

/**
 * 层级 = **从选中项出发能走多远**。所以亮的是 d 步以内实际走过的那些连线，
 * 而不是「两端只要有一端落在 d 跳集合里就算」。
 *
 * 两者差在**同层内部的横向边**：选中 x，一跳邻居 a、b，而 a 与 b 之间也有一条线 ——
 * 集合口径会点亮 a–b，可从 x 出发要走 x→a→b 第二步才到 b，层级=1 时它不该亮。
 * 多出来的这类线，两端都只是远房亲戚，屏幕上就是**线亮着、节点却暗着**（用户实测）。
 *
 * 用 BFS 而不是递归：图可能成环，递归会无限深；BFS 天然按跳数分层。
 */
export function graphScope(
  nodes: readonly string[],
  edges: readonly ScopeEdge[],
  selectedId: string | null,
  depth: unknown = DEFAULT_GRAPH_DEPTH,
): GraphScope {
  const hops = new Map<string, number>();
  const edgeIds = new Set<string>();
  const maxHops = clampGraphDepth(depth);
  if (!selectedId) return { hops, edgeIds };
  // 只在**真实存在的节点**之间连边：指向已删除节点的悬空边若不滤掉，
  // 悬空 id 会被当成节点加进高亮集 —— 画布上没有它，于是它既点亮不了、
  // 又让高亮范围的数字对不上实际。
  const exists = new Set(nodes);
  if (!exists.has(selectedId)) return { hops, edgeIds };
  hops.set(selectedId, 0);
  /** 节点 id → 从它连出去的边（无向：两端各存一次） */
  const adj = new Map<string, ScopeEdge[]>();
  for (const e of edges) {
    if (!exists.has(e.source) || !exists.has(e.target)) continue;
    const a = adj.get(e.source);
    if (a) a.push(e); else adj.set(e.source, [e]);
    const b = adj.get(e.target);
    if (b) b.push(e); else adj.set(e.target, [e]);
  }
  let frontier = [selectedId];
  for (let hop = 1; hop <= maxHops && frontier.length > 0; hop++) {
    const next: string[] = [];
    for (const id of frontier) {
      for (const e of adj.get(id) ?? []) {
        const nb = e.source === id ? e.target : e.source;
        // 先记边再判已访问：走到已访问的邻居那条边**也是**从选中项走得到的路，
        // 仍属于本层级范围（x-a-b 里 a-b 那条会被 b 那一层再走一次，不该漏）。
        edgeIds.add(e.id);
        if (hops.has(nb)) continue;
        hops.set(nb, hop);
        next.push(nb);
      }
    }
    frontier = next;
  }
  return { hops, edgeIds };
}

/** 作用域命中的节点 id（= 高亮的那批，也就是「整理选中」要整理的那批） */
export function scopeNodeIds(scope: GraphScope): Set<string> {
  return new Set(scope.hops.keys());
}

export const scopeEdgeIds = (scope: GraphScope): Set<string> => scope.edgeIds;

/**
 * 按跳数退亮度：1 跳最亮，逐跳递减，**5 跳封底 0.25**。
 *
 * 为什么要有它：没有距离信息时，设到 3 跳与设到 1 跳画出来完全一样
 * （都是「亮或暗」），用户就拿到了一个当摆设的开关。
 *
 * 为什么封底 0.25 而不是 0：退到 0 等于把五跳画成四跳 —— 层级滑块
 * 在最远端失去意义；留 0.25 至少还能看出「这一层更远」。
 */
export function hopFade(hop: number | undefined): number {
  if (hop === undefined || hop <= 1) return 1;
  return Math.max(0.25, 1 - (hop - 1) * 0.25);
}
