import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import hljs from "highlight.js/lib/core";
import javascript from "highlight.js/lib/languages/javascript";
import typescript from "highlight.js/lib/languages/typescript";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import bash from "highlight.js/lib/languages/bash";
import json from "highlight.js/lib/languages/json";
import yaml from "highlight.js/lib/languages/yaml";
import xml from "highlight.js/lib/languages/xml";
import css from "highlight.js/lib/languages/css";
import sql from "highlight.js/lib/languages/sql";
import markdown from "highlight.js/lib/languages/markdown";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import ini from "highlight.js/lib/languages/ini";
import diff from "highlight.js/lib/languages/diff";

hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("js", javascript);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("ts", typescript);
hljs.registerLanguage("python", python);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("go", go);
hljs.registerLanguage("java", java);
hljs.registerLanguage("c", c);
hljs.registerLanguage("cpp", cpp);
hljs.registerLanguage("csharp", csharp);
hljs.registerLanguage("cs", csharp);
hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sh", bash);
hljs.registerLanguage("shell", bash);
hljs.registerLanguage("json", json);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("html", xml);
hljs.registerLanguage("css", css);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("md", markdown);
hljs.registerLanguage("dockerfile", dockerfile);
hljs.registerLanguage("ini", ini);
hljs.registerLanguage("toml", ini);
hljs.registerLanguage("diff", diff);
import {
  FolderOpen, X, FileText, Search, PanelLeftClose, PanelLeftOpen, RefreshCw,
  ListTree, AlertCircle, Pencil, Eye, Save, Check, Columns, Link, List,
  ListOrdered, Quote, Code, Minus, Heading1, Heading2, Heading3, Bold, Italic,
  Strikethrough, Table, Square, Undo2, Redo2, ListChecks, FilePlus, Folder,
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
  dirty: boolean;        // 有未保存修改
  savedContent: string;  // 磁盘上的内容
  undo: string[];        // 编辑历史（撤销栈）
  redo: string[];        // 重做栈
}

/** 文档内提取出的标题，用于右侧大纲 */
interface Heading {
  id: string;
  text: string;
  level: number;
}

/** 文件关联状态（后端 markdown_assoc_status） */
interface AssocStatus {
  md: boolean;
  markdown: boolean;
  exePath: string;
}

const MD_RECENT_KEY = "any_version_markdown_recent";

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

/** 在光标选区前后包裹 prefix/suffix；无选区时插入占位符并选中它 */
function wrapSelection(ta: HTMLTextAreaElement, prefix: string, suffix: string, placeholder = ""): { text: string; selStart: number; selEnd: number } {
  const { selectionStart: s, selectionEnd: e, value } = ta;
  const sel = value.slice(s, e);
  const inner = sel || placeholder;
  const next = value.slice(0, s) + prefix + inner + suffix + value.slice(e);
  return {
    text: next,
    selStart: s + prefix.length,
    selEnd: s + prefix.length + inner.length,
  };
}

/** 行首前缀切换（标题/引用/列表） */
function toggleLinePrefix(ta: HTMLTextAreaElement, prefix: string, ordered = false): { text: string; selStart: number; selEnd: number } {
  const { value } = ta;
  let s = ta.selectionStart;
  let e = ta.selectionEnd;
  // 扩展到整行（多行支持）
  while (s > 0 && value[s - 1] !== "\n") s--;
  while (e < value.length && value[e] !== "\n") e++;
  const block = value.slice(s, e);
  const lines = block.split("\n");
  const allHave = lines.every((l) => l.trimStart().startsWith(prefix));
  const next = lines
    .map((l, i) => {
      if (allHave) return l.replace(new RegExp(`^(\\s*)${prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s?`), "$1");
      if (ordered) return `${i + 1}. ${l.replace(/^\s*(#{1,6}\s|>\s|[-*+]\s|\d+\.\s)/, "")}`;
      return `${prefix} ${l.replace(/^\s*(#{1,6}\s|>\s|[-*+]\s|\d+\.\s)/, "")}`;
    })
    .join("\n");
  const out = value.slice(0, s) + next + value.slice(e);
  return { text: out, selStart: s, selEnd: s + next.length };
}

export default function MarkdownReader() {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [siblings, setSiblings] = useState<MarkdownEntry[]>([]);
  const [siblingRoot, setSiblingRoot] = useState<string>("");
  const [scanning, setScanning] = useState(false);
  const [filter, setFilter] = useState("");
  const [showSidebar, setShowSidebar] = useState(true);
  const [notice, setNotice] = useState<string | null>(null);
  const [mode, setMode] = useState<"view" | "edit">("view"); // 默认预览
  const [split, setSplit] = useState(false);                 // 编辑+预览分栏
  const [saving, setSaving] = useState(false);
  const [assoc, setAssoc] = useState<AssocStatus | null>(null);
  const [showAssoc, setShowAssoc] = useState(false);
  const [cursor, setCursor] = useState({ line: 1, col: 1 }); // 光标行列（编辑态状态栏）
  const contentRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const saveTimer = useRef<number | null>(null);

  const active = useMemo(
    () => tabs.find((t) => t.path === activePath) || null,
    [tabs, activePath]
  );

  const flashNotice = useCallback((msg: string) => {
    setNotice(msg);
    window.setTimeout(() => setNotice((n) => (n === msg ? null : n)), 2600);
  }, []);

  const loadAssoc = useCallback(async () => {
    try { setAssoc(await invoke<AssocStatus>("markdown_assoc_status")); } catch { setAssoc(null); }
  }, []);

  useEffect(() => { void loadAssoc(); }, [loadAssoc]);

  // 记住打开的 markdown 文件（存本地），模块卸载重挂载时恢复已打开的标签与侧栏目录。
  useEffect(() => {
    let alive = true;
    try {
      const saved = JSON.parse(localStorage.getItem(MD_RECENT_KEY) || "null");
      const paths: string[] = Array.isArray(saved?.paths)
        ? saved.paths.filter((p: unknown): p is string => typeof p === "string" && !!p)
        : [];
      if (paths.length === 0) return;
      (async () => {
        for (const p of paths) {
          if (!alive) return;
          await openPath(p);
        }
      })();
    } catch {
      // 忽略恢复失败（文件可能已被删除），回落到空白状态
    }
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 标签列表变化时持久化打开路径（含未保存状态提示可在重挂载后重建，内容已自动保存到磁盘）。
  useEffect(() => {
    const paths = tabs.map((t) => t.path).filter((p): p is string => !!p);
    try {
      localStorage.setItem(MD_RECENT_KEY, JSON.stringify({ activePath, paths }));
    } catch {
      // 忽略写入失败
    }
  }, [tabs, activePath]);

  const toggleAssoc = useCallback(async (register: boolean) => {
    try {
      const msg = await invoke<string>("set_markdown_assoc", { register });
      flashNotice(msg);
      await loadAssoc();
    } catch (e) {
      flashNotice(String(e));
    }
  }, [flashNotice, loadAssoc]);

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
   */
  const openPath = useCallback(
    async (path: string, _rescan = false) => {
      setActivePath(path);
      setMode("view");
      let existed = false;
      setTabs((prev) => {
        if (prev.some((t) => t.path === path)) {
          existed = true;
          return prev;
        }
        const sep = path.includes("\\") ? "\\" : "/";
        const name = path.slice(path.lastIndexOf(sep) + 1);
        return [...prev, { path, name, content: "", error: null, loading: true, dirty: false, savedContent: "", undo: [], redo: [] }];
      });

      // 打开文件即刷新左侧同目录列表，保证「文件导航」始终可用。
      void scanSiblings(path);
      if (existed) return;

      try {
        const content = await invoke<string>("read_text_file", { path });
        setTabs((prev) =>
          prev.map((t) => (t.path === path ? { ...t, content, savedContent: content, loading: false } : t))
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

  /** 打开目录：列出目录下所有 markdown 文件，不自动打开文件 */
  const openDirectory = useCallback(async () => {
    try {
      const dir = await open({
        directory: true,
        title: "选择目录",
      });
      if (!dir || typeof dir !== "string") return;
      setSiblingRoot(dir);
      setScanning(true);
      try {
        const list = await invoke<MarkdownEntry[]>("list_sibling_markdown", {
          path: dir,
          maxDepth: 3,
        });
        setSiblings(list);
      } catch (e) {
        console.error(e);
        setSiblings([]);
      } finally {
        setScanning(false);
      }
    } catch (e) {
      console.error(e);
    }
  }, []);

  /** 新建文件：在当前侧栏目录下创建空 .md 文件并打开 */
  const createNewFile = useCallback(async () => {
    const baseDir = siblingRoot || active?.path;
    if (!baseDir) {
      flashNotice("请先打开目录或文件");
      return;
    }
    const dir = active?.path ? active.path.slice(0, active.path.lastIndexOf(active.path.includes("\\") ? "\\" : "/")) : baseDir;
    const name = window.prompt("文件名（不含扩展名）:");
    if (!name?.trim()) return;
    const sep = dir.includes("\\") ? "\\" : "/";
    const filePath = `${dir}${sep}${name.trim()}.md`;
    try {
      await invoke("write_text_file", { path: filePath, content: `# ${name.trim()}\n\n` });
      await openPath(filePath);
      flashNotice(`已创建 ${name.trim()}.md`);
    } catch (e) {
      flashNotice(`创建失败: ${e}`);
    }
  }, [siblingRoot, active, openPath, flashNotice]);

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
        const t = prev.find((x) => x.path === path);
        if (t?.dirty && !window.confirm(`「${t.name}」有未保存修改，确定关闭并丢弃？`)) return prev;
        const idx = prev.findIndex((x) => x.path === path);
        const next = prev.filter((x) => x.path !== path);
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

  /** 编辑器内容变更（带撤销栈） */
  const setContent = useCallback((path: string, next: string, pushUndo = true) => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.path !== path) return t;
        const undo = pushUndo ? [...t.undo.slice(-200), t.content] : t.undo;
        return { ...t, content: next, undo, redo: [], dirty: next !== t.savedContent };
      })
    );
  }, []);

  /** 撤销/重做 */
  const undo = useCallback(() => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.path !== activePath || t.undo.length === 0) return t;
        const prevContent = t.undo[t.undo.length - 1];
        return {
          ...t,
          content: prevContent,
          undo: t.undo.slice(0, -1),
          redo: [...t.redo, t.content],
          dirty: prevContent !== t.savedContent,
        };
      })
    );
  }, [activePath]);

  const redo = useCallback(() => {
    setTabs((prev) =>
      prev.map((t) => {
        if (t.path !== activePath || t.redo.length === 0) return t;
        const nextContent = t.redo[t.redo.length - 1];
        return {
          ...t,
          content: nextContent,
          undo: [...t.undo, t.content],
          redo: t.redo.slice(0, -1),
          dirty: nextContent !== t.savedContent,
        };
      })
    );
  }, [activePath]);

  /** 保存到磁盘 */
  const saveActive = useCallback(async () => {
    if (!active || !active.dirty || saving) return;
    setSaving(true);
    try {
      await invoke("write_text_file", { path: active.path, content: active.content });
      setTabs((prev) =>
        prev.map((t) => (t.path === active.path ? { ...t, savedContent: t.content, dirty: false } : t))
      );
      flashNotice("已保存");
    } catch (e) {
      flashNotice(`保存失败: ${e}`);
    } finally {
      setSaving(false);
    }
  }, [active, saving, flashNotice]);

  // 自动保存（2s 防抖）
  useEffect(() => {
    if (!active?.dirty) return;
    if (saveTimer.current) window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => { void saveActive(); }, 2000);
    return () => { if (saveTimer.current) window.clearTimeout(saveTimer.current); };
  }, [active?.content, active?.dirty, saveActive, active]);

  const reloadActive = useCallback(async () => {
    if (!active) return;
    if (active.dirty && !window.confirm("有未保存修改，重新加载将丢弃。继续？")) return;
    const path = active.path;
    setTabs((prev) =>
      prev.map((t) => (t.path === path ? { ...t, loading: true, error: null } : t))
    );
    try {
      const content = await invoke<string>("read_text_file", { path });
      setTabs((prev) =>
        prev.map((t) => (t.path === path ? { ...t, content, savedContent: content, loading: false, dirty: false, undo: [], redo: [] } : t))
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
    for (const line of active.content.split(/\r?\n/)) {
      if (/^\s*(```|~~~)/.test(line)) {
        inFence = !inFence;
        continue;
      }
      if (inFence) continue;
      const m = /^(#{1,6})\s+(.*)$/m.exec(line);
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

  /** 实时字数统计：字符数（含/不含空白）、中文字数、英文单词数、行数 */
  const stats = useMemo(() => {
    const text = active?.content || "";
    const chars = text.length;
    const noSpace = text.replace(/\s/g, "").length;
    const cjk = (text.match(/[\u4e00-\u9fff\u3400-\u4dbf\uf900-\ufaff]/g) || []).length;
    const words = (text.match(/[A-Za-z0-9_]+(?:['-][A-Za-z0-9_]+)*/g) || []).length;
    const lines = text ? text.split(/\r?\n/).length : 0;
    return { chars, noSpace, cjk, words, lines };
  }, [active?.content]);

  /** 根据 selectionStart 计算光标所在行/列（1-based） */
  const updateCursor = useCallback(() => {
    const ta = taRef.current;
    if (!ta) return;
    const before = ta.value.slice(0, ta.selectionStart);
    const idx = before.lastIndexOf("\n");
    setCursor({
      line: before.split("\n").length,
      col: before.length - idx,
    });
  }, []);

  // 快捷键：Ctrl+S 保存 / Ctrl+W 关闭标签 / Ctrl+Z、Ctrl+Y（编辑态）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key.toLowerCase() === "s" && active?.dirty) {
        e.preventDefault();
        void saveActive();
        return;
      }
      if (e.ctrlKey && e.key.toLowerCase() === "w" && activePath) {
        e.preventDefault();
        closeTab(activePath);
        return;
      }
      if (mode === "edit" && taRef.current === document.activeElement) {
        if (e.ctrlKey && e.key.toLowerCase() === "z") { e.preventDefault(); undo(); }
        if (e.ctrlKey && (e.key.toLowerCase() === "y" || (e.shiftKey && e.key.toLowerCase() === "z"))) { e.preventDefault(); redo(); }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activePath, closeTab, active, saveActive, mode, undo, redo]);

  /** 工具栏动作：对 textarea 应用变换 */
  const applyTransform = useCallback((fn: (ta: HTMLTextAreaElement) => { text: string; selStart: number; selEnd: number }) => {
    const ta = taRef.current;
    if (!ta || !activePath) return;
    const { text, selStart, selEnd } = fn(ta);
    setContent(activePath, text);
    requestAnimationFrame(() => {
      ta.focus();
      ta.setSelectionRange(selStart, selEnd);
    });
  }, [activePath, setContent]);

  const btn =
    "flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed";
  const tbtn =
    "inline-flex items-center justify-center h-7 w-7 rounded-md text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-30";
  const assocAll = assoc ? assoc.md && assoc.markdown : false;

  /** 带锚点 id 的标题渲染器 */
  const heading = (level: number, cls: string) =>
    function H({ children }: { children?: React.ReactNode }) {
      const text = React.Children.toArray(children)
        .map((c) => (typeof c === "string" ? c : ""))
        .join("");
      const Tag = `h${level}` as keyof React.JSX.IntrinsicElements;
      return React.createElement(Tag, { id: slugify(text), className: cls }, children);
    };

  const preview = active && !active.loading && !active.error && (
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
  );

  const editor = active && !active.loading && !active.error && (
    <div className="flex flex-col min-h-0 flex-1">
      {/* 格式工具栏 */}
      <div className="flex-shrink-0 flex items-center gap-0.5 px-2 py-1.5 border-b border-white/5 overflow-x-auto">
        <button className={tbtn} title="撤销 (Ctrl+Z)" disabled={active.undo.length === 0} onClick={undo}><Undo2 className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="重做 (Ctrl+Y)" disabled={active.redo.length === 0} onClick={redo}><Redo2 className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title="标题 1" onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "#"))}><Heading1 className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="标题 2" onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "##"))}><Heading2 className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="标题 3" onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "###"))}><Heading3 className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title="粗体 (Ctrl+B)" onClick={() => applyTransform((ta) => wrapSelection(ta, "**", "**", "粗体"))}><Bold className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="斜体 (Ctrl+I)" onClick={() => applyTransform((ta) => wrapSelection(ta, "*", "*", "斜体"))}><Italic className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="删除线" onClick={() => applyTransform((ta) => wrapSelection(ta, "~~", "~~", "删除"))}><Strikethrough className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="行内代码" onClick={() => applyTransform((ta) => wrapSelection(ta, "`", "`", "代码"))}><Code className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title="无序列表" onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "-"))}><List className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="有序列表" onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "1.", true))}><ListOrdered className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="任务列表" onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "- [ ]"))}><ListChecks className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="引用" onClick={() => applyTransform((ta) => toggleLinePrefix(ta, ">"))}><Quote className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title="代码块" onClick={() => applyTransform((ta) => {
          const { selectionStart: s, selectionEnd: e, value } = ta;
          const sel = value.slice(s, e) || "// code";
          const insert = `\n\`\`\`\n${sel}\n\`\`\`\n`;
          return { text: value.slice(0, s) + insert + value.slice(e), selStart: s + 4, selEnd: s + 4 + sel.length };
        })}><Square className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="链接" onClick={() => applyTransform((ta) => wrapSelection(ta, "[", "](https://)", "链接文本"))}><Link className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="表格" onClick={() => applyTransform((ta) => {
          const tpl = "\n| 列1 | 列2 | 列3 |\n| --- | --- | --- |\n|  a  |  b  |  c  |\n";
          const { selectionStart: s, value } = ta;
          return { text: value.slice(0, s) + tpl + value.slice(s), selStart: s + 3, selEnd: s + 3 };
        })}><Table className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title="分隔线" onClick={() => applyTransform((ta) => {
          const { selectionStart: s, value } = ta;
          return { text: value.slice(0, s) + "\n---\n" + value.slice(s), selStart: s + 5, selEnd: s + 5 };
        })}><Minus className="w-3.5 h-3.5" /></button>
      </div>
      {/* 编辑区（可选分栏预览） */}
      <div className="flex min-h-0 flex-1">
        <textarea
          ref={taRef}
          value={active.content}
          onChange={(e) => {
            if (activePath) setContent(activePath, e.target.value);
            requestAnimationFrame(updateCursor);
          }}
          onClick={updateCursor}
          onKeyUp={updateCursor}
          onSelect={updateCursor}
          onKeyDown={(e) => {
            // Tab 插入两个空格而不是移动焦点
            if (e.key === "Tab") {
              e.preventDefault();
              const ta = e.currentTarget;
              const { selectionStart: s, selectionEnd: ee, value } = ta;
              const next = value.slice(0, s) + "  " + value.slice(ee);
              setContent(activePath!, next);
              requestAnimationFrame(() => { ta.setSelectionRange(s + 2, s + 2); });
            }
          }}
          spellCheck={false}
          className={`min-h-0 flex-1 resize-none bg-transparent px-6 py-5 text-[12px] leading-6 text-slate-200 font-mono outline-none ${split ? "border-r border-white/10 max-w-[50%]" : ""}`}
          placeholder="开始编辑 Markdown…"
        />
        {split && (
          <div className="min-h-0 flex-1 overflow-y-auto">
            {preview}
          </div>
        )}
      </div>
      {/* 状态栏：光标位置 + 实时字数统计 */}
      <div className="flex-shrink-0 flex items-center gap-3 px-3 py-1 border-t border-white/5 bg-white/[0.02] text-[10px] text-slate-500 select-none">
        <span className="font-mono">Ln {cursor.line}, Col {cursor.col}</span>
        <span className="w-px h-3 bg-white/10" />
        <span title="总字符数">{stats.chars} 字符</span>
        <span title="不含空白字符数">{stats.noSpace} 非空</span>
        <span title="中文字数">{stats.cjk} 中文</span>
        <span title="英文单词数">{stats.words} 词</span>
        <span>{stats.lines} 行</span>
        <div className="flex-1" />
        {active.dirty ? (
          <span className="text-amber-300/80">未保存</span>
        ) : (
          <span className="text-emerald-400/80">已保存</span>
        )}
        <span className="font-mono">UTF-8</span>
      </div>
    </div>
  );

  return (
    <div className="h-full flex flex-col min-h-0 select-none">
      {/* 顶部工具栏 */}
      <div className="flex-shrink-0 flex items-center gap-2 px-4 py-2 border-b border-white/5">
        <button onClick={pickFiles} className={btn}>
          <FolderOpen className="w-3.5 h-3.5" />
          打开文件
        </button>
        <button onClick={() => void openDirectory()} className={btn} title="选择目录，列出所有 Markdown 文件">
          <Folder className="w-3.5 h-3.5" />
          打开目录
        </button>
        <button onClick={() => void createNewFile()} className={btn} title="在当前目录下新建 .md 文件">
          <FilePlus className="w-3.5 h-3.5" />
          新建文件
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
        <div className="w-px h-5 bg-white/10 mx-1" />
        {active && (
          <>
            <button onClick={() => setMode(mode === "view" ? "edit" : "view")}
              className={`${btn} ${mode === "edit" ? "!bg-emerald-600/25 !text-emerald-200" : ""}`}
              title={mode === "view" ? "切换到编辑" : "切换到预览"}>
              {mode === "view" ? <Pencil className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
              {mode === "view" ? "编辑" : "预览"}
            </button>
            {mode === "edit" && (
              <button onClick={() => setSplit(!split)}
                className={`${btn} ${split ? "!bg-emerald-600/25 !text-emerald-200" : ""}`} title="编辑+预览分栏">
                <Columns className="w-3.5 h-3.5" />
                分栏
              </button>
            )}
            <button onClick={() => void saveActive()} disabled={!active.dirty || saving} className={btn} title="保存 (Ctrl+S)">
              {saving ? <LoaderCircle /> : active.dirty ? <Save className="w-3.5 h-3.5" /> : <Check className="w-3.5 h-3.5 text-emerald-400" />}
              {active.dirty ? (saving ? "保存中…" : "保存") : "已保存"}
            </button>
          </>
        )}
        <div className="flex-1" />
        <button onClick={() => setShowAssoc((s) => !s)} className={btn} title=".md 文件关联设置">
          <FileText className="w-3.5 h-3.5" />
          文件关联{assoc ? (assocAll ? " ✓" : "") : ""}
        </button>
        {notice && (
          <div className="flex items-center gap-1 text-[10px] text-amber-300">
            <AlertCircle className="w-3 h-3" />
            {notice}
          </div>
        )}
        {active && (
          <span className="text-[10px] text-slate-500 truncate max-w-[30%]" title={active.path}>
            {active.path}
          </span>
        )}
      </div>

      {/* 文件关联面板 */}
      {showAssoc && (
        <div className="flex-shrink-0 px-4 py-2.5 border-b border-white/5 bg-white/[0.02] space-y-2">
          <div className="text-[10px] text-slate-400">
            将 <span className="font-mono text-slate-200">.md</span> / <span className="font-mono text-slate-200">.markdown</span> 注册为系统打开方式（用户级注册表，无需管理员），并加入右键「打开方式」列表。
          </div>
          <div className="flex items-center gap-2">
            <button onClick={() => void toggleAssoc(true)} className={btn} disabled={assocAll}>
              <Check className="w-3.5 h-3.5" />
              {assocAll ? "已注册" : "注册关联"}
            </button>
            <button onClick={() => void toggleAssoc(false)} className={btn} disabled={assoc ? !assoc.md && !assoc.markdown : false}>
              <X className="w-3.5 h-3.5" />
              解除关联
            </button>
            {assoc && (
              <span className="text-[10px] text-slate-500 font-mono truncate">
                .md: {assoc.md ? "✓" : "✗"} · .markdown: {assoc.markdown ? "✓" : "✗"} · {assoc.exePath}
              </span>
            )}
          </div>
        </div>
      )}

      {/* 选项卡条 */}
      {tabs.length > 0 && (
        <div className="flex-shrink-0 flex items-stretch gap-0.5 px-2 pt-1.5 border-b border-white/5 overflow-x-auto">
          {tabs.map((t) => (
            <div
              key={t.path}
              onClick={() => { setActivePath(t.path); setMode("view"); }}
              title={t.path}
              className={`group flex items-center gap-1.5 px-2.5 py-1.5 rounded-t-md text-[11px] whitespace-nowrap cursor-pointer transition-colors max-w-[200px] ${
                t.path === activePath
                  ? "bg-emerald-600/20 text-emerald-200 border-b-2 border-emerald-500"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/5 border-b-2 border-transparent"
              }`}
            >
              <FileText className="w-3 h-3 flex-shrink-0" />
              <span className="truncate">{t.name}{t.dirty ? " •" : ""}</span>
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
                  {siblingRoot ? siblingRoot.split(/[\\/]/).pop() : "文件列表"}
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

        {/* 中间：正文 / 编辑器 */}
        <div ref={contentRef} className="flex-1 min-w-0 flex flex-col overflow-y-auto select-text">
          {!active && (
            <div className="h-full flex flex-col items-center justify-center gap-3 text-slate-600">
              <FileText className="w-10 h-10 opacity-30" />
              <div className="text-[11px]">尚未打开文档</div>
              <div className="flex gap-2">
                <button onClick={pickFiles} className={btn}>
                  <FolderOpen className="w-3.5 h-3.5" />
                  打开文件
                </button>
                <button onClick={() => void openDirectory()} className={btn}>
                  <Folder className="w-3.5 h-3.5" />
                  打开目录
                </button>
                <button onClick={() => void createNewFile()} className={btn}>
                  <FilePlus className="w-3.5 h-3.5" />
                  新建文件
                </button>
              </div>
              <div className="text-[10px] text-slate-700 max-w-xs text-center leading-relaxed">
                默认预览；点「编辑」进入编辑器（自动保存、撤销/重做、格式工具栏、分栏实时预览）。
                支持打开目录列出所有文件、新建 .md 文件。可在「文件关联」中把 .md 注册为系统打开方式。
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
            mode === "view" && !split ? preview : editor
          )}
        </div>

        {/* 右侧：大纲 */}
        {active && headings.length > 0 && (
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

function LoaderCircle() {
  return <span className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-slate-400 border-t-transparent" />;
}

/** 代码块：语言标签 + 语法高亮 + 一键复制 */
function CodeBlock({ lang, children }: { lang: string; children: React.ReactNode }) {
  const [copied, setCopied] = useState(false);
  const text = typeof children === "string" ? children : String(children);

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    });
  }, [text]);

  // 用 hljs 高亮；语言未注册时退回自动检测，再退回纯文本转义
  const html = useMemo(() => {
    try {
      if (lang && hljs.getLanguage(lang)) {
        return hljs.highlight(text, { language: lang, ignoreIllegals: true }).value;
      }
      const auto = hljs.highlightAuto(text);
      if (auto.relevance > 0) return auto.value;
    } catch {
      /* fallthrough */
    }
    return text
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }, [lang, text]);

  return (
    <div className="relative my-3 rounded-lg overflow-hidden border border-white/10 bg-slate-900/80">
      <div className="flex items-center justify-between px-2.5 py-1 bg-slate-800/60 border-b border-white/5">
        <span className="text-[9px] font-mono text-slate-400 uppercase tracking-wide">
          {lang || "text"}
        </span>
        <button
          onClick={handleCopy}
          className="text-[9px] text-slate-500 hover:text-slate-200 transition-colors cursor-pointer"
        >
          {copied ? "✓ 已复制" : "复制"}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-[11px] leading-relaxed">
        <code
          className="font-mono hljs"
          style={{ background: "transparent", color: "#e2e8f0", padding: 0 }}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </pre>
    </div>
  );
}

/** hljs 深色主题样式（One Dark 风格，随应用主题） */
const hljsCss = `
.hljs{color:#abb2bf}
.hljs-comment,.hljs-quote{color:#5c6370;font-style:italic}
.hljs-doctag,.hljs-keyword,.hljs-formula{color:#c678dd}
.hljs-section,.hljs-name,.hljs-selector-tag,.hljs-deletion,.hljs-subst{color:#e06c75}
.hljs-literal{color:#56b6c2}
.hljs-string,.hljs-regexp,.hljs-addition,.hljs-attribute,.hljs-meta .hljs-string{color:#98c379}
.hljs-attr,.hljs-variable,.hljs-template-variable,.hljs-type,.hljs-selector-class,.hljs-selector-attr,.hljs-selector-pseudo,.hljs-number{color:#d19a66}
.hljs-symbol,.hljs-bullet,.hljs-link,.hljs-meta,.hljs-selector-id,.hljs-title{color:#61aeee}
.hljs-built_in,.hljs-title.class_,.hljs-class .hljs-title{color:#e6c07b}
.hljs-emphasis{font-style:italic}
.hljs-strong{font-weight:bold}
.hljs-link{text-decoration:underline}
`;
// 注入一次 hljs 主题（模块级，避免每次渲染都重建 <style>）
if (typeof document !== "undefined" && !document.getElementById("hljs-theme")) {
  const style = document.createElement("style");
  style.id = "hljs-theme";
  style.textContent = hljsCss;
  document.head.appendChild(style);
}
