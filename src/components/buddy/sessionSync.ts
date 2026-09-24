/**
 * 会话同步明细的前端纯函数：状态筛选。
 *
 * 后端的明细最多 500 条（`session_sync::MAX_DETAILS`），而冲突/失败常常只有几条，
 * 不筛就要在一长串「跳过」里翻。抽成纯函数放在这里，便于单测；
 * `BuddyPanel` 只负责渲染。
 */

/** 明细状态筛选项；`all` 表示不筛。 */
export type SyncStatusFilter = "all" | "copied" | "skipped" | "partial" | "conflict" | "failed";

/** 筛选按钮的顺序（取值与后端 `SessionSyncStatus` 一致，中文由 i18n 提供）。 */
export const SYNC_STATUS_FILTERS: SyncStatusFilter[] = [
  "all",
  "copied",
  "skipped",
  "partial",
  "conflict",
  "failed",
];

/**
 * 按状态过滤明细。
 *
 * - `all` 原样返回入参（不复制，便于调用方做恒等比较）；
 * - 其它取值返回新数组，**不改动入参**；未知取值得到空数组而不是抛错
 *   （后端将来新增状态时，旧前端只会显示「该状态下没有会话」）。
 *
 * 用 `T extends { status: string }` 而不是直接引用 `BuddySessionSyncDetail`，
 * 避免这个纯函数模块反向依赖组件文件。
 */
export function filterSyncDetails<T extends { status: string }>(
  details: T[],
  filter: string,
): T[] {
  if (filter === "all") {
    return details;
  }
  return details.filter((detail) => detail.status === filter);
}
