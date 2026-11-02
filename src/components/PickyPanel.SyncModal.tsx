import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Loader2, Cloud } from "lucide-react";
import { fmtTime, type PickySyncConfig } from "./PickyPanel.model";
import { btnPrimary, btnSecondary, inputCls, labelCls } from "./shared/Button";
import { ModalShell } from "./PickyPanel.ModalShell";
// ─── 云同步设置 ───

export function SyncModal({ onClose, onDone }: { onClose: () => void; onDone: (msg: string) => void }) {
  const { t } = useTranslation();
  const [cfg, setCfg] = useState<PickySyncConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const inFlight = useRef(false);
  useEffect(() => {
    let disposed = false;
    invoke<PickySyncConfig>("picky_get_sync_config")
      .then((config) => { if (!disposed) setCfg(config); })
      .catch((e) => { if (!disposed) setMsg(t("picky.readCfgFail", { err: String(e) })); });
    return () => { disposed = true; };
  }, []);

  if (!cfg) {
    return (
      <ModalShell onClose={onClose} title={t("picky.syncModalTitle")}>
        <div role={msg ? "alert" : "status"} className="flex items-center gap-2 text-body text-slate-500">
          {msg ? msg : <><Loader2 className="w-4 h-4 animate-spin" /> {t("picky.loadingCfg")}</>}
        </div>
      </ModalShell>
    );
  }

  const set = (k: keyof PickySyncConfig, v: unknown) => setCfg((c) => (c ? { ...c, [k]: v } : c));

  const save = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setMsg("");
    try {
      await invoke("picky_save_sync_config", { config: cfg });
      setMsg(t("picky.cfgSaved"));
    } catch (e) {
      setMsg(t("picky.saveFail", { err: String(e) }));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  };

  // 一个「同步」按钮 = 内部双向合并：先拉取云端合并到本地，再上传合并结果。
  const syncNow = async () => {
    if (!cfg.enabled) {
      setMsg(t("picky.needEnableSync"));
      return;
    }
    if (inFlight.current) return;
    inFlight.current = true;
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
      inFlight.current = false;
      setBusy(false);
    }
  };

  const field = inputCls;

  return (
    <ModalShell onClose={onClose} title={t("picky.syncModalTitle2")} busy={busy}>
      <fieldset disabled={busy} className="space-y-3 min-w-0" aria-busy={busy}>
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
          <label className={labelCls}>{t("picky.endpointLabel")}</label>
          <input value={cfg.endpoint || ""} onChange={(e) => set("endpoint", e.target.value)} placeholder="https://s3.example.com" disabled={!cfg.enabled} className={field} />
        </div>
        <div className="grid grid-cols-2 gap-2">
          <div>
            <label className={labelCls}>Region</label>
            <input value={cfg.region} onChange={(e) => set("region", e.target.value)} disabled={!cfg.enabled} className={field} />
          </div>
          <div>
            <label className={labelCls}>Bucket</label>
            <input value={cfg.bucketName} onChange={(e) => set("bucketName", e.target.value)} disabled={!cfg.enabled} className={field} />
          </div>
        </div>
        <div>
          <label className={labelCls}>AccessKey ID</label>
          <input value={cfg.accessKeyId} onChange={(e) => set("accessKeyId", e.target.value)} disabled={!cfg.enabled} className={field} />
        </div>
        <div>
          <label className={labelCls}>{t("picky.secretKey")}</label>
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
            <label className={labelCls}>{t("picky.prefix")}</label>
            <input value={cfg.prefix || ""} onChange={(e) => set("prefix", e.target.value)} placeholder="picky/" disabled={!cfg.enabled} className={field} />
          </div>
          <div>
            <label className={labelCls}>{t("picky.addrStyle")}</label>
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

        {msg && <p role="status" className="text-caption text-[var(--module-accent)] break-words">{msg}</p>}

        <div className="flex justify-end gap-2 pt-1">
          <button
            onClick={save}
            disabled={busy}
            className={btnSecondary}
          >
            {t("picky.saveCfg")}
          </button>
          <button
            onClick={syncNow}
            disabled={!cfg.enabled || busy}
            className={btnPrimary}
          >
            {busy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Cloud className="w-3 h-3" />} {t("picky.doSync")}
          </button>
        </div>
      </fieldset>
    </ModalShell>
  );
}
