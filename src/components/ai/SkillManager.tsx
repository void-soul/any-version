import { useState, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { open } from '@tauri-apps/plugin-dialog';
import { listen as listenEvent } from '@tauri-apps/api/event';
import { Note, ResultNote } from '../shared/Note';
import {
  Search, Tag, Store, Download, Trash2,
  CheckCircle, AlertTriangle, ExternalLink, X, Package, Loader2,
  ChevronDown, Settings2, Filter, Unlink, Sparkles, FolderOpen, Plus, Link2
} from 'lucide-react';
import { alertError } from "../shared/ThemedAlert";

// ─── 类型 ───
/** 预置技能来源（后端 get_skill_sources） */
interface SkillSourceItem {
  label: string;
  repo: string;
  hint: string;
}

/** 有更新的技能（后端 check_skill_updates；camelCase） */
interface SkillUpdateInfo {
  skillId: string;
  name: string;
  source: string;
  current: string;
  latest: string;
}

interface SkillEntry {
  id: string;
  name: string;
  description: string;
  installedAt: string;
  installMethod: string;
  /** 用户自定义分类（来自 .meta.json） */
  category: string;
  /** 用户自定义标签（来自 .meta.json） */
  tags: string[];
}

/** 内置技能（随 Kira 分发，装进技能库后由用户安装给项目） */
interface BuiltinSkillView {
  id: string;
  name: string;
  description: string;
}

/** 项目 = 一个目录，技能装给它就只在该目录生效（后端 list_projects / add_project） */
interface SkillProject {
  id: string;
  path: string;
  name: string;
  addedAt: string;
}

/** 项目里已安装的技能（后端 list_project_skills；camelCase） */
interface ProjectSkillEntry {
  skillId: string;
  name: string;
  description: string;
  /** 'linked' 已生效 · 'broken' 悬空（技能库里的技能已被删除） */
  state: string;
}

type TabKey = 'skills' | 'projects';

const MARKET_SOURCES = [
  { name: 'skills.sh 官网', desc: 'Agent Skills 官方生态与规范', url: 'https://skills.sh' },
  { name: 'Anthropic 官方 Skills', desc: 'anthropics/skills 仓库示例技能', url: 'https://github.com/anthropics/skills' },
  { name: 'GitHub 主题搜索', desc: '搜索社区维护的 agentskills 仓库', url: 'https://github.com/search?q=agentskills&type=repositories' },
];

export default function SkillManager() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<TabKey>('skills');

  // ── 技能数据 ──
  const [skills, setSkills] = useState<SkillEntry[]>([]);
  const [skillLoading, setSkillLoading] = useState(true);
  const [search, setSearch] = useState('');
  const [selCat, setSelCat] = useState<string | null>(null);
  const [selTags, setSelTags] = useState<string[]>([]);
  const [editing, setEditing] = useState<SkillEntry | null>(null);
  const [edCat, setEdCat] = useState('');
  const [edTags, setEdTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState('');
  const [savingMeta, setSavingMeta] = useState(false);
  const [skillMsg, setSkillMsg] = useState<{ id: string; msg: string; ok: boolean } | null>(null);

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
  // 市场 / 内置技能并入技能库 tab，按需展开（它们只是「获取技能的渠道」，不是并列的管理维度）
  const [showMarket, setShowMarket] = useState(false);
  const [showBuiltin, setShowBuiltin] = useState(false);

  // ── 项目（技能的作用域） ──
  const [projects, setProjects] = useState<SkillProject[]>([]);
  /** 技能库实际路径（如实显示，别让用户猜装到哪了） */
  const [skillsDir, setSkillsDir] = useState('');
  const [projMsg, setProjMsg] = useState<{ id: string; msg: string; ok: boolean } | null>(null);
  /** 展开查看某个项目装了哪些技能 */
  const [expandedProj, setExpandedProj] = useState<string | null>(null);
  /** projectId -> 该项目已装的技能（展开时才拉） */
  const [projSkills, setProjSkills] = useState<Record<string, ProjectSkillEntry[]>>({});
  /** projectId -> 技能库的技能勾选状态 */
  const [picked, setPicked] = useState<Record<string, Set<string>>>({});
  const [projBusy, setProjBusy] = useState<string | null>(null);

  // ── 加载 ──
  const loadSkills = async () => {
    setSkillLoading(true);
    try {
      const data = await invoke<SkillEntry[]>('get_skill_overview');
      setSkills(data);
    } catch (e) {
      console.error("加载 AI 技能概览失败:", e);
    }
    try {
      setSkillsDir(await invoke<string>('get_skills_dir'));
    } catch {
      setSkillsDir('');
    }
    setSkillLoading(false);
  };

  const loadProjects = async () => {
    try {
      setProjects(await invoke<SkillProject[]>('list_projects'));
    } catch (e) {
      console.error("加载项目列表失败:", e);
    }
  };

  /** 展开项目时按需拉它的技能清单（不预先拉所有项目的，避免无用 IO） */
  const loadProjectSkills = async (pid: string) => {
    try {
      const list = await invoke<ProjectSkillEntry[]>('list_project_skills', { projectId: pid });
      setProjSkills((m) => ({ ...m, [pid]: list }));
      // 勾选状态 = 已安装的技能
      setPicked((m) => ({ ...m, [pid]: new Set(list.filter((e) => e.state === 'linked').map((e) => e.skillId)) }));
    } catch (e: any) {
      setProjMsg({ id: pid, msg: String(e), ok: false });
    }
  };

  useEffect(() => {
    loadSkills();
    loadProjects();
    invoke<BuiltinSkillView[]>('list_builtin_skills')
      .then(setBuiltins)
      .catch(() => setBuiltins([]));
  }, []);

  // 监听安装进度
  useEffect(() => {
    const unlisten = listenEvent<{ stage: string; message: string }>('skill-install-progress', (e) => {
      setInstallLog((prev) => prev + `[${e.payload.stage}] ${e.payload.message}\n`);
      if (e.payload.stage === 'done') {
        setInstalling(false);
        loadSkills();
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // 分类与标签导出
  const allCats = useMemo(() => {
    const set = new Set<string>();
    for (const s of skills) if (s.category) set.add(s.category);
    return Array.from(set).sort();
  }, [skills]);

  const allTags = useMemo(() => {
    const set = new Set<string>();
    for (const s of skills) for (const t of s.tags) set.add(t);
    return Array.from(set).sort();
  }, [skills]);

  // 筛选技能列表
  const filtered = useMemo(() => {
    return skills.filter((s) => {
      if (search.trim()) {
        const q = search.toLowerCase();
        const m = s.id.toLowerCase().includes(q) || s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q);
        if (!m) return false;
      }
      if (selCat && s.category !== selCat) return false;
      if (selTags.length > 0 && !selTags.every((t) => s.tags.includes(t))) return false;
      return true;
    });
  }, [skills, search, selCat, selTags]);

  const hasFilter = search || selCat || selTags.length > 0;
  const clearFilter = () => { setSearch(''); setSelCat(null); setSelTags([]); };

  // 卸载技能
  const removeSkill = async (id: string) => {
    if (!confirm(t("skillmgr.uninstallConfirm", { id }))) return;
    try {
      await invoke('uninstall_skill', { skillId: id });
      setSkillMsg({ id, msg: t("skillmgr.deleted"), ok: true });
      loadSkills();
    } catch (e: any) {
      setSkillMsg({ id, msg: String(e), ok: false });
    }
  };

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

  // ── 项目操作 ──

  /** 添加项目：让用户直接选一个目录 */
  const addProject = async () => {
    try {
      const picked = await open({ directory: true, multiple: false, title: t("skillmgr.projPickTitle") });
      if (typeof picked !== 'string' || !picked) return;
      await invoke('add_project', { path: picked });
      await loadProjects();
      setProjMsg({ id: '*', msg: t("skillmgr.projAdded", { name: picked.split(/[\\/]/).filter(Boolean).pop() }), ok: true });
    } catch (e: any) {
      setProjMsg({ id: '*', msg: String(e), ok: false });
    }
  };

  /** 移除项目条目（只删记录，项目目录里的 .agents/skills 一概不动） */
  const removeProject = async (p: SkillProject) => {
    if (!confirm(t("skillmgr.projRemoveConfirm", { name: p.name || p.path }))) return;
    try {
      await invoke('remove_project', { id: p.id });
      setProjMsg({ id: p.id, msg: t("skillmgr.projRemoved"), ok: true });
      setExpandedProj((cur) => (cur === p.id ? null : cur));
      await loadProjects();
    } catch (e: any) {
      setProjMsg({ id: p.id, msg: String(e), ok: false });
    }
  };

  const toggleExpand = async (p: SkillProject) => {
    if (expandedProj === p.id) {
      setExpandedProj(null);
      return;
    }
    setExpandedProj(p.id);
    if (!projSkills[p.id]) await loadProjectSkills(p.id);
  };

  const togglePick = (pid: string, skillId: string) => {
    setPicked((m) => {
      const next = new Set(m[pid] || []);
      if (next.has(skillId)) next.delete(skillId);
      else next.add(skillId);
      return { ...m, [pid]: next };
    });
  };

  /**
   * 按勾选差异批量安装/卸载。
   *
   * 勾选是「期望状态」，与实况的差异即本次要做的动作 —— 这样用户勾一下就够，
   * 不必为每个技能点一次「装/卸」。
   */
  const applyPicks = async (p: SkillProject) => {
    const installed = new Set((projSkills[p.id] || []).filter((e) => e.state === 'linked').map((e) => e.skillId));
    const want = picked[p.id] || new Set<string>();
    const toInstall = [...want].filter((id) => !installed.has(id));
    const toRemove = [...installed].filter((id) => !want.has(id));
    if (toInstall.length === 0 && toRemove.length === 0) {
      setProjMsg({ id: p.id, msg: t("skillmgr.projNoChange"), ok: true });
      return;
    }
    setProjBusy(p.id);
    const errors: string[] = [];
    for (const id of toInstall) {
      try {
        await invoke('install_skill_to_project', { skillId: id, projectId: p.id });
      } catch (e: any) {
        errors.push(`${id}: ${String(e)}`);
      }
    }
    for (const id of toRemove) {
      try {
        await invoke('remove_skill_from_project', { skillId: id, projectId: p.id });
      } catch (e: any) {
        errors.push(`${id}: ${String(e)}`);
      }
    }
    setProjBusy(null);
    await loadProjectSkills(p.id);
    if (errors.length === 0) {
      setProjMsg({
        id: p.id,
        msg: t("skillmgr.projApplied", { add: toInstall.length, del: toRemove.length }),
        ok: true,
      });
    } else {
      setProjMsg({ id: p.id, msg: errors.join('；'), ok: false });
    }
  };

  /** 清掉悬空链接（技能库里的技能已被删除，junction 还留在项目里） */
  const cleanupBroken = async (p: SkillProject) => {
    const broken = (projSkills[p.id] || []).filter((e) => e.state === 'broken');
    if (broken.length === 0) return;
    setProjBusy(p.id);
    for (const e of broken) {
      try {
        await invoke('remove_skill_from_project', { skillId: e.skillId, projectId: p.id });
      } catch (e: any) {
        setProjMsg({ id: p.id, msg: String(e), ok: false });
      }
    }
    setProjBusy(null);
    await loadProjectSkills(p.id);
  };

  return (
    <div className="flex flex-col h-full text-slate-200 select-none">
      {/* 顶部导航 Tab */}
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
        <div className="text-tiny text-slate-500 font-mono truncate max-w-[50%]" title={skillsDir}>
          {t("skillmgr.libraryDir")}<code className="text-slate-400">{skillsDir || '…'}</code>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        {/* ════════ 项目 Tab ════════ */}
        {tab === 'projects' && (
          <div className="space-y-3 max-w-4xl">
            <Note tone="info" title={t("skillmgr.projHintTitle")}>
              <p>{t("skillmgr.projHint1")}</p>
            </Note>

            <div className="flex items-center justify-between">
              <span className="text-body font-bold text-slate-300">{t("skillmgr.projTitle")}</span>
              <button
                onClick={() => void addProject()}
                className="flex items-center gap-1 px-2.5 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-body font-semibold text-white cursor-pointer transition-all"
              >
                <Plus className="w-3.5 h-3.5" /> {t("skillmgr.projAdd")}
              </button>
            </div>

            {projMsg?.id === '*' && (
              <div className={`p-2.5 rounded-ctl text-body flex items-center gap-2 ${projMsg.ok ? 'bg-emerald-500/10 text-emerald-400' : 'bg-red-500/10 text-red-400'}`}>
                {projMsg.ok ? <CheckCircle className="w-3.5 h-3.5" /> : <AlertTriangle className="w-3.5 h-3.5" />}
                <span className="break-all">{projMsg.msg}</span>
              </div>
            )}

            {projects.length === 0 ? (
              <div className="text-center py-16 text-slate-500 text-sm">{t("skillmgr.projEmpty")}</div>
            ) : (
              <div className="space-y-2.5">
                {projects.map((p) => {
                  const expanded = expandedProj === p.id;
                  const installed = projSkills[p.id] || [];
                  const linkedCount = installed.filter((e) => e.state === 'linked').length;
                  const broken = installed.filter((e) => e.state === 'broken');
                  const want = picked[p.id] || new Set<string>();
                  // 勾选的是「期望状态」；与实况有差异才算脏，禁用「应用」按钮
                  const dirty = expanded && (() => {
                    const cur = new Set(installed.filter((e) => e.state === 'linked').map((e) => e.skillId));
                    if (cur.size !== want.size) return true;
                    for (const id of cur) if (!want.has(id)) return true;
                    return false;
                  })();
                  return (
                    <div key={p.id} className="rounded-card bg-white/[0.03] border border-white/10 overflow-hidden">
                      <div className="flex items-center justify-between gap-3 p-3.5">
                        <button
                          onClick={() => void toggleExpand(p)}
                          className="flex items-center gap-3 min-w-0 flex-1 text-left cursor-pointer"
                        >
                          <span className="flex-shrink-0 w-8 h-8 rounded-ctl bg-white/5 border border-white/10 flex items-center justify-center">
                            <FolderOpen className="w-4 h-4 text-[var(--module-accent)]" />
                          </span>
                          <span className="min-w-0">
                            <span className="flex items-center gap-2">
                              <span className="text-body font-semibold text-slate-100 truncate">{p.name || p.path}</span>
                              <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-emerald-500/15 text-emerald-400 flex-shrink-0">
                                {linkedCount}
                              </span>
                              {broken.length > 0 && (
                                <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-amber-500/15 text-amber-400 flex-shrink-0">
                                  {t("skillmgr.projBrokenBadge", { count: broken.length })}
                                </span>
                              )}
                            </span>
                            <span className="block text-tiny text-slate-500 font-mono truncate mt-0.5">{p.path}</span>
                          </span>
                          <ChevronDown className={`w-3.5 h-3.5 text-slate-500 flex-shrink-0 transition-transform ${expanded ? 'rotate-180' : ''}`} />
                        </button>
                        <button
                          onClick={() => void removeProject(p)}
                          disabled={projBusy === p.id}
                          className="text-slate-500 hover:text-red-400 cursor-pointer disabled:opacity-40 flex-shrink-0"
                          title={t("skillmgr.projRemove")}
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>

                      {expanded && (
                        <div className="border-t border-white/5 p-3.5 space-y-3">
                          {/* 悬空链接：技能库里的技能已被删除，项目里的 junction 还留着 */}
                          {broken.length > 0 && (
                            <div className="p-2.5 rounded-ctl bg-amber-500/10 text-amber-400 text-tiny flex items-center justify-between gap-2">
                              <span className="min-w-0 break-words">{t("skillmgr.projBrokenHint", { names: broken.map((b) => b.skillId).join('、') })}</span>
                              <button
                                onClick={() => void cleanupBroken(p)}
                                disabled={projBusy === p.id}
                                className="px-2 py-0.5 rounded bg-amber-500/20 hover:bg-amber-500/30 flex items-center gap-1 cursor-pointer disabled:opacity-50 flex-shrink-0"
                              >
                                <Unlink className="w-3 h-3" /> {t("skillmgr.projBrokenClean")}
                              </button>
                            </div>
                          )}

                          {/* 勾选 = 期望状态；点「应用」才批量增删，避免每装一个技能点一次 */}
                          {skills.length === 0 ? (
                            <div className="text-tiny text-slate-500">{t("skillmgr.projNoSkills")}</div>
                          ) : (
                            <>
                              <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-1.5 max-h-64 overflow-y-auto">
                                {skills.map((s) => {
                                  const on = want.has(s.id);
                                  return (
                                    <button
                                      key={s.id}
                                      onClick={() => togglePick(p.id, s.id)}
                                      className={`flex items-center gap-1.5 px-2 py-1.5 rounded-ctl text-left cursor-pointer transition-colors ${
                                        on ? 'bg-emerald-500/15 text-emerald-300' : 'bg-white/5 text-slate-400 hover:bg-white/10'
                                      }`}
                                      title={s.description || s.id}
                                    >
                                      <span className={`w-3 h-3 rounded-sm flex-shrink-0 border ${on ? 'bg-emerald-500 border-emerald-400' : 'border-white/20'}`} />
                                      <span className="text-tiny truncate">{s.name || s.id}</span>
                                    </button>
                                  );
                                })}
                              </div>
                              <div className="flex items-center justify-end gap-2 pt-1">
                                <span className="text-micro text-slate-500">{t("skillmgr.projPicked", { n: want.size })}</span>
                                <button
                                  onClick={() => void applyPicks(p)}
                                  disabled={!dirty || projBusy === p.id}
                                  className="flex items-center gap-1 px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-40 disabled:bg-slate-700 text-body font-semibold text-white cursor-pointer transition-all"
                                >
                                  {projBusy === p.id ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Link2 className="w-3.5 h-3.5" />}
                                  {t("skillmgr.projApply")}
                                </button>
                              </div>
                            </>
                          )}

                          {projMsg?.id === p.id && (
                            <div className={`text-tiny break-all ${projMsg.ok ? 'text-emerald-400' : 'text-red-400'}`}>{projMsg.msg}</div>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {/* ════════ 技能 Tab ════════ */}
        {tab === 'skills' && (
          <div className="space-y-3">
            {/* 检索筛选栏 */}
            <div className="flex flex-wrap items-center gap-2">
              <div className="relative flex-1 min-w-[200px]">
                <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
                <input
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder={t("skillmgr.searchPh")}
                  className="w-full pl-8 pr-2 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-body text-slate-200 placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent-ring)]"
                />
              </div>
              <div className="relative">
                <select
                  value={selCat ?? ''}
                  onChange={(e) => setSelCat(e.target.value || null)}
                  className="appearance-none pl-7 pr-7 py-1.5 rounded-ctl bg-slate-800 border border-white/10 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent-ring)] cursor-pointer"
                >
                  <option value="">{t("skillmgr.allCategories")}</option>
                  {allCats.map((c) => <option key={c} value={c}>{c}</option>)}
                </select>
                <Filter className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
                <ChevronDown className="absolute right-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500 pointer-events-none" />
              </div>
              {hasFilter && (
                <button onClick={clearFilter} className="px-2 py-1.5 rounded-ctl bg-white/5 hover:bg-white/10 text-tiny text-slate-400 flex items-center gap-1 cursor-pointer">
                  <X className="w-3 h-3" /> {t("skillmgr.clearFilter")}
                </button>
              )}
            </div>

            {/* 标签 chips */}
            {allTags.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5">
                <Tag className="w-3.5 h-3.5 text-slate-500" />
                {allTags.map((t) => {
                  const active = selTags.includes(t);
                  return (
                    <button
                      key={t}
                      onClick={() => setSelTags(active ? selTags.filter((x) => x !== t) : [...selTags, t])}
                      className={`vex-chip px-2 py-0.5 rounded-full text-tiny font-medium transition-all cursor-pointer ${
                        active
                          ? 'vex-chip-active bg-[color-mix(in_srgb,var(--module-accent)_30%,transparent)] text-[var(--module-accent)] border border-[var(--module-accent-ring)]'
                          : 'bg-white/5 text-slate-400 hover:bg-white/10 border border-transparent'
                      }`}
                    >
                      {t}
                    </button>
                  );
                })}
              </div>
            )}

            {/* 概览数据提示 + 获取渠道入口 */}
            <div className="flex items-center justify-between gap-2 text-tiny text-slate-500">
              <span>{skillLoading ? t("skillmgr.loading") : `${t("skillmgr.skillsCount", { count: filtered.length })}${hasFilter ? t("skillmgr.filteredCount", { total: skills.length }) : ''}`}</span>
              <div className="flex items-center gap-2">
                <span className="truncate max-w-[240px]" title={skillsDir}>{t("skillmgr.source")}<code className="text-slate-400">{skillsDir || '…'}</code></span>
                <button
                  onClick={() => { setShowMarket((v) => !v); setShowBuiltin(false); }}
                  className={`flex items-center gap-1 px-2 py-1 rounded-ctl cursor-pointer transition-colors ${showMarket ? 'bg-[var(--module-accent-soft)] text-[var(--module-accent)]' : 'bg-white/5 hover:bg-white/10 text-slate-300'}`}
                >
                  <Store className="w-3 h-3" /> {t("skillmgr.tabMarket")}
                </button>
                <button
                  onClick={() => { setShowBuiltin((v) => !v); setShowMarket(false); }}
                  className={`flex items-center gap-1 px-2 py-1 rounded-ctl cursor-pointer transition-colors ${showBuiltin ? 'bg-[var(--module-accent-soft)] text-[var(--module-accent)]' : 'bg-white/5 hover:bg-white/10 text-slate-300'}`}
                >
                  <Sparkles className="w-3 h-3" /> {t("skillmgr.tabBuiltin")}
                </button>
              </div>
            </div>

            {/* 技能网格 */}
            {!skillLoading && filtered.length === 0 && (
              <div className="text-center py-16 text-slate-500 text-sm">
                {skills.length === 0
                  ? t("skillmgr.emptyAll")
                  : t("skillmgr.emptyFiltered")}
              </div>
            )}

            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
              {filtered.map((s) => (
                <div key={s.id} className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 flex flex-col gap-2 hover:border-white/20 transition-all">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <div className="text-sm font-semibold text-slate-100 truncate">{s.name || s.id}</div>
                      <div className="text-tiny text-slate-500 font-mono truncate">{s.id}</div>
                    </div>
                    {s.category && (
                      <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-[var(--module-accent-soft)] text-[var(--module-accent)] flex-shrink-0">
                        {s.category}
                      </span>
                    )}
                  </div>
                  <p className="text-caption text-slate-400 line-clamp-2 min-h-[28px]">{s.description || t("skillmgr.noDesc")}</p>
                  {s.tags.length > 0 && (
                    <div className="flex flex-wrap gap-1">
                      {s.tags.map((t) => (
                        <span key={t} className="px-1.5 py-0.5 rounded text-micro bg-white/5 text-slate-400">
                          #{t}
                        </span>
                      ))}
                    </div>
                  )}
                  <div className="flex items-center justify-between pt-2 border-t border-white/5 text-tiny text-slate-500">
                    <span>{s.installMethod === 'managed' ? t("skillmgr.managedLib") : s.installMethod}</span>
                    <div className="flex items-center gap-2">
                      <button onClick={() => openEdit(s)} className="text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
                        <Settings2 className="w-3.5 h-3.5" />
                      </button>
                      <button onClick={() => removeSkill(s.id)} className="text-slate-500 hover:text-red-400 cursor-pointer">
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                  {skillMsg?.id === s.id && (
                    <div className={`text-tiny ${skillMsg.ok ? 'text-emerald-400' : 'text-red-400'}`}>{skillMsg.msg}</div>
                  )}
                </div>
              ))}
            </div>
        {/* ════════ 从市场获取 ════════ */}
        {showMarket && (
          <div className="space-y-4 max-w-3xl">
            <div className="rounded-card bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] p-3.5 text-caption text-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] leading-relaxed">
              {t("skillmgr.marketHint1")}
              {t("skillmgr.marketHint2")}
            </div>

            {/* 来源列表 */}
            <div>
              <div className="text-body font-semibold text-slate-300 mb-2 flex items-center gap-1.5">
                <Store className="w-3.5 h-3.5 text-[var(--module-accent)]" /> {t("skillmgr.marketSources")}
              </div>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-2.5">
                {MARKET_SOURCES.map((src) => (
                  <a
                    key={src.url}
                    href={src.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    onClick={(e) => { e.preventDefault(); void openUrl(src.url); }}
                    className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 hover:border-[var(--module-accent-ring)] transition-all group"
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-body font-semibold text-slate-100">{t(src.name)}</span>
                      <ExternalLink className="w-3.5 h-3.5 text-slate-500 group-hover:text-[var(--module-accent)] transition-colors" />
                    </div>
                    <p className="text-tiny text-slate-400 mt-1">{t(src.desc)}</p>
                  </a>
                ))}
              </div>
            </div>

            {/* 在线安装 */}
            <div>
              <div className="text-body font-semibold text-slate-300 mb-2 flex items-center gap-1.5">
                <Download className="w-3.5 h-3.5 text-[var(--module-accent)]" /> {t("skillmgr.installFrom")}
              </div>
              <div className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 space-y-2.5">
                <input
                  value={installInput}
                  onChange={(e) => setInstallInput(e.target.value)}
                  placeholder="如: owner/repo / https://github.com/... / 本地技能路径"
                  className="w-full px-3 py-2 rounded-ctl bg-white/5 border border-white/10 text-body text-slate-200 placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent-ring)]"
                />
                <button
                  onClick={startInstall}
                  disabled={installing || !installInput.trim()}
                  className="w-full px-3 py-2.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-body font-semibold text-white flex items-center justify-center gap-2 transition-all cursor-pointer"
                >
                  {installing ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
                  {installing ? t("skillmgr.installing") : t("skillmgr.installTo")}
                </button>
                {/* 预置来源：点一下把 owner/repo 填进输入框 */}
                {sources.length > 0 && (
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="text-micro text-slate-500">{t("skillmgr.presetSources")}</span>
                    {sources.map((s) => (
                      <button key={s.repo} onClick={() => setInstallInput(s.repo)} title={`${s.repo} — ${s.hint}`}
                        className="px-2 py-0.5 rounded-md text-micro bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer transition-colors">
                        {s.label}
                      </button>
                    ))}
                  </div>
                )}
                {/* 检查更新：只对「安装时记了来源」的技能有效 */}
                <div className="flex items-center gap-2 pt-0.5">
                  <button onClick={() => void checkUpdates()} disabled={checkingUpdates}
                    className="px-2.5 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer disabled:opacity-40 transition-colors">
                    {checkingUpdates ? <Loader2 className="w-3 h-3 inline animate-spin" /> : null} {t("skillmgr.checkUpdates")}
                  </button>
                  <span className="text-micro text-slate-500">
                    {updates.length > 0
                      ? t("skillmgr.updatesAvailable", { count: updates.length })
                      : t("skillmgr.noUpdates")}
                  </span>
                </div>
                {updates.length > 0 && (
                  <div className="rounded-ctl border border-white/10 divide-y divide-white/5">
                    {updates.map((u) => (
                      <div key={u.skillId} className="px-2.5 py-1.5 flex items-center gap-2 text-micro">
                        <span className="text-slate-300 truncate">{u.name}</span>
                        <span className="text-slate-600 font-mono truncate">{u.source}</span>
                        <span className="flex-1" />
                        <span className="text-amber-400/80 font-mono">{u.current.slice(0, 7) || "—"} → {u.latest.slice(0, 7)}</span>
                        <button onClick={() => setInstallInput(u.source)}
                          className="px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
                          {t("skillmgr.updateNow")}
                        </button>
                      </div>
                    ))}
                  </div>
                )}
                {installErr && (
                  <div className="p-2.5 rounded-ctl text-body flex items-center gap-2 bg-red-500/10 text-red-400">
                    <AlertTriangle className="w-3.5 h-3.5" /> {installErr}
                  </div>
                )}
                {installLog && (
                  <pre className="text-tiny text-slate-400 bg-black/30 rounded-ctl p-2.5 max-h-40 overflow-y-auto whitespace-pre-wrap font-mono border border-white/5">{installLog}</pre>
                )}
              </div>
            </div>
          </div>
        )}


        {/* ════════ 内置技能 ════════ */}
        {showBuiltin && (
          <div className="space-y-4 max-w-4xl">
            <Note tone="info" title={t("skillmgr.builtinTitle")}>
              <p>{t("skillmgr.builtinHint1")}</p>
              <p>{t("skillmgr.builtinHint2")}</p>
            </Note>

            {builtins.length === 0 ? (
              <div className="text-center py-16 text-slate-500 text-sm">{t("skillmgr.builtinEmpty")}</div>
            ) : (
              <div className="grid grid-cols-1 md:grid-cols-2 gap-2.5">
                {builtins.map((b) => (
                  <div key={b.id} className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 space-y-2">
                    <div className="flex items-center gap-2">
                      <Sparkles className="w-3.5 h-3.5 text-[var(--module-accent)] flex-shrink-0" />
                      <span className="text-body font-semibold text-slate-100 truncate">{b.name}</span>
                      <code className="ml-auto text-micro text-slate-600 font-mono truncate">{b.id}</code>
                    </div>
                    <p className="text-tiny text-slate-400 leading-relaxed">{b.description}</p>
                    <button
                      onClick={() => void installBuiltin(b.id)}
                      disabled={builtinBusy !== null}
                      className="w-full px-3 py-2 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-50 text-body font-semibold text-white flex items-center justify-center gap-2 transition-all cursor-pointer"
                    >
                      {builtinBusy === b.id
                        ? <Loader2 className="w-3.5 h-3.5 animate-spin" />
                        : <Download className="w-3.5 h-3.5" />}
                      {builtinBusy === b.id ? t("skillmgr.builtinInstalling") : t("skillmgr.builtinInstall")}
                    </button>
                  </div>
                ))}
              </div>
            )}

            {builtinResult && <ResultNote ok={builtinResult.ok} message={builtinResult.msg} />}
          </div>
        )}

          </div>
        )}

      </div>

      {/* 编辑分类/标签弹窗 */}
      {editing && (
        <div className="fixed inset-0 z-50 modal-mask bg-black/60 backdrop-blur-xs flex items-center justify-center p-4">
          <div className="rounded-card ui-input p-4 w-full max-w-md space-y-3 shadow-2xl">
            <div className="flex items-center justify-between border-b border-white/5 pb-2">
              <span className="text-body font-bold text-slate-200">{t("skillmgr.editAttr", { id: editing.id })}</span>
              <button onClick={() => setEditing(null)} className="text-slate-500 hover:text-slate-300">
                <X className="w-4 h-4" />
              </button>
            </div>

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

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-white/5">
              <button onClick={() => setEditing(null)} className="px-3 py-1.5 rounded text-body text-slate-400 hover:text-slate-200 cursor-pointer">
                {t("skillmgr.cancel")}
              </button>
              <button
                onClick={saveMeta}
                disabled={savingMeta}
                className="px-3 py-1.5 rounded bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-body font-semibold text-white flex items-center gap-1.5 cursor-pointer"
              >
                {savingMeta ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : null}
                {t("skillmgr.save")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
