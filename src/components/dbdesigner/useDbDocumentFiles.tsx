import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { theamedConfirm } from "../shared/ThemedAlert";
import type { DbDesignDocument } from "./types";

interface AdoptDocumentOptions { path: string; dirty: boolean; resetDraft?: boolean; resetMemory?: boolean; }
export function useDbDocumentFiles({ doc, path, dirty, ensureValid, saveChatNow, loadChatForDoc, onAdoptDocument, onSavedDocument, setResult }: {
  doc: DbDesignDocument | null; path: string; dirty: boolean;
  ensureValid: () => Promise<boolean>; saveChatNow: () => Promise<void>;
  loadChatForDoc: (doc: DbDesignDocument, path: string) => Promise<void>;
  onAdoptDocument: (doc: DbDesignDocument, options: AdoptDocumentOptions) => void;
  onSavedDocument: (path: string, snapshot: DbDesignDocument) => void;
  setResult: (result: { ok: boolean; msg: string }) => void;
}) {
  const { t } = useTranslation();
  const current = useRef({ doc, path, dirty });
  current.current = { doc, path, dirty };
  const gate = useRef(false);
  const [fileBusy, setFileBusy] = useState(false);
  const run = async (action: () => Promise<void>) => {
    if (gate.current) return;
    gate.current = true;
    setFileBusy(true);
    try { await action(); }
    catch (error) { setResult({ ok: false, msg: String(error) }); }
    finally { gate.current = false; setFileBusy(false); }
  };
  const replace = async (next: DbDesignDocument, target: string, from: string, reversed = false) => {
    const source = current.current.doc;
    if (current.current.dirty || (reversed && source && source.nodes.length > 0)) {
      if (!await theamedConfirm(t("dbd.replaceConfirm", { from }), {
        title: t("dbd.replaceTitle"), confirmText: t("dbd.replaceBtn"), danger: true,
      })) return;
    }
    if (current.current.doc !== source) return;
    await saveChatNow();
    if (current.current.doc !== source) return;
    onAdoptDocument(next, { path: target, dirty: !target, resetDraft: true, resetMemory: reversed });
    await loadChatForDoc(next, target);
    if (reversed) setResult({ ok: true, msg: t("dbd.reversed", { count: next.nodes.length, from }) });
  };
  const newDoc = () => run(async () => {
    const next = await invoke<DbDesignDocument>("dbd_new_document", { name: t("dbd.untitled"), dialect: "mysql" });
    await replace(next, "", t("dbd.untitled"));
  });
  const openFile = () => run(async () => {
    const picked = await openDialog({ multiple: false, directory: false, title: t("dbd.openTitle"),
      filters: [{ name: "dbdesign", extensions: ["json"] }] });
    if (typeof picked !== "string") return;
    const next = await invoke<DbDesignDocument>("dbd_open_file", { path: picked });
    await replace(next, picked, picked);
  });
  const saveFile = (as: boolean) => run(async () => {
    const snapshot = doc;
    if (!snapshot || !await ensureValid()) return;
    let target = path;
    if (!target || as) {
      const picked = await saveDialog({ title: t("dbd.saveTitle"), defaultPath: `${snapshot.name}.dbdesign.json`,
        filters: [{ name: "dbdesign", extensions: ["json"] }] });
      if (typeof picked !== "string") return;
      target = /\.dbdesign\.json$/i.test(picked) ? picked : picked.replace(/\.json$/i, "") + ".dbdesign.json";
    }
    await invoke("dbd_save_file", { path: target, doc: snapshot });
    onSavedDocument(target, snapshot);
    setResult({ ok: true, msg: t("dbd.saved", { path: target }) });
  });
  const reverseSqlite = () => run(async () => {
    const picked = await openDialog({ multiple: false, directory: false, title: t("dbd.pickSqlite"),
      filters: [{ name: "sqlite", extensions: ["db", "sqlite", "sqlite3", "db3"] }] });
    if (typeof picked !== "string") return;
    const next = await invoke<DbDesignDocument>("dbd_reverse_sqlite", { path: picked });
    await replace(next, "", picked, true);
  });
  const reverseDdl = () => run(async () => {
    const picked = await openDialog({ multiple: false, directory: false, title: t("dbd.pickDdl"),
      filters: [{ name: "sql", extensions: ["sql", "dump", "txt"] }] });
    if (typeof picked !== "string") return;
    const next = await invoke<DbDesignDocument>("dbd_reverse_ddl_file", { path: picked, dialect: doc?.dialect ?? "mysql" });
    await replace(next, "", picked, true);
  });
  return { newDoc, openFile, saveFile, reverseSqlite, reverseDdl, fileBusy };
}
