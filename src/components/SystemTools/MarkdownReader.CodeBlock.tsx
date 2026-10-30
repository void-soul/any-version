import React, { useState, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import hljs from "highlight.js/lib/core";
/** 代码块：语言标签 + 语法高亮 + 一键复制 */
export function CodeBlock({ lang, children }: { lang: string; children: React.ReactNode }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const text = typeof children === "string" ? children : String(children);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  }, [text]);

  // 用 hljs 高亮；语言未注册时退回自动检测，再退回纯文本转义
  const html = useMemo(() => {
    try {
      if (lang && hljs.getLanguage(lang)) {
        return hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
      }
      const auto = hljs.highlightAuto(text);
      if (auto.relevance > 0) return auto.value;
    } catch {
      /* fallthrough */
    }
    return text
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }, [lang, text]);

  return (
    <div className="relative my-3 rounded-ctl overflow-hidden border border-white/10 bg-slate-900/80">
      <div className="flex items-center justify-between px-2.5 py-1 bg-slate-800/60 border-b border-white/5">
        <span className="text-micro font-mono text-slate-400 uppercase tracking-wide">
          {lang || "text"}
        </span>
        <button
          onClick={handleCopy}
          className="text-micro text-slate-500 hover:text-slate-200 transition-colors cursor-pointer"
        >
          {copied ? t("mdreader.codeCopied") : t("mdreader.codeCopy")}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-caption leading-relaxed">
        <code
          className="font-mono hljs"
          style={{ background: "transparent", color: "#e2e8f0", padding: 0 }}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </pre>
    </div>
  );
}
