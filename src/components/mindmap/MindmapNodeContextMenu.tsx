import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import "@xyflow/react/dist/style.css";
import { Folder, Sparkles, Trash2, Plus, FileText, ListTree } from "lucide-react";
import { DocumentFull, MindmapNode } from "./types";
import { openSourceFile } from "./agentShared";
import type { Dispatch, SetStateAction } from "react";

interface MindmapNodeContextMenuProps {
  ctxMenu: { x: number; y: number; nodeId: string; };
  byId: Map<string, MindmapNode>;
  setDetailNode: Dispatch<SetStateAction<MindmapNode | null>>;
  setCtxMenu: Dispatch<SetStateAction<{ x: number; y: number; nodeId: string; } | null>>;
  addChildNode: (parentIdOverride?: string | undefined) => void;
  makeRoot: (nodeId: string) => void;
  full: DocumentFull;
  deleteNode: (nodeId: string) => void;
}

/** Pure MindmapNodeContextMenu view; its owning panel retains side effects and state. */
export function MindmapNodeContextMenu({ ctxMenu, byId, setDetailNode, setCtxMenu, addChildNode, makeRoot, full, deleteNode }: MindmapNodeContextMenuProps) {
  const { t } = useTranslation();
  return (
    <div className="fixed z-50 min-w-[160px] rounded-ctl border border-white/10 bg-surface-modal py-1 shadow-2xl" style={{ left: ctxMenu.x, top: ctxMenu.y }} onClick={e => e.stopPropagation()} onMouseDown={e => e.stopPropagation()}>
      <div className="border-b border-white/10 px-3 py-1.5 text-tiny font-semibold text-slate-400">{byId.get(ctxMenu.nodeId)?.name ?? ctxMenu.nodeId}</div>
      <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-caption text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
        onClick={() => { const n = byId.get(ctxMenu.nodeId); if (n) setDetailNode(n); setCtxMenu(null); }}><Sparkles className="h-3.5 w-3.5" />{t("mindmap.viewDetail")}</button>
      <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-caption text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
        onClick={() => { addChildNode(ctxMenu.nodeId); setCtxMenu(null); }}><Plus className="h-3.5 w-3.5" />{t("mindmap.addChild")}</button>
      <button type="button" disabled={byId.get(ctxMenu.nodeId)?.parentId === null} className="flex w-full items-center gap-2 px-3 py-2 text-caption text-slate-300 transition hover:bg-white/[0.08] hover:text-white disabled:opacity-40"
        onClick={() => { makeRoot(ctxMenu.nodeId); setCtxMenu(null); }}><ListTree className="h-3.5 w-3.5" />{t("mindmap.makeRoot")}</button>
      {(byId.get(ctxMenu.nodeId)?.sources?.length ?? 0) > 0 && (() => {
        const n = byId.get(ctxMenu.nodeId)!;
        const root = full.document.sourceDesc || "";
        const srcs = n.sources ?? [];
        return (<>
          <div className="border-t border-white/10" />
          {srcs.length === 1 && root && (() => { const p = `${root.replace(/\\/g, "/")}/${srcs[0]}`; return (
            <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-caption text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
              onClick={() => { void invoke("launcher_reveal_file", { path: p }).catch((e) => console.error("定位文件失败:", p, e)); setCtxMenu(null); }}><Folder className="h-3.5 w-3.5" />{t("mindmap.openFolder")}</button>
          ); })()}
          {srcs.map((s, i) => (
            <button key={i} type="button" className="flex w-full items-center gap-2 px-3 py-2 text-caption text-slate-300 transition hover:bg-white/[0.08] hover:text-white"
              onClick={() => {
                if (root) openSourceFile(root, s);
                setCtxMenu(null);
              }} title={s}><FileText className="h-3.5 w-3.5" />{t("mindmap.openFile", { name: s.length > 20 ? s.slice(0, 18) + "…" : s })}</button>
          ))}
        </>);
      })()}
      <div className="border-t border-white/10" />
      <button type="button" className="flex w-full items-center gap-2 px-3 py-2 text-caption text-red-300 transition hover:bg-white/[0.08] hover:text-red-100"
        onClick={() => { deleteNode(ctxMenu.nodeId); setCtxMenu(null); }}><Trash2 className="h-3.5 w-3.5" />{t("mindmap.delete")}</button>
    </div>
  );
}
