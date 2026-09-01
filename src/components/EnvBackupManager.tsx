import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
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
  const { t } = useTranslation();
  const [backups, setBackups] = useState<EnvBackup[]>([]);
  const [loading, setLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [repairLog] = useState<string[] | null>(null);
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
      const desc = description.trim() || t("envbackup.defaultDesc");
      const newBackup = await invoke<EnvBackup>("create_env_backup", { description: desc });
      setDescription("");
      setShowCreateForm(false);
      await fetchBackups();
      setSelectedBackup(newBackup);
    } catch (e: any) {
      alert(t("envbackup.createFail", { err: String(e) }));
    } finally {
      setCreating(false);
    }
  };

  const handleDeleteBackup = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm(t("envbackup.delConfirm"))) return;
    setDeletingId(id);
    try {
      await invoke("delete_env_backup", { id });
      if (selectedBackup?.id === id) {
        setSelectedBackup(null);
      }
      await fetchBackups();
    } catch (e: any) {
      alert(t("envbackup.delFail", { err: String(e) }));
    } finally {
      setDeletingId(null);
    }
  };

  const handleRestoreBackup = async (id: string) => {
    if (!confirm(t("envbackup.restoreConfirm"))) return;
    setRestoring(true);
    setRestoreMessage(null);
    try {
      await invoke("restore_env_backup", { id });
      setRestoreMessage({ text: t("envbackup.restoreOk"), isError: false });
    } catch (e: any) {
      // If error is related to admin permission warnings, display it as a warning instead of generic alert
      // （backendErrors 包装后原始中文消息保存在 rawMessage，这里用它做精确判断）
      const raw = (e as Error & { rawMessage?: string }).rawMessage ?? String(e);
      if (raw.includes("系统级环境变量恢复失败")) {
        setRestoreMessage({ text: String(e), isError: true });
      } else {
        alert(t("envbackup.restoreFail", { err: String(e) }));
      }
    } finally {
      setRestoring(false);
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
    <div className="glass-panel rounded-2xl p-6 border border-white/5 space-y-5">
      {/* Section header */}
      <div className="flex items-center justify-between pb-3 border-b border-white/5">
        <div className="flex items-center gap-2">
          <ShieldCheck className="w-4 h-4 text-[var(--module-accent)]" />
          <div>
            <h3 className="text-xs font-semibold text-white">{t("envbackup.title")}</h3>
            <p className="text-[9px] text-slate-500 mt-0.5">
              {t("envbackup.subtitle")}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => setShowCreateForm(!showCreateForm)}
            className="flex items-center gap-1.5 px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-xs font-semibold shadow-lg shadow-blue-500/20 cursor-pointer transition-all hover:scale-[1.02] active:scale-[0.98]"
          >
            <Plus className="w-4 h-4" />
            {t("envbackup.create")}
          </button>

          <button
            onClick={fetchBackups}
            disabled={loading}
            className="flex items-center gap-2 px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-xl text-xs border border-white/5 cursor-pointer transition-all"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            {t("envbackup.refresh")}
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
          <h3 className="text-xs font-semibold text-white">{t("envbackup.createFormTitle")}</h3>
          <div className="space-y-3">
            <input
              type="text"
              placeholder={t("envbackup.descPh")}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              className="w-full glass-input px-3.5 py-2 text-xs"
            />
            <div className="flex justify-end gap-2 text-xs">
              <button
                onClick={() => setShowCreateForm(false)}
                className="px-3.5 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-lg cursor-pointer"
              >
                {t("envbackup.cancel")}
              </button>
              <button
                onClick={handleCreateBackup}
                disabled={creating}
                className="px-4 py-2 bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-white rounded-lg font-semibold cursor-pointer transition-all"
              >
                {creating ? t("envbackup.backingUp") : t("envbackup.backupNow")}
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
            <h4 className="font-semibold text-xs">{restoreMessage.isError ? t("envbackup.restoreWarnTitle") : t("envbackup.restoreOkTitle")}</h4>
            <p className="text-[11px] mt-1 leading-relaxed whitespace-pre-line">{restoreMessage.text}</p>
          </div>
        </div>
      )}

      {/* Repair Log */}
      {repairLog && repairLog.length > 0 && (
        <div className="glass-panel p-4 rounded-2xl border border-amber-500/20 animate-fadeIn max-h-64 overflow-y-auto">
          <h4 className="text-xs font-semibold text-amber-300 mb-2 flex items-center gap-1.5">
            <Wrench className="w-3.5 h-3.5" />
            {t("envbackup.repairLog")}
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



      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
        {/* Left pane: Backups History */}
        <div className="lg:col-span-5 glass-panel rounded-2xl border border-white/5 overflow-hidden flex flex-col h-[520px]">
          <div className="p-4 bg-white/3 border-b border-white/5 flex items-center justify-between">
            <span className="text-xs font-semibold text-slate-300">{t("envbackup.historyTitle")}</span>
            <span className="text-[10px] text-slate-500">{t("envbackup.count", { count: backups.length })}</span>
          </div>

          <div className="flex-1 overflow-y-auto divide-y divide-white/5">
            {loading ? (
              <div className="p-12 text-center text-slate-500">
                <RefreshCw className="w-6 h-6 animate-spin text-blue-400 mx-auto mb-3" />
                {t("envbackup.loading")}
              </div>
            ) : backups.length === 0 ? (
              <div className="p-12 text-center text-slate-500 space-y-2">
                <ShieldCheck className="w-10 h-10 text-slate-600 mx-auto" />
                <p className="text-xs font-medium text-slate-400">{t("envbackup.empty")}</p>
                <p className="text-[10px] text-slate-500 max-w-[240px] mx-auto leading-relaxed">{t("envbackup.emptyHint")}</p>
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
                        title={t("envbackup.delTitle")}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>

                    <h4 className="font-semibold text-white text-xs leading-snug">{b.description}</h4>

                    <div className="flex items-center gap-3 text-[10px] text-slate-500">
                      <span>{t("envbackup.userVars")}<strong className="text-slate-300 font-mono">{Object.keys(b.user_vars).length}</strong></span>
                      <span>{t("envbackup.sysVars")}<strong className="text-slate-300 font-mono">{Object.keys(b.sys_vars).length}</strong></span>
                      <span className="text-slate-600">•</span>
                      <span>{t("envbackup.totalItems", { count: totalVars })}</span>
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
                  <p className="text-[10px] text-slate-400 mt-1">{t("envbackup.createdAt", { time: selectedBackup.timestamp, id: selectedBackup.id })}</p>
                </div>

                <button
                  onClick={() => handleRestoreBackup(selectedBackup.id)}
                  disabled={restoring}
                  className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 disabled:opacity-50 text-white rounded-xl text-xs font-semibold flex items-center gap-1.5 shadow-lg shadow-emerald-500/10 cursor-pointer transition-all hover:scale-[1.02] active:scale-[0.98]"
                >
                  <RotateCcw className={`w-3.5 h-3.5 ${restoring ? "animate-spin" : ""}`} />
                  {restoring ? t("envbackup.restoring") : t("envbackup.restore")}
                </button>
              </div>

              {/* Operations & search bar */}
              <div className="p-4 bg-white/1 border-b border-white/5 flex items-center gap-3">
                <div className="relative flex-1">
                  <Search className="w-3.5 h-3.5 text-slate-500 absolute left-3 top-2.5" />
                  <input
                    type="text"
                    placeholder={t("envbackup.searchPh")}
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
                    <h4 className="text-xs font-semibold text-slate-300">{t("envbackup.userVarsTitle")}</h4>
                  </div>

                  <div className="border border-white/5 rounded-xl overflow-hidden font-mono text-[10px]">
                    <table className="w-full text-left border-collapse">
                      <thead>
                        <tr className="bg-white/3 text-slate-400 font-semibold border-b border-white/5">
                          <th className="p-2 w-1/3">{t("envbackup.colKey")}</th>
                          <th className="p-2">{t("envbackup.colValue")}</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-white/5 text-slate-300">
                        {getFilteredVars(selectedBackup.user_vars).length === 0 ? (
                          <tr>
                            <td colSpan={2} className="p-4 text-center text-slate-500 italic">{t("envbackup.noMatch")}</td>
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
                    <h4 className="text-xs font-semibold text-slate-300">{t("envbackup.sysVarsTitle")}</h4>
                  </div>

                  <div className="border border-white/5 rounded-xl overflow-hidden font-mono text-[10px]">
                    <table className="w-full text-left border-collapse">
                      <thead>
                        <tr className="bg-white/3 text-slate-400 font-semibold border-b border-white/5">
                          <th className="p-2 w-1/3">{t("envbackup.colKey")}</th>
                          <th className="p-2">{t("envbackup.colValue")}</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-white/5 text-slate-300">
                        {getFilteredVars(selectedBackup.sys_vars).length === 0 ? (
                          <tr>
                            <td colSpan={2} className="p-4 text-center text-slate-500 italic">{t("envbackup.noMatch")}</td>
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
              <p className="text-xs font-medium text-slate-400">{t("envbackup.selectHint")}</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
