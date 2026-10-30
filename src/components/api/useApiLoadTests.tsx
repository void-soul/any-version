import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Play, Trash2, Square } from "lucide-react";
import type { ApiEndpoint, SendRequestInput, LoadTestConfig, LoadTestRun, LoadRunStatus } from "./types";
import { ACCENT } from "./panelParts";
import { LoadReportView } from "./panelReport";
import { theamedAlert } from "../shared/ThemedAlert";

/** Owns the ApiLoadTests feature, independently of page rendering. */
export function useApiLoadTests({ selectedId, draft, currentInput, variables }: {
  variables: Record<string, string | number | boolean | null>;
  selectedId: string | null;
  draft: ApiEndpoint | null;
  currentInput: SendRequestInput | null;
}) {
  const { t } = useTranslation();

  const [loadRuns, setLoadRuns] = useState<LoadTestRun[]>([]);

  const [loadConfig, setLoadConfig] = useState<LoadTestConfig>({ concurrency: 10, duration_secs: 10, ramp_up_secs: 0, rps_limit: 0 });

  const [runningRunId, setRunningRunId] = useState<string | null>(null);

  const [loadStatus, setLoadStatus] = useState<LoadRunStatus | null>(null);

  const pollRef = useRef<number | null>(null);

  // 压测轮询
  useEffect(() => {
    if (!runningRunId) return;
    let alive = true;
    const tick = async () => {
      try {
        const st = await invoke<LoadRunStatus>("api_load_run_status", { runId: runningRunId });
        if (!alive) return;
        setLoadStatus(st);
        if (!st.running && st.report) {
          setRunningRunId(null);
          // 监督任务落库报告有约 0.7s 延迟（等 worker 收尾），稍后刷新历史列表
          window.setTimeout(async () => {
            try {
              const runs = await invoke<LoadTestRun[]>("api_list_load_runs", { endpointId: selectedId });
              if (alive) setLoadRuns(runs);
            } catch { /* 忽略刷新失败 */ }
          }, 900);
        }
      } catch {
        if (alive) setRunningRunId(null);
      }
    };
    tick();
    pollRef.current = window.setInterval(tick, 1000);
    return () => {
      alive = false;
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, [runningRunId, selectedId]);

  const startLoadTest = async () => {
    if (!selectedId || !draft) return;
    try {
      const runId = await invoke<string>("api_start_load_test", {
        endpointId: selectedId,
        name: draft.name,
        config: loadConfig,
        variables,
        inputOverride: currentInput,
      });
      setLoadStatus({ running: true, elapsed_secs: 0, total: 0, success: 0, failed: 0, qps: 0, latency_avg_ms: 0, latency_p95_ms: 0, report: null });
      setRunningRunId(runId);
    } catch (e) {
      theamedAlert(String(e));
    }
  };

  // 压测参数变更：写入本地，切换接口后仍能恢复
  const updateLoadConfig = (patch: Partial<LoadTestConfig>) => {
    const next = { ...loadConfig, ...patch };
    setLoadConfig(next);
    if (selectedId) {
      try {
        localStorage.setItem(`any_version_api_load_cfg_${selectedId}`, JSON.stringify(next));
      } catch { /* 忽略写入失败 */ }
    }
  };

  const stopLoadTest = async () => {
    if (!runningRunId) return;
    try {
      await invoke("api_stop_load_test", { runId: runningRunId });
    } catch (e) {
      theamedAlert(String(e));
    }
  };

  const renderLoadTestPanel = () => draft && (<div className="space-y-3">
                <div className="grid grid-cols-4 gap-2">
                  {([
                    ["concurrency", t("api.concurrency"), loadConfig.concurrency, (v: number) => updateLoadConfig({ concurrency: v })],
                    ["duration_secs", t("api.duration"), loadConfig.duration_secs, (v: number) => updateLoadConfig({ duration_secs: v })],
                    ["ramp_up_secs", t("api.rampUp"), loadConfig.ramp_up_secs, (v: number) => updateLoadConfig({ ramp_up_secs: v })],
                    ["rps_limit", t("api.rpsLimit"), loadConfig.rps_limit, (v: number) => updateLoadConfig({ rps_limit: v })],
                  ] as const).map(([key, label, value, set]) => (
                    <label key={key} className="block">
                      <span className="text-tiny text-slate-500">{label}</span>
                      <input
                        type="number"
                        min={0}
                        value={value}
                        onChange={(e) => set(Number(e.target.value))}
                        className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body text-slate-200 focus:outline-none"
                      />
                    </label>
                  ))}
                </div>
                <div className="text-tiny text-slate-500">{t("api.loadHint")}</div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={startLoadTest}
                    disabled={!!runningRunId || !draft.url}
                    className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-body font-semibold text-white cursor-pointer disabled:opacity-50"
                    style={{ background: ACCENT }}
                  >
                    <Play className="w-3.5 h-3.5" />
                    {runningRunId ? t("api.loadRunning") : t("api.startLoad")}
                  </button>
                  {runningRunId && (
                    <button
                      onClick={stopLoadTest}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-body bg-rose-500/15 text-rose-300 hover:bg-rose-500/25 border border-rose-500/30 cursor-pointer"
                    >
                      <Square className="w-3.5 h-3.5" /> {t("api.stopLoad")}
                    </button>
                  )}
                  {runningRunId && loadStatus && (
                    <div className="flex-1 text-caption text-slate-300">
                      {t("api.loadStatus", { s: loadStatus.elapsed_secs, total: loadStatus.total, ok: loadStatus.success, fail: loadStatus.failed, qps: loadStatus.qps.toFixed(1), p95: loadStatus.latency_p95_ms.toFixed(1) })}
                    </div>
                  )}
                </div>
                {!runningRunId && loadStatus?.report && <LoadReportView report={loadStatus.report} />}
                {runningRunId && <div className="h-1.5 rounded-full bg-white/10 overflow-hidden"><div className="h-full bg-[var(--module-accent)]" style={{ width: `${Math.min(100, (loadStatus?.elapsed_secs ?? 0) / Math.max(1, loadConfig.duration_secs) * 100)}%` }} /></div>}
                {loadRuns.length > 0 && !runningRunId && (
                  <div className="space-y-1.5">
                    <div className="text-caption font-semibold text-slate-400">{t("api.loadHistory")}</div>
                    {loadRuns.map((run) => (
                      <div key={run.id} className="rounded-ctl border border-white/10 bg-black/20 px-3 py-2">
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-2 text-body text-slate-300">
                            <span className="font-semibold">{run.name || draft.name}</span>
                            <span className="text-tiny text-slate-500">{run.created_at.replace("T", " ").slice(0, 19)}</span>
                            <span className="text-tiny text-slate-500">{t("api.loadConfigInfo", { c: run.config.concurrency, d: run.config.duration_secs })}</span>
                          </div>
                          <button
                            onClick={async () => {
                              await invoke("api_delete_load_run", { runId: run.id });
                              setLoadRuns(loadRuns.filter((r) => r.id !== run.id));
                            }}
                            className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                        {run.report && <LoadReportView report={run.report} />}
                      </div>
                    ))}
                  </div>
                )}
              </div>);
  return {
    setLoadRuns,
    setLoadConfig,
    renderLoadTestPanel,
  };
}
