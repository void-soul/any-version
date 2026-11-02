import { useCallback, useEffect, useRef, useState } from "react";
import type { Dispatch, SetStateAction } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { theamedConfirm } from "../shared/ThemedAlert";
import type { Tab } from "./MarkdownReader.model";
import { recordMarkdownSave, trimMarkdownHistory } from "./MarkdownReader.state";
export function useMarkdownEditing(active: Tab | null, activePath: string | null, setTabs: Dispatch<SetStateAction<Tab[]>>, flashNotice: (message: string) => void) {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);
  const saveTimer = useRef<number | null>(null);
  const lastUndoAt = useRef(new Map<string, number>());
  const inFlight = useRef<string | null>(null);
  const alive = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; if (saveTimer.current !== null) window.clearTimeout(saveTimer.current); }; }, []);
  /** 编辑器内容变更（带撤销栈；连续输入按 800ms 合并成一次撤销） */
  const setContent = useCallback((path: string, next: string, pushUndo = true) => {
    const now = Date.now();
    const last = lastUndoAt.current.get(path) ?? 0;
    lastUndoAt.current.set(path, now);
    const mergeTyping = now - last < 800;
    setTabs((prev) =>
      prev.map((t) => {
        if (t.path !== path) return t;
        // 只有小幅增删（逐字输入）才并入上一次撤销点，批量替换仍单独入栈
        const smallEdit = Math.abs(next.length - t.content.length) <= 2;
        const undo = pushUndo && !(mergeTyping && smallEdit) ? trimMarkdownHistory([...t.undo, t.content]) : t.undo;
        return { ...t, content: next, undo, redo: [], dirty: next !== t.savedContent };
      })
    );
  }, []);

  /** 撤销/重做 */
  const undo = useCallback(() => {
    lastUndoAt.current.delete(activePath ?? "");
    setTabs((prev) =>
      prev.map((t) => {
        if (t.path !== activePath || t.undo.length === 0) return t;
        const prevContent = t.undo[t.undo.length - 1];
        return {
          ...t,
          content: prevContent,
          undo: t.undo.slice(0, -1),
          redo: trimMarkdownHistory([...t.redo, t.content]),
          dirty: prevContent !== t.savedContent,
        };
      })
    );
  }, [activePath]);

  const redo = useCallback(() => {
    lastUndoAt.current.delete(activePath ?? "");
    setTabs((prev) =>
      prev.map((t) => {
        if (t.path !== activePath || t.redo.length === 0) return t;
        const nextContent = t.redo[t.redo.length - 1];
        return {
          ...t,
          content: nextContent,
          undo: trimMarkdownHistory([...t.undo, t.content]),
          redo: t.redo.slice(0, -1),
          dirty: nextContent !== t.savedContent,
        };
      })
    );
  }, [activePath]);

  /** 保存到磁盘 */
  const saveActive = useCallback(async () => {
    if (!active || !active.dirty || inFlight.current) return;
    inFlight.current = active.path;
    setSaving(true);
    try {
      await invoke("write_text_file", { path: active.path, content: active.content });
      setTabs((prev) =>
        recordMarkdownSave(prev, active.path, active.content)
      );
      flashNotice(t("mdreader.saved"));
    } catch (e) {
      flashNotice(t("mdreader.saveFail", { err: String(e) }));
    } finally {
      inFlight.current = null;
      setSaving(false);
    }
  }, [active, saving, flashNotice]);

  // 自动保存（2s 防抖）
  useEffect(() => {
    if (!active?.dirty) return;
    if (saveTimer.current) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => { void saveActive(); }, 2000);
    return () => { if (saveTimer.current) window.clearTimeout(saveTimer.current); };
  }, [active?.content, active?.dirty, saveActive, active]);

  const reloadActive = useCallback(async () => {
    if (!active || inFlight.current || active.loading) return;
    if (active.dirty && !(await theamedConfirm(t("mdreader.reloadDirtyConfirm")))) return;
    if (inFlight.current || !alive.current) return;
    const path = active.path;
    inFlight.current = path;
    setSaving(true);
    setTabs((prev) =>
      prev.map((t) => (t.path === path ? { ...t, loading: true, error: null } : t))
    );
    try {
      const content = await invoke<string>("read_text_file", { path });
      setTabs((prev) =>
        prev.map((t) => (t.path === path ? { ...t, content, savedContent: content, loading: false, dirty: false, undo: [], redo: [] } : t))
      );
    } catch (e) {
      setTabs((prev) =>
        prev.map((t) => (t.path === path ? { ...t, error: String(e), loading: false } : t))
      );
    } finally { inFlight.current = null; setSaving(false); }
  }, [active]);

  return { setContent, undo, redo, saveActive, reloadActive, saving, isSaving: (path: string) => inFlight.current === path, forgetHistory: (path: string) => { lastUndoAt.current.delete(path); } };
}
