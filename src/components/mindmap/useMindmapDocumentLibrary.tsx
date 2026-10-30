import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import "@xyflow/react/dist/style.css";
import { Folder, Trash2, Pencil, ChevronDown, ChevronRight } from "lucide-react";
import { DocumentFull, MindmapDocument, MindmapFolder, mmApi } from "./types";
import { MM_LAST_DOC_KEY, buildChildren, getFolderPath, isFolderDescendant, type FolderNode } from "./MindmapPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the MindmapDocumentLibrary feature, independently of page rendering. */
export function useMindmapDocumentLibrary({ setFull, flash }: {
  setFull: Dispatch<SetStateAction<DocumentFull | null>>;
  flash: (m: string) => void;
}) {
  const { t } = useTranslation();

  const [docs, setDocs] = useState<MindmapDocument[]>([]);

  const [folders, setFolders] = useState<MindmapFolder[]>([]);

  const [activeFolderId, setActiveFolderId] = useState<string | null>(null);

  const [dragOverFolderId, setDragOverFolderId] = useState<string | null>(null);

  const [collapsedFolders, setCollapsedFolders] = useState<Set<string>>(new Set());

  // 跟随鼠标的拖拽浮层状态（自定义 pointer 拖拽；绕开 HTML5 DnD 在 WebView2 起拖/光标问题）
  const [ghost, setGhost] = useState<{ kind: "doc" | "folder"; name: string; x: number; y: number; ok: boolean } | null>(null);

  // 拖拽会话：按下即记录候选，移动超过阈值才确认为拖动
  const dragHandleRef = useRef<{ kind: "doc" | "folder"; id: string; name: string; startX: number; startY: number; active: boolean } | null>(null);

  const [showCreate, setShowCreate] = useState(false);

  const [showFolderCreate, setShowFolderCreate] = useState(false);

  const [editingFolder, setEditingFolder] = useState<MindmapFolder | null>(null);

  const [folderName, setFolderName] = useState("");

  const [renamingDocId, setRenamingDocId] = useState<string | null>(null);

  const [renameDocName, setRenameDocName] = useState("");
 // 当前 AI 导入运行的取消标识
  const [search, setSearch] = useState("");

  const [confirmState, setConfirmState] = useState<{ title: string; message: string; action: () => void } | null>(null);

  const refreshFolders = useCallback(async () => {
    try { setFolders(await mmApi.listFolders()); } catch {}
  }, []);

  const createDoc = useCallback(async (name: string, desc: string, folderId: string | null) => {
    try {
      const doc = await mmApi.create({ name, description: desc, sourceType: "manual", folderId });
      setDocs(prev => [doc, ...prev]);
      if (!folderId) setActiveFolderId(null);
      setShowCreate(false);
      const f = await mmApi.load(doc.id);
      if (f) setFull(f);
      void refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [flash, refreshFolders]);

  const removeDoc = useCallback((id: string, name: string) => {
    setConfirmState({ title: t("mindmap.delMapTitle"), message: t("mindmap.delMapMsg", { name }), action: () => void executeRemoveDoc(id) });
  }, []);

  const executeRemoveDoc = useCallback(async (id: string) => {
    setConfirmState(null);
    try {
      await mmApi.remove(id);
      setDocs(prev => prev.filter(d => d.id !== id));
      // 右侧若正打开被删文档 → 同步关闭（函数式更新，不依赖闭包 full，杜绝闭包过期导致漏关）
      setFull(prev => {
        if (prev?.document.id === id) {
          try { localStorage.removeItem(MM_LAST_DOC_KEY); } catch { /* 忽略 */ }
          return null;
        }
        return prev;
      });
      refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [flash, refreshFolders]);

  const createFolder = useCallback(async () => {
    if (!folderName.trim()) return;
    try {
      await mmApi.createFolder({ name: folderName.trim(), parentId: activeFolderId });
      setShowFolderCreate(false); setFolderName("");
      await refreshFolders();
      setCollapsedFolders((prev) => {
        if (!activeFolderId) return prev;
        const next = new Set(prev);
        next.delete(activeFolderId); // 展开父目录，让新文件夹可见
        return next;
      });
    } catch (e) { flash(String(e)); }
  }, [folderName, activeFolderId, flash, refreshFolders]);

  const updateFolder = useCallback(async () => {
    if (!editingFolder || !folderName.trim()) return;
    try {
      await mmApi.updateFolder({ id: editingFolder.id, name: folderName.trim() });
      setEditingFolder(null); setFolderName("");
      await refreshFolders();
    } catch (e) { flash(String(e)); }
  }, [editingFolder, folderName, flash, refreshFolders]);

  const loadDocs = useCallback(async () => {
    try { setDocs(await mmApi.list(activeFolderId)); } catch {}
  }, [activeFolderId]);

  const deleteFolder = useCallback((id: string, name: string) => {
    setConfirmState({ title: t("mindmap.delFolderTitle"), message: t("mindmap.delFolderMsg", { name }), action: () => void executeDeleteFolder(id) });
  }, []);

  const executeDeleteFolder = useCallback(async (id: string) => {
    setConfirmState(null);
    try { await mmApi.deleteFolder(id); if (activeFolderId === id) setActiveFolderId(null); await refreshFolders(); await loadDocs(); } catch (e) { flash(String(e)); }
  }, [activeFolderId, flash, refreshFolders, loadDocs]);

  const startRenameDoc = useCallback((d: MindmapDocument) => { setRenamingDocId(d.id); setRenameDocName(d.name); }, []);

  const saveRenameDoc = useCallback(async (d: MindmapDocument) => {
    setRenamingDocId(null);
    const name = renameDocName.trim();
    if (!name || name === d.name) return;
    try {
      await mmApi.update({ id: d.id, name });
      setDocs(prev => prev.map(x => x.id === d.id ? { ...x, name } : x));
      flash(t("mindmap.renamed"));
    } catch (e) { flash(String(e)); }
  }, [renameDocName, flash]);

  const moveDoc = useCallback(async (docId: string, fid: string | null) => {
    try {
      await mmApi.moveDocument({ documentId: docId, folderId: fid });
      await Promise.all([loadDocs(), refreshFolders()]);
    } catch (e) { flash(String(e)); }
  }, [flash, loadDocs]);

  const moveFolder = useCallback(async (fid: string, toParent: string | null) => {
    try {
      await mmApi.moveFolder({ folderId: fid, parentId: toParent });
      await Promise.all([loadDocs(), refreshFolders()]);
    } catch (e) { flash(String(e)); }
  }, [flash, loadDocs]);

  // ── 自定义拖拽（pointer events，绕开 HTML5 DnD）──
  // mousedown 只记录候选；窗口级 mousemove 移动超 5px 激活拖拽并绘制浮层、
  // 用 elementFromPoint 命中检测落点（文件夹行 / 根落点）；mouseup 执行移动。
  // 落点判定：文件夹不允许落入自身或自身后代（后端亦有防环兜底）。
  useEffect(() => {
    const onMove = (ev: MouseEvent) => {
      const d = dragHandleRef.current;
      if (!d) return;
      if (!d.active) {
        if (Math.hypot(ev.clientX - d.startX, ev.clientY - d.startY) < 5) return;
        d.active = true;
      }
      ev.preventDefault();
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      const hitFolder = el?.closest?.("[data-drop-folder]") as HTMLElement | null;
      const hitRoot = el?.closest?.("[data-drop-root]") as HTMLElement | null;
      let ok = false;
      let target: string | null = null;
      if (hitFolder) {
        const tid = hitFolder.getAttribute("data-drop-folder")!;
        if (d.kind === "folder" && (tid === d.id || isFolderDescendant(folders, d.id, tid))) {
          ok = false;
        } else {
          target = tid; ok = true;
        }
      } else if (hitRoot) {
        ok = true;
      }
      setDragOverFolderId(ok ? (target ?? "__root") : null);
      setGhost({ kind: d.kind, name: d.name, x: ev.clientX, y: ev.clientY, ok });
    };
    const onUp = (ev: MouseEvent) => {
      const d = dragHandleRef.current;
      if (!d) return;
      dragHandleRef.current = null;
      if (!d.active) return;
      ev.preventDefault();
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      const hitFolder = el?.closest?.("[data-drop-folder]") as HTMLElement | null;
      const hitRoot = el?.closest?.("[data-drop-root]") as HTMLElement | null;
      let target: string | null = null;
      let moved = false;
      if (hitFolder) {
        const tid = hitFolder.getAttribute("data-drop-folder")!;
        if (d.kind === "folder" && (tid === d.id || isFolderDescendant(folders, d.id, tid))) {
          moved = false; // 文件夹不能落入自身/自身后代
        } else {
          target = tid; moved = true;
        }
      } else if (hitRoot) {
        moved = true; // target 保持 null = 根目录
      }
      if (moved) {
        if (d.kind === "doc") void moveDoc(d.id, target);
        else void moveFolder(d.id, target);
      }
      setDragOverFolderId(null);
      setGhost(null);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => { window.removeEventListener("mousemove", onMove); window.removeEventListener("mouseup", onUp); };
  }, [folders, moveDoc, moveFolder]);

  // 按下时记录拖拽候选（阻止文本选择与滚动起手，保证拖动语义稳定）；
  // 普通点击不受影响（click 事件仍正常派发）。
  const startDragHandle = useCallback((e: React.MouseEvent, kind: "doc" | "folder", id: string, name: string) => {
    if (e.button !== 0) return;
    e.preventDefault();
    dragHandleRef.current = { kind, id, name, startX: e.clientX, startY: e.clientY, active: false };
  }, []);

  useEffect(() => { void loadDocs(); }, [activeFolderId, loadDocs]);

  const toggleFolderCollapse = useCallback((id: string) => {
    setCollapsedFolders((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  // 递归渲染文件夹树：每个文件夹既是拖源（可整理到任意目录/根），也是落点（接收文档与文件夹）
  const renderFolderNodes = (nodes: FolderNode[], depth: number): React.ReactNode => {
    return nodes.map((node) => {
      const f = node.folder;
      const over = dragOverFolderId === f.id;
      const open = !collapsedFolders.has(f.id);
      const hasChildren = node.children.length > 0;
      return (
        <div key={f.id}>
<div
            className={`group mb-0.5 flex items-center gap-1 rounded-md py-1.5 pr-1 transition select-none ${over ? "ring-1 ring-cyan-400/70 bg-cyan-400/10" : "hover:bg-white/[0.05]"}`}
            style={{ paddingLeft: 6 + depth * 14 }}
            data-drop-folder={f.id}
            title={depth > 0 ? t("mindmap.dragHintRoot") : t("mindmap.dragHint")}
            onMouseDown={(e) => startDragHandle(e, "folder", f.id, f.name)}
          >
            {hasChildren ? (
              <button type="button" className="flex h-4 w-4 shrink-0 items-center justify-center rounded text-slate-500 hover:text-white cursor-pointer"
                onClick={(e) => { e.stopPropagation(); toggleFolderCollapse(f.id); }} title={open ? t("mindmap.collapse") : t("mindmap.expand")}>
                {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
              </button>
            ) : (
              <span className="w-4 shrink-0" />
            )}
            <button type="button" className="flex min-w-0 flex-1 items-center gap-1.5 text-left cursor-pointer" onClick={() => setActiveFolderId(f.id)} title={t("mindmap.enterFolder", { name: f.name })}>
              <Folder className={`h-3.5 w-3.5 shrink-0 ${depth === 0 ? "text-amber-400" : "text-amber-400/60"}`} />
              <span className="truncate text-tiny text-slate-300">{f.name}</span>
              <span className="shrink-0 text-micro text-slate-400">{f.documentCount}</span>
            </button>
            <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-slate-300 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-white/10 hover:text-white"
              onClick={() => { setEditingFolder(f); setFolderName(f.name); }} title={t("mindmap.rename")}><Pencil className="h-3 w-3" /></button>
            <button type="button" className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-red-400/50 opacity-0 group-hover:opacity-100 pointer-events-none group-hover:pointer-events-auto hover:bg-red-400/10 hover:text-red-300"
              onClick={() => deleteFolder(f.id, f.name)} title={t("mindmap.deleteFolder")}><Trash2 className="h-3 w-3" /></button>
          </div>
          {open && renderFolderNodes(node.children, depth + 1)}
        </div>
      );
    });
  };

  // Filtered documents
  const filteredDocs = useMemo(() => {
    if (!search.trim()) return docs;
    const q = search.toLowerCase();
    return docs.filter(d => d.name.toLowerCase().includes(q) || (d.description && d.description.toLowerCase().includes(q)));
  }, [docs, search]);

  // 当前视图的文件夹树：根视图 = 顶层目录；进入目录 = 该目录直接子目录
  const treeRoots = activeFolderId ? buildChildren(folders, activeFolderId) : buildChildren(folders, null);

  const folderPath = getFolderPath(folders, activeFolderId);
  return {
    docs,
    setDocs,
    folders,
    setFolders,
    activeFolderId,
    setActiveFolderId,
    dragOverFolderId,
    ghost,
    showCreate,
    setShowCreate,
    showFolderCreate,
    setShowFolderCreate,
    editingFolder,
    setEditingFolder,
    folderName,
    setFolderName,
    renamingDocId,
    setRenamingDocId,
    renameDocName,
    setRenameDocName,
    search,
    setSearch,
    confirmState,
    setConfirmState,
    refreshFolders,
    createDoc,
    removeDoc,
    createFolder,
    updateFolder,
    startRenameDoc,
    saveRenameDoc,
    moveDoc,
    startDragHandle,
    renderFolderNodes,
    filteredDocs,
    treeRoots,
    folderPath,
  };
}
