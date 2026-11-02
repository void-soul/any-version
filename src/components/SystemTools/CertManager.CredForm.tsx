import { btnPrimary, btnSecondary } from "../shared/Button";
import { useRef, useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { CRED_FIELDS, DNS_PROVIDER_MAP, parseKV, type FieldSpec } from "./CertManager.model";
export function CredForm({ onDone }: { onDone: () => void }) {
  const { t } = useTranslation();
  const makeData = (fields: FieldSpec[]) => {
    const d: Record<string, string> = {};
    for (const f of fields) d[f.key] = "";
    return d;
  };
  const [name, setName] = useState("");
  const [credType, setCredType] = useState("alidns");
  const [data, setData] = useState<Record<string, string>>(() => makeData(CRED_FIELDS.alidns || []));
  const [customText, setCustomText] = useState("");
  const [note, setNote] = useState("");
  const [err, setErr] = useState("");
  const [saving, setSaving] = useState(false);
  const saveInFlight = useRef(false);

  useEffect(() => {
    setData(makeData(CRED_FIELDS[credType] || []));
    setCustomText("");
  }, [credType]);

  const setField = (key: string, val: string) => {
    setData((prev) => ({ ...prev, [key]: val }));
  };

  const isCustom = credType === "custom";

  const submit = async () => {
    if (saveInFlight.current) return;
    setErr("");
    saveInFlight.current = true;
    setSaving(true);
    try {
      await invoke("credential_upsert", {
        name,
        credType,
        data: isCustom ? parseKV(customText) : data,
        note,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    } finally {
      saveInFlight.current = false;
      setSaving(false);
    }
  };

  const fields = CRED_FIELDS[credType] || [];

  return (
    <fieldset disabled={saving} aria-busy={saving} className="ui-card min-w-0 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.name")}
          <input value={name} onChange={(e) => setName(e.target.value)} className="ui-input mt-1 w-full px-2.5 py-1.5" placeholder={t("certmgr.namePh")} />
        </label>
        <label className="text-caption text-slate-400">          {t("certmgr.dnsProvider")}
            <select value={credType} onChange={(e) => setCredType(e.target.value)} className="ui-input mt-1 w-full px-2.5 py-1.5">
            {([...DNS_PROVIDER_MAP, { id: "custom", label: t("certmgr.custom") }]).map((p) => (
              <option key={p.id} value={p.id} className="bg-slate-800 text-slate-200">{t(p.label)}</option>
            ))}
          </select>
        </label>
      </div>
      {isCustom ? (
        <label className="text-caption text-slate-400 block">
          {t("certmgr.envVars")}
          <textarea value={customText} onChange={(e) => setCustomText(e.target.value)} rows={4} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none font-mono text-tiny" placeholder="LEGO_ENV_VAR=value" />
        </label>
      ) : (
        <>
          <div className="text-caption text-slate-400 font-semibold">{t("certmgr.credFields")}</div>
          <div className="grid grid-cols-2 gap-3">
            {fields.map((f) => (
              <label key={f.key} className="text-caption text-slate-400">
                {t(f.label)}
                <input
                  type={f.type}
                  value={data[f.key] ?? ""}
                  onChange={(e) => setField(f.key, e.target.value)}
                  className="ui-input mt-1 w-full px-2.5 py-1.5"
                  placeholder={f.placeholder || ""}
                />
              </label>
            ))}
          </div>
        </>
      )}
      <label className="text-caption text-slate-400 block">
        {t("certmgr.remark")}
        <input value={note} onChange={(e) => setNote(e.target.value)} className="ui-input mt-1 w-full px-2.5 py-1.5" />
      </label>
      {err && <div role="alert" className="ui-note ui-note-error text-body px-3 py-2 break-words">{err}</div>}
      <div className="flex gap-2">
        <button type="button" onClick={submit} className={btnPrimary}>{saving ? t("common.loading") : t("certmgr.save")}</button>
        <button type="button" onClick={onDone} className={btnSecondary}>{t("certmgr.cancel")}</button>
      </div>
    </fieldset>
  );
}
