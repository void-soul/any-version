import { useState, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import { open } from '@tauri-apps/plugin-dialog';
import { listen as listenEvent } from '@tauri-apps/api/event';
import { Note, ResultNote } from '../shared/Note';
import {
  Search, Tag, Boxes, Store, Download, Trash2,
  CheckCircle, AlertTriangle, ExternalLink, X, Package, Loader2,
  ChevronDown, Settings2, Filter, Link2, Unlink, Sparkles
} from 'lucide-react';
import { DetectedAiTool } from './types';
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

interface SkillToolStatusView {
  toolId: string;
  label: string;
  skillsDir: string;
  /** 'managed' | 'unmanaged' | 'empty' */
  status: string;
  skillCount: number;
  symlinkEnabled: boolean;
  readsAgentsSkills: boolean;
  /** per-skill 已部署（junction 指向仓库）的技能数量 */
  deployedCount?: number;
}

/** 内置技能（随 Kira 分发，可装到用户指定目录） */
interface BuiltinSkillView {
  id: string;
  name: string;
  description: string;
}

type TabKey = 'skills' | 'tools' | 'market' | 'builtin';

const MARKET_SOURCES = [
  { name: 'skills.sh 官网', desc: 'Agent Skills 官方生态与规范', url: 'https://skills.sh' },
  { name: 'Anthropic 官方 Skills', desc: 'anthropics/skills 仓库示例技能', url: 'https://github.com/anthropics/skills' },
  { name: 'GitHub 主题搜索', desc: '搜索社区维护的 agentskills 仓库', url: 'https://github.com/search?q=agentskills&type=repositories' },
];

function StatusBadge({ status }: { status: string }) {
  const { t } = useTranslation();
  if (status === 'managed') return <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-emerald-500/15 text-emerald-400">{t("skillmgr.statusLinked")}</span>;
  if (status === 'unmanaged') return <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-amber-500/15 text-amber-400">{t("skillmgr.statusPrivate")}</span>;
  return <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-slate-500/15 text-slate-400">{t("skillmgr.statusUnmanaged")}</span>;
}

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

  // ── 工具数据 ──
  const [tools, setTools] = useState<DetectedAiTool[]>([]);
  const [toolStatus, setToolStatus] = useState<SkillToolStatusView[]>([]);
  const [_toolLoading, setToolLoading] = useState(true);
  const [toolMsg, setToolMsg] = useState<{ id: string; msg: string; ok: boolean } | null>(null);
  const [togglingToolId, setTogglingToolId] = useState<string | null>(null);

  // ── per-skill 部署（junction 指向仓库，非破坏性） ──
  // toolId -> 已部署技能 id 列表；deployingKey = "toolId:skillId" 或 "toolId:*"（全量）
  const [deployedMap, setDeployedMap] = useState<Record<string, string[]>>({});
  const [deployingKey, setDeployingKey] = useState<string | null>(null);
  const [deployTargetSkill, setDeployTargetSkill] = useState<string | null>(null); // 卡片内展开部署选择的技能

  const loadDeployed = async () => {
    try {
      const list = await invoke<SkillToolStatusView[]>('get_skill_tools_status');
      const map: Record<string, string[]> = {};
      for (const s of list) {
        const ids = await invoke<string[]>('get_tool_deployed_skills', { toolId: s.toolId });
        map[s.toolId] = ids;
      }
      setDeployedMap(map);
    } catch (e) {
      console.error('加载技能部署状态失败:', e);
    }
  };

  const setDeployMsg = (key: string, msg: string, ok: boolean) => {
    // 复用 skillMsg / toolMsg：技能级消息挂到技能 id，工具级挂到工具 id
    if (key.endsWith(':*')) {
      setToolMsg({ id: key.slice(0, -2), msg, ok });
    } else {
      setSkillMsg({ id: key.split(':')[1], msg, ok });
    }
  };

  const deploySkillToTool = async (skillId: string, toolId: string) => {
    const key = `${toolId}:${skillId}`;
    setDeployingKey(key);
    try {
      await invoke('deploy_skill_to_tool', { toolId, skillId });
      setDeployMsg(key, t('skillmgr.deployedTo', { tool: toolId }), true);
      setDeployedMap((m) => ({ ...m, [toolId]: [...new Set([...(m[toolId] || []), skillId])] }));
      loadTools();
    } catch (e) {
      setDeployMsg(key, String(e), false);
    } finally {
      setDeployingKey(null);
    }
  };

  const undeploySkillFromTool = async (skillId: string, toolId: string) => {
    const key = `${toolId}:${skillId}`;
    setDeployingKey(key);
    try {
      await invoke('undeploy_skill_from_tool', { toolId, skillId });
      setDeployMsg(key, t('skillmgr.undeployedFrom', { tool: toolId }), true);
      setDeployedMap((m) => ({ ...m, [toolId]: (m[toolId] || []).filter((x) => x !== skillId) }));
      loadTools();
    } catch (e) {
      setDeployMsg(key, String(e), false);
    } finally {
      setDeployingKey(null);
    }
  };

  const deployAllToTool = async (toolId: string) => {
    setDeployingKey(`${toolId}:*`);
    try {
      const n = await invoke<number>('deploy_all_skills_to_tool', { toolId });
      setToolMsg({ id: toolId, msg: t('skillmgr.deployAllDone', { count: n }), ok: true });
      await loadDeployed();
      loadTools();
    } catch (e) {
      setToolMsg({ id: toolId, msg: String(e), ok: false });
    } finally {
      setDeployingKey(null);
    }
  };

  const undeployAllFromTool = async (toolId: string) => {
    setDeployingKey(`${toolId}:*`);
    try {
      const n = await invoke<number>('undeploy_all_skills_from_tool', { toolId });
      setToolMsg({ id: toolId, msg: t('skillmgr.undeployAllDone', { count: n }), ok: true });
      await loadDeployed();
      loadTools();
    } catch (e) {
      setToolMsg({ id: toolId, msg: String(e), ok: false });
    } finally {
      setDeployingKey(null);
    }
  };

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

  // ── 加载 ──
  const loadSkills = async () => {
    setSkillLoading(true);
    try {
      const data = await invoke<SkillEntry[]>('get_skill_overview');
      setSkills(data);
    } catch (e) {
      console.error("加载 AI 技能概览失败:", e);
    }
    setSkillLoading(false);
  };

  const loadTools = async () => {
    setToolLoading(true);
    try {
      const [t, s] = await Promise.all([
        invoke<DetectedAiTool[]>('detect_ai_tools'),
        invoke<SkillToolStatusView[]>('get_skill_tools_status'),
      ]);
      setTools(t);
      setToolStatus(s);
    } catch (e) {
      console.error("加载 AI 工具检测状态失败:", e);
    }
    setToolLoading(false);
  };

  useEffect(() => {
    loadSkills();
    loadTools();
    loadDeployed();
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
        loadTools();
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
      loadTools();
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

  // 工具软链接开关控制
  const toggleSymlink = async (toolId: string, currentEnabled: boolean) => {
    setToolMsg(null);
    setTogglingToolId(toolId);
    const nextEnabled = !currentEnabled;
    try {
      await invoke('toggle_tool_symlink_setting', { toolId, enabled: nextEnabled });
      setToolMsg({
        id: toolId,
        msg: nextEnabled ? t("skillmgr.symlinkOn") : t("skillmgr.symlinkOff"),
        ok: true,
      });
      await loadTools();
    } catch (e: any) {
      setToolMsg({ id: toolId, msg: String(e), ok: false });
    } finally {
      setTogglingToolId(null);
    }
  };

  // 工具 + 状态合并
  const toolRows = useMemo(() => {
    const statusMap = new Map(toolStatus.map((s) => [s.toolId, s]));
    return tools.map((t) => ({ tool: t, status: statusMap.get(t.id) }));
  }, [tools, toolStatus]);

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

  // 安装内置技能：目录由用户选（后端不猜，也不偷偷装进公共仓库）
  const installBuiltin = async (id: string) => {
    const picked = await open({ directory: true, title: t("skillmgr.builtinPickDir") });
    if (typeof picked !== 'string') return;
    setBuiltinBusy(id);
    setBuiltinResult(null);
    try {
      const path = await invoke<string>('install_builtin_skill', { skillId: id, targetDir: picked });
      setBuiltinResult({ ok: true, msg: t("skillmgr.builtinInstalled", { path }) });
      loadSkills();
    } catch (e: any) {
      setBuiltinResult({ ok: false, msg: String(e) });
    }
    setBuiltinBusy(null);
  };

  return (
    <div className="flex flex-col h-full text-slate-200 select-none">
      {/* 顶部导航 Tab */}
      <div className="flex items-center justify-between px-4 pt-3 border-b border-white/5 bg-white/[0.02]">
        <div className="flex items-center gap-1">
          {([
            { k: 'skills' as TabKey, label: t("skillmgr.tabSkills"), icon: Package },
            { k: 'tools' as TabKey, label: t("skillmgr.tabTools"), icon: Boxes },
            { k: 'market' as TabKey, label: t("skillmgr.tabMarket"), icon: Store },
            { k: 'builtin' as TabKey, label: t("skillmgr.tabBuiltin"), icon: Sparkles },
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
        <div className="text-tiny text-slate-500 font-mono">
          {t("skillmgr.publicSkillsDir")}<code className="text-slate-400">~/.agents/skills</code>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
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

            {/* 概览数据提示 */}
            <div className="flex items-center justify-between text-tiny text-slate-500">
              <span>{skillLoading ? t("skillmgr.loading") : `${t("skillmgr.skillsCount", { count: filtered.length })}${hasFilter ? t("skillmgr.filteredCount", { total: skills.length }) : ''}`}</span>
              <span>{t("skillmgr.source")}<code className="text-slate-400">~/.agents/skills</code></span>
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
                      <button
                        onClick={() => setDeployTargetSkill(deployTargetSkill === s.id ? null : s.id)}
                        className={`cursor-pointer ${deployTargetSkill === s.id ? 'text-[var(--module-accent)]' : 'text-slate-400 hover:text-[var(--module-accent)]'}`}
                        title={t("skillmgr.deployToTool")}
                      >
                        <Link2 className="w-3.5 h-3.5" />
                      </button>
                      <button onClick={() => openEdit(s)} className="text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
                        <Settings2 className="w-3.5 h-3.5" />
                      </button>
                      <button onClick={() => removeSkill(s.id)} className="text-slate-500 hover:text-red-400 cursor-pointer">
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                  {deployTargetSkill === s.id && (
                    <div className="rounded-ctl bg-black/20 border border-white/5 p-2 space-y-1">
                      <div className="text-micro text-slate-500 font-semibold">{t("skillmgr.deployToTool")}</div>
                      {toolStatus.length === 0 && (
                        <div className="text-micro text-slate-600">{t("skillmgr.noTools")}</div>
                      )}
                      {toolStatus.map((ts) => {
                        const deployed = (deployedMap[ts.toolId] || []).includes(s.id);
                        const busy = deployingKey === `${ts.toolId}:${s.id}`;
                        return (
                          <div key={ts.toolId} className="flex items-center justify-between gap-2">
                            <span className="text-tiny text-slate-300 truncate" title={ts.skillsDir}>
                              {ts.label}
                            </span>
                            <button
                              disabled={busy}
                              onClick={() => (deployed ? undeploySkillFromTool(s.id, ts.toolId) : deploySkillToTool(s.id, ts.toolId))}
                              className={`flex items-center gap-1 px-2 py-0.5 rounded text-micro font-semibold cursor-pointer transition disabled:opacity-50 ${
                                deployed
                                  ? 'bg-emerald-500/15 text-emerald-400 hover:bg-emerald-500/25'
                                  : 'bg-white/5 text-slate-400 hover:bg-white/10 hover:text-slate-200'
                              }`}
                            >
                              {busy ? (
                                <Loader2 className="w-3 h-3 animate-spin" />
                              ) : deployed ? (
                                <><Unlink className="w-2.5 h-2.5" />{t("skillmgr.deployedUndeploy")}</>
                              ) : (
                                <><Link2 className="w-2.5 h-2.5" />{t("skillmgr.deployedDeploy")}</>
                              )}
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  )}
                  {skillMsg?.id === s.id && (
                    <div className={`text-tiny ${skillMsg.ok ? 'text-emerald-400' : 'text-red-400'}`}>{skillMsg.msg}</div>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* ════════ 工具 + 软链接集成 Tab ════════ */}
        {tab === 'tools' && (
          <div className="space-y-4 max-w-4xl">
            {/* 提示警示区块 —— 这块样式被全软件采纳为统一提示组件（shared/Note） */}
            <Note tone="warn" title={t("skillmgr.symlinkWarnTitle")}>
              <p>
                {t("skillmgr.symlinkWarn1")}
                {t("skillmgr.symlinkWarn2")}
              </p>
              <p className="text-tiny text-slate-400">{t("skillmgr.symlinkWarn3")}</p>
            </Note>

            {/* 工具状态与软链接开关列表 */}
            <div className="space-y-2">
              <div className="text-body font-bold text-slate-300 px-1">{t("skillmgr.toolsTitle")}</div>
              <div className="grid grid-cols-1 gap-2.5">
                {toolRows.map(({ tool, status }) => {
                  const st = status?.status || 'empty';
                  const isSymlinkOn = status?.symlinkEnabled ?? false;
                  const readsAgents = status?.readsAgentsSkills ?? false;
                  const nickname = tool.nickname || tool.display_name;
                  const showOrigName = tool.nickname && tool.nickname !== tool.display_name;

                  return (
                    <div key={tool.id} className="rounded-card bg-white/[0.03] border border-white/10 p-3.5 flex flex-col md:flex-row md:items-center justify-between gap-3 hover:border-white/20 transition-all">
                      <div className="flex items-center gap-3 min-w-0">
                        <span className="flex-shrink-0 w-8 h-8 rounded-ctl bg-white/5 border border-white/10 flex items-center justify-center text-base">
                          {tool.avatar || '🤖'}
                        </span>
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="text-body font-semibold text-slate-100 truncate">{nickname}</span>
                            {showOrigName && (
                              <span className="text-tiny text-slate-500 font-normal truncate">({tool.display_name})</span>
                            )}
                            <StatusBadge status={st} />
                          </div>
                          <div className="text-tiny text-slate-500 font-mono truncate mt-0.5">
                            {tool.id} · <span className="text-slate-400">{status?.skillsDir || t("skillmgr.noSkillPath")}</span>
                          </div>
                          {readsAgents && (
                            <div className="text-micro text-cyan-400/80 flex items-center gap-1 mt-0.5">
                              <span>{t("skillmgr.builtinHint")}</span>
                            </div>
                          )}
                          {/* per-skill 部署：部署仓库全部技能到此工具 / 移除（非破坏性，不动用户自有技能） */}
                          <div className="flex items-center gap-2 mt-1.5">
                            <span className="text-micro text-slate-500 font-mono">
                              {t("skillmgr.deployedCount", { count: (deployedMap[tool.id] || []).length })}
                            </span>
                            <button
                              onClick={() => deployAllToTool(tool.id)}
                              disabled={deployingKey === `${tool.id}:*`}
                              className="flex items-center gap-1 px-2 py-0.5 rounded text-micro font-semibold bg-white/5 text-slate-300 hover:bg-white/10 hover:text-white cursor-pointer transition disabled:opacity-50"
                              title={t("skillmgr.deployAllTitle")}
                            >
                              {deployingKey === `${tool.id}:*` ? (
                                <Loader2 className="w-2.5 h-2.5 animate-spin" />
                              ) : (
                                <Link2 className="w-2.5 h-2.5" />
                              )}
                              {t("skillmgr.deployAll")}
                            </button>
                            <button
                              onClick={() => undeployAllFromTool(tool.id)}
                              disabled={deployingKey === `${tool.id}:*` || (deployedMap[tool.id] || []).length === 0}
                              className="flex items-center gap-1 px-2 py-0.5 rounded text-micro font-semibold bg-white/5 text-slate-400 hover:bg-white/10 hover:text-red-300 cursor-pointer transition disabled:opacity-40"
                              title={t("skillmgr.undeployAllTitle")}
                            >
                              <Unlink className="w-2.5 h-2.5" />
                              {t("skillmgr.undeployAll")}
                            </button>
                          </div>
                        </div>
                      </div>

                      <div className="flex items-center gap-3 self-end md:self-auto flex-shrink-0 border-t md:border-t-0 pt-2 md:pt-0 border-white/5">
                        <div className="text-right">
                          <div className="text-tiny text-slate-400">{t("skillmgr.symlinkStatus")}</div>
                          <div className="text-micro text-slate-500 font-mono">
                            {isSymlinkOn ? (
                              <span className="text-emerald-400 font-semibold flex items-center gap-1"><Link2 className="w-2.5 h-2.5" /> {t("skillmgr.linkedPublic")}</span>
                            ) : (
                              <span className="text-slate-500 flex items-center gap-1"><Unlink className="w-2.5 h-2.5" /> {t("skillmgr.disabledSymlink")}</span>
                            )}
                          </div>
                        </div>

                        {/* 软链接开关 Toggle */}
                        <button
                          onClick={() => toggleSymlink(tool.id, isSymlinkOn)}
                          disabled={togglingToolId === tool.id}
                          className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none disabled:opacity-60 disabled:cursor-not-allowed ${
                            isSymlinkOn ? 'bg-[var(--module-accent)]' : 'bg-slate-700'
                          }`}
                          title={isSymlinkOn ? t("skillmgr.symlinkOnTitle") : t("skillmgr.symlinkOffTitle")}
                        >
                          {togglingToolId === tool.id ? (
                            <span className="absolute inset-0 flex items-center justify-center">
                              <Loader2 className="w-3 h-3 text-white animate-spin" />
                            </span>
                          ) : (
                            <span
                              className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                                isSymlinkOn ? 'translate-x-4' : 'translate-x-0'
                              }`}
                            />
                          )}
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>

            {toolMsg && (
              <div className={`p-2.5 rounded-ctl text-body flex items-center gap-2 ${toolMsg.ok ? 'bg-emerald-500/10 text-emerald-400' : 'bg-red-500/10 text-red-400'}`}>
                {toolMsg.ok ? <CheckCircle className="w-3.5 h-3.5" /> : <AlertTriangle className="w-3.5 h-3.5" />}
                <span>{toolMsg.msg}</span>
              </div>
            )}
          </div>
        )}

        {/* ════════ 市场 Tab ════════ */}
        {tab === 'market' && (
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

        {/* ════════ 内置技能 Tab ════════ */}
        {tab === 'builtin' && (
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
