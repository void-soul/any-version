import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { Brain, Maximize2, Square, MessageCircle } from "lucide-react";
import { useEventBufferSnapshot } from "../../utils/eventBuffer";
import { mmAiProgressBuffer, progressText, fmtDur, useAnsweredAsks } from "./agentShared";
// ════════════ AI 智能体（进度缓冲/事件渲染/工作台见 agentShared.tsx） ════════════

/** AI 弹窗最小化后的画布悬浮胶囊：显示后台任务仍在进行（最新进度 + 用时），点击恢复弹窗。
 *  由画布右上角统一列布局承载（与节点树/浮动工具栏垂直排布，互不重叠）。 */
export function AiRunningPill({ onRestore, onStop, running }: { onRestore: () => void; onStop: () => void; running: boolean }) {
  const { t } = useTranslation();
  const entries = useEventBufferSnapshot(mmAiProgressBuffer);
  const answeredAsks = useAnsweredAsks();
  const [now, setNow] = useState(() => Date.now());
  // 秒表：进度事件稀疏（长视图生成中）时用时也要跳动
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, []);
  const last = entries[entries.length - 1];
  const elapsed = entries.length ? now - (entries[0].at ?? now) : 0;
  // AI 正在等待用户回答询问（最后一条是未回答的 ask）→ 胶囊醒目标记，
  // 提示用户恢复弹窗填写表单，而不是误以为还在后台计算。
  const waitingForAnswer = running && Boolean(last && last.step === "ask" && !answeredAsks.has(last.at ?? 0));
  return (
    <div className={`flex items-center gap-2 rounded-full border bg-slate-900/90 px-3 py-1.5 shadow-xl backdrop-blur-sm ${waitingForAnswer ? "border-amber-400/50 ring-1 ring-amber-400/30" : running ? "border-cyan-400/25" : "border-white/15"}`}>
      {waitingForAnswer
        ? <MessageCircle className="h-3.5 w-3.5 shrink-0 animate-pulse text-amber-300" />
        : <Brain className={`h-3.5 w-3.5 shrink-0 ${running ? "animate-pulse text-cyan-300" : "text-slate-400"}`} />}
      <div className="min-w-0 text-left">
        <div className={`text-tiny font-semibold leading-tight ${waitingForAnswer ? "text-amber-200" : "text-white"}`}>
          {waitingForAnswer ? t("aiMinimized.waiting") : running ? t("aiMinimized.running") : t("aiMinimized.minimized")}
        </div>
        {/* 未运行时不再显示「用时」：那是只对在跑任务有意义的读数，
            这里要说的只有「草稿还在，点一下就能继续」 */}
        {running ? (
          <div className="max-w-[240px] truncate text-micro leading-tight text-slate-400" title={last ? progressText(last, t) : ""}>
            {last ? progressText(last, t) : "…"} · {t("aiMinimized.elapsed")} {fmtDur(elapsed)}
          </div>
        ) : (
          <div className="max-w-[240px] truncate text-micro leading-tight text-slate-400">{t("aiMinimized.minimizedHint")}</div>
        )}
      </div>
      <button type="button" onClick={onRestore} title={t("aiMinimized.restore")}
        className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-slate-300 transition hover:bg-white/10 hover:text-white">
        <Maximize2 className="h-3 w-3" />
      </button>
      {/* 停止按钮只在真在跑时出现：没在跑却摆一个停止键，只会让人以为「隐藏=关掉了」 */}
      {running && (
        <button type="button" onClick={onStop} title={t("mindmap.aiStop")}
          className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-red-300/80 transition hover:bg-red-400/10 hover:text-red-300">
          <Square className="h-2.5 w-2.5 fill-current" />
        </button>
      )}
    </div>
  );
}
