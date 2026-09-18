import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { StickyNote, X } from "lucide-react";
import { mmApi, type MindmapDocument, type DocumentFull, type MindmapSticker } from "./types";
import { usePopupSize } from "./usePopupSize";
import VexGlowAvatar from "../VexGlowAvatar";
import VexGreeting from "../VexGreeting";
import { VEX_CYBER_ACCENT, resolveThemeAccent } from "../../utils/brand";

const STICKER_PALETTE = ["#fef3c7", "#d4f5d4", "#dbeafe", "#fce7f3", "#e9d5ff", "#fef9c3", "#ccfbf1", "#ffe4e6"];

/** 思维导图贴纸悬浮窗：必须先选择目标文档，再输入贴纸内容并记录。
 *  不再支持选择类型（节点/贴纸切换），此窗口只处理贴纸。 */
export default function MindmapStickerPopup() {
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
  const [content, setContent] = useState("");
  const [color, setColor] = useState(STICKER_PALETTE[0]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [newDocName, setNewDocName] = useState("");

  const inputRef = useRef<HTMLTextAreaElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  // 高度管理：默认自动贴合内容（贴纸表单内容少，不再停在过高的初始窗口）；
  // 底部把手可手动拖拽调整高度（双击恢复自动）。
  const { manualH, gripHandlers } = usePopupSize({ kind: "sticker", rootRef, contentRef, minH: 260 });

  const onTitleMouseDown = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button")) return;
    // 拖动失败（如窗口缺权限）时至少在控制台留痕，避免无提示失效难排查。
    void getCurrentWindow().startDragging().catch((err) => console.warn("startDragging failed:", err));
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

  // 呼出时把后台捕获的选中文本默认填入贴纸内容。
  // 窗口是复用的（hide/show 不重挂载），挂载时的一次性拉取只覆盖首次创建——
  // 因此监听「tauri://focus」：每次热键呼出/聚焦窗口都重新拉取
  // （take 拉取即清空后端槽位，无新选区时返回 null，不误覆盖用户输入）。
  useEffect(() => {
    let un: (() => void) | undefined;
    const pull = async () => {
      try {
        const sel = await invoke<string | null>("take_mindmap_quick_selection");
        if (sel) setContent((cur) => (cur.trim() ? cur : sel));
      } catch { /* 无捕获或后端未支持时保持为空 */ }
    };
    void pull();
    try {
      void getCurrentWindow().listen("tauri://focus", () => { void pull(); }).then((u) => { un = u; });
    } catch { /* 浏览器预览无 Tauri 窗口事件 */ }
    return () => { un?.(); };
  }, []);

  useEffect(() => {
    if (!docId) { setFull(null); return; }
    void (async () => {
      try {
        const f = await mmApi.load(docId);
        setFull(f);
      } catch (e) { setError(String(e)); setFull(null); }
    })();
  }, [docId]);

  const hide = useCallback(async () => {
    try { await invoke("hide_mindmap_sticker_popup"); } catch { /* 窗口可能已关 */ }
  }, []);

  const submit = useCallback(async () => {
    // 必须选择目标文档
    if (!content.trim() || busy) return;
    if (!docId && !newDocName.trim()) {
      setError(t("mmdpop.needDoc"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      let targetId = docId;
      if (!targetId) {
        const doc = await mmApi.create({ name: newDocName.trim(), description: "", sourceType: "manual", folderId: null });
        targetId = doc.id;
        setDocId(doc.id);
      }
      const f = full && full.document.id === targetId ? full : await mmApi.load(targetId);
      if (!f) throw new Error(t("mmdpop.docLoadFail"));
      const now = new Date().toISOString();
      const id = `s${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
      const s: MindmapSticker = {
        id, documentId: targetId, content: content.trim(), imageData: "",
        color, positionX: 170 + (f.stickers.length % 5) * 30,
        positionY: 120 + (f.stickers.length % 5) * 24, createdAt: now, updatedAt: now,
      };
      await mmApi.upsertSticker({ documentId: targetId, sticker: s });
      setDone(t("mmdpop.recordedTo", { name: f.document.name }));
      setContent("");
      window.setTimeout(() => { void hide(); }, 2500);
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }, [content, busy, docId, newDocName, full, color, hide]);

  useEffect(() => { inputRef.current?.focus(); }, [full, docId]);

  return (
    <div ref={rootRef} className="h-screen w-screen overflow-hidden rounded-xl border border-white/10 bg-[#0d1524] shadow-2xl flex flex-col text-slate-200 select-none" style={themeVars}>
      {/* 标题栏 */}
      <div className="flex shrink-0 cursor-grab items-center gap-2 border-b border-white/10 px-3 py-2 active:cursor-grabbing" onMouseDown={onTitleMouseDown} style={{ backgroundColor: "var(--mm-accent-soft)" }}>
        <VexGlowAvatar size={18} />
        <StickyNote className="h-4 w-4" style={{ color: "var(--mm-accent)" }} />
        <span className="text-xs font-semibold text-white">{t("mmdpop.stickerTitle")}</span>
        <div className="flex-1" />
        <button className="rounded p-1 text-slate-400 hover:bg-white/10 hover:text-white" onClick={() => void hide()} title={t("mmdpop.close")}>
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* 提示 */}
      <div className="flex shrink-0 items-center gap-1.5 border-b border-white/5 bg-white/[0.02] px-3 py-1.5">
        <span className="text-[9px] italic leading-snug text-slate-400">
          💬<VexGreeting seconds={10} />
        </span>
      </div>

      <div ref={contentRef} className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto p-3">
        {/* 目标导图（必须选择） */}
        <div>
          <label className="mb-1 flex items-center justify-between text-[9px] uppercase font-semibold text-slate-500">
            <span>{t("mmdpop.targetDoc")}</span>
            <span className="font-normal normal-case text-amber-400/70">{t("mmdpop.required")}</span>
          </label>
          {docs && docs.length > 0 ? (
            <select value={docId} onChange={(e) => setDocId(e.target.value)} className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]">
              {docs.map((d) => <option key={d.id} value={d.id}>{d.name}</option>)}
            </select>
          ) : docs && docs.length === 0 ? (
            <input value={newDocName} onChange={(e) => setNewDocName(e.target.value)} placeholder={t("mmdpop.newDocPh")}
              className="h-8 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
          ) : (
            <div className="text-[10px] text-slate-600">{t("mmdpop.loading")}</div>
          )}
        </div>

        {/* 贴纸内容 */}
        <div className="flex min-h-0 flex-1 flex-col">
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">{t("mmdpop.stickerContent")}</label>
          <textarea ref={inputRef} value={content} onChange={(e) => setContent(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); void submit(); } }}
            placeholder={t("mmdpop.stickerPh")}
            className="min-h-[100px] flex-1 resize-none rounded-md border border-white/10 bg-slate-950/70 px-3 py-2 text-xs text-slate-200 outline-none focus:border-[var(--mm-accent)]" />
        </div>

        {/* 贴纸颜色 */}
        <div>
          <label className="mb-1 block text-[9px] uppercase font-semibold text-slate-500">{t("mmdpop.color")}</label>
          <div className="flex flex-wrap items-center gap-1.5">
            {STICKER_PALETTE.map((cl) => <button key={cl} type="button" className="h-5 w-5 rounded-full border border-white/20"
              style={{ backgroundColor: cl, boxShadow: color === cl ? `0 0 6px ${cl}` : "none" }}
              onClick={() => setColor(cl)} />)}
            <label className="relative inline-flex h-5 w-5 cursor-pointer items-center justify-center overflow-hidden rounded-full border border-white/30" title={t("mmdpop.customColor")}>
              <input type="color" value={color} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
                onChange={(e) => setColor(e.target.value)} />
              <span className="h-3 w-3 rounded-full" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
            </label>
          </div>
        </div>

        {error && <div className="rounded-md border border-red-500/20 bg-red-500/10 px-2.5 py-1.5 text-[10px] text-red-300">{error}</div>}
        {done && <div className="rounded-md border border-emerald-500/20 bg-emerald-500/10 px-2.5 py-1.5 text-[10px] text-emerald-300">✓ {done}</div>}

        <button onClick={() => void submit()} disabled={busy || !content.trim() || (!docId && !newDocName.trim())}
          className="shrink-0 rounded-lg py-2 text-xs font-semibold text-white disabled:opacity-40"
          style={{ backgroundColor: "var(--mm-accent)" }}>
          {busy ? t("mmdpop.recording") : t("mmdpop.asSticker")}
        </button>
      </div>

      {/* 底部高度拖拽把手：手动调整窗口高度（双击恢复自动贴合） */}
      <div {...gripHandlers}
        className="flex h-2.5 shrink-0 cursor-ns-resize touch-none select-none items-center justify-center"
        title={manualH ? t("mmdpop.resizeTipManual") : t("mmdpop.resizeTipAuto")}>
        <div className="h-0.5 w-10 rounded-full bg-white/20 transition hover:bg-[var(--mm-accent)]" />
      </div>
    </div>
  );
}
