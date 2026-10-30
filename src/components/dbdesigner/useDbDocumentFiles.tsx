import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { theamedConfirm } from "../shared/ThemedAlert";
import { type DbDesignDocument } from "./types";

/** Owns the DbDocumentFiles feature, independently of page rendering. */
interface AdoptDocumentOptions { path: string; dirty: boolean; resetDraft?: boolean; resetMemory?: boolean; }

export function useDbDocumentFiles({ doc, path, ensureValid, saveChatNow, loadChatForDoc, onAdoptDocument, onSavedDocument, setResult }: {
  doc: DbDesignDocument | null;
  path: string;
  ensureValid: () => Promise<boolean>;
  saveChatNow: () => Promise<void>;
  loadChatForDoc: (doc: DbDesignDocument, path: string) => Promise<void>;
  onAdoptDocument: (doc: DbDesignDocument, options: AdoptDocumentOptions) => void;
  onSavedDocument: (path: string) => void;
  setResult: (result: { ok: boolean; msg: string }) => void;
}) {
  const { t } = useTranslation();

  const newDoc = async () => {
    const d = await invoke<DbDesignDocument>("dbd_new_document", { name: t("dbd.untitled"), dialect: "mysql" });
    // 先把当前会话存住，再切走 —— 否则这一轮的对话就随组件状态一起丢了
    void saveChatNow();
    onAdoptDocument(d, { path: "", dirty: true, resetDraft: true });
    void loadChatForDoc(d, "");
  };

  const openFile = async () => {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: t("dbd.openTitle"),
      filters: [{ name: "dbdesign", extensions: ["json"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const d = await invoke<DbDesignDocument>("dbd_open_file", { path: picked });
      void saveChatNow();
      onAdoptDocument(d, { path: picked, dirty: false, resetDraft: true });
      void loadChatForDoc(d, picked);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  const saveFile = async (as: boolean) => {
    if (!doc) return;
    if (!(await ensureValid())) return;
    let target = path;
    if (!target || as) {
      const picked = await saveDialog({
        title: t("dbd.saveTitle"),
        defaultPath: `${doc.name}.dbdesign.json`,
        filters: [{ name: "dbdesign", extensions: ["json"] }],
      });
      if (typeof picked !== "string") return;
      target = picked.endsWith(".dbdesign.json") ? picked : picked.replace(/\.json$/i, "") + ".dbdesign.json";
    }
    try {
      await invoke("dbd_save_file", { path: target, doc });
      onSavedDocument(target);
      setResult({ ok: true, msg: t("dbd.saved", { path: target }) });
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  // ── 反向工程 ──
  /** 用反推结果替换当前文档（有未保存内容时先问一句，别把人刚画的东西冲掉） */
  const adoptReversed = async (next: DbDesignDocument, from: string) => {
    if (doc && doc.nodes.length > 0) {
      const ok = await theamedConfirm(t("dbd.replaceConfirm", { from }), {
        title: t("dbd.replaceTitle"),
        confirmText: t("dbd.replaceBtn"),
        danger: true,
      });
      if (!ok) return;
    }
    onAdoptDocument(next, { path: "", dirty: true, resetMemory: true });
    setResult({ ok: true, msg: t("dbd.reversed", { count: next.nodes.length, from }) });
  };

  const reverseSqlite = async () => {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: t("dbd.pickSqlite"),
      filters: [{ name: "sqlite", extensions: ["db", "sqlite", "sqlite3", "db3"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const d = await invoke<DbDesignDocument>("dbd_reverse_sqlite", { path: picked });
      await adoptReversed(d, picked);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  const reverseDdl = async () => {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: t("dbd.pickDdl"),
      filters: [{ name: "sql", extensions: ["sql", "dump", "txt"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const d = await invoke<DbDesignDocument>("dbd_reverse_ddl_file", {
        path: picked,
        dialect: doc?.dialect ?? "mysql",
      });
      await adoptReversed(d, picked);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };
  return {
    newDoc,
    openFile,
    saveFile,
    reverseSqlite,
    reverseDdl,
  };
}
