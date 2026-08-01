import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  FolderOpen,
  X,
  FileText,
  Search,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
  ListTree,
  AlertCircle,
} from "lucide-react";

/** 后端 list_sibling_markdown 返回的条目 */
interface MarkdownEntry {
  path: string;
  name: string;
  rel: string;
  size: number;
}

/** 一个打开的选项卡 */
interface Tab {
  path: string;
  name: string;
  content: string;
  error: string | null;
  loading: boolean;
}

/** 文档内提取出的标题，用于右侧大纲 */
interface Heading {
  id: string;
  text: string;
  level: number;
}

const formatSize = (n: number): string => {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
};

/** 把标题文本转成锚点 id，与 rehype-slug 的规则近似 */
const slugify = (text: string): string =>
  text
    .toLowerCase()
    .trim()
    .replace(/[^\w\u4e00-\u9fa5\s-]/g, "")
    .replace(/\s+/g, "-");

/** 外链判定：http(s)、协议链接、纯锚点都不当作本地文件处理 */
const isExternal = (href: string): boolean =>
  /^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("//");

export default function MarkdownReader() {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [siblings, setSiblings] = useState<MarkdownEntry[]>([]);
  const [siblingRoot, setSiblingRoot] = useState<string>("");
  const [scanning, setScanning] = useState(false);
  const [filter, setFilter] = useState("");
  const [showSidebar, setShowSidebar] = useState(true);
  const [notice, setNotice] = useState<string | null>(null);
  const contentRef = useRef<HTMLDivElement>(null);

  const active = useMemo(
    () => tabs.find((t) => t.path === activePath) || null,
    [tabs, activePath]
  );

  const flashNotice = useCallback((msg: string) => {
    setNotice(msg);
    window.setTimeout(() => setNotice((n) => (n === msg ? null : n)), 2600);
  }, []);

  /** 扫描某个文件所在目录的全部 markdown */
  const scanSiblings = useCallback(async (fromPath: string) => {
    setScanning(true);
    try {
      const list = await invoke<MarkdownEntry[]>("list_sibling_markdown", {
        path: fromPath,
        maxDepth: 3,
      });
      setSiblings(list);
      const sep = fromPath.includes("\\") ? "\\" : "/";
      setSiblingRoot(fromPath.slice(0, fromPath.lastIndexOf(sep)));
    } catch (e) {
      console.error(e);
      setSiblings([]);
    } finally {
      setScanning(false);
    }
  }, []);

  /**
   * 打开一个文件为选项卡。已打开则直接激活，避免重复标签。
   * rescan 为 true 时同时刷新左侧同目录列表。
   */
  const openPath = useCallback(
    async (path: string, rescan = false) => {
      setActivePath(path);
      let existed = false;
      setTabs((prev) => {
        if (prev.some((t) => t.path === path)) {
          existed = true;
          return prev;
        }
        const sep = path.includes("\\") ? "\\" : "/";
        const name = path.slice(path.lastIndexOf(sep) + 1);
        return [...prev, { path, name, content: "", error: null, loading: true }];
      });

      if (rescan) void scanSiblings(path);
      if (existed) return;

      try {
        const content = await invoke<string>("read_text_file", { path });
        setTabs((prev) =>
          prev.map((t) => (t.path === path ? { ...t, content, loading: false } : t))
        );
      } catch (e) {
        setTabs((prev) =>
          prev.map((t) =>
            t.path === path ? { ...t, error: String(e), loading: false } : t
          )
        );
      }
    },
    [scanSiblings]
  );

  /** 文件选择对话框，支持一次选多个 */
  const pickFiles = useCallback(async () => {
    try {
      const selected = await open({
        multiple: true,
        title: "打开 Markdown 文件",
        filters: [{ name: "Markdown", extensions: ["md", "markdown", "mdx", "mdown"] }],
      });
      if (!selected) return;
      const list = Array.isArray(selected) ? selected : [selected];
      for (let i = 0; i < list.length; i++) {
        await openPath(list[i], i === 0);
      }
    } catch (e) {
      console.error(e);
    }
  }, [openPath]);

  const closeTab = useCallback(
    (path: string, e?: React.MouseEvent) => {
      e?.stopPropagation();
      setTabs((prev) => {
        const idx = prev.findIndex((t) => t.path === path);
        const next = prev.filter((t) => t.path !== path);
        // 关掉的是当前标签时，激活右邻居，没有则激活左邻居
        if (path === activePath) {
          const fallback = next[idx] || next[idx - 1] || null;
          setActivePath(fallback ? fallback.path : null);
        }
        return next;
      });
    },
    [activePath]
  );

  const reloadActive = useCallback(async () => {
    if (!active) return;
    const path = active.path;
    setTabs((prev) =>
      prev.map((t) => (t.path === path ? { ...t, loading: true, error: null } : t))
    );
    try {
      const content = await invoke<string>("read_text_file", { path });
      setTabs((prev) =>
        prev.map((t) => (t.path === path ? { ...t, content, loading: false } : t))
      );
    } catch (e) {
      setTabs((prev) =>
        prev.map((t) => (t.path === path ? { ...t, error: String(e), loading: false } : t))
      );
    }
  }, [active]);

  /**
   * 文档内链接点击：本地相对链接交给后端解析成绝对路径，
   * 解析成功就在新选项卡打开；纯锚点则滚动到对应标题。
   */
  const handleLinkClick = useCallback(
    async (href: string, e: React.MouseEvent) => {
      if (!active) return;
      if (href.startsWith("#")) {
        e.preventDefault();
        const id = href.slice(1).toLowerCase();
        const el = contentRef.current?.querySelector(`#${CSS.escape(id)}`);
        el?.scrollIntoView({ behavior: "smooth", block: "start" });
        return;
      }
      if (isExternal(href)) return; // 交给浏览器/系统处理
      e.preventDefault();
      try {
        const resolved = await invoke<string | null>("resolve_markdown_link", {
          from: active.path,
          href,
        });
        if (resolved) {
          await openPath(resolved);
        } else {
          flashNotice(`未找到链接目标：${href}`);
        }
      } catch (err) {
        flashNotice(String(err));
      }
    },
    [active, openPath, flashNotice]
  );

  /** 从正文提取标题生成大纲（跳过围栏代码块内的 # 行） */
  const headings = useMemo<Heading[]>(() => {
    if (!active?.content) return [];
    const out: Heading[] = [];
    let inFence = false;
    for (const line of active.content.split("\n")) {
      if (/^\s*(```|~~~)/.test(line)) {
        inFence = !inFence;
        continue;
      }
      if (inFence) continue;
      const m = /^(#{1,6})\s+(.*)$/.exec(line);
      if (m) {
        const text = m[2].replace(/[*_`]/g, "").trim();
        if (text) out.push({ id: slugify(text), text, level: m[1].length });
      }
    }
    return out;
  }, [active?.content]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return siblings;
    return siblings.filter(
      (s) => s.rel.toLowerCase().includes(q) || s.name.toLowerCase().includes(q)
    );
  }, [siblings, filter]);

  // Ctrl+W 关闭当前标签
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key.toLowerCase() === "w" && activePath) {
        e.preventDefault();
        closeTab(activePath);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activePath, closeTab]);

  const btn =
    "flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed";

  /** 带锚点 id 的标题渲染器 */
  const heading = (level: number, cls: string) =>
    function H({ children }: { children?: React.ReactNode }) {
      const text = React.Children.toArray(children)
        .map((c) => (typeof c === "string" ? c : ""))
        .join("");
      const Tag = `h${level}` as keyof React.JSX.IntrinsicElements;
      return React.createElement(Tag, { id: slugify(text), className: cls }, children);
    };

  return (
    <div className="h-full flex flex-col min-h-0 select-none">
      {/* 顶部工具栏 */}
      <div className="flex-shrink-0 flex items-center gap-2 px-4 py-2 border-b border-white/5">
        <button onClick={pickFiles} className={btn}>
          <FolderOpen className="w-3.5 h-3.5" />
          打开文件
        </button>
        <button onClick={reloadActive} disabled={!active} className={btn}>
          <RefreshCw className="w-3.5 h-3.5" />
          重新加载
        </button>
        <button onClick={() => setShowSidebar((s) => !s)} className={btn}>
          {showSidebar ? (
            <PanelLeftClose className="w-3.5 h-3.5" />
          ) : (
            <PanelLeftOpen className="w-3.5 h-3.5" />
          )}
          {showSidebar ? "隐藏目录" : "显示目录"}
        </button>
        <div className="flex-1" />
        {notice && (
          <div className="flex items-center gap-1 text-[10px] text-amber-300">
            <AlertCircle className="w-3 h-3" />
            {notice}
          </div>
        )}
        {active && (
          <span className="text-[10px] text-slate-500 truncate max-w-[38%]" title={active.path}>
            {active.path}
          </span>
        )}
      </div>

      {/* 选项卡条 */}
      {tabs.length > 0 && (
        <div className="flex-shrink-0 flex items-stretch gap-0.5 px-2 pt-1.5 border-b border-white/5 overflow-x-auto">
          {tabs.map((t) => (
            <div
              key={t.path}
              onClick={() => setActivePath(t.path)}
              title={t.path}
              className={`group flex items-center gap-1.5 px-2.5 py-1.5 rounded-t-md text-[11px] whitespace-nowrap cursor-pointer transition-colors max-w-[200px] ${
                t.path === activePath
                  ? "bg-emerald-600/20 text-emerald-200 border-b-2 border-emerald-500"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/5 border-b-2 border-transparent"
              }`}
            >
              <FileText className="w-3 h-3 flex-shrink-0" />
              <span className="truncate">{t.name}</span>
              <button
                onClick={(e) => closeTab(t.path, e)}
                className="flex-shrink-0 opacity-0 group-hover:opacity-100 text-slate-500 hover:text-red-300 transition-opacity"
                title="关闭 (Ctrl+W)"
              >
                <X className="w-3 h-3" />
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="flex-1 flex min-h-0">
        {/* 左侧：同目录文件列表 */}
        {showSidebar && (
          <div className="w-56 flex-shrink-0 border-r border-white/5 flex flex-col min-h-0">
            <div className="flex-shrink-0 px-2 py-2 border-b border-white/5">
              <div className="flex items-center gap-1.5 mb-1.5 text-[10px] text-slate-500">
                <ListTree className="w-3 h-3" />
                <span className="truncate" title={siblingRoot}>
                  {siblingRoot ? siblingRoot.split(/[\\/]/).pop() : "同目录文件"}
                </span>
                {scanning && <RefreshCw className="w-3 h-3 animate-spin ml-auto" />}
                {!scanning && siblings.length > 0 && (
                  <span className="ml-auto">{siblings.length}</span>
                )}
              </div>
              <div className="relative">
                <Search className="w-3 h-3 absolute left-2 top-1/2 -translate-y-1/2 text-slate-500" />
                <input
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  placeholder="筛选文件"
                  className="w-full bg-white/5 rounded pl-6 pr-2 py-1 text-[10px] text-slate-200 placeholder:text-slate-600 outline-none focus:bg-white/10"
                />
              </div>
            </div>
            <div className="flex-1 overflow-y-auto py-1">
              {filtered.length === 0 && (
                <div className="px-3 py-4 text-[10px] text-slate-600 text-center">
                  {siblings.length === 0 ? "打开文件后自动列出同目录" : "无匹配"}
                </div>
              )}
              {filtered.map((s) => (
                <button
                  key={s.path}
                  onClick={() => openPath(s.path)}
                  title={`${s.rel} · ${formatSize(s.size)}`}
                  className={`w-full text-left px-2.5 py-1 text-[10px] flex items-center gap-1.5 transition-colors ${
                    s.path === activePath
                      ? "bg-emerald-600/20 text-emerald-200"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
                  }`}
                >
                  <FileText className="w-3 h-3 flex-shrink-0 opacity-60" />
                  <span className="truncate">{s.rel}</span>
                </button>
              ))}
            </div>
          </div>
        )}

        {/* 中间：正文 */}
        <div ref={contentRef} className="flex-1 min-w-0 overflow-y-auto select-text">
          {!active && (
            <div className="h-full flex flex-col items-center justify-center gap-3 text-slate-600">
              <FileText className="w-10 h-10 opacity-30" />
              <div className="text-[11px]">尚未打开文档</div>
              <button onClick={pickFiles} className={btn}>
                <FolderOpen className="w-3.5 h-3.5" />
                打开 Markdown 文件
              </button>
              <div className="text-[10px] text-slate-700 max-w-xs text-center leading-relaxed">
                打开后会自动扫描同目录（含子目录 3 层）的所有 Markdown，
                文中相对链接可直接点击，在新选项卡中打开。
              </div>
            </div>
          )}
          {active?.loading && (
            <div className="px-6 py-6 text-[11px] text-slate-500">加载中…</div>
          )}
          {active?.error && (
            <div className="px-6 py-6 text-[11px] text-red-300 flex items-start gap-2">
              <AlertCircle className="w-4 h-4 flex-shrink-0 mt-0.5" />
              <span className="break-all">{active.error}</span>
            </div>
          )}
          {active && !active.loading && !active.error && (
            <div className="px-6 py-5 max-w-4xl mx-auto text-[12px] leading-7 text-slate-200 break-words">
              <ReactMarkdown
                remarkPlugins={[remarkGfm]}
                components={{
                  h1: heading(1, "text-xl font-bold text-slate-100 mt-6 mb-3 pb-1.5 border-b border-white/10 first:mt-0"),
                  h2: heading(2, "text-lg font-bold text-slate-100 mt-5 mb-2.5 pb-1 border-b border-white/5 first:mt-0"),
                  h3: heading(3, "text-base font-bold text-slate-200 mt-4 mb-2 first:mt-0"),
                  h4: heading(4, "text-sm font-bold text-slate-200 mt-3 mb-1.5 first:mt-0"),
                  h5: heading(5, "text-[12px] font-semibold text-slate-300 mt-3 mb-1 first:mt-0"),
                  h6: heading(6, "text-[11px] font-semibold text-slate-400 mt-3 mb-1 first:mt-0"),
                  p: ({ children }) => <p className="my-2.5 first:mt-0 last:mb-0">{children}</p>,
                  ul: ({ children }) => <ul className="list-disc my-2.5 space-y-1 pl-6">{children}</ul>,
                  ol: ({ children }) => <ol className="list-decimal my-2.5 space-y-1 pl-6">{children}</ol>,
                  li: ({ children }) => <li className="leading-7">{children}</li>,
                  input: ({ checked }) => (
                    <input
                      type="checkbox"
                      checked={checked}
                      readOnly
                      className="mr-1.5 align-middle w-3 h-3 rounded accent-emerald-500"
                    />
                  ),
                  blockquote: ({ children }) => (
                    <blockquote className="border-l-2 border-emerald-600/50 pl-3 my-3 text-slate-400">
                      {children}
                    </blockquote>
                  ),
                  code: ({ className, children }) => {
                    const match = /language-(\w+)/.exec(className || "");
                    if (!match) {
                      return (
                        <code className="px-1 py-0.5 rounded bg-slate-700/60 text-[11px] text-emerald-300 font-mono">
                          {children}
                        </code>
                      );
                    }
                    return <CodeBlock lang={match[1]}>{children}</CodeBlock>;
                  },
                  pre: ({ children }) => <>{children}</>,
                  a: ({ href, children }) => {
                    const h = href || "";
                    const external = isExternal(h);
                    return (
                      <a
                        href={h}
                        target={external ? "_blank" : undefined}
                        rel={external ? "noopener noreferrer" : undefined}
                        onClick={(e) => handleLinkClick(h, e)}
                        className={`underline underline-offset-2 cursor-pointer ${
                          external
                            ? "text-sky-400 hover:text-sky-300"
                            : "text-emerald-400 hover:text-emerald-300"
                        }`}
                      >
                        {children}
                      </a>
                    );
                  },
                  img: ({ src, alt }) => (
                    <img src={typeof src === "string" ? src : ""} alt={alt || ""} className="max-w-full rounded my-3" />
                  ),
                  table: ({ children }) => (
                    <div className="overflow-x-auto my-3 rounded border border-white/10">
                      <table className="min-w-full text-[11px]">{children}</table>
                    </div>
                  ),
                  thead: ({ children }) => <thead className="bg-slate-800/80">{children}</thead>,
                  th: ({ children }) => (
                    <th className="px-2.5 py-1.5 text-left font-semibold text-slate-200 border-b border-white/10">
                      {children}
                    </th>
                  ),
                  td: ({ children }) => (
                    <td className="px-2.5 py-1.5 text-slate-300 border-b border-white/5">{children}</td>
                  ),
                  hr: () => <hr className="border-white/10 my-5" />,
                  strong: ({ children }) => <strong className="font-bold text-slate-100">{children}</strong>,
                  del: ({ children }) => <del className="text-slate-500">{children}</del>,
                }}
              >
                {active.content}
              </ReactMarkdown>
            </div>
          )}
        </div>

        {/* 右侧：大纲 */}
        {active && headings.length > 1 && (
          <div className="w-48 flex-shrink-0 border-l border-white/5 overflow-y-auto py-2">
            <div className="px-3 pb-1.5 text-[10px] text-slate-500">大纲</div>
            {headings.map((h, i) => (
              <button
                key={`${h.id}-${i}`}
                onClick={() => {
                  const el = contentRef.current?.querySelector(`#${CSS.escape(h.id)}`);
                  el?.scrollIntoView({ behavior: "smooth", block: "start" });
                }}
                style={{ paddingLeft: `${8 + (h.level - 1) * 10}px` }}
                className="w-full text-left pr-2 py-0.5 text-[10px] text-slate-500 hover:text-emerald-300 hover:bg-white/5 truncate transition-colors"
                title={h.text}
              >
                {h.text}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** 代码块：语言标签 + 一键复制 */
function CodeBlock({ lang, children }: { lang: string; children: React.ReactNode }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = useCallback(() => {
    const text = typeof children === "string" ? children : String(children);
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  }, [children]);

  return (
    <div className="relative my-3 rounded-lg overflow-hidden border border-white/10 bg-slate-900/80">
      <div className="flex items-center justify-between px-2.5 py-1 bg-slate-800/60 border-b border-white/5">
        <span className="text-[9px] font-mono text-slate-400 uppercase tracking-wide">{lang}</span>
        <button
          onClick={handleCopy}
          className="text-[9px] text-slate-500 hover:text-slate-200 transition-colors cursor-pointer"
        >
          {copied ? "✓ 已复制" : "复制"}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-[11px] leading-relaxed">
        <code className="font-mono text-slate-300">{children}</code>
      </pre>
    </div>
  );
}
