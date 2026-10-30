import { useState, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Plus, Trash2 } from "lucide-react";
import { type Credential } from "./CertManager.model";
import { CredForm } from "./CertManager.CredForm";
// ---- DNS凭据 ----
export function Credentials() {
  const { t } = useTranslation();
  const [creds, setCreds] = useState<Credential[]>([]);
  const [showForm, setShowForm] = useState(false);

  const refresh = useCallback(async () => {
    setCreds(await invoke<Credential[]>("credential_list"));
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);

  const del = async (id: string) => {
    if (!confirm(t("certmgr.delCredConfirm"))) return;
    await invoke("credential_delete", { id });
    refresh();
  };

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-bold text-slate-200">{t("certmgr.credsTitle")}</h3>
        <button onClick={() => setShowForm((v) => !v)} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold flex items-center gap-1.5">
          <Plus className="w-3.5 h-3.5" /> {t("certmgr.newCred")}
        </button>
      </div>
      {showForm && <CredForm onDone={() => { setShowForm(false); refresh(); }} />}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {creds.map((c) => (
          <div key={c.id} className="rounded-ctl border border-white/5 bg-white/5 p-3">
            <div className="flex items-center justify-between">
              <span className="text-body font-semibold text-slate-200">{c.name}</span>
              <span className="text-tiny px-2 py-0.5 rounded bg-amber-500/20 text-amber-300">{c.type}</span>
            </div>
            <div className="text-tiny text-slate-500 mt-1">{t("certmgr.fieldsLabel", { names: Object.keys(c.data).join(", ") })}</div>
            <div className="mt-2 text-caption">
              <button onClick={() => del(c.id)} className="text-rose-400 hover:text-rose-300 flex items-center gap-1"><Trash2 className="w-3 h-3" />{t("certmgr.delete")}</button>
            </div>
          </div>
        ))}
        {creds.length === 0 && <div className="text-caption text-slate-500">{t("certmgr.credsEmpty")}</div>}
      </div>
    </div>
  );
}
