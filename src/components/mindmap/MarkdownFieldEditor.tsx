import { memo, useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Bold, Italic, Strikethrough, Code, List, ListOrdered, ListChecks, Quote,
  Square, Link, Table, Minus, Heading1, Heading2, Heading3, Undo2, Redo2,
  Columns, Eye, Pencil,
} from "lucide-react";
import { MindmapMarkdown } from "./MindmapMarkdown";

/** 在光标选区前后包裹 prefix/suffix；无选区时插入占位符并选中它 */
function wrapSelection(ta: HTMLTextAreaElement, prefix: string, suffix: string, placeholder = ""): { text: string; selStart: number; selEnd: number } {
  const { selectionStart: s, selectionEnd: e, value } = ta;
  const sel = value.slice(s, e);
  const inner = sel || placeholder;
  const next = value.slice(0, s) + prefix + inner + suffix + value.slice(e);
  return { text: next, selStart: s + prefix.length, selEnd: s + prefix.length + inner.length };
}

/** 行首前缀切换（标题/引用/列表） */
function toggleLinePrefix(ta: HTMLTextAreaElement, prefix: string, ordered = false): { text: string; selStart: number; selEnd: number } {
  const { value } = ta;
  let s = ta.selectionStart;
  let e = ta.selectionEnd;
  while (s > 0 && value[s - 1] !== "\n") s--;
  while (e < value.length && value[e] !== "\n") e++;
  const block = value.slice(s, e);
  const lines = block.split("\n");
  const allHave = lines.every((l) => l.trimStart().startsWith(prefix));
  const next = lines
    .map((l, i) => {
      if (allHave) return l.replace(new RegExp(`^(\\s*)${prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s?`), "$1");
      if (ordered) return `${i + 1}. ${l.replace(/^\s*(#{1,6}\s|>\s|[-*+]\s|\d+\.\s)/, "")}`;
      return `${prefix} ${l.replace(/^\s*(#{1,6}\s|>\s|[-*+]\s|\d+\.\s)/, "")}`;
    })
    .join("\n");
  const out = value.slice(0, s) + next + value.slice(e);
  return { text: out, selStart: s, selEnd: s + next.length };
}

interface Props {
  value: string;
  onChange: (value: string) => void;
  minHeight?: string;
  /** 是否默认分栏（编辑+预览） */
  defaultSplit?: boolean;
}

/** 可复用的 Markdown 编辑器：格式工具栏 + 编辑区 + 可选分栏实时预览。
 *  供思维导图节点详情编辑、悬浮窗节点表单等场景复用。 */
export const MarkdownFieldEditor = memo(function MarkdownFieldEditor({ value, onChange, minHeight = "200px", defaultSplit = false }: Props) {
  const { t } = useTranslation();
  const [split, setSplit] = useState(defaultSplit);
  const [mode, setMode] = useState<"edit" | "view">("edit");
  const taRef = useRef<HTMLTextAreaElement>(null);
  const undoStack = useRef<string[]>([]);
  const redoStack = useRef<string[]>([]);
  const lastValue = useRef(value);

  const applyTransform = useCallback((fn: (ta: HTMLTextAreaElement) => { text: string; selStart: number; selEnd: number }) => {
    const ta = taRef.current;
    if (!ta) return;
    const { text, selStart, selEnd } = fn(ta);
    undoStack.current.push(lastValue.current);
    redoStack.current = [];
    lastValue.current = text;
    onChange(text);
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(selStart, selEnd);
    });
  }, [onChange]);

  const undo = useCallback(() => {
    if (undoStack.current.length === 0) return;
    const prev = undoStack.current.pop()!;
    redoStack.current.push(lastValue.current);
    lastValue.current = prev;
    onChange(prev);
  }, [onChange]);

  const redo = useCallback(() => {
    if (redoStack.current.length === 0) return;
    const next = redoStack.current.pop()!;
    undoStack.current.push(lastValue.current);
    lastValue.current = next;
    onChange(next);
  }, [onChange]);

  const handleChange = useCallback((e: React.ChangeEvent<HTMLTextAreaElement>) => {
    undoStack.current.push(lastValue.current);
    redoStack.current = [];
    lastValue.current = e.target.value;
    onChange(e.target.value);
  }, [onChange]);

  // Ctrl+Z / Ctrl+Y
  const onKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.ctrlKey && e.key.toLowerCase() === "z") { e.preventDefault(); undo(); }
    if (e.ctrlKey && (e.key.toLowerCase() === "y" || (e.shiftKey && e.key.toLowerCase() === "z"))) { e.preventDefault(); redo(); }
    if (e.key === "Tab") {
      e.preventDefault();
      const ta = e.currentTarget as HTMLTextAreaElement;
      const { selectionStart: s, selectionEnd: ee, value: v } = ta;
      const next = v.slice(0, s) + "  " + v.slice(ee);
      handleChange({ target: { value: next } } as React.ChangeEvent<HTMLTextAreaElement>);
      requestAnimationFrame(() => ta.setSelectionRange(s + 2, s + 2));
    }
  }, [undo, redo, handleChange]);

  const tbtn = "inline-flex items-center justify-center h-6 w-6 rounded-md text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-30";

  const preview = useMemo(() => (
    <div className="overflow-y-auto p-3">
      <MindmapMarkdown content={value || t("mmd.empty")} />
    </div>
  ), [value, t]);

  return (
    <div className="flex flex-col min-h-0 flex-1 rounded-md border border-white/10 bg-slate-950/70">
      {/* 格式工具栏 */}
      <div className="flex-shrink-0 flex items-center gap-0.5 px-1.5 py-1 border-b border-white/5 overflow-x-auto">
        <button className={tbtn} title={t("mmd.undo")} onClick={undo}><Undo2 className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.redo")} onClick={redo}><Redo2 className="w-3 h-3" /></button>
        <div className="w-px h-4 bg-white/10 mx-0.5" />
        <button className={tbtn} title={t("mmd.h1")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "#"))}><Heading1 className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.h2")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "##"))}><Heading2 className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.h3")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "###"))}><Heading3 className="w-3 h-3" /></button>
        <div className="w-px h-4 bg-white/10 mx-0.5" />
        <button className={tbtn} title={t("mmd.bold")} onClick={() => applyTransform((ta) => wrapSelection(ta, "**", "**", t("mmd.phBold")))}><Bold className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.italic")} onClick={() => applyTransform((ta) => wrapSelection(ta, "*", "*", t("mmd.phItalic")))}><Italic className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.strike")} onClick={() => applyTransform((ta) => wrapSelection(ta, "~~", "~~", t("mmd.phStrike")))}><Strikethrough className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.inlineCode")} onClick={() => applyTransform((ta) => wrapSelection(ta, "`", "`", t("mmd.phCode")))}><Code className="w-3 h-3" /></button>
        <div className="w-px h-4 bg-white/10 mx-0.5" />
        <button className={tbtn} title={t("mmd.ul")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "-"))}><List className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.ol")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "1.", true))}><ListOrdered className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.task")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "- [ ]"))}><ListChecks className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.quote")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, ">"))}><Quote className="w-3 h-3" /></button>
        <div className="w-px h-4 bg-white/10 mx-0.5" />
        <button className={tbtn} title={t("mmd.codeBlock")} onClick={() => applyTransform((ta) => {
          const { selectionStart: s, selectionEnd: e, value } = ta;
          const sel = value.slice(s, e) || "// code";
          const insert = `\n\`\`\`\n${sel}\n\`\`\`\n`;
          return { text: value.slice(0, s) + insert + value.slice(e), selStart: s + 4, selEnd: s + 4 + sel.length };
        })}><Square className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.link")} onClick={() => applyTransform((ta) => wrapSelection(ta, "[", "](https://)", t("mmd.phLink")))}><Link className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.table")} onClick={() => applyTransform((ta) => {
          const tpl = `\n| ${t("mmd.tableCol1")} | ${t("mmd.tableCol2")} | ${t("mmd.tableCol3")} |\n| --- | --- | --- |\n|  a  |  b  |  c  |\n`;
          const { selectionStart: s, value } = ta;
          return { text: value.slice(0, s) + tpl + value.slice(s), selStart: s + 3, selEnd: s + 3 };
        })}><Table className="w-3 h-3" /></button>
        <button className={tbtn} title={t("mmd.hr")} onClick={() => applyTransform((ta) => {
          const { selectionStart: s, value } = ta;
          return { text: value.slice(0, s) + "\n---\n" + value.slice(s), selStart: s + 5, selEnd: s + 5 };
        })}><Minus className="w-3 h-3" /></button>
        <div className="flex-1" />
        <button className={tbtn} title={mode === "edit" ? t("mmd.preview") : t("mmd.edit")} onClick={() => setMode(mode === "edit" ? "view" : "edit")}>
          {mode === "edit" ? <Eye className="w-3 h-3" /> : <Pencil className="w-3 h-3" />}
        </button>
        {mode === "edit" && (
          <button className={`${tbtn} ${split ? "bg-white/10 text-white" : ""}`} title={t("mmd.split")} onClick={() => setSplit(!split)}>
            <Columns className="w-3 h-3" />
          </button>
        )}
      </div>

      {/* 内容区 */}
      {mode === "view" ? (
        <div className="min-h-0 flex-1 overflow-y-auto">{preview}</div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <textarea
            ref={taRef}
            value={value}
            onChange={handleChange}
            onKeyDown={onKeyDown}
            spellCheck={false}
            className={`min-h-0 flex-1 resize-none bg-transparent px-3 py-2 text-[11px] leading-5 text-slate-200 font-mono outline-none ${split ? "border-r border-white/10 max-w-[50%]" : ""}`}
            style={{ minHeight }}
            placeholder={t("mmd.placeholder")}
          />
          {split && (
            <div className="min-h-0 flex-1 overflow-y-auto">{preview}</div>
          )}
        </div>
      )}
    </div>
  );
});
