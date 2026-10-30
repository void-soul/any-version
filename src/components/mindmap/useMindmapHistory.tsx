import type { MindmapDocument } from "./types";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { DocumentFull, mmApi } from "./types";
import type { Dispatch, SetStateAction } from "react";

/** Owns the MindmapHistory feature, independently of page rendering. */
export function useMindmapHistory({ full, setFull, setDocs, setError }: {
  full: DocumentFull | null;
  setFull: Dispatch<SetStateAction<DocumentFull | null>>;
  setDocs: Dispatch<SetStateAction<MindmapDocument[]>>;
  setError: Dispatch<SetStateAction<string>>;
}) {
  const { t } = useTranslation();

  // ── 撤销/重做（Ctrl+Z / Ctrl+Y）──
  // 快照式历史：每次离散操作前把当前 DocumentFull 压入撤销栈；撤销/重做时恢复快照并写回后端。
  const undoStack = useRef<DocumentFull[]>([]);

  const redoStack = useRef<DocumentFull[]>([]);

  const currentRef = useRef<DocumentFull | null>(null);

  const [historyVersion, setHistoryVersion] = useState(0);

  useEffect(() => { currentRef.current = full; }, [full]);

  const commitHistory = useCallback(() => {
    const cur = currentRef.current;
    if (cur) {
      undoStack.current.push(cur);
      if (undoStack.current.length > 50) undoStack.current.shift();
      redoStack.current = [];
    }
  }, []);

  // 恢复快照：本地 state 立即回退，并把节点/贴纸差异写回后端（持久化）
  const restoreSnapshot = useCallback((snap: DocumentFull) => {
    const cur = currentRef.current;
    setFull(snap);
    setDocs(ds => ds.map(x => x.id === snap.document.id ? snap.document : x));
    setHistoryVersion(v => v + 1);
    if (!cur) return;
    void (async () => {
      // 逐条写回，任一失败就停止并提示（不再静默吞错，避免画布显示已恢复但未持久化）
      try {
        const snapIds = new Set(snap.nodes.map(n => n.id));
        for (const n of cur.nodes) if (!snapIds.has(n.id)) { await mmApi.deleteNode({ documentId: snap.document.id, nodeId: n.id }); }
        for (const n of snap.nodes) { await mmApi.upsertNode({ documentId: snap.document.id, node: n }); }
        const snapStickers = new Set(snap.stickers.map(s => s.id));
        for (const s of cur.stickers) if (!snapStickers.has(s.id)) { await mmApi.deleteSticker({ documentId: snap.document.id, stickerId: s.id }); }
        for (const s of snap.stickers) { await mmApi.upsertSticker({ documentId: snap.document.id, sticker: s }); }
        // 自由关系线同样按快照对齐（同一对节点只允许一条，upsert 幂等）
        const snapLinks = new Set((snap.links ?? []).map(l => l.id));
        for (const l of (cur.links ?? [])) if (!snapLinks.has(l.id)) { await mmApi.deleteLink({ documentId: snap.document.id, linkId: l.id }); }
        for (const l of (snap.links ?? [])) { await mmApi.upsertLink({ documentId: snap.document.id, link: l }); }
      } catch (e) {
        // 本地已恢复，但持久化失败：明确告知用户，避免数据丢失后无从排查
        console.error("[mindmap] 撤销/重做持久化失败:", e);
        setError(t("mindmap.undoRedoFail", { err: String(e) }));
      }
    })();
  }, []);

  const undo = useCallback(() => {
    const prev = undoStack.current.pop();
    if (!prev) return;
    const cur = currentRef.current;
    if (cur) redoStack.current.push(cur);
    restoreSnapshot(prev);
  }, [restoreSnapshot]);

  const redo = useCallback(() => {
    const next = redoStack.current.pop();
    if (!next) return;
    const cur = currentRef.current;
    if (cur) undoStack.current.push(cur);
    restoreSnapshot(next);
  }, [restoreSnapshot]);

  // 全局 Ctrl+Z / Ctrl+Y（输入框内不拦截，保留浏览器原生文本撤销）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) return;
      const k = e.key.toLowerCase();
      if (k === "z") { e.preventDefault(); if (e.shiftKey) redo(); else undo(); }
      else if (k === "y") { e.preventDefault(); redo(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [undo, redo]);
  return {
    historyVersion,
    commitHistory,
  };
}
