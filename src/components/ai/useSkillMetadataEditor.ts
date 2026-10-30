import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { alertError } from "../shared/ThemedAlert";
import { type SkillEntry } from "./SkillManager.model";

/** Owns the SkillMetadataEditor feature, independently of page rendering. */
export function useSkillMetadataEditor({ loadSkills }: {
  loadSkills: () => Promise<void>;
}) {
  const { t } = useTranslation();

  const [editing, setEditing] = useState<SkillEntry | null>(null);

  const [edCat, setEdCat] = useState('');

  const [edTags, setEdTags] = useState<string[]>([]);

  const [tagInput, setTagInput] = useState('');

  const [savingMeta, setSavingMeta] = useState(false);

  // 保存元数据（分类/标签）
  const saveMeta = async () => {
    if (!editing) return;
    setSavingMeta(true);
    try {
      await invoke('update_skill_meta', {
        skillId: editing.id,
        category: edCat.trim(),
        tags: edTags,
      });
      setEditing(null);
      loadSkills();
    } catch (e: any) {
      alertError(t("skillmgr.saveFail", { err: String(e) }));
    }
    setSavingMeta(false);
  };

  const openEdit = (s: SkillEntry) => {
    setEditing(s);
    setEdCat(s.category || '');
    setEdTags([...s.tags]);
    setTagInput('');
  };

  const addTag = () => {
    const t = tagInput.trim();
    if (t && !edTags.includes(t)) setEdTags([...edTags, t]);
    setTagInput('');
  };

  const removeTag = (t: string) => setEdTags(edTags.filter((x) => x !== t));
  return {
    editing,
    setEditing,
    edCat,
    setEdCat,
    edTags,
    tagInput,
    setTagInput,
    savingMeta,
    saveMeta,
    openEdit,
    addTag,
    removeTag,
  };
}
