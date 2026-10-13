// 思考过程（reasoning）展示块：所有模块共用同一个组件。
//
// 为什么要有它：推理型模型的「思考内容」以前各处各写各的——数据库设计器显示字数、
// 思维导图显示尾部 1200 字预览、收藏检索与安装助手干脆被前端过滤掉。
// 于是同一个东西有四种不一致的呈现，而它恰恰是排查「AI 为什么给出这个结果」的唯一线索。
//
// 约定：
// - 默认折叠：长思考（几千字）会把结果正文挤到屏幕外，正文才是用户要的东西。
// - 流式过程中折叠态也显示尾部实时滚动（tail 参数），展开时给全文（text 参数）。
// - 「思考中」与「思考完成」是两种状态：前者说明还在想，后者说明想完了。
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Brain, Check, ChevronRight, Copy } from "lucide-react";

/** 尾部预览的字符数：够看出「正在想什么」，又不至于把面板撑开 */
const TAIL_CHARS = 160;

function tailOf(text: string, n: number): string {
  const chars = [...text];
  if (chars.length <= n) return text;
  return chars.slice(chars.length - n).join("");
}

export interface ThinkingBlockProps {
  /** 到目前为止的完整思考文本 */
  text: string;
  /** 是否仍在思考中（流式未结束） */
  pending?: boolean;
  /** 折叠态下的尾部预览；不传则按 TAIL_CHARS 自己截 */
  tail?: string;
  /** 紧凑模式：只有一行，不给展开（适合嵌在步骤列表里） */
  inline?: boolean;
  className?: string;
}

export function ThinkingBlock({
  text,
  pending = false,
  tail,
  inline = false,
  className = "",
}: ThinkingBlockProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);

  const chars = useMemo(() => [...text].length, [text]);
  const preview = useMemo(() => tail ?? tailOf(text, TAIL_CHARS), [tail, text]);

  const copy = () => {
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  };

  const title = pending
    ? t("thinking.inProgress", { count: chars })
    : t("thinking.done", { count: chars });

  if (inline) {
    return (
      <div className={`flex items-start gap-1.5 text-tiny text-slate-400 ${className}`}>
        <Brain className="w-3 h-3 flex-shrink-0 mt-0.5 text-slate-500" />
        <div className="min-w-0 flex-1">
          <span className="text-slate-500">{title}</span>
          <p className="text-slate-400 leading-relaxed whitespace-pre-wrap break-words">
            {preview || t("thinking.waiting")}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className={`rounded-ctl border border-white/10 bg-white/[0.02] ${className}`}>
      <div className="flex items-center gap-1.5 px-2 py-1">
        <button
          onClick={() => setOpen((v) => !v)}
          className="flex items-center gap-1.5 min-w-0 flex-1 text-left cursor-pointer group"
        >
          <ChevronRight
            className={`w-3 h-3 flex-shrink-0 text-slate-500 transition-transform ${open ? "rotate-90" : ""}`}
          />
          <Brain
            className={`w-3 h-3 flex-shrink-0 ${pending ? "text-[var(--module-accent)] animate-pulse" : "text-slate-500"}`}
          />
          <span className="text-tiny text-slate-400 truncate group-hover:text-slate-200">
            {title}
          </span>
        </button>
        {open && (
          <button
            onClick={copy}
            className="flex-shrink-0 p-0.5 rounded text-slate-500 hover:text-slate-200 cursor-pointer"
            title={copied ? t("common.copied") : t("common.copy")}
          >
            {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
          </button>
        )}
      </div>

      {/* 折叠时也显示尾部：流式思考要能看到「正在想什么」，不然折叠就是个黑盒 */}
      {!open && preview && (
        <div className="px-2 pb-1.5 pl-6 text-tiny text-slate-500 leading-relaxed line-clamp-2 whitespace-pre-wrap break-words">
          {preview}
        </div>
      )}

      {open && (
        <div className="px-2 pb-2 pl-6 max-h-64 overflow-y-auto">
          <p className="text-tiny text-slate-300 leading-relaxed whitespace-pre-wrap break-words">
            {text || t("thinking.waiting")}
          </p>
        </div>
      )}
    </div>
  );
}
