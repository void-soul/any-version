import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SharedModal } from "./shared/Modal";
import { SharedButton } from "./shared/Button";
export function ConfirmModal({ title, message, danger, onConfirm, onClose }: {
  title: string; message: string; danger?: boolean; onConfirm: () => void | Promise<void>; onClose: () => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const confirm = async () => {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true); setError("");
    try { await onConfirm(); onClose(); }
    catch (error) { setError(String(error)); }
    finally { inFlight.current = false; setBusy(false); }
  };
  return <SharedModal open closeDisabled={busy} title={title} width={400} onClose={() => { if (!busy) onClose(); }}>
    <p className="text-body text-slate-300 leading-relaxed">{message}</p>
    {error && <p role="alert" className="ui-note ui-note-error px-3 py-2 break-words">{error}</p>}
    <div className="flex justify-end gap-2" aria-busy={busy}>
      <SharedButton type="button" disabled={busy} onClick={onClose}>{t("otp.cancel")}</SharedButton>
      <SharedButton type="button" variant={danger ? "danger" : "primary"} disabled={busy} onClick={() => void confirm()}>
        {busy ? t("common.loading") : t("otp.delete")}
      </SharedButton>
    </div>
  </SharedModal>;
}
