import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { createPortal } from "react-dom";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import "@xyflow/react/dist/style.css";
import { MarkdownFieldEditor } from "../shared/markdown/MarkdownFieldEditor";
import { NodeFormFields } from "./NodeFormFields";
import { File, FolderOpen, X, Maximize2, Minimize2 } from "lucide-react";
import { MindmapNode, kindColor } from "./types";
import { openSourceFile } from "./agentShared";
import { normalizeHexColor } from "./MindmapPanel.model";
// 注：原先的「亮一条到根的祖先链」（ancestorChain）已随高亮口径改为「直接相连」而移除——
// 选中深处的节点时它会亮起一大片，注意力反而收不回来。层级关系仍由父子连线本身表达。

// ════════════ 详细弹窗（拖拽分隔条 + 全屏） ════════════

export function DetailModal({ node, onUpdate, onClose, projectRoot }: { node: MindmapNode; accent?: string; onUpdate: (patch: Partial<MindmapNode>) => void; onClose: () => void; projectRoot?: string }) {
  const { t } = useTranslation();
  // 文件点击：用全局配置的编辑器打开（未配置时回退资源管理器定位）
  const openSource = (src: string) => {
    if (!projectRoot) return;
    openSourceFile(projectRoot, src);
  };
  // 文件（原「证据文件」）：本地可编辑状态，增删后随防抖自动保存落库。
  // 存储路径优先项目相对路径；项目外文件存绝对路径（openSourceFile 兼容）。
  const [sources, setSources] = useState<string[]>(node.sources ?? []);
  const [srcInput, setSrcInput] = useState("");
  // 双击节点进入详情后直接可编辑；预览/分栏交给下方 MarkdownFieldEditor 自带工具栏。
  const [detail, setDetail] = useState(node.detail);
  const [name, setName] = useState(node.name);
  const [color, setColor] = useState(node.color);
  const [fullscreen, setFullscreen] = useState(false);
  const c = normalizeHexColor(color) ?? kindColor(node.kind);

  const toStoredPath = useCallback((p: string): string => {
    const norm = p.trim().replace(/\\/g, "/");
    const root = projectRoot ? projectRoot.replace(/[\\/]+$/, "").replace(/\\/g, "/") : "";
    if (root && norm.toLowerCase().startsWith(root.toLowerCase() + "/")) return norm.slice(root.length + 1);
    return norm;
  }, [projectRoot]);

  const addSource = useCallback(() => {
    const raw = srcInput.trim();
    if (!raw) return;
    const norm = toStoredPath(raw);
    setSources(prev => (prev.includes(norm) ? prev : [...prev, norm]));
    setSrcInput("");
  }, [srcInput, toStoredPath]);

  const pickSourceFile = useCallback(async () => {
    const sel = await openDialog({ multiple: false, title: t("mindmap.pickFileBtn") });
    if (typeof sel !== "string") return;
    const norm = toStoredPath(sel);
    setSources(prev => (prev.includes(norm) ? prev : [...prev, norm]));
  }, [t, toStoredPath]);

  const save = useCallback(() => {
    onUpdate({ name, color, detail, sources });
  }, [name, color, detail, sources, onUpdate]);

  // Auto-save on unmount（用 ref 保存最新 save，避免 save 身份变化时
  // cleanup 反复触发 save → 父级 setState → 新 save → 无限循环卡死）
  const saveRef = useRef(save);
  useEffect(() => { saveRef.current = save; });
  useEffect(() => () => { saveRef.current(); }, []);

  // 文本字段/文件列表输入防抖自动保存（800ms 无变化后落盘），不再只依赖 blur/关闭。
  // 依赖仅内容值，saveRef 取最新 save，不会因 save 身份变化陷入循环。
  useEffect(() => {
    const t = window.setTimeout(() => { saveRef.current(); }, 800);
    return () => window.clearTimeout(t);
  }, [name, detail, sources]);

  // 弹窗高度随内容自动伸缩（与速记悬浮窗一致）：先以 height:auto 测量各子块
  // 自然高度（标题栏 + 证据 + 表单 + 详情编辑器），再把弹窗高度设为
  // min(自然高度, 视口可用高度)；超出后由表单区内部滚动兜底。
  const cardRef = useRef<HTMLDivElement>(null);
  const [fitH, setFitH] = useState<number | null>(null);
  useEffect(() => {
    if (fullscreen) { setFitH(null); return; }
    const card = cardRef.current;
    if (!card) return;
    let raf = 0;
    const measure = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const prev = card.style.height;
        card.style.height = "auto";
        const natural = card.scrollHeight;
        card.style.height = prev;
        const maxH = Math.max(420, window.innerHeight - 48); // modal-mask p-4 上下各 16px
        const desired = Math.round(Math.min(maxH, Math.max(420, natural)));
        setFitH((h) => (h === desired ? h : desired));
      });
    };
    const ro = new ResizeObserver(measure);
    ro.observe(card);
    for (const k of card.children) ro.observe(k);
    measure();
    return () => { ro.disconnect(); cancelAnimationFrame(raf); };
  }, [fullscreen, sources.length]);

  return createPortal(
    <div className="fixed inset-0 z-[200] modal-mask flex items-center justify-center bg-black/70 p-4 backdrop-blur-[3px]">
      <div ref={cardRef} className={`flex ${fullscreen ? "h-full w-full" : "w-[min(92vw,680px)]"} flex-col overflow-hidden rounded-card border border-white/10 bg-surface-panel shadow-2xl`} style={fullscreen ? undefined : { height: fitH ?? "auto" }} onClick={(e) => e.stopPropagation()}>
        <div className="flex h-11 shrink-0 items-center gap-2 border-b border-white/10 px-3" style={{ backgroundColor: `${c}1f` }}>
          <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: c, boxShadow: `0 0 9px ${c}` }} />
          <input className="min-w-0 flex-1 bg-transparent text-body font-semibold text-slate-100 outline-none" value={name} onChange={(e) => setName(e.target.value)} onBlur={save} />
          <div className="flex items-center gap-1 ml-1">
            <button type="button" className={`nodrag nopan rounded p-1 text-tiny ${fullscreen ? "bg-white/10 text-white" : "text-slate-400"} hover:bg-white/10 hover:text-white`} onClick={() => setFullscreen(!fullscreen)} title={fullscreen ? t("mindmap.fullscreenExit") : t("mindmap.fullscreen")}>{fullscreen ? <Minimize2 className="h-3.5 w-3.5" /> : <Maximize2 className="h-3.5 w-3.5" />}</button>
            <button type="button" className="nodrag nopan rounded p-1 text-slate-400 hover:text-white" onClick={onClose}><X className="h-4 w-4" /></button>
          </div>
        </div>
        {/* 文件（原「证据文件」）：点击用编辑器打开；可增删（手动输入路径或从项目内选择文件） */}
        <div className="shrink-0 border-b border-white/5 bg-white/[0.02] px-4 py-2">
          <div className="mb-1 flex items-center gap-1 text-micro text-slate-500">
            <File className="h-2.5 w-2.5" />{t("mindmap.evidence", { count: sources.length })}
            <span className="text-slate-600">{t("mindmap.evidenceHintEdit")}</span>
          </div>
          {sources.length > 0 && (
            <div className="mb-1.5 flex flex-wrap gap-1">
              {sources.map(s => (
                <span key={s} className={`nodrag nopan inline-flex max-w-[280px] items-center gap-0.5 rounded border border-cyan-400/25 bg-cyan-400/[0.07] py-0.5 pl-1.5 pr-0.5 font-mono text-[8px] text-cyan-200 ${projectRoot ? "" : "opacity-70"}`}>
                  <button type="button" onClick={() => openSource(s)} className={`min-w-0 truncate ${projectRoot ? "cursor-pointer transition hover:text-white" : "cursor-default"}`} title={s}>
                    <File className="mr-0.5 inline h-2.5 w-2.5 shrink-0" />{s}
                  </button>
                  <button type="button" onClick={() => setSources(prev => prev.filter(x => x !== s))}
                    className="nodrag nopan shrink-0 rounded p-0.5 text-slate-500 transition hover:text-red-400" title={t("mindmap.fileRemove")}>
                    <X className="h-2.5 w-2.5" />
                  </button>
                </span>
              ))}
            </div>
          )}
          <div className="flex items-center gap-1">
            <input value={srcInput} onChange={(e) => setSrcInput(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addSource(); } }}
              placeholder={t("mindmap.filePh")}
              className="h-6 min-w-0 flex-1 rounded border border-white/10 bg-slate-950/70 px-2 font-mono text-[8px] text-slate-200 outline-none focus:border-cyan-400/60" />
            <button type="button" onClick={addSource} disabled={!srcInput.trim()}
              className="nodrag nopan shrink-0 rounded border border-white/10 bg-white/[0.05] px-2 py-0.5 text-[8px] text-slate-300 transition hover:bg-white/10 hover:text-white disabled:opacity-40"
              title={t("mindmap.fileAdd")}>
              ＋ {t("mindmap.fileAdd")}
            </button>
            {projectRoot && (
              <button type="button" onClick={() => void pickSourceFile()}
                className="nodrag nopan shrink-0 rounded border border-white/10 bg-white/[0.05] p-1 text-slate-300 transition hover:bg-white/10 hover:text-white"
                title={t("mindmap.pickFileBtn")}>
                <FolderOpen className="h-3 w-3" />
              </button>
            )}
          </div>
        </div>
        {/* 表单字段区（与速记悬浮窗一致的压缩布局）：颜色单行卡片 */}
        <div className="shrink-0 space-y-2.5 overflow-y-auto p-3" style={{ maxHeight: "42%" }}>
          <NodeFormFields ns="mindmap"
            color={color}
            onColor={(v) => { setColor(v); onUpdate({ color: v }); }}
          />
        </div>
        {/* 详细内容：完整 Markdown 编辑器，占满剩余高度（不再留大片空白） */}
        <div className="flex min-h-0 flex-1 flex-col px-4 pb-4">
          <label className="mb-1 block text-micro text-slate-500">{t("mindmap.detailLabel")}</label>
          <div className="flex min-h-0 flex-1 flex-col">
            <MarkdownFieldEditor value={detail} onChange={(v) => setDetail(v)} />
          </div>
        </div>
      </div>
    </div>, document.body);
}
