import { useTranslation } from 'react-i18next';
import { FileText } from 'lucide-react';
import { type SkillEntry } from "./SkillManager.model";

interface SkillLibraryPickerProps {
  libFiltered: SkillEntry[];
  pickedLib: Set<string>;
  toggleLibPick: (skillId: string) => void;
  openLibPreview: (skillId: string, name: string) => Promise<void>;
}

/** Pure SkillLibraryPicker view; its owning panel retains side effects and state. */
export function SkillLibraryPicker({ libFiltered, pickedLib, toggleLibPick, openLibPreview }: SkillLibraryPickerProps) {
  const { t } = useTranslation();
  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 gap-1 max-h-72 overflow-y-auto">
      {libFiltered.map((s) => {
        const on = pickedLib.has(s.id);
        return (
          <div
            key={s.id}
            className={`flex items-center gap-1 px-1.5 py-1 rounded-ctl transition-colors ${
              on ? 'bg-emerald-500/15' : 'bg-white/5 hover:bg-white/10'
            }`}
          >
            <button
              onClick={() => toggleLibPick(s.id)}
              className="flex items-center gap-1.5 min-w-0 flex-1 text-left cursor-pointer"
              title={s.description || s.id}
            >
              <span className={`w-3 h-3 rounded-sm flex-shrink-0 border ${on ? 'bg-emerald-500 border-emerald-400' : 'border-white/20'}`} />
              <span className={`text-tiny truncate flex-1 ${on ? 'text-emerald-300' : 'text-slate-400'}`}>
                {s.name || s.id}
              </span>
              <span className="text-micro text-slate-500 font-mono flex-shrink-0">
                {t("skillmgr.skillChars", { n: s.skillMdChars })}
              </span>
            </button>
            {/* 勾选与查看分开两个按钮：整个 item 点一下就切换勾选，
                看文档必须是显式动作，否则误触会打断选择节奏 */}
            <button
              onClick={() => void openLibPreview(s.id, s.name || s.id)}
              className="p-0.5 rounded text-slate-600 hover:text-[var(--module-accent)] cursor-pointer flex-shrink-0"
              title={t("skillmgr.projViewMd")}
            >
              <FileText className="w-3 h-3" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
