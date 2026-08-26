import type { LoadTestReport } from "./types";

// ─── 压测报告视图 ───
export function StatCard({ label, value, accent }: { label: string; value: string; accent?: string }) {
  return (
    <div className="rounded-lg border border-white/10 bg-black/25 px-2.5 py-1.5 text-center">
      <div className="text-[9px] text-slate-500">{label}</div>
      <div className="text-sm font-semibold" style={{ color: accent ?? "#e2e8f0" }}>{value}</div>
    </div>
  );
}

export function TimelineChart({ report }: { report: LoadTestReport }) {
  const maxQps = Math.max(1, ...report.timeline.map((t) => t.qps));
  const maxFail = Math.max(1, ...report.timeline.map((t) => t.failed));
  const maxMs = Math.max(1, ...report.timeline.map((t) => t.avg_ms));
  const width = 560;
  const height = 130;
  const chartBottom = height - 22; // 图表区底部（留出图例）
  const chartTop = 4;
  const usableH = chartBottom - chartTop;
  const pad = 4;
  const n = Math.max(1, report.timeline.length);
  const stepX = (width - pad * 2) / n;
  const bw = Math.max(1, stepX - 1);
  const cx = (i: number) => pad + i * stepX + stepX / 2;

  // 平均延迟折线
  const latencyPoints = report.timeline
    .map((t, i) => `${cx(i).toFixed(1)},${(chartBottom - (t.avg_ms / maxMs) * usableH).toFixed(1)}`)
    .join(" ");
  // 成功率折线（0~1 映射到图表区上半段）
  const successPoints = report.timeline
    .map((t, i) => {
      const total = t.success + t.failed;
      const rate = total > 0 ? t.success / total : 1;
      return `${cx(i).toFixed(1)},${(chartBottom - rate * usableH).toFixed(1)}`;
    })
    .join(" ");

  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="w-full h-auto">
      {/* QPS 柱 + 失败柱 */}
      {report.timeline.map((t, i) => {
        const x = pad + i * stepX;
        const hQps = (t.qps / maxQps) * usableH;
        const hFail = (t.failed / maxFail) * usableH;
        return (
          <g key={i}>
            <rect x={x} y={chartBottom - hQps} width={bw} height={hQps} fill="rgba(6,182,212,0.35)" />
            {t.failed > 0 && <rect x={x} y={chartBottom - hFail} width={bw} height={hFail} fill="rgba(244,63,94,0.55)" />}
          </g>
        );
      })}
      {/* 成功率曲线 */}
      {n > 1 && <polyline points={successPoints} fill="none" stroke="#34d399" strokeWidth="1.4" strokeOpacity="0.85" strokeLinejoin="round" />}
      {/* 平均延迟曲线 */}
      {n > 1 && <polyline points={latencyPoints} fill="none" stroke="#fbbf24" strokeWidth="1.4" strokeOpacity="0.9" strokeLinejoin="round" />}
      <line x1={0} y1={chartBottom} x2={width} y2={chartBottom} stroke="rgba(255,255,255,0.15)" />
      {/* 图例 */}
      <g fontSize="9" fill="#64748b">
        <rect x={4} y={chartBottom + 8} width={8} height={6} fill="rgba(6,182,212,0.5)" rx={1} />
        <text x={15} y={chartBottom + 14}>QPS</text>
        <rect x={44} y={chartBottom + 8} width={8} height={6} fill="rgba(244,63,94,0.6)" rx={1} />
        <text x={55} y={chartBottom + 14}>失败</text>
        <line x1={86} y1={chartBottom + 11} x2={100} y2={chartBottom + 11} stroke="#34d399" strokeWidth="1.4" />
        <text x={104} y={chartBottom + 14}>成功率</text>
        <line x1={142} y1={chartBottom + 11} x2={156} y2={chartBottom + 11} stroke="#fbbf24" strokeWidth="1.4" />
        <text x={160} y={chartBottom + 14}>平均延迟</text>
        <text x={width - 4} y={chartBottom + 14} textAnchor="end">峰值 {maxQps.toFixed(0)} QPS · 峰值延迟 {maxMs.toFixed(0)}ms</text>
      </g>
    </svg>
  );
}

export function LoadReportView({ report }: { report: LoadTestReport }) {
  const errRate = (report.error_rate * 100).toFixed(2);
  return (
    <div className="space-y-3">
      <div className="grid grid-cols-4 gap-1.5">
        <StatCard label="总请求" value={String(report.total)} />
        <StatCard label="成功" value={String(report.success)} accent="#34d399" />
        <StatCard label="失败" value={String(report.failed)} accent={report.failed > 0 ? "#fb7185" : undefined} />
        <StatCard label="错误率" value={`${errRate}%`} accent={report.error_rate > 0.05 ? "#fb7185" : "#34d399"} />
        <StatCard label="QPS 平均" value={report.qps_avg.toFixed(1)} accent="#22d3ee" />
        <StatCard label="QPS 峰值" value={report.qps_max.toFixed(1)} />
        <StatCard label="平均延迟" value={`${report.latency_avg_ms.toFixed(1)}ms`} />
        <StatCard label="最大延迟" value={`${report.latency_max_ms.toFixed(1)}ms`} />
        <StatCard label="p50" value={`${report.latency_p50_ms.toFixed(1)}ms`} />
        <StatCard label="p90" value={`${report.latency_p90_ms.toFixed(1)}ms`} />
        <StatCard label="p95" value={`${report.latency_p95_ms.toFixed(1)}ms`} />
        <StatCard label="p99" value={`${report.latency_p99_ms.toFixed(1)}ms`} />
      </div>
      <div className="flex flex-wrap gap-1.5">
        {report.status_codes.map(([code, count]) => (
          <span key={code} className={`text-[10px] px-2 py-0.5 rounded-full border ${code >= 500 ? "text-rose-300 border-rose-500/30 bg-rose-500/10" : code >= 400 ? "text-amber-300 border-amber-500/30 bg-amber-500/10" : "text-emerald-300 border-emerald-500/30 bg-emerald-500/10"}`}>
            {code} × {count}
          </span>
        ))}
      </div>
      <TimelineChart report={report} />
    </div>
  );
}