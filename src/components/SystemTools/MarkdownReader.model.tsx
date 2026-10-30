import { invoke } from "@tauri-apps/api/core";
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

/** 后端 list_sibling_markdown 返回的条目 */
export interface MarkdownEntry {
  path: string;
  name: string;
  rel: string;
  size: number;
}

/** 一个打开的选项卡 */
export interface Tab {
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
export interface Heading {
  id: string;
  text: string;
  level: number;
  /** 源码行号（0 基），编辑模式下跳转用 */
  line: number;
}

/** 最近打开的文件（localStorage 持久化） */
export interface RecentFile {
  path: string;
  name: string;
  ts: number;
}

/** 文件关联状态（后端 markdown_assoc_status） */
export interface AssocStatus {
  md: boolean;
  markdown: boolean;
  exePath: string;
}

export const MD_RECENT_KEY = "any_version_markdown_recent";

export const MD_RECENT_FILES_KEY = "any_version_markdown_recent_files";

/** 预览图片的 data URL 缓存：同一文档反复渲染不重复读盘 */
export const imageCache = new Map<string, Promise<string | null>>();

export const formatSize = (n: number): string => {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
};

/** 把标题文本转成锚点 id，与 rehype-slug 的规则近似 */
export const slugify = (text: string): string =>
  text
    .toLowerCase()
    .trim()
    .replace(/[^\w\u4e00-\u9fa5\s-]/g, "")
    .replace(/\s+/g, "-");

/** 外链判定：http(s)、协议链接、纯锚点都不当作本地文件处理 */
export const isExternal = (href: string): boolean =>
  /^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith("//");

export const escapeRegExp = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** 取路径的父目录（兼容 Windows 反斜杠与正斜杠） */
export const dirOf = (path: string): string => {
  const idx = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return idx > 0 ? path.slice(0, idx) : path;
};

/** 转为 file:// URL，供「导出 HTML」里引用本地图片 */
export const toFileUrl = (path: string): string => {
  let s = path.replace(/\\/g, "/");
  if (!s.startsWith("/")) s = `/${s}`;
  return `file://${encodeURI(s).replace(/#/g, "%23").replace(/\?/g, "%3F")}`;
};

/** 导出/打印用 HTML 模板：深色正文改为浅色，保证打印与外部阅读都清晰 */
export const EXPORT_STYLE = `
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
export async function buildExportHtml(docPath: string, content: string): Promise<string> {
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
export function continueListOnEnter(ta: HTMLTextAreaElement): { text: string; caret: number } | null {
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
export function collectTextRanges(root: HTMLElement | null, query: string): Range[] {
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
export function wrapSelection(ta: HTMLTextAreaElement, prefix: string, suffix: string, placeholder = ""): { text: string; selStart: number; selEnd: number } {
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
export function toggleLinePrefix(ta: HTMLTextAreaElement, prefix: string, ordered = false): { text: string; selStart: number; selEnd: number } {
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

/** hljs 深色主题样式（One Dark 风格，随应用主题） */
export const hljsCss = `
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
export const mdExtraCss = `
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
