import { memo, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { ChevronDown, ChevronRight, ListTree, PanelRight } from "lucide-react";

// ─── 本地路径工具 ───

function isLocalFilePath(value: string): boolean {
  return /^(?:[A-Za-z]:[\\/]|\\\\|\/)/.test(value.trim());
}
function decodeLocalPath(value: string): string {
  try { return decodeURIComponent(value); } catch { return value; }
}
/** react-markdown v10 默认 urlTransform 会把 C:/... 当作不安全协议清空 href/src，这里放行本地磁盘路径。 */
function localUrlTransform(value: string): string {
  if (isLocalFilePath(value)) return value;
  const colon = value.indexOf(":");
  const q = value.indexOf("?");
  const h = value.indexOf("#");
  const s = value.indexOf("/");
  if (colon === -1 || colon > (q === -1 ? value.length : q) || colon > (h === -1 ? value.length : h) || colon > (s === -1 ? value.length : s)) return value;
  const protocol = value.slice(0, colon).toLowerCase();
  return /^(https?|ircs?|mailto|xmpp)$/i.test(protocol) ? value : "";
}

// ─── GitHub 风格 slug（中文保留，用于锚点） ───

function slugify(text: string): string {
  return text.toLowerCase().trim().replace(/[^\w\u4e00-\u9fa5\s-]/g, "").replace(/\s+/g, "-").replace(/-+/g, "-") || "section";
}

// ─── 本地图片：通过 image_to_base64 读取为 data URL（不依赖 asset 协议作用域） ───

function LocalImage({ path, alt, onOpenFile }: { path: string; alt: string; onOpenFile: (p: string) => void }) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setSrc(null); setFailed(false);
    void invoke<string>("image_to_base64", { filePath: path })
      .then((d) => { if (!cancelled) setSrc(d); })
      .catch(() => { if (!cancelled) setFailed(true); });
    return () => { cancelled = true; };
  }, [path]);
  if (failed) {
    return <button type="button" className="my-2 block max-w-full text-left text-[9px] text-red-300" onClick={() => onOpenFile(path)} title="打开文件">图片加载失败，点击打开文件</button>;
  }
  if (!src) {
    return <div className="my-2 h-16 animate-pulse rounded border border-white/10 bg-slate-900/60" />;
  }
  return <button type="button" className="my-2 block max-w-full text-left" onClick={() => onOpenFile(path)} title="打开图片"><img src={src} alt={alt} className="max-h-64 max-w-full rounded border border-white/10 object-contain" /></button>;
}

// ─── 内置思维导图（```mindmap 代码块：缩进列表 → 可折叠树） ───

type MmTree = { name: string; children: MmTree[] };

function parseMindmapTree(lines: string[]): MmTree[] {
  const roots: MmTree[] = [];
  const stack: { indent: number; node: MmTree }[] = [];
  for (const raw of lines) {
    const norm = raw.replace(/\t/g, "    ");
    if (!norm.trim()) continue;
    const indent = norm.length - norm.trimStart().length;
    const name = norm.trim().replace(/^[-*+]\s+/, "").trim();
    if (!name) continue;
    const node: MmTree = { name, children: [] };
    while (stack.length && stack[stack.length - 1].indent >= indent) stack.pop();
    if (stack.length) stack[stack.length - 1].node.children.push(node);
    else roots.push(node);
    stack.push({ indent, node });
  }
  return roots;
}

const TREE_DEPTH_COLORS = ["#22d3ee", "#34d399", "#fbbf24", "#a78bfa", "#fb7185", "#60a5fa"];

function TreeItem({ node, depth }: { node: MmTree; depth: number }) {
  const [open, setOpen] = useState(true);
  const hasChildren = node.children.length > 0;
  const dot = TREE_DEPTH_COLORS[depth % TREE_DEPTH_COLORS.length];
  return (
    <div>
      <div className="flex items-center gap-1.5 py-0.5">
        {hasChildren ? (
          <button type="button" className="nodrag nopan inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center text-slate-400 hover:text-white"
            onClick={() => setOpen(!open)} title={open ? "折叠" : "展开"}>
            {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
          </button>
        ) : <span className="inline-block w-3.5 shrink-0" />}
        <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ backgroundColor: dot, boxShadow: `0 0 5px ${dot}66` }} />
        <span className="min-w-0 break-words text-[10px] leading-4 text-slate-200">{node.name}</span>
      </div>
      {open && hasChildren && (
        <div className="ml-3 border-l border-white/10 pl-2">
          {node.children.map((c, i) => <TreeItem key={i} node={c} depth={depth + 1} />)}
        </div>
      )}
    </div>
  );
}

function MindmapBlock({ code }: { code: string }) {
  const tree = useMemo(() => parseMindmapTree(code.split("\n")), [code]);
  return (
    <div className="my-2 rounded-lg border border-cyan-400/20 bg-slate-950/70 p-2.5">
      <div className="mb-1.5 flex items-center gap-1 text-[9px] uppercase tracking-wide text-cyan-300/80">
        <ListTree className="h-3 w-3" />内置思维导图
      </div>
      {tree.length === 0 ? <div className="text-[10px] text-slate-500">（空）</div> : tree.map((n, i) => <TreeItem key={i} node={n} depth={0} />)}
    </div>
  );
}

// ─── 章节目录 ───

type TocItem = { level: number; text: string; id: string };

function extractToc(content: string): TocItem[] {
  const items: TocItem[] = [];
  const seen = new Map<string, number>();
  for (const line of content.split("\n")) {
    const m = /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
    if (!m) continue;
    const text = m[2].trim();
    const base = slugify(text);
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    items.push({ level: m[1].length, text, id: count > 0 ? `${base}-${count}` : base });
  }
  return items;
}

// ─── 主渲染器 ───

export const MindmapMarkdown = memo(function MindmapMarkdown({ content }: { content: string }) {
  const toc = useMemo(() => extractToc(content), [content]);
  const [showToc, setShowToc] = useState(true);
  // 与 toc 顺序对齐的标题计数器（react-markdown 按文档顺序调用标题渲染器）
  const headingIndex = useRef(0);
  headingIndex.current = 0;

  const scrollTo = (id: string) => {
    document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  const openLocal = (path: string) => {
    void openPath(path).catch(() => { /* 浏览器预览无 Tauri 后端时静默 */ });
  };

  const heading = (level: number, cls: string) => {
    return ({ children }: { children?: React.ReactNode }) => {
      const item = toc[headingIndex.current];
      headingIndex.current += 1;
      const props = { id: item?.id, className: cls };
      switch (level) {
        case 1: return <h1 {...props}>{children}</h1>;
        case 2: return <h2 {...props}>{children}</h2>;
        case 3: return <h3 {...props}>{children}</h3>;
        case 4: return <h4 {...props}>{children}</h4>;
        case 5: return <h5 {...props}>{children}</h5>;
        default: return <h6 {...props}>{children}</h6>;
      }
    };
  };

  return (
    <div className="mindmap-markdown text-[11px] leading-relaxed text-slate-200 break-words">
      {toc.length >= 2 && (
        <div className="mb-3 rounded-lg border border-white/10 bg-slate-900/60">
          <button type="button" className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-[9px] uppercase tracking-wide text-slate-400 hover:text-slate-200"
            onClick={() => setShowToc(!showToc)}>
            <PanelRight className="h-3 w-3" />章节目录
            {showToc ? <ChevronDown className="h-3 w-3 ml-auto" /> : <ChevronRight className="h-3 w-3 ml-auto" />}
          </button>
          {showToc && (
            <nav className="max-h-40 overflow-y-auto px-2.5 pb-2">
              {toc.map((t, i) => (
                <button key={i} type="button"
                  className="block w-full truncate rounded px-1.5 py-0.5 text-left text-[10px] text-slate-400 hover:bg-white/5 hover:text-cyan-300"
                  style={{ paddingLeft: `${8 + (t.level - 1) * 12}px` }}
                  onClick={() => scrollTo(t.id)}>{t.text}</button>
              ))}
            </nav>
          )}
        </div>
      )}
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        urlTransform={localUrlTransform}
        components={{
          h1: heading(1, "mt-3 mb-2 text-base font-bold text-white first:mt-0"),
          h2: heading(2, "mt-3 mb-1.5 text-sm font-bold text-slate-100"),
          h3: heading(3, "mt-2.5 mb-1 text-[12px] font-semibold text-slate-200"),
          h4: heading(4, "mt-2 mb-1 text-[11px] font-semibold text-slate-300"),
          h5: heading(5, "mt-2 mb-1 text-[11px] font-semibold text-slate-400"),
          h6: heading(6, "mt-2 mb-1 text-[10px] font-semibold text-slate-400"),
          p: ({ children }) => <p className="my-1.5 first:mt-0 last:mb-0">{children}</p>,
          ul: ({ children }) => <ul className="my-1.5 list-disc space-y-0.5 pl-5">{children}</ul>,
          ol: ({ children }) => <ol className="my-1.5 list-decimal space-y-0.5 pl-5">{children}</ol>,
          li: ({ children }) => <li className="leading-5">{children}</li>,
          input: ({ checked }) => <input type="checkbox" checked={checked} readOnly className="mr-1.5 h-3 w-3 rounded align-middle accent-emerald-500" />,
          blockquote: ({ children }) => <blockquote className="my-2 border-l-2 border-cyan-400/50 pl-2.5 text-slate-400">{children}</blockquote>,
          code: ({ className, children }) => {
            const match = /language-(\w+)/.exec(className || "");
            const lang = match ? match[1] : "";
            if (lang === "mindmap") {
              return <MindmapBlock code={String(children).replace(/\n$/, "")} />;
            }
            return <code className="rounded bg-slate-700/60 px-1 py-0.5 font-mono text-[10px] text-cyan-200">{children}</code>;
          },
          pre: ({ children }) => <pre className="my-2 overflow-x-auto rounded border border-white/10 bg-slate-950 p-2 font-mono text-[10px] text-slate-300">{children}</pre>,
          a: ({ href, children }) => {
            const target = href ?? "";
            const local = isLocalFilePath(target);
            return <a href={local ? undefined : target} target={local ? undefined : "_blank"} rel={local ? undefined : "noopener noreferrer"}
              onClick={(e) => { if (local) { e.preventDefault(); openLocal(decodeLocalPath(target)); } }}
              className="text-cyan-300 underline decoration-cyan-400/40 underline-offset-2 hover:text-cyan-100">{children}</a>;
          },
          img: ({ src, alt }) => {
            const target = typeof src === "string" ? src : "";
            if (isLocalFilePath(target)) {
              return <LocalImage path={decodeLocalPath(target)} alt={alt ?? ""} onOpenFile={openLocal} />;
            }
            return <img src={target} alt={alt ?? ""} className="max-h-64 max-w-full rounded border border-white/10 object-contain" />;
          },
          table: ({ children }) => <div className="my-2 overflow-x-auto rounded border border-white/10"><table className="min-w-full text-[10px]">{children}</table></div>,
          thead: ({ children }) => <thead className="bg-slate-800/80">{children}</thead>,
          th: ({ children }) => <th className="border-b border-white/10 px-2 py-1 text-left text-cyan-200">{children}</th>,
          td: ({ children }) => <td className="border-b border-white/5 px-2 py-1 text-slate-300">{children}</td>,
          hr: () => <hr className="my-2 border-white/10" />,
          strong: ({ children }) => <strong className="font-semibold text-white">{children}</strong>,
          del: ({ children }) => <del className="text-slate-500">{children}</del>,
        }}
      >{content || "暂无内容"}</ReactMarkdown>
    </div>
  );
});
