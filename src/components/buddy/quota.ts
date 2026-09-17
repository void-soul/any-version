/**
 * 积分段解析 / 合并 / 轮换建议。
 *
 * 语义对齐参考实现 WorkDaddy `scripts/credit-segments.js` + `scripts/credit-rotation.js`：
 * - 同一礼包（`packageCode | source` + 到期时间）的多条记录合并成一行——官方会把「一次赠送」
 *   拆成多条记录（例如 10 条 500 表示 5000 的礼包），合并后按礼包展示；
 * - 单条记录的 `total` 不得小于 `remaining`；
 * - 已耗尽（remaining ≤ 0）的段不参与展示与建议；
 * - 轮换建议只考虑「未过期 + 有剩余」的账号：最早到期优先，同到期剩余多优先。
 */

/** 前端已解析的额度行（见 BuddyPanel `parseQuotaItems`）。 */
export interface QuotaItemLike {
  packageName: string;
  used: number;
  total: number;
  remain: number;
  unlimited?: boolean;
  cycleEndTime?: string | null;
  /** 官方礼包码（有则按它合并同一礼包的多条记录） */
  packageCode?: string;
}

// 官方计费接口历史上给同一个值换过多次字段名（对齐参考实现的别名表），
// 缺失任意一个别名都会让额度显示成 0，故集中在这里维护。
export const REMAINING_FIELDS = [
  "SlicePeriodCapacityRemainPrecise",
  "SlicePeriodCapacityRemain",
  "CycleCapacityRemainPrecise",
  "CycleCapacityRemain",
  "CapacityRemainPrecise",
  "CapacityRemain",
  "RemainPrecise",
  "Remain",
  "Remaining",
  "Balance",
];

export const TOTAL_FIELDS = [
  "SlicePeriodCapacitySizePrecise",
  "SlicePeriodCapacitySize",
  "CycleCapacitySizePrecise",
  "CycleCapacitySize",
  "CycleCapacityPrecise",
  "CycleCapacity",
  "CapacityPrecise",
  "Capacity",
  "TotalCapacityPrecise",
  "TotalCapacity",
  "PackageCapacity",
  "Quota",
  "Amount",
];

export const USED_FIELDS = [
  "SlicePeriodCapacityUsed",
  "CycleCapacityUsedPrecise",
  "CycleCapacityUsed",
  "CapacityUsedPrecise",
  "CapacityUsed",
  "Used",
  "Consumed",
];

export const EXPIRY_FIELDS = [
  "DeductionEndTime",
  "ExpiredTime",
  "SlicePeriodEndTime",
  "PackageEndTime",
  "EndTime",
  "CycleEndTime",
  "ExpireTime",
  "ExpirationTime",
  "ValidEndTime",
  "ValidPeriodEndTime",
  "EndAt",
  "ExpireAt",
];

export const LABEL_FIELDS = [
  "PackageName",
  "PackageTypeName",
  "AccountName",
  "ProductName",
  "Name",
  "RuleName",
  "Description",
];

/** 按别名表取第一个可用数值（`undefined`/`null`/`""` 与非法值都跳过）。 */
export function pickNumber(
  source: Record<string, unknown> | null | undefined,
  fields: string[]
): number | null {
  if (!source) return null;
  for (const field of fields) {
    const raw = source[field];
    if (raw === undefined || raw === null || raw === "") continue;
    const value = Number(raw);
    if (Number.isFinite(value)) return value;
  }
  return null;
}

/** 按别名表取第一个可用文本。 */
export function pickText(
  source: Record<string, unknown> | null | undefined,
  fields: string[]
): string {
  if (!source) return "";
  for (const field of fields) {
    const raw = source[field];
    if (typeof raw === "string" && raw.trim()) return raw.trim();
  }
  return "";
}

/** 按别名表取第一个可用时间戳（秒/毫秒/字符串归一为毫秒）。 */
export function pickTimestamp(
  source: Record<string, unknown> | null | undefined,
  fields: string[]
): number | null {
  if (!source) return null;
  for (const field of fields) {
    const parsed = parseCreditTimestamp(source[field]);
    if (parsed !== null) return parsed;
  }
  return null;
}

/** 单个积分段（礼包 / 到期时间 / 来源 / 剩余）。 */
export interface CreditSegment {
  remaining: number;
  total: number;
  expiresAt: number | null;
  source: string;
  packageCode: string;
}

export interface CreditSummary {
  remain: number;
  total: number;
  used: number;
  hasData: boolean;
}

export interface RotationAccount {
  accountId: string;
  uid?: string | null;
  label: string;
  segments: CreditSegment[];
}

export interface RotationCandidate {
  accountId: string;
  label: string;
  remaining: number;
  expiresAt: number | null;
}

const DEFAULT_SOURCE = "积分";

const round2 = (value: number) => Math.round(value * 100) / 100;

/** 时间戳归一：数字（秒 / 毫秒）与 ISO 字符串都支持，非法值返回 null。 */
export function parseCreditTimestamp(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    // 秒级时间戳（< 1e12）归一为毫秒
    return value < 1e12 ? Math.round(value * 1000) : Math.round(value);
  }
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) return null;
    const asNumber = Number(trimmed);
    if (Number.isFinite(asNumber)) return parseCreditTimestamp(asNumber);
    const parsed = Date.parse(trimmed);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

/** 额度行 → 积分段（丢弃已耗尽与不限量行；不限量由账号卡片单独展示）。 */
export function segmentsFromQuotaItems(items: QuotaItemLike[]): CreditSegment[] {
  const segments: CreditSegment[] = [];
  for (const item of items) {
    if (item.unlimited) continue;
    const remaining = Number(item.remain);
    if (!Number.isFinite(remaining) || remaining <= 0) continue;
    const rawTotal = Number(item.total);
    const total = Number.isFinite(rawTotal) ? Math.max(rawTotal, remaining) : remaining;
    segments.push({
      remaining: round2(remaining),
      total: round2(total),
      expiresAt: parseCreditTimestamp(item.cycleEndTime),
      source: item.packageName?.trim() || DEFAULT_SOURCE,
      packageCode: item.packageCode?.trim() || "",
    });
  }
  return segments;
}

function segmentKey(segment: CreditSegment): string {
  return `${segment.packageCode || segment.source || DEFAULT_SOURCE}|${segment.expiresAt ?? "unknown"}`;
}

/** 排序：先到期的在前，永不过期的排最后。 */
export function sortCreditSegments(segments: CreditSegment[]): CreditSegment[] {
  return [...segments]
    .filter((segment) => segment && Number(segment.remaining) > 0)
    .map((segment) => ({
      remaining: round2(Number(segment.remaining)),
      total: round2(Number(segment.total || segment.remaining)),
      expiresAt: segment.expiresAt ?? null,
      source: String(segment.source || DEFAULT_SOURCE),
      packageCode: String(segment.packageCode || ""),
    }))
    .sort((a, b) => {
      if (a.expiresAt === null && b.expiresAt !== null) return 1;
      if (a.expiresAt !== null && b.expiresAt === null) return -1;
      return (a.expiresAt ?? 0) - (b.expiresAt ?? 0);
    });
}

/** 同一礼包（key = 礼包码/来源 + 到期时间）的多条记录合并累加。 */
export function mergeCreditSegments(segments: CreditSegment[]): CreditSegment[] {
  const merged = new Map<string, CreditSegment>();
  for (const segment of segments ?? []) {
    if (!segment || !(Number(segment.remaining) > 0)) continue;
    const key = segmentKey(segment);
    const previous = merged.get(key);
    if (previous) {
      previous.remaining += Number(segment.remaining) || 0;
      previous.total += Number(segment.total || segment.remaining) || 0;
    } else {
      merged.set(key, {
        remaining: Number(segment.remaining) || 0,
        total: Number(segment.total || segment.remaining) || 0,
        expiresAt: segment.expiresAt ?? null,
        source: String(segment.source || DEFAULT_SOURCE),
        packageCode: String(segment.packageCode || ""),
      });
    }
  }
  return sortCreditSegments([...merged.values()]);
}

/** 汇总（先合并再求和）。 */
export function summarizeCreditSegments(segments: CreditSegment[]): CreditSummary {
  let remain = 0;
  let total = 0;
  let count = 0;
  for (const segment of segments ?? []) {
    if (!segment || !(Number(segment.remaining) > 0)) continue;
    remain += Number(segment.remaining) || 0;
    total += Number(segment.total || segment.remaining) || 0;
    count += 1;
  }
  return {
    remain: round2(remain),
    total: round2(total),
    used: round2(Math.max(0, total - remain)),
    hasData: count > 0,
  };
}

/** 最近到期且仍有剩余、且未过期的段（永不过期排最后）。 */
export function nearestExpiringSegment(
  segments: CreditSegment[],
  now: number = Date.now()
): CreditSegment | null {
  const candidates = (segments ?? [])
    .filter((segment) => segment && Number(segment.remaining) > 0)
    .filter((segment) => segment.expiresAt === null || segment.expiresAt > now)
    .sort((a, b) => {
      if (a.expiresAt === null && b.expiresAt !== null) return 1;
      if (a.expiresAt !== null && b.expiresAt === null) return -1;
      return (a.expiresAt ?? Number.MAX_SAFE_INTEGER) - (b.expiresAt ?? Number.MAX_SAFE_INTEGER);
    });
  return candidates[0] ?? null;
}

/**
 * 积分不足时的换号建议：排除当前账号与无剩余账号，
 * 最早到期优先，同到期时间取剩余多的。
 */
export function selectRotationCandidate(
  accounts: RotationAccount[],
  currentUid: string | null,
  now: number = Date.now()
): RotationCandidate | null {
  const current = String(currentUid ?? "");
  const candidates = (accounts ?? [])
    .filter((account) => String(account.uid ?? "") !== current)
    .map((account) => ({
      account,
      segment: nearestExpiringSegment(account.segments, now),
    }))
    .filter((entry) => entry.segment && entry.segment.remaining > 0)
    .sort((a, b) => {
      const left = a.segment!.expiresAt ?? Number.MAX_SAFE_INTEGER;
      const right = b.segment!.expiresAt ?? Number.MAX_SAFE_INTEGER;
      if (left !== right) return left - right;
      return b.segment!.remaining - a.segment!.remaining;
    });

  const best = candidates[0];
  if (!best) return null;
  return {
    accountId: best.account.accountId,
    label: best.account.label,
    remaining: best.segment!.remaining,
    expiresAt: best.segment!.expiresAt,
  };
}
