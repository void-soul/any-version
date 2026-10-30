import { useState, useEffect, useCallback } from "react";
import { theamedAlert } from "../shared/ThemedAlert";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Plus, RefreshCw } from "lucide-react";
import { fmtDate, type MonitorTarget } from "./CertManager.model";
import { MonitorForm } from "./CertManager.MonitorForm";
// ---- 证书监控（外部域名 / IP） ----
export function MonitorView() {
  const { t } = useTranslation();
  const [items, setItems] = useState<MonitorTarget[]>([]);
  const [editing, setEditing] = useState<MonitorTarget | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [checking, setChecking] = useState(false);

  const refresh = useCallback(async () => {
    setItems(await invoke<MonitorTarget[]>("monitor_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const checkAll = async () => {
    setChecking(true);
    try {
      setItems(await invoke<MonitorTarget[]>("monitor_check_all"));
    } catch (e) {
      theamedAlert(String(e));
    } finally {
      setChecking(false);
    }
  };
  const checkOne = async (id: string) => {
    try {
      await invoke<MonitorTarget>("monitor_check", { id });
    } catch (e) {
      theamedAlert(String(e));
    }
    refresh();
  };
  const del = async (id: string) => {
    if (!confirm(t("certmgr.delMonitorConfirm"))) return;
    await invoke("monitor_delete", { id });
    refresh();
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-slate-200">{t("certmgr.monitorsTitle")}</h3>
          <p className="text-tiny text-slate-500 mt-1">{t("certmgr.monitorsHint")}</p>
        </div>
        <div className="flex gap-2">
          <button onClick={checkAll} disabled={checking} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-200 text-caption flex items-center gap-1.5">
            <RefreshCw className={`w-3.5 h-3.5 ${checking ? "animate-spin" : ""}`} /> {checking ? t("certmgr.checking") : t("certmgr.checkAll")}
          </button>
          <button onClick={() => { setEditing(null); setShowForm(true); }} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
            <Plus className="w-3.5 h-3.5" /> {t("certmgr.newMonitor")}
          </button>
        </div>
      </div>

      {showForm && (
        <MonitorForm
          initial={editing}
          onDone={() => { setShowForm(false); setEditing(null); refresh(); }}
        />
      )}

      <div className="space-y-2">
        {items.map((m) => {
          const d = m.days_left;
          const tone = d == null ? "text-slate-400" : d <= 7 ? "text-rose-400" : d <= 30 ? "text-amber-400" : "text-emerald-400";
          return (
            <div key={m.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-body font-semibold text-slate-200 truncate">
                    {m.host}:{m.port}
                    {m.note && <span className="ml-2 text-tiny text-slate-500 font-normal">{m.note}</span>}
                  </div>
                  <div className="text-tiny text-slate-500 mt-1">
                    {t("certmgr.monitorExpire", { time: fmtDate(m.not_after) })} · {t("certmgr.monitorChecked", { time: fmtDate(m.last_check_at) })}
                  </div>
                  {m.subject && <div className="text-tiny text-slate-500 truncate">{m.subject} · {m.issuer}</div>}
                </div>
                <div className="flex items-center gap-3 flex-shrink-0">
                  <span className={`text-caption font-semibold ${tone}`}>
                    {d == null ? "-" : d >= 0 ? t("certmgr.daysLeft", { days: d }) : t("certmgr.daysExpired", { days: -d })}
                  </span>
                  <button onClick={() => checkOne(m.id)} className="text-caption text-sky-400 hover:text-sky-300">{t("certmgr.checkNow")}</button>
                  <button onClick={() => { setEditing(m); setShowForm(true); }} className="text-caption text-slate-300 hover:text-white">{t("certmgr.edit")}</button>
                  <button onClick={() => del(m.id)} className="text-caption text-rose-400 hover:text-rose-300">{t("certmgr.delete")}</button>
                </div>
              </div>
              {m.last_error && <div className="text-tiny text-rose-400 mt-1">⚠ {m.last_error}</div>}
            </div>
          );
        })}
        {items.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.monitorsEmpty")}</div>}
      </div>
    </div>
  );
}
