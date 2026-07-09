import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";
import {
  Puzzle,
  Plus,
  Trash2,
  RefreshCw,
  FolderOpen,
  FileText,
  Download,
  ExternalLink,
  Store,
  AlertTriangle,
  Wand2,
  Check,
  ChevronDown,
  PackageCheck,
  Search,
  LinkIcon,
  Unlink,
  CircleAlert,
} from "lucide-react";
import SkillFileViewer from "./SkillFileViewer";

// ─── 类型定义 ───

interface SkillToolStatus {
  tool_id: string;
  status: string; // "managed" | "foreign" | "none"
}

interface SkillOverview {
  id: string;
  name: string;
  description: string;
  in_store: boolean;
  registered: boolean;
  directory: string;
  installed_at: string;
  install_method: string;
  tools: SkillToolStatus[];
}

interface SkillIssue {
  issue_type: string; // "skills_sh" | "A" | "B" | "D"
  tool_id: string;
  skill_id: string;
  name: string;
  description: string;
  source_path: string;
  link_target: string;
  already_in_store: boolean;
}

interface IssueRef {
  toolId: string;
  skillId: string;
}

interface SkillToolInfo {
  id: string;
  label: string;
}

// ─── 常量 ───

const TOOL_IDS_FALLBACK = [
  { id: "claude-code", label: "Claude" },
  { id: "codex-cli", label: "Codex" },
  { id: "gemini-cli", label: "Gemini" },
  { id: "opencode", label: "OpenCode" },
  { id: "mimocode", label: "Mimo" },
  { id: "deveco", label: "DevEco" },
  { id: "qwencode", label: "Qwen" },
];

const MARKETPLACES = [
  { name: "skills.sh", desc: "官方规范 / 技能市场", url: "https://skills.sh" },
  { name: "ClawHub", desc: "OpenClaw 技能市场", url: "https://clawhub.ai" },
  { name: "Anthropic Skills", desc: "官方 Skills 仓库", url: "https://github.com/anthropics/skills" },
];

type TabId = "detection" | "list" | "install";

// ─── 主组件 ───

export default function SkillManager() {
  const [activeTab, setActiveTab] = useState<TabId>("list");
  const [skills, setSkills] = useState<SkillOverview[]>([]);
  const [issues, setIssues] = useState<SkillIssue[]>([]);
  const [loading, setLoading] = useState(true);
  const [skillTools, setSkillTools] = useState<SkillToolInfo[]>(TOOL_IDS_FALLBACK);
  const [refreshKey, setRefreshKey] = useState(0);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [ov, iss] = await Promise.all([
        invoke<SkillOverview[]>("get_skill_overview"),
        invoke<SkillIssue[]>("get_skill_issues"),
      ]);
      setSkills(ov);
      setIssues(iss);
    } catch (e) { console.error(e); }
    finally { setLoading(false); }
  }, []);

  const loadSkillTools = useCallback(async () => {
    try {
      const tools = await invoke<SkillToolInfo[]>("get_skill_tools");
      if (tools && tools.length > 0) setSkillTools(tools);
    } catch (e) { console.error(e); }
  }, []);

  useEffect(() => { load(); loadSkillTools(); }, [load, loadSkillTools, refreshKey]);

  const toolLabel = (id: string) =>
    skillTools.find((t) => t.id === id)?.label ?? id;

  const handleRefresh = () => setRefreshKey((k) => k + 1);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center text-slate-500">
        <RefreshCw className="w-5 h-5 animate-spin mr-2" />
        <span className="text-xs">加载中...</span>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center px-5 pt-4 pb-2 flex-shrink-0">
        <div>
          <h3 className="text-sm font-bold text-white">Skills 管理</h3>
          <p className="text-[10px] text-slate-500 mt-0.5">统一管理本地 AI 编程技能包</p>
        </div>
      </div>

      {/* Tab 按钮 + 刷新（同行，刷新居右） */}
      <div className="flex items-center gap-1 px-5 pb-2 flex-shrink-0">
        {([
          { id: "list" as TabId, label: "技能列表", icon: PackageCheck, count: skills.length, color: "violet" },
          { id: "install" as TabId, label: "新技能", icon: Plus, count: 0, color: "blue" },
          { id: "detection" as TabId, label: issues.length > 0 ? `问题检测 (${issues.length})` : "问题检测", icon: CircleAlert, count: 0, color: "amber" },
        ]).map((tab) => {
          const active = activeTab === tab.id;
          const Icon = tab.icon;
          return (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`px-3 py-1.5 rounded-lg text-[11px] font-semibold cursor-pointer transition-all flex items-center gap-1.5 border ${
                active
                  ? `bg-${tab.color}-500/15 border-${tab.color}-500/30 text-${tab.color}-300`
                  : "bg-transparent border-white/5 text-slate-500 hover:text-slate-300 hover:border-white/10"
              }`}
            >
              <Icon className="w-3.5 h-3.5" />
              {tab.label}
              {tab.count > 0 && (
                <span className={`px-1.5 py-0.5 rounded-full text-[8px] font-bold ${
                  active ? `bg-${tab.color}-500/20 text-${tab.color}-200` : "bg-white/5 text-slate-500"
                }`}>
                  {tab.count}
                </span>
              )}
            </button>
          );
        })}
        <button
          onClick={handleRefresh}
          className="ml-auto px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1"
        >
          <RefreshCw className="w-3 h-3" /> 刷新
        </button>
      </div>

      {/* Tab 内容 */}
      <div className="flex-1 overflow-y-auto px-5 pb-4">
        {activeTab === "detection" && (
          <DetectionTab
            issues={issues}
            skillTools={skillTools}
            toolLabel={toolLabel}
            onFixed={handleRefresh}
          />
        )}
        {activeTab === "list" && (
          <SkillListTab
            skills={skills}
            skillTools={skillTools}
            toolLabel={toolLabel}
            onChanged={handleRefresh}
          />
        )}
        {activeTab === "install" && (
          <InstallTab
            skillTools={skillTools}
            onInstalled={handleRefresh}
          />
        )}
      </div>
    </div>
  );
}

// ─── Tab 1: 问题检测 ───

function DetectionTab({
  issues,
  skillTools,
  toolLabel,
  onFixed,
}: {
  issues: SkillIssue[];
  skillTools: SkillToolInfo[];
  toolLabel: (id: string) => string;
  onFixed: () => void;
}) {
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [fixing, setFixing] = useState(false);
  const [fixingMap, setFixingMap] = useState<Record<string, boolean>>({});

  const issueKey = (iss: SkillIssue) => `${iss.tool_id}:${iss.skill_id}`;

  const checkedCount = checked.size;
  const allChecked = issues.length > 0 && checkedCount === issues.length;

  const toggleSelectAll = () => {
    if (allChecked) {
      setChecked(new Set());
    } else {
      setChecked(new Set(issues.map(issueKey)));
    }
  };

  const toggleCheck = (key: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const handleFixOne = async (iss: SkillIssue) => {
    const key = issueKey(iss);
    setFixingMap((prev) => ({ ...prev, [key]: true }));
    try {
      await invoke("fix_skill_issue", { toolId: iss.tool_id, skillId: iss.skill_id });
      onFixed();
    } catch (e: any) { alert(`解决问题失败: ${e}`); }
    finally { setFixingMap((prev) => ({ ...prev, [key]: false })); }
  };

  const handleFixAll = async () => {
    if (checkedCount === 0) return;
    setFixing(true);
    try {
      const selected: IssueRef[] = issues
        .filter((iss) => checked.has(issueKey(iss)))
        .map((iss) => ({ toolId: iss.tool_id, skillId: iss.skill_id }));
      const n = await invoke<number>("fix_all_issues", { issues: selected });
      setChecked(new Set());
      onFixed();
      alert(n > 0 ? `已解决 ${n} 个技能` : "没有需要解决的技能");
    } catch (e: any) { alert(`批量解决失败: ${e}`); }
    finally { setFixing(false); }
  };

  const issueBadge = (type: string) => {
    const config: Record<string, { label: string; color: string; icon: any }> = {
      skills_sh: { label: "skills.sh", color: "sky", icon: Store },
      A: { label: "直装", color: "amber", icon: Puzzle },
      B: { label: "外部链接", color: "orange", icon: LinkIcon },
      D: { label: "断链", color: "red", icon: Unlink },
    };
    const c = config[type] || { label: type, color: "slate", icon: AlertTriangle };
    const Icon = c.icon;
    return (
      <span className={`text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-${c.color}-500/15 text-${c.color}-300 flex items-center gap-0.5`}>
        <Icon className="w-2 h-2" /> {c.label}
      </span>
    );
  };

  return (
    <div className="space-y-3 pt-2">
      {/* 操作栏 */}
      <div className="flex items-center justify-between gap-2">
        <label className="flex items-center gap-1.5 text-[10px] text-slate-400 cursor-pointer select-none flex-shrink-0">
          <input
            type="checkbox"
            checked={allChecked}
            onChange={toggleSelectAll}
            disabled={issues.length === 0}
            className="accent-amber-500 cursor-pointer"
            title="全选 / 取消全选"
          />
          全选
        </label>
        <div className="text-[10px] text-slate-500 truncate">
          发现 {issues.length} 个问题
          {checkedCount > 0 && <span className="text-amber-300 ml-1">· 已勾选 {checkedCount}</span>}
        </div>
        <button
          onClick={handleFixAll}
          disabled={fixing || checkedCount === 0}
          className="px-3 py-1.5 rounded-lg bg-amber-600 hover:bg-amber-500 disabled:opacity-40 text-white text-[10px] font-semibold cursor-pointer transition-all flex items-center gap-1.5 flex-shrink-0"
          title="把勾选的技能问题解决并收纳到 AnyVersion 目录"
        >
          {fixing ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Wand2 className="w-3 h-3" />}
          一键解决问题{checkedCount > 0 ? ` (${checkedCount})` : ""}
        </button>
      </div>

      {issues.length === 0 ? (
        <div className="border border-dashed border-white/5 rounded-xl py-8 flex flex-col items-center justify-center text-slate-600">
          <Check className="w-6 h-6 text-emerald-500/60 mb-1" />
          <span className="text-[10px]">没有发现问题，所有技能均已由 AnyVersion 托管</span>
        </div>
      ) : (
        <div className="space-y-2">
          {issues.map((iss) => {
            const key = issueKey(iss);
            const isChecked = checked.has(key);
            const busy = fixingMap[key] || false;
            return (
              <div
                key={key}
                className={`rounded-xl bg-slate-900/30 border p-3 transition-all ${
                  isChecked ? "border-amber-500/40 bg-amber-500/5" : "border-white/5"
                }`}
              >
                <div className="flex items-start gap-3">
                  <input
                    type="checkbox"
                    checked={isChecked}
                    onChange={() => toggleCheck(key)}
                    className="mt-1 accent-amber-500 cursor-pointer"
                    title="勾选后参与一键解决问题"
                  />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="text-xs font-bold text-slate-200">{iss.name}</span>
                      {issueBadge(iss.issue_type)}
                      {iss.already_in_store ? (
                        <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-sky-500/15 text-sky-300">仓库已有·仅需链接</span>
                      ) : (
                        <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-amber-500/15 text-amber-300">需导入</span>
                      )}
                    </div>
                    {iss.description && (
                      <p className="text-[10px] text-slate-400 mt-0.5 line-clamp-1">{iss.description}</p>
                    )}
                    <div className="flex items-center gap-1.5 mt-2 flex-wrap">
                      <span className="text-[9px] text-slate-500">位置:</span>
                      <span className="px-1.5 py-0.5 rounded-md text-[8px] font-semibold bg-slate-800 border border-white/5 text-slate-300">
                        {iss.tool_id === "skills.sh" ? "skills.sh" : toolLabel(iss.tool_id)}
                      </span>
                      {iss.issue_type === "B" && iss.link_target && (
                        <span className="text-[8px] text-slate-500 truncate max-w-[200px]" title={iss.link_target}>
                          → {iss.link_target}
                        </span>
                      )}
                      {iss.issue_type === "D" && iss.link_target && (
                        <span className="text-[8px] text-red-400/70 truncate max-w-[200px]" title={iss.link_target}>
                          断链 → {iss.link_target}
                        </span>
                      )}
                    </div>
                  </div>
                  <button
                    onClick={() => handleFixOne(iss)}
                    disabled={busy}
                    className="px-2.5 py-1 rounded-md text-[9px] font-semibold bg-amber-600 hover:bg-amber-500 disabled:opacity-40 text-white cursor-pointer transition-all flex items-center gap-1 flex-shrink-0"
                  >
                    {busy ? <RefreshCw className="w-2.5 h-2.5 animate-spin" /> : <Wand2 className="w-2.5 h-2.5" />}
                    解决问题
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ─── Tab 2: 技能列表 ───

function SkillListTab({
  skills,
  skillTools,
  toolLabel,
  onChanged,
}: {
  skills: SkillOverview[];
  skillTools: SkillToolInfo[];
  toolLabel: (id: string) => string;
  onChanged: () => void;
}) {
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [viewingSkillId, setViewingSkillId] = useState<string | null>(null);
  const [togglingMap, setTogglingMap] = useState<Record<string, boolean>>({});
  const [registering, setRegistering] = useState(false);

  const handleToggleTool = async (skillId: string, toolId: string, enabled: boolean) => {
    const key = `${skillId}:${toolId}`;
    setTogglingMap((prev) => ({ ...prev, [key]: true }));
    try {
      await invoke("toggle_skill_tool", { skillId, toolId, enabled });
      onChanged();
    } catch (e: any) { alert(`操作失败: ${e}`); }
    finally { setTogglingMap((prev) => ({ ...prev, [key]: false })); }
  };

  const handleRegister = async (skillId: string) => {
    setRegistering(true);
    try {
      await invoke("register_store_skill", { skillId });
      onChanged();
    } catch (e: any) { alert(`纳入管理失败: ${e}`); }
    finally { setRegistering(false); }
  };

  const handleUninstall = async (id: string) => {
    try {
      await invoke("uninstall_skill", { skillId: id });
      setDeleteTarget(null);
      onChanged();
    } catch (e: any) { alert(`删除失败: ${e}`); }
  };

  const handleOpenSkillDir = async (dirPath: string) => {
    try { await invoke("open_ai_tool_cache_dir_path", { fullPath: dirPath }); }
    catch (e) { console.error(e); }
  };

  const statusOf = (s: SkillOverview, toolId: string) =>
    s.tools.find((t) => t.tool_id === toolId)?.status ?? "none";

  return (
    <div className="space-y-2 pt-2">
      {skills.length === 0 ? (
        <div className="border border-dashed border-white/5 rounded-2xl py-8 flex flex-col items-center justify-center text-slate-600">
          <Puzzle className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-xs font-bold text-slate-400">暂无技能</span>
          <span className="text-[10px] text-slate-600 mt-1">前往「新技能」安装，或在「问题检测」中收纳</span>
        </div>
      ) : (
        skills.map((skill) => (
          <div
            key={skill.id}
            className="rounded-xl bg-slate-900/30 border border-white/5 p-4 hover:border-white/10 transition-all"
          >
            <div className="flex items-start gap-3">
              <div className="p-2 rounded-lg bg-violet-500/10 flex-shrink-0">
                <Puzzle className="w-4 h-4 text-violet-400" />
              </div>
              <div className="flex-1 min-w-0">
                {/* 第一行：名称 + 状态徽章 + 操作 */}
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-xs font-bold text-slate-200">{skill.name}</span>
                  {skill.registered ? (
                    <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400 flex items-center gap-0.5">
                      <Check className="w-2 h-2" /> 托管
                    </span>
                  ) : (
                    <span className="text-[8px] font-bold px-1.5 py-0.5 rounded-full bg-amber-500/15 text-amber-400 flex items-center gap-0.5">
                      <AlertTriangle className="w-2 h-2" /> 未纳入
                    </span>
                  )}
                  {skill.registered && skill.installed_at && (
                    <span className="text-[9px] text-slate-600">{skill.installed_at}</span>
                  )}
                  <div className="flex items-center gap-0.5 ml-auto flex-shrink-0">
                    <button
                      onClick={() => handleOpenSkillDir(skill.directory)}
                      className="p-1 rounded text-slate-600 hover:text-blue-400 hover:bg-blue-500/10 cursor-pointer transition-all"
                      title="打开目录"
                    >
                      <ExternalLink className="w-3.5 h-3.5" />
                    </button>
                    <button
                      onClick={() => setViewingSkillId(skill.id)}
                      className="p-1 rounded text-slate-600 hover:text-blue-400 hover:bg-blue-500/10 cursor-pointer transition-all"
                      title="查看文件"
                    >
                      <FileText className="w-3.5 h-3.5" />
                    </button>
                    {skill.registered && (
                      <button
                        onClick={() => setDeleteTarget(skill.id)}
                        className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all"
                        title="卸载"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    )}
                  </div>
                </div>

                {skill.description && (
                  <p className="text-[10px] text-slate-400 mt-1 line-clamp-1">{skill.description}</p>
                )}

                {/* 各工具状态：点击可安装/卸载 */}
                <div className="flex items-center gap-2 mt-3 pt-2 border-t border-white/[0.03]">
                  <span className="text-[9px] text-slate-500 flex-shrink-0">工具:</span>
                  <div className="flex flex-wrap gap-1.5">
                    {skillTools.map((t) => {
                      const status = statusOf(skill, t.id);
                      const key = `${skill.id}:${t.id}`;
                      const busy = togglingMap[key] || false;
                      if (status === "managed") {
                        return (
                          <button
                            key={t.id}
                            onClick={() => handleToggleTool(skill.id, t.id, false)}
                            disabled={busy}
                            title="已安装 · 点击卸载（移除 junction）"
                            className="flex items-center gap-1 px-2 py-0.5 rounded-md text-[9px] font-semibold bg-emerald-500/15 border border-emerald-500/30 text-emerald-400 hover:bg-emerald-500/25 cursor-pointer transition-all disabled:opacity-50"
                          >
                            {busy ? <RefreshCw className="w-2.5 h-2.5 animate-spin" /> : <Check className="w-2.5 h-2.5" />}
                            {t.label}
                          </button>
                        );
                      }
                      // foreign 或 none：均视为未安装
                      return (
                        <button
                          key={t.id}
                          onClick={() => handleToggleTool(skill.id, t.id, true)}
                          disabled={busy}
                          title={status === "foreign" ? "存在非托管安装，点击创建托管 junction" : "点击安装（创建 junction）"}
                          className="px-2 py-0.5 rounded-md text-[9px] font-semibold bg-slate-800 border border-white/5 text-slate-600 hover:text-violet-300 hover:border-violet-500/30 cursor-pointer transition-all disabled:opacity-50"
                        >
                          {busy ? <RefreshCw className="w-2.5 h-2.5 animate-spin" /> : null}
                          {t.label}
                        </button>
                      );
                    })}
                  </div>
                </div>

                {/* 未纳入：一键纳入管理 */}
                {!skill.registered && (
                  <button
                    onClick={() => handleRegister(skill.id)}
                    disabled={registering}
                    className="mt-2 px-2.5 py-1 rounded-md text-[9px] font-semibold bg-violet-600/20 border border-violet-500/30 text-violet-300 hover:bg-violet-600/30 cursor-pointer transition-all flex items-center gap-1"
                  >
                    {registering ? <RefreshCw className="w-2.5 h-2.5 animate-spin" /> : <Check className="w-2.5 h-2.5" />}
                    纳入管理
                  </button>
                )}
              </div>
            </div>
          </div>
        ))
      )}

      {/* Delete confirm */}
      {deleteTarget && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4" onClick={() => setDeleteTarget(null)}>
          <div className="w-full max-w-sm bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl p-5" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center gap-3 mb-4">
              <div className="p-2 rounded-lg bg-red-500/10"><Trash2 className="w-4 h-4 text-red-400" /></div>
              <div>
                <h3 className="text-xs font-bold text-slate-200">确认卸载</h3>
                <p className="text-[10px] text-slate-500 mt-0.5">确定要卸载 {skills.find((s) => s.id === deleteTarget)?.name} 吗？将移除全局仓库与所有工具的链接。</p>
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <button onClick={() => setDeleteTarget(null)} className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer">取消</button>
              <button onClick={() => handleUninstall(deleteTarget)} className="px-3.5 py-1.5 rounded-lg bg-red-600 hover:bg-red-500 text-white text-[10px] font-semibold cursor-pointer">卸载</button>
            </div>
          </div>
        </div>
      )}

      {/* File Viewer */}
      {viewingSkillId && (
        <SkillFileViewer skillId={viewingSkillId} onClose={() => setViewingSkillId(null)} />
      )}
    </div>
  );
}

// ─── Tab 3: 新技能 ───

function InstallTab({
  skillTools,
  onInstalled,
}: {
  skillTools: SkillToolInfo[];
  onInstalled: () => void;
}) {
  const [sourceUrl, setSourceUrl] = useState("");
  const [installing, setInstalling] = useState(false);
  const [installMsg, setInstallMsg] = useState("");
  const [selectedTools, setSelectedTools] = useState<Set<string>>(new Set());
  const [showInstall, setShowInstall] = useState(true);

  // 实时接收后端安装进度事件
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ stage: string; current: number; total: number; skillName: string; message: string }>(
      "skill-install-progress",
      (e) => {
        const p = e.payload;
        setInstallMsg(p.total > 0 ? `${p.message} (${p.current}/${p.total})` : p.message);
      }
    ).then((u) => { unlisten = u; });
    return () => { if (unlisten) unlisten(); };
  }, []);

  const handleBrowseLocal = async () => {
    try {
      const selected = await open({ directory: true, title: "选择包含 SKILL.md 的技能目录" });
      if (selected) setSourceUrl(selected as string);
    } catch { /* ignore */ }
  };

  const handleInstall = async () => {
    if (!sourceUrl.trim()) return;
    const tools = Array.from(selectedTools);
    if (tools.length === 0) {
      alert("请至少选择一个目标工具");
      return;
    }
    setInstalling(true);
    setInstallMsg("正在克隆 / 读取技能源...");
    try {
      await invoke("install_skill_from_online", { source: sourceUrl.trim(), targetTools: tools });
      setInstallMsg("安装完成！已创建 junction 链接");
      setSourceUrl("");
      setSelectedTools(new Set());
      onInstalled();
      setTimeout(() => setInstallMsg(""), 3000);
    } catch (e: any) {
      setInstallMsg(`安装失败: ${e}`);
    } finally { setInstalling(false); }
  };

  const toggleToolSelection = (toolId: string) => {
    setSelectedTools((prev) => {
      const next = new Set(prev);
      if (next.has(toolId)) next.delete(toolId);
      else next.add(toolId);
      return next;
    });
  };

  const handleOpenMarket = async (url: string) => {
    try { await openUrl(url); }
    catch { window.open(url, "_blank"); }
  };

  return (
    <div className="space-y-4 pt-2">
      {/* 在线市场 */}
      <div className="p-3 rounded-xl bg-amber-500/5 border border-amber-500/10">
        <label className="text-[10px] text-amber-300 font-semibold flex items-center gap-1.5 block mb-2">
          <Store className="w-3.5 h-3.5" /> 技能在线市场
        </label>
        <div className="grid grid-cols-3 gap-2">
          {MARKETPLACES.map((m) => (
            <button
              key={m.url}
              onClick={() => handleOpenMarket(m.url)}
              className="px-2 py-2 rounded-lg bg-slate-900/40 border border-white/5 hover:border-amber-500/30 text-left cursor-pointer transition-all group"
              title={m.url}
            >
              <div className="flex items-center gap-1">
                <span className="text-[10px] font-bold text-slate-200 group-hover:text-amber-300 transition-colors truncate">{m.name}</span>
                <ExternalLink className="w-2.5 h-2.5 text-slate-600 group-hover:text-amber-400 flex-shrink-0 ml-auto" />
              </div>
              <p className="text-[8px] text-slate-500 mt-0.5 truncate">{m.desc}</p>
            </button>
          ))}
        </div>
      </div>

      {/* 从来源安装 */}
      <div className="rounded-xl bg-blue-500/5 border border-blue-500/10 overflow-hidden">
        <button
          onClick={() => setShowInstall((v) => !v)}
          className="w-full px-3 py-2.5 flex items-center gap-2 text-left hover:bg-blue-500/5 transition-colors"
        >
          <Plus className="w-3.5 h-3.5 text-blue-400" />
          <span className="text-[11px] font-semibold text-blue-200">从来源安装技能</span>
          <span className="text-[9px] text-slate-500">Git 仓库 / owner-repo / 本地目录</span>
          <ChevronDown className={`w-3.5 h-3.5 text-slate-500 ml-auto transition-transform ${showInstall ? "rotate-180" : ""}`} />
        </button>
        {showInstall && (
          <div className="px-3 pb-3 space-y-2">
            <div className="flex gap-2">
              <input
                value={sourceUrl}
                onChange={(e) => setSourceUrl(e.target.value)}
                placeholder="Git 仓库 URL 或 owner/repo（如 vercel-labs/skills）"
                className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-blue-500"
                disabled={installing}
              />
              <button
                onClick={handleBrowseLocal}
                disabled={installing}
                className="px-3 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 text-[10px] text-slate-300 hover:text-white cursor-pointer transition-all flex items-center gap-1 flex-shrink-0"
                title="选择本地文件夹"
              >
                <FolderOpen className="w-3 h-3" /> 浏览
              </button>
            </div>

            <div>
              <label className="text-[9px] text-slate-500 block mb-1">
                安装到以下工具（先装入 AnyVersion 仓库，再为每个工具创建 JUNCTION 链接）:
              </label>
              <div className="flex flex-wrap gap-1.5">
                {skillTools.map((t) => (
                  <button
                    key={t.id}
                    onClick={() => toggleToolSelection(t.id)}
                    disabled={installing}
                    className={`px-2.5 py-1 rounded-md text-[9px] font-semibold cursor-pointer transition-all border ${
                      selectedTools.has(t.id)
                        ? "bg-violet-500/20 border-violet-500/40 text-violet-300"
                        : "bg-slate-900 border-white/5 text-slate-500 hover:text-slate-300 hover:border-white/10"
                    }`}
                  >
                    {t.label}
                  </button>
                ))}
              </div>
            </div>

            {/* 安装进度 */}
            {installing && (
              <div className="flex items-center gap-2 text-[10px] text-blue-300">
                <RefreshCw className="w-3 h-3 animate-spin" />
                <span>{installMsg}</span>
              </div>
            )}
            {!installing && installMsg && (
              <div className="text-[10px] text-emerald-300">{installMsg}</div>
            )}

            <button
              onClick={handleInstall}
              disabled={installing || !sourceUrl.trim() || selectedTools.size === 0}
              className="w-full px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 disabled:opacity-40 text-white text-[10px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-1"
            >
              {installing ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />}
              {installing ? "安装中..." : `安装到 ${selectedTools.size} 个工具`}
            </button>
          </div>
        )}
      </div>

      {/* 说明 */}
      <div className="p-3 rounded-xl bg-violet-500/5 border border-violet-500/10 text-[10px] text-slate-400 space-y-1">
        <p className="font-semibold text-violet-300">安装流程</p>
        <p>
          1. 技能源（Git 仓库 / 本地目录）会被克隆或拷贝到 AnyVersion 技能仓库
          （全局设置可配置，默认 <span className="font-mono text-slate-300">~/.any-version/skills</span>）。
        </p>
        <p>2. 为每个勾选的工具在其技能目录下创建 JUNCTION 链接到仓库中的技能。</p>
        <p>3. 同名技能直接覆盖（视为新版本）。</p>
      </div>
    </div>
  );
}
