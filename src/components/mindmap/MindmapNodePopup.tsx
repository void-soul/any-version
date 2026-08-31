import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Brain, X } from "lucide-react";
import { mmApi, type MindmapDocument, type DocumentFull, type MindmapNode, kindColor } from "./types";
import { PlanDateTimePicker, KIND_LABELS } from "./MindmapPanel";
import { MarkdownFieldEditor } from "./MarkdownFieldEditor";
import VexAvatar from "../VexAvatar";
import VexGreeting from "../VexGreeting";
import { VEX_CYBER_ACCENT, resolveThemeAccent } from "../../utils/brand";

const KINDS = ["root", "module", "task", "requirement", "constraint", "risk", "component", "service", "route", "config", "file", "other"];
const COLORS = ["#f8fafc", "#22d3ee", "#34d399", "#fbbf24", "#60a5fa", "#fb7185", "#a78bfa", "#f97316", "#f59e0b", "#94a3b8"];

function normalizeHex(value: string): string | null {
  const raw = value?.trim() ?? "";
  if (/^#[0-9a-fA-F]{6}$/.test(raw)) return raw;
  return null;
}

/** 思维导图节点速记悬浮窗：复用内部表单（与 DetailModal 一致）。
 *  可编辑名称、描述、类型、进度、计划时间、颜色、详细内容（完整 Markdown 编辑器）。 */
export default function MindmapNodePopup() {
  const [accent, setAccent] = useState(VEX_CYBER_ACCENT);
  useEffect(() => {
    (async () => {
      try {
        const ap = await invoke<{ moduleThemeColors?: Record<string, string> }>("get_appearance_config");
        setAccent(resolveThemeAccent(ap.moduleThemeColors));
      } catch { /* 加载失败用默认色即可 */ }
    })();
  }, []);

  const themeVars = {
    "--mm-accent": accent,
    "--mm-accent-soft": `color-mix(in srgb, ${accent} 12%, transparent)`,
    "--mm-accent-ring": `color-mix(in srgb, ${accent} 30%, transparent)`,
    "--mm-accent-strong": `color-mix(in srgb, ${accent} 85%, white)`,
  } as React.CSSProperties;

  const [docs, setDocs] = useState<MindmapDocument[] | null>(null);
  const [docId, setDocId] = useState<string>("");
  const [full, setFull] = useState<DocumentFull | null>(null);
  const [loadingDoc, setLoadingDoc] = useState(false);
  const [parentId, setParentId] = useState<string>("");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [kind, setKind] = useState("other");
  const [progress, setProgress] = useState(0);
  const [planAt, setPlanAt] = useState("");
  const [repeat, setRepeat] = useState("none");
  const [color, setColor] = useState("");
  const [detail, setDetail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [newDocName, setNewDocName] = useState("");

  const inputRef = useRef<HTMLTextAreaElement>(null);

  const onTitleMouseDown = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button")) return;
    void getCurrentWindow().startDragging();
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        await mmApi.init();
        const list = await mmApi.list();
        setDocs(list);
        if (list.length > 0) setDocId(list[0].id);
      } catch (e) { setError(String(e)); setDocs([]); }
    })();
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        const sel = await invoke<string | null>("take_mindmap_quick_selection");
        if (sel) setDescription((cur) => (cur.trim() ? cur : sel));
      } catch { /* 无捕获或后端未支持时保持为空 */ }
    })();
  }, []);

  useEffect(() => {
    if (!docId) { setFull(null); return; }
    setLoadingDoc(true);
    void (async () => {
      try {
        const f = await mmApi.load(docId);
        setFull(f);
        const roots = f?.nodes.filter((n) => !n.parentId) ?? [];
        setParentId(roots[0]?.id ?? "");
      } catch (e) { setError(String(e)); setFull(null); }
      finally { setLoadingDoc(false); }
    })();
  }, [docId]);

  const childTargets = useMemo(() => {
    if (!full) return [];
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
    full.nodes.forEach((n) => { if (!visited.has(n.id)) visit(n, 0); });
    return out;
  }, [full]);

  const roots = useMemo(() => full?.nodes.filter((n) => !n.parentId) ?? [], [full]);

  const hide = useCallback(async () => {
    try { await invoke("hide_mindmap_quick_popup"); } catch { /* 窗口可能已关 */ }
  }, []);

  const submit = useCallback(async () => {
    if (!name.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      let targetId = docId;
      if (!targetId) {
        const docName = newDocName.trim() || `速记 ${new Date().toLocaleDateString("zh-CN")}`;
        const doc = await mmApi.create({ name: docName, description: "", sourceType: "manual", folderId: null });
        targetId = doc.id;
        setDocId(doc.id);
      }
      const f = full && full.document.id === targetId ? full : await mmApi.load(targetId);
      if (!f) throw new Error("文档加载失败");
      const now = new Date().toISOString();
      const c = normalizeHex(color) ?? kindColor(kind);
      const pid = parentId || roots[0]?.id || null;
      const n: MindmapNode = {
        id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        documentId: targetId, parentId: pid, name: name.trim(), description: description.trim(), detail,
        kind: pid ? kind : "root", color: c, progress, planAt: planAt.trim() || null, repeat,
        positionX: 0, positionY: 0, createdAt: now, updatedAt: now,
      };
      await mmApi.upsertNode({ documentId: targetId, node: n });
      const pname = pid ? f.nodes.find((x) => x.id === pid)?.name ?? "" : "（根节点）";
      setDone(`已记入「${f.document.name}」→ ${pname}`);
      setName(""); setDescription(""); setDetail(""); setPlanAt("");
      window.setTimeout(() => { void hide(); }, 2500);
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }, [name, description, detail, busy, docId, newDocName, full, parentId, roots, kind, progress, planAt, repeat, color, hide]);

  useEffect(() => { inputRef.current?.focus(); }, [full, docId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { e.preventDefault(); void hide(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [hide]);

  return (
    <div className="h-screen w-screen overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl flex flex-col text-slate-200 select-none" style={themeVars}>
      {/* 标题栏 */}
      <div className="flex shrink-0 cursor-grab items-center gap-2 border-b border-white/10 px-3 py-2 active:cursor-grabbing" onMouseDown={onTitleMouseDown} style={{ backgroundColor: "var(--mm-accent-soft)" }}>
        <VexAvatar size={18} />
        <Brain className="h-4 w-4" style={{ color: "var(--mm-accent)" }} />
        <span className="text-xs font-semibold text-white">思维导图节点速记</span>
        <div className="flex-1" />
        <button className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => void hide()} title="关闭 (Esc)">
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* 提示 */}
      <div className="flex shrink-0 items-center gap-1.5 border-b border-white/5 bg-white/[0.02] px-3 py-1.5">
        <span className="text-[9px] italic leading-snug text-slate-400">
          💬<VexGreeting seconds={10} />
        </span>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto p-3">
        {/* 目标导图 */}
        <div>
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">导图</label>
          {docs && docs.length > 0 ? (
            <select value={docId} onChange={(e) => setDocId(e.target.value)} className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]">
              {docs.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
            </select>
          ) : (
            <input value={newDocName} onChange={(e) => setNewDocName(e.target.value)} placeholder="新导图名称（可空=「速记 日期」）"
              className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
          )}
        </div>

        {/* 上级节点 */}
        <div>
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">上级节点</label>
          {loadingDoc ? (
            <div className="text-[10px] text-slate-600">加载中…</div>
          ) : childTargets.length > 0 ? (
            <select value={parentId} onChange={(e) => setParentId(e.target.value)} className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]">
              {childTargets.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
            </select>
          ) : (
            <div className="text-[10px] text-slate-600">导图为空 — 将作为根节点记录</div>
          )}
        </div>

        {/* 节点名 */}
        <div className="flex min-h-0 flex-col">
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">内容（节点名）</label>
          <textarea ref={inputRef} value={name} onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void submit(); } }}
            placeholder="节点的标题… (Ctrl+Enter 记录)"
            className="min-h-[40px] resize-none rounded-md border border-white/10 bg-slate-950/70 px-3 py-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
        </div>

        {/* 描述 */}
        <div className="flex min-h-0 flex-col">
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">描述</label>
          <textarea value={description} onChange={(e) => setDescription(e.target.value)}
            placeholder="补充说明…"
            className="min-h-[40px] resize-none rounded-md border border-white/10 bg-slate-950/70 px-3 py-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
        </div>

        {/* 类型 + 进度 */}
        <div className="grid grid-cols-[1.2fr_0.8fr] gap-1.5">
          <div>
            <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">类型</label>
            <select value={kind} onChange={(e) => setKind(e.target.value)}
              className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]">
              {KINDS.map((k) => <option key={k} value={k}>{KIND_LABELS[k] ?? k}</option>)}
            </select>
          </div>
          <div>
            <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">进度 {progress}%</label>
            <input type="range" min={0} max={100} value={progress} onChange={(e) => setProgress(Number(e.target.value))}
              className="h-8 w-full cursor-pointer accent-[var(--mm-accent)]" />
          </div>
        </div>

        {/* 计划时间 + 重复 */}
        <div>
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">计划时间（可空）</label>
          <div className="flex items-center gap-1.5">
            <PlanDateTimePicker value={planAt} onChange={(iso) => setPlanAt(iso ?? "")} />
            <select value={repeat} onChange={(e) => setRepeat(e.target.value)}
              className="h-8 flex-1 rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]">
              <option value="none">不重复</option>
              <option value="daily">每天</option>
              <option value="weekly">每周</option>
            </select>
          </div>
        </div>

        {/* 颜色 */}
        <div>
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">颜色</label>
          <div className="flex flex-wrap items-center gap-1.5">
            {COLORS.map((cl) => <button key={cl} type="button" className="h-5 w-5 rounded-full border border-white/20"
              style={{ backgroundColor: cl, boxShadow: color === cl ? `0 0 6px ${cl}` : "none" }}
              onClick={() => setColor(cl)} />)}
            <label className="relative inline-flex h-5 w-5 cursor-pointer items-center justify-center overflow-hidden rounded-full border border-white/30" title="自定义颜色">
              <input type="color" value={normalizeHex(color) ?? "#22d3ee"} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
                onChange={(e) => setColor(e.target.value)} />
              <span className="h-3 w-3 rounded-full" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
            </label>
            <button type="button" className="rounded border border-white/15 px-1.5 py-0.5 text-[9px] text-slate-400 hover:text-white"
              onClick={() => setColor("")} title="按类型自动配色">自动</button>
          </div>
        </div>

        {/* 详细内容（完整 Markdown 编辑器） */}
        <div className="flex min-h-0 flex-col">
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">详细内容 (Markdown)</label>
          <div style={{ minHeight: 200 }}>
            <MarkdownFieldEditor value={detail} onChange={setDetail} minHeight="180px" />
          </div>
        </div>

        {error && <div className="rounded-md border border-red-500/20 bg-red-500/10 px-2.5 py-1.5 text-[10px] text-red-300">{error}</div>}
        {done && <div className="rounded-md border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-1.5 text-[10px] text-emerald-300">✓ {done}</div>}

        <button onClick={() => void submit()} disabled={busy || !name.trim()}
          className="shrink-0 rounded-lg py-2 text-xs font-semibold text-white disabled:opacity-40"
          style={{ backgroundColor: "var(--mm-accent)" }}>
          {busy ? "记录中…" : "记录节点"}
        </button>
      </div>
    </div>
  );
}
