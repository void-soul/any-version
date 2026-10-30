import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { CheckCircle, Clock, Trash2, List, ListTree, Search, X, ChevronRight, Folder, GitFork } from "lucide-react";
import type { DetectedAiTool, ToolSession } from "../../contracts/ai";

/** Owns the ToolSessions feature, independently of page rendering. */
export function useToolSessions({ selectedTool, setProjectPath }: {
  setProjectPath: React.Dispatch<React.SetStateAction<string>>;
  selectedTool: DetectedAiTool | null;
}) {
  const { t } = useTranslation();

  const [sessions, setSessions] = useState<ToolSession[]>([]);

  const [sessionMode, setSessionMode] = useState<"new" | "continue" | "resume" | "fork">("new");

  const [selectedSession, setSelectedSession] = useState<ToolSession | null>(null);

  const [showSessionPicker, setShowSessionPicker] = useState(false);

  const [sessionViewMode, setSessionViewMode] = useState<"flat" | "grouped">("grouped");

  const [sessionSearch, setSessionSearch] = useState("");

  const [selectionMode, setSelectionMode] = useState(false);

  const [selectedSessionIds, setSelectedSessionIds] = useState<Set<string>>(new Set());

  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());

  // ── 会话分组 & 搜索 ──
  const filteredSessions = React.useMemo(() => {
    if (!sessionSearch.trim()) return sessions;
    const q = sessionSearch.toLowerCase();
    return sessions.filter(s =>
      s.project_path.toLowerCase().includes(q) ||
      (s.summary && s.summary.toLowerCase().includes(q)) ||
      s.session_id.toLowerCase().includes(q)
    );
  }, [sessions, sessionSearch]);

  const sessionDirGroups = React.useMemo(() => {
    const groups = new Map<string, { dir: string; label: string; sessions: ToolSession[] }>();
    for (const s of filteredSessions) {
      const dir = s.project_path || t("toollaunch.unknownDir");
      const label = dir.split(/[\\/]/).pop() || dir;
      if (!groups.has(dir)) groups.set(dir, { dir, label, sessions: [] });
      groups.get(dir)!.sessions.push(s);
    }
    return Array.from(groups.values()).sort((a, b) => a.label.localeCompare(b.label));
  }, [filteredSessions]);

  const handleDeleteSessions = async () => {
    if (selectedSessionIds.size === 0) return;
    if (!confirm(t("toollaunch.delSessionsConfirm", { count: selectedSessionIds.size }))) return;
    for (const sid of selectedSessionIds) {
      const s = sessions.find(x => x.session_id === sid);
      if (s) {
        try { await invoke("remove_ai_session", { toolId: selectedTool!.id, projectPath: s.project_path, sessionId: s.session_id }); }
        catch (e) { console.error(e); }
      }
    }
    setSelectedSessionIds(new Set());
    setSelectionMode(false);
    const updated = await invoke<ToolSession[]>("scan_tool_sessions", { toolId: selectedTool!.id }).catch(() => []);
    setSessions(updated);
  };

  const handleSelectAll = () => {
    if (selectedSessionIds.size === filteredSessions.length) setSelectedSessionIds(new Set());
    else setSelectedSessionIds(new Set(filteredSessions.map(s => s.session_id)));
  };

  const toggleSessionSelect = (sid: string) => {
    const next = new Set(selectedSessionIds);
    if (next.has(sid)) next.delete(sid); else next.add(sid);
    setSelectedSessionIds(next);
  };

  const toggleDirExpand = (dir: string) => {
    const next = new Set(expandedDirs);
    if (next.has(dir)) next.delete(dir); else next.add(dir);
    setExpandedDirs(next);
  };

const renderSessionPicker = () => selectedTool && (<div>
                  <div className="flex items-center justify-between mb-2">
                    <label className="text-body font-bold text-slate-300">{t("toollaunch.sessions")}</label>
                    {sessions.length > 0 && (
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => setSessionViewMode(sessionViewMode === "flat" ? "grouped" : "flat")}
                          className="p-1 rounded text-slate-500 hover:text-slate-300 cursor-pointer transition-all"
                          title={sessionViewMode === "flat" ? t("toollaunch.groupView") : t("toollaunch.listView")}
                        >
                          {sessionViewMode === "flat" ? <ListTree className="w-3.5 h-3.5" /> : <List className="w-3.5 h-3.5" />}
                        </button>
                        <button
                          onClick={() => { setSelectionMode(!selectionMode); setSelectedSessionIds(new Set()); }}
                          className={`p-1 rounded cursor-pointer transition-all ${selectionMode ? "text-[var(--module-accent)]" : "text-slate-500 hover:text-slate-300"}`}
                        >
                          <CheckCircle className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    )}
                  </div>

                  <div className="flex gap-2 flex-wrap mb-2">
                    <button onClick={() => { setSessionMode("new"); setSelectedSession(null); setShowSessionPicker(false); }}
                      className={`px-3 py-1.5 rounded-ctl text-tiny font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                        sessionMode === "new" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                      }`}>
                      {t("toollaunch.newSession")}
                    </button>
                    {sessions.length > 0 && (
                      <button onClick={() => { setSessionMode("resume"); setShowSessionPicker(!showSessionPicker); setSelectedSession(null); }}
                        className={`px-3 py-1.5 rounded-ctl text-tiny font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                          sessionMode === "resume" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                        }`}>
                        <Clock className="w-3 h-3" /> {t("toollaunch.historySessions", { count: sessions.length })}
                      </button>
                    )}
                    {/* 分叉：只有工具自己实现了 fork（注册表 forkCmd）才显示——
                        复制一份会话再进入，原会话保持不动 */}
                    {sessions.length > 0 && selectedTool?.fork_cmd && (
                      <button onClick={() => { setSessionMode("fork"); setShowSessionPicker(true); setSelectedSession(null); }}
                        className={`px-3 py-1.5 rounded-ctl text-tiny font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                          sessionMode === "fork" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                        }`}
                        title={t("toollaunch.forkSessionTip")}>
                        <GitFork className="w-3 h-3" /> {t("toollaunch.forkSession")}
                      </button>
                    )}
                  </div>

                  {showSessionPicker && (sessionMode === "resume" || sessionMode === "fork") && (
                    <div className="mb-2">
                      <div className="flex items-center gap-2">
                        <div className="flex-1 relative">
                          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500" />
                          <input value={sessionSearch} onChange={e => setSessionSearch(e.target.value)}
                            placeholder={t("toollaunch.searchSessions")} className="w-full ui-input rounded-ctl pl-7 pr-7 py-1.5 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                          {sessionSearch && (
                            <button onClick={() => setSessionSearch("")} className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300">
                              <X className="w-3 h-3" />
                            </button>
                          )}
                        </div>
                        {selectionMode && (
                          <>
                            <button onClick={handleSelectAll} className="px-2 py-1 rounded text-micro font-semibold bg-white/5 text-slate-400 hover:text-slate-200 cursor-pointer whitespace-nowrap">
                              {selectedSessionIds.size === filteredSessions.length ? t("toollaunch.cancelSelectAll") : t("toollaunch.selectAll")}
                            </button>
                            <button onClick={handleDeleteSessions} disabled={selectedSessionIds.size === 0}
                              className="px-2 py-1 rounded text-micro font-semibold bg-red-500/10 text-red-400 hover:bg-red-500/20 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed whitespace-nowrap flex items-center gap-1">
                              <Trash2 className="w-3 h-3" /> {t("toollaunch.delete", { count: selectedSessionIds.size })}
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                  )}

                  {showSessionPicker && (sessionMode === "resume" || sessionMode === "fork") && (
                    <div className="rounded-ctl border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="max-h-72 overflow-y-auto divide-y divide-white/[0.03]">
                        {filteredSessions.length === 0 ? (
                          <div className="px-3 py-6 text-tiny text-slate-600 text-center">
                            {sessionSearch ? t("toollaunch.noMatchSessions") : t("toollaunch.noHistorySessions")}
                          </div>
                        ) : sessionViewMode === "flat" ? (
                          filteredSessions.map(s => (
                            <div key={s.session_id}
                              className={`flex items-center px-3 py-2 text-tiny transition-all group ${
                                selectedSession?.session_id === s.session_id ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "text-slate-400 hover:bg-white/[0.03] hover:text-slate-200"
                              }`}>
                              {selectionMode && (
                                <button onClick={() => toggleSessionSelect(s.session_id)}
                                  className={`mr-2 w-4 h-4 rounded border flex-shrink-0 flex items-center justify-center cursor-pointer ${
                                    selectedSessionIds.has(s.session_id) ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white" : "border-slate-700 hover:border-slate-500"
                                  }`}>
                                  {selectedSessionIds.has(s.session_id) && <CheckCircle className="w-3 h-3" />}
                                </button>
                              )}
                              <button onClick={() => { if (!selectionMode) { setSelectedSession(s); setProjectPath(s.project_path); } }}
                                className="flex-1 text-left flex items-center justify-between min-w-0">
                                <div className="flex-1 min-w-0">
                                  <span className="font-mono text-slate-300 break-all block truncate">{s.project_path}</span>
                                  {s.summary && <div className="text-micro text-slate-500 mt-0.5 truncate italic">{s.summary}</div>}
                                </div>
                                <span className="text-micro text-slate-600 flex-shrink-0 ml-3">{s.last_used}</span>
                              </button>
                            </div>
                          ))
                        ) : (
                          sessionDirGroups.map(group => (
                            <div key={group.dir}>
                              <button onClick={() => toggleDirExpand(group.dir)}
                                className="w-full flex items-center gap-2 px-3 py-2 text-tiny bg-white/[0.02] hover:bg-white/[0.04] text-slate-400 hover:text-slate-200 cursor-pointer sticky top-0 z-10">
                                <ChevronRight className={`w-3 h-3 flex-shrink-0 transition-transform ${expandedDirs.has(group.dir) ? "rotate-90" : ""}`} />
                                <Folder className="w-3 h-3 flex-shrink-0 text-amber-500/70" />
                                <span className="font-semibold truncate">{group.label}</span>
                                <span className="text-micro text-slate-600 ml-auto">{group.sessions.length}</span>
                              </button>
                              {expandedDirs.has(group.dir) && group.sessions.map(s => (
                                <div key={s.session_id}
                                  className={`flex items-center pl-9 pr-3 py-2 text-tiny transition-all group ${
                                    selectedSession?.session_id === s.session_id ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "text-slate-400 hover:bg-white/[0.03] hover:text-slate-200"
                                  }`}>
                                  {selectionMode && (
                                    <button onClick={() => toggleSessionSelect(s.session_id)}
                                      className={`mr-2 w-3.5 h-3.5 rounded border flex-shrink-0 flex items-center justify-center cursor-pointer ${
                                        selectedSessionIds.has(s.session_id) ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white" : "border-slate-700 hover:border-slate-500"
                                      }`}>
                                      {selectedSessionIds.has(s.session_id) && <CheckCircle className="w-2.5 h-2.5" />}
                                    </button>
                                  )}
                                  <button onClick={() => { if (!selectionMode) { setSelectedSession(s); setProjectPath(s.project_path); } }}
                                    className="flex-1 text-left flex items-center justify-between min-w-0">
                                    <div className="flex-1 min-w-0">
                                      <span className="text-slate-400 truncate block">
                                        {s.session_id.slice(0, 8)}...
                                        {s.summary && <span className="text-micro text-slate-500 ml-2 italic truncate">{s.summary}</span>}
                                      </span>
                                    </div>
                                    <span className="text-micro text-slate-600 flex-shrink-0 ml-3">{s.last_used}</span>
                                  </button>
                                </div>
                              ))}
                            </div>
                          ))
                        )}
                      </div>
                    </div>
                  )}

                  {sessionMode === "resume" && selectedSession && (
                    <div className="mt-2 p-2 rounded-ctl bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] text-tiny text-[var(--module-accent)] flex items-center gap-2">
                      <CheckCircle className="w-3 h-3 flex-shrink-0" />
                      <span className="truncate">{t("toollaunch.willRestore", { path: selectedSession.project_path })}</span>
                    </div>
                  )}
                </div>);
  return {
    setSessions,
    sessionMode,
    setSessionMode,
    selectedSession,
    setSelectedSession,
    setShowSessionPicker,
    renderSessionPicker,
  };
}
