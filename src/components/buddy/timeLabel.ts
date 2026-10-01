/**
 * 时间标签（账号表格里的「到期」列与自定义时间列）的显示格式。
 *
 * 这些列一格只有几十像素宽，而一个账号可以挂好几个自定义时间列 ——
 * `55d21h` 这种写法逼得列宽下不来，一加列就把表格撑爆。所以统一写成
 * `55.21`（= 55 天 21 小时）：5 个字符，列宽因此可以压到 72px。
 * 精确时刻没有丢，仍在悬停里（`title` 给的是完整的日期时间）。
 */

/**
 * 剩余 / 已过时间的紧凑写法：`天数.小时`（小时两位补零）。
 *
 * - `55.21` = 还剩 55 天 21 小时
 * - `0.20`  = 还剩 20 小时（不足一天也是同一形状，不会读错）
 * - `-3.05` = **已过** 3 天 5 小时（负号表示已经过了那个时刻）
 *
 * 小时补零是刻意的：`3.5` 会被读成「三天半」，而实际是 3 天 5 小时。
 */
export function formatCountdown(ms: number): string {
  if (!Number.isFinite(ms)) return "0.00";
  const sign = ms < 0 ? "-" : "";
  const total = Math.abs(ms);
  const days = Math.floor(total / 86_400_000);
  const hours = Math.floor((total % 86_400_000) / 3_600_000);
  return `${sign}${days}.${String(hours).padStart(2, "0")}`;
}
