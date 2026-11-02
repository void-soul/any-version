import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SharedModal } from "./shared/Modal";
import { btnDanger, btnSecondary } from "./shared/Button";
export function PickyConfirmModal({ title, message, onConfirm, onClose }: { title: string; message: string; onConfirm: () => void | Promise<void>; onClose: () => void }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const confirm = async () => {
    if (inFlight.current) return;
    inFlight.current = true; setBusy(true); setError("");
    try { await onConfirm(); onClose(); }
    catch (e) { setError(String(e)); }
    finally { inFlight.current = false; setBusy(false); }
  };
  return <SharedModal open title={title} width={400} closeDisabled={busy} onClose={onClose}>
    <p className="text-body text-slate-300">{message}</p>
    {error && <p role="alert" className="ui-note-error">{error}</p>}
    <div className="flex justify-end gap-2" aria-busy={busy}>
      <button type="button" disabled={busy} className={btnSecondary} onClick={onClose}>{t("picky.cancel")}</button>
      <button type="button" disabled={busy} className={btnDanger} onClick={() => void confirm()}>{busy ? t("common.loading") : t("picky.confirm")}</button>
    </div>
  </SharedModal>;
}
