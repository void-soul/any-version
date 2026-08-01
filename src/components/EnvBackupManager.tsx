import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ShieldCheck,
  Trash2,
  RefreshCw,
  Clock,
  FileText,
  Plus,
  RotateCcw,
  AlertTriangle,
  Search,
  CheckCircle,
  Info,
  Wrench
} from "lucide-react";

interface EnvBackup {
  id: string;
  timestamp: string;
  description: string;
  user_vars: Record<string, string>;
  sys_vars: Record<string, string>;
}

export default function EnvBackupManager() {
  const [backups, setBackups] = useState<EnvBackup[]>([]);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [repairing, setRepairing] = useState(false);
  const [repairLog, setRepairLog] = useState<string[] | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  // Description for new backup
  const [description, setDescription] = useState("");
  const [showCreateForm, setShowCreateForm] = useState(false);

  // Selected backup for details view
  const [selectedBackup, setSelectedBackup] = useState<EnvBackup | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [restoreMessage, setRestoreMessage] = useState<{ text: string; isError: boolean } | null>(null);

  const fetchBackups = async () => {
    setLoading(true);
    try {
      const list = await invoke<EnvBackup[]>("list_env_backups");
      setBackups(list);

      // Auto select the latest backup if nothing selected
      if (list.length > 0 && !selectedBackup) {
        setSelectedBackup(list[0]);
      }
    } catch (e) {
      console.error("加载备份失败:", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchBackups();
  }, []);

  const handleCreateBackup = async () => {
    setCreating(true);
    setRestoreMessage(null);
    try {
      const desc = description.trim() || "手动创建的备份";
      const newBackup = await invoke<EnvBackup>("create_env_backup", { description: desc });
      setDescription("");
      setShowCreateForm(false);
      await fetchBackups();
      setSelectedBackup(newBackup);
    } catch (e: any) {
      alert(`创建备份失败: ${e}`);
    } finally {
      setCreating(false);
    }
  };

  const handleDeleteBackup = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm("确定要删除这个环境备份吗？删除后将无法通过此备份还原。")) return;
    setDeletingId(id);
    try {
      await invoke("delete_env_backup", { id });
      if (selectedBackup?.id === id) {
        setSelectedBackup(null);
      }
      await fetchBackups();
    } catch (e: any) {
      alert(`删除备份失败: ${e}`);
    } finally {
      setDeletingId(null);
    }
  };

  const handleRestoreBackup = async (id: string) => {
    if (!confirm("警告！还原环境变量将覆盖当前注册表中的所有环境变量（新增变量将被删除，现有变量将被重置为备份值）。\n确定要继续还原吗？")) return;
    setRestoring(true);
    setRestoreMessage(null);
    try {
      await invoke("restore_env_backup", { id });
      setRestoreMessage({ text: "环境变量已成功恢复！更改已广播至系统。", isError: false });
    } catch (e: any) {
      // If error is related to admin permission warnings, display it as a warning instead of generic alert
      if (e.toString().includes("系统级环境变量恢复失败")) {
        setRestoreMessage({ text: e.toString(), isError: true });
      } else {
        alert(`还原失败: ${e}`);
      }
    } finally {
      setRestoring(false);
    }
  };

  const handleRepairRegistry = async () => {
    if (!confirm("此操作将修复注册表中环境变量的类型错误（REG_SZ ↔ REG_EXPAND_SZ）。\n\n这不会修改变量的值，只修正其存储类型。\n\n如果因还原备份导致 PATH 等含 %SystemRoot% 的变量损坏（例如无法打开'高级系统设置'），此修复可以解决。\n\n确定执行修复吗？")) return;
    setRepairing(true);
    setRepairLog(null);
    setRestoreMessage(null);
    try {
      const log = await invoke<string[]>("repair_registry_env_types");
      setRepairLog(log);
      setRestoreMessage({ text: `修复完成！共处理 ${log.length} 项（详见下方日志）。更改已广播至系统。`, isError: false });
    } catch (e: any) {
      setRestoreMessage({ text: `修复失败: ${e}`, isError: true });
    } finally {
      setRepairing(false);
    }
  };

  // Filter variables based on search query
  const getFilteredVars = (vars: Record<string, string>) => {
    if (!searchQuery) return Object.entries(vars);
    const query = searchQuery.toLowerCase();
    return Object.entries(vars).filter(
      ([key, val]) => key.toLowerCase().includes(query) || val.toLowerCase().includes(query)
    );
  };

  return (
    <div className="flex-1 p-8 overflow-y-auto space-y-6 h-screen select-none flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-semibold text-white tracking-wide">环境备份还原</h2>
          <p className="text-xs text-slate-400 mt-1">
            提供环境变量注册表备份与还原服务，防范开发工具误删/覆盖 PATH 导致系统崩溃。
          </p>
        </div>

        <div className="flex items-center gap-3">
          <button
            onClick={() => setShowCreateForm(!showCreateForm)}
            className="flex items-center gap-1.5 px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all hover:scale-[1.02] active:scale-[0.98]"
          >
            <Plus className="w-4 h-4" />
            创建备份
          </button>

          <button
            onClick={fetchBackups}
            disabled={loading}
            className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 cursor-pointer transition-all"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            刷新列表
          </button>

          {/* <button
            onClick={handleRepairRegistry}
            disabled={repairing}
            className="flex items-center gap-1.5 px-4 py-2 bg-amber-600 hover:bg-amber-500 text-white rounded-xl text-xs font-semibold shadow-lg shadow-amber-500/20 cursor-pointer transition-all hover:scale-[1.02] active:scale-[0.98] disabled:opacity-50"
          >
            <Wrench className="w-4 h-4" />
            {repairing ? "修复中..." : "修复注册表"}
          </button> */}
        </div>
      </div>

      {/* Backup Form Overlay/Dropdown */}
      {showCreateForm && (
        <div className="glass-panel p-5 rounded-2xl border border-white/5 space-y-4 max-w-xl animate-fadeIn">
          <h3 className="text-xs font-semibold text-white">创建当前环境备份</h3>
          <div className="space-y-3">
            <input
              type="text"
              placeholder="输入本次备份的描述（例如: 安装 SDK 之前的备份，或 初始环境变量备份）"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              className="w-full glass-input px-3.5 py-2 text-xs"
            />
            <div className="flex justify-end gap-2 text-xs">
              <button
                onClick={() => setShowCreateForm(false)}
                className="px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg cursor-pointer"
              >
                取消
              </button>
              <button
                onClick={handleCreateBackup}
                disabled={creating}
                className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg font-semibold cursor-pointer transition-all"
              >
                {creating ? "正在备份..." : "立即备份"}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Restore Result Notification Alert */}
      {restoreMessage && (
        <div className={`p-4 rounded-xl border flex items-start gap-3 animate-fadeIn ${restoreMessage.isError
            ? "bg-amber-500/10 border-amber-500/20 text-amber-300"
            : "bg-emerald-500/10 border-emerald-500/20 text-emerald-400"
          }`}>
          {restoreMessage.isError ? (
            <AlertTriangle className="w-5 h-5 flex-shrink-0 text-amber-400 mt-0.5" />
          ) : (
            <CheckCircle className="w-5 h-5 flex-shrink-0 text-emerald-400 mt-0.5" />
          )}
          <div>
            <h4 className="font-semibold text-xs">{restoreMessage.isError ? "环境变量恢复完成（有警告）" : "环境变量恢复成功"}</h4>
            <p className="text-[11px] mt-1 leading-relaxed whitespace-pre-line">{restoreMessage.text}</p>
          </div>
        </div>
      )}

      {/* Repair Log */}
      {repairLog && repairLog.length > 0 && (
        <div className="glass-panel p-4 rounded-2xl border border-amber-500/20 animate-fadeIn max-h-64 overflow-y-auto">
          <h4 className="text-xs font-semibold text-amber-300 mb-2 flex items-center gap-1.5">
            <Wrench className="w-3.5 h-3.5" />
            修复日志
          </h4>
          <div className="space-y-0.5">
            {repairLog.map((line, i) => {
              const isError = line.startsWith("❌");
              const isWarning = line.startsWith("⚠️");
              const isOk = line.startsWith("OK");
              const isInfo = line.startsWith("ℹ️");
              return (
                <p key={i} className={`text-[10px] leading-relaxed font-mono ${isError ? "text-red-400" :
                    isWarning ? "text-amber-300" :
                      isOk ? "text-slate-500" :
                        isInfo ? "text-blue-300" :
                          "text-emerald-400"
                  }`}>
                  {line}
                </p>
              );
            })}
          </div>
        </div>
      )}



      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6 flex-1 min-h-0">
        {/* Left pane: Backups History */}
        <div className="lg:col-span-5 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col h-[520px]">
          <div className="p-4 bg-white/3 border-b border-white/5 flex items-center justify-between">
            <span className="text-xs font-semibold text-slate-300">历史环境备份列表</span>
            <span className="text-[10px] text-slate-500">共 {backups.length} 个备份</span>
          </div>

          <div className="flex-1 overflow-y-auto divide-y divide-white/5">
            {loading ? (
              <div className="p-12 text-center text-slate-500">
                <RefreshCw className="w-6 h-6 animate-spin text-blue-400 mx-auto mb-3" />
                正在加载备份记录...
              </div>
            ) : backups.length === 0 ? (
              <div className="p-12 text-center text-slate-500 space-y-2">
                <ShieldCheck className="w-10 h-10 text-slate-600 mx-auto" />
                <p className="text-xs font-medium text-slate-400">目前暂无环境备份</p>
                <p className="text-[10px] text-slate-500 max-w-[240px] mx-auto leading-relaxed">建议在每次一键修复或安装新的 SDK 前，先点击上方“创建备份”安全归档。</p>
              </div>
            ) : (
              backups.map((b) => {
                const isSelected = selectedBackup?.id === b.id;
                const totalVars = Object.keys(b.user_vars).length + Object.keys(b.sys_vars).length;
                return (
                  <div
                    key={b.id}
                    onClick={() => {
                      setSelectedBackup(b);
                      setRestoreMessage(null);
                    }}
                    className={`p-4 flex flex-col gap-2 hover:bg-white/2 cursor-pointer transition-all ${isSelected ? "bg-blue-600/5 border-l-2 border-blue-500" : ""
                      }`}
                  >
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-1.5 text-[10px] text-slate-400 font-mono">
                        <Clock className="w-3.5 h-3.5 text-slate-500" />
                        {b.timestamp}
                      </div>

                      <button
                        onClick={(e) => handleDeleteBackup(b.id, e)}
                        disabled={deletingId === b.id}
                        className="p-1 hover:bg-red-500/10 hover:text-red-400 rounded text-slate-500 transition-all cursor-pointer"
                        title="删除备份"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>

                    <h4 className="font-semibold text-white text-xs leading-snug">{b.description}</h4>

                    <div className="flex items-center gap-3 text-[10px] text-slate-500">
                      <span>用户变量: <strong className="text-slate-300 font-mono">{Object.keys(b.user_vars).length}</strong></span>
                      <span>系统变量: <strong className="text-slate-300 font-mono">{Object.keys(b.sys_vars).length}</strong></span>
                      <span className="text-slate-600">•</span>
                      <span>共 {totalVars} 项</span>
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </div>

        {/* Right pane: Backup Variables Inspection & Restore */}
        <div className="lg:col-span-7 flex flex-col h-[520px]">
          {selectedBackup ? (
            <div className="flex-1 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col">
              {/* Backup details header */}
              <div className="p-5 border-b border-white/5 flex items-center justify-between bg-white/2">
                <div className="min-w-0">
                  <h3 className="text-xs font-semibold text-white truncate">{selectedBackup.description}</h3>
                  <p className="text-[10px] text-slate-400 mt-1">创建时间: {selectedBackup.timestamp} | ID: {selectedBackup.id}</p>
                </div>

                <button
                  onClick={() => handleRestoreBackup(selectedBackup.id)}
                  disabled={restoring}
                  className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold flex items-center gap-1.5 shadow-lg shadow-emerald-500/10 cursor-pointer transition-all hover:scale-[1.02] active:scale-[0.98]"
                >
                  <RotateCcw className={`w-3.5 h-3.5 ${restoring ? "animate-spin" : ""}`} />
                  {restoring ? "正在还原..." : "还原此备份"}
                </button>
              </div>

              {/* Operations & search bar */}
              <div className="p-4 bg-white/1 border-b border-white/5 flex items-center gap-3">
                <div className="relative flex-1">
                  <Search className="w-3.5 h-3.5 text-slate-500 absolute left-3 top-2.5" />
                  <input
                    type="text"
                    placeholder="输入关键词搜索备份中的环境变量键或值..."
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    className="w-full glass-input pl-9 pr-3.5 py-1.5 text-[11px] font-sans"
                  />
                </div>
              </div>

              {/* Variables List Tables */}
              <div className="flex-1 overflow-y-auto p-5 space-y-6">
                {/* User Variables */}
                <div className="space-y-2">
                  <div className="flex items-center gap-1.5">
                    <FileText className="w-4 h-4 text-blue-400" />
                    <h4 className="text-xs font-semibold text-slate-300">用户环境变量 (HKEY_CURRENT_USER\Environment)</h4>
                  </div>

                  <div className="border border-white/5 rounded-xl overflow-hidden font-mono text-[10px]">
                    <table className="w-full text-left border-collapse">
                      <thead>
                        <tr className="bg-white/3 text-slate-400 font-semibold border-b border-white/5">
                          <th className="p-2 w-1/3">变量名 (Key)</th>
                          <th className="p-2">备份值 (Value)</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-white/5 text-slate-300">
                        {getFilteredVars(selectedBackup.user_vars).length === 0 ? (
                          <tr>
                            <td colSpan={2} className="p-4 text-center text-slate-500 italic">无匹配的变量</td>
                          </tr>
                        ) : (
                          getFilteredVars(selectedBackup.user_vars).map(([k, v]) => (
                            <tr key={k} className="hover:bg-white/1">
                              <td className="p-2 font-semibold text-slate-200 break-all select-text">{k}</td>
                              <td className="p-2 break-all select-text whitespace-pre-wrap">{v}</td>
                            </tr>
                          ))
                        )}
                      </tbody>
                    </table>
                  </div>
                </div>

                {/* System Variables */}
                <div className="space-y-2">
                  <div className="flex items-center gap-1.5">
                    <ShieldCheck className="w-4 h-4 text-purple-400" />
                    <h4 className="text-xs font-semibold text-slate-300">系统环境变量 (HKEY_LOCAL_MACHINE\Session Manager...)</h4>
                  </div>

                  <div className="border border-white/5 rounded-xl overflow-hidden font-mono text-[10px]">
                    <table className="w-full text-left border-collapse">
                      <thead>
                        <tr className="bg-white/3 text-slate-400 font-semibold border-b border-white/5">
                          <th className="p-2 w-1/3">变量名 (Key)</th>
                          <th className="p-2">备份值 (Value)</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-white/5 text-slate-300">
                        {getFilteredVars(selectedBackup.sys_vars).length === 0 ? (
                          <tr>
                            <td colSpan={2} className="p-4 text-center text-slate-500 italic">无匹配的变量</td>
                          </tr>
                        ) : (
                          getFilteredVars(selectedBackup.sys_vars).map(([k, v]) => (
                            <tr key={k} className="hover:bg-white/1">
                              <td className="p-2 font-semibold text-slate-200 break-all select-text">{k}</td>
                              <td className="p-2 break-all select-text whitespace-pre-wrap">{v}</td>
                            </tr>
                          ))
                        )}
                      </tbody>
                    </table>
                  </div>
                </div>
              </div>
            </div>
          ) : (
            <div className="flex-1 glass-panel rounded-2xl border border-white/5 flex flex-col items-center justify-center text-center text-slate-500 p-8">
              <Info className="w-12 h-12 text-slate-600 mb-4" />
              <p className="text-xs font-medium text-slate-400">选择左侧列表中的环境备份查看详情和进行还原</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
