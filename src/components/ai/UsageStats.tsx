import React, { useState, useEffect, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Trash2,
  RefreshCw,
  ArrowUpRight,
  ArrowDownRight,
  Hash,
  Clock,
  Boxes,
  Cpu,
  Server,
  ChevronDown,
  ChevronUp,
  Activity,
} from "lucide-react";

interface UsageSummary {
  total_records: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_tokens: number;
  by_tool: { tool_id: string; request_count: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
  by_model: { model: string; provider: string; request_count: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
  by_provider: { provider: string; request_count: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
  recent: { date: string; request_count: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
}

type SortKey = "requests" | "input" | "output" | "total";

interface Row {
  key: string;
  label: string;
  sub?: string;
  requests: number;
  input: number;
  output: number;
  total: number;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function formatFull(n: number): string {
  return n.toLocaleString("en-US");
}

interface SectionStyle {
  iconClass: string;
  softClass: string;
  barClass: string;
}

const SECTION_STYLES: Record<string, SectionStyle> = {
  recent: { iconClass: "text-sky-400", softClass: "bg-sky-500/10", barClass: "bg-sky-500/60" },
  tool: { iconClass: "text-violet-400", softClass: "bg-violet-500/10", barClass: "bg-violet-500/60" },
  model: { iconClass: "text-amber-400", softClass: "bg-amber-500/10", barClass: "bg-amber-500/60" },
  provider: { iconClass: "text-emerald-400", softClass: "bg-emerald-500/10", barClass: "bg-emerald-500/60" },
};

function SortableTable({
  title,
  icon,
  rows,
  nameHeader,
  section,
  maxRows,
}: {
  title: string;
  icon: React.ReactNode;
  rows: Row[];
  nameHeader: string;
  section: keyof typeof SECTION_STYLES;
  maxRows?: number;
}) {
  const [sortKey, setSortKey] = useState<SortKey>("total");
  const [asc, setAsc] = useState(false);
  const style = SECTION_STYLES[section];

  const sorted = useMemo(() => {
    const list = maxRows ? rows.slice(0, maxRows) : rows;
    const copy = [...list];
    copy.sort((a, b) => {
      const diff = a[sortKey] - b[sortKey];
      return asc ? diff : -diff;
    });
    return copy;
  }, [rows, sortKey, asc, maxRows]);

  const maxTotal = Math.max(...rows.map(r => r.total), 1);

  const setSort = (k: SortKey) => {
    if (sortKey === k) setAsc(!asc);
    else { setSortKey(k); setAsc(false); }
  };

  const Th = ({ k, children, align }: { k: SortKey; children: React.ReactNode; align?: string }) => (
    <th
      onClick={() => setSort(k)}
      className={`px-2 py-1.5 text-[9px] font-semibold text-slate-500 cursor-pointer select-none hover:text-slate-300 whitespace-nowrap ${align || "text-right"}`}
      title="点击排序"
    >
      <span className="inline-flex items-center gap-0.5">
        {children}
        {sortKey === k && (asc ? <ChevronUp className="w-2.5 h-2.5" /> : <ChevronDown className="w-2.5 h-2.5" />)}
      </span>
    </th>
  );

  if (rows.length === 0) {
    return (
      <div className="rounded-xl bg-slate-900/30 border border-white/5">
        <div className="flex items-center gap-1.5 px-3 py-2 text-[10px] font-bold text-slate-300">
          <span className={`p-1 rounded-md ${style.softClass} ${style.iconClass}`}>{icon}</span>
          {title}
          <span className="text-[9px] font-normal text-slate-600 ml-1">（暂无数据）</span>
        </div>
      </div>
    );
  }

  const rankBadge = (idx: number) => {
    if (idx >= 3) return <span className="text-[8px] text-slate-600 tabular-nums w-4 text-right">{idx + 1}</span>;
    const cls = idx === 0 ? "bg-amber-500/20 text-amber-300" : idx === 1 ? "bg-slate-400/20 text-slate-300" : "bg-orange-500/20 text-orange-300";
    return <span className={`text-[8px] font-bold tabular-nums w-4 text-right ${cls}`}>{idx + 1}</span>;
  };

  return (
    <div className="rounded-xl bg-slate-900/30 border border-white/5 overflow-hidden">
      <div className="flex items-center gap-1.5 px-3 py-2 text-[10px] font-bold text-slate-200 border-b border-white/[0.04] bg-white/[0.015]">
        <span className={`p-1 rounded-md ${style.softClass} ${style.iconClass}`}>{icon}</span>
        {title}
        <span className="text-[9px] font-normal text-slate-600 ml-1">（{rows.length}）</span>
      </div>
      <table className="w-full border-collapse">
        <thead>
          <tr className="text-slate-500">
            <th className="w-4" />
            <th className="px-2 py-1 text-[9px] font-semibold text-left">{nameHeader}</th>
            <Th k="requests">请求</Th>
            <Th k="input">输入</Th>
            <Th k="output">输出</Th>
            <Th k="total">总计</Th>
          </tr>
        </thead>
        <tbody>
          {sorted.map((r, idx) => (
            <tr key={r.key} className="border-t border-white/[0.03] hover:bg-white/[0.025] transition-colors">
              <td className="pl-2 py-1.5">{rankBadge(idx)}</td>
              <td className="px-2 py-1.5 max-w-[180px]">
                <div className="text-[10px] text-slate-200 font-mono truncate" title={r.label}>{r.label}</div>
                {r.sub && <div className="text-[8px] text-slate-600 truncate">{r.sub}</div>}
              </td>
              <td className="px-2 py-1.5 text-[10px] text-violet-300 text-right font-semibold tabular-nums">{r.requests}</td>
              <td className="px-2 py-1.5 text-[10px] text-blue-300 text-right tabular-nums">{formatTokens(r.input)}</td>
              <td className="px-2 py-1.5 text-[10px] text-emerald-300 text-right tabular-nums">{formatTokens(r.output)}</td>
              <td className="px-2 py-1.5 text-right">
                <div className="flex items-center justify-end gap-1.5">
                  <div className="w-12 h-1.5 bg-white/5 rounded-full overflow-hidden hidden sm:block">
                    <div className={`h-full ${style.barClass} rounded-full`} style={{ width: `${Math.min(100, (r.total / maxTotal) * 100)}%` }} />
                  </div>
                  <span className="text-[10px] text-slate-400 tabular-nums w-10 text-right">{formatTokens(r.total)}</span>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export default function UsageStats() {
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<UsageSummary>("get_usage_summary");
      setSummary(data);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  const handleClear = async () => {
    if (!confirm("确定要清空所有用量统计数据吗？")) return;
    await invoke("clear_usage");
    await load();
  };

  const toolRows: Row[] = (summary?.by_tool || []).map(t => ({
    key: t.tool_id,
    label: t.tool_id,
    requests: t.request_count,
    input: t.input_tokens,
    output: t.output_tokens,
    total: t.total_tokens,
  }));
  const modelRows: Row[] = (summary?.by_model || []).map(m => ({
    key: m.model + "|" + m.provider,
    label: m.model,
    sub: m.provider,
    requests: m.request_count,
    input: m.input_tokens,
    output: m.output_tokens,
    total: m.total_tokens,
  }));
  const providerRows: Row[] = (summary?.by_provider || []).map(p => ({
    key: p.provider || "（未知）",
    label: p.provider || "（未知）",
    requests: p.request_count,
    input: p.input_tokens,
    output: p.output_tokens,
    total: p.total_tokens,
  }));
  const recentRows: Row[] = (summary?.recent || []).slice(-14).map(d => ({
    key: d.date,
    label: d.date,
    requests: d.request_count,
    input: d.input_tokens,
    output: d.output_tokens,
    total: d.total_tokens,
  }));

  const hasData = !!summary && summary.total_records > 0;
  const totalRecords = summary?.total_records ?? 0;
  const totalInput = summary?.total_input_tokens ?? 0;
  const totalOutput = summary?.total_output_tokens ?? 0;
  const totalTokens = summary?.total_tokens ?? 0;
  const avgPerReq = totalRecords > 0 ? Math.round(totalTokens / totalRecords) : 0;
  const inputPct = totalTokens > 0 ? Math.round((totalInput / totalTokens) * 100) : 0;
  const outputPct = totalTokens > 0 ? 100 - inputPct : 0;

  return (
    <div className="h-full flex flex-col min-h-0">
      {/* 顶部操作条 */}
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-white/5 flex-shrink-0">
        <div className="flex items-center gap-2">
          <Activity className="w-4 h-4 text-violet-400" />
          <h3 className="text-sm font-bold text-white">用量统计</h3>
        </div>
        <div className="flex gap-1.5">
          <button onClick={load} disabled={loading}
            className="px-2.5 py-1 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex items-center gap-1 disabled:opacity-50">
            <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} /> 刷新
          </button>
          <button onClick={handleClear} disabled={!hasData}
            className="px-2.5 py-1 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-300 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all flex items-center gap-1 disabled:opacity-30 disabled:cursor-not-allowed">
            <Trash2 className="w-3 h-3" /> 清空
          </button>
        </div>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto p-3 space-y-2.5">
        {loading ? (
          <div className="h-full flex items-center justify-center text-slate-500">
            <RefreshCw className="w-5 h-5 animate-spin mr-2" />
            <span className="text-xs">加载中...</span>
          </div>
        ) : !hasData ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500">
            <div className="p-4 rounded-2xl bg-slate-900/40 border border-white/5 mb-3">
              <Hash className="w-8 h-8 text-slate-700" />
            </div>
            <span className="text-sm font-bold text-slate-400">暂无用量数据</span>
            <span className="text-[10px] text-slate-600 mt-1">通过代理启动 AI 工具后，用量数据将自动记录</span>
            <button onClick={load}
              className="mt-3 px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex items-center gap-1">
              <RefreshCw className="w-3 h-3" /> 刷新
            </button>
          </div>
        ) : (
          <>
            {/* 主卡：总 Token 消耗 + 输入/输出占比 */}
            <div className="relative rounded-2xl bg-gradient-to-br from-violet-500/[0.10] via-slate-900/30 to-amber-500/[0.06] border border-white/5 p-4 overflow-hidden">
              <div className="absolute -right-6 -top-6 w-24 h-24 rounded-full bg-violet-500/10 blur-2xl" />
              <div className="relative flex items-start justify-between">
                <div>
                  <div className="text-[10px] text-slate-400 flex items-center gap-1">
                    <Activity className="w-3 h-3 text-violet-400" /> 总 Token 消耗
                  </div>
                  <div className="text-3xl font-bold text-white tabular-nums mt-1 leading-none">
                    {formatTokens(totalTokens)}
                  </div>
                  <div className="text-[9px] text-slate-500 mt-1.5 tabular-nums">
                    {formatFull(totalTokens)} tokens · {totalRecords} 次请求 · 平均 {formatTokens(avgPerReq)}/次
                  </div>
                </div>
                <div className="flex gap-1.5">
                  <span className="px-2 py-1 rounded-lg bg-blue-500/10 border border-blue-500/20 text-blue-300 text-[10px] font-semibold flex items-center gap-1 tabular-nums">
                    <ArrowDownRight className="w-3 h-3" />{formatTokens(totalInput)}
                  </span>
                  <span className="px-2 py-1 rounded-lg bg-emerald-500/10 border border-emerald-500/20 text-emerald-300 text-[10px] font-semibold flex items-center gap-1 tabular-nums">
                    <ArrowUpRight className="w-3 h-3" />{formatTokens(totalOutput)}
                  </span>
                </div>
              </div>
              {/* 输入/输出占比条 */}
              <div className="relative mt-3.5 flex h-2.5 rounded-full overflow-hidden bg-white/5">
                <div className="bg-blue-500/70" style={{ width: `${inputPct}%` }} />
                <div className="bg-emerald-500/70" style={{ width: `${outputPct}%` }} />
              </div>
              <div className="relative flex justify-between text-[8px] mt-1.5 tabular-nums">
                <span className="text-blue-300/80">输入 {inputPct}%</span>
                <span className="text-emerald-300/80">输出 {outputPct}%</span>
              </div>
            </div>

            {/* 指标卡 */}
            <div className="grid grid-cols-3 gap-2">
              <div className="rounded-xl bg-slate-900/30 border border-white/5 p-3">
                <div className="text-[9px] text-slate-500 flex items-center gap-1"><Hash className="w-3 h-3 text-violet-400" />请求总数</div>
                <div className="text-lg font-bold text-slate-100 tabular-nums mt-1">{formatTokens(totalRecords)}</div>
              </div>
              <div className="rounded-xl bg-slate-900/30 border border-white/5 p-3">
                <div className="text-[9px] text-slate-500 flex items-center gap-1"><ArrowDownRight className="w-3 h-3 text-blue-400" />输入 Token</div>
                <div className="text-lg font-bold text-slate-100 tabular-nums mt-1">{formatTokens(totalInput)}</div>
              </div>
              <div className="rounded-xl bg-slate-900/30 border border-white/5 p-3">
                <div className="text-[9px] text-slate-500 flex items-center gap-1"><ArrowUpRight className="w-3 h-3 text-emerald-400" />输出 Token</div>
                <div className="text-lg font-bold text-slate-100 tabular-nums mt-1">{formatTokens(totalOutput)}</div>
              </div>
            </div>

            <SortableTable title="最近 14 天" icon={<Clock className="w-3 h-3" />} section="recent" rows={recentRows} nameHeader="日期" />
            <SortableTable title="工具" icon={<Boxes className="w-3 h-3" />} section="tool" rows={toolRows} nameHeader="工具" />
            <SortableTable title="模型" icon={<Cpu className="w-3 h-3" />} section="model" rows={modelRows} nameHeader="模型" />
            <SortableTable title="供应商" icon={<Server className="w-3 h-3" />} section="provider" rows={providerRows} nameHeader="供应商" />
          </>
        )}
      </div>
    </div>
  );
}
