// Token / 积分 / 调用量统计面板（对应后端 commands::buddy::stats，仅 WorkBuddy）。
//
// 数据源有两套，语义不同，界面上分开呈现：
// - Token 统计：扫本地会话 JSONL（"我本机跑了多少 token"），按 日/账号/模型 分桶，支持三维筛选；
// - 积分 / 调用量：官方计费接口（"平台扣了多少积分、发了多少次请求"），今日 60 秒缓存。

import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { AlertTriangle, BarChart3, ChevronDown, ChevronRight, CircleCheck, Eraser, Loader2, RefreshCw } from "lucide-react";

interface Bucket {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  calls: number;
  total: number;
}

interface TokenView {
  source: string;
  since: string;
  until: string;
  scannedFiles: number;
  totals: Bucket;
  daily: (Bucket & { day: string })[];
  models: (Bucket & { model: string })[];
  accounts: (Bucket & { account: string })[];
  dailyBreakdown: (Bucket & { day: string; account: string; model: string })[];
}

interface CreditView {
  source: string;
  since: string;
  until: string;
  totals: { credits: number; requests: number };
  daily: { day: string; credits: number; requests: number }[];
  accounts: { accountId: string; uid: string; name: string; credits: number; requests: number }[];
  models: { model: string; credits: number; requests: number }[];
  errors: string[];
}

interface AccountOption {
  id: string;
  email: string;
  nickname?: string | null;
}

const DAY_RANGES = [1, 7, 30, 90] as const;

/** 纯 CSS 柱状图：一行一天，宽度按最大值归一。 */
function BarChart({ rows, label }: { rows: { name: string; value: number }[]; label: string }) {
  const max = rows.reduce((acc, row) => Math.max(acc, row.value), 0);
  if (rows.length === 0) return null;
  return (
    <div className="space-y-0.5">
      {rows.map((row) => (
        <div key={row.name} className="flex items-center gap-2 text-[10px]">
          <span className="w-20 flex-shrink-0 text-slate-500">{row.name}</span>
          <div className="flex-1 h-2 rounded bg-white/5 overflow-hidden">
            <div
              className="h-full bg-[var(--module-accent)]"
              style={{ width: `${max > 0 ? Math.max(2, Math.round((row.value / max) * 100)) : 0}%` }}
              title={`${label}: ${row.value}`}
            />
          </div>
          <span className="w-16 flex-shrink-0 text-right text-slate-400">{row.value.toLocaleString()}</span>
        </div>
      ))}
    </div>
  );
}

function formatNumber(value: number | undefined): string {
  return (value ?? 0).toLocaleString();
}

export default function CreditStatsPanel({ accounts }: { accounts: AccountOption[] }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(true);
  const [days, setDays] = useState<number>(7);
  const [accountId, setAccountId] = useState<string>("");
  const [model, setModel] = useState<string>("");
  const [tokens, setTokens] = useState<TokenView | null>(null);
  const [credits, setCredits] = useState<CreditView | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const load = useCallback(
    async (force: boolean) => {
      setBusy(true);
      try {
        const [tokenView, creditView] = await Promise.all([
          invoke<TokenView>("buddy_token_stats", {
            days,
            account: accountId || null,
            model: model || null,
            force,
          }),
          invoke<CreditView>("buddy_credit_usage_stats", {
            accountId: accountId || null,
            days,
            force,
          }).catch((error) => {
            setMessage({ ok: false, text: String(error) });
            return null;
          }),
        ]);
        setTokens(tokenView);
        setCredits(creditView);
      } catch (error) {
        setMessage({ ok: false, text: String(error) });
      } finally {
        setBusy(false);
      }
    },
    [days, accountId, model]
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  const tokenDailyRows = useMemo(
    () => (tokens?.daily ?? []).map((row) => ({ name: row.day.slice(5), value: row.total })),
    [tokens]
  );
  const creditDailyRows = useMemo(
    () => (credits?.daily ?? []).map((row) => ({ name: row.day.slice(5), value: row.credits })),
    [credits]
  );
  const callDailyRows = useMemo(
    () => (credits?.daily ?? []).map((row) => ({ name: row.day.slice(5), value: row.requests })),
    [credits]
  );
  const modelOptions = useMemo(() => (tokens?.models ?? []).map((row) => row.model), [tokens]);

  const clearCache = async () => {
    setBusy(true);
    try {
      await invoke("buddy_stats_clear_cache");
      setMessage({ ok: true, text: t("buddy.stats.cacheCleared") });
      await load(true);
    } catch (error) {
      setMessage({ ok: false, text: String(error) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="xl:col-span-2 rounded-xl border border-white/10 bg-white/[0.03] p-4">
      <div className="flex items-center gap-2 mb-3 flex-wrap">
        <button onClick={() => setOpen((value) => !value)} className="flex items-center gap-2 cursor-pointer select-none">
          {open ? <ChevronDown className="w-4 h-4 text-slate-500" /> : <ChevronRight className="w-4 h-4 text-slate-500" />}
          <BarChart3 className="w-4 h-4 text-[var(--module-accent)]" />
          <span className="text-[13px] font-bold text-white">{t("buddy.stats.title")}</span>
        </button>
        {/* 日期区间 */}
        <div className="flex items-center gap-1 ml-2">
          {DAY_RANGES.map((range) => (
            <button
              key={range}
              onClick={() => setDays(range)}
              className={`px-2 py-0.5 rounded text-[10px] border cursor-pointer ${
                days === range
                  ? "bg-[var(--module-accent)]/25 text-white border-[var(--module-accent)]/40"
                  : "bg-white/5 text-slate-400 border-white/10 hover:text-white"
              }`}
            >
              {t("buddy.stats.range", { days: range })}
            </button>
          ))}
        </div>
        {/* 账号筛选 */}
        <select
          value={accountId}
          onChange={(event) => setAccountId(event.target.value)}
          className="px-2 py-0.5 rounded text-[10px] bg-white/5 border border-white/10 text-slate-300 outline-none cursor-pointer"
        >
          <option value="">{t("buddy.stats.allAccounts")}</option>
          {accounts.map((account) => (
            <option key={account.id} value={account.id}>
              {account.nickname || account.email || account.id}
            </option>
          ))}
        </select>
        {/* 模型筛选（Token 维度） */}
        <select
          value={model}
          onChange={(event) => setModel(event.target.value)}
          className="px-2 py-0.5 rounded text-[10px] bg-white/5 border border-white/10 text-slate-300 outline-none cursor-pointer"
        >
          <option value="">{t("buddy.stats.allModels")}</option>
          {modelOptions.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
        <div className="flex-1" />
        <button
          onClick={() => void load(true)}
          disabled={busy}
          className="px-2 py-1 rounded-md text-[10px] bg-white/5 hover:bg-white/10 border border-white/10 flex items-center gap-1 cursor-pointer disabled:opacity-40"
        >
          {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
          {t("buddy.auto.refresh")}
        </button>
        <button
          onClick={() => void clearCache()}
          disabled={busy}
          className="px-2 py-1 rounded-md text-[10px] bg-white/5 hover:bg-white/10 border border-white/10 flex items-center gap-1 cursor-pointer disabled:opacity-40"
        >
          <Eraser className="w-3 h-3" /> {t("buddy.stats.clearCache")}
        </button>
      </div>

      {message && (
        <div
          className={`mb-2 flex items-start gap-2 px-2 py-1.5 rounded-lg text-[10px] border ${
            message.ok
              ? "bg-emerald-500/10 text-emerald-300 border-emerald-500/20"
              : "bg-amber-500/10 text-amber-300 border-amber-500/20"
          }`}
        >
          {message.ok ? <CircleCheck className="w-3 h-3 mt-0.5" /> : <AlertTriangle className="w-3 h-3 mt-0.5" />}
          <span className="break-all">{message.text}</span>
          <button onClick={() => setMessage(null)} className="ml-auto text-slate-500 hover:text-white cursor-pointer">
            ✕
          </button>
        </div>
      )}

      {open && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-[11px]">
          {/* Token 统计 */}
          <div className="rounded-lg bg-white/[0.03] border border-white/5 p-2.5 space-y-2">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-slate-300">{t("buddy.stats.tokenTitle")}</span>
              <span className="ml-auto text-slate-400">
                {t("buddy.stats.tokenTotals", {
                  input: formatNumber(tokens?.totals.input),
                  output: formatNumber(tokens?.totals.output),
                  calls: formatNumber(tokens?.totals.calls),
                })}
              </span>
            </div>
            <div className="text-[10px] text-slate-500">
              {t("buddy.stats.tokenSource", {
                since: tokens?.since ?? "",
                until: tokens?.until ?? "",
                files: formatNumber(tokens?.scannedFiles),
              })}
            </div>
            <BarChart rows={tokenDailyRows} label={t("buddy.stats.tokens")} />
            {(tokens?.models?.length ?? 0) > 0 && (
              <div className="space-y-0.5 max-h-32 overflow-y-auto">
                {tokens?.models.slice(0, 12).map((row) => (
                  <div key={row.model} className="flex items-center gap-2 text-[10px]">
                    <span className="min-w-0 flex-1 truncate text-slate-400" title={row.model}>
                      {row.model}
                    </span>
                    <span className="text-slate-500">
                      {formatNumber(row.input)} / {formatNumber(row.output)}
                    </span>
                    <span className="w-16 text-right text-slate-300">{formatNumber(row.total)}</span>
                  </div>
                ))}
              </div>
            )}
            {tokens && tokens.totals.calls === 0 && (
              <div className="text-[10px] text-slate-600">{t("buddy.stats.tokenEmpty")}</div>
            )}
          </div>

          {/* 积分 / 调用量 */}
          <div className="rounded-lg bg-white/[0.03] border border-white/5 p-2.5 space-y-2">
            <div className="flex items-center gap-2">
              <span className="font-semibold text-slate-300">{t("buddy.stats.creditTitle")}</span>
              <span className="ml-auto text-slate-400">
                {t("buddy.stats.creditTotals", {
                  credits: formatNumber(credits?.totals.credits),
                  requests: formatNumber(credits?.totals.requests),
                })}
              </span>
            </div>
            <div className="text-[10px] text-slate-500">
              {t("buddy.stats.creditSource", { since: credits?.since ?? "", until: credits?.until ?? "" })}
            </div>
            {credits && credits.errors.length > 0 && (
              <div className="text-[10px] text-amber-300/80 break-all">
                {t("buddy.stats.creditErrors", { count: credits.errors.length })}
              </div>
            )}
            <BarChart rows={creditDailyRows} label={t("buddy.stats.credits")} />
            <BarChart rows={callDailyRows} label={t("buddy.stats.requests")} />
            {(credits?.accounts?.length ?? 0) > 0 && (
              <div className="space-y-0.5">
                {credits?.accounts.slice(0, 12).map((row) => (
                  <div key={row.uid} className="flex items-center gap-2 text-[10px]">
                    <span className="min-w-0 flex-1 truncate text-slate-400" title={row.name}>
                      {row.name}
                    </span>
                    <span className="text-slate-500">{row.requests}</span>
                    <span className="w-16 text-right text-slate-300">{row.credits}</span>
                  </div>
                ))}
              </div>
            )}
            {(credits?.models?.length ?? 0) > 0 && (
              <div className="space-y-0.5 max-h-24 overflow-y-auto border-t border-white/5 pt-1">
                {credits?.models.slice(0, 8).map((row) => (
                  <div key={row.model} className="flex items-center gap-2 text-[10px]">
                    <span className="min-w-0 flex-1 truncate text-slate-500" title={row.model}>
                      {row.model}
                    </span>
                    <span className="text-slate-600">{row.requests}</span>
                    <span className="w-16 text-right text-slate-400">{row.credits}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
