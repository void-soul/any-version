import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  BarChart3,
  Trash2,
  RefreshCw,
  Zap,
  ArrowUpRight,
  ArrowDownRight,
  Hash,
} from "lucide-react";

interface UsageSummary {
  total_records: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_tokens: number;
  by_tool: { tool_id: string; request_count: number; total_tokens: number }[];
  by_model: { model: string; provider: string; request_count: number; total_tokens: number }[];
  daily: { date: string; request_count: number; total_tokens: number }[];
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
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

  if (loading) {
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-xs">加载中...</span></div>;
  }

  if (!summary || summary.total_records === 0) {
    return (
      <div className="h-full flex flex-col items-center justify-center text-slate-500">
        <BarChart3 className="w-8 h-8 text-slate-700 mb-2" />
        <span className="text-xs font-bold text-slate-400">暂无用量数据</span>
        <span className="text-[10px] text-slate-600 mt-1">通过代理启动 AI 工具后，用量数据将自动记录</span>
      </div>
    );
  }

  // 最近 14 天的柱状图数据
  const maxTokens = Math.max(...summary.daily.slice(0, 14).map(d => d.total_tokens), 1);

  return (
    <div className="h-full overflow-y-auto p-6 space-y-5">
      {/* 总览卡片 */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-white">用量统计</h3>
          <p className="text-[10px] text-slate-500 mt-0.5">共 {summary.total_records} 次 API 调用</p>
        </div>
        <div className="flex gap-2">
          <button onClick={load} className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1">
            <RefreshCw className="w-3 h-3" /> 刷新
          </button>
          <button onClick={handleClear} className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-red-400 cursor-pointer transition-all flex items-center gap-1">
            <Trash2 className="w-3 h-3" /> 清空
          </button>
        </div>
      </div>

      {/* 概览数字 */}
      <div className="grid grid-cols-3 gap-3">
        {[
          { label: "输入 Token", value: formatTokens(summary.total_input_tokens), icon: <ArrowDownRight className="w-3.5 h-3.5 text-blue-400" />, color: "text-blue-400" },
          { label: "输出 Token", value: formatTokens(summary.total_output_tokens), icon: <ArrowUpRight className="w-3.5 h-3.5 text-emerald-400" />, color: "text-emerald-400" },
          { label: "总计 Token", value: formatTokens(summary.total_tokens), icon: <Zap className="w-3.5 h-3.5 text-amber-400" />, color: "text-amber-400" },
        ].map(card => (
          <div key={card.label} className="p-3 rounded-xl bg-slate-900/30 border border-white/5">
            <div className="flex items-center gap-2 mb-1">
              {card.icon}
              <span className="text-[9px] text-slate-500">{card.label}</span>
            </div>
            <div className={`text-lg font-bold ${card.color}`}>{card.value}</div>
          </div>
        ))}
      </div>

      {/* 每日用量柱状图 */}
      {summary.daily.length > 0 && (
        <div className="p-4 rounded-xl bg-slate-900/30 border border-white/5">
          <h4 className="text-xs font-bold text-slate-300 mb-3">最近 14 天</h4>
          <div className="flex items-end gap-1 h-24">
            {summary.daily.slice(0, 14).reverse().map(d => {
              const height = Math.max(4, (d.total_tokens / maxTokens) * 80);
              return (
                <div key={d.date} className="flex-1 flex flex-col items-center gap-1" title={`${d.date}: ${formatTokens(d.total_tokens)} tokens, ${d.request_count} 次`}>
                  <div
                    className="w-full rounded-t bg-violet-500/60 hover:bg-violet-500/80 transition-all"
                    style={{ height: `${height}px` }}
                  />
                  <span className="text-[7px] text-slate-600">{d.date.slice(5)}</span>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* 按工具统计 */}
      {summary.by_tool.length > 0 && (
        <div className="p-4 rounded-xl bg-slate-900/30 border border-white/5">
          <h4 className="text-xs font-bold text-slate-300 mb-3">按工具</h4>
          <div className="space-y-2">
            {summary.by_tool.map(t => (
              <div key={t.tool_id} className="flex items-center gap-3">
                <span className="text-[11px] text-slate-300 font-mono w-24 truncate">{t.tool_id}</span>
                <div className="flex-1 h-2 bg-white/5 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-violet-500/60 rounded-full"
                    style={{ width: `${Math.min(100, (t.total_tokens / (summary.by_tool[0]?.total_tokens || 1)) * 100)}%` }}
                  />
                </div>
                <span className="text-[10px] text-slate-400 w-16 text-right">{formatTokens(t.total_tokens)}</span>
                <span className="text-[9px] text-slate-600 w-10 text-right">{t.request_count}次</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 按模型统计 */}
      {summary.by_model.length > 0 && (
        <div className="p-4 rounded-xl bg-slate-900/30 border border-white/5">
          <h4 className="text-xs font-bold text-slate-300 mb-3">按模型</h4>
          <div className="space-y-2">
            {summary.by_model.map(m => (
              <div key={m.model} className="flex items-center gap-3">
                <span className="text-[11px] text-slate-300 font-mono w-40 truncate">{m.model}</span>
                <span className="text-[9px] text-slate-500 w-20 truncate">{m.provider}</span>
                <div className="flex-1 h-2 bg-white/5 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-amber-500/60 rounded-full"
                    style={{ width: `${Math.min(100, (m.total_tokens / (summary.by_model[0]?.total_tokens || 1)) * 100)}%` }}
                  />
                </div>
                <span className="text-[10px] text-slate-400 w-16 text-right">{formatTokens(m.total_tokens)}</span>
                <span className="text-[9px] text-slate-600 w-10 text-right">{m.request_count}次</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
