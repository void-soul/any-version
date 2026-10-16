// AI 助手侧栏：**窄条 + 展开**两态，与左侧表树同款（不用 translate-x 滑出，
// 思维导图那边也是宽度切换 —— 项目里没有滑出动画，滑出会与整体手感不一致）。
//
// 对话按**气泡**渲染，用户右对齐、助手左对齐；进度（思考中 / 正在写 / 已完成）
// 收进当轮的助手气泡，不再单独占一块常驻区域 —— 常驻的进度条会把「读对话」这件事压扁。
import { useEffect, useRef } from "react";
import { ChevronDown, HelpCircle, Lightbulb, Loader2, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ThinkingBlock } from "../shared/ThinkingBlock";
import type { ChatMsg } from "./chatState";

/** \u6536\u8d77\u65f6\u7684\u7a84\u6761\u5bbd\u5ea6\uff0c\u4e0e\u5de6\u4fa7\u8868\u6811\u7684 w-8 \u5bf9\u9f50 */
export const AI_RAIL_W = 36;

interface Props {
  open: boolean;
  busy: boolean;
  width: number;
  messages: ChatMsg[];
  /** \u6587\u6863\u7ea7\u51b3\u7b56\u8bb0\u5f55\uff08\u4e0d\u662f\u67d0\u4e00\u8f6e\u7684\uff09\uff0c\u8d70\u9876\u90e8\u6309\u94ae\u5c55\u5f00 */
  decisions: string[];
  decisionsOpen: boolean;
  onToggleDecisions: () => void;
  onRemoveDecision: (text: string) => void;
  onDecisionDraft: (text: string) => void;
  decisionDraft: string;
  onClose: () => void;
  onResizeStart: (e: React.MouseEvent) => void;
  onAnswer: (answer: string) => void;
  onOpen: () => void;
}

export default function AiSidebar(props: Props) {
  const { t } = useTranslation();
  const { open, busy, messages } = props;
  const scrollRef = useRef<HTMLDivElement>(null);

  // \u65b0\u6d88\u606f / \u8fdb\u5ea6\u53d8\u5316\u65f6\u6eda\u5230\u5e95\uff1a\u6c14\u6ce1\u5bf9\u8bdd\u4e0d\u81ea\u52a8\u8ddf\u968f\u7684\u8bdd\uff0c\u6700\u65b0\u90a3\u6761\u4f1a\u5728\u89c6\u53e3\u5916\u770b\u4e0d\u89c1
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages]);

  if (!open) {
    return (
      <aside
        className="relative flex-shrink-0 border-l border-white/10 bg-surface-panel/60"
        style={{ width: AI_RAIL_W }}
      >
        <button
          onClick={props.onOpen}
          title={t("dbd.aiTitle")}
          className="flex h-full w-full cursor-pointer flex-col items-center gap-2 py-2 text-slate-400 transition hover:bg-white/[0.03] hover:text-slate-200"
        >
          <Sparkles className="h-3.5 w-3.5 flex-shrink-0" />
          {/* \u751f\u6210\u4e2d\u5728\u7a84\u6761\u4e0a\u8f6c\u5708\uff1a\u4efb\u52a1\u8fd8\u5728\u8dd1 \u2014\u2014 \u4e0d\u770b\u8fdb\u5ea6\u4f1a\u4ee5\u4e3a\u5361\u6b7b\u4e86 */}
          {busy ? <Loader2 className="h-3 w-3 flex-shrink-0 animate-spin" /> : null}
        </button>
      </aside>
    );
  }


  return (
    <aside
      className="relative flex-shrink-0 border-l border-white/10 bg-surface-panel/60"
      style={{ width: props.width }}
    >
      <div className="flex h-full min-h-0 flex-col">
        {/* \u6807\u9898\u680f */}
        <div className="flex flex-shrink-0 items-center gap-2 border-b border-white/5 px-3 py-2">
          <Sparkles className="h-3.5 w-3.5 flex-shrink-0 text-[var(--module-accent)]" />
          <span className="min-w-0 flex-1 truncate text-caption font-semibold text-slate-200">
            {t("dbd.aiTitle")}
          </span>
          <button onClick={props.onClose} className="ui-btn shrink-0 p-1" title={t("common.dialogClose")}>
            \u2715
          </button>
        </div>

        {/* \u6587\u6863\u7ea7\u51b3\u7b56\u8bb0\u5f55\uff1a**\u9876\u90e8\u4e00\u4e2a\u6309\u94ae**\uff0c\u70b9\u5f00\u624d\u5360\u5730\u65b9\u3002
            \u4e0d\u5e38\u9a7b\u5c55\u793a\u662f\u56e0\u4e3a\u5b83\u662f\u6587\u6863\u7ea7\u7684\u957f\u671f\u8bb0\u5fc6\uff0c\u4e0d\u662f\u5f53\u524d\u8f6e\u7684\u65e5\u5fd7\u2014\u2014
            \u5e38\u9a7b\u7740\u4f1a\u628a\u300c\u8bfb\u5bf9\u8bdd\u300d\u8fd9\u4ef6\u4e8b\u538b\u7f29\u6ca1\u3002 */}
        <div className="shrink-0 border-b border-white/5 px-2 py-1.5">
          <button
            type="button"
            onClick={props.onToggleDecisions}
            className="flex w-full cursor-pointer items-center gap-1.5 rounded px-1.5 py-1 text-left text-micro text-slate-300 transition hover:bg-white/[0.03]"
          >
            <Lightbulb className="h-3 w-3 flex-shrink-0 text-amber-300/80" />
            <span className="min-w-0 flex-1 truncate">{t("dbd.aiDecisions")}</span>
            {props.decisions.length > 0 ? (
              <span className="shrink-0 rounded-full bg-white/10 px-1.5 text-micro text-slate-300">
                {props.decisions.length}
              </span>
            ) : null}
            <ChevronDown
              className={`h-3 w-3 flex-shrink-0 transition-transform ${props.decisionsOpen ? "" : "-rotate-90"}`}
            />
          </button>
          {props.decisionsOpen ? (
            <div className="space-y-1 px-1.5 pb-1 pt-1.5">
              {props.decisions.length === 0 ? (
                <p className="text-micro text-slate-500">{t("dbd.aiDecisionNone")}</p>
              ) : (
                props.decisions.map((d) => (
                  <div key={d} className="group/dec flex items-start gap-1.5">
                    <span className="min-w-0 flex-1 text-micro leading-relaxed text-slate-300">{d}</span>
                    <button
                      type="button"
                      title={t("dbd.aiDecisionDelete")}
                      onClick={() => props.onRemoveDecision(d)}
                      className="shrink-0 cursor-pointer text-slate-600 opacity-0 transition group-hover/dec:opacity-100 hover:text-rose-300"
                    >
                      \u2715
                    </button>
                  </div>
                ))
              )}
              <textarea
                value={props.decisionDraft}
                onChange={(e) => props.onDecisionDraft(e.target.value)}
                rows={2}
                placeholder={t("dbd.aiDecisionAdd")}
                className="w-full resize-none rounded bg-black/30 px-2 py-1 text-micro text-slate-200 outline-none placeholder:text-slate-600"
              />
            </div>
          ) : null}
        </div>

        {/* \u5bf9\u8bdd\u6c14\u6ce1 */}
        <div ref={scrollRef} className="min-h-0 flex-1 space-y-2.5 overflow-y-auto p-3">
          {messages.map((m) => (
            <Bubble key={m.id} msg={m} onAnswer={props.onAnswer} busy={busy} />
          ))}
        </div>
      </div>

      {/* \u5de6\u7f18\u62d6\u5bbd\u628a\u624b\uff1a\u4e0e\u601d\u7ef4\u5bfc\u56fe\u53f3\u680f\u540c\u4e00\u5957\u903b\u8f91\uff08\u62d6\u52a8\u65b9\u5411\u76f8\u53cd\uff09 */}
      <div
        className="absolute -left-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]"
        onMouseDown={props.onResizeStart}
      />
    </aside>
  );
}


/**
 * 一条气泡：用户右对齐、助手左对齐。
 *
 * 进度（思考中 / 正在写 / 自动修复）与结果摘要都收在**这一条**里 ——
 * 早先是面板顶部一整块常驻的进度区，任何一轮生成都在那儿，用户要读对话得先把它划过去。
 */
function Bubble(props: { msg: ChatMsg; onAnswer: (a: string) => void; busy: boolean }) {
  const { t } = useTranslation();
  const { msg, busy } = props;
  const isUser = msg.role === "user";
  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[92%] rounded-ctl px-2.5 py-1.5 text-tiny leading-relaxed ${
          isUser
            ? "bg-[var(--module-accent)] text-white"
            : "border border-white/10 bg-white/[0.03] text-slate-200"
        }`}
      >
        {msg.text ? <div className="whitespace-pre-wrap">{msg.text}</div> : null}

        {/* 追问卡：问题 + 选项 + 自由输入，都在这个气泡里 */}
        {msg.clarify ? (
          <div className="mt-1.5 border-t border-white/10 pt-1.5">
            <div className="flex items-start gap-1.5 font-semibold text-cyan-200">
              <HelpCircle className="mt-px h-3.5 w-3.5 flex-shrink-0" />
              <span>{msg.clarify.question}</span>
            </div>
            {msg.clarify.options.length > 0 ? (
              <div className="mt-1.5 flex flex-wrap gap-1">
                {msg.clarify.options.map((opt) => (
                  <button
                    key={opt}
                    type="button"
                    disabled={busy}
                    onClick={() => props.onAnswer(opt)}
                    className="cursor-pointer rounded border border-cyan-400/30 bg-cyan-400/10 px-1.5 py-0.5 text-micro text-cyan-100 hover:bg-cyan-400/20"
                  >
                    {opt}
                  </button>
                ))}
              </div>
            ) : null}
            <input
              disabled={busy}
              placeholder={t("dbd.aiClarifyPh")}
              onKeyDown={(e) => {
                if (e.key !== "Enter") return;
                const v = e.currentTarget.value.trim();
                if (!v) return;
                e.currentTarget.value = "";
                props.onAnswer(v);
              }}
              className="mt-1.5 w-full rounded bg-black/30 px-2 py-1 text-micro text-slate-200 outline-none placeholder:text-slate-600"
            />
          </div>
        ) : null}

        {/* 思考过程：折叠在气泡里，不占地方 */}
        {msg.thinking ? (
          <details className="mt-1">
            <summary className="cursor-pointer text-micro text-slate-400">{t("dbd.aiThinking")}</summary>
            <ThinkingBlock text={msg.thinking} pending={msg.status === "thinking"} />
          </details>
        ) : null}

        {/* 进行中的进度 */}
        {msg.status === "thinking" || msg.status === "writing" || msg.status === "repair" ? (
          <div className="mt-1 flex items-center gap-1.5 text-micro text-slate-400">
            <Loader2 className="h-3 w-3 animate-spin" />
            {msg.status === "writing"
              ? t("dbd.aiWritingChars", { count: msg.chars ?? 0 })
              : t("dbd.aiThinking")}
          </div>
        ) : null}

        {msg.summary ? <div className="mt-1 text-slate-300">{msg.summary}</div> : null}

        {msg.patch ? (
          <div className="mt-1 text-micro text-slate-400">
            {([
              msg.patch.added > 0 ? t("dbd.aiPatchAdded", { count: msg.patch.added }) : null,
              msg.patch.updated > 0 ? t("dbd.aiPatchUpdated", { count: msg.patch.updated }) : null,
              msg.patch.removed > 0 ? t("dbd.aiPatchRemoved", { count: msg.patch.removed }) : null,
            ]
              .filter(Boolean)
              .join("，") || t("dbd.aiPatchNoop"))}
          </div>
        ) : null}

        {msg.truncated ? (
          <div className="mt-1 text-micro text-amber-400">{t("dbd.aiTruncated", { count: 0 })}</div>
        ) : null}

        {msg.error ? <div className="mt-1 text-micro text-rose-300">{msg.error}</div> : null}

        {msg.decisions && msg.decisions.length > 0 ? (
          <ul className="mt-1.5 space-y-0.5 border-t border-white/10 pt-1.5">
            {msg.decisions.map((d) => (
              <li key={d} className="flex items-start gap-1.5 text-micro text-slate-300">
                <span className="mt-1 h-1 w-1 flex-shrink-0 rounded-full bg-[var(--module-accent)]" />
                <span className="min-w-0 flex-1">{d}</span>
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    </div>
  );
}
