import { File, Terminal } from "lucide-react";
import { ThinkingBlock } from "../shared/ThinkingBlock";
import { STEP_ICONS, fmtClock, openSourceFile, progressText, type AiProgressEntry } from "./agentShared.state";
/** 单条工具调用/进度行：图标 + 文案 + 可选文件 chips（可点击打开） */
export function ActivityLine({ e, projectRoot, t }: { e: AiProgressEntry; projectRoot?: string | null; t: (k: string, o?: any) => string }) {
  const files = e.step === "read" || e.step === "explore" ? e.files ?? [] : [];
  return (
    <div className="flex items-start gap-1.5">
      <span className="mt-0.5 shrink-0 font-mono text-[7px] text-slate-600">{e.at ? fmtClock(e.at) : ""}</span>
      <span className="mt-0.5 shrink-0">{STEP_ICONS[e.step] ?? <Terminal className="h-3 w-3 text-slate-400" />}</span>
      <div className="min-w-0 flex-1">
        <div className="text-micro leading-4 text-slate-300">
          {e.step === "read" && (e.files?.length ?? 0) > 0 ? t("mindmap.aiStepReadFiles", { count: (e.files ?? []).length }) : progressText(e, t)}
        </div>
        {files.length > 0 && (
          <div className="mt-0.5 flex flex-wrap gap-0.5">
            {files.map((f) => (
              <button key={f} type="button" disabled={!projectRoot}
                onClick={(ev) => { ev.stopPropagation(); if (projectRoot) openSourceFile(projectRoot, f); }}
                className={`nodrag nopan inline-flex max-w-[160px] items-center gap-0.5 truncate rounded border border-white/10 bg-white/[0.04] px-1 py-px font-mono text-[8px] ${projectRoot ? "cursor-pointer text-slate-300 transition hover:border-cyan-400/50 hover:text-cyan-200" : "text-slate-500"}`}
                title={projectRoot ? f : ""}>
                <File className="h-2 w-2 shrink-0" /><span className="truncate">{f.split(/[\\/]/).pop()}</span>
              </button>
            ))}
          </div>
        )}
        {e.step === "reject" && (e.paths?.length ?? 0) > 0 && (
          <div className="mt-0.5 flex flex-wrap gap-0.5">
            {e.paths!.map((p) => (
              <span key={p} className="max-w-[180px] truncate rounded border border-yellow-400/20 bg-yellow-400/5 px-1 py-px font-mono text-[8px] text-yellow-300/80" title={p}>{p}</span>
            ))}
          </div>
        )}
        {/* 思考过程：模型调用工具前的想法 / 模型产出的推理文本 / 流式思考帧。
            原来是这里自己写的一块定高 96px 的预览框，超长就看不全；
            换成共享 ThinkingBlock（可折叠、全文可滚动、可复制）。 */}
        {(e.step === "agentThought" || e.step === "agentReasoning" || e.step === "reasoning") && !!e.text && (
          <div className="mt-0.5">
            <ThinkingBlock text={e.text} pending={!e.done} />
          </div>
        )}
        {/* 工具参数：折叠展示，避免长 JSON 把时间线撑爆 */}
        {e.step === "agentTool" && !!e.args && e.args !== "{}" && (
          <div className="mt-0.5 max-h-16 overflow-y-auto whitespace-pre-wrap break-all rounded border border-cyan-400/15 bg-cyan-400/[0.03] px-1.5 py-1 font-mono text-[8px] leading-3.5 text-cyan-100/70" title={e.args}>{e.args}</div>
        )}
      </div>
    </div>
  );
}
