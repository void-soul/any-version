import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Brain, ListTree, Plus, StickyNote, X } from "lucide-react";
import { mmApi, type MindmapDocument, type DocumentFull, type MindmapNode } from "./types";

type Mode = "child" | "root" | "sticker";

const ACCENT = "#22d3ee";

export default function MindmapQuickPopup() {
  const [docs, setDocs] = useState<MindmapDocument[] | null>(null);
  const [docId, setDocId] = useState<string>("");
  const [full, setFull] = useState<DocumentFull | null>(null);
  const [loadingDoc, setLoadingDoc] = useState(false);
  const [mode, setMode] = useState<Mode>("child");
  const [parentId, setParentId] = useState<string>("");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [newDocName, setNewDocName] = useState("");

  const inputRef = useRef<HTMLTextAreaElement>(null);

  // 无边框窗口拖拽：标题栏 mousedown → startDragging
  const onTitleMouseDown = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button")) return;
    void getCurrentWindow().startDragging();
  }, []);

  // 加载文档列表
  useEffect(() => {
    void (async () => {
      try {
        await mmApi.init();
        const list = await mmApi.list();
        setDocs(list);
        if (list.length > 0) setDocId(list[0].id);
      } catch (e) {
        setError(String(e));
        setDocs([]);
      }
    })();
  }, []);

  // 选中文档 → 加载完整内容
  useEffect(() => {
    if (!docId) { setFull(null); return; }
    setLoadingDoc(true);
    void (async () => {
      try {
        const f = await mmApi.load(docId);
        setFull(f);
        // 默认挂到当前根节点下；没有根节点则作为根
        const roots = f?.nodes.filter((n) => !n.parentId) ?? [];
        setParentId(roots[0]?.id ?? "");
      } catch (e) {
        setError(String(e));
        setFull(null);
      } finally {
        setLoadingDoc(false);
      }
    })();
  }, [docId]);

  const roots = useMemo(() => full?.nodes.filter((n) => !n.parentId) ?? [], [full]);
  const childTargets = useMemo(() => {
    if (!full) return [];
    // 有序树列表（深度优先），带缩进标签
    const byId = new Map(full.nodes.map((n) => [n.id, n]));
    const out: { id: string; label: string }[] = [];
    const visited = new Set<string>();
    const visit = (n: MindmapNode, depth: number) => {
      if (visited.has(n.id)) return;
      visited.add(n.id);
      out.push({ id: n.id, label: `${"\u00a0\u00a0".repeat(depth)}${n.name}` });
      full.nodes.filter((c) => c.parentId === n.id && c.id !== n.id).forEach((c) => visit(c, depth + 1));
    };
    full.nodes.filter((n) => !n.parentId || !byId.has(n.parentId!)).forEach((n) => visit(n, 0));
    // 孤立/环兜底
    full.nodes.forEach((n) => { if (!visited.has(n.id)) visit(n, 0); });
    return out;
  }, [full]);

  const submit = useCallback(async () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      // 无文档：先自动创建
      let targetId = docId;
      if (!targetId) {
        const name = newDocName.trim() || `速记 ${new Date().toLocaleDateString("zh-CN")}`;
        const doc = await mmApi.create({ name, description: "", sourceType: "manual", folderId: null });
        targetId = doc.id;
        setDocId(doc.id);
      }
      const f = full && full.document.id === targetId ? full : await mmApi.load(targetId);
      if (!f) throw new Error("文档加载失败");
      const now = new Date().toISOString();
      if (mode === "sticker") {
        const id = `s${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
        const s = {
          id, documentId: targetId, content: text.trim(), imageData: "",
          color: "#fef3c7", positionX: 170 + (f.stickers.length % 5) * 30,
          positionY: 120 + (f.stickers.length % 5) * 24, createdAt: now, updatedAt: now,
        };
        await mmApi.upsertSticker({ documentId: targetId, sticker: s });
        setDone(`贴纸已记入「${f.document.name}」`);
      } else if (mode === "root") {
        const n: MindmapNode = {
          id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
          documentId: targetId, parentId: null, name: text.trim(), description: "", detail: "",
          kind: "root", color: "", progress: 0, planAt: null, positionX: 0, positionY: 0,
          createdAt: now, updatedAt: now,
        };
        await mmApi.upsertNode({ documentId: targetId, node: n });
        setDone(`新根节点已记入「${f.document.name}」`);
      } else {
        const pid = parentId || roots[0]?.id || null;
        if (!pid) {
          // 没有任何节点 → 作为根
          const n: MindmapNode = {
            id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
            documentId: targetId, parentId: null, name: text.trim(), description: "", detail: "",
            kind: "root", color: "", progress: 0, planAt: null, positionX: 0, positionY: 0,
            createdAt: now, updatedAt: now,
          };
          await mmApi.upsertNode({ documentId: targetId, node: n });
          setDone(`根节点已记入「${f.document.name}」`);
        } else {
          const n: MindmapNode = {
            id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
            documentId: targetId, parentId: pid, name: text.trim(), description: "", detail: "",
            kind: "other", color: "", progress: 0, planAt: null, positionX: 0, positionY: 0,
            createdAt: now, updatedAt: now,
          };
          await mmApi.upsertNode({ documentId: targetId, node: n });
          const pname = f.nodes.find((x) => x.id === pid)?.name ?? "";
          setDone(`已记入「${f.document.name}」→ ${pname}`);
        }
      }
      setText("");
      // 3 秒后自动隐藏
      window.setTimeout(() => { void hide(); }, 3000);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [text, busy, docId, newDocName, full, mode, parentId, roots]);

  const hide = useCallback(async () => {
    try { await invoke("hide_mindmap_quick_popup"); } catch { /* 窗口可能已关 */ }
  }, []);

  useEffect(() => { inputRef.current?.focus(); }, [full, docId]);

  // Esc dentro del popup: cerrar (ocultar) la ventana
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); void hide(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [hide]);

  return (
    <div className="h-screen w-screen overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl flex flex-col text-slate-200 select-none">
      {/* 标题栏（拖拽区） */}
      <div className="flex shrink-0 cursor-grab items-center gap-2 border-b border-white/10 px-3 py-2 active:cursor-grabbing" onMouseDown={onTitleMouseDown} style={{ backgroundColor: `${ACCENT}14` }}>
        <Brain className="h-4 w-4" style={{ color: ACCENT }} />
        <span className="text-xs font-semibold text-white">思维导图速记</span>
        <div className="flex-1" />
        <button className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => void hide()} title="关闭 (Esc)">
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto p-3">
        {/* 目标导图 */}
        <div>
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">导图</label>
          {docs && docs.length > 0 ? (
            <select value={docId} onChange={(e) => setDocId(e.target.value)} className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60">
              {docs.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
            </select>
          ) : (
            <div className="flex gap-1.5">
              <input value={newDocName} onChange={(e) => setNewDocName(e.target.value)} placeholder="新导图名称（可空=「速记 日期」）"
                className="h-8 min-w-0 flex-1 rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60" />
            </div>
          )}
        </div>

        {/* 记录方式 */}
        <div>
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">记录为</label>
          <div className="grid grid-cols-3 gap-1.5">
            <button onClick={() => setMode("child")} disabled={!!docs && docs.length === 0 && !newDocName}
              className={`flex items-center justify-center gap-1 rounded-md border px-2 py-1.5 text-[10px] font-semibold transition ${mode === "child" ? "border-cyan-400/60 bg-cyan-500/15 text-cyan-200" : "border-white/10 bg-white/5 text-slate-400 hover:text-white"}`}>
              <Plus className="h-3 w-3" />子节点
            </button>
            <button onClick={() => setMode("root")}
              className={`flex items-center justify-center gap-1 rounded-md border px-2 py-1.5 text-[10px] font-semibold transition ${mode === "root" ? "border-cyan-400/60 bg-cyan-500/15 text-cyan-200" : "border-white/10 bg-white/5 text-slate-400 hover:text-white"}`}>
              <ListTree className="h-3 w-3" />根节点
            </button>
            <button onClick={() => setMode("sticker")}
              className={`flex items-center justify-center gap-1 rounded-md border px-2 py-1.5 text-[10px] font-semibold transition ${mode === "sticker" ? "border-cyan-400/60 bg-cyan-500/15 text-cyan-200" : "border-white/10 bg-white/5 text-slate-400 hover:text-white"}`}>
              <StickyNote className="h-3 w-3" />贴纸
            </button>
          </div>
        </div>

        {/* 上级节点选择（仅子节点模式） */}
        {mode === "child" && (
          <div>
            <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">上级节点</label>
            {loadingDoc ? (
              <div className="text-[10px] text-slate-600">加载中…</div>
            ) : childTargets.length > 0 ? (
              <select value={parentId} onChange={(e) => setParentId(e.target.value)} className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60">
                {childTargets.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
              </select>
            ) : (
              <div className="text-[10px] text-slate-600">导图为空 — 将作为根节点记录</div>
            )}
          </div>
        )}

        {/* 内容 */}
        <div className="flex min-h-0 flex-1 flex-col">
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">内容</label>
          <textarea ref={inputRef} value={text} onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void submit(); }
            }}
            placeholder="记点什么… (Ctrl+Enter 记录)"
            className="min-h-[110px] flex-1 resize-none rounded-md border border-white/10 bg-slate-950/70 px-3 py-2 text-xs text-slate-200 outline-none focus:border-cyan-400/60" />
        </div>

        {error && <div className="rounded-md border border-red-500/20 bg-red-500/10 px-2.5 py-1.5 text-[10px] text-red-300">{error}</div>}
        {done && <div className="rounded-md border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-1.5 text-[10px] text-emerald-300">✓ {done}</div>}

        <button onClick={() => void submit()} disabled={busy || !text.trim()}
          className="shrink-0 rounded-lg py-2 text-xs font-semibold text-white disabled:opacity-40"
          style={{ backgroundColor: ACCENT }}>
          {busy ? "记录中…" : mode === "sticker" ? "记为贴纸" : mode === "root" ? "记为新根节点" : "记为子节点"}
        </button>
      </div>
    </div>
  );
}
