import { Check } from "lucide-react";
import type { AiImportResult } from "./types";
import { fmtNum, viewLabel } from "./agentShared.state";
/** 运行完成的结果卡：视图清单 + 节点/证据/用量统计 + 校验报告入口 */
export function ResultCard({ result, t, onShowReport }: { result: AiImportResult; t: (k: string, o?: any) => string; onShowReport: () => void }) {
  const usage = result.usage;
  const totalNodes = result.documents.reduce((s, d) => s + d.nodes.length, 0);
  return (
    <div className="overflow-hidden rounded-ctl border border-emerald-400/20 bg-emerald-400/[0.04]">
      <div className="flex items-center gap-2 border-b border-emerald-400/15 px-2.5 py-1.5">
        <Check className="h-3 w-3 text-emerald-300" />
        <span className="text-tiny font-semibold text-emerald-200">{t("agent.resultTitle", { count: result.documents.length })}</span>
        <span className="ml-auto font-mono text-[8px] tabular-nums text-slate-400">
          {t("mindmap.aiRunUsageLine", { req: usage.requests, in: fmtNum(usage.inputTokens), out: fmtNum(usage.outputTokens), total: fmtNum(usage.totalTokens) })}
        </span>
      </div>
      <div className="space-y-1 px-2.5 py-2">
        {result.documents.map((d) => {
          const r = result.reports.find((x) => x.documentId === d.document.id);
          const ok = (r?.diagnostics.length ?? 0) === 0;
          return (
            <div key={d.document.id} className="flex items-center gap-1.5 text-micro">
              <span className="shrink-0 rounded border border-cyan-400/40 bg-cyan-400/10 px-1.5 py-px text-[8px] text-cyan-300">{viewLabel(t, r?.view ?? "architecture")}</span>
              <span className="min-w-0 flex-1 truncate text-slate-300" title={d.document.name}>{d.document.name}</span>
              <span className="shrink-0 font-mono text-[8px] text-slate-500">{r ? t("mindmap.nodeCount", { count: r.nodeCount }) : `${d.nodes.length}`}</span>
              <span className={`shrink-0 rounded px-1 py-px text-[8px] ${ok ? "bg-emerald-500/15 text-emerald-300" : "bg-amber-500/15 text-amber-300"}`}>
                {ok ? t("mindmap.validationOk") : t("mindmap.validationError")}
              </span>
            </div>
          );
        })}
        <div className="flex items-center gap-2 pt-0.5 font-mono text-[8px] text-slate-500">
          <span>{t("mindmap.nodeCount", { count: totalNodes })}</span>
          {result.failures.length > 0 && (
            <span className="text-amber-300/80">{t("mindmap.viewFailures", { count: result.failures.length, names: result.failures.map((f) => f.view).join("、") })}</span>
          )}
        </div>
      </div>
      <div className="border-t border-emerald-400/15 px-2.5 py-1.5">
        <button type="button" onClick={onShowReport} className="cursor-pointer text-micro font-medium text-cyan-300 transition hover:text-cyan-200">
          {t("agent.viewReport")} →
        </button>
      </div>
    </div>
  );
}
