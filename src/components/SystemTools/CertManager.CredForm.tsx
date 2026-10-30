import { useState, useEffect } from "react";
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

  useEffect(() => {
    setData(makeData(CRED_FIELDS[credType] || []));
    setCustomText("");
  }, [credType]);

  const setField = (key: string, val: string) => {
    setData((prev) => ({ ...prev, [key]: val }));
  };

  const isCustom = credType === "custom";

  const submit = async () => {
    setErr("");
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
    }
  };

  const fields = CRED_FIELDS[credType] || [];

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.name")}
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder={t("certmgr.namePh")} />
        </label>
        <label className="text-caption text-slate-400">          {t("certmgr.dnsProvider")}
            <select value={credType} onChange={(e) => setCredType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
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
                  className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none"
                  placeholder={f.placeholder || ""}
                />
              </label>
            ))}
          </div>
        </>
      )}
      <label className="text-caption text-slate-400 block">
        {t("certmgr.remark")}
        <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
      </label>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}
