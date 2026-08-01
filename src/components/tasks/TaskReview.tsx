import React, { useState, useEffect, useCallback, useMemo } from "react";
import { RefreshCw, TrendingUp, CheckCircle2, Clock, AlertTriangle } from "lucide-react";
import { DayStat, tasksApi, todayStr, addDays, formatMinutes } from "./types";

const RANGES = [
  { label: "近 7 天", days: 7 },
  { label: "近 14 天", days: 14 },
  { label: "近 30 天", days: 30 },
];

export default function TaskReview() {
  const [days, setDays] = useState(7);
  const [stats, setStats] = useState<DayStat[]>([]);
  const [loading, setLoading] = useState(true);

  const start = useMemo(() => addDays(todayStr(), -(days - 1)), [days]);
  const end = todayStr();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setStats(await tasksApi.dayStats(start, end));
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, [start, end]);

  useEffect(() => {
    load();
  }, [load]);

  /** 补齐区间内没有任务的日期，保证图表连续 */
  const series = useMemo(() => {
    const map = new Map(stats.map((s) => [s.date, s]));
    return Array.from({ length: days }, (_, i) => {
      const d = addDays(start, i);
      return (
        map.get(d) ?? {
          date: d,
          total: 0,
          completed: 0,
          inProgress: 0,
          pending: 0,
          minutesSpent: 0,
        }
      );
    });
  }, [stats, days, start]);

  const totals = useMemo(() => {
    const total = series.reduce((a, s) => a + s.total, 0);
    const completed = series.reduce((a, s) => a + s.completed, 0);
    const pending = series.reduce((a, s) => a + s.pending, 0);
    const minutes = series.reduce((a, s) => a + s.minutesSpent, 0);
    return {
      total,
      completed,
      pending,
      minutes,
      rate: total > 0 ? Math.round((completed / total) * 100) : 0,
    };
  }, [series]);

  const maxTotal = Math.max(1, ...series.map((s) => s.total));

  return (
    <div className="h-full overflow-y-auto p-6 space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-white">复盘统计</h3>
          <p className="text-[10px] text-slate-500 mt-0.5">
            {start} ~ {end}
          </p>
        </div>
        <div className="flex gap-2">
          <div className="flex items-center gap-0.5 bg-white/5 border border-white/10 rounded-lg p-0.5">
            {RANGES.map((r) => (
              <button
                key={r.days}
                onClick={() => setDays(r.days)}
                className={`px-2.5 py-1 rounded-md text-[10px] font-semibold cursor-pointer transition-all ${
                  days === r.days
                    ? "bg-amber-500 text-slate-900"
                    : "text-slate-400 hover:text-slate-200"
                }`}
              >
                {r.label}
              </button>
            ))}
          </div>
          <button
            onClick={load}
            className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
          >
            <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} /> 刷新
          </button>
        </div>
      </div>

      {/* 汇总指标 */}
      <div className="grid grid-cols-4 gap-2">
        <MetricCard
          icon={<TrendingUp className="w-3.5 h-3.5" />}
          label="任务总数"
          value={totals.total}
        />
        <MetricCard
          icon={<CheckCircle2 className="w-3.5 h-3.5" />}
          label="已完成"
          value={totals.completed}
          accent="text-emerald-400"
        />
        <MetricCard
          icon={<AlertTriangle className="w-3.5 h-3.5" />}
          label="未完成"
          value={totals.pending}
          accent="text-red-400"
        />
        <MetricCard
          icon={<Clock className="w-3.5 h-3.5" />}
          label="累计投入"
          value={formatMinutes(totals.minutes)}
          accent="text-amber-300"
          small
        />
      </div>

      {/* 完成率环形提示 */}
      <div className="p-4 rounded-xl bg-slate-900/40 border border-amber-500/20">
        <div className="flex items-center justify-between mb-2">
          <span className="text-[11px] font-semibold text-slate-300">整体完成率</span>
          <span className="text-lg font-bold text-amber-300">{totals.rate}%</span>
        </div>
        <div className="h-2 rounded-full bg-white/5 overflow-hidden">
          <div
            className="h-full rounded-full bg-gradient-to-r from-amber-500 to-emerald-500 transition-all"
            style={{ width: `${totals.rate}%` }}
          />
        </div>
      </div>

      {/* 每日柱状图 */}
      <div className="p-4 rounded-xl bg-slate-900/40 border border-white/10">
        <h4 className="text-[11px] font-semibold text-slate-300 mb-3">每日任务分布</h4>
        {loading ? (
          <p className="text-[10px] text-slate-500">加载中…</p>
        ) : (
          <div className="flex items-end gap-1 h-40">
            {series.map((s) => {
              const h = (s.total / maxTotal) * 100;
              const doneH = s.total > 0 ? (s.completed / s.total) * 100 : 0;
              return (
                <div key={s.date} className="flex-1 flex flex-col items-center gap-1 group">
                  <span className="text-[8px] text-slate-500 opacity-0 group-hover:opacity-100 transition-opacity">
                    {s.completed}/{s.total}
                  </span>
                  <div className="w-full flex-1 flex items-end">
                    <div
                      className="w-full rounded-t bg-white/8 relative overflow-hidden transition-all"
                      style={{ height: `${Math.max(h, 2)}%` }}
                      title={`${s.date}：完成 ${s.completed} / 共 ${s.total}，投入 ${formatMinutes(s.minutesSpent)}`}
                    >
                      <div
                        className="absolute bottom-0 left-0 w-full bg-amber-400 transition-all"
                        style={{ height: `${doneH}%` }}
                      />
                    </div>
                  </div>
                  <span className="text-[8px] text-slate-600">
                    {Number(s.date.split("-")[2])}
                  </span>
                </div>
              );
            })}
          </div>
        )}
        <div className="flex items-center gap-3 mt-3 text-[9px] text-slate-500">
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-sm bg-amber-400" /> 已完成
          </span>
          <span className="flex items-center gap-1">
            <span className="w-2 h-2 rounded-sm bg-white/8" /> 未完成
          </span>
        </div>
      </div>

      {/* 每日明细 */}
      <div className="rounded-xl bg-slate-900/40 border border-white/10 overflow-hidden">
        <table className="w-full text-[10px]">
          <thead>
            <tr className="border-b border-white/5 text-slate-500">
              <th className="text-left px-3 py-2 font-semibold">日期</th>
              <th className="text-right px-3 py-2 font-semibold">总数</th>
              <th className="text-right px-3 py-2 font-semibold">完成</th>
              <th className="text-right px-3 py-2 font-semibold">进行中</th>
              <th className="text-right px-3 py-2 font-semibold">未开始</th>
              <th className="text-right px-3 py-2 font-semibold">投入</th>
              <th className="text-right px-3 py-2 font-semibold">完成率</th>
            </tr>
          </thead>
          <tbody>
            {series
              .filter((s) => s.total > 0 || s.minutesSpent > 0)
              .reverse()
              .map((s) => (
                <tr key={s.date} className="border-b border-white/5 last:border-0">
                  <td className="px-3 py-1.5 text-slate-300 font-mono">{s.date}</td>
                  <td className="px-3 py-1.5 text-right text-slate-400">{s.total}</td>
                  <td className="px-3 py-1.5 text-right text-emerald-400">{s.completed}</td>
                  <td className="px-3 py-1.5 text-right text-amber-400">{s.inProgress}</td>
                  <td className="px-3 py-1.5 text-right text-slate-500">{s.pending}</td>
                  <td className="px-3 py-1.5 text-right text-slate-400">
                    {formatMinutes(s.minutesSpent)}
                  </td>
                  <td className="px-3 py-1.5 text-right text-amber-300 font-semibold">
                    {s.total > 0 ? Math.round((s.completed / s.total) * 100) : 0}%
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
        {series.every((s) => s.total === 0) && (
          <p className="text-[10px] text-slate-600 text-center py-6">该区间暂无数据</p>
        )}
      </div>
    </div>
  );
}

function MetricCard({
  icon,
  label,
  value,
  accent,
  small,
}: {
  icon: React.ReactNode;
  label: string;
  value: string | number;
  accent?: string;
  small?: boolean;
}) {
  return (
    <div className="p-3 rounded-xl bg-slate-900/40 border border-white/10">
      <div className="flex items-center gap-1.5 text-slate-500">
        {icon}
        <span className="text-[9px]">{label}</span>
      </div>
      <p className={`${small ? "text-sm" : "text-xl"} font-bold ${accent ?? "text-slate-100"} mt-1`}>
        {value}
      </p>
    </div>
  );
}
