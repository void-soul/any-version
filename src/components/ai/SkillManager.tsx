import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Puzzle,
  Plus,
  Trash2,
  RefreshCw,
  Link2,
  FolderOpen,
  FileText,
  Download,
  Sparkles,
  ExternalLink,
} from "lucide-react";
import SkillFileViewer from "./SkillFileViewer";

interface Skill {
  id: string;
  name: string;
  description: string;
  directory: string;
  enabled_tools: string[];
  installed_at: string;
  install_method: string;
}

interface ScannedSkill {
  name: string;
  description: string;
  directory: string;
  full_path: string;
  found_in: string[];
  is_symlink: boolean;
}

const TOOL_IDS = [
  { id: "claude-code", label: "Claude" },
  { id: "codex-cli", label: "Codex" },
  { id: "gemini-cli", label: "Gemini" },
  { id: "kilocode", label: "Kilo" },
  { id: "aider", label: "Aider" },
  { id: "opencode", label: "OpenCode" },
];

export default function SkillManager() {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(true);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [viewingSkillId, setViewingSkillId] = useState<string | null>(null);
  const [importingExisting, setImportingExisting] = useState<string | null>(null);
  const [scannedSkills, setScannedSkills] = useState<ScannedSkill[]>([]);
  const [sourceUrl, setSourceUrl] = useState("");
  const [installingFromSource, setInstallingFromSource] = useState(false);
  const [selectedTools, setSelectedTools] = useState<Set<string>>(new Set());
  const [togglingMap, setTogglingMap] = useState<Record<string, boolean>>({});

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<Skill[]>("get_skills");
      setSkills(data);
    } catch (e) { console.error(e); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => { load(); }, [load]);

  const handleScanExisting = async () => {
    try {
      const results = await invoke<ScannedSkill[]>("scan_existing_skills");
      setScannedSkills(results);
    } catch (e: any) { alert(`扫描失败: ${e}`); }
  };

  const handleImportExisting = async (skill: ScannedSkill) => {
    setImportingExisting(skill.full_path);
    try {
      await invoke("import_existing_skill", { skillPath: skill.full_path });
      await load();
      setScannedSkills(prev => prev.filter(s => s.full_path !== skill.full_path));
    } catch (e: any) { alert(`导入失败: ${e}`); }
    finally { setImportingExisting(null); }
  };

  const handleInstallLocal = async () => {
    try {
      const selected = await open({ directory: true, title: "选择包含 SKILL.md 的目录" });
      if (!selected) return;
      await invoke("install_skill", { skillDir: selected as string });
      await load();
    } catch (e: any) { alert(`安装失败: ${e}`); }
  };

  const handleBrowseLocal = async () => {
    try {
      const selected = await open({ directory: true, title: "选择包含 SKILL.md 的技能目录" });
      if (selected) setSourceUrl(selected as string);
    } catch { /* ignore */ }
  };

  const handleInstallFromSource = async () => {
    if (!sourceUrl.trim()) return;
    const tools = Array.from(selectedTools);
    if (tools.length === 0) {
      alert("请至少选择一个目标工具");
      return;
    }
    setInstallingFromSource(true);
    try {
      await invoke("install_skill_from_online", { source: sourceUrl.trim(), targetTools: tools });
      setSourceUrl("");
      setSelectedTools(new Set());
      await load();
    } catch (e: any) { alert(`安装失败: ${e}`); }
    finally { setInstallingFromSource(false); }
  };

  const toggleToolSelection = (toolId: string) => {
    setSelectedTools(prev => {
      const next = new Set(prev);
      if (next.has(toolId)) next.delete(toolId);
      else next.add(toolId);
      return next;
    });
  };

  const handleUninstall = async (id: string) => {
    try {
      await invoke("uninstall_skill", { skillId: id });
      setDeleteTarget(null);
      await load();
    } catch (e: any) { alert(`删除失败: ${e}`); }
  };

  const handleToggle = async (skillId: string, toolId: string, current: boolean) => {
    const key = `${skillId}:${toolId}`;
    setTogglingMap(prev => ({ ...prev, [key]: true }));
    try {
      await invoke("toggle_skill_tool", { skillId, toolId, enabled: !current });
      await load();
    } catch (e: any) { alert(`操作失败: ${e}`); }
    finally { setTogglingMap(prev => ({ ...prev, [key]: false })); }
  };

  const handleOpenSkillDir = async (dirPath: string) => {
    try { await invoke("open_ai_tool_cache_dir_path", { fullPath: dirPath }); }
    catch (e) { console.error(e); }
  };

  if (loading) {
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-xs">加载中...</span></div>;
  }

  return (
    <div className="h-full overflow-y-auto p-6 space-y-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-white">Skills 管理</h3>
          <p className="text-[10px] text-slate-500 mt-0.5">管理本地 AI 编程技能包</p>
        </div>
        <div className="flex gap-2">
          <button onClick={load} className="px-2.5 py-1.5 rounded-lg bg-white/5 border border-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex items-center gap-1">
            <RefreshCw className="w-3 h-3" /> 刷新
          </button>
          <button onClick={handleScanExisting} className="px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-[10px] font-semibold cursor-pointer transition-all flex items-center gap-1">
            <FolderOpen className="w-3 h-3" /> 导入已有
          </button>
          <button onClick={handleInstallLocal} className="px-3 py-1.5 rounded-lg bg-violet-600 hover:bg-violet-500 text-white text-[10px] font-semibold cursor-pointer transition-all flex items-center gap-1 shadow-lg shadow-violet-500/10">
            <Plus className="w-3 h-3" /> 本地安装
          </button>
        </div>
      </div>

      {/* 在线安装 */}
      <div className="p-3 rounded-xl bg-blue-500/5 border border-blue-500/10">
        <label className="text-[10px] text-blue-300 font-semibold block mb-1.5">安装技能</label>
        <div className="flex gap-2 mb-2">
          <input
            value={sourceUrl}
            onChange={(e) => setSourceUrl(e.target.value)}
            placeholder="Git 仓库 URL 或 owner/repo（如 vercel-labs/skills）"
            className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-3 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-blue-500"
          />
          <button
            onClick={handleBrowseLocal}
            className="px-3 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 text-[10px] text-slate-300 hover:text-white cursor-pointer transition-all flex items-center gap-1 flex-shrink-0"
            title="选择本地文件夹"
          >
            <FolderOpen className="w-3 h-3" />
            浏览
          </button>
        </div>

        {/* 目标工具选择 */}
        <div className="mb-2">
          <label className="text-[9px] text-slate-500 block mb-1">安装到以下工具（通过 JUNCTION 链接）:</label>
          <div className="flex flex-wrap gap-1.5">
            {TOOL_IDS.map(t => (
              <button
                key={t.id}
                onClick={() => toggleToolSelection(t.id)}
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

        <button
          onClick={handleInstallFromSource}
          disabled={installingFromSource || !sourceUrl.trim() || selectedTools.size === 0}
          className="w-full px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 disabled:opacity-40 text-white text-[10px] font-semibold cursor-pointer transition-all flex items-center justify-center gap-1"
        >
          {installingFromSource ? <RefreshCw className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />}
          {installingFromSource ? "安装中..." : `安装到 ${selectedTools.size} 个工具`}
        </button>
      </div>

      {/* 已检测到的技能（导入） */}
      {scannedSkills.length > 0 && (
        <div className="p-3 rounded-xl bg-emerald-500/5 border border-emerald-500/10">
          <div className="flex items-center justify-between mb-2">
            <h4 className="text-xs font-bold text-emerald-300 flex items-center gap-1.5">
              <FolderOpen className="w-3.5 h-3.5" />
              检测到已安装的技能 ({scannedSkills.length})
            </h4>
            <button onClick={() => setScannedSkills([])} className="text-[9px] text-slate-500 hover:text-slate-300 cursor-pointer">收起</button>
          </div>
          <div className="space-y-1.5">
            {scannedSkills.filter(s => !skills.some(ms => ms.directory === s.directory)).map(skill => (
              <div key={skill.full_path} className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-slate-900/30">
                <span className="text-[10px] font-semibold text-white flex-1">{skill.name}</span>
                <span className="text-[9px] text-slate-500 font-mono max-w-[200px] truncate">{skill.full_path}</span>
                {skill.is_symlink && <Link2 className="w-3 h-3 text-blue-400" />}
                <button
                  onClick={() => handleImportExisting(skill)}
                  disabled={importingExisting === skill.full_path}
                  className="px-2 py-0.5 rounded text-[9px] font-semibold bg-emerald-600 hover:bg-emerald-500 text-white cursor-pointer transition-all disabled:opacity-50"
                >
                  {importingExisting === skill.full_path ? "导入中..." : "导入"}
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 已安装技能卡片列表 */}
      {skills.length === 0 ? (
        <div className="h-48 border border-dashed border-white/5 rounded-2xl flex flex-col items-center justify-center text-slate-500">
          <Puzzle className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-xs font-bold text-slate-400">暂无已安装的技能</span>
          <span className="text-[10px] text-slate-600 mt-1">点击"本地安装"或"导入已有"添加技能</span>
        </div>
      ) : (
        <div className="space-y-2">
          {skills.map(skill => (
            <div
              key={skill.id}
              className="rounded-xl bg-slate-900/30 border border-white/5 p-4 hover:border-white/10 transition-all"
            >
              {/* 左侧：技能信息 */}
              <div className="flex items-start gap-3">
                <div className="p-2 rounded-lg bg-violet-500/10 flex-shrink-0">
                  <Puzzle className="w-4 h-4 text-violet-400" />
                </div>
                <div className="flex-1 min-w-0">
                  {/* 第一行：名称 + 标签 + 操作 */}
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-xs font-bold text-slate-200">{skill.name}</span>
                    <span className={`text-[8px] font-bold px-1.5 py-0.5 rounded-full ${
                      skill.install_method === "managed"
                        ? "bg-violet-500/15 text-violet-400"
                        : "bg-amber-500/15 text-amber-400"
                    }`}>
                      {skill.install_method === "managed" ? "托管" : "本地"}
                    </span>
                    <span className="text-[9px] text-slate-600">{skill.installed_at}</span>
                    {/* 操作按钮组 */}
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
                      <button
                        onClick={() => setDeleteTarget(skill.id)}
                        className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all"
                        title="卸载"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>

                  {/* 描述 */}
                  {skill.description && (
                    <p className="text-[10px] text-slate-400 mt-1 line-clamp-1">{skill.description}</p>
                  )}

                  {/* 路径 */}
                  <div className="text-[9px] text-slate-600 font-mono mt-1 truncate" title={skill.directory}>
                    {skill.directory}
                  </div>

                  {/* 工具开关行 */}
                  <div className="flex items-center gap-3 mt-3 pt-2 border-t border-white/[0.03]">
                    <span className="text-[9px] text-slate-500 flex-shrink-0">启用工具:</span>
                    <div className="flex flex-wrap gap-1.5">
                      {TOOL_IDS.map(t => {
                        const enabled = skill.enabled_tools.includes(t.id);
                        const key = `${skill.id}:${t.id}`;
                        const toggling = togglingMap[key] || false;
                        return (
                          <button
                            key={t.id}
                            onClick={() => handleToggle(skill.id, t.id, enabled)}
                            disabled={toggling}
                            className={`flex items-center gap-1 px-2 py-0.5 rounded-md text-[9px] font-semibold cursor-pointer transition-all border disabled:opacity-50 ${
                              enabled
                                ? "bg-emerald-500/15 border-emerald-500/30 text-emerald-400"
                                : "bg-slate-800 border-white/5 text-slate-600 hover:text-slate-400 hover:border-white/10"
                            }`}
                          >
                            <span className={`w-1.5 h-1.5 rounded-full ${enabled ? "bg-emerald-400" : "bg-slate-600"}`} />
                            {t.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Info */}
      <div className="p-3 rounded-xl bg-violet-500/5 border border-violet-500/10 text-[10px] text-slate-400 space-y-1">
        <p className="font-semibold text-violet-300">什么是 Skill？</p>
        <p>Skill 是遵循 skills.sh 规范的技能扩展包，启用后自动部署到对应工具的 skills/ 目录。</p>
      </div>

      {/* Delete confirm */}
      {deleteTarget && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4" onClick={() => setDeleteTarget(null)}>
          <div className="w-full max-w-sm bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl p-5" onClick={e => e.stopPropagation()}>
            <div className="flex items-center gap-3 mb-4">
              <div className="p-2 rounded-lg bg-red-500/10"><Trash2 className="w-4 h-4 text-red-400" /></div>
              <div>
                <h3 className="text-xs font-bold text-slate-200">确认卸载</h3>
                <p className="text-[10px] text-slate-500 mt-0.5">确定要卸载 {skills.find(s => s.id === deleteTarget)?.name} 吗？</p>
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
