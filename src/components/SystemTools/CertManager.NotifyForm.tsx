import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { NOTIFY_TYPES, type NotifyChannel } from "./CertManager.model";
export function NotifyForm({ initial, onDone }: { initial: NotifyChannel | null; onDone: () => void }) {
  const { t } = useTranslation();
  const [type, setType] = useState(initial?.type || "dingtalk");
  const [name, setName] = useState(initial?.name || "");
  const [url, setUrl] = useState(initial?.config?.url || "");
  const [secret, setSecret] = useState(initial?.config?.secret || "");
  const [token, setToken] = useState(initial?.config?.token || "");
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [note, setNote] = useState(initial?.note || "");
  const [err, setErr] = useState("");

  const submit = async () => {
    setErr("");
    try {
      const config: Record<string, string> = { url };
      if (type === "dingtalk" && secret) config.secret = secret;
      if (type === "webhook" && token) config.token = token;
      await invoke("notify_channel_upsert", {
        id: initial?.id,
        channelType: type,
        name,
        config,
        enabled,
        note,
      });
      onDone();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="rounded-ctl border border-white/10 bg-white/5 p-4 space-y-3">
      <div className="grid grid-cols-2 gap-3">
        <label className="text-caption text-slate-400">
          {t("certmgr.notifyType")}
          <select value={type} onChange={(e) => setType(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none">
            {NOTIFY_TYPES.map((o) => (
              <option key={o.value} value={o.value} className="bg-slate-800 text-slate-200">{o.label}</option>
            ))}
          </select>
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.name")}
          <input value={name} onChange={(e) => setName(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      <label className="text-caption text-slate-400 block">
        {t("certmgr.notifyUrl")}
        <input value={url} onChange={(e) => setUrl(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="https://..." />
      </label>
      {type === "dingtalk" && (
        <label className="text-caption text-slate-400 block">
          {t("certmgr.notifySecret")}
          <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" placeholder="SEC..." />
        </label>
      )}
      {type === "webhook" && (
        <label className="text-caption text-slate-400 block">
          {t("certmgr.notifyToken")}
          <input type="password" value={token} onChange={(e) => setToken(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      )}
      <div className="grid grid-cols-2 gap-3">
        <label className="flex items-center gap-2 text-caption text-slate-400">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} />
          {t("certmgr.monitorEnabled")}
        </label>
        <label className="text-caption text-slate-400">
          {t("certmgr.remark")}
          <input value={note} onChange={(e) => setNote(e.target.value)} className="mt-1 w-full bg-black/30 rounded px-2 py-1.5 text-slate-200 outline-none" />
        </label>
      </div>
      {err && <div className="text-caption text-rose-400">{err}</div>}
      <div className="flex gap-2">
        <button onClick={submit} className="px-3 py-1.5 rounded-ctl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-caption font-semibold">{t("certmgr.save")}</button>
        <button onClick={onDone} className="px-3 py-1.5 rounded-ctl bg-white/10 text-slate-300 text-caption">{t("certmgr.cancel")}</button>
      </div>
    </div>
  );
}
