// AI 助手侧栏：**窄条 + 展开**两态，与左侧表树同款（不用 translate-x 滑出，
// 思维导图那边也是宽度切换 —— 项目里没有滑出动画，滑出会与整体手感不一致）。
//
// 对话按**气泡**渲染，用户右对齐、助手左对齐；进度（思考中 / 正在写 / 已完成）
// 收进当轮的助手气泡，不再单独占一块常驻区域 —— 常驻的进度条会把「读对话」这件事压扁。
import { useEffect, useRef } from "react";
import { Ban, ChevronDown, HelpCircle, Lightbulb, Loader2, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ThinkingBlock } from "../shared/ThinkingBlock";
import { ModelSelector, type ProviderLike } from "../ai/ModelSelector";
import SessionBar from "./SessionBar";
import type { ChatMsg } from "./chatState";
import type { SessionEntry } from "./sessionKey";

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
  /** 提示词草稿。父组件持有 —— 发送后清空、追问后重置都在那边做 */
  text: string;
  onTextChange: (v: string) => void;
  /** 发起一轮生成。清空与发起由父组件合成一次，这里只管触发 */
  onSubmit: () => void;
  /** 中止本轮生成 */
  onCancel: () => void;
  /** 一个可用供应商都没有：提交禁用，并提示先去 AI 模块配置 */
  noProvider: boolean;
  /**
   * 当前用哪个供应商 + 模型，以及切换回调。
   *
   * 这一组 prop 是**从 Q-0421 的面板重写里丢过一次又找回来的**：
   * 它连同 `dbd_save_settings` 的写入一起消失，导致用户既选不了模型、
   * 上次选的也不再被记住。详见 `modelChoice.ts` 顶部。
   */
  providers: ProviderLike[];
  providerId: string;
  modelId: string;
  onModelSelect: (providerId: string, modelId: string) => void;
  /** ── 会话（见 sessionKey.ts）── */
  sessionId: string;
  sessionPath: string;
  sessionDocName: string;
  sessions: SessionEntry[];
  onSessionSelect: (id: string) => void;
  onSessionCreate: () => void;
  onSessionDelete: (id: string) => void;
  /**
   * 「读盘中」：会话正从磁盘读回来。这段时间 `sessionId` 与 `aiHistory`
   * 还是上一份的，此时回车就是把「新表 + 旧对话」发给模型并存进旧会话 ——
   * **串台换个入口复活**。所以发送按钮禁用。
   */
  chatLoading: boolean;
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
          {/* 关闭：写真正的 ✕（U+2715）字符。曾写成字面量 "\u2715"，
              JSX 文本节点不解析转义 → 界面上显示的是 "u2715" 这六个字符。 */}
          <button onClick={props.onClose} className="ui-btn shrink-0 p-1" title={t("common.dialogClose")}>
            ✕
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

        {/* 输入区：贴底固定，不跟着气泡滚动。
            **这里是发起一轮的唯一入口** —— 顶部按钮已撤掉，侧栏收起时靠画布胶囊恢复，
            但输入提示词只能在这儿打，所以这一块不能省。 */}
        <div className="flex flex-shrink-0 flex-col gap-2 border-t border-white/5 p-3">
          {/* 会话与模型**都必须在发请求之前就能改**：选错模型 / 看错会话的代价都是整轮重跑。
              会话条在上、模型选择器在下 —— 「用什么模型、聊哪份设计」是同一个问题的两面。 */}
          <SessionBar
            currentId={props.sessionId}
            currentPath={props.sessionPath}
            currentTitle={props.sessionDocName}
            sessions={props.sessions}
            busy={busy}
            onSelect={props.onSessionSelect}
            onCreate={props.onSessionCreate}
            onDelete={props.onSessionDelete}
          />
          {/* 哪张表会用什么模型生成，**必须在发请求之前就能改** —— 生成一轮几十秒，
              事后才发现选错了就得整轮重跑。所以选择器贴在输入框上，而不是收进设置里。
              用 AI 模块的共享组件（与思维导图 Agent、API 智能导入同一套 UI）。 */}
          <div className="flex items-center gap-2">
            <span className="shrink-0 text-micro text-slate-500">{t("dbd.aiModel")}</span>
            <div className="min-w-0 flex-1">
              <ModelSelector
                providers={props.providers}
                providerId={props.providerId}
                modelId={props.modelId}
                onSelect={props.onModelSelect}
                // 生成中不给换：这一轮已经用旧模型发出去了，此刻切换只会在下一次生效，
                // 让人以为「切换对这一轮也有效」反而更容易用错模型重跑。
                disabled={busy}
                compact
                placeholder={t("dbd.aiModel")}
              />
            </div>
          </div>
          <textarea
            value={props.text}
            onChange={(e) => props.onTextChange(e.target.value)}
            rows={4}
            placeholder={t("dbd.aiPh")}
            className="w-full resize-none rounded-ctl bg-black/30 px-2 py-1.5 text-caption text-slate-200 outline-none placeholder:text-slate-600"
          />
          <div className="flex items-center gap-2">
            <button
              onClick={props.onSubmit}
              disabled={busy || props.chatLoading || !props.text.trim() || props.noProvider}
              title={props.chatLoading ? t("dbd.sessionLoading") : undefined}
              className="ui-btn ui-btn-primary px-3 py-1 text-caption disabled:opacity-40"
            >
              {busy ? t("dbd.aiBusy") : props.chatLoading ? t("dbd.sessionLoading") : t("dbd.aiSubmit")}
            </button>
            {busy ? (
              <button onClick={props.onCancel} className="ui-btn px-2 py-1 text-caption">
                <Ban className="h-3 w-3" /> {t("dbd.aiCancel")}
              </button>
            ) : null}
            {!busy && props.noProvider ? (
              <span className="min-w-0 truncate text-micro text-amber-400">{t("dbd.aiNoProvider")}</span>
            ) : null}
          </div>
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
