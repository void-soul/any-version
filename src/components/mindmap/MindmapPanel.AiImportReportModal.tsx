import { useState } from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import "@xyflow/react/dist/style.css";
import { BarChart3, Sparkles, X, ChevronDown, ChevronRight, Search } from "lucide-react";
import { AiImportResult } from "./types";
import { fmtNum, viewLabel } from "./agentShared";
// ════════════ AI 导入校验报告 ════════════

/** 导入完成弹窗：逐视图展示节点数、修复轮数与残留校验诊断；点击条目跳转到对应文档 */
export function AiImportReportModal({ result, onClose, onOpenDoc }: {
  result: AiImportResult;
  onClose: () => void;
  onOpenDoc: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [expOpen, setExpOpen] = useState(false);
  const allOk = result.reports.length > 0 && result.reports.every(r => r.diagnostics.length === 0) && result.failures.length === 0;
  return createPortal(
    <div className="fixed inset-0 z-[410] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]" onClick={onClose}>
      <div className="w-[min(94vw,560px)] rounded-card border border-white/10 bg-surface-panel p-4 shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex items-center justify-between">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-white"><Sparkles className="h-4 w-4 text-cyan-400" />{t("mindmap.aiReportTitle")}</h3>
          <button type="button" className="cursor-pointer rounded p-1 text-slate-400 hover:text-white" onClick={onClose} title={t("mindmap.close")}><X className="h-4 w-4" /></button>
        </div>
        {/* 本次运行总消耗（含路由/探索阶段，来自后端累计） */}
        {result.usage && (result.usage.requests > 0 || result.usage.totalTokens > 0) && (
          <div className="mb-2 flex items-center gap-2 rounded-ctl border border-white/10 bg-white/[0.03] px-3 py-1.5 text-micro">
            <BarChart3 className="h-3 w-3 shrink-0 text-emerald-300" />
            <span className="shrink-0 font-semibold text-slate-300">{t("mindmap.aiRunUsage")}</span>
            <span className="font-mono tabular-nums text-slate-400">{t("mindmap.aiRunUsageLine", { req: result.usage.requests, in: fmtNum(result.usage.inputTokens), out: fmtNum(result.usage.outputTokens), total: fmtNum(result.usage.totalTokens) })}</span>
          </div>
        )}
        {/* 项目探索过程：每轮 AI 点单的理由 + 实际读取的文件清单（可折叠） */}
        {result.exploration && result.exploration.length > 0 && (() => {
          const expFiles = result.exploration.reduce((n, r) => n + r.files.length, 0);
          return (
            <div className="mb-2 rounded-ctl border border-white/10 bg-white/[0.03]">
              <button type="button"
                onClick={() => setExpOpen(v => !v)}
                className="flex w-full cursor-pointer items-center justify-between gap-2 px-3 py-1.5 text-left">
                <span className="flex items-center gap-1.5 text-tiny font-semibold text-slate-300">
                  <Search className="h-3 w-3 text-cyan-300" />
                  {t("mindmap.aiExploreTitle", { rounds: result.exploration.length, files: expFiles })}
                </span>
                {expOpen ? <ChevronDown className="h-3 w-3 shrink-0 text-slate-400" /> : <ChevronRight className="h-3 w-3 shrink-0 text-slate-400" />}
              </button>
              {expOpen && (
                <div className="space-y-1.5 border-t border-white/5 px-3 py-2">
                  {result.exploration.map(r => (
                    <div key={r.round}>
                      <div className="text-micro text-slate-400">
                        <span className="mr-1 rounded border border-cyan-400/30 bg-cyan-400/10 px-1 py-px font-mono text-cyan-300">#{r.round}</span>
                        {r.reason || t("mindmap.aiExploreNoReason")}
                      </div>
                      {(r.files.length > 0 || r.dirs.length > 0) && (
                        <div className="mt-0.5 flex flex-wrap gap-1">
                          {r.files.map(f => (
                            <span key={f} className="rounded bg-white/[0.05] px-1 py-px font-mono text-[8px] text-slate-400" title={f}>{f}</span>
                          ))}
                          {r.dirs.map(d => (
                            <span key={d} className="rounded bg-white/[0.05] px-1 py-px font-mono text-[8px] text-slate-500" title={d}>{d}/</span>
                          ))}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })()}
        <div className="max-h-[60vh] space-y-2 overflow-y-auto pr-1">
          {result.reports.length === 0 && (
            <p className="py-4 text-center text-tiny text-slate-500">{t("mindmap.noViewsGenerated")}</p>
          )}
          {result.reports.map(r => {
            const doc = result.documents.find(d => d.document.id === r.documentId);
            const ok = r.diagnostics.length === 0;
            return (
              <button key={r.documentId} type="button"
                onClick={() => onOpenDoc(r.documentId)}
                className="block w-full cursor-pointer rounded-ctl border border-white/10 bg-white/[0.03] px-3 py-2 text-left transition hover:bg-white/[0.08]"
                title={t("mindmap.openDoc")}>
                <div className="flex items-center justify-between gap-2">
                  <span className="flex min-w-0 items-center gap-1.5 text-caption font-semibold text-slate-200">
                    <span className="shrink-0 rounded border border-cyan-400/40 bg-cyan-400/10 px-1.5 py-0.5 text-micro text-cyan-300">{viewLabel(t, r.view)}</span>
                    <span className="truncate">{doc?.document.name ?? t("mindmap.docPlaceholder")}</span>
                  </span>
                  <span className={`shrink-0 rounded px-1.5 py-0.5 text-micro ${ok ? "bg-emerald-500/15 text-emerald-300" : "bg-amber-500/15 text-amber-300"}`}>{ok ? t("mindmap.validationOk") : t("mindmap.validationError")}</span>
                </div>
                <div className="mt-1 text-micro text-slate-500">
                  {t("mindmap.nodeCount", { count: r.nodeCount })} · {r.repairRounds === 1 ? t("mindmap.repairFirstPass") : t("mindmap.repairRound", { count: r.repairRounds })}
                  {(r.usage?.requests ?? 0) > 0 && (
                    <span className="ml-1 font-mono text-emerald-300/80" title={t("mindmap.aiRunUsage")}>⚡ {fmtNum(r.usage?.totalTokens ?? 0)} tok</span>
                  )}
                  {(doc?.document.aiImports ?? 0) > 0 && (
                    <span className="ml-1 text-slate-600">{t("mindmap.aiDocUsage", { imports: doc!.document.aiImports, tokens: fmtNum((doc!.document.aiInputTokens ?? 0) + (doc!.document.aiOutputTokens ?? 0)) })}</span>
                  )}
                </div>
                <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-micro">
                  <span className="text-slate-500">{t("mindmap.evidenceStats", { nodes: r.evidenceNodes, count: r.evidenceCount })}</span>
                  {r.nodeCount - r.evidenceNodes > 0 && (
                    <span className="text-slate-600">{t("mindmap.aiInferred", { count: r.nodeCount - r.evidenceNodes })}</span>
                  )}
                  {r.evidenceVerified && r.evidenceCount > 0 && (
                    <span className={r.evidenceHitCount === r.evidenceCount ? "text-emerald-300" : "text-amber-300"}>
                      {t("mindmap.evidenceHit", { hit: r.evidenceHitCount, total: r.evidenceCount, pct: Math.round((r.evidenceHitCount / r.evidenceCount) * 100) })}
                    </span>
                  )}
                </div>
                {r.diagnostics.length > 0 && (
                  <div className="mt-1.5 rounded border border-red-500/20 bg-red-500/10 px-2 py-1 text-micro leading-relaxed text-red-300">{r.diagnostics.join("；")}</div>
                )}
              </button>
            );
          })}
          {result.failures.length > 0 && (
            <div className="rounded-ctl border border-amber-500/20 bg-amber-500/[0.06] px-3 py-2">
              <div className="text-tiny font-semibold text-amber-300">{t("mindmap.failedViews")}</div>
              {result.failures.map((f, i) => (
                <div key={i} className="mt-1 text-micro leading-relaxed text-amber-200/80">「{viewLabel(t, f.view)}」：{f.reason}</div>
              ))}
            </div>
          )}
        </div>
        <div className="mt-3 flex items-center justify-between gap-2">
          <span className={`text-micro ${allOk ? "text-emerald-300" : "text-slate-500"}`}>{allOk ? t("mindmap.allViewsOk") : t("mindmap.someViewsError")}</span>
          <button type="button" className="cursor-pointer rounded bg-cyan-500 px-3 py-1.5 text-tiny font-semibold text-slate-950 hover:bg-cyan-400" onClick={onClose}>{t("mindmap.closeBtn")}</button>
        </div>
      </div>
    </div>, document.body);
}
