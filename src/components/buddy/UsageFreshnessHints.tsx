import { useTranslation } from "react-i18next";

import CreditSegmentsBar from "./CreditSegmentsBar";
import { COL_CREDITS_MIN, accountCreditSegments, type BuddyAccount } from "./BuddyPanel.model";
import { mergeCreditSegments } from "./quota";
import { usageFreshness, usageFreshnessLabel } from "./usageFreshness";

/** 列表积分列：不占列宽的圆点，悬停给完整年龄/失败原因。 */
export function UsageFreshnessDot({ account, now }: { account: BuddyAccount; now: number }) {
  const { t, i18n } = useTranslation();
  const freshness = usageFreshness(account, now);
  const hint = usageFreshnessLabel(freshness, i18n.language);
  if (!hint) return null;
  const label = hint.age ? t(hint.key, { age: hint.age }) : t(hint.key);
  return (
    <span
      title={label}
      role="img"
      aria-label={label}
      className={`inline-flex h-1.5 w-1.5 flex-shrink-0 rounded-full cursor-help ${
        freshness.kind === "failed" ? "bg-rose-400" : "bg-amber-400"
      }`}
    />
  );
}

/** 顶部汇总条：有多少账号在显示上次查询的旧数字，避免整体假装实时。 */
export function UsageStaleHint({ accounts, now }: { accounts: BuddyAccount[]; now: number }) {
  const { t } = useTranslation();
  const count = accounts.filter((account) => {
    const kind = usageFreshness(account, now).kind;
    return kind === "cached" || kind === "failed";
  }).length;
  if (count === 0) return null;
  return (
    <span className="text-amber-400/80">
      · {t("buddy.quotaTotalStale", { count })}
    </span>
  );
}

/** 积分单元格：构成条 + 右侧总额 + 新鲜度圆点。单独成件，避免大面板继续膨胀。 */
export function BuddyCreditCell({
  account,
  quota,
  now,
  onOpen,
}: {
  account: BuddyAccount;
  quota: { unlimited: boolean };
  now: number;
  onOpen: () => void;
}) {
  return (
    <div
      style={{ minWidth: COL_CREDITS_MIN }}
      className="flex-1 min-w-0 flex items-center gap-1 px-2 border-l border-white/5"
    >
      <div className="flex-1 min-w-0">
        {quota.unlimited ? (
          <span className="text-emerald-400">∞</span>
        ) : (
          <CreditSegmentsBar
            segments={mergeCreditSegments(accountCreditSegments(account))}
            now={now}
            onClick={onOpen}
          />
        )}
      </div>
      <UsageFreshnessDot account={account} now={now} />
    </div>
  );
}
