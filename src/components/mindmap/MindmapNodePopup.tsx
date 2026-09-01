import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { Brain, X } from "lucide-react";
import { mmApi, type MindmapDocument, type DocumentFull, type MindmapNode, kindColor } from "./types";
import { MarkdownFieldEditor } from "./MarkdownFieldEditor";
import { NodeFormFields } from "./NodeFormFields";
import VexAvatar from "../VexAvatar";
import { VEX_CYBER_ACCENT, resolveThemeAccent } from "../../utils/brand";

function normalizeHex(value: string): string | null {
  const raw = value?.trim() ?? "";
  if (/^#[0-9a-fA-F]{6}$/.test(raw)) return raw;
  return null;
}

/** 思维导图节点速记悬浮窗：复用内部表单（与 DetailModal 一致）。
 *  可编辑名称、描述、类型、进度、计划时间、颜色、详细内容（完整 Markdown 编辑器）。 */
export default function MindmapNodePopup() {
  const { t } = useTranslation();
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
  const titleRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  // 窗口高度随内容自动伸缩：测量「标题栏 + 各字段块自然高度」,
  // 调用 setSize 让悬浮窗贴合内容；超出屏幕可用高度后由内部滚动兜底。
  const fitWindow = useCallback(() => {
    try {
      const el = contentRef.current;
      if (!el) return;
      const titleH = titleRef.current?.offsetHeight ?? 36;
      let contentH = 0;
      const kids = Array.from(el.children) as HTMLElement[];
      for (const k of kids) contentH += k.getBoundingClientRect().height;
      const gaps = Math.max(0, kids.length - 1) * 8; // gap-2
      const pad = 20; // p-2.5 上下各 10px
      const maxH = Math.max(420, window.screen.availHeight - 40);
      const minH = Math.min(430, maxH);
      const desired = Math.round(Math.min(maxH, Math.max(minH, titleH + contentH + gaps + pad + 2)));
      if (Math.abs(desired - window.innerHeight) < 16) return;
      void getCurrentWindow().setSize(new LogicalSize(window.innerWidth, desired)).catch(() => {/* 忽略 */});
    } catch { /* 浏览器预览等无 Tauri 环境静默降级 */ }
  }, []);

  useEffect(() => {
    const el = contentRef.current;
    if (!el) return;
    let raf = 0;
    const schedule = () => { cancelAnimationFrame(raf); raf = requestAnimationFrame(fitWindow); };
    const ro = new ResizeObserver(schedule);
    ro.observe(el);
    for (const k of el.children) ro.observe(k);
    schedule();
    return () => { ro.disconnect(); cancelAnimationFrame(raf); };
  }, [fitWindow]);

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
        const docName = newDocName.trim() || t("mindmap.quickNote", { date: new Date().toLocaleDateString("zh-CN") });
        const doc = await mmApi.create({ name: docName, description: "", sourceType: "manual", folderId: null });
        targetId = doc.id;
        setDocId(doc.id);
      }
      const f = full && full.document.id === targetId ? full : await mmApi.load(targetId);
      if (!f) throw new Error(t("mindmap.docLoadFail"));
      const now = new Date().toISOString();
      const c = normalizeHex(color) ?? kindColor("other");
      const pid = parentId || roots[0]?.id || null;
      const n: MindmapNode = {
        id: `n${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
        documentId: targetId, parentId: pid, name: name.trim(), description: description.trim(), detail,
        kind: "other", color: c, progress, planAt: planAt.trim() || null, repeat,
        positionX: 0, positionY: 0, createdAt: now, updatedAt: now,
      };
      await mmApi.upsertNode({ documentId: targetId, node: n });
      const pname = pid ? f.nodes.find((x) => x.id === pid)?.name ?? "" : t("mmdpop.rootNodePh");
      setDone(t("mmdpop.recordedTo2", { name: f.document.name, parent: pname }));
      setName(""); setDescription(""); setDetail(""); setPlanAt("");
      window.setTimeout(() => { void hide(); }, 2500);
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }, [name, description, detail, busy, docId, newDocName, full, parentId, roots, progress, planAt, repeat, color, hide]);

  useEffect(() => { inputRef.current?.focus(); }, [full, docId]);

  return (
    <div className="h-screen w-screen overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl flex flex-col text-slate-200 select-none" style={themeVars}>
      {/* 标题栏 */}
      <div ref={titleRef} className="flex shrink-0 cursor-grab items-center gap-2 border-b border-white/10 px-3 py-2 active:cursor-grabbing" onMouseDown={onTitleMouseDown} style={{ backgroundColor: "var(--mm-accent-soft)" }}>
        <VexAvatar size={18} />
        <Brain className="h-4 w-4" style={{ color: "var(--mm-accent)" }} />
        <span className="text-xs font-semibold text-white">{t("mmdpop.nodeTitle")}</span>
        <div className="flex-1" />
        <button className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => void hide()} title={t("mmdpop.close")}>
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      <div ref={contentRef} className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2.5">
        {/* 目标导图 + 上级节点：同一行双列 */}
        <div className="grid grid-cols-2 gap-2">
          <div className="min-w-0">
            <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">{t("mmdpop.map")}</label>
            {docs && docs.length > 0 ? (
              <select value={docId} onChange={(e) => setDocId(e.target.value)} className="h-7 w-full truncate rounded-md border border-white/10 bg-slate-950/70 px-2 text-[11px] text-slate-200 outline-none focus:border-[var(--mm-accent)]">
                {docs.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
              </select>
            ) : (
              <input value={newDocName} onChange={(e) => setNewDocName(e.target.value)} placeholder={t("mmdpop.newDocPh2")}
                className="h-7 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-[11px] text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
            )}
          </div>
          <div className="min-w-0">
            <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">{t("mmdpop.parentNode")}</label>
            {loadingDoc ? (
              <div className="text-[10px] text-slate-600">{t("mmdpop.loading")}</div>
            ) : childTargets.length > 0 ? (
              <select value={parentId} onChange={(e) => setParentId(e.target.value)} className="h-7 w-full truncate rounded-md border border-white/10 bg-slate-950/70 px-2 text-[11px] text-slate-200 outline-none focus:border-[var(--mm-accent)]">
                {childTargets.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
              </select>
            ) : (
              <div className="text-[10px] text-slate-600">{t("mmdpop.emptyMapRoot")}</div>
            )}
          </div>
        </div>

        {/* 节点名 + 描述：同一行（名称 1 行，描述自动换行补足高度） */}
        <div className="grid grid-cols-[1.1fr_1fr] items-start gap-2">
          <div className="flex min-h-0 flex-col">
            <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">{t("mmdpop.content")}</label>
            <textarea ref={inputRef} value={name} onChange={(e) => setName(e.target.value)} rows={2}
              onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void submit(); } }}
              placeholder={t("mmdpop.contentPh")}
              className="min-h-[56px] resize-none rounded-md border border-white/10 bg-slate-950/70 px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
          </div>
          <div className="flex min-h-0 flex-col">
            <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">{t("mmdpop.desc")}</label>
            <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={2}
              placeholder={t("mmdpop.descPh2")}
              className="min-h-[56px] resize-none rounded-md border border-white/10 bg-slate-950/70 px-2.5 py-1.5 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
          </div>
        </div>

        <NodeFormFields ns="mmdpop"
          progress={progress} planAt={planAt} repeat={repeat} color={color}
          onProgress={setProgress}
          onPlanAt={(iso) => setPlanAt(iso ?? "")}
          onRepeat={setRepeat}
          onColor={setColor}
          showHexInput={false}
        />

        {/* 详细内容（完整 Markdown 编辑器） */}
        <div className="flex min-h-0 flex-col">
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">{t("mmdpop.detailMd")}</label>
          <div style={{ minHeight: 150 }}>
            <MarkdownFieldEditor value={detail} onChange={setDetail} minHeight="150px" />
          </div>
        </div>

        {error && <div className="rounded-md border border-red-500/20 bg-red-500/10 px-2.5 py-1.5 text-[10px] text-red-300">{error}</div>}
        {done && <div className="rounded-md border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-1.5 text-[10px] text-emerald-300">✓ {done}</div>}

        <button onClick={() => void submit()} disabled={busy || !name.trim()}
          className="shrink-0 rounded-lg py-1.5 text-xs font-semibold text-white disabled:opacity-40"
          style={{ backgroundColor: "var(--mm-accent)" }}>
          {busy ? t("mmdpop.recording") : t("mmdpop.recordNode")}
        </button>
      </div>
    </div>
  );
}
