import { formatQuotaNumber } from "./quota";
// 额度余量进度条：宽 = 剩余占比，色 = 剩余多少（≥50 绿 / ≥20 黄 / <20 红）
export function QuotaBar({
  remain,
  total,
  className,
  title,
}: {
  remain: number;
  total: number;
  className?: string;
  title?: string;
}) {
  const ratio = total > 0 ? Math.max(0, Math.min(1, remain / total)) : 0;
  const pct = Math.round(ratio * 100);
  const color = ratio >= 0.5 ? "bg-emerald-500" : ratio >= 0.2 ? "bg-amber-500" : "bg-rose-500";
  return (
    <span className={`inline-flex items-center gap-1 ${className ?? ""}`} title={title}>
      <span className="h-1 flex-1 min-w-[20px] bg-white/10 rounded-full overflow-hidden">
        <span className={`block h-full rounded-full ${color}`} style={{ width: `${pct}%` }} />
      </span>
      <span className="text-slate-400 tabular-nums text-micro">{formatQuotaNumber(remain)}</span>
    </span>
  );
}
