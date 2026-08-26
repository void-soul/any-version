import { useMemo, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Plus, ArrowDown, Trash2, RefreshCcw, Wrench, MoreHorizontal, FileText, Lock, ChevronsUpDown, ChevronDown, ChevronRight,
} from "lucide-react";
import type { ApiEndpoint, KeyValueItem, FormDataItem, Authorization, RequestSettings } from "./types";
import { AUTH_TYPES, RANDOM_VARIABLES, defaultAuthorization, defaultSettings } from "./types";

export const ACCENT = "var(--module-accent)";

// ─── 变量自动补全输入框 ───
// 输入 {{ 触发下拉：环境变量 + 随机变量，选择后回填完整 {{var}}。
// 环境变量来自当前生效的变量集合（名 → 值），随机变量来自 RANDOM_VARIABLES 常量。
// 支持模糊匹配（子序列评分），悬停候选可预览当前环境值。

/** 模糊匹配评分：q 需为 label 的子序列；前缀/连续/分隔符边界加分，返回 -1 表示不匹配。 */
function fuzzyScore(label: string, q: string): number {
  const s = label.toLowerCase();
  const t = q.toLowerCase();
  if (!t) return 0;
  if (s.startsWith(t)) return 1000 + (s.length - t.length);
  let score = 0;
  let ti = 0;
  let prev = -2;
  for (let i = 0; i < s.length && ti < t.length; i++) {
    if (s[i] !== t[ti]) continue;
    // 连续命中加分；分隔符/驼峰边界额外加分
    let add = i === prev + 1 ? 3 : 1;
    if (i > 0) {
      const prevCh = s[i - 1];
      if (prevCh === "_" || prevCh === "-" || prevCh === " " || prevCh === "." || prevCh === "/") add += 4;
      else if (prevCh >= "a" && prevCh <= "z" && s[i] >= "A" && s[i] <= "Z") add += 2;
    } else {
      add += 5;
    }
    if (i > prev + 1) add -= 1; // 中间有跳跃的小惩罚
    score += add;
    prev = i;
    ti++;
  }
  return ti === t.length ? score : -1;
}

export function VarInput({ value, onChange, envVars, placeholder, className, disabled, multiline, rows, onKeyDown }: {
  value: string;
  onChange: (v: string) => void;
  /** 当前环境变量（名 → 值） */
  envVars: Record<string, unknown>;
  placeholder?: string;
  className?: string;
  disabled?: boolean;
  /** 多行模式（textarea） */
  multiline?: boolean;
  rows?: number;
  onKeyDown?: (e: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIdx, setActiveIdx] = useState(0);
  const [caret, setCaret] = useState<{ top: number; left: number } | null>(null);
  // 随机变量行的展开说明（格式 + 示例）
  const [expandedIdx, setExpandedIdx] = useState<number | null>(null);
  const [hoverExample, setHoverExample] = useState<{ idx: number; value: string } | null>(null);
  const inputRef = useRef<HTMLInputElement | HTMLTextAreaElement>(null);

  // 计算光标在输入框中的像素位置（用于定位下拉）
  const caretPos = (el: HTMLInputElement | HTMLTextAreaElement, sel: number) => {
    try {
      // 用临时镜像节点测量光标坐标
      const s = window.getComputedStyle(el);
      const mirror = document.createElement("div");
      mirror.style.cssText = `position:fixed;top:0;left:0;visibility:hidden;white-space:pre-wrap;word-break:break-all;font:${s.font};width:${el.clientWidth}px;padding:${s.padding};border:${s.borderWidth} solid transparent;`;
      const text = el.value.slice(0, sel);
      const lastNewline = Math.max(text.lastIndexOf("\n"), 0);
      mirror.textContent = text.slice(0, lastNewline) + "\u200b" + text.slice(lastNewline);
      document.body.appendChild(mirror);
      const span = document.createElement("span");
      span.textContent = text.slice(lastNewline);
      mirror.appendChild(span);
      const pos = { top: mirror.offsetTop + span.offsetTop - el.scrollTop, left: mirror.offsetLeft + span.offsetLeft };
      document.body.removeChild(mirror);
      const elRect = el.getBoundingClientRect();
      return { top: pos.top - 8, left: Math.min(pos.left, elRect.width - 180) };
    } catch {
      return null;
    }
  };

  const maybeOpen = (el: HTMLInputElement | HTMLTextAreaElement) => {
    const v = el.value;
    const sel = el.selectionStart ?? v.length;
    // 查找光标前最近的未闭合 {{
    let idx = -1;
    for (let i = sel - 1; i >= 0; i--) {
      if (v[i] === "{" && v[i - 1] === "{") { idx = i - 1; break; }
      if (v[i] === "}") break;
    }
    if (idx >= 0) {
      setQuery(v.slice(idx + 2, sel));
      setActiveIdx(0);
      setOpen(true);
      setCaret(caretPos(el, sel));
    } else {
      setOpen(false);
    }
  };

  const insert = (token: string) => {
    const el = inputRef.current;
    const sel = el?.selectionStart ?? value.length;
    // 查找光标前最近的未闭合 {{，整体替换
    let idx = -1;
    const v = el?.value ?? value;
    for (let i = sel - 1; i >= 0; i--) {
      if (v[i] === "{" && v[i - 1] === "{") { idx = i - 1; break; }
      if (v[i] === "}") break;
    }
    if (idx >= 0) {
      onChange(v.slice(0, idx) + token + v.slice(sel));
    } else {
      onChange(v.slice(0, sel) + token + v.slice(sel));
    }
    setOpen(false);
    requestAnimationFrame(() => inputRef.current?.focus());
  };

  // 候选：环境变量优先，随机变量在后；查询时按模糊评分排序
  const candidates = useMemo(() => {
    const env = Object.entries(envVars).map(([name, val]) => ({ label: name, value: String(val), desc: "环境变量", token: `{{${name}}}` as string, format: undefined as string | undefined, example: undefined as (() => string) | undefined }));
    const rnd = RANDOM_VARIABLES.map((r) => ({ label: r.token, value: "", desc: r.desc, token: r.token, format: r.format, example: r.example }));
    const all = [...env, ...rnd];
    const q = query.trim();
    if (!q) return all;
    return all
      .map((c, i) => ({ c, i, s: fuzzyScore(c.label, q) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s || a.i - b.i)
      .map((x) => x.c);
  }, [envVars, query]);

  const commonProps = {
    ref: inputRef as React.Ref<HTMLInputElement>,
    value,
    placeholder,
    disabled,
    className,
    onChange: (e: React.ChangeEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      const v = e.target.value;
      onChange(v);
      maybeOpen(e.target);
    },
    onFocus: (e: React.FocusEvent<HTMLInputElement | HTMLTextAreaElement>) => maybeOpen(e.target),
    onBlur: () => setTimeout(() => setOpen(false), 150),
    onKeyDown: (e: React.KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      if (open && candidates.length > 0) {
        if (e.key === "ArrowDown") { e.preventDefault(); setActiveIdx((i) => (i + 1) % candidates.length); return; }
        if (e.key === "ArrowUp") { e.preventDefault(); setActiveIdx((i) => (i - 1 + candidates.length) % candidates.length); return; }
        if (e.key === "Enter") { e.preventDefault(); insert(candidates[activeIdx].token); return; }
        if (e.key === "Escape") { setOpen(false); return; }
      }
      onKeyDown?.(e);
    },
  };

  return (
    <div className="relative">
      {multiline ? (
        <textarea rows={rows} {...(commonProps as React.TextareaHTMLAttributes<HTMLTextAreaElement>)} />
      ) : (
        <input {...(commonProps as React.InputHTMLAttributes<HTMLInputElement>)} />
      )}
      {open && candidates.length > 0 && (
        <div
          className="absolute z-30 max-h-48 overflow-y-auto rounded-md border border-white/10 bg-[#0d1524] shadow-xl"
          style={caret ? { left: caret.left, top: caret.top } : { left: 0, right: 0, top: "calc(100% + 2px)" }}
        >
          {candidates.map((c, i) => {
            const exampleFn = c.example;
            const isRandom = !!exampleFn;
            const expanded = isRandom && expandedIdx === i;
            const exampleVal = isRandom && hoverExample?.idx === i ? hoverExample.value : isRandom ? exampleFn() : "";
            return (
              <div key={c.label + c.desc}>
                <button
                  type="button"
                  onMouseDown={(e) => { e.preventDefault(); insert(c.token); }}
                  onMouseEnter={() => {
                    setActiveIdx(i);
                    if (exampleFn) setHoverExample({ idx: i, value: exampleFn() });
                  }}
                  title={
                    c.value
                      ? `当前环境值：${c.value}`
                      : isRandom
                        ? (hoverExample?.idx === i ? `示例值：${hoverExample.value}` : c.format)
                        : c.desc
                  }
                  className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[11px] cursor-pointer ${i === activeIdx ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)]" : ""}`}
                >
                  {isRandom ? (
                    <span
                      role="button"
                      tabIndex={-1}
                      onClick={(e) => { e.stopPropagation(); setExpandedIdx(expanded ? null : i); }}
                      className="shrink-0 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                      title={expanded ? "收起格式说明" : "展开格式说明"}
                    >
                      {expanded ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                    </span>
                  ) : (
                    <ChevronsUpDown className="w-3 h-3 shrink-0 text-slate-500" />
                  )}
                  <code className="font-mono text-[var(--module-accent)] whitespace-nowrap">{c.label}</code>
                  {c.value !== "" && (
                    <span className="max-w-[9rem] truncate font-mono text-[9px] text-slate-600">{c.value}</span>
                  )}
                  <span className="ml-auto shrink-0 truncate text-[9px] text-slate-500">{c.desc}</span>
                </button>
                {expanded && (
                  <div className="mx-2 mb-1 rounded-md border border-white/10 bg-black/30 px-2 py-1.5 text-[10px] leading-relaxed">
                    <div className="text-slate-300">{c.format}</div>
                    <div className="mt-0.5 flex items-center gap-1.5">
                      <span className="text-slate-500">示例：</span>
                      <code className="font-mono text-emerald-300 break-all">{exampleVal}</code>
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

export function methodIcon(method: string) {
  switch (method) {
    case "GET": return ArrowDown;
    case "POST": return Plus;
    case "PUT": return RefreshCcw;
    case "DELETE": return Trash2;
    case "PATCH": return Wrench;
    default: return MoreHorizontal;
  }
}

export function emptyEndpoint(projectId: string, moduleId: string | null): ApiEndpoint {
  return {
    id: "", project_id: projectId, module_id: moduleId,
    name: "新接口", method: "GET", url: "", headers: [], query_params: [], path_params: [],
    body: "", body_type: "none", body_form: [], body_urlencoded: [],
    body_graphql_query: "", body_graphql_variables: "",
    authorization: defaultAuthorization(), cookies: [], settings: defaultSettings(),
    response_comment: "", is_favorite: false, description: "", docs_md: "", timeout_ms: 15000,
    created_at: "", updated_at: "",
  };
}

export function fmtTime(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

export function prettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

// ─── 响应体 JSON 轻量语法高亮 ───

/** JSON token 类型 → 颜色类名 */
const JSON_TOKEN_CLS: Record<string, string> = {
  key: "text-sky-300",
  str: "text-emerald-300",
  num: "text-amber-300",
  bool: "text-violet-300",
  null: "text-slate-500",
};

/** 把 JSON 文本切成带颜色的 token（无依赖，正则扫描）。非 JSON 文本原样返回。 */
export function highlightJsonTokens(text: string): Array<{ cls: string; content: string }> | null {
  // 快速探测是否为 JSON
  const t = text.trim();
  if (!t.startsWith("{") && !t.startsWith("[")) return null;
  try {
    JSON.parse(t);
  } catch {
    return null;
  }
  const re = /"(?:[^"\\]|\\.)*"(\s*:)?|\btrue\b|\bfalse\b|\bnull\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;
  const tokens: Array<{ cls: string; content: string }> = [];
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(t))) {
    if (m.index > last) tokens.push({ cls: "", content: t.slice(last, m.index) });
    const isKey = m[1] !== undefined; // 引号后跟冒号 → 键
    const raw = isKey ? m[0] : m[0];
    let cls: string;
    if (isKey) cls = JSON_TOKEN_CLS.key;
    else if (raw.startsWith('"')) cls = JSON_TOKEN_CLS.str;
    else if (raw === "true" || raw === "false") cls = JSON_TOKEN_CLS.bool;
    else if (raw === "null") cls = JSON_TOKEN_CLS.null;
    else cls = JSON_TOKEN_CLS.num;
    tokens.push({ cls, content: raw });
    last = m.index + m[0].length;
  }
  if (last < t.length) tokens.push({ cls: "", content: t.slice(last) });
  return tokens;
}

/** 响应体查看器：JSON 高亮（pretty）或原始文本（raw）。 */
export function ResponseBody({ body, mode }: { body: string; mode: "pretty" | "raw" }) {
  const shown = mode === "pretty" ? prettyJson(body) : body;
  const tokens = mode === "pretty" ? highlightJsonTokens(shown) : null;
  if (tokens) {
    return (
      <pre className="overflow-auto p-2 text-[11px] font-mono whitespace-pre-wrap break-all h-full">
        {tokens.map((tk, i) =>
          tk.cls ? (
            <span key={i} className={tk.cls}>{tk.content}</span>
          ) : (
            <span key={i} className="text-slate-200">{tk.content}</span>
          )
        )}
      </pre>
    );
  }
  return <pre className="overflow-auto p-2 text-[11px] font-mono text-slate-200 whitespace-pre-wrap break-all h-full">{shown}</pre>;
}

// ─── 键值编辑器（Key-value 编辑 / Bulk 编辑 / 描述） ───
export function KvEditor({ items, onChange, placeholderKey = "名称", placeholderValue = "值", withDescription = true, envVars = {} }: {
  items: KeyValueItem[];
  onChange: (items: KeyValueItem[]) => void;
  placeholderKey?: string;
  placeholderValue?: string;
  withDescription?: boolean;
  /** 当前环境变量（名 → 值，用于值输入框的自动补全与悬停预览） */
  envVars?: Record<string, unknown>;
}) {
  const [mode, setMode] = useState<"kv" | "bulk">("kv");
  const [bulkText, setBulkText] = useState(items.map((kv) => kv.enabled ? `${kv.key}:${kv.value}` : `// ${kv.key}:${kv.value}`).join("\n"));

  const update = (i: number, patch: Partial<KeyValueItem>) => {
    // 模板继承项不允许修改（名称/值/启用状态均由项目模板控制）
    const target = items[i];
    if (target?.from_template && ("key" in patch || "value" in patch || "enabled" in patch)) {
      return;
    }
    onChange(items.map((kv, idx) => (idx === i ? { ...kv, ...patch } : kv)));
  };

  const applyBulk = () => {
    const parsed: KeyValueItem[] = [];
    for (const raw of bulkText.split("\n")) {
      const line = raw.trim();
      if (!line) continue;
      if (line.startsWith("//")) {
        const rest = line.replace(/^\/\/\s*/, "");
        const idx = rest.indexOf(":");
        if (idx > 0) parsed.push({ key: rest.slice(0, idx).trim(), value: rest.slice(idx + 1).trim(), enabled: false, description: "" });
        continue;
      }
      const idx = line.indexOf(":");
      if (idx > 0) parsed.push({ key: line.slice(0, idx).trim(), value: line.slice(idx + 1).trim(), enabled: true, description: "" });
      else parsed.push({ key: line, value: "", enabled: true, description: "" });
    }
    // 模板继承项不允许被 Bulk 修改/删除，保留原样
    const templateItems = items.filter((kv) => kv.from_template);
    onChange([...templateItems, ...parsed]);
  };

  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-1">
        <button
          type="button"
          onClick={() => { setMode("kv"); setBulkText(items.map((kv) => kv.enabled ? `${kv.key}:${kv.value}` : `// ${kv.key}:${kv.value}`).join("\n")); }}
          className={`text-[10px] px-2 py-0.5 rounded cursor-pointer ${mode === "kv" ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
        >
          Key-value 编辑
        </button>
        <button
          type="button"
          onClick={() => { setMode("bulk"); setBulkText(items.map((kv) => kv.enabled ? `${kv.key}:${kv.value}` : `// ${kv.key}:${kv.value}`).join("\n")); }}
          className={`text-[10px] px-2 py-0.5 rounded cursor-pointer ${mode === "bulk" ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
        >
          Bulk 编辑
        </button>
        {mode === "bulk" && (
          <button type="button" onClick={applyBulk} className="text-[10px] px-2 py-0.5 rounded bg-[var(--module-accent)]/20 text-[var(--module-accent)] cursor-pointer">
            应用
          </button>
        )}
      </div>
      {mode === "bulk" ? (
        <textarea
          value={bulkText}
          onChange={(e) => setBulkText(e.target.value)}
          rows={6}
          placeholder={"每行一个 key:value；以 // 开头表示禁用"}
          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
        />
      ) : (
        <div className="space-y-1">
          {items.map((kv, i) => {
            const locked = !!kv.from_template;
            return (
              <div key={i} className="flex items-center gap-1.5">
                <input
                  type="checkbox"
                  checked={kv.enabled}
                  disabled={locked}
                  onChange={(e) => update(i, { enabled: e.target.checked })}
                  className="accent-[var(--module-accent)] shrink-0 disabled:opacity-40"
                  title={locked ? "模板继承项：启用状态由项目模板控制" : (kv.enabled ? "启用" : "禁用")}
                />
                {locked && <Lock className="w-3 h-3 shrink-0 text-[var(--module-accent)]" aria-label="继承自项目模板（只读）" />}
                <input
                  value={kv.key}
                  disabled={locked}
                  onChange={(e) => update(i, { key: e.target.value })}
                  placeholder={placeholderKey}
                  className={`w-1/4 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs focus:outline-none ${locked ? "text-[var(--module-accent)]/80 opacity-70 cursor-not-allowed" : "text-slate-200 focus:border-[var(--module-accent)]/60"}`}
                  title={locked ? "继承自项目模板：名称在项目设置中修改" : undefined}
                />
                <div className="flex-1">
                  <VarInput
                    value={kv.value}
                    disabled={locked}
                    envVars={envVars}
                    onChange={(v) => update(i, { value: v })}
                    placeholder={placeholderValue}
                    className={`w-full bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs focus:outline-none ${locked ? "text-[var(--module-accent)]/80 opacity-70 cursor-not-allowed" : "text-slate-200 focus:border-[var(--module-accent)]/60"}`}
                  />
                </div>
                {withDescription && (
                  <input
                    value={kv.description ?? ""}
                    disabled={locked}
                    onChange={(e) => update(i, { description: e.target.value })}
                    placeholder="描述"
                    className="w-1/4 hidden lg:block bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-500 focus:outline-none focus:border-[var(--module-accent)]/60 disabled:opacity-50"
                  />
                )}
                <button
                  type="button"
                  disabled={locked}
                  onClick={() => onChange(items.filter((_, idx) => idx !== i))}
                  className={`p-1 shrink-0 ${locked ? "text-slate-700 cursor-not-allowed" : "text-slate-500 hover:text-rose-400 cursor-pointer"}`}
                  title={locked ? "模板继承项不可删除" : "删除"}
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            );
          })}
          <button
            type="button"
            onClick={() => onChange([...items, { key: "", value: "", enabled: true, description: "" }])}
            className="flex items-center gap-1 text-[11px] text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
          >
            <Plus className="w-3 h-3" /> 添加
          </button>
        </div>
      )}
    </div>
  );
}

// ─── form-data 编辑器（text / 文件） ───
export function FormDataEditor({ items, onChange, envVars = {} }: { items: FormDataItem[]; onChange: (items: FormDataItem[]) => void; envVars?: Record<string, unknown> }) {
  const update = (i: number, patch: Partial<FormDataItem>) => {
    const target = items[i];
    // 模板继承项不允许修改（名称/值/类型/启用状态均由项目模板控制）
    if (target?.from_template && ("key" in patch || "value" in patch || "kind" in patch || "enabled" in patch || "file_path" in patch)) {
      return;
    }
    onChange(items.map((kv, idx) => (idx === i ? { ...kv, ...patch } : kv)));
  };
  const pickFile = async (i: number) => {
    const f = await openDialog({ multiple: false });
    if (f) update(i, { file_path: String(f), kind: "file" });
  };
  return (
    <div className="space-y-1">
      {items.map((kv, i) => {
        const locked = !!kv.from_template;
        return (
          <div key={i} className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={kv.enabled}
              disabled={locked}
              onChange={(e) => update(i, { enabled: e.target.checked })}
              className="accent-[var(--module-accent)] shrink-0 disabled:opacity-40"
              title={locked ? "继承自项目通用 Body 模板（只读）" : undefined}
            />
            {locked && <Lock className="w-3 h-3 shrink-0 text-[var(--module-accent)]" aria-label="继承自项目模板（只读）" />}
            <input
              value={kv.key}
              disabled={locked}
              onChange={(e) => update(i, { key: e.target.value })}
              placeholder="字段名"
              className={`w-1/4 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs focus:outline-none ${locked ? "text-[var(--module-accent)]/80 opacity-70 cursor-not-allowed" : "text-slate-200"}`}
              title={locked ? "继承自项目模板：名称在项目设置中修改" : undefined}
            />
            {kv.kind === "file" ? (
              <button
                type="button"
                disabled={locked}
                onClick={() => pickFile(i)}
                className="flex-1 flex items-center gap-1.5 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer truncate disabled:opacity-50"
                title={kv.file_path}
              >
                <FileText className="w-3 h-3 shrink-0" />
                <span className="truncate">{kv.file_path || "选择文件…"}</span>
              </button>
            ) : (
              <VarInput
                value={kv.value}
                envVars={envVars}
                disabled={locked}
                onChange={(v) => update(i, { value: v })}
                placeholder="值（支持 {{$guid}} 等）"
                className={`flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs focus:outline-none ${locked ? "text-[var(--module-accent)]/80 opacity-70 cursor-not-allowed" : "text-slate-200"}`}
              />
            )}
            <select
              value={kv.kind}
              disabled={locked}
              onChange={(e) => update(i, { kind: e.target.value as "text" | "file" })}
              className="bg-black/30 border border-white/10 rounded-md px-1 py-1 text-[10px] text-slate-300 cursor-pointer disabled:opacity-40"
            >
              <option value="text">Text</option>
              <option value="file">File</option>
            </select>
            <input
              value={kv.description ?? ""}
              disabled={locked}
              onChange={(e) => update(i, { description: e.target.value })}
              placeholder="描述"
              className="w-1/4 hidden lg:block bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-500 focus:outline-none disabled:opacity-50"
            />
            <button
              type="button"
              disabled={locked}
              onClick={() => onChange(items.filter((_, idx) => idx !== i))}
              className={`p-1 shrink-0 ${locked ? "text-slate-700 cursor-not-allowed" : "text-slate-500 hover:text-rose-400 cursor-pointer"}`}
              title={locked ? "模板继承项不可删除" : "删除"}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          </div>
        );
      })}
      <button
        type="button"
        onClick={() => onChange([...items, { key: "", value: "", enabled: true, kind: "text", file_path: "", description: "" }])}
        className="flex items-center gap-1 text-[11px] text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
      >
        <Plus className="w-3 h-3" /> 添加字段
      </button>
    </div>
  );
}

// ─── 认证面板 ───
export function AuthPanel({ auth, onChange }: { auth: Authorization; onChange: (a: Authorization) => void }) {
  const set = (patch: Partial<Authorization>) => onChange({ ...auth, ...patch });
  return (
    <div className="space-y-2">
      <div className="flex gap-1">
        {AUTH_TYPES.map((t) => (
          <button
            key={t.value}
            type="button"
            onClick={() => set({ type: t.value })}
            className={`px-2.5 py-1 text-[11px] rounded-md cursor-pointer ${auth.type === t.value ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
          >
            {t.label}
          </button>
        ))}
      </div>
      {auth.type === "basic" && (
        <div className="grid grid-cols-2 gap-2">
          <input value={auth.username} onChange={(e) => set({ username: e.target.value })} placeholder="用户名" className="bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200" />
          <input type="password" value={auth.password} onChange={(e) => set({ password: e.target.value })} placeholder="密码" className="bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200" />
        </div>
      )}
      {auth.type === "bearer" && (
        <input value={auth.token} onChange={(e) => set({ token: e.target.value })} placeholder="Token（支持 {{token}} 变量）" className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200" />
      )}
      {auth.type === "jwt" && (
        <input value={auth.jwt_token} onChange={(e) => set({ jwt_token: e.target.value })} placeholder="JWT Token（支持 {{jwt}} 变量）" className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200" />
      )}
      {auth.type === "apiKey" && (
        <div className="grid grid-cols-[auto_1fr_1fr] gap-2 items-center">
          <select value={auth.api_key_in} onChange={(e) => set({ api_key_in: e.target.value as "header" | "query" })} className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1.5 text-xs text-slate-300 cursor-pointer">
            <option value="header">Header</option>
            <option value="query">Query</option>
          </select>
          <input value={auth.api_key_name} onChange={(e) => set({ api_key_name: e.target.value })} placeholder="Key 名（如 X-API-Key）" className="bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200" />
          <input value={auth.api_key_value} onChange={(e) => set({ api_key_value: e.target.value })} placeholder="Key 值" className="bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200" />
        </div>
      )}
      {auth.type === "none" && <div className="text-[10px] text-slate-500">不附加认证信息。</div>}
    </div>
  );
}

// ─── 设置面板 ───
export function SettingsPanel({ settings, timeoutMs, onChange, onTimeout }: {
  settings: RequestSettings;
  timeoutMs: number;
  onChange: (s: RequestSettings) => void;
  onTimeout: (ms: number) => void;
}) {
  const set = (patch: Partial<RequestSettings>) => onChange({ ...settings, ...patch });
  const Toggle = ({ label, value, on }: { label: string; value: boolean; on: (v: boolean) => void }) => (
    <label className="flex items-center gap-2 py-1 cursor-pointer">
      <button
        type="button"
        onClick={() => on(!value)}
        className={`w-8 h-4.5 rounded-full transition-colors relative shrink-0 ${value ? "bg-[var(--module-accent)]" : "bg-white/15"}`}
        style={{ height: 18 }}
      >
        <span className={`absolute top-0.5 w-3.5 h-3.5 rounded-full bg-white transition-all ${value ? "left-4" : "left-0.5"}`} />
      </button>
      <span className="text-xs text-slate-300">{label}</span>
    </label>
  );
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <span className="text-[10px] text-slate-500 w-32">HTTP 版本</span>
        <select value={settings.http_version} onChange={(e) => set({ http_version: e.target.value as RequestSettings["http_version"] })} className="bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 cursor-pointer">
          <option value="auto">Auto</option>
          <option value="http1">HTTP/1.x</option>
          <option value="http2">HTTP/2</option>
        </select>
      </div>
      <div className="flex items-center gap-2">
        <span className="text-[10px] text-slate-500 w-32">超时时间(ms)</span>
        <input type="number" min={100} value={timeoutMs} onChange={(e) => onTimeout(Number(e.target.value))} className="w-32 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200" />
      </div>
      <div className="grid grid-cols-2 gap-x-4">
        <Toggle label="启用 SSL 证书验证" value={settings.verify_ssl} on={(v) => set({ verify_ssl: v })} />
        <Toggle label="自动跟随重定向" value={settings.follow_redirects} on={(v) => set({ follow_redirects: v })} />
        <Toggle label="重定向时保持原始 HTTP 方法" value={settings.follow_original_method} on={(v) => set({ follow_original_method: v })} />
        <Toggle label="重定向时携带 Authorization" value={settings.follow_authorization_header} on={(v) => set({ follow_authorization_header: v })} />
        <Toggle label="重定向时移除 Referer" value={settings.remove_referer_on_redirect} on={(v) => set({ remove_referer_on_redirect: v })} />
        <Toggle label="启用严格 HTTP 解析器" value={settings.strict_http_parser} on={(v) => set({ strict_http_parser: v })} />
      </div>
    </div>
  );
}