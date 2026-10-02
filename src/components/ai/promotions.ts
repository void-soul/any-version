/**
 * 供应商促销倒计时（纯逻辑）。
 *
 * 供应商经常搞「xx 模型免费 1 个月」这类活动，用户记不住 —— 在供应商行内
 * 挂一组倒计时 chip（见 ModelConfig），到期置灰、保留 7 天后自动清除。
 *
 * 清除是**惰性**的：不跑定时器，渲染时过滤（`prunePromotions`），
 * 下次保存供应商时顺带落盘清掉。不打开 AI 模块就不清 —— 无所谓，数据在配置里。
 */

import { formatCountdown } from "../../utils/timeLabel";
import type { ProviderPromotion } from "./types";

/** 已结束的活动保留多久再清（给用户一周回看「我好像有个活动」）。 */
export const PROMOTION_RETENTION_MS = 7 * 86_400_000;

const DAY = 86_400_000;

/** chip 的紧急程度（颜色跟着走）：ended 置灰、urgent 红、soon 琥珀、normal 常规。 */
export type PromotionState = "ended" | "urgent" | "soon" | "normal";

export function promotionState(endsAt: number, now: number): PromotionState {
  const left = endsAt - now;
  if (left <= 0) return "ended";
  if (left <= 3 * DAY) return "urgent";
  if (left <= 7 * DAY) return "soon";
  return "normal";
}

/** 惰性 GC：丢掉「已过期超过 7 天」的活动（过期整 7 天仍保留）。 */
export function prunePromotions(
  list: ProviderPromotion[] | undefined | null,
  now: number
): ProviderPromotion[] {
  return (list ?? []).filter((p) => now - p.ends_at <= PROMOTION_RETENTION_MS);
}

/** chip 上的倒计时文本：进行中 `12.05`；已结束 → null（UI 用「已结束」文案）。 */
export function promotionCountdown(endsAt: number, now: number): string | null {
  if (endsAt - now <= 0) return null;
  return formatCountdown(endsAt - now);
}
