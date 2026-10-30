import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { MarkdownRenderer } from "../shared/markdown/MarkdownRenderer";
import { Quote, Loader2, Bot, Cpu, User as UserIcon, Zap, Check, Copy } from "lucide-react";
import type { CollabMessage, DetectedAiTool } from "../../contracts/ai";
import { ERROR_KEYS, THINKING_KEYS, formatDuration, getToolAvatar, getToolNickname, pickRandom, senderColor, useRotatingPhrase } from "./CollabRoom.model";
import { ReferenceCard } from "./CollabRoom.ReferenceCard";
import { FileBadge } from "./CollabRoom.FileBadge";
import { ToolActivityIcon } from "./CollabRoom.ToolActivityIcon";
import { PromptResponse } from "./CollabRoom.PromptResponse";
export function MessageView({
  m,
  tools,
  onQuote,
  activity,
  prompt,
  proxy,
  onRespond,
}: {
  m: CollabMessage;
  tools: DetectedAiTool[];
  onQuote: (m: CollabMessage) => void;
  activity?: string;
  prompt?: { question: string; options: string[] };
  proxy?: { text: string; status: string; elapsed?: number };
  onRespond: (response: string) => void;
}) {
  const isUser = m.sender === "user";
  const color = senderColor(m.sender);
  const running = m.status === "running";
  const isError = m.status === "error";
  const isTool = !isUser;

  // 轮换思维短语（仅 running 状态时轮换）
  const { t } = useTranslation();
  const thinkingPhrase = useRotatingPhrase(THINKING_KEYS.map((k) => t(`collab.${k}`)), running && !m.content, 4000);
  // 错误短语（仅 error 状态时使用，不轮换）
  const errorPhrase = useMemo(() => {
    const key = pickRandom(ERROR_KEYS);
    return t(`collab.${key}`);
  }, [m.id, isError, t]);

  // 头像：消息携带 > 工具配置 > null
  const avatar = isTool ? getToolAvatar(tools, m.sender, m.avatar) : null;
  // 昵称：消息携带(sender_name 已含 nickname) > 工具配置 > sender_name
  const displayName = isTool
    ? (getToolNickname(tools, m.sender, m.sender_name) ?? m.sender_name)
    : m.sender_name;

  const [copied, setCopied] = useState(false);

  const handleCopyMessage = async () => {
    if (!m.content) return;
    try {
      await navigator.clipboard.writeText(m.content);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      const textArea = document.createElement("textarea");
      textArea.value = m.content;
      document.body.appendChild(textArea);
      textArea.select();
      document.execCommand("copy");
      document.body.removeChild(textArea);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <div className="flex gap-2.5">
      {/* 头像 */}
      <div
        className={`w-7 h-7 rounded-full flex items-center justify-center text-white text-caption font-bold flex-shrink-0 ${color}`}
      >
        {isUser ? (
          <UserIcon className="w-3.5 h-3.5" />
        ) : avatar ? (
          <span className="text-[14px] leading-none">{avatar}</span>
        ) : (
          <Bot className="w-3.5 h-3.5" />
        )}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-0.5">
          <span className="text-caption font-semibold text-slate-200">
            {displayName}
          </span>
          <span className="text-[8px] text-slate-500">{m.created_at}</span>
          {m.dispatch && (
            <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] text-[8px] font-medium flex items-center gap-1">
              <Cpu className="w-2.5 h-2.5" />
              {m.dispatch.model || m.dispatch.tool_id}
              {m.dispatch.duration_ms != null && (
                <span className="text-[color-mix(in_srgb,var(--module-accent)_70%,transparent)]">· {formatDuration(m.dispatch.duration_ms)}</span>
              )}
              {m.dispatch.usage && (
                <span className="text-[color-mix(in_srgb,var(--module-accent)_70%,transparent)]">
                  · {(m.dispatch.usage.input_tokens + m.dispatch.usage.output_tokens).toLocaleString()} tokens
                </span>
              )}
            </span>
          )}
        </div>

        {/* 引用卡 */}
        {m.references.length > 0 && (
          <div className="mb-1 space-y-1">
            {m.references.map((r, i) => (
              <ReferenceCard
                key={i}
                senderName={r.source_sender_name}
                excerpt={r.excerpt}
                variant="message"
              />
            ))}
          </div>
        )}

        {/* 文件附件 */}
        {m.files.length > 0 && (
          <div className="mb-1 flex flex-wrap gap-1">
            {m.files.map((f, i) => (
              <FileBadge key={i} path={f.path} color="cyan" />
            ))}
          </div>
        )}

        {/* 内容 */}
        <div
          className={`rounded-ctl px-3 py-2 text-caption leading-relaxed break-words ${
            isUser
              ? "bg-blue-500/10 border border-blue-500/20 whitespace-pre-wrap"
              : running
              ? "bg-slate-800/40 border border-white/5 text-slate-400"
              : isError
              ? "bg-red-500/5 border border-red-500/20"
              : "bg-slate-800/60 border border-white/5"
          }`}
        >
          {running && !m.content ? (
            <span className="flex items-center gap-1.5 text-slate-400">
              <Loader2 className="w-3 h-3 animate-spin" /> {thinkingPhrase}
            </span>
          ) : isUser ? (
            m.content || <span className="text-slate-600">{t("collab.emptyContent")}</span>
          ) : isError ? (
            <div className="flex items-center gap-1.5 text-red-400">
              <span className="text-tiny">{t("collab.errPrefix", { phrase: errorPhrase })}</span>
              <span className="flex-1">
                <MarkdownRenderer content={m.content} />
              </span>
            </div>
          ) : (
            <MarkdownRenderer content={m.content} />
          )}
        </div>

        {/* 活动状态指示器：思考中/调用工具等实时状态 */}
        {running && activity && (
          <div className="mt-1.5 flex items-center gap-1.5 text-micro text-slate-400">
            <span className="relative flex h-2 w-2">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-[var(--module-accent)] opacity-60"></span>
              <span className="relative inline-flex rounded-full h-2 w-2 bg-[var(--module-accent)]"></span>
            </span>
            <span className="animate-pulse flex items-center gap-1">
              <ToolActivityIcon activity={activity} />
              {activity}
            </span>
          </div>
        )}

        {/* 代理层实时状态：当 stdout 无输出但代理已收到响应时显示 */}
        {running && !m.content && proxy && proxy.status !== "complete" && (
          <div className="mt-1.5 rounded-md border border-cyan-500/20 bg-cyan-500/5 px-2 py-1 text-micro text-cyan-300/80">
            <span className="flex items-center gap-1">
              <span className="relative flex h-1.5 w-1.5">
                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-cyan-400 opacity-60"></span>
                <span className="relative inline-flex rounded-full h-1.5 w-1.5 bg-cyan-500"></span>
              </span>
              {t("collab.proxyStatus", { status: proxy.status })}
            </span>
          </div>
        )}
        {running && !m.content && proxy?.status === "complete" && proxy.text && (
          <div className="mt-1.5 rounded-md border border-emerald-500/20 bg-emerald-500/5 px-2 py-1.5 text-micro text-emerald-300/80">
            <div className="flex items-center gap-1 mb-1">
              <Zap className="w-2.5 h-2.5" />
              <span>{t("collab.proxyReceived", { count: proxy.text.length, elapsed: proxy.elapsed ? t("collab.elapsedMs", { ms: proxy.elapsed }) : "" })}</span>
            </div>
            <div className="text-[8px] text-slate-400 max-h-24 overflow-y-auto whitespace-pre-wrap break-words">
              {proxy.text.slice(0, 200)}{proxy.text.length > 200 ? "…" : ""}
            </div>
          </div>
        )}

        {/* 有内容后仍在运行时显示当前活动（工具调用等） */}
        {running && m.content && activity && (
          <div className="mt-1 flex items-center gap-1 text-[8px] text-slate-500">
            <Loader2 className="w-2.5 h-2.5 animate-spin" />
            <ToolActivityIcon activity={activity} />
            <span>{activity}</span>
          </div>
        )}

        {/* 交互式询问：工具需要用户做出选择 */}
        {prompt && <PromptResponse prompt={prompt} onRespond={onRespond} />}

        {/* 回链 + 引用 + 复制按钮 */}
        <div className="flex items-center gap-3 mt-1">
          <button
            onClick={() => onQuote(m)}
            className="flex items-center gap-1 text-[8px] text-slate-500 hover:text-[var(--module-accent)] cursor-pointer transition-colors"
          >
            <Quote className="w-3 h-3" /> {t("collab.quote")}
          </button>
          <button
            onClick={handleCopyMessage}
            disabled={!m.content}
            className={`flex items-center gap-1 text-[8px] transition-colors cursor-pointer ${
              copied ? "text-emerald-400 font-medium" : "text-slate-500 hover:text-slate-300"
            }`}
            title={t("collab.copyMsg")}
          >
            {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
            {copied ? t("collab.copied") : t("collab.copy")}
          </button>
        </div>
      </div>
    </div>
  );
}
