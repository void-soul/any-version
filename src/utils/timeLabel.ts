/**
 * 时间标签（紧凑倒计时）的通用格式化。
 *
 * 从 Buddy 模块上移到共享工具：AI 模型的供应商促销倒计时也用它。
 * 这些场景一格只有几十像素宽，`55d21h` 这种写法逼得列宽下不来，所以统一写成
 * `55.21`（= 55 天 21 小时）：5 个字符。精确时刻由调用方在悬停里给出。
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
