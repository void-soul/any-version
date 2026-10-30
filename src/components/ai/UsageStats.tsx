import { useState, useEffect, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
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
  Activity,
} from "lucide-react";
import { MODULE_CATALOG_MAP as MODULE_MAP, moduleLabel } from "../../moduleCatalog";
import type { DetectedAiTool } from "../../contracts/ai";

interface UsageSummary {
  total_records: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_tokens: number;
  /** 成功 / 失败请求数：失败请求也落库（只进成功率的分母） */
  total_success: number;
  total_failure: number;
  total_cache_read_tokens: number;
  by_tool: { tool_id: string; request_count: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
  by_model: {
    model: string;
    provider: string;
    request_count: number;
    input_tokens: number;
    output_tokens: number;
    total_tokens: number;
    output_tps?: number | null;
    success_count: number;
    failure_count: number;
    /** 0-1；无请求时为 null */
    success_rate?: number | null;
    cache_read_tokens: number;
    /** 0-1；未上报缓存的模型为 null */
    cache_hit_rate?: number | null;
  }[];
  by_provider: { provider: string; request_count: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
  recent: { date: string; request_count: number; input_tokens: number; output_tokens: number; total_tokens: number }[];
}



interface Row {
  key: string;
  label: string;
  sub?: string;
  /** 友好展示图标（模块 Lucide 图标或 AI 工具 emoji 头像）；缺省不渲染 */
  icon?: React.ReactNode;
  requests: number;
  input: number;
  output: number;
  total: number;
  /** 输出速度（tokens/s）：仅模型维度由后端聚合给出，其余维度无值不渲染 */
  tps?: number | null;
  /** 成功率（0-1）：仅模型维度由后端聚合给出；null/undefined 表示无数据 */
  successRate?: number | null;
  /** 失败请求数（有值时在 tooltip 里标出） */
  failureCount?: number;
  /** 缓存命中率（0-1）：仅模型维度由后端聚合给出；未上报缓存的模型为 null */
  cacheHitRate?: number | null;
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function formatFull(n: number): string {
  return n.toLocaleString("en-US");
}

// 自动刷新间隔（抄自 cc-switch 98ccde00）：0 表示关闭自动刷新
const DEFAULT_REFRESH_INTERVAL_MS = 30000;
const REFRESH_INTERVAL_OPTIONS_MS = [0, 5000, 10000, 30000, 60000] as const;
type RefreshIntervalOption = (typeof REFRESH_INTERVAL_OPTIONS_MS)[number];
const REFRESH_INTERVAL_KEYS: Record<number, string> = {
  0: "usagestats.interval0",
  5000: "usagestats.interval5000",
  10000: "usagestats.interval10000",
  30000: "usagestats.interval30000",
  60000: "usagestats.interval60000",
};
const LS_REFRESH_KEY = "usage_refresh_interval_ms";

function normalizeInterval(value: number | null | undefined): number {
  return (REFRESH_INTERVAL_OPTIONS_MS as readonly number[]).includes(value ?? -1)
    ? (value as number)
    : DEFAULT_REFRESH_INTERVAL_MS;
}

// ─── tool_id → 友好名称 + 图标 ───
// 三级解析：应用模块注册表 → 别名（tool_id ≠ 模块 id 的直连调用方）→ AI CLI 工具注册表
// （detect_ai_tools，displayName + emoji 头像）。都未命中时回退原始 id。
// 未来新模块只要把 tool_id 取成模块 id（或在别名表里登记），即可自动获得友好展示。

/** 直连 AI 通道的调用方：tool_id 与模块 id 不一致时在此登记（→ 模块 id） */
const TOOL_ID_ALIASES: Record<string, string> = {
  "api-import": "api", // API 智能导入 → api 模块
  // workBuddy2API：它的 tool_id 是 buddy2api，但面板注册表里的模块 id 是 buddy。
  // 不登记的话这一行只能回落成原始 id「buddy2api」——纯文本、无图标无颜色，
  // 在一排都有模块图标的行里显得像没归类的脏数据。
  "buddy2api": "buddy",
};

function toolDisplay(rawId: string, aiTools: DetectedAiTool[]): { label: string; icon?: React.ReactNode } {
  const moduleId = TOOL_ID_ALIASES[rawId] ?? rawId;
  const mod = MODULE_MAP[moduleId];
  if (mod) {
    const Icon = mod.icon;
    return { label: moduleLabel(moduleId), icon: <Icon className="w-3 h-3" style={{ color: mod.color }} /> };
  }
  const aiTool = aiTools.find(x => x.id === rawId);
  if (aiTool) {
    return {
      label: aiTool.nickname || aiTool.display_name,
      icon: aiTool.avatar ? <span className="text-tiny leading-none">{aiTool.avatar}</span> : undefined,
    };
  }
  return { label: rawId };
}

interface SectionStyle {
  iconClass: string;
  softClass: string;
  barClass: string;
}

const SECTION_STYLES: Record<string, SectionStyle> = {
  recent: { iconClass: "text-sky-400", softClass: "bg-sky-500/10", barClass: "bg-sky-500/60" },
  tool: { iconClass: "text-[var(--module-accent)]", softClass: "bg-violet-500/10", barClass: "bg-violet-500/60" },
  model: { iconClass: "text-amber-400", softClass: "bg-amber-500/10", barClass: "bg-amber-500/60" },
  provider: { iconClass: "text-emerald-400", softClass: "bg-emerald-500/10", barClass: "bg-emerald-500/60" },
};

/**
 * 一组统计的**柱状图**（原来是表格）。
 *
 * 为什么换：这四组要回答的是「谁花得多、什么时候花得多」——分布与量级对比，
 * 柱状图一眼能看出峰值和占比，表格得逐个读数字再在脑子里排。
 *
 * 柱子刻意做成**输入/输出堆叠**：换成图之后最容易丢的就是这个拆分（哪个模型是
 * 疯狂输入、哪个是疯狂输出，量级完全不同），堆叠能保住它，高度仍是总量。
 * 请求数、成功率、缓存命中率这些放不进图的信息挂到 title tooltip 上。
 */
function UsageBarSection({
  title,
  icon,
  rows,
  section,
  chronological,
}: {
  title: string;
  icon: React.ReactNode;
  rows: Row[];
  section: keyof typeof SECTION_STYLES;
  /**
   * 按**给定顺序**画（日期维度用）。
   *
   * 其余维度按总量降序排 —— 那是「谁花得多」的排名视图。
   * 但日期必须按日期走：后端已经是 ORDER BY date ASC，前端 .slice(-14) 取最近 14 天，
   * 结果本身有序；一旦也按总量排，柱子左右跳动，趋势线就没了。
   */
  chronological?: boolean;
}) {
  const { t } = useTranslation();
  const style = SECTION_STYLES[section];

  const bars = useMemo(() => {
    if (chronological) return rows;
    return [...rows].sort((a, b) => b.total - a.total);
  }, [rows, chronological]);

  // 分母至少为 1：全是 0 时不能除零，且柱子高度算出来是 NaN 会让整排消失
  const maxTotal = useMemo(() => Math.max(...rows.map(r => r.total), 1), [rows]);

  if (rows.length === 0) {
    return (
      <div className="rounded-card bg-slate-900/30 border border-white/5">
        <div className="flex items-center gap-1.5 px-3 py-2 text-tiny font-bold text-slate-300">
          <span className={`p-1 rounded-md ${style.softClass} ${style.iconClass}`}>{icon}</span>
          {title}
          <span className="text-micro font-normal text-slate-600 ml-1">{t("usagestats.noData")}</span>
        </div>
      </div>
    );
  }

  return (
    <div className="rounded-card bg-slate-900/30 border border-white/5 overflow-hidden">
      <div className="flex items-center gap-1.5 px-3 py-2 text-tiny font-bold text-slate-200 border-b border-white/[0.04] bg-white/[0.015]">
        <span className={`p-1 rounded-md ${style.softClass} ${style.iconClass}`}>{icon}</span>
        {title}
        <span className="text-micro font-normal text-slate-600 ml-1">{t("usagestats.rowsCount", { count: rows.length })}</span>
        <div className="flex-1" />
        {/* 图例：堆叠的两段分别是什么，不然两段同色系会被当成一根柱子 */}
        <span className="flex items-center gap-2 text-[8px] font-normal text-slate-500">
          <span className="flex items-center gap-1">
            <span className={`w-1.5 h-1.5 rounded-sm ${style.barClass} opacity-50`} />
            {t("usagestats.colInput")}
          </span>
          <span className="flex items-center gap-1">
            <span className={`w-1.5 h-1.5 rounded-sm ${style.barClass}`} />
            {t("usagestats.colOutput")}
          </span>
        </span>
      </div>

      <div className="flex items-stretch gap-1 px-3 py-2 h-36">
        {bars.map((r) => {
          const pct = Math.min(100, (r.total / maxTotal) * 100);
          // 零值也要留 2px 的底：完全没高度会让人以为这天没数据点、或柱子渲染失败了
          const height = r.total > 0 ? Math.max(pct, 1.5) : 0;
          const outShare = r.total > 0 ? (r.output / r.total) * 100 : 0;
          const inShare = r.total > 0 ? (r.input / r.total) * 100 : 0;
          const tip = [
            r.label,
            `${t("usagestats.colRequests")}: ${r.requests}`,
            `${t("usagestats.colInput")}: ${formatTokens(r.input)}`,
            `${t("usagestats.colOutput")}: ${formatTokens(r.output)}${typeof r.tps === "number" ? ` (${r.tps.toFixed(1)} t/s)` : ""}`,
            `${t("usagestats.colTotal")}: ${formatTokens(r.total)}`,
            r.successRate != null ? `${t("usagestats.colSuccess")}: ${(r.successRate * 100).toFixed(0)}%` : "",
            r.cacheHitRate != null ? `${t("usagestats.colCacheHit")}: ${(r.cacheHitRate * 100).toFixed(0)}%` : "",
          ].filter(Boolean).join("\n");
          return (
            <div key={r.key} className="flex-1 min-w-0 flex flex-col items-center gap-0.5 h-full" title={tip}>
              <span className="text-[8px] tabular-nums text-slate-400 truncate w-full text-center">
                {formatTokens(r.total)}
              </span>
              <div className="w-full flex-1 flex items-end">
                {height > 0 ? (
                  <div
                    className="w-full flex flex-col justify-end rounded-t-sm overflow-hidden"
                    style={{ height: `${height}%` }}
                  >
                    <div className={`w-full ${style.barClass}`} style={{ height: `${outShare}%` }} />
                    <div className={`w-full ${style.barClass} opacity-50`} style={{ height: `${inShare}%` }} />
                  </div>
                ) : (
                  <div className="w-full h-[2px] bg-white/10 rounded-sm" />
                )}
              </div>
              <span
                className="text-[8px] text-slate-500 truncate w-full text-center"
                title={r.label}
              >
                {/* 日期组只留 MM-DD：14 根柱子写全 2026-10-14 会挤成一团，年份月份重复 14 次也没信息量 */}
                {chronological ? r.label.slice(5) : r.label}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export default function UsageStats() {
  const { t } = useTranslation();
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [loading, setLoading] = useState(true);
  // AI CLI 工具注册表（Claude Code 等）：为代理启动的工具提供显示名 + emoji 头像
  const [aiTools, setAiTools] = useState<DetectedAiTool[]>([]);
  const [refreshIntervalMs, setRefreshIntervalMs] = useState<number>(() => {
    try {
      const saved = localStorage.getItem(LS_REFRESH_KEY);
      return normalizeInterval(saved ? Number(saved) : undefined);
    } catch {
      return DEFAULT_REFRESH_INTERVAL_MS;
    }
  });

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

  // AI CLI 工具注册表加载失败不致命：回退显示原始 tool_id
  useEffect(() => {
    invoke<DetectedAiTool[]>("detect_ai_tools").then(setAiTools).catch(() => setAiTools([]));
  }, []);

  // 自动刷新：间隔 > 0 时定时拉取；切走页面 / 关闭后由 cleanup 取消
  useEffect(() => {
    if (refreshIntervalMs <= 0) return;
    const id = setInterval(() => { load(); }, refreshIntervalMs);
    return () => clearInterval(id);
  }, [refreshIntervalMs, load]);

  const changeRefreshInterval = (next: number) => {
    const normalized = normalizeInterval(next);
    setRefreshIntervalMs(normalized);
    try {
      localStorage.setItem(LS_REFRESH_KEY, String(normalized));
    } catch {
      /* localStorage 不可用时静默降级为本次会话生效 */
    }
  };

  const handleClear = async () => {
    if (!confirm(t("usagestats.clearConfirm"))) return;
    await invoke("clear_usage");
    await load();
  };

  const toolRows: Row[] = (summary?.by_tool || []).map(tool => {
    const d = toolDisplay(tool.tool_id, aiTools);
    return {
      key: tool.tool_id,
      label: d.label,
      sub: d.label !== tool.tool_id ? tool.tool_id : undefined,
      icon: d.icon,
      requests: tool.request_count,
      input: tool.input_tokens,
      output: tool.output_tokens,
      total: tool.total_tokens,
    };
  });
  const modelRows: Row[] = (summary?.by_model || []).map(m => ({
    key: m.model + "|" + m.provider,
    label: m.model,
    sub: m.provider,
    requests: m.request_count,
    input: m.input_tokens,
    output: m.output_tokens,
    total: m.total_tokens,
    tps: m.output_tps ?? null,
    successRate: m.success_rate ?? null,
    failureCount: m.failure_count,
    cacheHitRate: m.cache_hit_rate ?? null,
  }));
  const providerRows: Row[] = (summary?.by_provider || []).map(p => ({
    key: p.provider || t("usagestats.unknown"),
    label: p.provider || t("usagestats.unknown"),
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
          <Activity className="w-4 h-4 text-[var(--module-accent)]" />
          <h3 className="text-sm font-bold text-white">{t("usagestats.title")}</h3>
        </div>
        <div className="flex gap-1.5 items-center">
          <div className="flex items-center gap-1 pr-1 text-slate-500" title={t("usagestats.refreshInterval")}>
            <Clock className="w-3 h-3" />
            <select
              value={refreshIntervalMs}
              onChange={(e) => changeRefreshInterval(Number(e.target.value))}
              className="bg-white/5 border border-white/10 rounded-ctl text-tiny text-slate-300 px-1.5 py-1 cursor-pointer hover:text-white hover:bg-white/10 transition-all focus:outline-none"
            >
              {REFRESH_INTERVAL_OPTIONS_MS.map((ms) => (
                <option key={ms} value={ms} className="bg-surface-topbar text-slate-200">
                  {t(REFRESH_INTERVAL_KEYS[ms as RefreshIntervalOption] ?? "usagestats.interval0")}
                </option>
              ))}
            </select>
          </div>
          <button onClick={load} disabled={loading}
            className="px-2.5 py-1 rounded-ctl bg-white/5 border border-white/10 text-tiny text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex items-center gap-1 disabled:opacity-50">
            <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} /> {t("usagestats.refresh")}
          </button>
          <button onClick={handleClear} disabled={!hasData}
            className="px-2.5 py-1 rounded-ctl bg-white/5 border border-white/10 text-tiny text-slate-300 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all flex items-center gap-1 disabled:opacity-30 disabled:cursor-not-allowed">
            <Trash2 className="w-3 h-3" /> {t("usagestats.clear")}
          </button>
        </div>
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto p-3 space-y-2.5">
        {loading ? (
          <div className="h-full flex items-center justify-center text-slate-500">
            <RefreshCw className="w-5 h-5 animate-spin mr-2" />
            <span className="text-body">{t("usagestats.loading")}</span>
          </div>
        ) : !hasData ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500">
            <div className="p-4 rounded-panel bg-slate-900/40 border border-white/5 mb-3">
              <Hash className="w-8 h-8 text-slate-700" />
            </div>
            <span className="text-sm font-bold text-slate-400">{t("usagestats.emptyTitle")}</span>
            <span className="text-tiny text-slate-600 mt-1">{t("usagestats.emptyDesc")}</span>
            <button onClick={load}
              className="mt-3 px-3 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-tiny text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex items-center gap-1">
              <RefreshCw className="w-3 h-3" /> {t("usagestats.refresh")}
            </button>
          </div>
        ) : (
          <>
            {/* 主卡：总 Token 消耗 + 输入/输出占比 */}
            <div className="relative rounded-panel bg-gradient-to-br from-[color-mix(in_srgb,var(--module-accent)_10%,transparent)] via-slate-900/30 to-amber-500/[0.06] border border-white/5 p-4 overflow-hidden">
              <div className="absolute -right-6 -top-6 w-24 h-24 rounded-full bg-[var(--module-accent-soft)] blur-2xl" />
              <div className="relative flex items-start justify-between">
                <div>
                  <div className="text-tiny text-slate-400 flex items-center gap-1">
                    <Activity className="w-3 h-3 text-[var(--module-accent)]" /> {t("usagestats.totalTokens")}
                  </div>
                  <div className="text-3xl font-bold text-white tabular-nums mt-1 leading-none">
                    {formatTokens(totalTokens)}
                  </div>
                  <div className="text-micro text-slate-500 mt-1.5 tabular-nums">
                    {t("usagestats.summaryLine", { tokens: formatFull(totalTokens), count: totalRecords, avg: formatTokens(avgPerReq) })}
                  </div>
                </div>
                <div className="flex gap-1.5">
                  <span className="px-2 py-1 rounded-ctl bg-blue-500/10 border border-blue-500/20 text-blue-300 text-tiny font-semibold flex items-center gap-1 tabular-nums">
                    <ArrowDownRight className="w-3 h-3" />{formatTokens(totalInput)}
                  </span>
                  <span className="px-2 py-1 rounded-ctl bg-emerald-500/10 border border-emerald-500/20 text-emerald-300 text-tiny font-semibold flex items-center gap-1 tabular-nums">
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
                <span className="text-blue-300/80">{t("usagestats.inputPct", { pct: inputPct })}</span>
                <span className="text-emerald-300/80">{t("usagestats.outputPct", { pct: outputPct })}</span>
              </div>
            </div>

            {/* 指标卡 */}
            <div className="grid grid-cols-3 gap-2">
              <div className="rounded-card bg-slate-900/30 border border-white/5 p-3">
                <div className="text-micro text-slate-500 flex items-center gap-1"><Hash className="w-3 h-3 text-[var(--module-accent)]" />{t("usagestats.reqTotal")}</div>
                <div className="text-lg font-bold text-slate-100 tabular-nums mt-1">{formatTokens(totalRecords)}</div>
              </div>
              <div className="rounded-card bg-slate-900/30 border border-white/5 p-3">
                <div className="text-micro text-slate-500 flex items-center gap-1"><ArrowDownRight className="w-3 h-3 text-blue-400" />{t("usagestats.inputTokens")}</div>
                <div className="text-lg font-bold text-slate-100 tabular-nums mt-1">{formatTokens(totalInput)}</div>
              </div>
              <div className="rounded-card bg-slate-900/30 border border-white/5 p-3">
                <div className="text-micro text-slate-500 flex items-center gap-1"><ArrowUpRight className="w-3 h-3 text-emerald-400" />{t("usagestats.outputTokens")}</div>
                <div className="text-lg font-bold text-slate-100 tabular-nums mt-1">{formatTokens(totalOutput)}</div>
              </div>
            </div>

            {/* 只有日期组按给定顺序画（后端已 ORDER BY date ASC，这里传 chronological 别再按量重排） */}
                      <UsageBarSection title={t("usagestats.recent14")} icon={<Clock className="w-3 h-3" />} section="recent" rows={recentRows} chronological />
                      <UsageBarSection title={t("usagestats.tools")} icon={<Boxes className="w-3 h-3" />} section="tool" rows={toolRows} />
                      <UsageBarSection title={t("usagestats.models")} icon={<Cpu className="w-3 h-3" />} section="model" rows={modelRows} />
                      <UsageBarSection title={t("usagestats.providers")} icon={<Server className="w-3 h-3" />} section="provider" rows={providerRows} />
          </>
        )}
      </div>
    </div>
  );
}
