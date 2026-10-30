import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import { FileText, Search, RefreshCw, ListTree } from "lucide-react";
import { formatSize, type MarkdownEntry } from "./MarkdownReader.model";
import type { Dispatch, SetStateAction } from "react";

interface MarkdownFileSidebarProps {
  siblingRoot: string;
  scanning: boolean;
  siblings: MarkdownEntry[];
  filter: string;
  setFilter: Dispatch<SetStateAction<string>>;
  filtered: MarkdownEntry[];
  openPath: (path: string, _rescan?: boolean) => Promise<void>;
  activePath: string | null;
}

/** Pure MarkdownFileSidebar view; its owning panel retains side effects and state. */
export function MarkdownFileSidebar({ siblingRoot, scanning, siblings, filter, setFilter, filtered, openPath, activePath }: MarkdownFileSidebarProps) {
  const { t } = useTranslation();
  return (
    <div className="w-56 flex-shrink-0 border-r border-white/5 flex flex-col min-h-0">
      <div className="flex-shrink-0 px-2 py-2 border-b border-white/5">
        <div className="flex items-center gap-1.5 mb-1.5 text-tiny text-slate-500">
          <ListTree className="w-3 h-3" />
          <span className="truncate" title={siblingRoot}>
            {siblingRoot ? siblingRoot.split(/[\\/]/).pop() : t("mdreader.fileList")}
          </span>
          {scanning && <RefreshCw className="w-3 h-3 animate-spin ml-auto" />}
          {!scanning && siblings.length > 0 && (
            <span className="ml-auto">{siblings.length}</span>
          )}
        </div>
        <div className="relative">
          <Search className="w-3 h-3 absolute left-2 top-1/2 -translate-y-1/2 text-slate-500" />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={t("mdreader.filterPh")}
            className="w-full bg-white/5 rounded pl-6 pr-2 py-1 text-tiny text-slate-200 placeholder:text-slate-600 outline-none focus:bg-white/10"
          />
        </div>
      </div>
      <div className="flex-1 overflow-y-auto py-1">
        {filtered.length === 0 && (
          <div className="px-3 py-4 text-tiny text-slate-600 text-center">
            {siblings.length === 0 ? t("mdreader.siblingsEmpty") : t("mdreader.noMatch")}
          </div>
        )}
        {filtered.map((s) => (
          <button
            key={s.path}
            onClick={() => openPath(s.path)}
            title={`${s.rel} · ${formatSize(s.size)}`}
            className={`w-full text-left px-2.5 py-1 text-tiny flex items-center gap-1.5 transition-colors ${
              s.path === activePath
                ? "bg-emerald-600/20 text-emerald-200"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <FileText className="w-3 h-3 flex-shrink-0 opacity-60" />
            <span className="truncate">{s.rel}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
