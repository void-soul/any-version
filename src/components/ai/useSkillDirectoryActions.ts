import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { type ProjectSkillEntry } from "./SkillManager.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the SkillDirectoryActions feature, independently of page rendering. */
export function useSkillDirectoryActions({ dirDialog, setDirDialog, setSkillsDir, loadSkills, loadProjects, setProjSkills, setProjMsg }: {
  dirDialog: { path: string; busy: boolean; msg: string | null; } | null;
  setDirDialog: Dispatch<SetStateAction<{ path: string; busy: boolean; msg: string | null; } | null>>;
  setSkillsDir: Dispatch<SetStateAction<string>>;
  loadSkills: () => Promise<void>;
  loadProjects: () => Promise<void>;
  setProjSkills: Dispatch<SetStateAction<Record<string, ProjectSkillEntry[]>>>;
  setProjMsg: Dispatch<SetStateAction<{ id: string; msg: string; ok: boolean; } | null>>;
}) {
  const { t } = useTranslation();

// ── 项目操作 ──

  /**
   * 改技能库位置。
   *
   * 后端会连着搬三样东西：技能实体目录、skills.json 里记录的 directory、
   * 以及各项目里指向旧库的 junction（目录联接记绝对路径，不重建就全成悬空）。
   * 所以这里成功提示要报迁移数量，让用户知道发生了什么。
   */
  const saveSkillsDir = async () => {
    if (!dirDialog) return;
    setDirDialog((d) => (d ? { ...d, busy: true, msg: null } : d));
    try {
      const r = await invoke<{ dir: string; moved: number; relinked: number; unchanged: boolean }>(
        'set_skills_dir',
        { path: dirDialog.path },
      );
      setSkillsDir(r.dir);
      setDirDialog(null);
      // 技能库换位置了，列表和项目清单都得重拉
      await Promise.all([loadSkills(), loadProjects()]);
      setProjSkills({});
      setProjMsg({
        id: '*',
        msg: r.unchanged
          ? t("skillmgr.dirSame")
          : t("skillmgr.dirChanged", { path: r.dir, n: r.moved, links: r.relinked }),
        ok: true,
      });
    } catch (e: any) {
      setDirDialog((d) => (d ? { ...d, busy: false, msg: String(e) } : d));
    }
  };

  const pickSkillsDir = async () => {
    try {
      const picked = await open({ directory: true, multiple: false, title: t("skillmgr.dirPick") });
      if (typeof picked === 'string' && picked) {
        setDirDialog((d) => (d ? { ...d, path: picked, msg: null } : d));
      }
    } catch {
      /* 用户取消 */
    }
  };
  return {
    saveSkillsDir,
    pickSkillsDir,
  };
}
