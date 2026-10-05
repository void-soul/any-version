// Buddy → 2API 选项卡：启停 / 三项自检 / 客户端接入 / 端点自检
//
// 后端是内嵌在 kira 主进程的 axum 服务（buddy/twoapi），协议转换复用 src-tauri/src/proxy。
// 这里只做控制与展示；账号联动由后端两处完成：Buddy 面板切号时 `on_account_switched`
// 立刻热更新，在 WorkBuddy 客户端里自己切号则由服务运行期间的凭据巡查兜住（最坏 30s）。
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Check, Copy, Play, RefreshCw, Square } from "lucide-react";

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

      {/* 说明 */}
      <div className="text-slate-500 leading-relaxed">{t("buddy.twoapi.hint")}</div>
    </div>
  );
}
