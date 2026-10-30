import { useTranslation } from 'react-i18next';
import { Note, ResultNote } from '../shared/Note';
import { X, Loader2, FolderOpen } from 'lucide-react';
import type { Dispatch, SetStateAction } from "react";

interface SkillDirectoryDialogProps {
  setDirDialog: Dispatch<SetStateAction<{ path: string; busy: boolean; msg: string | null; } | null>>;
  skillsDir: string;
  dirDialog: { path: string; busy: boolean; msg: string | null; };
  pickSkillsDir: () => Promise<void>;
  saveSkillsDir: () => Promise<void>;
}

/** Pure SkillDirectoryDialog view; its owning panel retains side effects and state. */
export function SkillDirectoryDialog({ setDirDialog, skillsDir, dirDialog, pickSkillsDir, saveSkillsDir }: SkillDirectoryDialogProps) {
  const { t } = useTranslation();
  return (
    <div className="fixed inset-0 z-50 modal-mask bg-black/60 backdrop-blur-xs flex items-center justify-center p-4">
      <div className="rounded-card ui-input p-4 w-full max-w-lg space-y-3 shadow-2xl">
        <div className="flex items-center justify-between border-b border-white/5 pb-2">
          <span className="text-body font-bold text-slate-200">{t("skillmgr.dirChange")}</span>
          <button onClick={() => setDirDialog(null)} className="text-slate-500 hover:text-slate-300">
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="space-y-1">
          <div className="text-tiny text-slate-400">{t("skillmgr.dirCurrent")}</div>
          <code className="block text-micro text-slate-300 font-mono break-all">{skillsDir || '…'}</code>
        </div>

        <div className="space-y-1">
          <div className="text-tiny text-slate-400">{t("skillmgr.dirNew")}</div>
          <code className="block text-micro text-slate-300 font-mono break-all">
            {dirDialog.path || t("skillmgr.dirDefaultHint")}
          </code>
          <div className="flex items-center gap-2 pt-1">
            <button
              onClick={() => void pickSkillsDir()}
              className="flex items-center gap-1 px-2.5 py-1.5 rounded-ctl bg-white/5 hover:bg-white/10 text-tiny text-slate-200 cursor-pointer"
            >
              <FolderOpen className="w-3 h-3" /> {t("skillmgr.dirPick")}
            </button>
            <button
              onClick={() => setDirDialog((d) => (d ? { ...d, path: '', msg: null } : d))}
              disabled={dirDialog.path === ''}
              className="px-2.5 py-1.5 rounded-ctl bg-white/5 hover:bg-white/10 disabled:opacity-40 text-tiny text-slate-300 cursor-pointer"
            >
              {t("skillmgr.dirReset")}
            </button>
          </div>
        </div>

        <Note tone="warn" title={t("skillmgr.dirWarnTitle")}>
          <p>{t("skillmgr.dirWarn1")}</p>
          <p className="text-micro text-slate-400">{t("skillmgr.dirWarn2")}</p>
        </Note>

        {dirDialog.msg && <ResultNote ok={false} message={dirDialog.msg} />}

        <div className="flex items-center justify-end gap-2 pt-1">
          <button
            onClick={() => setDirDialog(null)}
            className="px-3 py-1.5 rounded-ctl bg-white/5 hover:bg-white/10 text-body text-slate-300 cursor-pointer"
          >
            {t("skillmgr.cancel")}
          </button>
          <button
            onClick={() => void saveSkillsDir()}
            disabled={dirDialog.busy}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-body font-semibold text-white cursor-pointer"
          >
            {dirDialog.busy && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            {t("skillmgr.dirSave")}
          </button>
        </div>
      </div>
    </div>
  );
}
