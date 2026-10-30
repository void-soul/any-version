import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Owns the SkillPreview feature, independently of page rendering. */
export function useSkillPreview({ selProject }: {
  selProject: string | null;
}) {

  /** 右侧勾选的「项目内已装技能」= 要删除的技能 */
  /** SKILL.md 预览弹窗 */
  const [preview, setPreview] = useState<{ name: string; md: string } | null>(null);

  const [previewBusy, setPreviewBusy] = useState(false);

  /** 打开 SKILL.md 预览（按需取全文，避免列表被撑大） */
  const openPreview = async (e: { skillId: string; name: string }) => {
    if (!selProject) return;
    setPreviewBusy(true);
    setPreview({ name: e.name, md: '' });
    try {
      const md = await invoke<string>('get_project_skill_md', { projectId: selProject, skillId: e.skillId });
      setPreview({ name: e.name, md });
    } catch (err: any) {
      setPreview({ name: e.name, md: String(err) });
    } finally {
      setPreviewBusy(false);
    }
  };

  /** 打开技能库里某个技能的 SKILL.md 预览（与项目内预览共用同一个弹窗） */
  const openLibPreview = async (skillId: string, name: string) => {
    setPreviewBusy(true);
    setPreview({ name, md: '' });
    try {
      const md = await invoke<string>('get_library_skill_md', { skillId });
      setPreview({ name, md });
    } catch (err: any) {
      setPreview({ name, md: String(err) });
    } finally {
      setPreviewBusy(false);
    }
  };
  return {
    preview,
    setPreview,
    previewBusy,
    openPreview,
    openLibPreview,
  };
}
