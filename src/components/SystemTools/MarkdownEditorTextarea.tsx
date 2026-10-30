import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import { continueListOnEnter, type Tab } from "./MarkdownReader.model";

interface MarkdownEditorTextareaProps {
  taRef: RefObject<HTMLTextAreaElement | null>;
  active: Tab;
  activePath: string | null;
  setContent: (path: string, next: string, pushUndo?: boolean) => void;
  updateCursor: () => void;
  split: boolean;
  syncScroll: (source: "editor" | "preview") => void;
  updateActiveHeading: () => void;
}

/** Pure MarkdownEditorTextarea view; its owning panel retains side effects and state. */
export function MarkdownEditorTextarea({ taRef, active, activePath, setContent, updateCursor, split, syncScroll, updateActiveHeading }: MarkdownEditorTextareaProps) {
  const { t } = useTranslation();
  return (
    <textarea
      ref={taRef}
      value={active.content}
      onChange={(e) => {
        if (activePath) setContent(activePath, e.target.value);
        requestAnimationFrame(updateCursor);
      }}
      onClick={updateCursor}
      onKeyUp={updateCursor}
      onSelect={updateCursor}
      onScroll={() => {
        if (split) syncScroll("editor");
        updateActiveHeading();
      }}
      onKeyDown={(e) => {
        // Tab 插入两个空格而不是移动焦点
        if (e.key === "Tab") {
          e.preventDefault();
          const ta = e.currentTarget;
          const { selectionStart: s, selectionEnd: ee, value } = ta;
          const next = value.slice(0, s) + "  " + value.slice(ee);
          setContent(activePath!, next);
          requestAnimationFrame(() => { ta.setSelectionRange(s + 2, s + 2); });
          return;
        }
        // 回车延续列表 / 引用；空条目回车退出列表
        if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey) {
          const ta = e.currentTarget;
          const res = continueListOnEnter(ta);
          if (res) {
            e.preventDefault();
            setContent(activePath!, res.text);
            requestAnimationFrame(() => {
              ta.setSelectionRange(res.caret, res.caret);
              updateCursor();
            });
          }
        }
      }}
      spellCheck={false}
      className={`min-h-0 flex-1 resize-none bg-transparent px-6 py-5 text-body leading-6 text-slate-200 font-mono outline-none ${split ? "border-r border-white/10 max-w-[50%]" : ""}`}
      placeholder={t("mdreader.editorPh")}
    />
  );
}
