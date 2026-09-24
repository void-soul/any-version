import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, CheckCircle, KeyRound, RefreshCw, Save, Trash2, Upload } from "lucide-react";

/// Codex 官方登录态（~/.codex/auth.json）账号快照管理。
/// 抄自 ai-toolbox c126d68e：粘贴导入只落快照，只有「应用」才写运行时文件（写前先备份）。
interface CodexAuthSummary {
  id: string;
  label: string;
  savedAt: string;
  isCurrent: boolean;
}

interface CodexAuthPaths {
  liveAuth: string | null;
  liveExists: boolean;
  store: string;
}

export default function CodexAuthPanel() {
  const { t } = useTranslation();
  const [paths, setPaths] = useState<CodexAuthPaths | null>(null);
  const [accounts, setAccounts] = useState<CodexAuthSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [label, setLabel] = useState("");
  const [authJson, setAuthJson] = useState("");
  const [importing, setImporting] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [p, list] = await Promise.all([
        invoke<CodexAuthPaths>("codex_auth_paths"),
        invoke<CodexAuthSummary[]>("codex_auth_list"),
      ]);
      setPaths(p);
      setAccounts(list);
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleImport = async () => {
    if (!authJson.trim()) {
      setMessage({ ok: false, text: t("codexauth.pasteRequired") });
      return;
    }
    setImporting(true);
    setMessage(null);
    try {
      const summary = await invoke<CodexAuthSummary>("codex_auth_import", {
        authJson,
        label: label.trim() || null,
        overwrite: false,
      });
      setAuthJson("");
      setLabel("");
      setMessage({ ok: true, text: t("codexauth.imported", { label: summary.label }) });
      await refresh();
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setImporting(false);
    }
  };

  const handleCaptureCurrent = async () => {
    setImporting(true);
    setMessage(null);
    try {
      const summary = await invoke<CodexAuthSummary>("codex_auth_capture_current", { label: null });
      setMessage({ ok: true, text: t("codexauth.captured", { label: summary.label }) });
      await refresh();
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setImporting(false);
    }
  };

  const handleApply = async (account: CodexAuthSummary) => {
    setBusyId(account.id);
    setMessage(null);
    try {
      const text = await invoke<string>("codex_auth_apply", { id: account.id });
      setMessage({ ok: true, text });
      await refresh();
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setBusyId(null);
    }
  };

  const handleDelete = async (account: CodexAuthSummary) => {
    setBusyId(account.id);
    setMessage(null);
    try {
      await invoke("codex_auth_delete", { id: account.id });
      await refresh();
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="h-full overflow-y-auto p-4 space-y-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-xs font-bold text-slate-200 flex items-center gap-2">
            <KeyRound className="w-3.5 h-3.5 text-[var(--module-accent)]" />
            {t("codexauth.title")}
          </h3>
          <p className="text-[10px] text-slate-500 mt-1">{t("codexauth.hint")}</p>
        </div>
        <button
          onClick={() => void refresh()}
          disabled={loading}
          className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] text-slate-400 hover:text-slate-200 hover:bg-white/5 disabled:opacity-40 cursor-pointer transition-all"
        >
          <RefreshCw className={`w-3 h-3 ${loading ? "animate-spin" : ""}`} />
          {t("codexauth.refresh")}
        </button>
      </div>

      {/* 运行时路径 */}
      <div className="p-3 rounded-xl bg-slate-900/40 border border-white/5 space-y-1.5">
        <div className="flex items-center gap-2 text-[10px]">
          {paths?.liveExists ? (
            <CheckCircle className="w-3 h-3 text-emerald-400 flex-shrink-0" />
          ) : (
            <AlertTriangle className="w-3 h-3 text-amber-400 flex-shrink-0" />
          )}
          <span className="text-slate-400">{t("codexauth.liveAuth")}</span>
          <span className="font-mono text-slate-300 truncate">{paths?.liveAuth ?? "-"}</span>
          <span className={`ml-auto flex-shrink-0 ${paths?.liveExists ? "text-emerald-400" : "text-amber-400"}`}>
            {paths?.liveExists ? t("codexauth.livePresent") : t("codexauth.liveMissing")}
          </span>
        </div>
        <div className="flex items-center gap-2 text-[10px]">
          <Save className="w-3 h-3 text-slate-500 flex-shrink-0" />
          <span className="text-slate-400">{t("codexauth.store")}</span>
          <span className="font-mono text-slate-500 truncate">{paths?.store ?? "-"}</span>
        </div>
      </div>

      {/* 导入 */}
      <div className="p-3 rounded-xl bg-slate-900/40 border border-white/5 space-y-2">
        <label className="text-[10px] text-slate-400 font-semibold block">{t("codexauth.importTitle")}</label>
        <p className="text-[9px] text-slate-600">{t("codexauth.importHint")}</p>
        <input
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder={t("codexauth.labelPlaceholder")}
          className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
        />
        <textarea
          value={authJson}
          onChange={(e) => setAuthJson(e.target.value)}
          rows={5}
          placeholder={t("codexauth.pastePlaceholder")}
          className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-[10px] font-mono text-slate-300 resize-y focus:outline-none focus:border-[var(--module-accent)]"
        />
        <div className="flex items-center gap-2">
          <button
            onClick={() => void handleImport()}
            disabled={importing}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-semibold text-white bg-[var(--module-accent)] hover:brightness-110 disabled:opacity-50 cursor-pointer transition-all"
          >
            <Upload className="w-3 h-3" />
            {t("codexauth.import")}
          </button>
          <button
            onClick={() => void handleCaptureCurrent()}
            disabled={importing || !paths?.liveExists}
            title={!paths?.liveExists ? t("codexauth.liveMissing") : undefined}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] text-slate-300 border border-white/10 hover:bg-white/5 disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer transition-all"
          >
            <Save className="w-3 h-3" />
            {t("codexauth.captureCurrent")}
          </button>
        </div>
      </div>

      {message && (
        <div
          className={`px-3 py-2 rounded-lg text-[10px] border ${
            message.ok
              ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300"
              : "bg-red-500/10 border-red-500/30 text-red-300"
          }`}
        >
          {message.text}
        </div>
      )}

      {/* 快照列表 */}
      <div className="space-y-1.5">
        <div className="text-[10px] text-slate-400 font-semibold">
          {t("codexauth.accounts", { count: accounts.length })}
        </div>
        {accounts.length === 0 ? (
          <div className="px-3 py-6 rounded-xl border border-dashed border-white/10 text-center text-[10px] text-slate-600">
            {t("codexauth.empty")}
          </div>
        ) : (
          <div className="rounded-xl border border-white/5 overflow-hidden divide-y divide-white/[0.04]">
            {accounts.map((account) => (
              <div key={account.id} className="flex items-center gap-2 px-3 py-2 bg-slate-900/20">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] text-slate-200 truncate">{account.label}</span>
                    {account.isCurrent && (
                      <span className="px-1.5 py-0.5 rounded text-[9px] bg-emerald-500/15 text-emerald-300 flex-shrink-0">
                        {t("codexauth.current")}
                      </span>
                    )}
                  </div>
                  <div className="text-[9px] text-slate-600 font-mono truncate">
                    {account.savedAt} · {account.id}
                  </div>
                </div>
                <button
                  onClick={() => void handleApply(account)}
                  disabled={busyId === account.id || account.isCurrent}
                  className="px-2 py-1 rounded-md text-[10px] text-slate-300 border border-white/10 hover:bg-white/5 disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer transition-all"
                >
                  {t("codexauth.apply")}
                </button>
                <button
                  onClick={() => void handleDelete(account)}
                  disabled={busyId === account.id}
                  title={t("codexauth.delete")}
                  className="p-1 rounded-md text-slate-500 hover:text-red-400 disabled:opacity-40 cursor-pointer transition-all"
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
