import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LogSummary, MinuteBucket } from "./twoapiLogModel";

type Metric = "tokens" | "credit" | "requests";

interface TwoApiUsageChartProps {
  summary: LogSummary;
  buckets: MinuteBucket[];
}

const W = 1000;
const H = 120;

const fmt = (n: number, digits = 0) =>
  n.toLocaleString(undefined, { maximumFractionDigits: digits });

/** 参考 NetworkOverview.tsx 的 TrafficChart：零依赖 SVG 折线，非等宽容器用 preserveAspectRatio="none"。 */
function Sparkline({ buckets, metric }: { buckets: MinuteBucket[]; metric: Metric }) {
  const { t } = useTranslation();
  const value = (b: MinuteBucket) =>
    metric === "tokens" ? b.tokens : metric === "credit" ? b.credit : b.requests;
  const max = Math.max(1, ...buckets.map(value));
  const points = buckets
    .map((b, i) => {
      const x = (i / Math.max(1, buckets.length - 1)) * W;
      const y = H - (value(b) / max) * (H - 8) - 4;
      return `${x},${y}`;
    })
    .join(" ");

  return (
    <div className="relative h-28 w-full overflow-hidden rounded-ctl border border-white/5 bg-black/20">
      {buckets.length < 2 && (
        <div className="absolute inset-0 flex items-center justify-center text-micro text-slate-600">
          {t("buddy.twoapi.chartNoData")}
        </div>
      )}
      <svg className="absolute inset-0 h-full w-full" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        {[0.25, 0.5, 0.75].map((p) => (
          <line
            key={p}
            x1="0" x2={W} y1={H * p} y2={H * p}
            stroke="rgba(255,255,255,0.05)" strokeWidth="1" vectorEffect="non-scaling-stroke"
          />
        ))}
        {buckets.length > 1 && (
          <>
            <polygon
              points={`0,${H} ${points} ${W},${H}`}
              fill="rgba(56,189,248,0.10)"
            />
            <polyline
              points={points} fill="none" stroke="#38bdf8" strokeWidth="2"
              strokeLinejoin="round" strokeLinecap="round" vectorEffect="non-scaling-stroke"
            />
          </>
        )}
      </svg>
      <span className="absolute right-2 top-1 font-mono text-micro text-slate-500">
        {metric === "credit" ? fmt(max, 2) : fmt(max)}
      </span>
      <span className="absolute bottom-1 left-2 font-mono text-micro text-slate-600">
        {buckets[0]?.label ?? ""} — {buckets[buckets.length - 1]?.label ?? ""}
      </span>
    </div>
  );
}

export function TwoApiUsageChart({ summary, buckets }: TwoApiUsageChartProps) {
  const { t } = useTranslation();
  const [metric, setMetric] = useState<Metric>("tokens");

  const cards = useMemo<Array<[string, string]>>(() => [
    [t("buddy.twoapi.statRequests"), fmt(summary.total)],
    [t("buddy.twoapi.statSuccessRate"), `${Math.round(summary.successRate * 100)}%`],
    [t("buddy.twoapi.statTotalCredit"), fmt(summary.totalCredit, 2)],
    [t("buddy.twoapi.statTotalTokens"), fmt(summary.totalTokens)],
    [t("buddy.twoapi.statAvgDuration"), summary.avgDurationMs > 0 ? `${fmt(summary.avgDurationMs)}ms` : "-"],
  ], [summary, t]);

  const metrics: Array<[Metric, string]> = [
    ["tokens", t("buddy.twoapi.metricTokens")],
    ["credit", t("buddy.twoapi.metricCredit")],
    ["requests", t("buddy.twoapi.metricRequests")],
  ];

  return (
    <div className="flex flex-col gap-2 px-2 py-2">
      <div className="text-micro text-slate-600">{t("buddy.twoapi.sessionScopeHint")}</div>

      <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-3 lg:grid-cols-5">
        {cards.map(([label, value]) => (
          <div key={label} className="rounded-ctl border border-white/5 bg-black/20 px-2 py-1.5">
            <div className="truncate text-micro text-slate-500" title={label}>{label}</div>
            <div className="font-mono text-caption font-semibold text-slate-100 tabular-nums">{value}</div>
          </div>
        ))}
      </div>

      <div className="flex items-center gap-1">
        {metrics.map(([id, label]) => (
          <button
            key={id}
            onClick={() => setMetric(id)}
            className={`rounded px-1.5 py-0.5 text-micro transition ${
              metric === id
                ? "bg-[var(--module-accent)] text-white"
                : "bg-white/5 text-slate-400 hover:text-slate-100"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      <Sparkline buckets={buckets} metric={metric} />
    </div>
  );
}
