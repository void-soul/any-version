/**
 * 会话同步明细的前端纯函数：状态筛选。
 *
 * 后端的明细最多 500 条（`session_sync::MAX_DETAILS`），而冲突/失败常常只有几条，
 * 不筛就要在一长串「跳过」里翻。抽成纯函数放在这里，便于单测；
 * `BuddyPanel` 只负责渲染。
 */

/** 明细状态筛选项；`all` 表示不筛，`resolved` = 冲突**已处理过**的（按处理结果过滤）。 */
export type SyncStatusFilter =
  | "all"
  | "copied"
  | "skipped"
  | "partial"
  | "conflict"
  | "failed"
  | "resolved";

/** 筛选按钮的顺序（取值与后端 `SessionSyncStatus` 一致，中文由 i18n 提供）。
 *  `resolved` 不是后端状态，而是「有冲突处理结果」的前端视图，故排在最后。 */
export const SYNC_STATUS_FILTERS: SyncStatusFilter[] = [
  "all",
  "copied",
  "skipped",
  "partial",
  "conflict",
  "failed",
  "resolved",
];

/**
 * 按状态过滤明细。
 *
 * - `all` 原样返回入参（不复制，便于调用方做恒等比较）；
 * - `resolved` 按「有处理结果」过滤（`resolvedIds` 为处理过的会话 id 集合）；
 * - 其它取值返回新数组，**不改动入参**；未知取值得到空数组而不是抛错
 *   （后端将来新增状态时，旧前端只会显示「该状态下没有会话」）。
 *
 * 用 `T extends { id: string; status: string }` 而不是直接引用 `BuddySessionSyncDetail`，
 * 避免这个纯函数模块反向依赖组件文件。
 */
export function filterSyncDetails<T extends { id: string; status: string }>(
  details: T[],
  filter: string,
  resolvedIds?: ReadonlySet<string>,
): T[] {
  if (filter === "all") {
    return details;
  }
  if (filter === "resolved") {
    const ids = resolvedIds ?? new Set<string>();
    return details.filter((detail) => ids.has(detail.id));
  }
  return details.filter((detail) => detail.status === filter);
}
