import type { BuddyAccount } from "./BuddyPanel.model";

/** 用量时间戳单位归一：历史 OAuth 账号曾落过毫秒值，一律换成秒再判新鲜度。 */
export function usageUpdatedAtSeconds(t: number | null | undefined): number | null {
  if (!t || t <= 0) return null;
  return t > 1e11 ? Math.floor(t / 1000) : t;
}

export type UsageFreshness = {
  kind: "unknown" | "fresh" | "cached" | "failed";
  ageSec: number | null;
};

/** 落盘数据超过 60 秒就不算「刚查的」；查询失败优先，此时展示的是上次成功的旧数字。 */
export function usageFreshness(
  account: Pick<BuddyAccount, "usageUpdatedAt" | "quotaQueryLastError">,
  nowMs: number,
): UsageFreshness {
  const updatedAtSec = usageUpdatedAtSeconds(account.usageUpdatedAt);
  const ageSec = updatedAtSec === null ? null : Math.max(0, Math.floor(nowMs / 1000) - updatedAtSec);
  if (account.quotaQueryLastError) return { kind: "failed", ageSec };
  if (updatedAtSec === null) return { kind: "unknown", ageSec: null };
  return { kind: ageSec !== null && ageSec <= 60 ? "fresh" : "cached", ageSec };
}

/** 相对年龄只用于展示，不参与判定；交给 Intl 做中英文本地化。 */
export function usageAgeLabel(ageSec: number, language: string): string {
  const rtf = new Intl.RelativeTimeFormat(language, { numeric: "always" });
  if (ageSec < 60) return rtf.format(-Math.max(1, Math.round(ageSec)), "second");
  if (ageSec < 3600) return rtf.format(-Math.floor(ageSec / 60), "minute");
  if (ageSec < 86400) return rtf.format(-Math.floor(ageSec / 3600), "hour");
  return rtf.format(-Math.floor(ageSec / 86400), "day");
}

/** 选出「缓存 / 已过时 / 失败」的文案键与年龄（列表悬停与用量弹窗共用）。 */
export function usageFreshnessLabel(
  freshness: UsageFreshness,
  language: string,
): { key: string; age?: string } | null {
  if (freshness.kind === "fresh" || freshness.kind === "unknown") return null;
  if (freshness.kind === "failed") return { key: "buddy.usageFailedStale" };
  const age = usageAgeLabel(freshness.ageSec ?? 0, language);
  return freshness.ageSec !== null && freshness.ageSec > 600
    ? { key: "buddy.usageStale", age }
    : { key: "buddy.usageCached", age };
}
