import type { QuotaItem } from "./BuddyPanel.model";
import { useTranslation } from "react-i18next";
import { formatQuotaNumber } from "./quota";
import { RefreshCw, Loader2, Gauge, X } from "lucide-react";
import { formatTime, type BuddyAccount } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface BuddyAccountUsageDialogProps {
  displayName: (acc: BuddyAccount) => string;
  usageAccount: BuddyAccount;
  planBadge: string;
  setUsageAccountId: Dispatch<SetStateAction<string | null>>;
  quotaItems: QuotaItem[];
  dosageText: string | null;
  refreshAccount: (id: string) => Promise<void>;
  busy: boolean;
}

/** Pure BuddyAccountUsageDialog view; its owning panel retains side effects and state. */
export function BuddyAccountUsageDialog({ displayName, usageAccount, planBadge, setUsageAccountId, quotaItems, dosageText, refreshAccount, busy }: BuddyAccountUsageDialogProps) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
      <div className="w-[460px] max-w-[95vw] max-h-[85vh] overflow-y-auto rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5">
        <div className="flex items-center gap-2.5 mb-4">
          <div className="w-9 h-9 rounded-card bg-[var(--module-accent)]/15 border border-[var(--module-accent)]/30 flex items-center justify-center">
            <Gauge className="w-4 h-4 text-[var(--module-accent)]" />
          </div>
          <div className="flex-1 min-w-0">
            <h3 className="text-sm font-bold text-white truncate">{displayName(usageAccount)}</h3>
            <p className="text-tiny text-slate-500 truncate">{usageAccount.email}</p>
          </div>
          {planBadge !== "UNKNOWN" && (
            <span className="inline-flex items-center text-micro px-1.5 py-0.5 rounded bg-slate-500/15 text-slate-300 border border-slate-500/25 flex-shrink-0">
              {t(`buddy.plan.${planBadge.toLowerCase()}`, { defaultValue: planBadge })}
            </span>
          )}
          <button
            onClick={() => setUsageAccountId(null)}
            className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 cursor-pointer flex-shrink-0"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {quotaItems.length === 0 ? (
          <div className="text-caption text-slate-600 mb-3">
            {usageAccount.quotaQueryLastError
              ? `${t("buddy.usageError")}: ${usageAccount.quotaQueryLastError}`
              : t("buddy.noUsage")}
          </div>
        ) : (
          <div className="space-y-2.5 mb-3">
            {quotaItems.map((item, idx) => {
              const pct =
                item.total > 0 && !item.unlimited
                  ? Math.max(0, Math.min(100, (item.used / item.total) * 100))
                  : null;
              return (
                <div key={idx}>
                  <div className="flex items-center justify-between text-tiny mb-1">
                    <span className="text-slate-300 truncate">
                      {item.packageName || t("buddy.quota.package")}
                    </span>
                    {item.unlimited ? (
                      <span className="text-emerald-400">∞ {t("buddy.quota.unlimited")}</span>
                    ) : (
                      <span className="text-slate-400 tabular-nums">
                        {formatQuotaNumber(item.used)} / {formatQuotaNumber(item.total)}
                        {pct !== null ? ` (${pct.toFixed(1)}%)` : ""}
                      </span>
                    )}
                  </div>
                  {!item.unlimited && (
                    <div className="h-1.5 bg-white/10 rounded-full overflow-hidden">
                      <div
                        className={`h-full rounded-full ${
                          pct === null
                            ? "bg-slate-500/50"
                            : pct >= 90
                              ? "bg-rose-500"
                              : pct >= 60
                                ? "bg-amber-500"
                                : "bg-emerald-500"
                        }`}
                        style={{ width: `${pct ?? 0}%` }}
                      />
                    </div>
                  )}
                  {item.cycleEndTime && (
                    <div className="text-micro text-slate-600 mt-0.5">
                      {t("buddy.quota.cycleEnd")}: {item.cycleEndTime}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}

        {dosageText && <div className="text-tiny text-slate-400 mb-1">{dosageText}</div>}
        {usageAccount.paymentType && (
          <div className="text-micro text-slate-600 mb-1">
            {t("buddy.paymentType")}: {usageAccount.paymentType}
          </div>
        )}
        {usageAccount.usageUpdatedAt ? (
          <div className="text-micro text-slate-600 mb-3">
            {t("buddy.usageUpdatedAt")}: {formatTime(usageAccount.usageUpdatedAt)}
          </div>
        ) : null}

        <div className="flex justify-end gap-2 pt-1">
          <button
            onClick={() => refreshAccount(usageAccount.id)}
            disabled={busy}
            className="px-3 py-1.5 rounded-ctl text-caption bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
            {t("buddy.refresh")}
          </button>
          <button
            onClick={() => setUsageAccountId(null)}
            className="px-4 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold cursor-pointer"
          >
            {t("buddy.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
