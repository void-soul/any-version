import { useState, useEffect, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen as listenEvent } from '@tauri-apps/api/event';
import {
  Search, Tag, Boxes, Store, Download, Trash2,
  CheckCircle, AlertTriangle, ExternalLink, X, Package, Loader2,
  ChevronDown, Settings2, Filter, Link2, Unlink, Info
} from 'lucide-react';
import { DetectedAiTool } from './types';

// ─── 类型 ───
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
}

type TabKey = 'skills' | 'tools' | 'market';

const MARKET_SOURCES = [
  { name: 'skills.sh 官网', desc: 'Agent Skills 官方生态与规范', url: 'https://skills.sh' },
  { name: 'Anthropic 官方 Skills', desc: 'anthropics/skills 仓库示例技能', url: 'https://github.com/anthropics/skills' },
  { name: 'GitHub 主题搜索', desc: '搜索社区维护的 agentskills 仓库', url: 'https://github.com/search?q=agentskills&type=repositories' },
];

function StatusBadge({ status }: { status: string }) {
  if (status === 'managed') return <span className="px-1.5 py-0.5 rounded text-[9px] font-semibold bg-emerald-500/15 text-emerald-400">已软链接</span>;
  if (status === 'unmanaged') return <span className="px-1.5 py-0.5 rounded text-[9px] font-semibold bg-amber-500/15 text-amber-400">私有目录</span>;
  return <span className="px-1.5 py-0.5 rounded text-[9px] font-semibold bg-slate-500/15 text-slate-400">未接入</span>;
}

export default function SkillManager() {
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
  const [toolLoading, setToolLoading] = useState(true);
  const [toolMsg, setToolMsg] = useState<{ id: string; msg: string; ok: boolean } | null>(null);
  const [togglingToolId, setTogglingToolId] = useState<string | null>(null);

  // ── 市场安装 ──
  const [installInput, setInstallInput] = useState('');
  const [installing, setInstalling] = useState(false);
  const [installLog, setInstallLog] = useState('');
  const [installErr, setInstallErr] = useState('');

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
    if (!confirm(`确定要卸载技能「${id}」吗？`)) return;
    try {
      await invoke('uninstall_skill', { skillId: id });
      setSkillMsg({ id, msg: '删除成功', ok: true });
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
      alert(`保存失败: ${e}`);
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
        msg: nextEnabled ? '已开启该工具软链接接入' : '已关闭该工具软链接接入（可防止重复读取告警）',
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

  // 市场安装
  const startInstall = async () => {
    const src = installInput.trim();
    if (!src) return;
    setInstalling(true);
    setInstallLog('开始安装...\n');
    setInstallErr('');
    try {
      await invoke('install_skill_from_online', { source: src });
    } catch (e: any) {
      setInstalling(false);
      setInstallErr(String(e));
    }
  };

  return (
    <div className="flex flex-col h-full bg-[#0b0e14] text-slate-200 select-none">
      {/* 顶部导航 Tab */}
      <div className="flex items-center justify-between px-4 pt-3 border-b border-white/5 bg-slate-900/40">
        <div className="flex items-center gap-1">
          {([
            { k: 'skills' as TabKey, label: '技能库管理', icon: Package },
            { k: 'tools' as TabKey, label: '软链接与工具集成', icon: Boxes },
            { k: 'market' as TabKey, label: '技能市场与在线安装', icon: Store },
          ]).map(({ k, label, icon: Icon }) => (
            <button
              key={k}
              onClick={() => setTab(k)}
              className={`flex items-center gap-1.5 px-3.5 py-2.5 text-xs font-semibold border-b-2 transition-all cursor-pointer ${
                tab === k
                  ? 'border-violet-500 text-white bg-white/[0.03]'
                  : 'border-transparent text-slate-400 hover:text-slate-200 hover:bg-white/[0.01]'
              }`}
            >
              <Icon className="w-3.5 h-3.5" />
              {label}
            </button>
          ))}
        </div>
        <div className="text-[10px] text-slate-500 font-mono">
          公共技能库: <code className="text-slate-400">~/.agents/skills</code>
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
                  placeholder="搜索技能名称 / ID / 描述..."
                  className="w-full pl-8 pr-2 py-1.5 rounded-lg bg-white/5 border border-white/10 text-xs text-slate-200 placeholder-slate-500 focus:outline-none focus:border-violet-500/50"
                />
              </div>
              <div className="relative">
                <select
                  value={selCat ?? ''}
                  onChange={(e) => setSelCat(e.target.value || null)}
                  className="appearance-none pl-7 pr-7 py-1.5 rounded-lg bg-slate-800 border border-white/10 text-xs text-slate-200 focus:outline-none focus:border-violet-500/50 cursor-pointer"
                >
                  <option value="">全部分类</option>
                  {allCats.map((c) => <option key={c} value={c}>{c}</option>)}
                </select>
                <Filter className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500" />
                <ChevronDown className="absolute right-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-slate-500 pointer-events-none" />
              </div>
              {hasFilter && (
                <button onClick={clearFilter} className="px-2 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 text-[10px] text-slate-400 flex items-center gap-1 cursor-pointer">
                  <X className="w-3 h-3" /> 清除筛选
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
                      className={`px-2 py-0.5 rounded-full text-[10px] font-medium transition-all cursor-pointer ${
                        active
                          ? 'bg-violet-500/30 text-violet-200 border border-violet-400/40'
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
            <div className="flex items-center justify-between text-[10px] text-slate-500">
              <span>{skillLoading ? '加载中...' : `共 ${filtered.length} 个技能${hasFilter ? `（已筛选，全部 ${skills.length}）` : ''}`}</span>
              <span>来源：<code className="text-slate-400">~/.agents/skills</code></span>
            </div>

            {/* 技能网格 */}
            {!skillLoading && filtered.length === 0 && (
              <div className="text-center py-16 text-slate-500 text-sm">
                {skills.length === 0
                  ? '尚未安装任何技能，前往「技能市场」一键安装'
                  : '没有符合筛选条件的技能'}
              </div>
            )}

            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
              {filtered.map((s) => (
                <div key={s.id} className="rounded-xl bg-white/[0.03] border border-white/10 p-3.5 flex flex-col gap-2 hover:border-white/20 transition-all">
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <div className="text-sm font-semibold text-slate-100 truncate">{s.name || s.id}</div>
                      <div className="text-[10px] text-slate-500 font-mono truncate">{s.id}</div>
                    </div>
                    {s.category && (
                      <span className="px-1.5 py-0.5 rounded text-[9px] font-semibold bg-violet-500/15 text-violet-300 flex-shrink-0">
                        {s.category}
                      </span>
                    )}
                  </div>
                  <p className="text-[11px] text-slate-400 line-clamp-2 min-h-[28px]">{s.description || '（无描述）'}</p>
                  {s.tags.length > 0 && (
                    <div className="flex flex-wrap gap-1">
                      {s.tags.map((t) => (
                        <span key={t} className="px-1.5 py-0.5 rounded text-[9px] bg-white/5 text-slate-400">
                          #{t}
                        </span>
                      ))}
                    </div>
                  )}
                  <div className="flex items-center justify-between pt-2 border-t border-white/5 text-[10px] text-slate-500">
                    <span>{s.installMethod === 'managed' ? '托管库' : s.installMethod}</span>
                    <div className="flex items-center gap-2">
                      <button onClick={() => openEdit(s)} className="text-slate-400 hover:text-violet-300 cursor-pointer">
                        <Settings2 className="w-3.5 h-3.5" />
                      </button>
                      <button onClick={() => removeSkill(s.id)} className="text-slate-500 hover:text-red-400 cursor-pointer">
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                  {skillMsg?.id === s.id && (
                    <div className={`text-[10px] ${skillMsg.ok ? 'text-emerald-400' : 'text-red-400'}`}>{skillMsg.msg}</div>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* ════════ 工具 + 软链接集成 Tab ════════ */}
        {tab === 'tools' && (
          <div className="space-y-4 max-w-4xl">
            {/* 提示警示区块 */}
            <div className="rounded-xl bg-amber-500/10 border border-amber-500/20 p-3.5 space-y-1.5">
              <div className="flex items-center gap-2 text-amber-300 text-xs font-bold">
                <Info className="w-4 h-4 flex-shrink-0" />
                <span>工具软链接（Symlink / Junction）与技能双重读取说明</span>
              </div>
              <p className="text-[11px] text-slate-300 leading-relaxed">
                一些现代化 CLI 工具（如 <code className="text-amber-200">OpenCode</code>、<code className="text-amber-200">Claude Code</code>、<code className="text-amber-200">Qwen Code</code> 等）原生具备自动扫描公共技能库 <code className="text-amber-200">~/.agents/skills</code> 的能力。
                若为这类工具强行开启私有技能目录的软链接，会导致工具重复扫描到两份相同技能。
              </p>
              <p className="text-[10px] text-slate-400">
                👉 您可以在下方独立关闭或开启每个工具的软链接部署开关，随意定制。
              </p>
            </div>

            {/* 工具状态与软链接开关列表 */}
            <div className="space-y-2">
              <div className="text-xs font-bold text-slate-300 px-1">已检测到的 AI 工具及其软链接配置</div>
              <div className="grid grid-cols-1 gap-2.5">
                {toolRows.map(({ tool, status }) => {
                  const st = status?.status || 'empty';
                  const isSymlinkOn = status?.symlinkEnabled ?? false;
                  const readsAgents = status?.readsAgentsSkills ?? false;
                  const nickname = tool.nickname || tool.display_name;
                  const showOrigName = tool.nickname && tool.nickname !== tool.display_name;

                  return (
                    <div key={tool.id} className="rounded-xl bg-white/[0.03] border border-white/10 p-3.5 flex flex-col md:flex-row md:items-center justify-between gap-3 hover:border-white/20 transition-all">
                      <div className="flex items-center gap-3 min-w-0">
                        <span className="flex-shrink-0 w-8 h-8 rounded-lg bg-white/5 border border-white/10 flex items-center justify-center text-base">
                          {tool.avatar || '🤖'}
                        </span>
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <span className="text-xs font-semibold text-slate-100 truncate">{nickname}</span>
                            {showOrigName && (
                              <span className="text-[10px] text-slate-500 font-normal truncate">({tool.display_name})</span>
                            )}
                            <StatusBadge status={st} />
                          </div>
                          <div className="text-[10px] text-slate-500 font-mono truncate mt-0.5">
                            {tool.id} · <span className="text-slate-400">{status?.skillsDir || '未找到技能路径'}</span>
                          </div>
                          {readsAgents && (
                            <div className="text-[9px] text-cyan-400/80 flex items-center gap-1 mt-0.5">
                              <span>💡 该工具内建自动读取 ~/.agents/skills，无需强开软链接</span>
                            </div>
                          )}
                        </div>
                      </div>

                      <div className="flex items-center gap-3 self-end md:self-auto flex-shrink-0 border-t md:border-t-0 pt-2 md:pt-0 border-white/5">
                        <div className="text-right">
                          <div className="text-[10px] text-slate-400">软链接状态</div>
                          <div className="text-[9px] text-slate-500 font-mono">
                            {isSymlinkOn ? (
                              <span className="text-emerald-400 font-semibold flex items-center gap-1"><Link2 className="w-2.5 h-2.5" /> 已链接公共库</span>
                            ) : (
                              <span className="text-slate-500 flex items-center gap-1"><Unlink className="w-2.5 h-2.5" /> 已禁用软链接</span>
                            )}
                          </div>
                        </div>

                        {/* 软链接开关 Toggle */}
                        <button
                          onClick={() => toggleSymlink(tool.id, isSymlinkOn)}
                          disabled={togglingToolId === tool.id}
                          className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none disabled:opacity-60 disabled:cursor-not-allowed ${
                            isSymlinkOn ? 'bg-violet-600' : 'bg-slate-700'
                          }`}
                          title={isSymlinkOn ? '关闭软链接部署' : '开启软链接部署'}
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
              <div className={`p-2.5 rounded-lg text-xs flex items-center gap-2 ${toolMsg.ok ? 'bg-emerald-500/10 text-emerald-400' : 'bg-red-500/10 text-red-400'}`}>
                {toolMsg.ok ? <CheckCircle className="w-3.5 h-3.5" /> : <AlertTriangle className="w-3.5 h-3.5" />}
                <span>{toolMsg.msg}</span>
              </div>
            )}
          </div>
        )}

        {/* ════════ 市场 Tab ════════ */}
        {tab === 'market' && (
          <div className="space-y-4 max-w-3xl">
            <div className="rounded-xl bg-violet-500/5 border border-violet-500/20 p-3.5 text-[11px] text-violet-200/80 leading-relaxed">
              从 Git 仓库或线上源安装的技能将统一下载落盘至 <code className="text-violet-300">~/.agents/skills</code>（skills.sh 标准技能根目录）。
              所有支持的工具均可直接接入该技能库。
            </div>

            {/* 来源列表 */}
            <div>
              <div className="text-xs font-semibold text-slate-300 mb-2 flex items-center gap-1.5">
                <Store className="w-3.5 h-3.5 text-violet-400" /> 推荐技能生态源
              </div>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-2.5">
                {MARKET_SOURCES.map((src) => (
                  <a
                    key={src.url}
                    href={src.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="rounded-xl bg-white/[0.03] border border-white/10 p-3.5 hover:border-violet-500/40 transition-all group"
                  >
                    <div className="flex items-center justify-between">
                      <span className="text-xs font-semibold text-slate-100">{src.name}</span>
                      <ExternalLink className="w-3.5 h-3.5 text-slate-500 group-hover:text-violet-400 transition-colors" />
                    </div>
                    <p className="text-[10px] text-slate-400 mt-1">{src.desc}</p>
                  </a>
                ))}
              </div>
            </div>

            {/* 在线安装 */}
            <div>
              <div className="text-xs font-semibold text-slate-300 mb-2 flex items-center gap-1.5">
                <Download className="w-3.5 h-3.5 text-violet-400" /> 从 Git / 远程 / 本地安装
              </div>
              <div className="rounded-xl bg-white/[0.03] border border-white/10 p-3.5 space-y-2.5">
                <input
                  value={installInput}
                  onChange={(e) => setInstallInput(e.target.value)}
                  placeholder="如: owner/repo / https://github.com/... / 本地技能路径"
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-xs text-slate-200 placeholder-slate-500 focus:outline-none focus:border-violet-500/50"
                />
                <button
                  onClick={startInstall}
                  disabled={installing || !installInput.trim()}
                  className="w-full px-3 py-2.5 rounded-lg bg-violet-600 hover:bg-violet-500 disabled:opacity-50 text-xs font-semibold text-white flex items-center justify-center gap-2 transition-all cursor-pointer"
                >
                  {installing ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
                  {installing ? '正在安装技能中...' : '安装到 ~/.agents/skills'}
                </button>
                {installErr && (
                  <div className="p-2.5 rounded-lg text-xs flex items-center gap-2 bg-red-500/10 text-red-400">
                    <AlertTriangle className="w-3.5 h-3.5" /> {installErr}
                  </div>
                )}
                {installLog && (
                  <pre className="text-[10px] text-slate-400 bg-black/30 rounded-lg p-2.5 max-h-40 overflow-y-auto whitespace-pre-wrap font-mono border border-white/5">{installLog}</pre>
                )}
              </div>
            </div>
          </div>
        )}
      </div>

      {/* 编辑分类/标签弹窗 */}
      {editing && (
        <div className="fixed inset-0 z-50 bg-black/60 backdrop-blur-xs flex items-center justify-center p-4">
          <div className="rounded-xl bg-slate-900 border border-white/10 p-4 w-full max-w-md space-y-3 shadow-2xl">
            <div className="flex items-center justify-between border-b border-white/5 pb-2">
              <span className="text-xs font-bold text-slate-200">修改技能属性 · {editing.id}</span>
              <button onClick={() => setEditing(null)} className="text-slate-500 hover:text-slate-300">
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="space-y-2 text-xs">
              <div>
                <label className="text-[10px] text-slate-400 mb-1 block">分类 (Category)</label>
                <input
                  value={edCat}
                  onChange={(e) => setEdCat(e.target.value)}
                  placeholder="如: 代码审计 / 前端 / 运维"
                  className="w-full px-2.5 py-1.5 rounded bg-white/5 border border-white/10 text-xs text-slate-200 focus:outline-none focus:border-violet-500"
                />
              </div>

              <div>
                <label className="text-[10px] text-slate-400 mb-1 block">标签 (Tags)</label>
                <div className="flex flex-wrap gap-1 mb-2">
                  {edTags.map((t) => (
                    <span key={t} className="px-2 py-0.5 rounded bg-violet-500/20 text-violet-300 text-[10px] flex items-center gap-1">
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
                    placeholder="输入标签按回车..."
                    className="flex-1 px-2.5 py-1.5 rounded bg-white/5 border border-white/10 text-xs text-slate-200 focus:outline-none focus:border-violet-500"
                  />
                  <button onClick={addTag} className="px-3 py-1.5 rounded bg-white/10 hover:bg-white/20 text-xs font-semibold text-slate-200 cursor-pointer">
                    添加
                  </button>
                </div>
              </div>
            </div>

            <div className="flex items-center justify-end gap-2 pt-2 border-t border-white/5">
              <button onClick={() => setEditing(null)} className="px-3 py-1.5 rounded text-xs text-slate-400 hover:text-slate-200 cursor-pointer">
                取消
              </button>
              <button
                onClick={saveMeta}
                disabled={savingMeta}
                className="px-3 py-1.5 rounded bg-violet-600 hover:bg-violet-500 text-xs font-semibold text-white flex items-center gap-1.5 cursor-pointer"
              >
                {savingMeta ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : null}
                保存设置
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
