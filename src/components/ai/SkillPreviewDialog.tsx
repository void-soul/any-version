import { useTranslation } from 'react-i18next';
import { X, Loader2 } from 'lucide-react';
import { MarkdownRenderer } from "../shared/markdown/MarkdownRenderer";
import type { Dispatch, SetStateAction } from "react";

interface SkillPreviewDialogProps {
  preview: { name: string; md: string; };
  setPreview: Dispatch<SetStateAction<{ name: string; md: string; } | null>>;
  previewBusy: boolean;
}

/** Pure SkillPreviewDialog view; its owning panel retains side effects and state. */
export function SkillPreviewDialog({ preview, setPreview, previewBusy }: SkillPreviewDialogProps) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 modal-mask bg-black/60 backdrop-blur-xs flex items-center justify-center p-6">
      <div className="rounded-card ui-input w-full max-w-4xl max-h-[85vh] flex flex-col shadow-2xl">
        <div className="flex items-center justify-between border-b border-white/5 px-4 py-2.5 flex-shrink-0">
          <span className="text-body font-bold text-slate-200 truncate">
            {t("skillmgr.projMdTitle", { name: preview.name })}
          </span>
          <button onClick={() => setPreview(null)} className="text-slate-500 hover:text-slate-300 ml-3">
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto px-4 py-3">
          {previewBusy ? (
            <div className="flex items-center justify-center py-16 gap-2 text-slate-500 text-sm">
              <Loader2 className="w-4 h-4 animate-spin" /> {t("skillmgr.projMdLoading")}
            </div>
          ) : preview.md.trim() ? (
            <MarkdownRenderer content={preview.md} />
          ) : (
            <div className="text-center py-16 text-slate-500 text-sm">{t("skillmgr.projMdEmpty")}</div>
          )}
        </div>
      </div>
    </div>
  );
}
