// 积分构成条：一个积分段一个方块，条右侧是本账号的**剩余总额**。
//
// - **颜色 = 到期远近**：红（快到期）→ 绿（还早 / 永不过期）；已过期画成灰色 ——
//   过期的积分用不掉了，再按「紧急程度」染色反而误导；
// - **长度 = 该段在这个账号剩余积分中的占比**（整条恒满宽）：
//   因此它表达的是**构成**，不用于跨账号比大小（跨账号看悬停数字与右侧总额）；
// - **块内只显示剩余天数**（纯数字，方块里默认单位就是天）：两行（额度+天数）在真机
//   截图里挤得没法看，砍掉了额度一行；各段的额度、类型、到期时刻都在悬停里。
//   已过期 / 永不过期没有天数，块留空；
// - **右侧数字 = 剩余总额**（悬停给「剩余/总量」）：总额与构成本是同一件事的两面
//   （构成就是这些剩余积分的来源分布），分成两列时用户得来回对照才知道一共多少。
// - 悬停显示：类型 · 剩余/总量 · 到期时刻 · 还剩几天。
//
// 数据来自 `mergeCreditSegments`（同一礼包已合并、按到期先后排序），
// 所以方块从左到右依次是「最快到期 → 最慢到期」。

import { useTranslation } from "react-i18next";

import {
  formatQuotaNumber,
  formatQuotaPlain,
  segmentDayLabels,
  segmentDaysLeft,
  segmentHue,
  summarizeCreditSegments,
  type CreditSegment,
} from "./quota";

/** 一个方块的最小宽度：再窄就放不下最长的天数（`99+`，9px 字体约 16px）。
    占比极小的段会被**撑到**这个宽度 —— 严格等比会让它们的信息没法显示，
    可读性优先（悬停里仍有精确值）。 */
const BLOCK_MIN_WIDTH = 16;

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

  const labels = segmentDayLabels(segments, now);
  const summary = summarizeCreditSegments(segments);
  // 段特别多时收一档字号与块宽：宁可字小一点，也不能让尾部的段被挤出可视区（丢段更糟）
  const dense = segments.length > 10;

  return (
    <div className="flex items-center gap-2 w-full">
      <div
        className={`flex flex-1 min-w-0 h-4 gap-px rounded-sm overflow-hidden bg-white/5 ${
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
              className="flex items-center justify-center h-full overflow-hidden leading-none font-semibold text-white"
              style={{
                flexGrow: Math.max(0, segment.remaining),
                flexBasis: 0,
                minWidth: dense ? 12 : BLOCK_MIN_WIDTH,
                backgroundColor:
                  hue === null ? "rgba(148,163,184,0.45)" : `hsl(${hue} 68% 45%)`,
              }}
              title={describe(segment)}
            >
              {/* 只显示剩余天数；额度/类型/到期都在悬停里 */}
              <span className={dense ? "text-[8px]" : "text-[9px]"}>{labels[index]}</span>
            </span>
          );
        })}
      </div>
      {/* 剩余总额：与构成条同源（各段 remaining 之和），悬停给出「剩余/总量」 */}
      <span
        className="flex-shrink-0 tabular-nums text-micro text-slate-400"
        title={`${t("buddy.quotaTotal")}: ${formatQuotaPlain(summary.remain)}/${formatQuotaPlain(summary.total)}`}
      >
        {formatQuotaPlain(summary.remain)}
      </span>
    </div>
  );
}
