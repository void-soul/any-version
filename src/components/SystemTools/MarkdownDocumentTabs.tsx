import type * as React from "react";
import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import { X, FileText } from "lucide-react";
import { type Tab } from "./MarkdownReader.model";
import type { Dispatch, SetStateAction } from "react";

interface MarkdownDocumentTabsProps {
  tabs: Tab[];
  busy?: boolean;
  setActivePath: Dispatch<SetStateAction<string | null>>;
  setMode: Dispatch<SetStateAction<"edit" | "view">>;
  activePath: string | null;
  closeTab: (path: string, e?: React.MouseEvent<Element> | undefined) => Promise<void>;
}

/** Pure MarkdownDocumentTabs view; its owning panel retains side effects and state. */
export function MarkdownDocumentTabs({ tabs, busy = false, setActivePath, setMode, activePath, closeTab }: MarkdownDocumentTabsProps) {
  const { t } = useTranslation();
  return (
    <div className="flex-shrink-0 flex items-stretch gap-0.5 px-2 pt-1.5 border-b border-white/5 overflow-x-auto">
      {tabs.map((tab) => (
        <div
          key={tab.path}
          title={tab.path}
          className={`group flex items-center gap-1.5 px-2.5 py-1.5 rounded-t-md text-caption whitespace-nowrap cursor-pointer transition-colors max-w-[200px] ${
            tab.path === activePath
              ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)] border-b-2 border-[var(--module-accent)]"
              : "text-slate-400 hover:text-slate-200 hover:bg-white/5 border-b-2 border-transparent"
          }`}
        >
          <button type="button" aria-pressed={tab.path === activePath} onClick={() => { setActivePath(tab.path); setMode("view"); }} className="flex min-w-0 items-center gap-1.5 text-inherit cursor-pointer">
            <FileText className="w-3 h-3 shrink-0" />
            <span className="truncate">{tab.name}{tab.dirty ? " •" : ""}</span>
          </button>
          <button type="button" disabled={busy} aria-label={t("mdreader.closeTip")}
            onClick={(e) => closeTab(tab.path, e)}
            className="ui-icon-btn w-6 h-6 shrink-0 opacity-0 group-hover:opacity-100 focus:opacity-100 text-slate-500 hover:text-red-300"
            title={t("mdreader.closeTip")}
          >
            <X className="w-3 h-3" />
          </button>
        </div>
      ))}
    </div>
  );
}
