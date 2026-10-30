import React, { type MouseEvent } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { isExternal, slugify, type Tab } from "./MarkdownReader.model";
import { CodeBlock } from "./MarkdownReader.CodeBlock";
import { LocalImage } from "./MarkdownReader.LocalImage";

interface MarkdownPreviewContentProps {
  headingIdByLine: Map<number, string>;
  toggleTaskAtLine: (line1: number) => void;
  handleLinkClick: (href: string, e: MouseEvent<Element>) => Promise<void>;
  active: Tab;
}

/** Pure MarkdownPreviewContent view; its owning panel retains side effects and state. */
export function MarkdownPreviewContent({ headingIdByLine, toggleTaskAtLine, handleLinkClick, active }: MarkdownPreviewContentProps) {
  /** 带锚点 id 的标题渲染器（用 hast 行号对齐大纲 id，重复标题也不会撞锚点） */
  const heading = (level: number, cls: string) =>
    function H({ children, node }: { children?: React.ReactNode; node?: { position?: { start?: { line?: number } } } }) {
      const text = React.Children.toArray(children)
        .map((c) => (typeof c === "string" ? c : ""))
        .join("");
      const line = node?.position?.start?.line;
      const id = (line !== undefined ? headingIdByLine.get(line) : undefined) || slugify(text);
      const Tag = `h${level}` as keyof React.JSX.IntrinsicElements;
      return React.createElement(Tag, { id, className: cls }, children);
    };

  return (
    <div className="px-6 py-5 max-w-4xl mx-auto text-body leading-7 text-slate-200 break-words">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={{
          h1: heading(1, "text-xl font-bold text-slate-100 mt-6 mb-3 pb-1.5 border-b border-white/10 first:mt-0"),
          h2: heading(2, "text-lg font-bold text-slate-100 mt-5 mb-2.5 pb-1 border-b border-white/5 first:mt-0"),
          h3: heading(3, "text-base font-bold text-slate-200 mt-4 mb-2 first:mt-0"),
          h4: heading(4, "text-sm font-bold text-slate-200 mt-3 mb-1.5 first:mt-0"),
          h5: heading(5, "text-body font-semibold text-slate-300 mt-3 mb-1 first:mt-0"),
          h6: heading(6, "text-caption font-semibold text-slate-400 mt-3 mb-1 first:mt-0"),
          p: ({ children }) => <p className="my-2.5 first:mt-0 last:mb-0">{children}</p>,
          ul: ({ children }) => <ul className="list-disc my-2.5 space-y-1 pl-6">{children}</ul>,
          ol: ({ children }) => <ol className="list-decimal my-2.5 space-y-1 pl-6">{children}</ol>,
          li: ({ children, node }) => (
            <li className="leading-7" data-line={node?.position?.start?.line}>
              {children}
            </li>
          ),
          input: ({ checked }) => (
            <input
              type="checkbox"
              checked={!!checked}
              onChange={(e) => {
                // 任务项所在行由 li 上的 data-line 标记，点击即可回写源码
                const line = e.currentTarget.closest("li")?.getAttribute("data-line");
                if (line) toggleTaskAtLine(Number(line));
              }}
              className="mr-1.5 align-middle w-3 h-3 rounded accent-emerald-500 cursor-pointer"
            />
          ),
          blockquote: ({ children }) => (
            <blockquote className="border-l-2 border-emerald-600/50 pl-3 my-3 text-slate-400">
              {children}
            </blockquote>
          ),
          code: ({ className, children }) => {
            const match = /language-(\w+)/.exec(className || "");
            if (!match) {
              return (
                <code className="px-1 py-0.5 rounded bg-slate-700/60 text-caption text-emerald-300 font-mono">
                  {children}
                </code>
              );
            }
            return <CodeBlock lang={match[1]}>{children}</CodeBlock>;
          },
          pre: ({ children }) => <>{children}</>,
          a: ({ href, children }) => {
            const h = href || "";
            const external = isExternal(h);
            return (
              <a
                href={h}
                target={external ? "_blank" : undefined}
                rel={external ? "noopener noreferrer" : undefined}
                onClick={(e) => handleLinkClick(h, e)}
                className={`underline underline-offset-2 cursor-pointer ${
                  external
                    ? "text-sky-400 hover:text-sky-300"
                    : "text-emerald-400 hover:text-emerald-300"
                }`}
              >
                {children}
              </a>
            );
          },
          img: ({ src, alt }) => (
            <LocalImage docPath={active.path} src={typeof src === "string" ? src : ""} alt={alt || ""} />
          ),
          table: ({ children }) => (
            <div className="overflow-x-auto my-3 rounded border border-white/10">
              <table className="min-w-full text-caption">{children}</table>
            </div>
          ),
          thead: ({ children }) => <thead className="bg-slate-800/80">{children}</thead>,
          th: ({ children }) => (
            <th className="px-2.5 py-1.5 text-left font-semibold text-slate-200 border-b border-white/10">
              {children}
            </th>
          ),
          td: ({ children }) => (
            <td className="px-2.5 py-1.5 text-slate-300 border-b border-white/5">{children}</td>
          ),
          hr: () => <hr className="border-white/10 my-5" />,
          strong: ({ children }) => <strong className="font-bold text-slate-100">{children}</strong>,
          del: ({ children }) => <del className="text-slate-500">{children}</del>,
        }}
      >
        {active.content}
      </ReactMarkdown>
    </div>
  );
}
