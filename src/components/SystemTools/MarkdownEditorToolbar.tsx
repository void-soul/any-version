import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import { Link, List, ListOrdered, Quote, Code, Minus, Heading1, Heading2, Heading3, Bold, Italic, Strikethrough, Table, Square, Undo2, Redo2, ListChecks } from "lucide-react";
import { toggleLinePrefix, type Tab, wrapSelection } from "./MarkdownReader.model";

interface MarkdownEditorToolbarProps {
  tbtn: string;
  active: Tab;
  undo: () => void;
  redo: () => void;
  applyTransform: (fn: (ta: HTMLTextAreaElement) => { text: string; selStart: number; selEnd: number; }) => void;
}

/** Pure MarkdownEditorToolbar view; its owning panel retains side effects and state. */
export function MarkdownEditorToolbar({ tbtn, active, undo, redo, applyTransform }: MarkdownEditorToolbarProps) {
  const { t } = useTranslation();
  return (
    <div className="flex-shrink-0 flex items-center gap-0.5 px-2 py-1.5 border-b border-white/5 overflow-x-auto">
      <button className={tbtn} title={t("mdreader.undoTitle")} disabled={active.undo.length === 0} onClick={undo}><Undo2 className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.redoTitle")} disabled={active.redo.length === 0} onClick={redo}><Redo2 className="w-3.5 h-3.5" /></button>
      <div className="w-px h-5 bg-white/10 mx-1" />
      <button className={tbtn} title={t("mdreader.h1")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "#"))}><Heading1 className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.h2")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "##"))}><Heading2 className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.h3")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "###"))}><Heading3 className="w-3.5 h-3.5" /></button>
      <div className="w-px h-5 bg-white/10 mx-1" />
      <button className={tbtn} title={t("mdreader.bold")} onClick={() => applyTransform((ta) => wrapSelection(ta, "**", "**", t("mdreader.boldPh")))}><Bold className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.italic")} onClick={() => applyTransform((ta) => wrapSelection(ta, "*", "*", t("mdreader.italicPh")))}><Italic className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.strike")} onClick={() => applyTransform((ta) => wrapSelection(ta, "~~", "~~", t("mdreader.strikePh")))}><Strikethrough className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.inlineCode")} onClick={() => applyTransform((ta) => wrapSelection(ta, "`", "`", t("mdreader.codePh")))}><Code className="w-3.5 h-3.5" /></button>
      <div className="w-px h-5 bg-white/10 mx-1" />
      <button className={tbtn} title={t("mdreader.ul")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "-"))}><List className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.ol")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "1.", true))}><ListOrdered className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.task")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "- [ ]"))}><ListChecks className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.quote")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, ">"))}><Quote className="w-3.5 h-3.5" /></button>
      <div className="w-px h-5 bg-white/10 mx-1" />
      <button className={tbtn} title={t("mdreader.codeBlock")} onClick={() => applyTransform((ta) => {
        const { selectionStart: s, selectionEnd: e, value } = ta;
        const sel = value.slice(s, e) || "// code";
        const insert = `\n\`\`\`\n${sel}\n\`\`\`\n`;
        return { text: value.slice(0, s) + insert + value.slice(e), selStart: s + 4, selEnd: s + 4 + sel.length };
      })}><Square className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.link")} onClick={() => applyTransform((ta) => wrapSelection(ta, "[", "](https://)", t("mdreader.linkTextPh")))}><Link className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.table")} onClick={() => applyTransform((ta) => {
        const tpl = `\n| ${t("mdreader.col1")} | ${t("mdreader.col2")} | ${t("mdreader.col3")} |\n| --- | --- | --- |\n|  a  |  b  |  c  |\n`;
        const { selectionStart: s, value } = ta;
        return { text: value.slice(0, s) + tpl + value.slice(s), selStart: s + 3, selEnd: s + 3 };
      })}><Table className="w-3.5 h-3.5" /></button>
      <button className={tbtn} title={t("mdreader.hr")} onClick={() => applyTransform((ta) => {
        const { selectionStart: s, value } = ta;
        return { text: value.slice(0, s) + "\n---\n" + value.slice(s), selStart: s + 5, selEnd: s + 5 };
      })}><Minus className="w-3.5 h-3.5" /></button>
    </div>
  );
}
