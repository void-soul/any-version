import { useTranslation } from 'react-i18next';
import { Trash2, Settings2, FileText } from 'lucide-react';
import { type SkillEntry } from "./SkillManager.model";

interface SkillLibraryListProps {
  filtered: SkillEntry[];
  skillMsg: { id: string; msg: string; ok: boolean; } | null;
  openLibPreview: (skillId: string, name: string) => Promise<void>;
  openEdit: (s: SkillEntry) => void;
  removeSkill: (id: string) => void;
}

/** Pure SkillLibraryList view; its owning panel retains side effects and state. */
export function SkillLibraryList({ filtered, skillMsg, openLibPreview, openEdit, removeSkill }: SkillLibraryListProps) {
  const { t } = useTranslation();
  return (
    <div className="divide-y divide-white/5 rounded-card border border-white/10 overflow-hidden">
      {filtered.map((s) => (
        <div
          key={s.id}
          className="group flex items-center gap-2 px-3 py-1.5 hover:bg-white/[0.04] transition-colors"
          title={[
            s.name || s.id,
            s.id,
            s.description || t("skillmgr.noDesc"),
            s.category,
            s.tags.length > 0 ? `#${s.tags.join(" #")}` : "",
            s.installMethod === "managed" ? t("skillmgr.managedLib") : s.installMethod,
          ].filter(Boolean).join("\n")}
        >
          <span className="text-tiny font-semibold text-slate-100 truncate flex-shrink-0 max-w-[46%]">
            {s.name || s.id}
          </span>
          {/* 描述仍留在行内：判断「这个技能是干什么的」靠它，全藏进 tooltip 就得逐个悬停 */}
          <span className="text-micro text-slate-500 truncate flex-1 min-w-0">
            {s.description || ""}
          </span>
          {skillMsg?.id === s.id && (
            <span className={`text-micro flex-shrink-0 ${skillMsg.ok ? "text-emerald-400" : "text-red-400"}`}>
              {skillMsg.msg}
            </span>
          )}
          <div className="flex items-center gap-2 flex-shrink-0 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity">
            <button
              onClick={() => void openLibPreview(s.id, s.name || s.id)}
              className="text-slate-400 hover:text-[var(--module-accent)] cursor-pointer"
              title={t("skillmgr.projViewMd")}
            >
              <FileText className="w-3.5 h-3.5" />
            </button>
            <button
              onClick={() => openEdit(s)}
              className="text-slate-400 hover:text-[var(--module-accent)] cursor-pointer"
              title={t("skillmgr.managedLib")}
            >
              <Settings2 className="w-3.5 h-3.5" />
            </button>
            <button
              onClick={() => removeSkill(s.id)}
              className="text-slate-500 hover:text-red-400 cursor-pointer"
              title={t("common.delete")}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}
