import { useTranslation } from 'react-i18next';
import type { Dispatch, SetStateAction } from "react";

interface SkillScanDirectoryOptionsProps {
  SCAN_DIRS: string[];
  pickedDirs: string[];
  toggleScanDir: (d: string) => void;
  setPickedDirs: Dispatch<SetStateAction<string[]>>;
  autoGitignore: boolean;
  setAutoGitignore: Dispatch<SetStateAction<boolean>>;
}

/** Pure SkillScanDirectoryOptions view; its owning panel retains side effects and state. */
export function SkillScanDirectoryOptions({ SCAN_DIRS, pickedDirs, toggleScanDir, setPickedDirs, autoGitignore, setAutoGitignore }: SkillScanDirectoryOptionsProps) {
  const { t } = useTranslation();
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="text-micro text-slate-500">{t("skillmgr.installToDirs")}</span>
      {SCAN_DIRS.map((d) => {
        const on = pickedDirs.includes(d);
        return (
          <button
            key={d}
            onClick={() => toggleScanDir(d)}
            title={t("skillmgr.scanDirHint", { dir: d })}
            className={`flex items-center gap-1 px-2 py-0.5 rounded text-micro font-mono cursor-pointer transition-colors border ${
              on
                ? "bg-[var(--module-accent-soft)] border-[var(--module-accent-ring)] text-slate-200"
                : "bg-white/5 border-white/10 text-slate-600 hover:text-slate-400"
            }`}
          >
            <span className={`w-1.5 h-1.5 rounded-full ${on ? "bg-emerald-400" : "bg-slate-600"}`} />
            {d}
          </button>
        );
      })}
      <button
        onClick={() => setPickedDirs([...SCAN_DIRS])}
        className="px-2 py-0.5 rounded text-micro bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer"
      >
        {t("skillmgr.scanDirAll")}
      </button>
      <label
        className="flex items-center gap-1 px-2 py-0.5 rounded text-micro bg-white/5 border border-white/10 text-slate-400 cursor-pointer"
        title={t("skillmgr.autoGitignoreHint")}
      >
        <input
          type="checkbox"
          checked={autoGitignore}
          onChange={(e) => setAutoGitignore(e.target.checked)}
          className="w-2.5 h-2.5 accent-[var(--module-accent)]"
        />
        {t("skillmgr.autoGitignore")}
      </label>
    </div>
  );
}
