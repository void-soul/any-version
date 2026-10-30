import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { type BuiltinSkillView, type SkillSourceItem, type SkillUpdateInfo } from "./SkillManager.model";

/** Owns the SkillInstallation feature, independently of page rendering. */
export function useSkillInstallation({ loadSkills }: {
  loadSkills: () => Promise<void>;
}) {
  const { t } = useTranslation();

  // ── 市场安装 ──
  const [installInput, setInstallInput] = useState('');

  const [installing, setInstalling] = useState(false);

  const [installLog, setInstallLog] = useState('');

  const [installErr, setInstallErr] = useState('');

  // 预置来源（点一下填进输入框，省得用户自己翻仓库地址）
  const [sources, setSources] = useState<SkillSourceItem[]>([]);

  // 可更新的技能（安装时记了来源的才能比对）
  const [updates, setUpdates] = useState<SkillUpdateInfo[]>([]);

  const [checkingUpdates, setCheckingUpdates] = useState(false);

  // ── 内置技能 ──
  const [builtins, setBuiltins] = useState<BuiltinSkillView[]>([]);

  const [builtinBusy, setBuiltinBusy] = useState<string | null>(null);

  const [builtinResult, setBuiltinResult] = useState<{ ok: boolean; msg: string } | null>(null);

  /** 预置来源：进来就拉一次，失败就静默（不给用户添噪音） */
  useEffect(() => {
    invoke<SkillSourceItem[]>('get_skill_sources').then(setSources).catch(() => setSources([]));
  }, []);

  /** 检查已装技能是否有更新：后端按记录的来源 `git ls-remote` 比对，不 clone */
  const checkUpdates = async () => {
    setCheckingUpdates(true);
    try {
      setUpdates(await invoke<SkillUpdateInfo[]>('check_skill_updates'));
    } catch {
      setUpdates([]);
    } finally {
      setCheckingUpdates(false);
    }
  };

  // 市场安装
  const startInstall = async () => {
    const src = installInput.trim();
    if (!src) return;
    setInstalling(true);
    setInstallLog(t("skillmgr.installStart") + '\n');
    setInstallErr('');
    try {
      await invoke('install_skill_from_online', { source: src });
    } catch (e: any) {
      setInstalling(false);
      setInstallErr(String(e));
    }
  };

  // 更新内置技能：后端会先删掉旧目录再按随包内容重写（否则新版移除的文件会残留）。
  // 不走「先 uninstall 再 install」的前端两步 —— 那样中间会出现技能库实体短暂消失，
  // 此刻所有项目里的 junction 都是 broken，失败时还会停在那个状态。
  const updateBuiltin = async (id: string) => {
    setBuiltinBusy(id);
    setBuiltinResult(null);
    try {
      const path = await invoke<string>('update_builtin_skill', { skillId: id });
      setBuiltinResult({ ok: true, msg: t("skillmgr.builtinUpdated", { path }) });
      loadSkills();
    } catch (e: any) {
      setBuiltinResult({ ok: false, msg: String(e) });
    }
    setBuiltinBusy(null);
  };

  // 安装内置技能：直接落进技能库。装完它就是普通技能，由用户显式安装给某个项目
  const installBuiltin = async (id: string) => {
    setBuiltinBusy(id);
    setBuiltinResult(null);
    try {
      const path = await invoke<string>('install_builtin_skill', { skillId: id });
      setBuiltinResult({ ok: true, msg: t("skillmgr.builtinInstalled", { path }) });
      loadSkills();
    } catch (e: any) {
      setBuiltinResult({ ok: false, msg: String(e) });
    }
    setBuiltinBusy(null);
  };
  return {
    installInput,
    setInstallInput,
    installing,
    setInstalling,
    installLog,
    setInstallLog,
    installErr,
    sources,
    updates,
    checkingUpdates,
    builtins,
    setBuiltins,
    builtinBusy,
    builtinResult,
    checkUpdates,
    startInstall,
    updateBuiltin,
    installBuiltin,
  };
}
