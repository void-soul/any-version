import { useMemo } from "react";
import { AlertTriangle, Ban, Check, Loader2 } from "lucide-react";
import { PHASE_KEYS_PROJECT, PHASE_KEYS_TEXT, PHASE_STEPS_PROJECT, PHASE_STEPS_TEXT, type AgentWorkbenchMode, type AiProgressEntry } from "./agentShared.state";
/** 阶段步进器：按事件流推导各阶段状态（待办/进行/完成/失败） */
export function PhaseStepper({ mode, entries, loading, t }: { mode: AgentWorkbenchMode; entries: readonly AiProgressEntry[]; loading: boolean; t: (k: string, o?: any) => string }) {
  // 对话模式没有「阶段计划」：工具循环的事件走时间线即可
  if (mode === "chat") return null;
  const phaseMap = mode === "project" ? PHASE_STEPS_PROJECT : PHASE_STEPS_TEXT;
  const keys = mode === "project" ? PHASE_KEYS_PROJECT : PHASE_KEYS_TEXT;
  const { maxPhase, failPhase, drawCount, cancelled } = useMemo(() => {
    let m = -1, fp = -1, dc = 0;
    let cancelled = false;
    for (const e of entries) {
      if (e.step === "cancel") cancelled = true;
      if (e.step === "view_done") dc++;
      if (e.step === "fail") { const p = phaseMap["view"] ?? 0; fp = Math.max(fp, p); }
      const p = phaseMap[e.step];
      if (p !== undefined && p > m) m = p;
    }
    return { maxPhase: m, failPhase: fp, drawCount: dc, cancelled };
  }, [entries, phaseMap]);
  // 运行结束（未失败）：全部阶段视为完成
  const finished = !loading && maxPhase >= 0;
  return (
    <div className="flex items-center gap-0.5 px-1 py-0.5">
      {keys.map((k, i) => {
        let state: "pending" | "active" | "done" | "fail" = "pending";
        if (i < maxPhase) state = "done";
        else if (i === maxPhase) state = finished ? "done" : "active";
        if (i === failPhase) state = "fail";
        return (
          <div key={k} className="flex items-center gap-0.5">
            {i > 0 && <span className={`h-px w-2.5 ${i <= maxPhase ? "bg-cyan-400/50" : "bg-white/10"}`} />}
            <span
              className={`inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[8px] font-medium ${
                state === "done" ? "border-emerald-400/30 bg-emerald-400/10 text-emerald-300"
                : state === "active" ? "border-cyan-400/50 bg-cyan-400/10 text-cyan-300"
                : state === "fail" ? "border-red-400/40 bg-red-400/10 text-red-300"
                : "border-white/10 bg-white/[0.03] text-slate-500"
              }`}>
              {state === "active" ? <Loader2 className="h-2.5 w-2.5 animate-spin" />
                : state === "done" ? <Check className="h-2.5 w-2.5" />
                : state === "fail" ? <AlertTriangle className="h-2.5 w-2.5" />
                : <span className="h-1 w-1 rounded-full bg-current" />}
              {t(k)}{i === keys.length - 1 && drawCount > 0 && <span className="text-[7px] opacity-80">×{drawCount}</span>}
            </span>
          </div>
        );
      })}
      {cancelled && <span className="ml-1 inline-flex items-center gap-1 rounded-full border border-red-400/30 bg-red-400/10 px-1.5 py-0.5 text-[8px] text-red-300"><Ban className="h-2.5 w-2.5" />{t("agent.stopped")}</span>}
    </div>
  );
}
