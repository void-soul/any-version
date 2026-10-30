import { MarkdownDocumentTabs } from "./MarkdownDocumentTabs";
import { MarkdownAssociationBanner } from "./MarkdownAssociationBanner";
import { useMarkdownExport } from "./useMarkdownExport";
import { MarkdownOutlineSidebar } from "./MarkdownOutlineSidebar";
import { MarkdownEmptyState } from "./MarkdownEmptyState";
import { MarkdownFileSidebar } from "./MarkdownFileSidebar";
import { MarkdownEditorTextarea } from "./MarkdownEditorTextarea";
import { MarkdownEditorToolbar } from "./MarkdownEditorToolbar";
import { MarkdownPreviewContent } from "./MarkdownPreviewContent";
import { useMarkdownSearch } from "./useMarkdownSearch";
import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { theamedConfirm } from "../shared/ThemedAlert";
import "katex/dist/katex.min.css";
import { FolderOpen, X, FileText, Search, PanelLeftClose, PanelLeftOpen, RefreshCw, AlertCircle, Pencil, Eye, Save, Check, Columns, FilePlus, Folder, Download, Printer, ArrowUp, ArrowDown, Replace } from "lucide-react";
import { MD_RECENT_FILES_KEY, MD_RECENT_KEY, isExternal, slugify, type AssocStatus, type Heading, type MarkdownEntry, type RecentFile, type Tab, wrapSelection } from "./MarkdownReader.model";
import { LoaderCircle } from "./MarkdownReader.LoaderCircle";

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

  const contentRef = useRef<HTMLDivElement>(null);
  const previewPaneRef = useRef<HTMLDivElement>(null);
  const taRef = useRef<HTMLTextAreaElement>(null);

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

  const {
    findOpen,
    setFindOpen,
    findQuery,
    setFindQuery,
    findReplace,
    setFindReplace,
    findIndex,
    setFindIndex,
    findInputRef,
    findMatches,
    jumpToMatch,
    replaceCurrent,
    replaceAll,
  } = useMarkdownSearch({
    active,
    mode,
    taRef,
    setContent,
    flashNotice,
    previewVisible,
    contentRef,
  });

  // 匹配数变化时把当前索引收敛到有效范围

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

  const {
    exportHtml,
    printActive,
  } = useMarkdownExport({
    active,
    flashNotice,
  });

  const btn =
    "flex items-center gap-1 rounded-md px-2 py-1 text-caption font-medium text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed";
  const tbtn =
    "inline-flex items-center justify-center h-7 w-7 rounded-md text-slate-300 bg-white/5 hover:bg-white/10 transition-colors cursor-pointer disabled:opacity-30";
  const assocAll = assoc ? assoc.md && assoc.markdown : false;

  const preview = active && !active.loading && !active.error && (
    <MarkdownPreviewContent
      headingIdByLine={headingIdByLine}
      toggleTaskAtLine={toggleTaskAtLine}
      handleLinkClick={handleLinkClick}
      active={active}
    />
  );

  const editor = active && !active.loading && !active.error && (
    <div className="flex flex-col min-h-0 flex-1">
      {/* 格式工具栏 */}
      <MarkdownEditorToolbar
        tbtn={tbtn}
        active={active}
        undo={undo}
        redo={redo}
        applyTransform={applyTransform}
      />
      {/* 编辑区（可选分栏预览） */}
      <div className="flex min-h-0 flex-1">
        <MarkdownEditorTextarea
          taRef={taRef}
          active={active}
          activePath={activePath}
          setContent={setContent}
          updateCursor={updateCursor}
          split={split}
          syncScroll={syncScroll}
          updateActiveHeading={updateActiveHeading}
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
        <MarkdownAssociationBanner
          toggleAssoc={toggleAssoc}
          btn={btn}
          assocAll={assocAll}
          assoc={assoc}
        />
      )}

      {/* 选项卡条 */}
      {tabs.length > 0 && (
        <MarkdownDocumentTabs
          tabs={tabs}
          setActivePath={setActivePath}
          setMode={setMode}
          activePath={activePath}
          closeTab={closeTab}
        />
      )}

      <div className="flex-1 flex min-h-0">
        {/* 左侧：同目录文件列表 */}
        {showSidebar && (
          <MarkdownFileSidebar
            siblingRoot={siblingRoot}
            scanning={scanning}
            siblings={siblings}
            filter={filter}
            setFilter={setFilter}
            filtered={filtered}
            openPath={openPath}
            activePath={activePath}
          />
        )}

        {/* 中间：正文 / 编辑器 */}
        <div
          ref={contentRef}
          onScroll={() => { if (mode === "view" && !split) updateActiveHeading(); }}
          className="flex-1 min-w-0 flex flex-col overflow-y-auto select-text"
        >
          {!active && (
            <MarkdownEmptyState
              pickFiles={pickFiles}
              btn={btn}
              openDirectory={openDirectory}
              createNewFile={createNewFile}
              recent={recent}
              openPath={openPath}
            />
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
          <MarkdownOutlineSidebar
            outlineRows={outlineRows}
            activeHeading={activeHeading}
            collapsed={collapsed}
            setCollapsed={setCollapsed}
            jumpToHeading={jumpToHeading}
          />
        )}
      </div>
    </div>
  );
}
