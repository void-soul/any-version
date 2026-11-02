import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SharedModal } from "../shared/Modal";
import { SharedButton, inputCls, labelCls } from "../shared/Button";

export function CreateDocModal({ onClose, onCreate, folderId }: {
  onClose: () => void;
  onCreate: (name: string, desc: string, folderId: string | null) => Promise<void>;
  folderId: string | null;
}) {
  const { t } = useTranslation();
  const id = useId();
  const [name, setName] = useState("");
  const [desc, setDesc] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const submit = async () => {
    if (!name.trim() || inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError("");
    try { await onCreate(name.trim(), desc, folderId); }
    catch (error) { setError(String(error)); }
    finally { inFlight.current = false; setBusy(false); }
  };
  return <SharedModal open onClose={() => { if (!busy) onClose(); }} title={t("mindmap.newMapTitle")} width={420}>
    <form aria-busy={busy} className="space-y-3" onSubmit={(event) => { event.preventDefault(); void submit(); }}>
      <div>
        <label className={labelCls} htmlFor={`${id}-name`}>{t("mindmap.nameLabel")}</label>
        <input id={`${id}-name`} autoFocus required disabled={busy} className={inputCls} value={name}
          onChange={(event) => setName(event.target.value)} placeholder={t("mindmap.mapNamePh")} />
      </div>
      <div>
        <label className={labelCls} htmlFor={`${id}-description`}>{t("mindmap.descLabel")}</label>
        <textarea id={`${id}-description`} disabled={busy} className="ui-input w-full min-h-20 px-2.5 py-2 resize-y"
          value={desc} onChange={(event) => setDesc(event.target.value)} placeholder={t("mindmap.descPh")} />
      </div>
      {error && <p role="alert" className="ui-note ui-note-error text-body px-3 py-2 break-words">{error}</p>}
      <div className="flex justify-end gap-2">
        <SharedButton type="button" disabled={busy} onClick={onClose}>{t("mindmap.cancel")}</SharedButton>
        <SharedButton type="submit" variant="primary" disabled={busy || !name.trim()}>{busy ? t("common.loading") : t("mindmap.create")}</SharedButton>
      </div>
    </form>
  </SharedModal>;
}
