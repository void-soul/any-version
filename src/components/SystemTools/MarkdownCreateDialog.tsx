import { useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { SharedModal } from "../shared/Modal";
import { btnPrimary, btnSecondary, inputCls, labelCls } from "../shared/Button";
export function MarkdownCreateDialog({ directory, onClose, onCreated }: { directory: string; onClose: () => void; onCreated: (path: string) => Promise<void> }) {
  const { t } = useTranslation();
  const id = useId();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [createdPath, setCreatedPath] = useState<string | null>(null);
  const inFlight = useRef(false);
  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim() || inFlight.current) return;
    inFlight.current = true; setBusy(true); setError("");
    try {
      const path = createdPath ?? await invoke<string>("create_markdown_file", { directory, name: name.trim() });
      setCreatedPath(path);
      await onCreated(path);
      onClose();
    } catch (error) { setError(String(error)); }
    finally { inFlight.current = false; setBusy(false); }
  };
  return <SharedModal open title={t("mdreader.newFile")} closeDisabled={busy} onClose={onClose}>
    <form onSubmit={(event) => void submit(event)} className="space-y-3" aria-busy={busy}>
      <p className="text-caption text-slate-500 break-all">{directory}</p>
      <label htmlFor={id} className={labelCls}>{t("mdreader.fileName")}</label>
      <input id={id} autoFocus disabled={busy || createdPath !== null} className={inputCls} value={name} onChange={(event) => setName(event.target.value)} placeholder="notes.md" />
      <p className="text-caption text-slate-500">{t("mdreader.newFileHint")}</p>
      {error && <p role="alert" className="ui-note-error">{error}</p>}
      <div className="flex justify-end gap-2">
        <button type="button" disabled={busy} className={btnSecondary} onClick={onClose}>{t("common.cancel")}</button>
        <button type="submit" disabled={busy || !name.trim()} className={btnPrimary}>{busy ? t("common.loading") : t("common.create")}</button>
      </div>
    </form>
  </SharedModal>;
}
