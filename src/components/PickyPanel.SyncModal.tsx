import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Loader2, Cloud } from "lucide-react";
import { fmtTime, type PickySyncConfig } from "./PickyPanel.model";
import { ModalShell } from "./PickyPanel.ModalShell";
// ─── 云同步设置 ───

export function SyncModal({ onClose, onDone }: { onClose: () => void; onDone: (msg: string) => void }) {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<PickySyncConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  useEffect(() => {
    invoke<PickySyncConfig>("picky_get_sync_config")
      .then(setCfg)
      .catch((e) => setMsg(t("picky.readCfgFail", { err: String(e) })));
  }, []);

  if (!cfg) {
    return (
      <ModalShell onClose={onClose} title={t("picky.syncModalTitle")}>
        <div className="flex items-center gap-2 text-body text-slate-500">
          <Loader2 className="w-4 h-4 animate-spin" /> {t("picky.loadingCfg")}
        </div>
      </ModalShell>
    );
  }

  const set = (k: keyof PickySyncConfig, v: unknown) => setCfg((c) => (c ? { ...c, [k]: v } : c));

  const save = async () => {
    if (!cfg.enabled) {
      setMsg(t("picky.needEnableSave"));
      return;
    }
    setBusy(true);
    setMsg("");
    try {
      await invoke("picky_save_sync_config", { config: cfg });
      setMsg(t("picky.cfgSaved"));
    } catch (e) {
      setMsg(t("picky.saveFail", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  // 一个「同步」按钮 = 内部双向合并：先拉取云端合并到本地，再上传合并结果。
  const syncNow = async () => {
    if (!cfg.enabled) {
      setMsg(t("picky.needEnableSync"));
      return;
    }
    setBusy(true);
    setMsg("");
    try {
      await invoke("picky_save_sync_config", { config: cfg });
      const res = await invoke<string>("picky_sync_now");
      setMsg(res);
      onDone(res);
    } catch (e) {
      setMsg(t("picky.syncFail", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const field = "w-full glass-input px-3 py-2 text-body bg-black/30 border border-white/10 rounded-ctl focus:outline-none focus:border-[var(--module-accent)]/50";

  return (
    <ModalShell onClose={onClose} title={t("picky.syncModalTitle2")}>
      <div className="space-y-3">
        <label className="flex items-center gap-2 text-caption text-slate-300 cursor-pointer">
          <input
            type="checkbox"
            checked={cfg.enabled}
            onChange={(e) => set("enabled", e.target.checked)}
            className="accent-[var(--module-accent)]"
          />
          {t("picky.enableSync")}
        </label>
        {!cfg.enabled && (
          <p className="text-tiny text-amber-400/90 bg-amber-400/10 border border-amber-400/20 rounded-ctl px-3 py-2">
            {t("picky.syncDisabledWarn")}
          </p>
        )}
        {cfg.lastSyncAt && <p className="text-tiny text-slate-500">{t("picky.lastSync", { time: fmtTime(cfg.lastSyncAt) })}</p>}

        <div>
          <label className="text-tiny text-slate-400 mb-1 block">{t("picky.endpointLabel")}</label>
          <input value={cfg.endpoint || ""} onChange={(e) => set("endpoint", e.target.value)} placeholder="https://s3.example.com" disabled={!cfg.enabled} className={field} />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">Region</label>
            <input value={cfg.region} onChange={(e) => set("region", e.target.value)} disabled={!cfg.enabled} className={field} />
          </div>
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">Bucket</label>
            <input value={cfg.bucketName} onChange={(e) => set("bucketName", e.target.value)} disabled={!cfg.enabled} className={field} />
          </div>
        </div>
        <div>
          <label className="text-tiny text-slate-400 mb-1 block">AccessKey ID</label>
          <input value={cfg.accessKeyId} onChange={(e) => set("accessKeyId", e.target.value)} disabled={!cfg.enabled} className={field} />
        </div>
        <div>
          <label className="text-tiny text-slate-400 mb-1 block">{t("picky.secretKey")}</label>
          <input
            type="password"
            value={cfg.secretAccessKey}
            onChange={(e) => set("secretAccessKey", e.target.value)}
            disabled={!cfg.enabled}
            className={field}
          />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">{t("picky.prefix")}</label>
            <input value={cfg.prefix || ""} onChange={(e) => set("prefix", e.target.value)} placeholder="picky/" disabled={!cfg.enabled} className={field} />
          </div>
          <div>
            <label className="text-tiny text-slate-400 mb-1 block">{t("picky.addrStyle")}</label>
            <select value={cfg.addressingStyle} onChange={(e) => set("addressingStyle", e.target.value)} disabled={!cfg.enabled} className={field}>
              <option value="auto">{t("picky.styleAuto")}</option>
              <option value="path">{t("picky.stylePath")}</option>
              <option value="virtual-host">{t("picky.styleVirtualHost")}</option>
            </select>
          </div>
        </div>
        <label className="flex items-center gap-2 text-caption text-slate-300 cursor-pointer">
          <input
            type="checkbox"
            checked={cfg.tlsVerify}
            onChange={(e) => set("tlsVerify", e.target.checked)}
            disabled={!cfg.enabled}
            className="accent-[var(--module-accent)]"
          />
          {t("picky.verifyTls")}
        </label>

        <p className="text-tiny text-slate-600 leading-relaxed">
          {t("picky.syncDesc")}
        </p>

        {msg && <p className="text-caption text-[var(--module-accent)] break-words">{msg}</p>}

        <div className="flex justify-end gap-2 pt-1">
          <button
            onClick={save}
            disabled={!cfg.enabled || busy}
            className="px-3 py-1.5 rounded-ctl text-caption ui-btn text-slate-300 cursor-pointer disabled:opacity-50"
          >
            {t("picky.saveCfg")}
          </button>
          <button
            onClick={syncNow}
            disabled={!cfg.enabled || busy}
            className="px-4 py-1.5 rounded-ctl text-caption bg-[var(--module-accent)] text-white font-semibold cursor-pointer hover:opacity-85 disabled:opacity-50 flex items-center gap-1"
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Cloud className="w-3 h-3" />} {t("picky.doSync")}
          </button>
        </div>
      </div>
    </ModalShell>
  );
}
