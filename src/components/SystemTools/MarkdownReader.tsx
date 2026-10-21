import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { theamedConfirm } from "../shared/ThemedAlert";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { marked } from "marked";
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
  Download, Printer, ChevronRight, ChevronDown, ArrowUp, ArrowDown, Replace, History,
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
  /** 源码行号（0 基），编辑模式下跳转用 */
  line: number;
}

/** 最近打开的文件（localStorage 持久化） */
interface RecentFile {
  path: string;
  name: string;
  ts: number;
}

/** 文件关联状态（后端 markdown_assoc_status） */
interface AssocStatus {
  md: boolean;
  markdown: boolean;
  exePath: string;
}

const MD_RECENT_KEY = "any_version_markdown_recent";
const MD_RECENT_FILES_KEY = "any_version_markdown_recent_files";
/** 预览图片的 data URL 缓存：同一文档反复渲染不重复读盘 */
const imageCache = new Map<string, Promise<string | null>>();

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

const escapeRegExp = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** 取路径的父目录（兼容 Windows 反斜杠与正斜杠） */
const dirOf = (path: string): string => {
  const idx = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return idx > 0 ? path.slice(0, idx) : path;
};

/** 转为 file:// URL，供「导出 HTML」里引用本地图片 */
const toFileUrl = (path: string): string => {
  let s = path.replace(/\\/g, "/");
  if (!s.startsWith("/")) s = `/${s}`;
  return `file://${encodeURI(s).replace(/#/g, "%23").replace(/\?/g, "%3F")}`;
};

/** 导出/打印用 HTML 模板：深色正文改为浅色，保证打印与外部阅读都清晰 */
const EXPORT_STYLE = `
  :root{color-scheme:light}
  body{margin:0;padding:32px;font-family:-apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC","Microsoft YaHei",sans-serif;
    color:#1f2937;background:#fff;line-height:1.75;font-size:15px}
  main{max-width:860px;margin:0 auto}
  h1,h2,h3,h4,h5,h6{line-height:1.35;margin:1.4em 0 .6em;font-weight:700;color:#111827}
  h1{font-size:2em;border-bottom:1px solid #e5e7eb;padding-bottom:.3em}
  h2{font-size:1.5em;border-bottom:1px solid #f1f5f9;padding-bottom:.25em}
  h3{font-size:1.25em} h4{font-size:1.1em}
  p{margin:.75em 0} ul,ol{padding-left:1.6em;margin:.75em 0} li{margin:.25em 0}
  blockquote{margin:1em 0;padding:.25em 1em;border-left:3px solid #10b981;color:#4b5563;background:#f9fafb}
  code{font-family:ui-monospace,SFMono-Regular,Consolas,monospace;background:#f3f4f6;border-radius:4px;padding:.1em .35em;font-size:.9em}
  pre{background:#0f172a;color:#e2e8f0;border-radius:8px;padding:14px;overflow:auto;margin:1em 0}
  pre code{background:transparent;color:inherit;padding:0}
  table{border-collapse:collapse;width:100%;margin:1em 0;font-size:.92em}
  th,td{border:1px solid #e5e7eb;padding:6px 10px;text-align:left} th{background:#f9fafb}
  img{max-width:100%;border-radius:6px}
  hr{border:0;border-top:1px solid #e5e7eb;margin:2em 0}
  a{color:#2563eb}
  .task-list-item{list-style:none;margin-left:-1.4em}
  input[type=checkbox]{margin-right:.45em}
  @media print{body{padding:0} main{max-width:none} pre{white-space:pre-wrap}}
`;

/** 用 marked 生成可独立打开的 HTML：本地图片优先内联为 data URL，找不到再退回 file:// */
async function buildExportHtml(docPath: string, content: string): Promise<string> {
  const rendered = marked.parse(content, { gfm: true, async: false }) as string;
  const parsed = new DOMParser().parseFromString(`<div id="md-root">${rendered}</div>`, "text/html");
  const root = parsed.getElementById("md-root");
  const dir = dirOf(docPath);
  const imgs = Array.from(root?.querySelectorAll("img") ?? []);
  await Promise.all(imgs.map(async (img) => {
    const src = img.getAttribute("src") || "";
    if (!src || isExternal(src) || src.startsWith("//")) return;
    let rel = src;
    try { rel = decodeURIComponent(src); } catch { /* 保留原样 */ }
    const fallback = toFileUrl(`${dir}/${rel}`);
    try {
      const dataUrl = await invoke<string | null>("read_local_image", { from: docPath, src });
      img.setAttribute("src", dataUrl || fallback);
    } catch {
      img.setAttribute("src", fallback);
    }
  }));
  const title = (content.match(/^#\s+(.+)$/m)?.[1] || docPath.split(/[\\/]/).pop() || "Markdown").trim();
  const body = root?.innerHTML ?? rendered;
  return `<!DOCTYPE html><html lang="zh-CN"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>${title.replace(/</g, "&lt;")}</title><style>${EXPORT_STYLE}</style></head>
<body><main>${body}</main></body></html>`;
}

/**
 * 在光标处回车时延续列表/引用前缀；空列表项回车则退出列表。
 * 返回 null 表示不处理（普通换行）。
 */
function continueListOnEnter(ta: HTMLTextAreaElement): { text: string; caret: number } | null {
  const { value, selectionStart: s, selectionEnd: e } = ta;
  const lineStart = value.lastIndexOf("\n", s - 1) + 1;
  const lineEndIdx = value.indexOf("\n", s);
  const lineEnd = lineEndIdx === -1 ? value.length : lineEndIdx;
  const line = value.slice(lineStart, lineEnd);
  const m = /^(\s*)([-*+]\s+\[[ xX]\]\s+|[-*+]\s+|(\d+)\.\s+|>\s?)(.*)$/.exec(line);
  if (!m) return null;
  const [, indent, prefix, num, rest] = m;

  // 空条目（只有前缀）→ 清掉前缀，结束列表
  if (!rest.trim()) {
    const next = value.slice(0, lineStart) + indent + value.slice(lineEnd);
    return { text: next, caret: lineStart + indent.length };
  }

  let nextPrefix = prefix;
  if (num) nextPrefix = `${Number.parseInt(num, 10) + 1}. `;
  else if (/\[[ xX]\]/.test(prefix)) nextPrefix = prefix.replace(/\[[ xX]\]/, "[ ]");
  const insert = `\n${indent}${nextPrefix}`;
  return { text: value.slice(0, s) + insert + value.slice(e), caret: s + insert.length };
}

/** 把预览 DOM 里的文本按查询词切成 Range，供 CSS Custom Highlight API 使用 */
function collectTextRanges(root: HTMLElement | null, query: string): Range[] {
  const ranges: Range[] = [];
  if (!root || !query) return ranges;
  const needle = query.toLowerCase();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) =>
      n.parentElement?.closest("script,style") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
  });
  let node = walker.nextNode();
  while (node && ranges.length < 500) {
    const text = node.textContent || "";
    const hay = text.toLowerCase();
    let from = hay.indexOf(needle);
    while (from !== -1) {
      const r = document.createRange();
      r.setStart(node, from);
      r.setEnd(node, from + needle.length);
      ranges.push(r);
      if (ranges.length >= 500) break;
      from = hay.indexOf(needle, from + needle.length);
    }
    node = walker.nextNode();
  }
  return ranges;
}

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
  const { t } = useTranslation();
  const [mode, setMode] = useState<"view" | "edit">("view"); // 默认预览
  const [split, setSplit] = useState(false);                 // 编辑+预览分栏
  const [saving, setSaving] = useState(false);
  const [assoc, setAssoc] = useState<AssocStatus | null>(null);
  const [showAssoc, setShowAssoc] = useState(false);
  const [cursor, setCursor] = useState({ line: 1, col: 1 }); // 光标行列（编辑态状态栏）
  const [recent, setRecent] = useState<RecentFile[]>([]);     // 最近打开
  const [activeHeading, setActiveHeading] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set()); // 大纲折叠
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState("");
  const [findReplace, setFindReplace] = useState("");
  const [findIndex, setFindIndex] = useState(0);
  const contentRef = useRef<HTMLDivElement>(null);
  const previewPaneRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);
  const findInputRef = useRef<HTMLInputElement>(null);
  const saveTimer = useRef<number | null>(null);
  const lastUndoAt = useRef<Map<string, number>>(new Map());
  const openingRef = useRef<Set<string>>(new Set());
  const splitSyncRef = useRef<"editor" | "preview" | null>(null);
  const restoredRef = useRef(false);

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

  // 切换标签时重置大纲高亮，避免沿用上一篇文档的标题 id
  useEffect(() => { setActiveHeading(null); }, [activePath]);

  // 记住打开的 markdown 文件（存本地），模块卸载重挂载时恢复已打开的标签与侧栏目录。
  useEffect(() => {
    // 切走再切回时组件状态还在（Activity 只销毁 Effects），不要重复恢复把当前标签打回原位
    if (restoredRef.current) return;
    restoredRef.current = true;
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
        // 恢复上次激活的标签（openPath 会逐个激活，最后再切回来）
        if (alive && typeof saved?.activePath === "string" && paths.includes(saved.activePath)) {
          setActivePath(saved.activePath);
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

  // 载入最近打开记录（空态与菜单用）
  useEffect(() => {
    try {
      const saved = JSON.parse(localStorage.getItem(MD_RECENT_FILES_KEY) || "null");
      if (Array.isArray(saved)) {
        setRecent(
          saved.filter(
            (x: unknown): x is RecentFile =>
              !!x && typeof (x as RecentFile).path === "string" && typeof (x as RecentFile).name === "string"
          )
        );
      }
    } catch {
      // 忽略损坏的本地记录
    }
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
      // 并发打开同一路径时（恢复标签 + 系统打开同时到达），只允许一个读取流程
      if (openingRef.current.has(path)) return;
      openingRef.current.add(path);

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
      if (existed) {
        openingRef.current.delete(path);
        return;
      }

      // 写入「最近打开」，最多保留 12 条
      try {
        const sep = path.includes("\\") ? "\\" : "/";
        const name = path.slice(path.lastIndexOf(sep) + 1);
        setRecent((prev) => {
          const next = [{ path, name, ts: Date.now() }, ...prev.filter((x) => x.path !== path)].slice(0, 12);
          localStorage.setItem(MD_RECENT_FILES_KEY, JSON.stringify(next));
          return next;
        });
      } catch {
        // 本地记录写入失败不影响打开
      }

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
      } finally {
        openingRef.current.delete(path);
      }
    },
    [scanSiblings]
  );

  /** 系统「打开方式」/ 单实例传入的 .md：模块挂载或收到事件时取走待打开队列 */
  useEffect(() => {
    let alive = true;
    const drain = async () => {
      try {
        const paths = await invoke<string[]>("take_pending_markdown_files");
        for (const p of paths) {
          if (!alive) return;
          await openPath(p);
        }
      } catch {
        // 后端不可用时忽略
      }
    };
    void drain();
    let unlisten: (() => void) | undefined;
    listen("markdown-open-file", () => { void drain(); }).then((fn) => {
      if (alive) unlisten = fn;
      else fn();
    }).catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [openPath]);

  /** 打开目录：列出目录下所有 markdown 文件，不自动打开文件 */
  const openDirectory = useCallback(async () => {
    try {
      const dir = await open({
        directory: true,
        title: t("mdreader.selectDirTitle"),
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
      flashNotice(t("mdreader.flashNoDir"));
      return;
    }
    const dir = active?.path ? active.path.slice(0, active.path.lastIndexOf(active.path.includes("\\") ? "\\" : "/")) : baseDir;
    const name = window.prompt(t("mdreader.promptFileName"));
    if (!name?.trim()) return;
    const sep = dir.includes("\\") ? "\\" : "/";
    const filePath = `${dir}${sep}${name.trim()}.md`;
    try {
      await invoke("write_text_file", { path: filePath, content: `# ${name.trim()}\n\n` });
      await openPath(filePath);
      flashNotice(t("mdreader.createdOk", { name: name.trim() }));
    } catch (e) {
      flashNotice(t("mdreader.createFail", { err: String(e) }));
    }
  }, [siblingRoot, active, openPath, flashNotice]);

  /** 文件选择对话框，支持一次选多个 */
  const pickFiles = useCallback(async () => {
    try {
      const selected = await open({
        multiple: true,
        title: t("mdreader.openMdTitle"),
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
    async (path: string, e?: React.MouseEvent) => {
      e?.stopPropagation();
      // 未保存确认要 await，所以不能在 setTabs 的 updater 里问（updater 必须同步返回）
      const tab = tabs.find((x) => x.path === path);
      if (tab?.dirty && !(await theamedConfirm(t("mdreader.dirtyCloseConfirm", { name: tab.name })))) return;
      setTabs((prev) => {
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
    [activePath, tabs]
  );

  /** 编辑器内容变更（带撤销栈；连续输入按 800ms 合并成一次撤销） */
  const setContent = useCallback((path: string, next: string, pushUndo = true) => {
    const now = Date.now();
    const last = lastUndoAt.current.get(path) ?? 0;
    lastUndoAt.current.set(path, now);
    const mergeTyping = now - last < 800;
    setTabs((prev) =>
      prev.map((t) => {
        if (t.path !== path) return t;
        // 只有小幅增删（逐字输入）才并入上一次撤销点，批量替换仍单独入栈
        const smallEdit = Math.abs(next.length - t.content.length) <= 2;
        const undo = pushUndo && !(mergeTyping && smallEdit) ? [...t.undo.slice(-200), t.content] : t.undo;
        return { ...t, content: next, undo, redo: [], dirty: next !== t.savedContent };
      })
    );
  }, []);

  /** 撤销/重做 */
  const undo = useCallback(() => {
    lastUndoAt.current.delete(activePath ?? "");
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
    lastUndoAt.current.delete(activePath ?? "");
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
      flashNotice(t("mdreader.saved"));
    } catch (e) {
      flashNotice(t("mdreader.saveFail", { err: String(e) }));
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
    if (active.dirty && !(await theamedConfirm(t("mdreader.reloadDirtyConfirm")))) return;
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
      if (isExternal(href)) {
        // 交给系统默认浏览器，避免 WebView 内导航把整个应用带走
        e.preventDefault();
        void openUrl(href).catch(() => flashNotice(href));
        return;
      }
      e.preventDefault();
      try {
        const resolved = await invoke<string | null>("resolve_markdown_link", {
          from: active.path,
          href,
        });
        if (resolved) {
          await openPath(resolved);
        } else {
          flashNotice(t("mdreader.linkTargetNotFound", { href }));
        }
      } catch (err) {
        flashNotice(String(err));
      }
    },
    [active, openPath, flashNotice]
  );

  /** 从正文提取标题生成大纲（跳过围栏代码块内的 # 行；重复标题自动加序号） */
  const headings = useMemo<Heading[]>(() => {
    if (!active?.content) return [];
    const out: Heading[] = [];
    const idCount = new Map<string, number>();
    let inFence = false;
    active.content.split(/\r?\n/).forEach((line, idx) => {
      if (/^\s*(```|~~~)/.test(line)) {
        inFence = !inFence;
        return;
      }
      if (inFence) return;
      const m = /^(#{1,6})\s+(.*)$/.exec(line);
      if (!m) return;
      const text = m[2].replace(/[*_`]/g, "").trim();
      if (!text) return;
      const base = slugify(text) || "section";
      const n = idCount.get(base) ?? 0;
      idCount.set(base, n + 1);
      out.push({ id: n === 0 ? base : `${base}-${n}`, text, level: m[1].length, line: idx });
    });
    return out;
  }, [active?.content]);

  /** hast 节点的行号（1 基）→ 标题 id，供预览标题锚点精确复用（含重复标题去重） */
  const headingIdByLine = useMemo(() => {
    const map = new Map<number, string>();
    headings.forEach((h) => map.set(h.line + 1, h.id));
    return map;
  }, [headings]);

  /** 大纲行：是否含子级、是否被折叠隐藏 */
  const outlineRows = useMemo(() => {
    const rows: { h: Heading; hasChildren: boolean; hidden: boolean }[] = [];
    const stack: Heading[] = [];
    headings.forEach((h, i) => {
      while (stack.length && stack[stack.length - 1].level >= h.level) stack.pop();
      const hidden = stack.some((a) => collapsed.has(a.id));
      const next = headings[i + 1];
      const hasChildren = !!next && next.level > h.level;
      if (!hidden) stack.push(h);
      rows.push({ h, hasChildren, hidden });
    });
    return rows;
  }, [headings, collapsed]);

  /** 预览区是否可见（视图模式或分栏） */
  const previewVisible = mode === "view" || (mode === "edit" && split);

  /** 查找匹配偏移（源码层），最多 500 处 */
  const findMatches = useMemo(() => {
    if (!findOpen || !findQuery || !active?.content) return [];
    const hay = active.content.toLowerCase();
    const needle = findQuery.toLowerCase();
    const out: number[] = [];
    let from = hay.indexOf(needle);
    while (from !== -1 && out.length < 500) {
      out.push(from);
      from = hay.indexOf(needle, from + Math.max(needle.length, 1));
    }
    return out;
  }, [findOpen, findQuery, active?.content]);

  // 匹配数变化时把当前索引收敛到有效范围
  useEffect(() => {
    if (findIndex >= findMatches.length) setFindIndex(0);
  }, [findMatches.length, findIndex]);

  /** 勾选/取消预览里的任务项（按源码行回写） */
  const toggleTaskAtLine = useCallback((line1: number) => {
    if (!active) return;
    const lines = active.content.split("\n");
    const idx = line1 - 1;
    if (idx < 0 || idx >= lines.length) return;
    const line = lines[idx];
    const next = /\[ \]/.test(line)
      ? line.replace(/\[ \]/, "[x]")
      : line.replace(/\[[xX]\]/, "[ ]");
    if (next === line) return;
    lines[idx] = next;
    setContent(active.path, lines.join("\n"));
  }, [active, setContent]);

  /** 跳动到第 delta 个匹配（相对当前位置；-1 表示尚未定位） */
  const jumpToMatch = useCallback((delta: number) => {
    const total = findMatches.length;
    if (!total || !active) return;
    let next: number;
    if (findIndex < 0) next = delta >= 0 ? 0 : total - 1;
    else next = ((findIndex + delta) % total + total) % total;
    setFindIndex(next);
    if (mode !== "edit") return;
    const ta = taRef.current;
    if (!ta) return;
    const off = findMatches[next];
    ta.focus();
    ta.setSelectionRange(off, off + findQuery.length);
    const lh = Number.parseFloat(getComputedStyle(ta).lineHeight) || 24;
    const line = ta.value.slice(0, off).split("\n").length - 1;
    ta.scrollTop = Math.max(0, line * lh - ta.clientHeight / 2 + lh);
  }, [active, findIndex, findMatches, findQuery, mode]);

  /** 替换当前匹配 */
  const replaceCurrent = useCallback(() => {
    if (!active || !findMatches.length) return;
    const idx = findIndex < 0 ? 0 : Math.min(findIndex, findMatches.length - 1);
    const off = findMatches[idx];
    setContent(active.path, active.content.slice(0, off) + findReplace + active.content.slice(off + findQuery.length));
  }, [active, findMatches, findIndex, findQuery, findReplace, setContent]);

  /** 全部替换 */
  const replaceAll = useCallback(() => {
    if (!active || !findQuery) return;
    const count = findMatches.length;
    if (!count) return;
    const re = new RegExp(escapeRegExp(findQuery), "gi");
    setContent(active.path, active.content.replace(re, () => findReplace));
    flashNotice(t("mdreader.replacedCount", { n: count }));
  }, [active, findQuery, findMatches.length, findReplace, setContent, flashNotice, t]);

  // 预览态查找高亮（CSS Custom Highlight API；不支持时只做定位滚动）
  useEffect(() => {
    const api = (window as unknown as { CSS?: { highlights?: Map<string, unknown> }; Highlight?: new (...r: Range[]) => unknown });
    try { api.CSS?.highlights?.delete("md-find"); } catch { /* 忽略 */ }
    if (!findOpen || !findQuery || !previewVisible || !active || active.loading) return;
    const ranges = collectTextRanges(contentRef.current, findQuery);
    if (ranges.length && api.CSS?.highlights && api.Highlight) {
      try { api.CSS.highlights.set("md-find", new api.Highlight(...ranges)); } catch { /* 忽略 */ }
    }
    const cur = findIndex >= 0 ? ranges[Math.min(findIndex, ranges.length - 1)] : undefined;
    cur?.startContainer.parentElement?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [findOpen, findQuery, findIndex, active?.content, active?.loading, previewVisible]);

  /** 滚动时更新大纲高亮 */
  const updateActiveHeading = useCallback(() => {
    if (!headings.length) return;
    if (mode === "edit" && !split) {
      const ta = taRef.current;
      if (!ta) return;
      const lh = Number.parseFloat(getComputedStyle(ta).lineHeight) || 24;
      const firstLine = Math.floor(ta.scrollTop / lh) + 1;
      let cur: Heading | null = null;
      for (const h of headings) {
        if (h.line <= firstLine) cur = h;
        else break;
      }
      setActiveHeading(cur?.id ?? null);
      return;
    }
    const container = split ? previewPaneRef.current : contentRef.current;
    if (!container) return;
    const threshold = container.getBoundingClientRect().top + 64;
    let cur: Heading | null = null;
    for (const h of headings) {
      const el = container.querySelector<HTMLElement>(`#${CSS.escape(h.id)}`);
      if (el && el.getBoundingClientRect().top <= threshold) cur = h;
    }
    setActiveHeading(cur?.id ?? null);
  }, [headings, mode, split]);

  /** 编辑/预览双栏滚动联动（按比例同步） */
  const syncScroll = useCallback((source: "editor" | "preview") => {
    if (splitSyncRef.current && splitSyncRef.current !== source) return;
    const ta = taRef.current;
    const pv = previewPaneRef.current;
    if (!ta || !pv) return;
    splitSyncRef.current = source;
    if (source === "editor") {
      const max = ta.scrollHeight - ta.clientHeight;
      const ratio = max > 0 ? ta.scrollTop / max : 0;
      pv.scrollTop = ratio * (pv.scrollHeight - pv.clientHeight);
    } else {
      const max = pv.scrollHeight - pv.clientHeight;
      const ratio = max > 0 ? pv.scrollTop / max : 0;
      ta.scrollTop = ratio * (ta.scrollHeight - ta.clientHeight);
    }
    requestAnimationFrame(() => { splitSyncRef.current = null; });
  }, []);

  /** 点击大纲跳转：编辑态定位到源码行，预览态滚动到标题 */
  const jumpToHeading = useCallback((h: Heading) => {
    setActiveHeading(h.id);
    splitSyncRef.current = "preview";
    if (mode === "edit" || split) {
      const ta = taRef.current;
      if (ta) {
        const lh = Number.parseFloat(getComputedStyle(ta).lineHeight) || 24;
        ta.scrollTop = Math.max(0, h.line * lh - 24);
        if (mode === "edit") {
          const lines = ta.value.split("\n");
          let off = 0;
          for (let i = 0; i < h.line && i < lines.length; i++) off += lines[i].length + 1;
          ta.focus();
          ta.setSelectionRange(off, off);
        }
      }
    }
    if (previewVisible) {
      const el = contentRef.current?.querySelector<HTMLElement>(`#${CSS.escape(h.id)}`);
      el?.scrollIntoView({ behavior: "smooth", block: "start" });
    }
    window.setTimeout(() => { splitSyncRef.current = null; }, 160);
  }, [mode, split, previewVisible]);

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

  // 快捷键：Ctrl+S 保存 / Ctrl+W 关闭标签 / Ctrl+F 查找 / 编辑态撤销重做与 B、I
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setFindOpen(true);
        requestAnimationFrame(() => { findInputRef.current?.focus(); findInputRef.current?.select(); });
        return;
      }
      if (e.key === "Escape" && findOpen) {
        setFindOpen(false);
        return;
      }
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
        if (e.ctrlKey && e.key.toLowerCase() === "z") { e.preventDefault(); undo(); return; }
        if (e.ctrlKey && (e.key.toLowerCase() === "y" || (e.shiftKey && e.key.toLowerCase() === "z"))) { e.preventDefault(); redo(); return; }
        if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "b") {
          e.preventDefault();
          applyTransform((ta) => wrapSelection(ta, "**", "**", t("mdreader.boldPh")));
          return;
        }
        if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "i") {
          e.preventDefault();
          applyTransform((ta) => wrapSelection(ta, "*", "*", t("mdreader.italicPh")));
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activePath, closeTab, active, saveActive, mode, undo, redo, findOpen, applyTransform, t]);

  /** 导出为独立 HTML 文件 */
  const exportHtml = useCallback(async () => {
    if (!active) return;
    try {
      const target = await save({
        title: t("mdreader.exportHtmlTitle"),
        defaultPath: active.path.replace(/\.(md|markdown|mdx|mdown)$/i, ".html"),
        filters: [{ name: "HTML", extensions: ["html"] }],
      });
      if (!target) return;
      const html = await buildExportHtml(active.path, active.content);
      await invoke("write_text_file", { path: target, content: html });
      flashNotice(t("mdreader.exportedOk", { path: target }));
    } catch (e) {
      flashNotice(t("mdreader.exportFail", { err: String(e) }));
    }
  }, [active, flashNotice, t]);

  /** 调用系统打印（用隐藏 iframe 渲染导出模板，避免把应用界面一起打出来） */
  const printActive = useCallback(() => {
    if (!active) return;
    void (async () => {
      const html = await buildExportHtml(active.path, active.content);
      const frame = document.createElement("iframe");
      frame.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0;opacity:0;pointer-events:none";
      document.body.appendChild(frame);
      frame.onload = () => {
        try {
          frame.contentWindow?.focus();
          frame.contentWindow?.print();
        } catch {
          flashNotice(t("mdreader.printFail"));
        }
        window.setTimeout(() => frame.remove(), 500);
      };
      const doc = frame.contentDocument;
      if (!doc) {
        frame.remove();
        flashNotice(t("mdreader.printFail"));
        return;
      }
      doc.open();
      doc.write(html);
      doc.close();
    })().catch(() => flashNotice(t("mdreader.printFail")));
  }, [active, flashNotice, t]);

  const btn =
    "flex items-center gap-1 rounded-md px-2 py-1 text-caption font-medium text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed";
  const tbtn =
    "inline-flex items-center justify-center h-7 w-7 rounded-md text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-30";
  const assocAll = assoc ? assoc.md && assoc.markdown : false;

  /** 带锚点 id 的标题渲染器（用 hast 行号对齐大纲 id，重复标题也不会撞锚点） */
  const heading = (level: number, cls: string) =>
    function H({ children, node }: { children?: React.ReactNode; node?: { position?: { start?: { line?: number } } } }) {
      const text = React.Children.toArray(children)
        .map((c) => (typeof c === "string" ? c : ""))
        .join("");
      const line = node?.position?.start?.line;
      const id = (line !== undefined ? headingIdByLine.get(line) : undefined) || slugify(text);
      const Tag = `h${level}` as keyof React.JSX.IntrinsicElements;
      return React.createElement(Tag, { id, className: cls }, children);
    };

  const preview = active && !active.loading && !active.error && (
    <div className="px-6 py-5 max-w-4xl mx-auto text-body leading-7 text-slate-200 break-words">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={{
          h1: heading(1, "text-xl font-bold text-slate-100 mt-6 mb-3 pb-1.5 border-b border-white/10 first:mt-0"),
          h2: heading(2, "text-lg font-bold text-slate-100 mt-5 mb-2.5 pb-1 border-b border-white/5 first:mt-0"),
          h3: heading(3, "text-base font-bold text-slate-200 mt-4 mb-2 first:mt-0"),
          h4: heading(4, "text-sm font-bold text-slate-200 mt-3 mb-1.5 first:mt-0"),
          h5: heading(5, "text-body font-semibold text-slate-300 mt-3 mb-1 first:mt-0"),
          h6: heading(6, "text-caption font-semibold text-slate-400 mt-3 mb-1 first:mt-0"),
          p: ({ children }) => <p className="my-2.5 first:mt-0 last:mb-0">{children}</p>,
          ul: ({ children }) => <ul className="list-disc my-2.5 space-y-1 pl-6">{children}</ul>,
          ol: ({ children }) => <ol className="list-decimal my-2.5 space-y-1 pl-6">{children}</ol>,
          li: ({ children, node }) => (
            <li className="leading-7" data-line={node?.position?.start?.line}>
              {children}
            </li>
          ),
          input: ({ checked }) => (
            <input
              type="checkbox"
              checked={!!checked}
              onChange={(e) => {
                // 任务项所在行由 li 上的 data-line 标记，点击即可回写源码
                const line = e.currentTarget.closest("li")?.getAttribute("data-line");
                if (line) toggleTaskAtLine(Number(line));
              }}
              className="mr-1.5 align-middle w-3 h-3 rounded accent-emerald-500 cursor-pointer"
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
                <code className="px-1 py-0.5 rounded bg-slate-700/60 text-caption text-emerald-300 font-mono">
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
            <LocalImage docPath={active.path} src={typeof src === "string" ? src : ""} alt={alt || ""} />
          ),
          table: ({ children }) => (
            <div className="overflow-x-auto my-3 rounded border border-white/10">
              <table className="min-w-full text-caption">{children}</table>
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
        <button className={tbtn} title={t("mdreader.undoTitle")} disabled={active.undo.length === 0} onClick={undo}><Undo2 className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.redoTitle")} disabled={active.redo.length === 0} onClick={redo}><Redo2 className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title={t("mdreader.h1")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "#"))}><Heading1 className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.h2")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "##"))}><Heading2 className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.h3")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "###"))}><Heading3 className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title={t("mdreader.bold")} onClick={() => applyTransform((ta) => wrapSelection(ta, "**", "**", t("mdreader.boldPh")))}><Bold className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.italic")} onClick={() => applyTransform((ta) => wrapSelection(ta, "*", "*", t("mdreader.italicPh")))}><Italic className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.strike")} onClick={() => applyTransform((ta) => wrapSelection(ta, "~~", "~~", t("mdreader.strikePh")))}><Strikethrough className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.inlineCode")} onClick={() => applyTransform((ta) => wrapSelection(ta, "`", "`", t("mdreader.codePh")))}><Code className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title={t("mdreader.ul")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "-"))}><List className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.ol")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "1.", true))}><ListOrdered className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.task")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, "- [ ]"))}><ListChecks className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.quote")} onClick={() => applyTransform((ta) => toggleLinePrefix(ta, ">"))}><Quote className="w-3.5 h-3.5" /></button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        <button className={tbtn} title={t("mdreader.codeBlock")} onClick={() => applyTransform((ta) => {
          const { selectionStart: s, selectionEnd: e, value } = ta;
          const sel = value.slice(s, e) || "// code";
          const insert = `\n\`\`\`\n${sel}\n\`\`\`\n`;
          return { text: value.slice(0, s) + insert + value.slice(e), selStart: s + 4, selEnd: s + 4 + sel.length };
        })}><Square className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.link")} onClick={() => applyTransform((ta) => wrapSelection(ta, "[", "](https://)", t("mdreader.linkTextPh")))}><Link className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.table")} onClick={() => applyTransform((ta) => {
          const tpl = `\n| ${t("mdreader.col1")} | ${t("mdreader.col2")} | ${t("mdreader.col3")} |\n| --- | --- | --- |\n|  a  |  b  |  c  |\n`;
          const { selectionStart: s, value } = ta;
          return { text: value.slice(0, s) + tpl + value.slice(s), selStart: s + 3, selEnd: s + 3 };
        })}><Table className="w-3.5 h-3.5" /></button>
        <button className={tbtn} title={t("mdreader.hr")} onClick={() => applyTransform((ta) => {
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
          onScroll={() => {
            if (split) syncScroll("editor");
            updateActiveHeading();
          }}
          onKeyDown={(e) => {
            // Tab 插入两个空格而不是移动焦点
            if (e.key === "Tab") {
              e.preventDefault();
              const ta = e.currentTarget;
              const { selectionStart: s, selectionEnd: ee, value } = ta;
              const next = value.slice(0, s) + "  " + value.slice(ee);
              setContent(activePath!, next);
              requestAnimationFrame(() => { ta.setSelectionRange(s + 2, s + 2); });
              return;
            }
            // 回车延续列表 / 引用；空条目回车退出列表
            if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.altKey) {
              const ta = e.currentTarget;
              const res = continueListOnEnter(ta);
              if (res) {
                e.preventDefault();
                setContent(activePath!, res.text);
                requestAnimationFrame(() => {
                  ta.setSelectionRange(res.caret, res.caret);
                  updateCursor();
                });
              }
            }
          }}
          spellCheck={false}
          className={`min-h-0 flex-1 resize-none bg-transparent px-6 py-5 text-body leading-6 text-slate-200 font-mono outline-none ${split ? "border-r border-white/10 max-w-[50%]" : ""}`}
          placeholder={t("mdreader.editorPh")}
        />
        {split && (
          <div
            ref={previewPaneRef}
            onScroll={() => { syncScroll("preview"); updateActiveHeading(); }}
            className="min-h-0 flex-1 overflow-y-auto"
          >
            {preview}
          </div>
        )}
      </div>
      {/* 状态栏：光标位置 + 实时字数统计 */}
      <div className="flex-shrink-0 flex items-center gap-3 px-3 py-1 border-t border-white/5 bg-white/[0.02] text-tiny text-slate-500 select-none">
        <span className="font-mono">Ln {cursor.line}, Col {cursor.col}</span>
        <span className="w-px h-3 bg-white/10" />
        <span title={t("mdreader.charsTip")}>{t("mdreader.statsChars", { n: stats.chars })}</span>
        <span title={t("mdreader.noSpaceTip")}>{t("mdreader.statsNoSpace", { n: stats.noSpace })}</span>
        <span title={t("mdreader.cjkTip")}>{t("mdreader.statsCjk", { n: stats.cjk })}</span>
        <span title={t("mdreader.wordsTip")}>{t("mdreader.statsWords", { n: stats.words })}</span>
        <span>{t("mdreader.statsLines", { n: stats.lines })}</span>
        <div className="flex-1" />
        {active.dirty ? (
          <span className="text-amber-300/80">{t("mdreader.unsaved")}</span>
        ) : (
          <span className="text-emerald-400/80">{t("mdreader.saved")}</span>
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
          {t("mdreader.openFile")}
        </button>
        <button onClick={() => void openDirectory()} className={btn} title={t("mdreader.openDirTip")}>
          <Folder className="w-3.5 h-3.5" />
          {t("mdreader.openDir")}
        </button>
        <button onClick={() => void createNewFile()} className={btn} title={t("mdreader.newFileTip")}>
          <FilePlus className="w-3.5 h-3.5" />
          {t("mdreader.newFile")}
        </button>
        <button onClick={reloadActive} disabled={!active} className={btn}>
          <RefreshCw className="w-3.5 h-3.5" />
          {t("mdreader.reload")}
        </button>
        <button onClick={() => setShowSidebar((s) => !s)} className={btn}>
          {showSidebar ? (
            <PanelLeftClose className="w-3.5 h-3.5" />
          ) : (
            <PanelLeftOpen className="w-3.5 h-3.5" />
          )}
          {showSidebar ? t("mdreader.hideSidebar") : t("mdreader.showSidebar")}
        </button>
        <div className="w-px h-5 bg-white/10 mx-1" />
        {active && (
          <>
            <button onClick={() => setMode(mode === "view" ? "edit" : "view")}
              className={`${btn} ${mode === "edit" ? "!bg-emerald-600/25 !text-emerald-200" : ""}`}
              title={mode === "view" ? t("mdreader.switchEditTip") : t("mdreader.switchPreviewTip")}>
              {mode === "view" ? <Pencil className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
              {mode === "view" ? t("mdreader.edit") : t("mdreader.preview")}
            </button>
            {mode === "edit" && (
              <button onClick={() => setSplit(!split)}
                className={`${btn} ${split ? "!bg-emerald-600/25 !text-emerald-200" : ""}`} title={t("mdreader.splitTip")}>
                <Columns className="w-3.5 h-3.5" />
                {t("mdreader.split")}
              </button>
            )}
            <button onClick={() => void saveActive()} disabled={!active.dirty || saving} className={btn} title={t("mdreader.saveTip")}>
              {saving ? <LoaderCircle /> : active.dirty ? <Save className="w-3.5 h-3.5" /> : <Check className="w-3.5 h-3.5 text-emerald-400" />}
              {active.dirty ? (saving ? t("mdreader.saving") : t("common.save")) : t("mdreader.saved")}
            </button>
            <div className="w-px h-5 bg-white/10 mx-1" />
            <button
              className={tbtn}
              title={t("mdreader.findTip")}
              onClick={() => {
                setFindOpen(true);
                requestAnimationFrame(() => { findInputRef.current?.focus(); findInputRef.current?.select(); });
              }}
            >
              <Search className="w-3.5 h-3.5" />
            </button>
            <button className={tbtn} title={t("mdreader.exportHtmlTip")} onClick={() => void exportHtml()}>
              <Download className="w-3.5 h-3.5" />
            </button>
            <button className={tbtn} title={t("mdreader.printTip")} onClick={printActive}>
              <Printer className="w-3.5 h-3.5" />
            </button>
          </>
        )}
        <div className="flex-1" />
        <button onClick={() => setShowAssoc((s) => !s)} className={btn} title={t("mdreader.assocTip")}>
          <FileText className="w-3.5 h-3.5" />
          {t("mdreader.assocBtn")}{assoc ? (assocAll ? " ✓" : "") : ""}
        </button>
        {notice && (
          <div className="flex items-center gap-1 text-tiny text-amber-300">
            <AlertCircle className="w-3 h-3" />
            {notice}
          </div>
        )}
        {active && (
          <span className="text-tiny text-slate-500 truncate max-w-[30%] min-w-0" title={active.path}>
            {active.path}
          </span>
        )}
      </div>

      {/* 查找 / 替换栏 */}
      {findOpen && active && (
        <div className="flex-shrink-0 flex items-center gap-1.5 px-4 py-1.5 border-b border-white/5 bg-white/[0.02]">
          <Search className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
          <input
            ref={findInputRef}
            value={findQuery}
            onChange={(e) => { setFindQuery(e.target.value); setFindIndex(-1); }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                jumpToMatch(e.shiftKey ? -1 : 1);
              }
            }}
            placeholder={t("mdreader.findPh")}
            className="w-44 bg-white/5 rounded px-2 py-1 text-tiny text-slate-200 placeholder:text-slate-600 outline-none focus:bg-white/10"
          />
          <span className="text-tiny text-slate-500 font-mono min-w-[52px] text-center">
            {findMatches.length ? `${findIndex < 0 ? 0 : findIndex + 1}/${findMatches.length}` : "0/0"}
          </span>
          <button className={tbtn} title={t("mdreader.findPrev")} disabled={!findMatches.length} onClick={() => jumpToMatch(-1)}>
            <ArrowUp className="w-3.5 h-3.5" />
          </button>
          <button className={tbtn} title={t("mdreader.findNext")} disabled={!findMatches.length} onClick={() => jumpToMatch(1)}>
            <ArrowDown className="w-3.5 h-3.5" />
          </button>
          <div className="w-px h-5 bg-white/10 mx-1" />
          <Replace className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
          <input
            value={findReplace}
            onChange={(e) => setFindReplace(e.target.value)}
            placeholder={t("mdreader.replacePh")}
            className="w-40 bg-white/5 rounded px-2 py-1 text-tiny text-slate-200 placeholder:text-slate-600 outline-none focus:bg-white/10"
          />
          <button className={btn} disabled={!findMatches.length} onClick={replaceCurrent}>{t("mdreader.replace")}</button>
          <button className={btn} disabled={!findMatches.length} onClick={replaceAll}>{t("mdreader.replaceAll")}</button>
          <div className="flex-1" />
          <button className={tbtn} title={t("mdreader.findClose")} onClick={() => setFindOpen(false)}>
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {/* 文件关联面板 */}
      {showAssoc && (
        <div className="flex-shrink-0 px-4 py-2.5 border-b border-white/5 bg-white/[0.02] space-y-2">
          <div className="text-tiny text-slate-400">
            {t("mdreader.assocDesc")}
          </div>
          <div className="flex items-center gap-2">
            <button onClick={() => void toggleAssoc(true)} className={btn} disabled={assocAll}>
              <Check className="w-3.5 h-3.5" />
              {assocAll ? t("mdreader.registered") : t("mdreader.registerAssoc")}
            </button>
            <button onClick={() => void toggleAssoc(false)} className={btn} disabled={assoc ? !assoc.md && !assoc.markdown : false}>
              <X className="w-3.5 h-3.5" />
              {t("mdreader.unregisterAssoc")}
            </button>
            {assoc && (
              <span className="text-tiny text-slate-500 font-mono truncate">
                .md: {assoc.md ? "✓" : "✗"} · .markdown: {assoc.markdown ? "✓" : "✗"} · {assoc.exePath}
              </span>
            )}
          </div>
        </div>
      )}

      {/* 选项卡条 */}
      {tabs.length > 0 && (
        <div className="flex-shrink-0 flex items-stretch gap-0.5 px-2 pt-1.5 border-b border-white/5 overflow-x-auto">
          {tabs.map((tab) => (
            <div
              key={tab.path}
              onClick={() => { setActivePath(tab.path); setMode("view"); }}
              title={tab.path}
              className={`group flex items-center gap-1.5 px-2.5 py-1.5 rounded-t-md text-caption whitespace-nowrap cursor-pointer transition-colors max-w-[200px] ${
                tab.path === activePath
                  ? "bg-emerald-600/20 text-emerald-200 border-b-2 border-emerald-500"
                  : "text-slate-400 hover:text-slate-200 hover:bg-white/5 border-b-2 border-transparent"
              }`}
            >
              <FileText className="w-3 h-3 flex-shrink-0" />
              <span className="truncate">{tab.name}{tab.dirty ? " •" : ""}</span>
              <button
                onClick={(e) => closeTab(tab.path, e)}
                className="flex-shrink-0 opacity-0 group-hover:opacity-100 text-slate-500 hover:text-red-300 transition-opacity"
                title={t("mdreader.closeTip")}
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
              <div className="flex items-center gap-1.5 mb-1.5 text-tiny text-slate-500">
                <ListTree className="w-3 h-3" />
                <span className="truncate" title={siblingRoot}>
                  {siblingRoot ? siblingRoot.split(/[\\/]/).pop() : t("mdreader.fileList")}
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
                  placeholder={t("mdreader.filterPh")}
                  className="w-full bg-white/5 rounded pl-6 pr-2 py-1 text-tiny text-slate-200 placeholder:text-slate-600 outline-none focus:bg-white/10"
                />
              </div>
            </div>
            <div className="flex-1 overflow-y-auto py-1">
              {filtered.length === 0 && (
                <div className="px-3 py-4 text-tiny text-slate-600 text-center">
                  {siblings.length === 0 ? t("mdreader.siblingsEmpty") : t("mdreader.noMatch")}
                </div>
              )}
              {filtered.map((s) => (
                <button
                  key={s.path}
                  onClick={() => openPath(s.path)}
                  title={`${s.rel} · ${formatSize(s.size)}`}
                  className={`w-full text-left px-2.5 py-1 text-tiny flex items-center gap-1.5 transition-colors ${
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
        <div
          ref={contentRef}
          onScroll={() => { if (mode === "view" && !split) updateActiveHeading(); }}
          className="flex-1 min-w-0 flex flex-col overflow-y-auto select-text"
        >
          {!active && (
            <div className="h-full flex flex-col items-center justify-center gap-3 text-slate-600">
              <FileText className="w-10 h-10 opacity-30" />
              <div className="text-caption">{t("mdreader.noDocOpen")}</div>
              <div className="flex gap-2">
                <button onClick={pickFiles} className={btn}>
                  <FolderOpen className="w-3.5 h-3.5" />
                  {t("mdreader.openFile")}
                </button>
                <button onClick={() => void openDirectory()} className={btn}>
                  <Folder className="w-3.5 h-3.5" />
                  {t("mdreader.openDir")}
                </button>
                <button onClick={() => void createNewFile()} className={btn}>
                  <FilePlus className="w-3.5 h-3.5" />
                  {t("mdreader.newFile")}
                </button>
              </div>
              <div className="text-tiny text-slate-700 max-w-xs text-center leading-relaxed">
                {t("mdreader.emptyDesc1")}
                {t("mdreader.emptyDesc2")}
              </div>
              {recent.length > 0 && (
                <div className="mt-2 w-80 max-w-[86%]">
                  <div className="flex items-center gap-1.5 mb-1 text-tiny text-slate-600">
                    <History className="w-3 h-3" />
                    {t("mdreader.recent")}
                  </div>
                  <div className="max-h-44 overflow-y-auto rounded border border-white/5 bg-white/[0.02]">
                    {recent.slice(0, 10).map((r) => (
                      <button
                        key={r.path}
                        onClick={() => void openPath(r.path)}
                        title={r.path}
                        className="w-full text-left px-2.5 py-1 text-tiny flex items-center gap-1.5 text-slate-500 hover:text-slate-200 hover:bg-white/5 transition-colors"
                      >
                        <FileText className="w-3 h-3 opacity-60 flex-shrink-0" />
                        <span className="truncate">{r.name}</span>
                        <span className="ml-auto max-w-[48%] truncate text-slate-700">{dirOf(r.path)}</span>
                      </button>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
          {active?.loading && (
            <div className="px-6 py-6 text-caption text-slate-500">{t("mdreader.loading")}</div>
          )}
          {active?.error && (
            <div className="px-6 py-6 text-caption text-red-300 flex items-start gap-2">
              <AlertCircle className="w-4 h-4 flex-shrink-0 mt-0.5" />
              <span className="break-all">{active.error}</span>
            </div>
          )}
          {active && !active.loading && !active.error && (
            mode === "view" ? preview : editor
          )}
        </div>

        {/* 右侧：大纲 */}
        {active && headings.length > 0 && (
          <div className="w-48 flex-shrink-0 border-l border-white/5 overflow-y-auto py-2">
            <div className="px-3 pb-1.5 text-tiny text-slate-500">{t("mdreader.outline")}</div>
            {outlineRows.filter((row) => !row.hidden).map(({ h, hasChildren }) => {
              const isActive = h.id === activeHeading;
              const isCollapsed = collapsed.has(h.id);
              return (
                <div
                  key={h.id}
                  className={`group flex items-center pr-1 transition-colors ${
                    isActive ? "bg-emerald-600/15 text-emerald-200" : "text-slate-500 hover:bg-white/5"
                  }`}
                >
                  <button
                    onClick={() => {
                      if (!hasChildren) return;
                      setCollapsed((prev) => {
                        const next = new Set(prev);
                        if (next.has(h.id)) next.delete(h.id);
                        else next.add(h.id);
                        return next;
                      });
                    }}
                    className={`flex-shrink-0 flex items-center justify-center w-4 h-5 ${hasChildren ? "cursor-pointer text-slate-500 hover:text-slate-200" : "invisible"}`}
                    title={isCollapsed ? t("mdreader.expand") : t("mdreader.collapse")}
                  >
                    {isCollapsed ? <ChevronRight className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
                  </button>
                  <button
                    onClick={() => jumpToHeading(h)}
                    style={{ paddingLeft: `${(h.level - 1) * 10}px` }}
                    className={`flex-1 min-w-0 text-left py-0.5 text-tiny truncate ${
                      isActive ? "text-emerald-200" : "group-hover:text-slate-200"
                    }`}
                    title={h.text}
                  >
                    {h.text}
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

function LoaderCircle() {
  return <span className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-slate-400 border-t-transparent" />;
}

/** 本地图片：相对路径交给后端读成 data URL，避免 file:// 被 CSP 拦截 */
function LocalImage({ docPath, src, alt }: { docPath: string; src: string; alt: string }) {
  const { t } = useTranslation();
  const external = isExternal(src);
  const [url, setUrl] = useState<string | null | undefined>(external ? src : undefined);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
    if (!src || external) {
      setUrl(src || null);
      return;
    }
    setUrl(undefined);
    let alive = true;
    const key = `${docPath}::${src}`;
    let task = imageCache.get(key);
    if (!task) {
      task = invoke<string | null>("read_local_image", { from: docPath, src });
      imageCache.set(key, task);
    }
    task
      .then((u) => { if (alive) setUrl(u); })
      .catch(() => { if (alive) { setUrl(null); setFailed(true); } });
    return () => { alive = false; };
  }, [docPath, src, external]);

  if (!src) return null;
  if (url === undefined) {
    // 读取中：先显示替代文本，避免布局跳动
    return <span className="text-tiny text-slate-600">{alt || src}</span>;
  }
  if (failed || !url) {
    return (
      <span className="inline-flex items-center gap-1 px-2 py-1 my-1 rounded border border-amber-500/20 bg-amber-500/5 text-tiny text-amber-300/80">
        <AlertCircle className="w-3 h-3 flex-shrink-0" />
        {t("mdreader.imageFailed")}{alt ? ` (${alt})` : ""}
      </span>
    );
  }
  return <img src={url || ""} alt={alt} className="max-w-full rounded my-3" onError={() => setFailed(true)} />;
}

/** 代码块：语言标签 + 语法高亮 + 一键复制 */
function CodeBlock({ lang, children }: { lang: string; children: React.ReactNode }) {
  const { t } = useTranslation();
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
    <div className="relative my-3 rounded-ctl overflow-hidden border border-white/10 bg-slate-900/80">
      <div className="flex items-center justify-between px-2.5 py-1 bg-slate-800/60 border-b border-white/5">
        <span className="text-micro font-mono text-slate-400 uppercase tracking-wide">
          {lang || "text"}
        </span>
        <button
          onClick={handleCopy}
          className="text-micro text-slate-500 hover:text-slate-200 transition-colors cursor-pointer"
        >
          {copied ? t("mdreader.codeCopied") : t("mdreader.codeCopy")}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-caption leading-relaxed">
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

/** 查找高亮 + KaTeX 溢出处理（只注入一次） */
const mdExtraCss = `
::highlight(md-find){background:rgba(250,204,21,.35);color:inherit}
.katex-display{overflow-x:auto;overflow-y:hidden;padding:2px 0}
.katex{font-size:1.05em}
`;
if (typeof document !== "undefined" && !document.getElementById("mdreader-extra")) {
  const style = document.createElement("style");
  style.id = "mdreader-extra";
  style.textContent = mdExtraCss;
  document.head.appendChild(style);
}
