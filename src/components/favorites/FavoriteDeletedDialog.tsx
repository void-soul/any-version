import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { SOURCE_LABELS, type FavoriteDeletedRow } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface FavoriteDeletedDialogProps {
  setDeletedOpen: Dispatch<SetStateAction<boolean>>;
  deletedRows: FavoriteDeletedRow[];
  restoreDeleted: (source?: string | undefined, externalId?: string | undefined) => Promise<void>;
}

/** Pure FavoriteDeletedDialog view; its owning panel retains side effects and state. */
export function FavoriteDeletedDialog({ setDeletedOpen, deletedRows, restoreDeleted }: FavoriteDeletedDialogProps) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-[300] modal-mask bg-black/60 backdrop-blur-sm flex items-center justify-center p-4">
      <div className="w-full max-w-md bg-surface-panel border border-white/15 rounded-panel p-4 shadow-2xl space-y-3 text-body">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-bold text-white">{t("favorites.deletedTitle")}</h3>
          <button onClick={() => setDeletedOpen(false)} className="text-slate-400 hover:text-white p-1">
            <X className="w-4 h-4" />
          </button>
        </div>
        <p className="text-tiny text-slate-500 leading-relaxed">{t("favorites.deletedHint")}</p>
        <div className="max-h-64 overflow-y-auto space-y-1">
          {deletedRows.map((d) => (
            <div key={`${d.source}-${d.externalId}`} className="flex items-center gap-2 rounded-ctl border border-white/5 bg-white/[0.02] px-2 py-1.5">
              <span className="shrink-0 text-micro text-slate-500">{SOURCE_LABELS[d.source] ?? d.source}</span>
              <div className="min-w-0 flex-1">
                <div className="truncate text-caption text-slate-200">{d.title || d.externalId}</div>
                <div className="truncate text-micro text-slate-600">{d.deletedAt}</div>
              </div>
              <button
                onClick={() => void restoreDeleted(d.source, d.externalId)}
                className="shrink-0 px-2 py-1 rounded-ctl border border-white/10 text-tiny text-slate-300 hover:text-[var(--module-accent)] cursor-pointer"
              >
                {t("favorites.deletedRestore")}
              </button>
            </div>
          ))}
        </div>
        <div className="flex items-center justify-end gap-2 pt-1">
          <button
            onClick={() => void restoreDeleted()}
            className="px-2.5 py-1.5 rounded-ctl border border-white/10 text-tiny text-slate-300 hover:text-white cursor-pointer"
          >
            {t("favorites.deletedRestoreAll", { count: deletedRows.length })}
          </button>
          <button
            onClick={() => setDeletedOpen(false)}
            className="px-2.5 py-1.5 rounded-ctl bg-[var(--module-accent)] text-white text-tiny font-semibold cursor-pointer"
          >
            {t("favorites.deletedClose")}
          </button>
        </div>
      </div>
    </div>
  );
}
