import { useState, useEffect, useCallback } from "react";
import { theamedAlert } from "../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Play } from "lucide-react";
import { fmtDate, type SchedulerState } from "./CertManager.model";
// ---- 调度日志 ----
export function SchedulerView() {
  const { t } = useTranslation();
  const [state, setState] = useState<SchedulerState | null>(null);
  const [running, setRunning] = useState(false);
  const [runResult, setRunResult] = useState<string[]>([]);

  const refresh = useCallback(async () => {
    setState(await invoke<SchedulerState>("cert_scheduler_status"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const toggle = async (enabled: boolean) => {
    try {
      await invoke("cert_scheduler_set", { enabled, intervalMinutes: state?.interval_minutes || 360 });
      await refresh();
    } catch (e) {
      theamedAlert(String(e));
    }
  };
  const runNow = async () => {
    setRunning(true);
    try {
      setRunResult(await invoke<string[]>("cert_scheduler_run_now"));
    } catch (e) {
      setRunResult([String(e)]);
    } finally {
      setRunning(false);
      refresh();
    }
  };

  return (
    <div className="space-y-4">
      <h3 className="text-sm font-bold text-slate-200">{t("certmgr.schedTitle")}</h3>
      <div className="rounded-ctl border border-white/5 bg-white/5 p-4 text-caption text-slate-300 space-y-2">
        <div>{t("certmgr.statusLabel")}{state?.enabled ? <span className="text-emerald-400">{t("certmgr.stateRunning")}</span> : <span className="text-amber-400">{t("certmgr.statePaused")}</span>}</div>
        <div>{t("certmgr.scanInterval", { count: state?.interval_minutes ?? 0 })}</div>
        <div>{t("certmgr.lastRun", { time: fmtDate(state?.last_run_at) })}</div>
        <div>{t("certmgr.nextRun", { time: fmtDate(state?.next_run_at) })}</div>
        <div className="flex gap-2 pt-1">
          <button onClick={() => toggle(!state?.enabled)} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-200 text-caption">
            {state?.enabled ? t("certmgr.pause") : t("certmgr.enable")}
          </button>
          <button onClick={runNow} disabled={running} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption flex items-center gap-1.5">
            <Play className="w-3 h-3" /> {running ? t("certmgr.runningNow") : t("certmgr.runAll")}
          </button>
        </div>
      </div>

      {runResult.length > 0 && (
        <pre className="text-tiny bg-black/40 rounded p-2 overflow-auto max-h-32 text-slate-300">{runResult.join("\n")}</pre>
      )}

      <div>
        <div className="text-caption text-slate-400 mb-2">{t("certmgr.history")}</div>
        <div className="space-y-1">
          {(state?.log || []).slice().reverse().map((e, i) => (
            <div key={i} className="text-tiny flex items-center gap-2">
              <RefreshCw className={`w-3 h-3 ${e.ok ? "text-emerald-400" : "text-rose-400"}`} />
              <span className="text-slate-500">{fmtDate(e.at)}</span>
              <span className="text-slate-300">{e.cert_id}</span>
              <span className="text-slate-400">{e.action}</span>
              <span className={e.ok ? "text-emerald-400" : "text-rose-400"}>{e.message}</span>
            </div>
          ))}
          {(!state?.log || state.log.length === 0) && <div className="text-tiny text-slate-500">{t("certmgr.noRecords")}</div>}
        </div>
      </div>
    </div>
  );
}
