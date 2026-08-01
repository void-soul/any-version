import React, { useState, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  FolderOpen,
  Save,
  Search,
  Copy,
  Check,
  ChevronRight,
  ChevronDown,
  Indent,
  Minimize2,
  FileJson,
  ClipboardCopy,
  ListTree,
} from "lucide-react";

type Json = unknown;

const getType = (v: Json): string => {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  return typeof v; // "object" | "string" | "number" | "boolean"
};

const VALUE_COLORS: Record<string, string> = {
  string: "text-emerald-300",
  number: "text-amber-300",
  boolean: "text-violet-300",
  null: "text-slate-500",
};

const TYPE_BADGE: Record<string, string> = {
  string: "text-emerald-400",
  number: "text-amber-400",
  boolean: "text-violet-400",
  null: "text-slate-500",
  object: "text-sky-400",
  array: "text-sky-400",
};

function ValueView({ value, type }: { value: Json; type: string }) {
  if (type === "string") {
    return <span className={VALUE_COLORS.string}>"{String(value)}"</span>;
  }
  if (type === "null") {
    return <span className={VALUE_COLORS.null}>null</span>;
  }
  if (type === "boolean" || type === "number") {
    return <span className={VALUE_COLORS[type]}>{String(value)}</span>;
  }
  return <span className="text-slate-400">{String(value)}</span>;
}

export default function JsonBrowser() {
  const [text, setText] = useState<string>("");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [copied, setCopied] = useState<string | null>(null);

  const { data, error } = useMemo(() => {
    if (!text.trim()) return { data: null as Json, error: null as string | null };
    try {
      return { data: JSON.parse(text) as Json, error: null as string | null };
    } catch (e) {
      return { data: null as Json, error: (e as Error).message };
    }
  }, [text]);

  const parsed = data;

  const flash = (key: string) => {
    setCopied(key);
    window.setTimeout(() => setCopied((c) => (c === key ? null : c)), 1200);
  };
  const copyText = async (val: string, key: string) => {
    try {
      await navigator.clipboard.writeText(val);
      flash(key);
    } catch {
      /* ignore */
    }
  };

  const toggle = (path: string) => {
    setCollapsed((p) => {
      const n = new Set(p);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });
  };

  const expandAll = () => setCollapsed(new Set());
  const collapseAll = () => {
    const n = new Set<string>();
    const walk = (val: Json, path: string) => {
      const t = getType(val);
      if (t === "object" || t === "array") {
        n.add(path || "root");
        for (const [k, v] of Object.entries(val as object)) {
          walk(v, path ? `${path}.${k}` : k);
        }
      }
    };
    if (parsed !== null) walk(parsed, "");
    setCollapsed(n);
  };

  const format = () => {
    if (parsed !== null) setText(JSON.stringify(parsed, null, 2));
  };
  const minify = () => {
    if (parsed !== null) setText(JSON.stringify(parsed));
  };
  const copyAll = async () => {
    if (parsed !== null) copyText(JSON.stringify(parsed, null, 2), "all");
  };

  const openFile = async () => {
    try {
      const selected = await open({
        multiple: false,
        title: "打开 JSON 文件",
        filters: [{ name: "JSON", extensions: ["json", "jsonc", "ndjson"] }],
      });
      if (selected && typeof selected === "string") {
        const content = await invoke<string>("read_text_file", { path: selected });
        setText(content);
      }
    } catch (e) {
      console.error(e);
    }
  };
  const saveFile = async () => {
    try {
      const target = await save({
        title: "保存 JSON 文件",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (target && typeof target === "string") {
        await invoke("write_text_file", { path: target, content: text });
      }
    } catch (e) {
      console.error(e);
    }
  };

  const matches = (keyName: string, value: Json, path: string): boolean => {
    if (!query) return true;
    const q = query.toLowerCase();
    if (keyName.toLowerCase().includes(q)) return true;
    if (path.toLowerCase().includes(q)) return true;
    if (typeof value !== "object" || value === null) {
      if (String(value).toLowerCase().includes(q)) return true;
    }
    return false;
  };
  const subtreeHasMatch = (keyName: string, value: Json, path: string): boolean => {
    if (matches(keyName, value, path)) return true;
    if (value && typeof value === "object") {
      for (const [k, v] of Object.entries(value as object)) {
        if (subtreeHasMatch(k, v, path ? `${path}.${k}` : k)) return true;
      }
    }
    return false;
  };

  const countNodes = (val: Json): number => {
    if (val === null || typeof val !== "object") return 1;
    let n = 1;
    for (const v of Object.values(val as object)) n += countNodes(v);
    return n;
  };

  const btn =
    "flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-slate-300 bg-white/5 hover:bg-white/10 transition-colors";

  const renderNode = (keyName: string, value: Json, path: string, depth: number): React.ReactNode => {
    const type = getType(value);
    const isContainer = type === "object" || type === "array";
    const hasMatch = query ? subtreeHasMatch(keyName, value, path) : true;
    if (query && !hasMatch) return null;
    const nodePath = path || "root";
    const isCollapsed = collapsed.has(nodePath) && !query;
    const indent = { paddingLeft: `${depth * 14}px` };
    const hit = query ? matches(keyName, value, path) : false;

    if (!isContainer) {
      const valStr = typeof value === "object" ? JSON.stringify(value) : String(value);
      return (
        <div key={nodePath} style={indent} className="flex items-start gap-1 py-[1px] hover:bg-white/5 group">
          {keyName !== "root" && (
            <>
              <span className={hit ? "text-sky-200 bg-sky-400/20 rounded px-0.5" : "text-sky-300"}>
                {keyName}
              </span>
              <span className="text-slate-500">:&nbsp;</span>
            </>
          )}
          <ValueView value={value} type={type} />
          <button
            onClick={() => copyText(valStr, `v:${nodePath}`)}
            className="ml-1 opacity-0 group-hover:opacity-100 text-slate-500 hover:text-slate-200"
            title="复制值"
          >
            {copied === `v:${nodePath}` ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
          </button>
        </div>
      );
    }

    const entries: [string, Json][] =
      type === "array"
        ? (value as Json[]).map((v, i) => [String(i), v])
        : (Object.entries(value as object) as [string, Json][]);
    const summary = type === "array" ? `数组 [${entries.length}]` : `对象 {${entries.length}}`;

    return (
      <div key={nodePath}>
        <div style={indent} className="flex items-center gap-1 py-[1px] hover:bg-white/5 group">
          <button onClick={() => toggle(nodePath)} className="text-slate-400 hover:text-slate-200">
            {isCollapsed ? <ChevronRight className="w-3.5 h-3.5" /> : <ChevronDown className="w-3.5 h-3.5" />}
          </button>
          {keyName !== "root" && (
            <>
              <span className={hit ? "text-sky-200 bg-sky-400/20 rounded px-0.5" : "text-sky-300"}>
                {keyName}
              </span>
              <span className="text-slate-500">:&nbsp;</span>
            </>
          )}
          <span className={`text-[10px] ${TYPE_BADGE[type]}`}>{summary}</span>
          <button
            onClick={() => copyText(JSON.stringify(value, null, 2), `v:${nodePath}`)}
            className="ml-1 opacity-0 group-hover:opacity-100 text-slate-500 hover:text-slate-200"
            title="复制子树"
          >
            {copied === `v:${nodePath}` ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />}
          </button>
        </div>
        {!isCollapsed && (
          <div>
            {entries.map(([k, v]) => renderNode(k, v, path ? `${path}.${k}` : k, depth + 1))}
          </div>
        )}
      </div>
    );
  };

  const topCount =
    parsed !== null && typeof parsed === "object"
      ? Array.isArray(parsed)
        ? parsed.length
        : Object.keys(parsed).length
      : 0;
  const totalNodes = parsed !== null ? countNodes(parsed) : 0;

  return (
    <div className="h-full flex flex-col text-slate-200 bg-slate-950/30">
      {/* 工具栏 */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-white/5 flex-wrap">
        <button onClick={openFile} className={btn}>
          <FolderOpen className="w-3.5 h-3.5" /> 打开文件
        </button>
        <button onClick={saveFile} className={btn}>
          <Save className="w-3.5 h-3.5" /> 保存
        </button>
        <div className="w-px h-4 bg-white/10" />
        <button onClick={format} disabled={parsed === null} className={btn}>
          <Indent className="w-3.5 h-3.5" /> 格式化
        </button>
        <button onClick={minify} disabled={parsed === null} className={btn}>
          <Minimize2 className="w-3.5 h-3.5" /> 压缩
        </button>
        <button onClick={expandAll} disabled={parsed === null} className={btn}>
          展开全部
        </button>
        <button onClick={collapseAll} disabled={parsed === null} className={btn}>
          折叠全部
        </button>
        <button onClick={copyAll} disabled={parsed === null} className={btn}>
          {copied === "all" ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <ClipboardCopy className="w-3.5 h-3.5" />} 复制
        </button>
        <div className="flex items-center gap-1 ml-auto bg-white/5 rounded-md px-2 py-1 w-56">
          <Search className="w-3.5 h-3.5 text-slate-500" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索键名 / 路径 / 值"
            className="bg-transparent outline-none text-xs w-full placeholder:text-slate-600"
          />
        </div>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* 编辑区 */}
        <div className="w-1/2 border-r border-white/5 flex flex-col min-h-0">
          <div className="flex items-center justify-between px-3 py-1 text-[10px] text-slate-500 border-b border-white/5">
            <span className="flex items-center gap-1">
              <FileJson className="w-3 h-3" /> JSON 源
            </span>
            {parsed !== null && <span>已解析 · 顶层 {topCount} 项</span>}
          </div>
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            spellCheck={false}
            placeholder='在此粘贴 JSON，或点击“打开文件”加载…'
            className="flex-1 w-full bg-transparent p-3 font-mono text-xs leading-relaxed text-slate-200 resize-none outline-none"
          />
          {error && (
            <div className="px-3 py-1.5 text-[10px] text-red-300 bg-red-500/10 border-t border-red-500/20">
              解析错误：{error}
            </div>
          )}
        </div>

        {/* 浏览树 */}
        <div className="w-1/2 flex flex-col min-h-0 bg-slate-950/20">
          <div className="flex items-center justify-between px-3 py-1 text-[10px] text-slate-500 border-b border-white/5">
            <span className="flex items-center gap-1">
              <ListTree className="w-3 h-3" /> 结构树
            </span>
            {parsed !== null && <span>共 {totalNodes} 个节点</span>}
          </div>
          <div className="flex-1 overflow-auto p-3 text-xs font-mono min-h-0">
            {parsed === null ? (
              <div className="text-slate-600 mt-8 text-center">
                {error ? "JSON 格式有误，无法浏览" : "在左侧粘贴或打开 JSON 以浏览结构"}
              </div>
            ) : (
              renderNode("root", parsed, "", 0)
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
