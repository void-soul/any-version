import { useTranslation } from 'react-i18next';
import { X } from 'lucide-react';
import type { Dispatch, SetStateAction } from "react";

interface SkillMetadataFieldsProps {
  edCat: string;
  setEdCat: Dispatch<SetStateAction<string>>;
  edTags: string[];
  removeTag: (t: string) => void;
  tagInput: string;
  setTagInput: Dispatch<SetStateAction<string>>;
  addTag: () => void;
}

/** Pure SkillMetadataFields view; its owning panel retains side effects and state. */
export function SkillMetadataFields({ edCat, setEdCat, edTags, removeTag, tagInput, setTagInput, addTag }: SkillMetadataFieldsProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2 text-body">
      <div>
        <label className="text-tiny text-slate-400 mb-1 block">{t("skillmgr.category")}</label>
        <input
          value={edCat}
          onChange={(e) => setEdCat(e.target.value)}
          placeholder={t("skillmgr.categoryPh")}
          className="w-full px-2.5 py-1.5 rounded bg-white/5 border border-white/10 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
        />
      </div>

      <div>
        <label className="text-tiny text-slate-400 mb-1 block">{t("skillmgr.tags")}</label>
        <div className="flex flex-wrap gap-1 mb-2">
          {edTags.map((t) => (
            <span key={t} className="px-2 py-0.5 rounded bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[var(--module-accent)] text-tiny flex items-center gap-1">
              #{t}
              <button onClick={() => removeTag(t)} className="hover:text-red-300">
                <X className="w-2.5 h-2.5" />
              </button>
            </span>
          ))}
        </div>
        <div className="flex gap-1.5">
          <input
            value={tagInput}
            onChange={(e) => setTagInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); addTag(); } }}
            placeholder={t("skillmgr.tagPh")}
            className="flex-1 px-2.5 py-1.5 rounded bg-white/5 border border-white/10 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
          />
          <button onClick={addTag} className="px-3 py-1.5 rounded bg-white/10 hover:bg-white/20 text-body font-semibold text-slate-200 cursor-pointer">
            {t("skillmgr.add")}
          </button>
        </div>
      </div>
    </div>
  );
}
