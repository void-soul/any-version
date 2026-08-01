// 流量统计页 —— 1:1 复刻 clash-party src/renderer/src/pages/traffic.tsx
// （1h/24h/7d/30d 时间范围 / 4 维度视图 / 汇总卡片 / 排行榜 / 趋势图 / 下钻明细 / 清空）
import React, { useCallback, useEffect, useRef, useState } from "react";
import { Trash2, ChevronDown, ChevronRight } from "lucide-react";
import {
  trafficDb, getTrafficOverview, getSubStatsByHost, getDevicesByHost, getProxyStatsByHost,
  AggregatedData, DataUsageType,
} from "./trafficDb";
import { cardCls, btnSec, calcTraffic } from "./ui";

type TimeRange = "1h" | "24h" | "7d" | "30d";
const TIME_RANGES: TimeRange[] = ["1h", "24h", "7d", "30d"];
const TIME_LABEL: Record<TimeRange, string> = { "1h": "1 小时", "24h": "24 小时", "7d": "7 天", "30d": "30 天" };
const VIEW_LABEL: Record<DataUsageType, string> = { sourceIP: "来源 IP", host: "域名", outbound: "出站代理", process: "进程" };
const AUTO_REFRESH_MS = 5000;

function getTimeRange(range: TimeRange) {
  const end = Date.now();
  const ms: Record<TimeRange, number> = {
    "1h": 3600e3, "24h": 24 * 3600e3, "7d": 7 * 24 * 3600e3, "30d": 30 * 24 * 3600e3,
  };
  const bucket: Record<TimeRange, number> = {
    "1h": 5 * 60e3, "24h": 3600e3, "7d": 6 * 3600e3, "30d": 24 * 3600e3,
  };
  return { start: end - ms[range], end, bucketSizeMs: bucket[range] };
}

function fmtBucketTime(ts: number, bucketSizeMs: number): string {
  const d = new Date(ts);
  if (bucketSizeMs < 3600e3) return d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  if (bucketSizeMs < 24 * 3600e3) return d.toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit" });
  return d.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}

export default function TrafficPanel() {
  const [activeView, setActiveView] = useState<DataUsageType>("host");
  const [timeRange, setTimeRange] = useState<TimeRange>("24h");
  const [rankings, setRankings] = useState<AggregatedData[]>([]);
  const [trend, setTrend] = useState<{ timestamp: number; upload: number; download: number }[]>([]);
  const [selectedRow, setSelectedRow] = useState<string | null>(null);
  const [subStats, setSubStats] = useState<AggregatedData[]>([]);
  const [proxyStatsMap, setProxyStatsMap] = useState<Record<string, AggregatedData[]>>({});
  const [selectedSubRow, setSelectedSubRow] = useState<string | null>(null);
  const [totalStats, setTotalStats] = useState({ upload: 0, download: 0, total: 0, count: 0 });
  const [bucketSizeMs, setBucketSizeMs] = useState(3600e3);
  const genRef = useRef(0);

  const load = useCallback(async (resetSelection = true) => {
    const gen = genRef.current;
    const { start, end, bucketSizeMs: bms } = getTimeRange(timeRange);
    const { rankings: agg, trend: tr } = await getTrafficOverview(activeView, start, end, bms);
    if (gen !== genRef.current) return;
    setBucketSizeMs(bms);
    setRankings(agg);
    setTrend(tr);
    setTotalStats(agg.reduce(
      (acc, r) => ({ upload: acc.upload + r.upload, download: acc.download + r.download, total: acc.total + r.total, count: acc.count + r.count }),
      { upload: 0, download: 0, total: 0, count: 0 }
    ));
    if (resetSelection) {
      setSelectedRow(null); setSubStats([]); setProxyStatsMap({}); setSelectedSubRow(null);
    }
  }, [activeView, timeRange]);

  useEffect(() => {
    genRef.current++;
    let cancelled = false;
    let timer: any = null;
    const refresh = async (reset: boolean) => {
      await load(reset);
      if (cancelled) return;
      timer = setTimeout(() => refresh(false), AUTO_REFRESH_MS);
    };
    refresh(true);
    return () => { cancelled = true; if (timer) clearTimeout(timer); };
  }, [load]);

  // 复刻 handleSelectRow：点击排行下钻
  const handleSelectRow = async (label: string) => {
    if (selectedRow === label) {
      setSelectedRow(null); setSubStats([]); setProxyStatsMap({}); setSelectedSubRow(null);
      return;
    }
    setSelectedRow(label);
    setSelectedSubRow(null);
    setProxyStatsMap({});
    const { start, end } = getTimeRange(timeRange);
    const subs = activeView === "host"
      ? await getDevicesByHost(label, start, end)
      : await getSubStatsByHost(activeView as Exclude<DataUsageType, "host">, label, start, end);
    setSubStats(subs);
  };

  // 复刻 handleSubRowClick：二级下钻（出站代理分布）
  const handleSubRowClick = async (parentLabel: string, subLabel: string) => {
    const key = `${parentLabel}:${subLabel}`;
    if (selectedSubRow === key) { setSelectedSubRow(null); return; }
    setSelectedSubRow(key);
    if (proxyStatsMap[key]) return;
    const { start, end } = getTimeRange(timeRange);
    const proxies = await getProxyStatsByHost(activeView, parentLabel, subLabel, start, end);
    setProxyStatsMap((prev) => ({ ...prev, [key]: proxies }));
  };

  const maxTrend = Math.max(1, ...trend.map((t) => t.upload + t.download));
  const maxRank = Math.max(1, ...rankings.map((r) => r.total));

  return (
    <div className="space-y-3">
      {/* 时间范围 + 清空 */}
      <div className="flex items-center gap-2">
        <div className="flex rounded-lg bg-white/5 border border-white/10 overflow-hidden">
          {TIME_RANGES.map((r) => (
            <button key={r} onClick={() => setTimeRange(r)}
              className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
                timeRange === r ? "bg-emerald-600 text-white" : "text-slate-400 hover:text-slate-200"
              }`}>{TIME_LABEL[r]}</button>
          ))}
        </div>
        <div className="flex-1" />
        <button className={btnSec} title="清空统计" onClick={async () => { await trafficDb.clearAll(); load(); }}>
          <Trash2 className="w-3.5 h-3.5 text-rose-300" />
        </button>
      </div>

      {/* 汇总卡片 */}
      <div className="grid grid-cols-4 gap-2">
        {[
          ["会话数", String(totalStats.count)],
          ["上传", calcTraffic(totalStats.upload)],
          ["下载", calcTraffic(totalStats.download)],
          ["总计", calcTraffic(totalStats.total)],
        ].map(([label, value]) => (
          <div key={label} className={`${cardCls} flex flex-col items-center py-3`}>
            <span className="text-[10px] text-slate-500 uppercase tracking-wide">{label}</span>
            <span className="mt-0.5 text-sm font-bold text-white">{value}</span>
          </div>
        ))}
      </div>

      {/* 维度切换 */}
      <div className="flex rounded-lg bg-white/5 border border-white/10 overflow-hidden w-fit">
        {(Object.keys(VIEW_LABEL) as DataUsageType[]).map((v) => (
          <button key={v} onClick={() => setActiveView(v)}
            className={`px-3 py-1.5 text-[11px] font-semibold cursor-pointer transition-all ${
              activeView === v ? "bg-emerald-600 text-white" : "text-slate-400 hover:text-slate-200"
            }`}>{VIEW_LABEL[v]}</button>
        ))}
      </div>

      {/* 排行 + 趋势 */}
      <div className="grid grid-cols-4 gap-3">
        <div className={`${cardCls} col-span-1 h-56 p-3 overflow-y-auto`}>
          <div className="text-[11px] text-slate-400 font-semibold mb-2">{VIEW_LABEL[activeView]} 排行</div>
          {rankings.length === 0 && <div className="text-[11px] text-slate-500 text-center pt-8">暂无数据</div>}
          {rankings.slice(0, 50).map((r) => (
            <div key={r.label}
              className={`px-2 py-1.5 rounded-lg cursor-pointer mb-0.5 ${selectedRow === r.label ? "bg-emerald-500/15" : "hover:bg-white/5"}`}
              onClick={() => handleSelectRow(r.label)}>
              <div className="flex justify-between gap-2 text-[11px]">
                <span className="text-slate-200 truncate">{r.label}</span>
                <span className="text-slate-400 font-mono flex-shrink-0">{calcTraffic(r.total)}</span>
              </div>
              <div className="h-1 mt-1 rounded-full bg-white/5 overflow-hidden">
                <div className="h-full bg-emerald-500/60" style={{ width: `${(r.total / maxRank) * 100}%` }} />
              </div>
            </div>
          ))}
        </div>
        {/* 趋势图（纯 CSS 柱状，上传/下载堆叠） */}
        <div className={`${cardCls} col-span-3 h-56 p-3 flex flex-col`}>
          <div className="text-[11px] text-slate-400 font-semibold mb-2">流量趋势</div>
          <div className="flex-1 flex items-end gap-px min-h-0">
            {trend.map((t) => (
              <div key={t.timestamp} className="flex-1 flex flex-col justify-end h-full group relative">
                <div className="w-full bg-sky-500/70 rounded-t-sm" style={{ height: `${(t.upload / maxTrend) * 100}%` }} />
                <div className="w-full bg-emerald-500/70" style={{ height: `${(t.download / maxTrend) * 100}%` }} />
                <div className="hidden group-hover:block absolute bottom-full left-1/2 -translate-x-1/2 mb-1 px-2 py-1 rounded-lg bg-black/90 text-[10px] text-white whitespace-nowrap z-20 pointer-events-none">
                  {fmtBucketTime(t.timestamp, bucketSizeMs)}<br />
                  ↑ {calcTraffic(t.upload)} ↓ {calcTraffic(t.download)}
                </div>
              </div>
            ))}
          </div>
          <div className="flex justify-between text-[9px] text-slate-500 mt-1">
            <span>{trend.length ? fmtBucketTime(trend[0].timestamp, bucketSizeMs) : ""}</span>
            <span className="flex gap-3">
              <span className="inline-flex items-center gap-1"><i className="w-2 h-2 rounded-sm bg-sky-500/70 inline-block" />上传</span>
              <span className="inline-flex items-center gap-1"><i className="w-2 h-2 rounded-sm bg-emerald-500/70 inline-block" />下载</span>
            </span>
            <span>{trend.length ? fmtBucketTime(trend[trend.length - 1].timestamp, bucketSizeMs) : ""}</span>
          </div>
        </div>
      </div>

      {/* 下钻明细表 */}
      {selectedRow && (
        <div className={`${cardCls} p-3`}>
          <div className="text-[11px] text-slate-400 font-semibold mb-2">
            {selectedRow} 的{activeView === "host" ? "设备" : "域名"}明细
          </div>
          <table className="w-full text-[11px]">
            <thead>
              <tr className="text-slate-500 text-left">
                <th className="py-1 font-medium">{activeView === "host" ? "来源 IP" : "域名"}</th>
                <th className="py-1 font-medium text-right">上传</th>
                <th className="py-1 font-medium text-right">下载</th>
                <th className="py-1 font-medium text-right">总计</th>
                <th className="py-1 font-medium text-right">会话</th>
              </tr>
            </thead>
            <tbody>
              {subStats.map((s) => {
                const key = `${selectedRow}:${s.label}`;
                const expanded = selectedSubRow === key;
                return (
                  <React.Fragment key={s.label}>
                    <tr className="border-t border-white/5 hover:bg-white/[0.03] cursor-pointer text-slate-300"
                      onClick={() => handleSubRowClick(selectedRow, s.label)}>
                      <td className="py-1.5 flex items-center gap-1">
                        {expanded ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                        <span className="truncate">{s.label}</span>
                      </td>
                      <td className="py-1.5 text-right font-mono">{calcTraffic(s.upload)}</td>
                      <td className="py-1.5 text-right font-mono">{calcTraffic(s.download)}</td>
                      <td className="py-1.5 text-right font-mono">{calcTraffic(s.total)}</td>
                      <td className="py-1.5 text-right font-mono">{s.count}</td>
                    </tr>
                    {expanded && (proxyStatsMap[key] || []).map((p) => (
                      <tr key={p.label} className="text-slate-400 bg-white/[0.02]">
                        <td className="py-1 pl-8">{p.label}</td>
                        <td className="py-1 text-right font-mono">{calcTraffic(p.upload)}</td>
                        <td className="py-1 text-right font-mono">{calcTraffic(p.download)}</td>
                        <td className="py-1 text-right font-mono">{calcTraffic(p.total)}</td>
                        <td className="py-1 text-right font-mono">{p.count}</td>
                      </tr>
                    ))}
                  </React.Fragment>
                );
              })}
              {subStats.length === 0 && (
                <tr><td colSpan={5} className="py-3 text-center text-slate-500">暂无明细</td></tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
