import { useTranslation } from "react-i18next";
import "katex/dist/katex.min.css";
import { FolderOpen, FileText, FilePlus, Folder, History } from "lucide-react";
import { dirOf, type RecentFile } from "./MarkdownReader.model";

interface MarkdownEmptyStateProps {
  pickFiles: () => Promise<void>;
  btn: string;
  openDirectory: () => Promise<void>;
  createNewFile: () => Promise<void>;
  recent: RecentFile[];
  openPath: (path: string, _rescan?: boolean) => Promise<void>;
}

/** Pure MarkdownEmptyState view; its owning panel retains side effects and state. */
export function MarkdownEmptyState({ pickFiles, btn, openDirectory, createNewFile, recent, openPath }: MarkdownEmptyStateProps) {
  const { t } = useTranslation();
  return (
    <div className="h-full flex flex-col items-center justify-center gap-3 text-slate-600">
      <FileText className="w-10 h-10 opacity-30" />
      <div className="text-caption">{t("mdreader.noDocOpen")}</div>
      <div className="flex gap-2">
        <button onClick={pickFiles} className={btn}>
          <FolderOpen className="w-3.5 h-3.5" />
          {t("mdreader.openFile")}
        </button>
        <button onClick={() => void openDirectory()} className={btn}>
          <Folder className="w-3.5 h-3.5" />
          {t("mdreader.openDir")}
        </button>
        <button onClick={() => void createNewFile()} className={btn}>
          <FilePlus className="w-3.5 h-3.5" />
          {t("mdreader.newFile")}
        </button>
      </div>
      <div className="text-tiny text-slate-700 max-w-xs text-center leading-relaxed">
        {t("mdreader.emptyDesc1")}
        {t("mdreader.emptyDesc2")}
      </div>
      {recent.length > 0 && (
        <div className="mt-2 w-80 max-w-[86%]">
          <div className="flex items-center gap-1.5 mb-1 text-tiny text-slate-600">
            <History className="w-3 h-3" />
            {t("mdreader.recent")}
          </div>
          <div className="max-h-44 overflow-y-auto rounded border border-white/5 bg-white/[0.02]">
            {recent.slice(0, 10).map((r) => (
              <button
                key={r.path}
                onClick={() => void openPath(r.path)}
                title={r.path}
                className="w-full text-left px-2.5 py-1 text-tiny flex items-center gap-1.5 text-slate-500 hover:text-slate-200 hover:bg-white/5 transition-colors"
              >
                <FileText className="w-3 h-3 opacity-60 flex-shrink-0" />
                <span className="truncate">{r.name}</span>
                <span className="ml-auto max-w-[48%] truncate text-slate-700">{dirOf(r.path)}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
