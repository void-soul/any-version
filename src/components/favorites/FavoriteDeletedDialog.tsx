import { useTranslation } from "react-i18next";
import { SharedModal } from "../shared/Modal";
import { SharedButton } from "../shared/Button";
import { useRef, useState } from "react";
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
  const gate = useRef(false);
  const [busy, setBusy] = useState(false);
  const restore = async (source?: string, externalId?: string) => {
    if (gate.current) return;
    gate.current = true;
    setBusy(true);
    try { await restoreDeleted(source, externalId); }
    finally { gate.current = false; setBusy(false); }
  };
  return (
    <SharedModal open title={t("favorites.deletedTitle")} width={480}
      closeDisabled={busy} onClose={() => setDeletedOpen(false)}>
      <div className="space-y-3 text-body">
        <p className="text-tiny text-slate-500 leading-relaxed">{t("favorites.deletedHint")}</p>
        <div className="max-h-64 overflow-y-auto space-y-1">
          {deletedRows.map((d) => (
            <div key={`${d.source}-${d.externalId}`} className="flex items-center gap-2 rounded-ctl border border-white/5 bg-white/[0.02] px-2 py-1.5">
              <span className="shrink-0 text-micro text-slate-500">{SOURCE_LABELS[d.source] ?? d.source}</span>
              <div className="min-w-0 flex-1">
                <div className="truncate text-caption text-slate-200">{d.title || d.externalId}</div>
                <div className="truncate text-micro text-slate-600">{d.deletedAt}</div>
              </div>
              <SharedButton disabled={busy}
                onClick={() => void restore(d.source, d.externalId)}
                className="shrink-0 px-2 py-1 rounded-ctl border border-white/10 text-tiny text-slate-300 hover:text-[var(--module-accent)] cursor-pointer"
              >
                {t("favorites.deletedRestore")}
              </SharedButton>
            </div>
          ))}
        </div>
        <div className="flex items-center justify-end gap-2 pt-1">
          <SharedButton disabled={busy}
            onClick={() => void restore()}
            className="px-2.5 py-1.5 rounded-ctl border border-white/10 text-tiny text-slate-300 hover:text-white cursor-pointer"
          >
            {t("favorites.deletedRestoreAll", { count: deletedRows.length })}
          </SharedButton>
          <SharedButton disabled={busy}
            onClick={() => setDeletedOpen(false)}
            className="px-2.5 py-1.5 rounded-ctl bg-[var(--module-accent)] text-white text-tiny font-semibold cursor-pointer"
          >
            {t("favorites.deletedClose")}
          </SharedButton>
        </div>
      </div>
    </SharedModal>
  );
}
