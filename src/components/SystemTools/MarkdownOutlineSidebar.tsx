import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import { ChevronRight, ChevronDown } from "lucide-react";
import { type Heading } from "./MarkdownReader.model";
import type { Dispatch, SetStateAction } from "react";

interface MarkdownOutlineSidebarProps {
  outlineRows: { h: Heading; hasChildren: boolean; hidden: boolean; }[];
  activeHeading: string | null;
  collapsed: Set<string>;
  setCollapsed: Dispatch<SetStateAction<Set<string>>>;
  jumpToHeading: (h: Heading) => void;
}

/** Pure MarkdownOutlineSidebar view; its owning panel retains side effects and state. */
export function MarkdownOutlineSidebar({ outlineRows, activeHeading, collapsed, setCollapsed, jumpToHeading }: MarkdownOutlineSidebarProps) {
  const { t } = useTranslation();
  return (
    <div className="w-48 flex-shrink-0 border-l border-white/5 overflow-y-auto py-2">
      <div className="px-3 pb-1.5 text-tiny text-slate-500">{t("mdreader.outline")}</div>
      {outlineRows.filter((row) => !row.hidden).map(({ h, hasChildren }) => {
        const isActive = h.id === activeHeading;
        const isCollapsed = collapsed.has(h.id);
        return (
          <div
            key={h.id}
            className={`group flex items-center pr-1 transition-colors ${
              isActive ? "bg-emerald-600/15 text-emerald-200" : "text-slate-500 hover:bg-white/5"
            }`}
          >
            <button
              onClick={() => {
                if (!hasChildren) return;
                setCollapsed((prev) => {
                  const next = new Set(prev);
                  if (next.has(h.id)) next.delete(h.id);
                  else next.add(h.id);
                  return next;
                });
              }}
              className={`flex-shrink-0 flex items-center justify-center w-4 h-5 ${hasChildren ? "cursor-pointer text-slate-500 hover:text-slate-200" : "invisible"}`}
              title={isCollapsed ? t("mdreader.expand") : t("mdreader.collapse")}
            >
              {isCollapsed ? <ChevronRight className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
            </button>
            <button
              onClick={() => jumpToHeading(h)}
              style={{ paddingLeft: `${(h.level - 1) * 10}px` }}
              className={`flex-1 min-w-0 text-left py-0.5 text-tiny truncate ${
                isActive ? "text-emerald-200" : "group-hover:text-slate-200"
              }`}
              title={h.text}
            >
              {h.text}
            </button>
          </div>
        );
      })}
    </div>
  );
}
