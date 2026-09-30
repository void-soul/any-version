// 积分构成条：一个积分段一个方块。
//
// - **颜色 = 到期远近**：红（快到期）→ 绿（还早 / 永不过期）；已过期画成灰色 ——
//   过期的积分用不掉了，再按「紧急程度」染色反而误导；
// - **长度 = 该段在这个账号剩余积分中的占比**（整条恒满宽）：
//   因此它表达的是**构成**，不用于跨账号比大小（跨账号看「额度」列与悬停数字）；
// - 悬停显示：类型 · 剩余/总量 · 到期时刻 · 还剩几天。
//
// 数据来自 `mergeCreditSegments`（同一礼包已合并、按到期先后排序），
// 所以方块从左到右依次是「最快到期 → 最慢到期」。

import { useTranslation } from "react-i18next";

import {
  formatQuotaNumber,
  segmentDaysLeft,
  segmentHue,
  type CreditSegment,
} from "./quota";

interface Props {
  segments: CreditSegment[];
  /** 时间基准：列表里所有行传同一个 now，避免逐行取时间导致色差 */
  now?: number;
  /** 点击整条 → 打开该账号的用量详情 */
  onClick?: () => void;
}

export default function CreditSegmentsBar({ segments, now = Date.now(), onClick }: Props) {
  const { t } = useTranslation();

  const describe = (segment: CreditSegment): string => {
    const amount = `${formatQuotaNumber(segment.remaining)}/${formatQuotaNumber(segment.total)}`;
    const head = `${segment.source} · ${amount}`;
    if (segment.expiresAt == null) {
      return `${head} · ${t("buddy.creditNeverExpires")}`;
    }
    const absolute = new Date(segment.expiresAt).toLocaleString();
    const days = segmentDaysLeft(segment, now);
    if (days === 0) {
      return `${head} · ${absolute} · ${t("buddy.overdue")}`;
    }
    return `${head} · ${absolute} · ${t("buddy.creditDaysLeft", { days })}`;
  };

  if (segments.length === 0) {
    return <span className="text-slate-600">—</span>;
  }

  return (
    <div
      className={`flex w-full h-2.5 gap-px rounded-sm overflow-hidden bg-white/5 ${
        onClick ? "cursor-pointer" : ""
      }`}
      onClick={onClick}
      // 整条也给一份：鼠标落在方块间隙上时不会什么都不显示
      title={segments.map(describe).join("\n")}
    >
      {segments.map((segment, index) => {
        const hue = segmentHue(segment, now);
        return (
          <span
            key={`${segment.packageCode || segment.source}-${index}`}
            className="block h-full"
            style={{
              flexGrow: Math.max(0, segment.remaining),
              flexBasis: 0,
              // 占比极小的段也要看得见（否则会退化成一条看不见的缝）
              minWidth: 3,
              backgroundColor:
                hue === null ? "rgba(148,163,184,0.45)" : `hsl(${hue} 68% 45%)`,
            }}
            title={describe(segment)}
          />
        );
      })}
    </div>
  );
}
