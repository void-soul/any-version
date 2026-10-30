import { useState, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Plus } from "lucide-react";
import { NOTIFY_TYPES, type NotifyChannel } from "./CertManager.model";
import { NotifyForm } from "./CertManager.NotifyForm";
export function NotifyView() {
  const { t } = useTranslation();
  const [items, setItems] = useState<NotifyChannel[]>([]);
  const [editing, setEditing] = useState<NotifyChannel | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [msg, setMsg] = useState("");

  const refresh = useCallback(async () => {
    setItems(await invoke<NotifyChannel[]>("notify_channel_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const test = async (id: string) => {
    setMsg(t("certmgr.testing"));
    try {
      setMsg(await invoke<string>("notify_channel_test", { id }));
    } catch (e) {
      setMsg(t("certmgr.testFail", { err: String(e) }));
    }
  };
  const del = async (id: string) => {
    if (!confirm(t("certmgr.delNotifyConfirm"))) return;
    await invoke("notify_channel_delete", { id });
    refresh();
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-bold text-slate-200">{t("certmgr.notifyTitle")}</h3>
          <p className="text-tiny text-slate-500 mt-1">{t("certmgr.notifyHint")}</p>
        </div>
        <button onClick={() => { setEditing(null); setShowForm(true); }} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newNotify")}
        </button>
      </div>

      {showForm && (
        <NotifyForm
          initial={editing}
          onDone={() => { setShowForm(false); setEditing(null); refresh(); }}
        />
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {items.map((c) => (
          <div key={c.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <div className="text-body font-semibold text-slate-200">{c.name}</div>
              <span className="text-tiny px-2 py-0.5 rounded bg-sky-500/20 text-sky-300">
                {NOTIFY_TYPES.find((x) => x.value === c.type)?.label ?? c.type}
              </span>
            </div>
            <div className="text-tiny text-slate-500 mt-1 break-all">{c.config.url}</div>
            {!c.enabled && <div className="text-tiny text-amber-400 mt-1">{t("certmgr.notifyDisabled")}</div>}
            <div className="flex gap-3 mt-2 text-caption">
              <button onClick={() => { setEditing(c); setShowForm(true); }} className="text-slate-300 hover:text-white">{t("certmgr.edit")}</button>
              <button onClick={() => test(c.id)} className="text-sky-400 hover:text-sky-300">{t("certmgr.sendTest")}</button>
              <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300">{t("certmgr.delete")}</button>
            </div>
          </div>
        ))}
        {items.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.notifyEmpty")}</div>}
      </div>
      {msg && <div className="text-caption text-slate-400">{t("certmgr.testLabel", { msg })}</div>}
    </div>
  );
}
