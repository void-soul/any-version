import React, { memo, useState, useCallback } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * 可复用的 Markdown 渲染器，专为 AI 对话场景设计。
 *
 * 特性：
 * - 支持 GFM（表格、删除线、任务列表、自动链接）
 * - 代码块带语言标签 + 一键复制
 * - 暗色主题样式，与 collab / chat UI 一致
 * - 安全：react-markdown 默认不执行 HTML（无需 rehype-raw）
 * - 懒加载：用 memo 避免每次 delta 都重新 parse 全文
 */
function MarkdownRendererBase({ content }: { content: string }) {
  return (
    <div className="md-body text-[11px] leading-relaxed text-slate-200 break-words">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          // 标题
          h1: ({ children }) => (
            <h1 className="text-sm font-bold text-slate-100 mt-3 mb-1.5 first:mt-0">{children}</h1>
          ),
          h2: ({ children }) => (
            <h2 className="text-[13px] font-bold text-slate-100 mt-3 mb-1.5 first:mt-0">{children}</h2>
          ),
          h3: ({ children }) => (
            <h3 className="text-[12px] font-bold text-slate-200 mt-2.5 mb-1 first:mt-0">{children}</h3>
          ),
          h4: ({ children }) => (
            <h4 className="text-[11px] font-bold text-slate-200 mt-2 mb-1 first:mt-0">{children}</h4>
          ),
          h5: ({ children }) => (
            <h5 className="text-[11px] font-semibold text-slate-300 mt-2 mb-1 first:mt-0">{children}</h5>
          ),
          h6: ({ children }) => (
            <h6 className="text-[10px] font-semibold text-slate-400 mt-2 mb-1 first:mt-0">{children}</h6>
          ),
          // 段落
          p: ({ children }) => <p className="my-1.5 first:mt-0 last:mb-0">{children}</p>,
          // 列表
          ul: ({ children }) => <ul className="list-disc list-inside my-1.5 space-y-0.5 pl-1">{children}</ul>,
          ol: ({ children }) => <ol className="list-decimal list-inside my-1.5 space-y-0.5 pl-1">{children}</ol>,
          li: ({ children }) => <li className="leading-relaxed">{children}</li>,
          // 任务列表（GFM）
          input: ({ checked }) => (
            <input
              type="checkbox"
              checked={checked}
              readOnly
              className="mr-1.5 align-middle w-3 h-3 rounded accent-violet-500"
            />
          ),
          // 引用
          blockquote: ({ children }) => (
            <blockquote className="border-l-2 border-slate-600 pl-2.5 my-2 text-slate-400 italic">
              {children}
            </blockquote>
          ),
          // 行内代码
          code: ({ className, children }) => {
            const match = /language-(\w+)/.exec(className || "");
            const isBlock = match !== null;
            if (!isBlock) {
              // 行内代码
              return (
                <code className="px-1 py-0.5 rounded bg-slate-700/60 text-[10px] text-violet-300 font-mono">
                  {children}
                </code>
              );
            }
            return <CodeBlock lang={match[1]}>{children}</CodeBlock>;
          },
          // pre 标签（react-markdown 会把 code 包在 pre 里）
          pre: ({ children }) => <>{children}</>,
          // 链接
          a: ({ href, children }) => (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="text-violet-400 hover:text-violet-300 underline underline-offset-1"
            >
              {children}
            </a>
          ),
          // 表格
          table: ({ children }) => (
            <div className="overflow-x-auto my-2 rounded border border-white/10">
              <table className="min-w-full text-[10px]">{children}</table>
            </div>
          ),
          thead: ({ children }) => <thead className="bg-slate-800/80">{children}</thead>,
          th: ({ children }) => (
            <th className="px-2 py-1 text-left font-semibold text-slate-200 border-b border-white/10">
              {children}
            </th>
          ),
          td: ({ children }) => (
            <td className="px-2 py-1 text-slate-300 border-b border-white/5">{children}</td>
          ),
          // 分割线
          hr: () => <hr className="border-white/10 my-3" />,
          // 强调
          strong: ({ children }) => <strong className="font-bold text-slate-100">{children}</strong>,
          em: ({ children }) => <em className="text-slate-300">{children}</em>,
          // 删除线（GFM）
          del: ({ children }) => <del className="text-slate-500">{children}</del>,
        }}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}

/// 代码块：语言标签 + 一键复制
function CodeBlock({ lang, children }: { lang: string; children: React.ReactNode }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(() => {
    const text = typeof children === "string" ? children : String(children);
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [children]);

  return (
    <div className="relative my-2 rounded-lg overflow-hidden border border-white/10 bg-slate-900/80">
      <div className="flex items-center justify-between px-2.5 py-1 bg-slate-800/60 border-b border-white/5">
        <span className="text-[9px] font-mono text-slate-400 uppercase tracking-wide">{lang}</span>
        <button
          onClick={handleCopy}
          className="text-[9px] text-slate-500 hover:text-slate-200 transition-colors cursor-pointer"
        >
          {copied ? "✓ 已复制" : "复制"}
        </button>
      </div>
      <pre className="overflow-x-auto p-2.5 text-[10px] leading-relaxed">
        <code className="font-mono text-slate-300">{children}</code>
      </pre>
    </div>
  );
}

export const MarkdownRenderer = memo(MarkdownRendererBase);
