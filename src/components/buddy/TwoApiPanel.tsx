// Buddy → 2API 选项卡：启停 / 三项自检 / 客户端接入 / 端点自检
//
// 后端是内嵌在 kira 主进程的 axum 服务（buddy/twoapi），协议转换复用 src-tauri/src/proxy。
// 这里只做控制与展示；账号联动由后端两处完成：Buddy 面板切号时 `on_account_switched`
// 立刻热更新，在 WorkBuddy 客户端里自己切号则由服务运行期间的凭据巡查兜住（最坏 30s）。
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Check, Copy, Play, RefreshCw, Square, Trash2 } from "lucide-react";

type Phase = "stopped" | "starting" | "running" | "failed";

interface TwoApiStatus {
  phase: Phase;
  port: number;
  startedAtMs: number | null;
  lastError: string | null;
  account: string | null;
  modelCount: number;
}

interface TwoApiCheck {
  id: string;
  label: string;
  ok: boolean;
  detail: string;
}

interface TwoApiReport {
  ok: boolean;
  checks: TwoApiCheck[];
  message: string;
}

interface RequestLogEntry {
  ts: string;
  source: string;
  message: string;
}

/** 用量统计（后端 `ai_usage` 表按 tool_id=buddy2api 聚合，口径与 AI 用量面板一致） */
interface UsageByModel {
  model: string;
  request_count: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  /** 无速率数据（未测耗时）时为 null */
  output_tps: number | null;
  success_rate: number | null;
  cache_hit_rate: number | null;
}

interface UsageSummary {
  total_records: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_success: number;
  total_failure: number;
  total_cache_read_tokens: number;
  by_model: UsageByModel[];
}

/** 比率：没有请求时显示「—」而不是 0%（两者是两回事） */
function pct(ok: number, total: number): string {
  if (total === 0) return "—";
  return `${((ok / total) * 100).toFixed(1)}%`;
}

/** 统计卡片：一行一个指标（值 + 说明） */
function StatCell({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex flex-col gap-0.5 px-2 py-1.5 rounded bg-white/5" title={title}>
      <span className="text-slate-500">{label}</span>
      <span className="text-slate-100 font-semibold tabular-nums">{value}</span>
    </div>
  );
}

/** 与后端 twoapi::AUTOSTART_ID 一致：写进 config.auto_start_services */
const AUTOSTART_ID = "buddy2api";

export default function TwoApiPanel() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<TwoApiStatus | null>(null);
  const [report, setReport] = useState<TwoApiReport | null>(null);
  const [autoStart, setAutoStart] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [copied, setCopied] = useState("");

  const refresh = useCallback(() => {
    invoke<TwoApiStatus>("buddy2api_status")
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  // 状态轮询：服务在跑时端口/账号/模型数会变（Buddy 切号后账号即时更新）
  useEffect(() => {
    refresh();
    const id = window.setInterval(refresh, 2000);
    return () => window.clearInterval(id);
  }, [refresh]);

  // ─── 请求日志 ───
  const [logs, setLogs] = useState<RequestLogEntry[]>([]);
  const logsEndRef = useRef<HTMLDivElement>(null);

  const refreshLogs = useCallback(() => {
    invoke<RequestLogEntry[]>("buddy2api_request_logs")
      .then(setLogs)
      .catch(() => {});
  }, []);

  // 日志轮询（与状态轮询同频；请求日志是内存环形缓冲，轮询拿全量）
  useEffect(() => {
    refreshLogs();
    const id = window.setInterval(refreshLogs, 2000);
    return () => window.clearInterval(id);
  }, [refreshLogs]);

  // 有新日志时滚到底部
  useEffect(() => {
    logsEndRef.current?.scrollIntoView({ block: "nearest" });
  }, [logs]);

  // ─── 用量统计 ───
  const [usage, setUsage] = useState<UsageSummary | null>(null);

  // 统计只在「有请求跑完」时才变，10s 足够（状态/日志是 2s）；
  // 数据是累计值，服务重启不清零。
  const refreshUsage = useCallback(() => {
    invoke<UsageSummary>("buddy2api_usage_stats")
      .then(setUsage)
      .catch(() => setUsage(null));
  }, []);

  useEffect(() => {
    refreshUsage();
    const id = window.setInterval(refreshUsage, 10000);
    return () => window.clearInterval(id);
  }, [refreshUsage]);

  const clearLogs = async () => {
    try {
      await invoke("buddy2api_clear_request_logs");
      setLogs([]);
    } catch (e) {
      setNotice(String(e));
    }
  };

  useEffect(() => {
    void invoke<string[]>("get_auto_start_services")
      .then((list) => setAutoStart((list || []).includes(AUTOSTART_ID)))
      .catch(() => {});
  }, []);

  const toggleAutoStart = async () => {
    const next = !autoStart;
    setAutoStart(next);
    try {
      await invoke("set_auto_start_service", { serviceId: AUTOSTART_ID, enabled: next });
    } catch (e) {
      setAutoStart(!next);
      setNotice(String(e));
    }
  };

  const [portInput, setPortInput] = useState("");

  // 状态里的端口是实际在用的（可能与配置不同：服务未启动时显示配置值）
  useEffect(() => {
    if (status?.port) setPortInput(String(status.port));
  }, [status?.port]);

  const savePort = async () => {
    const n = Number(portInput);
    if (!Number.isInteger(n) || n < 1024 || n > 65535) {
      setNotice(t("buddy.twoapi.badPort"));
      return;
    }
    setBusy(true);
    setNotice("");
    try {
      // 后端会：保存配置 → 同步 AI 模块里指向 2API 的供应商 URL → 若在跑则按新端口重启
      setStatus(await invoke<TwoApiStatus>("buddy2api_set_port", { port: n }));
      setNotice(t("buddy.twoapi.portSaved"));
    } catch (e) {
      setNotice(String(e));
    } finally {
      setBusy(false);
    }
  };

  const runPreflight = useCallback(async () => {
    setBusy(true);
    try {
      setReport(await invoke<TwoApiReport>("buddy2api_preflight"));
    } catch (e) {
      setNotice(String(e));
    } finally {
      setBusy(false);
    }
  }, []);

  const start = async () => {
    setBusy(true);
    setNotice("");
    try {
      setStatus(await invoke<TwoApiStatus>("buddy2api_start", { port: null }));
    } catch (e) {
      setNotice(String(e));
      setStatus(await invoke<TwoApiStatus>("buddy2api_status"));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      setStatus(await invoke<TwoApiStatus>("buddy2api_stop"));
    } catch (e) {
      setNotice(String(e));
    } finally {
      setBusy(false);
    }
  };

  const copy = async (text: string, tag: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(tag);
      window.setTimeout(() => setCopied(""), 1500);
    } catch {
      setNotice(t("buddy.twoapi.copyFailed"));
    }
  };

  const running = status?.phase === "running";
  const baseUrl = status ? "http://127.0.0.1:" + status.port : "http://127.0.0.1:8788";

  return (
    <div className="flex flex-col gap-3 p-3 text-tiny">
      {/* ① 状态条 */}
      <div className="flex items-center gap-2 flex-wrap">
        <span
          className={
            "inline-flex items-center gap-1 px-2 py-0.5 rounded border " +
            (running
              ? "border-emerald-400/40 text-emerald-300"
              : status?.phase === "failed"
              ? "border-rose-400/40 text-rose-300"
              : "border-white/15 text-slate-400")
          }
        >
          <span className={"w-1.5 h-1.5 rounded-full " + (running ? "bg-emerald-400" : "bg-slate-500")} />
          {running ? t("buddy.twoapi.running") : status?.phase === "failed" ? t("buddy.twoapi.failed") : t("buddy.twoapi.stopped")}
        </span>
        <span className="text-slate-400">
          {baseUrl} · {t("buddy.twoapi.account")}: {status?.account || "-"}
        </span>
        {status && status.modelCount > 0 && (
          <span className="text-slate-500">
            {t("buddy.twoapi.modelCount", { count: status.modelCount })}
          </span>
        )}
      </div>

      {status?.lastError && (
        <div className="px-2 py-1.5 rounded bg-rose-500/10 border border-rose-400/25 text-rose-200 break-words">
          {status.lastError}
        </div>
      )}
      {notice && (
        <div className="px-2 py-1.5 rounded bg-amber-500/10 border border-amber-400/25 text-amber-200 break-words">
          {notice}
        </div>
      )}

      {/* ② 控制 */}
      <div className="flex items-center gap-2 flex-wrap">
        {running ? (
          <button
            onClick={() => void stop()}
            disabled={busy}
            className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-white/10 hover:bg-white/15 disabled:opacity-50"
          >
            <Square className="w-3 h-3" /> {t("buddy.twoapi.stop")}
          </button>
        ) : (
          <button
            onClick={() => void start()}
            disabled={busy}
            className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-[var(--module-accent)] text-white disabled:opacity-50"
          >
            <Play className="w-3 h-3" /> {t("buddy.twoapi.start")}
          </button>
        )}
        <button
          onClick={() => void runPreflight()}
          disabled={busy}
          className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-white/10 hover:bg-white/15 disabled:opacity-50"
        >
          <RefreshCw className="w-3 h-3" /> {t("buddy.twoapi.preflight")}
        </button>
        <label className="inline-flex items-center gap-1.5 cursor-pointer text-slate-300">
          <input type="checkbox" checked={autoStart} onChange={() => void toggleAutoStart()} />
          {t("buddy.twoapi.autoStart")}
        </label>
        <span className="inline-flex items-center gap-1">
          <span className="text-slate-400">{t("buddy.twoapi.port")}</span>
          <input
            value={portInput}
            onChange={(e) => setPortInput(e.target.value.replace(/[^0-9]/g, ""))}
            onKeyDown={(e) => {
              if (e.key === "Enter") void savePort();
            }}
            className="w-20 px-1.5 py-0.5 rounded bg-black/40 border border-white/15 text-slate-200 outline-none focus:border-[var(--module-accent)]/50"
          />
          <button
            onClick={() => void savePort()}
            disabled={busy}
            className="px-2 py-0.5 rounded bg-white/10 hover:bg-white/15 disabled:opacity-50"
          >
            {t("buddy.twoapi.savePort")}
          </button>
        </span>
      </div>

      {/* ③ 自检结果 */}
      {report && (
        <div className="flex flex-col gap-1 p-2 rounded bg-black/20 border border-white/8">
          {report.checks.map((c) => (
            <div key={c.id} className="flex items-start gap-2">
              <span
                className={
                  "mt-0.5 inline-flex items-center justify-center w-3.5 h-3.5 rounded " +
                  (c.ok ? "bg-emerald-500/20 text-emerald-300" : "bg-rose-500/20 text-rose-300")
                }
              >
                {c.ok ? <Check className="w-2.5 h-2.5" /> : <span className="text-[9px]">!</span>}
              </span>
              <span className="text-slate-200">{c.label}</span>
              <span className="text-slate-500 break-words">{c.detail}</span>
            </div>
          ))}
        </div>
      )}

      {/* ④ 客户端接入 */}
      <div className="flex flex-col gap-1.5 p-2 rounded bg-black/20 border border-white/8">
        <div className="flex items-center gap-2">
          <span className="text-slate-400">{t("buddy.twoapi.baseUrl")}</span>
          <code className="text-slate-200">{baseUrl}</code>
          <button
            onClick={() => void copy(baseUrl, "url")}
            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/10 hover:bg-white/15"
          >
            {copied === "url" ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
          </button>
        </div>
        <div className="text-slate-500">{t("buddy.twoapi.endpoints")}</div>
        <div className="flex flex-wrap gap-1">
          {["/v1/chat/completions", "/v1/messages", "/v1/responses", "/v1/models"].map((p) => (
            <code key={p} className="px-1.5 py-0.5 rounded bg-white/5 text-slate-300">
              {p}
            </code>
          ))}
        </div>
      </div>

      {/* ⑤ 请求日志（2API 服务转发的请求，可清空） */}
      <div className="flex flex-col rounded bg-black/20 border border-white/8">
        <div className="flex items-center gap-2 px-2 py-1.5 border-b border-white/8">
          <span className="text-slate-300 font-semibold">{t("buddy.twoapi.requestLog")}</span>
          <span className="text-slate-600 tabular-nums">({logs.length})</span>
          <div className="flex-1" />
          <button
            onClick={() => void refreshLogs()}
            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer transition"
            title={t("buddy.refresh")}
          >
            <RefreshCw className="w-3 h-3" />
          </button>
          <button
            onClick={() => void clearLogs()}
            disabled={logs.length === 0}
            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 hover:text-rose-300 cursor-pointer transition disabled:opacity-40 disabled:cursor-not-allowed"
            title={t("buddy.twoapi.clearLogs")}
          >
            <Trash2 className="w-3 h-3" /> {t("buddy.twoapi.clearLogs")}
          </button>
        </div>
        <div className="px-2 py-1.5 font-mono text-[10px] leading-relaxed max-h-48 overflow-y-auto">
          {logs.length === 0 ? (
            <div className="text-slate-600">{t("buddy.twoapi.noRequestLog")}</div>
          ) : (
            logs.map((l, i) => (
              <div key={i} className="text-slate-400 whitespace-pre-wrap break-all">
                <span className="text-slate-600">{l.ts}</span> {l.message}
              </div>
            ))
          )}
          <div ref={logsEndRef} />
        </div>
      </div>

      {/* ⑥ 用量统计（累计，按 tool_id=buddy2api 从 ai_usage 聚合；口径同 AI 用量面板） */}
      <div className="flex flex-col rounded bg-black/20 border border-white/8">
        <div className="flex items-center gap-2 px-2 py-1.5 border-b border-white/8">
          <span className="text-slate-300 font-semibold">{t("buddy.twoapi.usageTitle")}</span>
          <div className="flex-1" />
          <button
            onClick={() => void refreshUsage()}
            className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer transition"
            title={t("buddy.refresh")}
          >
            <RefreshCw className="w-3 h-3" />
          </button>
        </div>

        {usage === null ? (
          <div className="px-2 py-1.5 text-slate-600">{t("buddy.twoapi.usageLoading")}</div>
        ) : usage.total_records === 0 ? (
          <div className="px-2 py-1.5 text-slate-600">{t("buddy.twoapi.usageEmpty")}</div>
        ) : (
          <div className="flex flex-col gap-1.5 p-2">
            <div className="grid grid-cols-3 gap-1 sm:grid-cols-4">
              <StatCell
                label={t("buddy.twoapi.usageRequests")}
                value={usage.total_records.toLocaleString()}
              />
              <StatCell
                label={t("buddy.twoapi.usageSuccessRate")}
                value={pct(usage.total_success, usage.total_records)}
                title={`${usage.total_success} / ${usage.total_records}`}
              />
              <StatCell
                label={t("buddy.twoapi.usageInput")}
                value={usage.total_input_tokens.toLocaleString()}
              />
              <StatCell
                label={t("buddy.twoapi.usageOutput")}
                value={usage.total_output_tokens.toLocaleString()}
              />
              <StatCell
                label={t("buddy.twoapi.usageTotal")}
                value={(usage.total_input_tokens + usage.total_output_tokens).toLocaleString()}
              />
              <StatCell
                label={t("buddy.twoapi.usageCacheRead")}
                value={usage.total_cache_read_tokens.toLocaleString()}
              />
              <StatCell
                label={t("buddy.twoapi.usageFailures")}
                value={usage.total_failure.toLocaleString()}
              />
            </div>

            {/* 按模型 Top 5：总量最大的 5 个，速度与命中率只在测到时显示 */}
            <div className="flex flex-col gap-0.5">
              <span className="text-slate-500">{t("buddy.twoapi.usageByModel")}</span>
              {usage.by_model.slice(0, 5).map((m) => (
                <div key={m.model} className="flex items-center gap-2 text-slate-400">
                  <span className="flex-1 min-w-0 truncate text-slate-300">{m.model}</span>
                  <span className="tabular-nums">{m.request_count.toLocaleString()} 次</span>
                  <span className="tabular-nums">{m.total_tokens.toLocaleString()} tok</span>
                  <span className="tabular-nums text-slate-500">
                    {m.output_tps != null ? `${m.output_tps.toFixed(1)} tok/s` : ""}
                  </span>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* 说明 */}
      <div className="text-slate-500 leading-relaxed">{t("buddy.twoapi.hint")}</div>
    </div>
  );
}
