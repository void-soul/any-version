import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Play, Square, ListChecks, CheckCircle2, XCircle, Loader2 } from "lucide-react";
import type { ApiEndpoint, ApiModule, SendRequestOutput, UnitTest, UnitTestRunOutput } from "./types";
import { endpointToInput, fmtTime, methodIcon, ACCENT } from "./panelParts";

interface RunnerResult {
  id: string;
  name: string;
  method: string;
  ok: boolean;
  status: number;
  time_ms: number;
  /** 是否执行了单测断言 */
  hasTests: boolean;
  /** 断言结果（null = 未执行断言） */
  testPass: boolean | null;
  error?: string;
}

/** 集合批量运行：按模块/全部接口顺序执行，可联动单测断言。 */
export function RunnerPanel({ endpoints, modules, variables, onSelectEndpoint }: {
  endpoints: ApiEndpoint[];
  modules: ApiModule[];
  variables: Record<string, string | number | boolean | null>;
  onSelectEndpoint: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [scope, setScope] = useState("all");
  const [withTests, setWithTests] = useState(true);
  const [stopOnFail, setStopOnFail] = useState(false);
  const [running, setRunning] = useState(false);
  const [results, setResults] = useState<RunnerResult[]>([]);
  const [total, setTotal] = useState(0);
  const [current, setCurrent] = useState<string | null>(null);
  const abortRef = useRef(false);

  const list = useMemo(
    () => (scope === "all" ? endpoints : endpoints.filter((e) => e.module_id === scope)),
    [endpoints, scope]
  );

  const summary = useMemo(() => {
    const pass = results.filter((r) => r.ok && r.testPass !== false).length;
    const fail = results.length - pass;
    const time = results.reduce((n, r) => n + r.time_ms, 0);
    return { pass, fail, time };
  }, [results]);

  const run = async () => {
    if (running || list.length === 0) return;
    abortRef.current = false;
    setResults([]);
    setTotal(list.length);
    setRunning(true);
    for (let i = 0; i < list.length; i++) {
      if (abortRef.current) break;
      const ep = list[i];
      setCurrent(`${i + 1}/${list.length} · ${ep.name}`);
      let entry: RunnerResult;
      try {
        if (withTests) {
          const tests = await invoke<UnitTest[]>("api_list_unit_tests", { endpointId: ep.id });
          if (tests.length > 0) {
            const outs = await invoke<UnitTestRunOutput[]>("api_run_unit_test", {
              endpointId: ep.id,
              variables,
            });
            const first = outs[0];
            entry = {
              id: ep.id,
              name: ep.name,
              method: ep.method,
              ok: !!first && first.status > 0 && first.status < 400,
              status: first?.status ?? 0,
              time_ms: outs.reduce((n, o) => n + o.time_ms, 0),
              hasTests: true,
              testPass: outs.every((o) => o.pass),
            };
          } else {
            entry = await plainRun(ep, variables);
          }
        } else {
          entry = await plainRun(ep, variables);
        }
      } catch (e) {
        entry = {
          id: ep.id,
          name: ep.name,
          method: ep.method,
          ok: false,
          status: 0,
          time_ms: 0,
          hasTests: false,
          testPass: null,
          error: String(e),
        };
      }
      setResults((prev) => [...prev, entry]);
      if ((!entry.ok || entry.testPass === false) && stopOnFail) break;
    }
    setRunning(false);
    setCurrent(null);
  };

  const stop = async () => {
    abortRef.current = true;
    try {
      await invoke("api_cancel_request");
    } catch {
      // 忽略取消失败
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2 flex-wrap">
        <select
          value={scope}
          onChange={(e) => setScope(e.target.value)}
          disabled={running}
          className="bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-body text-slate-200 cursor-pointer disabled:opacity-50"
        >
          <option value="all">{t("api.runnerAll")}</option>
          {modules.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
        </select>
        <label className="flex items-center gap-1.5 text-tiny text-slate-400 cursor-pointer">
          <input type="checkbox" checked={withTests} onChange={(e) => setWithTests(e.target.checked)} disabled={running} className="accent-[var(--module-accent)]" />
          {t("api.runnerWithTests")}
        </label>
        <label className="flex items-center gap-1.5 text-tiny text-slate-400 cursor-pointer">
          <input type="checkbox" checked={stopOnFail} onChange={(e) => setStopOnFail(e.target.checked)} disabled={running} className="accent-[var(--module-accent)]" />
          {t("api.runnerStopOnFail")}
        </label>
        <span className="text-tiny text-slate-500">{t("api.runnerCount", { n: list.length })}</span>
        <div className="ml-auto flex items-center gap-2">
          {running ? (
            <button onClick={stop} className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-body font-semibold text-white bg-rose-500/80 hover:bg-rose-500 cursor-pointer">
              <Square className="w-3.5 h-3.5" /> {t("api.runnerStop")}
            </button>
          ) : (
            <button
              onClick={run}
              disabled={list.length === 0}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-body font-semibold text-white cursor-pointer disabled:opacity-50"
              style={{ background: ACCENT }}
            >
              <Play className="w-3.5 h-3.5" /> {t("api.runnerStart")}
            </button>
          )}
        </div>
      </div>
      <div className="text-tiny text-slate-500">{t("api.runnerHint")}</div>

      {(running || results.length > 0) && (
        <div className="space-y-2">
          <div className="h-1.5 rounded-full bg-white/10 overflow-hidden">
            <div
              className="h-full bg-[var(--module-accent)] transition-[width]"
              style={{ width: `${total > 0 ? Math.min(100, (results.length / total) * 100) : 0}%` }}
            />
          </div>
          <div className="flex items-center gap-3 text-tiny text-slate-400">
            {running && <Loader2 className="w-3 h-3 animate-spin text-cyan-400" />}
            <span>{current ?? t("api.runnerDone", { n: results.length })}</span>
            <span className="text-emerald-400">{t("api.runnerPass", { n: summary.pass })}</span>
            <span className="text-rose-400">{t("api.runnerFail", { n: summary.fail })}</span>
            <span>{t("api.runnerTotalTime", { time: fmtTime(summary.time) })}</span>
          </div>
          <div className="rounded-ctl border border-white/10 bg-black/20 divide-y divide-white/5 max-h-[420px] overflow-y-auto">
            {results.map((r, idx) => {
              const Icon = methodIcon(r.method);
              return (
                <button
                  key={`${r.id}-${idx}`}
                  onClick={() => onSelectEndpoint(r.id)}
                  className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left hover:bg-white/5 cursor-pointer"
                  title={r.error || `${r.method} ${r.name}`}
                >
                  <Icon className="w-3 h-3 shrink-0 text-slate-400" />
                  <span className="w-14 shrink-0 text-micro font-semibold text-slate-500">{r.method}</span>
                  <span className="flex-1 truncate text-caption text-slate-300">{r.name}</span>
                  {r.status > 0 && (
                    <span className={`text-tiny tabular-nums ${r.status < 300 ? "text-emerald-400" : r.status < 500 ? "text-amber-400" : "text-rose-400"}`}>
                      {r.status}
                    </span>
                  )}
                  <span className="w-16 text-right text-tiny tabular-nums text-slate-500">{fmtTime(r.time_ms)}</span>
                  {r.hasTests ? (
                    r.testPass ? (
                      <span className="shrink-0" title={t("api.runnerAssertPass")}>
                        <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
                      </span>
                    ) : (
                      <span className="shrink-0" title={t("api.runnerAssertFail")}>
                        <XCircle className="w-3.5 h-3.5 text-rose-400" />
                      </span>
                    )
                  ) : (
                    <span className="shrink-0" title={t("api.runnerNoAssert")}>
                      <ListChecks className="w-3.5 h-3.5 text-slate-600" />
                    </span>
                  )}
                </button>
              );
            })}
            {results.length === 0 && running && (
              <div className="px-3 py-6 text-center text-caption text-slate-500">{t("api.runnerWaiting")}</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

async function plainRun(ep: ApiEndpoint, variables: Record<string, string | number | boolean | null>): Promise<RunnerResult> {
  const out = await invoke<SendRequestOutput>("api_send_request", {
    input: endpointToInput(ep, variables),
  });
  return {
    id: ep.id,
    name: ep.name,
    method: ep.method,
    ok: out.ok,
    status: out.status,
    time_ms: out.time_ms,
    hasTests: false,
    testPass: null,
  };
}
