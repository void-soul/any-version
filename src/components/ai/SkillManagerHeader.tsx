import { useTranslation } from 'react-i18next';
import { Package, Settings2, FolderOpen } from 'lucide-react';
import { type TabKey } from "./SkillManager.model";
import type { Dispatch, SetStateAction } from "react";

interface SkillManagerHeaderProps {
  setTab: Dispatch<SetStateAction<TabKey>>;
  tab: TabKey;
  skillsDir: string;
  setDirDialog: Dispatch<SetStateAction<{ path: string; busy: boolean; msg: string | null; } | null>>;
}

/** Pure SkillManagerHeader view; its owning panel retains side effects and state. */
export function SkillManagerHeader({ setTab, tab, skillsDir, setDirDialog }: SkillManagerHeaderProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center justify-between px-4 pt-3 border-b border-white/5 bg-white/[0.02]">
      <div className="flex items-center gap-1">
        {([
          { k: 'skills' as TabKey, label: t("skillmgr.tabSkills"), icon: Package },
          { k: 'projects' as TabKey, label: t("skillmgr.tabProjects"), icon: FolderOpen },
        ]).map(({ k, label, icon: Icon }) => (
          <button
            key={k}
            onClick={() => setTab(k)}
            className={`flex items-center gap-1.5 px-3.5 py-2.5 text-body font-semibold border-b-2 transition-all cursor-pointer ${
              tab === k
                ? 'border-[var(--module-accent)] text-white bg-white/[0.03]'
                : 'border-transparent text-slate-400 hover:text-slate-200 hover:bg-white/[0.01]'
            }`}
          >
            <Icon className="w-3.5 h-3.5" />
            {label}
          </button>
        ))}
      </div>
      <div className="flex items-center gap-1.5 min-w-0">
        <div className="text-tiny text-slate-500 font-mono truncate max-w-[320px]" title={skillsDir}>
          {t("skillmgr.libraryDir")}<code className="text-slate-400">{skillsDir || '…'}</code>
        </div>
        <button
          onClick={() => setDirDialog({ path: skillsDir, busy: false, msg: null })}
          className="p-1 rounded hover:bg-white/10 text-slate-400 hover:text-slate-200 cursor-pointer flex-shrink-0"
          title={t("skillmgr.dirChange")}
        >
          <Settings2 className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  );
}
