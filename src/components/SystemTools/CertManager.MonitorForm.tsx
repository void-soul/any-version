import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { type MonitorTarget } from "./CertManager.model";
export function MonitorForm({ initial, onDone }: { initial: MonitorTarget | null; onDone: () => void }) {
  const { t } = useTranslation();
  const [host, setHost] = useState(initial?.host || "");
  const [port, setPort] = useState(String(initial?.port || 443));
  const [days, setDays] = useState((initial?.notify_days?.length ? initial.notify_days : [30, 14, 7, 3, 1]).join(","));
  const [note, setNote] = useState(initial?.note || "");
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [err, setErr] = useState("");

  const submit = async () => {
    setErr("");
    try {
      const notifyDays = days.split(/[,，\s]+/).map((x) => Number(x.trim())).filter((x) => Number.isFinite(x) && x > 0);
      await invoke("monitor_upsert", {
        id: initial?.id,
        host,
        port: Number(port) || 443,
        note,
        enabled,
        notifyDays,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-3 gap-3">
        <label className="text-caption text-slate-400 col-span-2">
          {t("certmgr.monitorHost")}
          <input value={host} onChange={(e) => setHost(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="example.com 或 https://example.com:8443" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.monitorPort")}
          <input type="number" value={port} onChange={(e) => setPort(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.monitorDays")}
          <input value={days} onChange={(e) => setDays(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="30,14,7,3,1" />
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.remark")}
          <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      <label className="flex items-center gap-2 text-caption text-slate-400">
        <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
        {t("certmgr.monitorEnabled")}
      </label>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}
