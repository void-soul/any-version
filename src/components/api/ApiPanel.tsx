import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Send, Play, Plus, Trash2, Save, FolderPlus, FilePlus2, Settings2,
  ChevronDown, ChevronRight, Download, Upload, FlaskConical, Gauge,
  BookOpen, ListChecks, Copy, Check, Loader2, Pencil, X, Database,
  Folder, KeyRound, Cookie, SlidersHorizontal, ArrowDown,
  RefreshCcw, Wrench, MoreHorizontal, FileText, TestTube2, Braces,
  Link2, StickyNote, Star, History, Eraser, ListTree,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type {
  ApiProject, ApiEnvironment, ApiModule, ApiEndpoint, KeyValueItem, FormDataItem,
  Authorization, RequestSettings, PresetHeaderSet,
  SendRequestInput, SendRequestOutput, UnitTest,
  UnitTestRunOutput, LoadTestConfig, LoadTestRun, LoadRunStatus, LoadTestReport,
  ApiHistoryEntry,
} from "./types";
import {
  METHODS, BODY_TYPES, AUTH_TYPES, ASSERTION_TYPES, RANDOM_VARIABLES,
  COMMON_AUTO_HEADERS, defaultAuthorization, defaultSettings,
} from "./types";

const ACCENT = "var(--module-accent)";

function methodIcon(method: string) {
  switch (method) {
    case "GET": return ArrowDown;
    case "POST": return Plus;
    case "PUT": return RefreshCcw;
    case "DELETE": return Trash2;
    case "PATCH": return Wrench;
    default: return MoreHorizontal;
  }
}

function emptyEndpoint(projectId: string, moduleId: string | null): ApiEndpoint {
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

function fmtTime(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

function prettyJson(text: string): string {
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
function highlightJsonTokens(text: string): Array<{ cls: string; content: string }> | null {
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
function ResponseBody({ body, mode }: { body: string; mode: "pretty" | "raw" }) {
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
function KvEditor({ items, onChange, placeholderKey = "名称", placeholderValue = "值", withDescription = true }: {
  items: KeyValueItem[];
  onChange: (items: KeyValueItem[]) => void;
  placeholderKey?: string;
  placeholderValue?: string;
  withDescription?: boolean;
}) {
  const [mode, setMode] = useState<"kv" | "bulk">("kv");
  const [bulkText, setBulkText] = useState(items.map((kv) => kv.enabled ? `${kv.key}:${kv.value}` : `// ${kv.key}:${kv.value}`).join("\n"));

  const update = (i: number, patch: Partial<KeyValueItem>) => {
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
    onChange(parsed);
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
          {items.map((kv, i) => (
            <div key={i} className="flex items-center gap-1.5">
              <input
                type="checkbox"
                checked={kv.enabled}
                onChange={(e) => update(i, { enabled: e.target.checked })}
                className="accent-[var(--module-accent)] shrink-0"
                title={kv.enabled ? "启用" : "禁用"}
              />
              <input
                value={kv.key}
                onChange={(e) => update(i, { key: e.target.value })}
                placeholder={placeholderKey}
                className="w-1/4 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
              />
              <input
                value={kv.value}
                onChange={(e) => update(i, { value: e.target.value })}
                placeholder={placeholderValue}
                className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
              />
              {withDescription && (
                <input
                  value={kv.description ?? ""}
                  onChange={(e) => update(i, { description: e.target.value })}
                  placeholder="描述"
                  className="w-1/4 hidden lg:block bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-500 focus:outline-none focus:border-[var(--module-accent)]/60"
                />
              )}
              <button
                type="button"
                onClick={() => onChange(items.filter((_, idx) => idx !== i))}
                className="p-1 text-slate-500 hover:text-rose-400 cursor-pointer shrink-0"
                title="删除"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
          ))}
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
function FormDataEditor({ items, onChange }: { items: FormDataItem[]; onChange: (items: FormDataItem[]) => void }) {
  const update = (i: number, patch: Partial<FormDataItem>) => {
    onChange(items.map((kv, idx) => (idx === i ? { ...kv, ...patch } : kv)));
  };
  const pickFile = async (i: number) => {
    const f = await openDialog({ multiple: false });
    if (f) update(i, { file_path: String(f), kind: "file" });
  };
  return (
    <div className="space-y-1">
      {items.map((kv, i) => (
        <div key={i} className="flex items-center gap-1.5">
          <input
            type="checkbox"
            checked={kv.enabled}
            onChange={(e) => update(i, { enabled: e.target.checked })}
            className="accent-[var(--module-accent)] shrink-0"
          />
          <input
            value={kv.key}
            onChange={(e) => update(i, { key: e.target.value })}
            placeholder="字段名"
            className="w-1/4 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-200 focus:outline-none"
          />
          {kv.kind === "file" ? (
            <button
              type="button"
              onClick={() => pickFile(i)}
              className="flex-1 flex items-center gap-1.5 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer truncate"
              title={kv.file_path}
            >
              <FileText className="w-3 h-3 shrink-0" />
              <span className="truncate">{kv.file_path || "选择文件…"}</span>
            </button>
          ) : (
            <input
              value={kv.value}
              onChange={(e) => update(i, { value: e.target.value })}
              placeholder="值（支持 {{$guid}} 等）"
              className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-200 focus:outline-none"
            />
          )}
          <select
            value={kv.kind}
            onChange={(e) => update(i, { kind: e.target.value as "text" | "file" })}
            className="bg-black/30 border border-white/10 rounded-md px-1 py-1 text-[10px] text-slate-300 cursor-pointer"
          >
            <option value="text">Text</option>
            <option value="file">File</option>
          </select>
          <input
            value={kv.description ?? ""}
            onChange={(e) => update(i, { description: e.target.value })}
            placeholder="描述"
            className="w-1/4 hidden lg:block bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-500 focus:outline-none"
          />
          <button
            type="button"
            onClick={() => onChange(items.filter((_, idx) => idx !== i))}
            className="p-1 text-slate-500 hover:text-rose-400 cursor-pointer shrink-0"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
      ))}
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
function AuthPanel({ auth, onChange }: { auth: Authorization; onChange: (a: Authorization) => void }) {
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
function SettingsPanel({ settings, timeoutMs, onChange, onTimeout }: {
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

// ─── 预设 Headers 弹窗 ───
function PresetHeadersModal({ projectId, sets, onClose, onChanged }: {
  projectId: string;
  sets: PresetHeaderSet[];
  onClose: () => void;
  onChanged: () => void;
}) {
  const [local, setLocal] = useState<PresetHeaderSet[]>(sets);
  const update = (i: number, patch: Partial<PresetHeaderSet>) => {
    setLocal(local.map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
  };
  const add = () => {
    setLocal([...local, { id: "", project_id: projectId, name: `预设 ${local.length + 1}`, headers: [], created_at: "" }]);
  };
  const saveAll = async () => {
    for (const s of local) {
      await invoke("api_save_preset_headers", { set: s });
    }
    onChanged();
    onClose();
  };
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-[620px] max-h-[80vh] overflow-hidden glass-panel rounded-2xl border border-white/10 shadow-2xl flex flex-col" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Link2 className="w-4 h-4" style={{ color: ACCENT }} /> 预设 Headers（项目级）
          </div>
          <button onClick={onClose} className="p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="flex-1 overflow-y-auto p-4 space-y-3">
          {local.map((s, i) => (
            <div key={i} className="rounded-xl border border-white/10 bg-black/20 p-3 space-y-2">
              <div className="flex items-center gap-2">
                <input value={s.name} onChange={(e) => update(i, { name: e.target.value })} className="flex-1 bg-transparent border border-white/10 rounded-md px-2 py-1 text-xs font-semibold text-slate-100 focus:outline-none" />
                <button onClick={() => setLocal(local.filter((_, idx) => idx !== i))} className="p-1 text-slate-500 hover:text-rose-400 cursor-pointer"><Trash2 className="w-3.5 h-3.5" /></button>
              </div>
              <KvEditor items={s.headers} onChange={(h) => update(i, { headers: h })} placeholderKey="Header 名" placeholderValue="值" withDescription={false} />
            </div>
          ))}
          <button onClick={add} className="flex items-center gap-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
            <Plus className="w-3.5 h-3.5" /> 新建预设集合
          </button>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button onClick={saveAll} className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer" style={{ background: ACCENT }}>保存</button>
        </div>
      </div>
    </div>
  );
}

// ─── 变量集合弹窗（矩阵编辑：每列一个环境，每行一个变量名）───
function EnvModal({ projectId, envs, activeEnvId, onClose, onChanged }: {
  projectId: string;
  envs: ApiEnvironment[];
  activeEnvId: string | null;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [local, setLocal] = useState<ApiEnvironment[]>(envs);
  const [active, setActive] = useState<string | null>(activeEnvId);

  const updateEnv = (i: number, patch: Partial<ApiEnvironment>) => {
    setLocal(prev => prev.map((e, idx) => (idx === i ? { ...e, ...patch } : e)));
  };

  const addEnv = async () => {
    const created = await invoke<ApiEnvironment>("api_create_environment", {
      projectId, name: `环境${local.length + 1}`, variables: {},
    });
    setLocal(prev => [...prev, created]);
    if (!active) setActive(created.id);
  };

  const saveAll = async () => {
    for (const e of local) {
      await invoke("api_update_environment", { env: e });
    }
    if (active && active !== activeEnvId) {
      await invoke("api_set_active_env", { projectId, envId: active });
    }
    onChanged();
    onClose();
  };

  // 变量名并集（保持首次出现的顺序），保证各环境列对齐
  const allKeys = (() => {
    const seen: string[] = [];
    for (const e of local) for (const k of Object.keys(e.variables)) if (!seen.includes(k)) seen.push(k);
    return seen;
  })();

  const setVar = (envIdx: number, key: string, value: string) => {
    setLocal(prev => prev.map((e, i) => (i === envIdx ? { ...e, variables: { ...e.variables, [key]: value } } : e)));
  };

  const renameVar = (oldKey: string, newKey: string) => {
    if (!newKey.trim() || newKey === oldKey) return;
    setLocal(prev => prev.map(e => {
      const vars = { ...e.variables };
      if (oldKey in vars) { vars[newKey] = vars[oldKey]; delete vars[oldKey]; }
      return { ...e, variables: vars };
    }));
  };

  const addRow = () => {
    const key = `变量${allKeys.length + 1}`;
    setLocal(prev => prev.map(e => ({ ...e, variables: { ...e.variables, [key]: "" } })));
  };

  const deleteRow = (key: string) => {
    setLocal(prev => prev.map(e => {
      const vars = { ...e.variables }; delete vars[key]; return { ...e, variables: vars };
    }));
  };

  const cellCls = "bg-black/30 border border-white/10 rounded-md px-2 py-1 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-[860px] max-w-[95vw] max-h-[82vh] overflow-hidden glass-panel rounded-2xl border border-white/10 shadow-2xl flex flex-col" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Database className="w-4 h-4" style={{ color: ACCENT }} /> 变量集合（环境）
          </div>
          <div className="flex items-center gap-2">
            <select
              value={active ?? ""}
              onChange={(e) => setActive(e.target.value)}
              className="bg-black/30 border border-white/10 rounded-md px-2 py-1 text-[11px] text-slate-200 focus:outline-none"
              title="当前生效的环境（请求变量取自该列）"
            >
              {local.map(e => <option key={e.id} value={e.id}>{e.name}</option>)}
            </select>
            <button onClick={onClose} className="p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
          </div>
        </div>
        <div className="flex-1 overflow-auto p-4">
          {local.length === 0 ? (
            <div className="py-10 text-center text-xs text-slate-500">暂无环境，点击下方“新建环境”创建第一列</div>
          ) : (
            <table className="w-full border-separate border-spacing-0 text-xs">
              <thead>
                <tr>
                  <th className="sticky left-0 z-10 bg-[#0d1524] px-2 py-1.5 text-left text-[10px] font-semibold text-slate-400 border-b border-white/10">变量名</th>
                  {local.map((e, i) => (
                    <th key={e.id} className={`px-1.5 py-1 border-b border-white/10 ${e.id === active ? "bg-[color-mix(in_srgb,var(--module-accent)_10%,transparent)]" : "bg-black/20"}`}>
                      <div className="flex items-center gap-1">
                        {e.id === active && <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--module-accent)" }} title="当前生效" />}
                        <input
                          value={e.name}
                          onChange={(ev) => updateEnv(i, { name: ev.target.value })}
                          className={`w-full min-w-[110px] bg-transparent border ${e.id === active ? "border-[var(--module-accent)]/50" : "border-white/10"} rounded-md px-1.5 py-0.5 text-[11px] font-semibold text-slate-100 focus:outline-none`}
                        />
                        <button
                          onClick={() => setActive(e.id)}
                          className="shrink-0 p-0.5 text-[9px] text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                          title="设为当前环境"
                        >当前</button>
                        <button
                          onClick={async () => {
                            await invoke("api_delete_environment", { envId: e.id });
                            setLocal(prev => prev.filter((_, idx) => idx !== i));
                            if (active === e.id) setActive(local.find(x => x.id !== e.id)?.id ?? null);
                          }}
                          className="shrink-0 p-0.5 text-slate-500 hover:text-rose-400 cursor-pointer"
                          title="删除此环境列"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {allKeys.map((key) => (
                  <tr key={key} className="group">
                    <td className="sticky left-0 z-10 bg-[#0d1524] px-2 py-1 border-b border-white/5">
                      <div className="flex items-center gap-1">
                        <input
                          defaultValue={key}
                          onBlur={(e) => renameVar(key, e.target.value.trim())}
                          className={`w-full min-w-[110px] bg-transparent border border-transparent rounded-md px-1 py-0.5 text-[11px] font-medium text-slate-200 focus:border-[var(--module-accent)]/50 focus:outline-none`}
                          title="编辑变量名（失焦后同步到所有环境）"
                        />
                        <button
                          onClick={() => deleteRow(key)}
                          className="shrink-0 p-0.5 text-slate-600 opacity-0 group-hover:opacity-100 hover:text-rose-400 cursor-pointer"
                          title="删除此行（所有环境）"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    </td>
                    {local.map((e, ei) => (
                      <td key={e.id} className={`px-1.5 py-1 border-b border-white/5 ${e.id === active ? "bg-[color-mix(in_srgb,var(--module-accent)_4%,transparent)]" : ""}`}>
                        <input
                          value={String(e.variables[key] ?? "")}
                          onChange={(ev) => setVar(ei, key, ev.target.value)}
                          placeholder="{{$guid}} 等随机变量"
                          className={`${cellCls} w-full min-w-[120px]`}
                        />
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <div className="mt-3 flex items-center gap-2">
            <button onClick={addRow} className="flex items-center gap-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
              <Plus className="w-3.5 h-3.5" /> 添加变量行
            </button>
            <button onClick={addEnv} className="flex items-center gap-1 text-xs text-slate-400 hover:text-[var(--module-accent)] cursor-pointer">
              <Plus className="w-3.5 h-3.5" /> 新建环境列
            </button>
          </div>
          <div className="mt-2 text-[10px] text-slate-500 space-y-0.5">
            提示：每组环境代表一列，每行一个变量名；不同环境的变量名自动对齐，只改值即可。
            请求中通过 <code className="text-[var(--module-accent)]">{"{{变量名}}"}</code> 引用当前生效环境的值。
          </div>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button onClick={saveAll} className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer" style={{ background: ACCENT }}>
            保存
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 项目新建/编辑弹窗 ───
function ProjectModal({ project, onClose, onSave }: {
  project: ApiProject | null;
  onClose: () => void;
  onSave: (name: string, description: string, commonHeaders: KeyValueItem[], commonParams: KeyValueItem[]) => void;
}) {
  const [name, setName] = useState(project?.name ?? "");
  const [description, setDescription] = useState(project?.description ?? "");
  const [commonHeaders, setCommonHeaders] = useState<KeyValueItem[]>(project?.common_headers ?? []);
  const [commonParams, setCommonParams] = useState<KeyValueItem[]>(project?.common_params ?? []);
  const editing = !!project;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-[560px] glass-panel rounded-2xl border border-white/10 shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-3 border-b border-white/10">
          <FlaskConical className="w-4 h-4" style={{ color: ACCENT }} />
          <span className="text-sm font-semibold text-white">{editing ? "编辑项目" : "新建项目"}</span>
          <button onClick={onClose} className="ml-auto p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="max-h-[60vh] overflow-y-auto p-4 space-y-3">
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">项目名称</span>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如：电商后台、开放平台…"
              onKeyDown={(e) => e.key === "Enter" && name.trim() && onSave(name.trim(), description, commonHeaders, commonParams)}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-100 focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">简介</span>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="这个项目面向什么场景？包含哪些模块？…"
              rows={2}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 resize-none focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
          <div>
            <div className="flex items-center gap-1.5 mb-1">
              <Link2 className="w-3 h-3" style={{ color: ACCENT }} />
              <span className="text-[11px] text-slate-400">通用 Headers（接口模板）</span>
              <span className="text-[9px] text-slate-600">新建接口时自动附加</span>
            </div>
            <KvEditor items={commonHeaders} onChange={setCommonHeaders} placeholderKey="Header 名" placeholderValue="值" withDescription={false} />
          </div>
          <div>
            <div className="flex items-center gap-1.5 mb-1">
              <ListTree className="w-3 h-3" style={{ color: ACCENT }} />
              <span className="text-[11px] text-slate-400">通用 Params（接口模板）</span>
              <span className="text-[9px] text-slate-600">新建接口时自动附加</span>
            </div>
            <KvEditor items={commonParams} onChange={setCommonParams} placeholderKey="参数名" placeholderValue="值" withDescription={false} />
          </div>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button
            onClick={() => name.trim() && onSave(name.trim(), description, commonHeaders, commonParams)}
            disabled={!name.trim()}
            className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer disabled:opacity-50"
            style={{ background: ACCENT }}
          >
            {editing ? "保存" : "创建"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 模块新建/编辑弹窗 ───
function ModuleModal({ module, onClose, onSave }: {
  module: ApiModule | null;
  onClose: () => void;
  onSave: (name: string, description: string) => void;
}) {
  const [name, setName] = useState(module?.name ?? "");
  const [description, setDescription] = useState(module?.description ?? "");
  const editing = !!module;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="w-[440px] glass-panel rounded-2xl border border-white/10 shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 px-4 py-3 border-b border-white/10">
          <Folder className="w-4 h-4" style={{ color: ACCENT }} />
          <span className="text-sm font-semibold text-white">{editing ? "编辑模块" : "新建模块"}</span>
          <button onClick={onClose} className="ml-auto p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="p-4 space-y-3">
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">模块名称</span>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如：订单、用户、商品…"
              onKeyDown={(e) => e.key === "Enter" && name.trim() && onSave(name.trim(), description)}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-100 focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
          <label className="block">
            <span className="text-[11px] text-slate-400 mb-1 block">简介</span>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="这个模块包含哪些接口？用途说明…"
              rows={3}
              className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 resize-none focus:outline-none focus:border-[var(--module-accent)]/60"
            />
          </label>
        </div>
        <div className="flex justify-end gap-2 px-4 py-3 border-t border-white/10">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">取消</button>
          <button
            onClick={() => name.trim() && onSave(name.trim(), description)}
            disabled={!name.trim()}
            className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer disabled:opacity-50"
            style={{ background: ACCENT }}
          >
            {editing ? "保存" : "创建"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 压测报告视图 ───
function StatCard({ label, value, accent }: { label: string; value: string; accent?: string }) {
  return (
    <div className="rounded-lg border border-white/10 bg-black/25 px-2.5 py-1.5 text-center">
      <div className="text-[9px] text-slate-500">{label}</div>
      <div className="text-sm font-semibold" style={{ color: accent ?? "#e2e8f0" }}>{value}</div>
    </div>
  );
}

function TimelineChart({ report }: { report: LoadTestReport }) {
  const maxQps = Math.max(1, ...report.timeline.map((t) => t.qps));
  const maxFail = Math.max(1, ...report.timeline.map((t) => t.failed));
  const maxMs = Math.max(1, ...report.timeline.map((t) => t.avg_ms));
  const width = 560;
  const height = 130;
  const chartBottom = height - 22; // 图表区底部（留出图例）
  const chartTop = 4;
  const usableH = chartBottom - chartTop;
  const pad = 4;
  const n = Math.max(1, report.timeline.length);
  const stepX = (width - pad * 2) / n;
  const bw = Math.max(1, stepX - 1);
  const cx = (i: number) => pad + i * stepX + stepX / 2;

  // 平均延迟折线
  const latencyPoints = report.timeline
    .map((t, i) => `${cx(i).toFixed(1)},${(chartBottom - (t.avg_ms / maxMs) * usableH).toFixed(1)}`)
    .join(" ");
  // 成功率折线（0~1 映射到图表区上半段）
  const successPoints = report.timeline
    .map((t, i) => {
      const total = t.success + t.failed;
      const rate = total > 0 ? t.success / total : 1;
      return `${cx(i).toFixed(1)},${(chartBottom - rate * usableH).toFixed(1)}`;
    })
    .join(" ");

  return (
    <svg viewBox={`0 0 ${width} ${height}`} className="w-full h-auto">
      {/* QPS 柱 + 失败柱 */}
      {report.timeline.map((t, i) => {
        const x = pad + i * stepX;
        const hQps = (t.qps / maxQps) * usableH;
        const hFail = (t.failed / maxFail) * usableH;
        return (
          <g key={i}>
            <rect x={x} y={chartBottom - hQps} width={bw} height={hQps} fill="rgba(6,182,212,0.35)" />
            {t.failed > 0 && <rect x={x} y={chartBottom - hFail} width={bw} height={hFail} fill="rgba(244,63,94,0.55)" />}
          </g>
        );
      })}
      {/* 成功率曲线 */}
      {n > 1 && <polyline points={successPoints} fill="none" stroke="#34d399" strokeWidth="1.4" strokeOpacity="0.85" strokeLinejoin="round" />}
      {/* 平均延迟曲线 */}
      {n > 1 && <polyline points={latencyPoints} fill="none" stroke="#fbbf24" strokeWidth="1.4" strokeOpacity="0.9" strokeLinejoin="round" />}
      <line x1={0} y1={chartBottom} x2={width} y2={chartBottom} stroke="rgba(255,255,255,0.15)" />
      {/* 图例 */}
      <g fontSize="9" fill="#64748b">
        <rect x={4} y={chartBottom + 8} width={8} height={6} fill="rgba(6,182,212,0.5)" rx={1} />
        <text x={15} y={chartBottom + 14}>QPS</text>
        <rect x={44} y={chartBottom + 8} width={8} height={6} fill="rgba(244,63,94,0.6)" rx={1} />
        <text x={55} y={chartBottom + 14}>失败</text>
        <line x1={86} y1={chartBottom + 11} x2={100} y2={chartBottom + 11} stroke="#34d399" strokeWidth="1.4" />
        <text x={104} y={chartBottom + 14}>成功率</text>
        <line x1={142} y1={chartBottom + 11} x2={156} y2={chartBottom + 11} stroke="#fbbf24" strokeWidth="1.4" />
        <text x={160} y={chartBottom + 14}>平均延迟</text>
        <text x={width - 4} y={chartBottom + 14} textAnchor="end">峰值 {maxQps.toFixed(0)} QPS · 峰值延迟 {maxMs.toFixed(0)}ms</text>
      </g>
    </svg>
  );
}

function LoadReportView({ report }: { report: LoadTestReport }) {
  const errRate = (report.error_rate * 100).toFixed(2);
  return (
    <div className="space-y-3">
      <div className="grid grid-cols-4 gap-1.5">
        <StatCard label="总请求" value={String(report.total)} />
        <StatCard label="成功" value={String(report.success)} accent="#34d399" />
        <StatCard label="失败" value={String(report.failed)} accent={report.failed > 0 ? "#fb7185" : undefined} />
        <StatCard label="错误率" value={`${errRate}%`} accent={report.error_rate > 0.05 ? "#fb7185" : "#34d399"} />
        <StatCard label="QPS 平均" value={report.qps_avg.toFixed(1)} accent="#22d3ee" />
        <StatCard label="QPS 峰值" value={report.qps_max.toFixed(1)} />
        <StatCard label="平均延迟" value={`${report.latency_avg_ms.toFixed(1)}ms`} />
        <StatCard label="最大延迟" value={`${report.latency_max_ms.toFixed(1)}ms`} />
        <StatCard label="p50" value={`${report.latency_p50_ms.toFixed(1)}ms`} />
        <StatCard label="p90" value={`${report.latency_p90_ms.toFixed(1)}ms`} />
        <StatCard label="p95" value={`${report.latency_p95_ms.toFixed(1)}ms`} />
        <StatCard label="p99" value={`${report.latency_p99_ms.toFixed(1)}ms`} />
      </div>
      <div className="flex flex-wrap gap-1.5">
        {report.status_codes.map(([code, count]) => (
          <span key={code} className={`text-[10px] px-2 py-0.5 rounded-full border ${code >= 500 ? "text-rose-300 border-rose-500/30 bg-rose-500/10" : code >= 400 ? "text-amber-300 border-amber-500/30 bg-amber-500/10" : "text-emerald-300 border-emerald-500/30 bg-emerald-500/10"}`}>
            {code} × {count}
          </span>
        ))}
      </div>
      <TimelineChart report={report} />
    </div>
  );
}

// ─── 主面板 ───
export default function ApiPanel() {
  const [projects, setProjects] = useState<ApiProject[]>([]);
  const [activeProjectId, setActiveProjectId] = useState<string | null>(null);
  const [envs, setEnvs] = useState<ApiEnvironment[]>([]);
  const [activeEnvId, setActiveEnvId] = useState<string | null>(null);
  const [modules, setModules] = useState<ApiModule[]>([]);
  const [endpoints, setEndpoints] = useState<ApiEndpoint[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<ApiEndpoint | null>(null);
  const [tests, setTests] = useState<UnitTest[]>([]);
  const [testResults, setTestResults] = useState<Record<string, UnitTestRunOutput>>({});
  const [testing, setTesting] = useState(false);
  const [loadRuns, setLoadRuns] = useState<LoadTestRun[]>([]);
  const [response, setResponse] = useState<SendRequestOutput | null>(null);
  const [sending, setSending] = useState(false);
  const [activeTab, setActiveTab] = useState<"request" | "tests" | "load" | "docs">("request");
  const [subTab, setSubTab] = useState<"params" | "auth" | "headers" | "body" | "settings" | "cookies">("params");
  const [envModal, setEnvModal] = useState(false);
  const [importModal, setImportModal] = useState(false);
  const [presetModal, setPresetModal] = useState(false);
  const [presetSets, setPresetSets] = useState<PresetHeaderSet[]>([]);
  const [hideCommonHeaders, setHideCommonHeaders] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [copied, setCopied] = useState(false);
  const [bodyMode, setBodyMode] = useState<"pretty" | "raw">("pretty");
  const [loadConfig, setLoadConfig] = useState<LoadTestConfig>({ concurrency: 10, duration_secs: 10, ramp_up_secs: 0, rps_limit: 0 });
  const [runningRunId, setRunningRunId] = useState<string | null>(null);
  const [loadStatus, setLoadStatus] = useState<LoadRunStatus | null>(null);
  const [commentDraft, setCommentDraft] = useState("");
  const pollRef = useRef<number | null>(null);

  const variables = useMemo(() => {
    const env = envs.find((e) => e.id === activeEnvId);
    return env?.variables ?? {};
  }, [envs, activeEnvId]);

  // 初始化
  useEffect(() => {
    invoke("api_init").then(() => loadProjects());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadProjects = useCallback(async () => {
    const list = await invoke<ApiProject[]>("api_list_projects");
    setProjects(list);
    if (list.length > 0 && !list.some((p) => p.id === activeProjectId)) {
      setActiveProjectId(list[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 切换项目：加载环境/模块/接口/预设
  useEffect(() => {
    if (!activeProjectId) return;
    (async () => {
      const [envList, modList, epList, presetList] = await Promise.all([
        invoke<ApiEnvironment[]>("api_list_environments", { projectId: activeProjectId }),
        invoke<ApiModule[]>("api_list_modules", { projectId: activeProjectId }),
        invoke<ApiEndpoint[]>("api_list_endpoints", { projectId: activeProjectId, moduleId: null }),
        invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId: activeProjectId }),
      ]);
      setEnvs(envList);
      setModules(modList);
      setEndpoints(epList);
      setPresetSets(presetList);
      const project = projects.find((p) => p.id === activeProjectId);
      const active = project?.active_env_id && envList.some((e) => e.id === project.active_env_id)
        ? project.active_env_id
        : envList[0]?.id ?? null;
      setActiveEnvId(active);
      setSelectedId(null);
      setDraft(null);
      setResponse(null);
      loadHistory();
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeProjectId]);

  // 选择接口
  useEffect(() => {
    if (!selectedId) return;
    (async () => {
      const ep = await invoke<ApiEndpoint>("api_get_endpoint", { endpointId: selectedId });
      setDraft(ep);
      setCommentDraft(ep.response_comment ?? "");
      const t = await invoke<UnitTest[]>("api_list_unit_tests", { endpointId: selectedId });
      setTests(t);
      const runs = await invoke<LoadTestRun[]>("api_list_load_runs", { endpointId: selectedId });
      setLoadRuns(runs);
      setResponse(null);
      setTestResults({});
      setActiveTab("request");
    })();
  }, [selectedId]);

  // 压测轮询
  useEffect(() => {
    if (!runningRunId) return;
    let alive = true;
    const tick = async () => {
      try {
        const st = await invoke<LoadRunStatus>("api_load_run_status", { runId: runningRunId });
        if (!alive) return;
        setLoadStatus(st);
        if (!st.running && st.report) {
          setRunningRunId(null);
          const runs = await invoke<LoadTestRun[]>("api_list_load_runs", { endpointId: selectedId });
          if (alive) setLoadRuns(runs);
        }
      } catch {
        if (alive) setRunningRunId(null);
      }
    };
    tick();
    pollRef.current = window.setInterval(tick, 1000);
    return () => {
      alive = false;
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, [runningRunId, selectedId]);

  const updateDraft = (patch: Partial<ApiEndpoint>) => {
    setDraft((d) => (d ? { ...d, ...patch } : d));
  };

  const saveDraft = async (patch?: Partial<ApiEndpoint>) => {
    if (!draft) return;
    const next = patch ? { ...draft, ...patch } : draft;
    if (!next.id) {
      const created = await invoke<ApiEndpoint>("api_create_endpoint", { ep: next });
      setDraft(created);
      setSelectedId(created.id);
      refreshEndpoints();
    } else {
      await invoke("api_update_endpoint", { ep: next });
      if (patch) setDraft(next);
      refreshEndpoints();
    }
  };

  const refreshEndpoints = async () => {
    if (!activeProjectId) return;
    const list = await invoke<ApiEndpoint[]>("api_list_endpoints", { projectId: activeProjectId, moduleId: null });
    setEndpoints(list);
  };

  // 当前请求配置 → SendRequestInput（供发送/单测/压测复用）
  const currentInput = useMemo((): SendRequestInput | null => {
    if (!draft) return null;
    return {
      method: draft.method,
      url: draft.url,
      headers: draft.headers,
      query_params: draft.query_params,
      path_params: draft.path_params,
      body: draft.body,
      body_type: draft.body_type,
      body_form: draft.body_form,
      body_urlencoded: draft.body_urlencoded,
      body_graphql_query: draft.body_graphql_query,
      body_graphql_variables: draft.body_graphql_variables,
      authorization: draft.authorization,
      cookies: draft.cookies,
      settings: draft.settings,
      timeout_ms: draft.timeout_ms,
      variables,
    };
  }, [draft, variables]);

  const sendRequest = async () => {
    if (!draft || !currentInput) return;
    setSending(true);
    try {
      const out = await invoke<SendRequestOutput>("api_send_request", { input: currentInput });
      setResponse(out);
      // 记录请求历史
      if (activeProjectId) {
        try {
          await invoke("api_add_history", {
            projectId: activeProjectId,
            endpointId: selectedId,
            name: draft.name,
            input: currentInput,
          });
          loadHistory();
        } catch { /* 历史记录失败不影响请求 */ }
      }
    } catch (e) {
      setResponse({ ok: false, status: 0, status_text: "错误", headers: [], body: String(e), body_truncated: false, time_ms: 0, size_bytes: 0 });
    } finally {
      setSending(false);
    }
  };

  const [history, setHistory] = useState<ApiHistoryEntry[]>([]);

  const loadHistory = useCallback(async () => {
    if (!activeProjectId) return;
    try {
      const list = await invoke<ApiHistoryEntry[]>("api_list_history", { projectId: activeProjectId });
      setHistory(list);
    } catch { /* 忽略 */ }
  }, [activeProjectId]);

  // 回放历史请求：填入编辑器（若关联接口仍存在则选中）
  const replayHistory = async (h: ApiHistoryEntry) => {
    if (h.endpoint_id && endpoints.some((e) => e.id === h.endpoint_id)) {
      setSelectedId(h.endpoint_id);
      return;
    }
    // 无关联接口：以临时草稿打开（不落库，避免污染接口树）
    const draftEp: ApiEndpoint = {
      id: "", project_id: activeProjectId ?? "", module_id: null,
      name: h.name, method: h.method, url: h.url,
      headers: h.input.headers ?? [], query_params: h.input.query_params ?? [], path_params: h.input.path_params ?? [],
      body: h.input.body ?? "", body_type: h.input.body_type ?? "none",
      body_form: h.input.body_form ?? [], body_urlencoded: h.input.body_urlencoded ?? [],
      body_graphql_query: h.input.body_graphql_query ?? "", body_graphql_variables: h.input.body_graphql_variables ?? "",
      authorization: h.input.authorization ?? defaultAuthorization(),
      cookies: h.input.cookies ?? [], settings: h.input.settings ?? defaultSettings(),
      response_comment: "", is_favorite: false, description: "", docs_md: "",
      timeout_ms: h.input.timeout_ms ?? 15000, created_at: "", updated_at: "",
    };
    setSelectedId(null);
    setDraft(draftEp);
    setResponse(null);
    setActiveTab("request");
  };

  // 收藏/取消收藏
  const toggleFavorite = async (ep: ApiEndpoint) => {
    const next = !ep.is_favorite;
    await invoke("api_set_favorite", { endpointId: ep.id, favorite: next });
    setEndpoints(endpoints.map((e) => (e.id === ep.id ? { ...e, is_favorite: next } : e)));
    if (selectedId === ep.id && draft) setDraft({ ...draft, is_favorite: next });
  };

  const runTests = async () => {
    if (!selectedId) return;
    setTesting(true);
    try {
      const outs = await invoke<UnitTestRunOutput[]>("api_run_unit_test", { endpointId: selectedId, variables, inputOverride: currentInput });
      const map: Record<string, UnitTestRunOutput> = {};
      tests.forEach((t, i) => {
        map[t.id] = outs[i];
      });
      setTestResults(map);
    } catch (e) {
      window.alert(String(e));
    } finally {
      setTesting(false);
    }
  };

  const startLoadTest = async () => {
    if (!selectedId || !draft) return;
    try {
      const runId = await invoke<string>("api_start_load_test", {
        endpointId: selectedId,
        name: draft.name,
        config: loadConfig,
        variables,
        inputOverride: currentInput,
      });
      setLoadStatus({ running: true, elapsed_secs: 0, total: 0, success: 0, failed: 0, qps: 0, latency_avg_ms: 0, latency_p95_ms: 0, report: null });
      setRunningRunId(runId);
    } catch (e) {
      window.alert(String(e));
    }
  };

  // 衍生：把当前请求 + 响应保存为文档
  const saveAsDoc = async () => {
    if (!draft) return;
    const ep = draft;
    let md = `# ${ep.name}\n\n> \`${ep.method} ${ep.url}\`\n`;
    if (ep.authorization.type !== "none") md += `\n**认证：** ${ep.authorization.type}\n`;
    const kvMd = (list: KeyValueItem[], title: string) => {
      const rows = list.filter((k) => k.enabled && k.key).map((k) => `| \`${k.key}\` | \`${k.value}\` | ${k.description ?? ""} |`);
      if (rows.length) md += `\n### ${title}\n\n| 参数 | 值 | 说明 |\n|------|------|------|\n${rows.join("\n")}\n`;
    };
    kvMd(ep.query_params, "查询参数");
    kvMd(ep.headers, "请求头");
    if (ep.body_type !== "none" && ep.body.trim()) {
      md += `\n### Body（${ep.body_type}）\n\n\`\`\`\n${ep.body}\n\`\`\`\n`;
    }
    if (response) {
      md += `\n## 响应示例（HTTP ${response.status} · ${fmtTime(response.time_ms)}）\n\n\`\`\`json\n${prettyJson(response.body)}\n\`\`\`\n`;
    }
    await saveDraft({ docs_md: md });
    setActiveTab("docs");
  };

  // 衍生：从响应自动生成单测断言并保存
  const saveAsTest = async () => {
    if (!draft || !selectedId) return;
    const assertions: UnitTest["assertions"] = [];
    if (response?.status) {
      assertions.push({ type: "status_eq", expected: response.status });
    }
    if (response) {
      try {
        const j = JSON.parse(response.body);
        if (j && typeof j === "object") {
          const keys = Object.keys(j).slice(0, 3);
          for (const k of keys) {
            const v = j[k];
            if (typeof v === "string") assertions.push({ type: "json_path", path: k, op: "eq", expected: v });
            else if (typeof v === "number" || typeof v === "boolean") assertions.push({ type: "json_path", path: k, op: "eq", expected: v });
          }
        }
      } catch { /* non-json body */ }
    }
    if (assertions.length === 0) {
      window.alert("请先发送请求，以便从响应生成断言");
      return;
    }
    const test: UnitTest = {
      id: "", endpoint_id: selectedId, name: `从请求衍生 ${new Date().toLocaleTimeString()}`,
      assertions, created_at: "",
    };
    await invoke("api_save_unit_test", { test });
    const t = await invoke<UnitTest[]>("api_list_unit_tests", { endpointId: selectedId });
    setTests(t);
    setActiveTab("tests");
    window.alert("已从当前请求/响应生成单测断言");
  };

  // 应用预设 Headers 到当前接口
  const applyPreset = async (setId: string) => {
    const set = presetSets.find((s) => s.id === setId);
    if (!set || !draft) return;
    const merged = [...draft.headers];
    for (const h of set.headers) {
      if (!h.key) continue;
      const idx = merged.findIndex((x) => x.key.toLowerCase() === h.key.toLowerCase());
      if (idx >= 0) merged[idx] = h;
      else merged.push(h);
    }
    updateDraft({ headers: merged });
  };

  // 保存响应注释
  const saveComment = async () => {
    if (!draft) return;
    await saveDraft({ response_comment: commentDraft });
  };

  const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const copyBody = async () => {
    if (!response) return;
    await navigator.clipboard.writeText(response.body);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

  const [projectModal, setProjectModal] = useState<{ open: boolean; project: ApiProject | null }>({ open: false, project: null });

  const openCreateProject = () => setProjectModal({ open: true, project: null });

  const openEditProject = (p: ApiProject) => setProjectModal({ open: true, project: p });

  const saveProjectModal = async (name: string, description: string, commonHeaders: KeyValueItem[], commonParams: KeyValueItem[]) => {
    if (projectModal.project) {
      const updated = { ...projectModal.project, name, description, common_headers: commonHeaders, common_params: commonParams };
      await invoke("api_update_project", { project: updated });
      setProjects(projects.map((p) => (p.id === updated.id ? updated : p)));
    } else {
      await invoke("api_create_project", { name, description });
      const list = await invoke<ApiProject[]>("api_list_projects");
      setProjects(list);
      setActiveProjectId(list[list.length - 1].id);
      // 新项目模板字段由后端默认空；若用户在创建时就填了模板，则再更新一次
      if (commonHeaders.length > 0 || commonParams.length > 0) {
        const created = list[list.length - 1];
        await invoke("api_update_project", { project: { ...created, common_headers: commonHeaders, common_params: commonParams } });
        setProjects(await invoke<ApiProject[]>("api_list_projects"));
      }
    }
    setProjectModal({ open: false, project: null });
  };

  const [moduleModal, setModuleModal] = useState<{ open: boolean; module: ApiModule | null }>({ open: false, module: null });

  const openCreateModule = () => {
    if (!activeProjectId) return;
    setModuleModal({ open: true, module: null });
  };

  const openEditModule = (m: ApiModule) => {
    setModuleModal({ open: true, module: m });
  };

  const saveModuleModal = async (name: string, description: string) => {
    if (!activeProjectId) return;
    if (moduleModal.module) {
      const updated = { ...moduleModal.module, name, description };
      await invoke("api_update_module", { module: updated });
      setModules(modules.map((m) => (m.id === updated.id ? updated : m)));
    } else {
      const created = await invoke<ApiModule>("api_create_module", { projectId: activeProjectId, name, description });
      setModules([...modules, created]);
    }
    setModuleModal({ open: false, module: null });
  };

  const createEndpoint = async (moduleId: string | null) => {
    if (!activeProjectId) return;
    const ep = emptyEndpoint(activeProjectId, moduleId);
    const created = await invoke<ApiEndpoint>("api_create_endpoint", { ep });
    setSelectedId(created.id);
    refreshEndpoints();
  };

  const deleteEndpoint = async (id: string) => {
    if (!window.confirm("确定删除该接口？")) return;
    await invoke("api_delete_endpoint", { endpointId: id });
    if (selectedId === id) {
      setSelectedId(null);
      setDraft(null);
    }
    refreshEndpoints();
  };

  const deleteModule = async (id: string) => {
    if (!window.confirm("确定删除该模块？其下的接口将变为未分组。")) return;
    await invoke("api_delete_module", { moduleId: id });
    setModules(modules.filter((m) => m.id !== id));
    refreshEndpoints();
  };

  const deleteProject = async (id: string) => {
    if (!window.confirm("确定删除该项目？项目下所有数据将被删除。")) return;
    await invoke("api_delete_project", { projectId: id });
    const list = projects.filter((p) => p.id !== id);
    setProjects(list);
    if (activeProjectId === id) setActiveProjectId(list[0]?.id ?? null);
  };

  const exportPostman = async () => {
    if (!activeProjectId) return;
    try {
      const json = await invoke<string>("api_export_postman", { projectId: activeProjectId });
      const target = await saveDialog({ defaultPath: "postman_collection.json", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (target) {
        await invoke("write_text_file", { path: target, content: json });
      }
    } catch (e) {
      window.alert(String(e));
    }
  };

  // 渲染树（项目图标 + 模块文件夹图标 + 各方法图标）
  const renderTree = () => {
    const loose = endpoints.filter((e) => !e.module_id);
    const favs = endpoints.filter((e) => e.is_favorite);
    const row = (ep: ApiEndpoint) => (
      <EndpointRow
        key={ep.id}
        ep={ep}
        selected={selectedId === ep.id}
        onSelect={() => setSelectedId(ep.id)}
        onDelete={() => deleteEndpoint(ep.id)}
        onToggleFavorite={() => toggleFavorite(ep)}
      />
    );
    return (
      <div className="space-y-0.5">
        {favs.length > 0 && (
          <div className="mb-1">
            <div className="flex items-center gap-1 px-1.5 pt-1 pb-0.5">
              <Star className="w-3 h-3 text-amber-400 fill-amber-400" />
              <span className="text-[10px] font-semibold text-slate-500">收藏</span>
            </div>
            {favs.map(row)}
          </div>
        )}
        {modules.map((m) => {
          const children = endpoints.filter((e) => e.module_id === m.id);
          const isOpen = expanded.has(m.id);
          return (
            <div key={m.id}>
              <div className="group flex items-center gap-1 rounded-md px-1.5 py-1 hover:bg-white/5 cursor-pointer" onClick={() => toggleExpand(m.id)}>
                {isOpen ? <ChevronDown className="w-3.5 h-3.5 text-slate-500" /> : <ChevronRight className="w-3.5 h-3.5 text-slate-500" />}
                <Folder className="w-3.5 h-3.5 text-amber-400/80 shrink-0" />
                <span className="flex-1 text-xs text-slate-300 truncate">{m.name}</span>
                {m.description && <span className="text-[10px] text-slate-600 truncate max-w-28 hidden group-hover:block" title={m.description}>{m.description}</span>}
                <span className="text-[10px] px-1.5 py-px rounded-full bg-white/5 border border-white/5 text-slate-500 tabular-nums">{children.length}</span>
                <button
                  onClick={(e) => { e.stopPropagation(); openEditModule(m); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title="编辑模块"
                >
                  <Pencil className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); createEndpoint(m.id); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title="新增接口"
                >
                  <Plus className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); deleteModule(m.id); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-rose-400 cursor-pointer"
                  title="删除模块"
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
              {/* 折叠动画容器（grid-rows 过渡） */}
              <div
                className="ml-4 grid transition-[grid-template-rows] duration-200 ease-out"
                style={{ gridTemplateRows: isOpen ? "1fr" : "0fr" }}
              >
                <div className="overflow-hidden min-h-0">
                  <div className="space-y-0.5 py-0.5">
                    {m.description && (
                      <div className="text-[10px] text-slate-500/90 leading-snug px-1.5 py-0.5 border-l border-white/10 ml-1">{m.description}</div>
                    )}
                    {children.map(row)}
                    {children.length === 0 && <div className="text-[10px] text-slate-600 px-2 py-0.5">（空模块）</div>}
                  </div>
                </div>
              </div>
            </div>
          );
        })}
        {loose.map(row)}
        {modules.length === 0 && endpoints.length === 0 && (
          <div className="text-[10px] text-slate-600 px-2 py-2">暂无接口，点击下方按钮新建</div>
        )}
      </div>
    );
  };

  const randomHint = (
    <div className="flex flex-wrap gap-1 px-1 pb-1">
      {RANDOM_VARIABLES.map((r) => (
        <span key={r.token} className="text-[9px] px-1.5 py-0.5 rounded bg-black/30 border border-white/10 text-slate-500" title={r.desc}>
          <code className="text-[var(--module-accent)]">{r.token}</code>
        </span>
      ))}
    </div>
  );

  const statusBadge = response ? (
    <span className={`text-xs font-bold px-2 py-0.5 rounded-md border ${response.status === 0 ? "text-rose-300 border-rose-500/40 bg-rose-500/10" : response.status < 300 ? "text-emerald-300 border-emerald-500/40 bg-emerald-500/10" : response.status < 500 ? "text-amber-300 border-amber-500/40 bg-amber-500/10" : "text-rose-300 border-rose-500/40 bg-rose-500/10"}`}>
      {response.status === 0 ? "错误" : response.status}
    </span>
  ) : null;

  return (
    <div className="h-full flex" style={{ ["--module-accent" as string]: "#06b6d4" }}>
      {/* 左侧栏 */}
      <div className="w-60 shrink-0 border-r border-white/10 flex flex-col bg-black/20">
        <div className="p-2 border-b border-white/10">
          <div className="flex items-center justify-between px-1 pb-1.5">
            <span className="text-[11px] font-semibold text-slate-400">API 项目</span>
            <button onClick={openCreateProject} className="p-1 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title="新建项目">
              <Plus className="w-3.5 h-3.5" />
            </button>
          </div>
          <div className="space-y-0.5">
            {projects.map((p) => (
              <div
                key={p.id}
                onClick={() => setActiveProjectId(p.id)}
                className={`group flex items-center gap-1.5 rounded-md px-2 py-1 cursor-pointer ${activeProjectId === p.id ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)] text-white" : "text-slate-300 hover:bg-white/5"}`}
              >
                <FlaskConical className="w-3.5 h-3.5 shrink-0" style={{ color: activeProjectId === p.id ? ACCENT : undefined }} />
                <span className="flex-1 text-xs truncate" title={p.description || undefined}>{p.name}</span>
                <button
                  onClick={(e) => { e.stopPropagation(); openEditProject(p); }}
                  className="hidden group-hover:block p-0.5 text-slate-600 hover:text-[var(--module-accent)] cursor-pointer"
                  title="编辑项目"
                >
                  <Pencil className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); deleteProject(p.id); }}
                  className="hidden group-hover:block p-0.5 text-slate-600 hover:text-rose-400 cursor-pointer"
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
            ))}
            {projects.length === 0 && <div className="text-[10px] text-slate-600 px-2 py-1">暂无项目，点击 + 新建</div>}
          </div>
        </div>
        {activeProjectId && (
          <>
            {/* 变量集合快速切换 */}
            <div className="px-2 pt-2">
              <div className="flex items-center justify-between px-1 pb-1">
                <span className="text-[11px] font-semibold text-slate-400">变量集合</span>
                <button onClick={() => setEnvModal(true)} className="p-1 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title="管理变量集合">
                  <Settings2 className="w-3.5 h-3.5" />
                </button>
              </div>
              <div className="flex flex-wrap gap-1">
                {envs.map((e) => (
                  <button
                    key={e.id}
                    onClick={() => { setActiveEnvId(e.id); invoke("api_set_active_env", { projectId: activeProjectId, envId: e.id }); }}
                    className={`text-[10px] px-2 py-0.5 rounded-full border cursor-pointer transition-colors ${e.id === activeEnvId ? "border-[var(--module-accent)]/70 text-white" : "border-white/10 text-slate-400 hover:border-white/30"}`}
                    style={e.id === activeEnvId ? { background: "color-mix(in srgb, var(--module-accent) 18%, transparent)" } : undefined}
                  >
                    {e.name}
                  </button>
                ))}
              </div>
            </div>
            <div className="flex-1 overflow-y-auto p-2 mt-1">{renderTree()}</div>
            {/* 请求历史 */}
            <div className="border-t border-white/10">
              <div className="flex items-center gap-1 px-2 pt-1.5 pb-0.5">
                <History className="w-3 h-3 text-slate-500" />
                <span className="text-[10px] font-semibold text-slate-500">请求历史</span>
                {history.length > 0 && (
                  <button
                    onClick={async () => {
                      if (!window.confirm("清空该项目全部请求历史？")) return;
                      await invoke("api_clear_history", { projectId: activeProjectId });
                      setHistory([]);
                    }}
                    className="ml-auto p-0.5 text-slate-600 hover:text-rose-400 cursor-pointer"
                    title="清空历史"
                  >
                    <Eraser className="w-3 h-3" />
                  </button>
                )}
              </div>
              <div className="max-h-44 overflow-y-auto px-1.5 pb-1.5 space-y-0.5">
                {history.slice(0, 20).map((h) => {
                  const Icon = methodIcon(h.method);
                  return (
                    <div
                      key={h.id}
                      onClick={() => replayHistory(h)}
                      className="group flex items-center gap-1.5 rounded-md px-1.5 py-1 hover:bg-white/5 cursor-pointer"
                      title={`${h.method} ${h.url}\n${h.created_at.replace("T", " ").slice(0, 19)}`}
                    >
                      <Icon className={`w-3 h-3 shrink-0 ${h.method === "GET" ? "text-emerald-400" : h.method === "POST" ? "text-amber-400" : h.method === "DELETE" ? "text-rose-400" : "text-slate-400"}`} />
                      <span className="flex-1 text-[11px] text-slate-400 truncate">{h.name || h.url}</span>
                    </div>
                  );
                })}
                {history.length === 0 && <div className="text-[10px] text-slate-600 px-1.5 py-1">发送请求后自动记录</div>}
              </div>
            </div>
            <div className="p-2 border-t border-white/10 space-y-1">
              <button onClick={openCreateModule} className="flex w-full items-center gap-1.5 px-2 py-1 text-[11px] text-slate-400 hover:text-white hover:bg-white/5 rounded-md cursor-pointer">
                <FolderPlus className="w-3.5 h-3.5" /> 新建模块
              </button>
              <button onClick={() => createEndpoint(null)} className="flex w-full items-center gap-1.5 px-2 py-1 text-[11px] text-slate-400 hover:text-white hover:bg-white/5 rounded-md cursor-pointer">
                <FilePlus2 className="w-3.5 h-3.5" /> 新建接口
              </button>
              <div className="flex gap-1 pt-1">
                <button onClick={() => setImportModal(true)} className="flex flex-1 items-center justify-center gap-1 px-2 py-1 text-[10px] rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
                  <Upload className="w-3 h-3" /> 导入
                </button>
                <button onClick={exportPostman} className="flex flex-1 items-center justify-center gap-1 px-2 py-1 text-[10px] rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
                  <Download className="w-3 h-3" /> 导出
                </button>
              </div>
            </div>
          </>
        )}
      </div>

      {/* 主区域 */}
      {draft ? (
        <div className="flex-1 min-w-0 flex flex-col">
          {/* 顶部工具条 */}
          <div className="flex items-center gap-2 px-3 py-2 border-b border-white/10">
            <select
              value={draft.method}
              onChange={(e) => updateDraft({ method: e.target.value })}
              className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1.5 text-xs font-bold cursor-pointer focus:outline-none"
            >
              {METHODS.map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
            <input
              value={draft.name}
              onChange={(e) => updateDraft({ name: e.target.value })}
              className="w-40 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 focus:outline-none"
              placeholder="接口名称"
            />
            <input
              value={draft.url}
              onChange={(e) => updateDraft({ url: e.target.value })}
              className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]/60"
              placeholder="https://api.example.com/users/{{userId}}"
              onKeyDown={(e) => e.key === "Enter" && sendRequest()}
            />
            <button
              onClick={sendRequest}
              disabled={sending}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-xs font-semibold text-white cursor-pointer disabled:opacity-50"
              style={{ background: ACCENT }}
            >
              {sending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Send className="w-3.5 h-3.5" />}
              {sending ? "发送中" : "发送"}
            </button>
            <button onClick={() => saveDraft()} className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <Save className="w-3.5 h-3.5" /> 保存
            </button>
            {selectedId && (
              <button
                onClick={() => toggleFavorite(draft)}
                title={draft.is_favorite ? "取消收藏" : "收藏此接口"}
                className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs cursor-pointer ${draft.is_favorite ? "text-amber-400 bg-amber-500/10" : "text-slate-400 bg-white/5 hover:bg-white/10"}`}
              >
                <Star className={`w-3.5 h-3.5 ${draft.is_favorite ? "fill-amber-400" : ""}`} /> {draft.is_favorite ? "已收藏" : "收藏"}
              </button>
            )}
            <button onClick={saveAsDoc} title="把当前请求参数 + 返回结果保存为 Markdown 文档" className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <BookOpen className="w-3.5 h-3.5" /> 存为文档
            </button>
            <button onClick={saveAsTest} title="根据当前请求与返回结果自动生成单测断言" className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
              <TestTube2 className="w-3.5 h-3.5" /> 存为单测
            </button>
          </div>

          {/* 层级说明：模块简介 + 接口简介 */}
          <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/5 bg-black/10">
            {(() => {
              const mod = modules.find((m) => m.id === draft.module_id);
              return (
                <>
                  {mod && (
                    <div className="flex items-center gap-1 shrink-0 text-[10px] text-amber-300/80">
                      <Folder className="w-3 h-3" />
                      <span className="font-semibold">{mod.name}</span>
                      {mod.description && <span className="text-slate-500 max-w-40 truncate" title={mod.description}>· {mod.description}</span>}
                    </div>
                  )}
                  <input
                    value={draft.description}
                    onChange={(e) => updateDraft({ description: e.target.value })}
                    placeholder="接口简介：这个接口做什么？用途、注意事项…（随保存持久化）"
                    className="flex-1 min-w-0 bg-transparent border border-transparent hover:border-white/10 focus:border-[var(--module-accent)]/40 rounded-md px-2 py-1 text-[11px] text-slate-300 placeholder:text-slate-600 focus:outline-none"
                  />
                </>
              );
            })()}
          </div>

          {/* 功能页签 */}
          <div className="flex items-center gap-1 px-3 pt-1.5">
            {([
              ["request", "请求", Send],
              ["tests", "单测", ListChecks],
              ["load", "压测", Gauge],
              ["docs", "文档", BookOpen],
            ] as const).map(([key, label, Icon]) => (
              <button
                key={key}
                onClick={() => setActiveTab(key)}
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-t-lg text-xs cursor-pointer border-b-2 ${activeTab === key ? "text-white border-[var(--module-accent)] bg-white/5" : "text-slate-500 border-transparent hover:text-slate-300"}`}
              >
                <Icon className="w-3.5 h-3.5" /> {label}
              </button>
            ))}
          </div>

          <div className="flex-1 min-h-0 overflow-y-auto px-3 py-2">
            {activeTab === "request" && (
              <div className="space-y-2">
                <div className="flex gap-1 flex-wrap">
                  {([
                    ["params", "Params", Braces],
                    ["auth", "Authorization", KeyRound],
                    ["headers", "Headers", Link2],
                    ["body", "Body", FileText],
                    ["settings", "设置", SlidersHorizontal],
                    ["cookies", "Cookies", Cookie],
                  ] as const).map(([key, label, Icon]) => (
                    <button
                      key={key}
                      onClick={() => setSubTab(key)}
                      className={`flex items-center gap-1 px-2.5 py-1 text-[11px] rounded-md cursor-pointer ${subTab === key ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
                    >
                      <Icon className="w-3 h-3" /> {label}
                    </button>
                  ))}
                </div>
                {subTab === "params" && (
                  <KvEditor
                    items={draft.query_params}
                    onChange={(v) => updateDraft({ query_params: v })}
                    placeholderValue="值（支持 {{baseUrl}} / {{$guid}}）"
                  />
                )}
                {subTab === "auth" && (
                  <AuthPanel auth={draft.authorization} onChange={(a) => updateDraft({ authorization: a })} />
                )}
                {subTab === "headers" && (
                  <div className="space-y-2">
                    <div className="flex items-center gap-2 flex-wrap">
                      <label className="flex items-center gap-1.5 text-[10px] text-slate-400 cursor-pointer">
                        <input type="checkbox" checked={hideCommonHeaders} onChange={(e) => setHideCommonHeaders(e.target.checked)} className="accent-[var(--module-accent)]" />
                        隐藏常见自动附加头（Content-Type/Length、Host、User-Agent、Accept…）
                      </label>
                      <select
                        value=""
                        onChange={(e) => { if (e.target.value) applyPreset(e.target.value); }}
                        className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1 text-[10px] text-slate-300 cursor-pointer"
                      >
                        <option value="">应用预设 Headers…</option>
                        {presetSets.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
                      </select>
                      <button onClick={() => setPresetModal(true)} className="flex items-center gap-1 text-[10px] px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
                        <Settings2 className="w-3 h-3" /> 管理预设
                      </button>
                    </div>
                    <KvEditor
                      items={hideCommonHeaders ? draft.headers.filter((h) => !COMMON_AUTO_HEADERS.includes(h.key.toLowerCase())) : draft.headers}
                      onChange={(v) => {
                        // 隐藏模式下把被隐藏的常见头合并回去
                        if (hideCommonHeaders) {
                          const hidden = draft.headers.filter((h) => COMMON_AUTO_HEADERS.includes(h.key.toLowerCase()));
                          updateDraft({ headers: [...v, ...hidden] });
                        } else {
                          updateDraft({ headers: v });
                        }
                      }}
                      placeholderKey="Header 名"
                      placeholderValue="值（如 Bearer {{token}}）"
                    />
                  </div>
                )}
                {subTab === "body" && (
                  <div className="space-y-2">
                    <div className="flex gap-1 flex-wrap">
                      {BODY_TYPES.map((b) => (
                        <button
                          key={b.value}
                          onClick={() => updateDraft({ body_type: b.value })}
                          className={`px-2.5 py-1 text-[11px] rounded-md cursor-pointer ${draft.body_type === b.value ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
                        >
                          {b.label}
                        </button>
                      ))}
                    </div>
                    {draft.body_type === "formdata" && (
                      <FormDataEditor items={draft.body_form} onChange={(v) => updateDraft({ body_form: v })} />
                    )}
                    {draft.body_type === "form" && (
                      <KvEditor items={draft.body_urlencoded} onChange={(v) => updateDraft({ body_urlencoded: v })} placeholderValue="值" />
                    )}
                    {draft.body_type === "graphql" && (
                      <div className="space-y-2">
                        <textarea
                          value={draft.body_graphql_query}
                          onChange={(e) => updateDraft({ body_graphql_query: e.target.value })}
                          rows={6}
                          placeholder={"query GetUser($id: ID!) {\n  user(id: $id) { id name }\n}"}
                          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200 focus:outline-none"
                        />
                        <textarea
                          value={draft.body_graphql_variables}
                          onChange={(e) => updateDraft({ body_graphql_variables: e.target.value })}
                          rows={3}
                          placeholder='{"id": "{{random:int:1:100}}"}'
                          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200 focus:outline-none"
                        />
                      </div>
                    )}
                    {draft.body_type === "binary" && (
                      <div className="space-y-1.5">
                        <input
                          value={draft.body}
                          onChange={(e) => updateDraft({ body: e.target.value })}
                          placeholder="本地文件路径"
                          className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200"
                        />
                        <button
                          onClick={async () => {
                            const f = await openDialog({ multiple: false });
                            if (f) updateDraft({ body: String(f) });
                          }}
                          className="text-[11px] px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer"
                        >
                          选择文件
                        </button>
                      </div>
                    )}
                    {(draft.body_type === "raw" || draft.body_type === "json") && (
                      <textarea
                        value={draft.body}
                        onChange={(e) => updateDraft({ body: e.target.value })}
                        rows={7}
                        className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
                        placeholder={draft.body_type === "json" ? '{"name": "{{random:string:6}}", "age": {{random:int:18:60}}}' : "原始内容"}
                      />
                    )}
                    {randomHint}
                  </div>
                )}
                {subTab === "settings" && (
                  <SettingsPanel
                    settings={draft.settings}
                    timeoutMs={draft.timeout_ms}
                    onChange={(s) => updateDraft({ settings: s })}
                    onTimeout={(ms) => updateDraft({ timeout_ms: ms })}
                  />
                )}
                {subTab === "cookies" && (
                  <div className="space-y-1.5">
                    <KvEditor items={draft.cookies} onChange={(v) => updateDraft({ cookies: v })} placeholderKey="Cookie 名" placeholderValue="值" />
                    <div className="text-[10px] text-slate-500">独立设置的 Cookie 会随请求发送。</div>
                  </div>
                )}
              </div>
            )}

            {activeTab === "tests" && (
              <UnitTestsPanel
                endpointId={selectedId}
                tests={tests}
                setTests={setTests}
                results={testResults}
                running={testing}
                onRun={runTests}
              />
            )}

            {activeTab === "load" && (
              <div className="space-y-3">
                <div className="grid grid-cols-4 gap-2">
                  {([
                    ["concurrency", "并发数", loadConfig.concurrency, (v: number) => setLoadConfig({ ...loadConfig, concurrency: v })],
                    ["duration_secs", "时长(秒)", loadConfig.duration_secs, (v: number) => setLoadConfig({ ...loadConfig, duration_secs: v })],
                    ["ramp_up_secs", "Ramp-up(秒)", loadConfig.ramp_up_secs, (v: number) => setLoadConfig({ ...loadConfig, ramp_up_secs: v })],
                    ["rps_limit", "RPS 上限(0=不限)", loadConfig.rps_limit, (v: number) => setLoadConfig({ ...loadConfig, rps_limit: v })],
                  ] as const).map(([key, label, value, set]) => (
                    <label key={key} className="block">
                      <span className="text-[10px] text-slate-500">{label}</span>
                      <input
                        type="number"
                        min={0}
                        value={value}
                        onChange={(e) => set(Number(e.target.value))}
                        className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 focus:outline-none"
                      />
                    </label>
                  ))}
                </div>
                <div className="text-[10px] text-slate-500">压测使用当前请求配置（含认证/Cookie/随机变量，每次请求重新生成随机值）。</div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={startLoadTest}
                    disabled={!!runningRunId || !draft.url}
                    className="flex items-center gap-1.5 px-4 py-1.5 rounded-md text-xs font-semibold text-white cursor-pointer disabled:opacity-50"
                    style={{ background: ACCENT }}
                  >
                    <Play className="w-3.5 h-3.5" />
                    {runningRunId ? "压测进行中…" : "开始压测"}
                  </button>
                  {runningRunId && loadStatus && (
                    <div className="flex-1 text-[11px] text-slate-300">
                      已运行 {loadStatus.elapsed_secs}s · 请求 {loadStatus.total} · 成功 {loadStatus.success} · 失败 {loadStatus.failed} · QPS {loadStatus.qps.toFixed(1)} · p95 {loadStatus.latency_p95_ms.toFixed(1)}ms
                    </div>
                  )}
                </div>
                {!runningRunId && loadStatus?.report && <LoadReportView report={loadStatus.report} />}
                {runningRunId && <div className="h-1.5 rounded-full bg-white/10 overflow-hidden"><div className="h-full bg-[var(--module-accent)]" style={{ width: `${Math.min(100, (loadStatus?.elapsed_secs ?? 0) / Math.max(1, loadConfig.duration_secs) * 100)}%` }} /></div>}
                {loadRuns.length > 0 && !runningRunId && (
                  <div className="space-y-1.5">
                    <div className="text-[11px] font-semibold text-slate-400">历史压测报告</div>
                    {loadRuns.map((run) => (
                      <div key={run.id} className="rounded-lg border border-white/10 bg-black/20 px-3 py-2">
                        <div className="flex items-center justify-between">
                          <div className="flex items-center gap-2 text-xs text-slate-300">
                            <span className="font-semibold">{run.name || draft.name}</span>
                            <span className="text-[10px] text-slate-500">{run.created_at.replace("T", " ").slice(0, 19)}</span>
                            <span className="text-[10px] text-slate-500">并发 {run.config.concurrency} · {run.config.duration_secs}s</span>
                          </div>
                          <button
                            onClick={async () => {
                              await invoke("api_delete_load_run", { runId: run.id });
                              setLoadRuns(loadRuns.filter((r) => r.id !== run.id));
                            }}
                            className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                        {run.report && <LoadReportView report={run.report} />}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}

            {activeTab === "docs" && (
              <DocsPanel draft={draft} onSave={(md) => saveDraft({ docs_md: md })} />
            )}
          </div>

          {/* 响应区 */}
          <div className="shrink-0 h-64 border-t border-white/10 flex flex-col">
            <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/10">
              <span className="text-[11px] font-semibold text-slate-400">响应</span>
              {statusBadge}                  {response && (
                    <>
                      <span className="text-[10px] text-slate-500">{fmtTime(response.time_ms)}</span>
                  <span className="text-[10px] text-slate-500">{response.size_bytes > 1024 * 1024 ? `${(response.size_bytes / 1024 / 1024).toFixed(1)}MB` : `${(response.size_bytes / 1024).toFixed(1)}KB`}</span>
                  {response.body_truncated && <span className="text-[10px] text-amber-400">Body 已截断（2MB 上限）</span>}
                  <div className="ml-auto flex items-center gap-2">
                    <div className="flex gap-0.5 bg-black/30 rounded p-0.5">
                      {(["pretty", "raw"] as const).map((m) => (
                        <button
                          key={m}
                          onClick={() => setBodyMode(m)}
                          className={`px-1.5 py-0.5 rounded text-[10px] cursor-pointer ${bodyMode === m ? "bg-white/10 text-cyan-300" : "text-slate-500 hover:text-slate-300"}`}
                        >
                          {m === "pretty" ? "美化" : "原始"}
                        </button>
                      ))}
                    </div>
                    <button onClick={copyBody} className="flex items-center gap-1 text-[10px] px-2 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
                      {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />} {copied ? "已复制" : "复制 Body"}
                    </button>
                  </div>
                </>
              )}
            </div>
            <div className="flex-1 min-h-0 grid grid-cols-[minmax(180px,24%)_1fr_220px]">
              <div className="border-r border-white/10 overflow-y-auto p-2">
                {response?.headers.map((h, i) => (
                  <div key={i} className="flex text-[10px] py-0.5">
                    <span className="w-1/2 text-slate-500 truncate">{h.key}</span>
                    <span className="w-1/2 text-slate-300 truncate" title={h.value}>{h.value}</span>
                  </div>
                ))}
                {response && response.headers.length === 0 && <div className="text-[10px] text-slate-600">无响应头</div>}
              </div>
              {sending ? (
                <div className="flex flex-col items-center justify-center h-full gap-2 text-slate-500 select-none">
                  <Loader2 className="w-5 h-5 animate-spin text-cyan-400" />
                  <span className="text-[11px]">请求发送中…</span>
                </div>
              ) : response ? (
                <ResponseBody body={response.body} mode={bodyMode} />
              ) : (
                <div className="h-full flex flex-col items-center justify-center text-slate-600 gap-2 select-none">
                  <Send className="w-6 h-6 opacity-40" />
                  <span className="text-[11px]">发送请求后在此查看响应</span>
                </div>
              )}
              {/* 响应注释 */}
              <div className="border-l border-white/10 flex flex-col">
                <div className="flex items-center gap-1 px-2 py-1 border-b border-white/10">
                  <StickyNote className="w-3 h-3 text-slate-500" />
                  <span className="text-[10px] text-slate-500">响应注释</span>
                  <button onClick={saveComment} className="ml-auto text-[10px] px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">保存</button>
                </div>
                <textarea
                  value={commentDraft}
                  onChange={(e) => setCommentDraft(e.target.value)}
                  placeholder="记录本次返回结果的含义、注意事项…"
                  className="flex-1 bg-transparent p-2 text-[11px] text-slate-300 resize-none focus:outline-none"
                />
              </div>
            </div>
          </div>
        </div>
      ) : (
        <div className="flex-1 flex items-center justify-center text-sm text-slate-500">
          <div className="text-center space-y-2">
            <FlaskConical className="w-10 h-10 mx-auto opacity-40" />
            <p>选择左侧接口开始调试，或新建项目/接口</p>
            <p className="text-[10px] text-slate-600">支持 {'{{\"变量名\"}}'}、{'{{$guid}}'} 等随机变量 · 单测断言 · 并发压测 · Markdown 文档 · Postman 导入导出</p>
          </div>
        </div>
      )}

      {envModal && activeProjectId && (
        <EnvModal
          projectId={activeProjectId}
          envs={envs}
          activeEnvId={activeEnvId}
          onClose={() => setEnvModal(false)}
          onChanged={async () => {
            const envList = await invoke<ApiEnvironment[]>("api_list_environments", { projectId: activeProjectId });
            setEnvs(envList);
            const project = (await invoke<ApiProject[]>("api_list_projects")).find((p) => p.id === activeProjectId);
            setActiveEnvId(project?.active_env_id ?? envList[0]?.id ?? null);
          }}
        />
      )}

      {presetModal && activeProjectId && (
        <PresetHeadersModal
          projectId={activeProjectId}
          sets={presetSets}
          onClose={() => setPresetModal(false)}
          onChanged={async () => {
            const list = await invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId: activeProjectId });
            setPresetSets(list);
          }}
        />
      )}

      {importModal && activeProjectId && (
        <ImportModal
          projectId={activeProjectId}
          modules={modules}
          onClose={() => setImportModal(false)}
          onImported={async () => {
            setImportModal(false);
            refreshEndpoints();
            const [modList, envList, presetList] = await Promise.all([
              invoke<ApiModule[]>("api_list_modules", { projectId: activeProjectId }),
              invoke<ApiEnvironment[]>("api_list_environments", { projectId: activeProjectId }),
              invoke<PresetHeaderSet[]>("api_list_preset_headers", { projectId: activeProjectId }),
            ]);
            setModules(modList);
            setEnvs(envList);
            setPresetSets(presetList);
          }}
        />
      )}

      {projectModal.open && (
        <ProjectModal
          project={projectModal.project}
          onClose={() => setProjectModal({ open: false, project: null })}
          onSave={saveProjectModal}
        />
      )}

      {moduleModal.open && (
        <ModuleModal
          module={moduleModal.module}
          onClose={() => setModuleModal({ open: false, module: null })}
          onSave={saveModuleModal}
        />
      )}
    </div>
  );
}

// ─── 接口树行（方法图标 + 名称） ───
function EndpointRow({ ep, selected, onSelect, onDelete, onToggleFavorite }: {
  ep: ApiEndpoint;
  selected: boolean;
  onSelect: () => void;
  onDelete: () => void;
  onToggleFavorite: () => void;
}) {
  const Icon = methodIcon(ep.method);
  const color = ep.method === "GET" ? "text-emerald-400" : ep.method === "POST" ? "text-amber-400" : ep.method === "PUT" ? "text-sky-400" : ep.method === "DELETE" ? "text-rose-400" : ep.method === "PATCH" ? "text-violet-400" : "text-slate-400";
  return (
    <div
      onClick={onSelect}
      title={ep.description || undefined}
      className={`group flex items-center gap-1.5 rounded-md px-2 py-1 cursor-pointer ${selected ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)]" : "hover:bg-white/5"}`}
    >
      <Icon className={`w-3 h-3 shrink-0 ${color}`} />
      <Star className={`w-3 h-3 shrink-0 cursor-pointer ${ep.is_favorite ? "text-amber-400 fill-amber-400" : "text-slate-700 opacity-0 group-hover:opacity-100 hover:text-amber-400"}`} onClick={(e) => { e.stopPropagation(); onToggleFavorite(); }} />
      <span className="flex-1 text-xs text-slate-300 truncate">{ep.name}</span>
      <button
        onClick={(e) => { e.stopPropagation(); onDelete(); }}
        className="hidden group-hover:block p-0.5 text-slate-600 hover:text-rose-400 cursor-pointer"
      >
        <Trash2 className="w-3 h-3" />
      </button>
    </div>
  );
}

// ─── 单测面板 ───
function UnitTestsPanel({ endpointId, tests, setTests, results, running, onRun }: {
  endpointId: string | null;
  tests: UnitTest[];
  setTests: (t: UnitTest[]) => void;
  results: Record<string, UnitTestRunOutput>;
  running: boolean;
  onRun: () => void;
}) {
  const addTest = () => {
    setTests([...tests, { id: "", endpoint_id: endpointId ?? "", name: `测试 ${tests.length + 1}`, assertions: [{ type: "status_eq", expected: 200 }], created_at: "" }]);
  };

  const saveAll = async () => {
    for (const t of tests) {
      await invoke("api_save_unit_test", { test: t });
    }
    window.alert("已保存");
  };

  const deleteTest = async (id: string, i: number) => {
    if (id) await invoke("api_delete_unit_test", { testId: id });
    setTests(tests.filter((_, idx) => idx !== i));
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <button onClick={addTest} className="flex items-center gap-1 text-xs px-2.5 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
          <Plus className="w-3.5 h-3.5" /> 添加断言组
        </button>
        <button onClick={saveAll} className="flex items-center gap-1 text-xs px-2.5 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
          <Save className="w-3.5 h-3.5" /> 保存
        </button>
        <button
          onClick={onRun}
          disabled={running || tests.length === 0}
          className="flex items-center gap-1 text-xs px-3 py-1.5 rounded-md font-semibold text-white cursor-pointer disabled:opacity-50"
          style={{ background: "var(--module-accent)" }}
        >
          {running ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />} 运行全部
        </button>
      </div>
      {tests.map((t, ti) => {
        const result = results[t.id];
        return (
          <div key={ti} className={`rounded-xl border p-3 ${result ? (result.pass ? "border-emerald-500/30" : "border-rose-500/30") : "border-white/10"} bg-black/20`}>
            <div className="flex items-center gap-2 mb-2">
              <input
                value={t.name}
                onChange={(e) => setTests(tests.map((x, i) => (i === ti ? { ...x, name: e.target.value } : x)))}
                className="flex-1 bg-transparent text-xs font-semibold text-slate-100 focus:outline-none border-b border-transparent focus:border-white/20"
              />
              {result && (
                <span className={`text-[10px] px-2 py-0.5 rounded-full border ${result.pass ? "text-emerald-300 border-emerald-500/40" : "text-rose-300 border-rose-500/40"}`}>
                  {result.pass ? "通过" : "失败"} · {fmtTime(result.time_ms)} · HTTP {result.status}
                </span>
              )}
              <button onClick={() => deleteTest(t.id, ti)} className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"><Trash2 className="w-3.5 h-3.5" /></button>
            </div>
            <div className="space-y-1">
              {t.assertions.map((a, ai) => (
                <div key={ai} className="flex items-center gap-1.5">
                  <select
                    value={a.type}
                    onChange={(e) => setTests(tests.map((x, i) => (i === ti ? { ...x, assertions: x.assertions.map((y, j) => (j === ai ? { ...y, type: e.target.value, op: ASSERTION_TYPES.find((s) => s.value === e.target.value)?.ops?.[0]?.value ?? y.op } : y)) } : x)))}
                    className="bg-black/30 border border-white/10 rounded-md px-1.5 py-1 text-[11px] text-slate-200 cursor-pointer"
                  >
                    {ASSERTION_TYPES.map((s) => <option key={s.value} value={s.value}>{s.label}</option>)}
                  </select>
                  {a.type === "json_path" && (
                    <input
                      value={a.path ?? ""}
                      onChange={(e) => setTests(tests.map((x, i) => (i === ti ? { ...x, assertions: x.assertions.map((y, j) => (j === ai ? { ...y, path: e.target.value } : y)) } : x)))}
                      placeholder="data.items[0].id"
                      className="w-36 bg-black/30 border border-white/10 rounded-md px-1.5 py-1 text-[11px] font-mono text-slate-200"
                    />
                  )}
                  {(a.type === "json_path" || a.type === "body_contains" || a.type === "body_not_contains") && (
                    <input
                      value={a.type === "body_contains" || a.type === "body_not_contains" ? String(a.expected ?? "") : a.type === "json_path" ? String(a.expected ?? "") : ""}
                      onChange={(e) => setTests(tests.map((x, i) => (i === ti ? { ...x, assertions: x.assertions.map((y, j) => (j === ai ? { ...y, expected: e.target.value } : y)) } : x)))}
                      placeholder={a.type === "body_contains" ? "期望包含的文本" : "期望值"}
                      className="flex-1 bg-black/30 border border-white/10 rounded-md px-1.5 py-1 text-[11px] text-slate-200"
                    />
                  )}
                  {(a.type === "status_eq" || a.type === "status_lt" || a.type === "status_gt" || a.type === "time_lt_ms") && (
                    <input
                      type="number"
                      value={a.type === "time_lt_ms" ? String(a.expected ?? 1000) : String(a.expected ?? 200)}
                      onChange={(e) => setTests(tests.map((x, i) => (i === ti ? { ...x, assertions: x.assertions.map((y, j) => (j === ai ? { ...y, expected: Number(e.target.value) } : y)) } : x)))}
                      className="w-24 bg-black/30 border border-white/10 rounded-md px-1.5 py-1 text-[11px] text-slate-200"
                    />
                  )}
                  <button
                    onClick={() => setTests(tests.map((x, i) => (i === ti ? { ...x, assertions: x.assertions.filter((_, j) => j !== ai) } : x)))}
                    className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"
                  >
                    <Trash2 className="w-3 h-3" />
                  </button>
                </div>
              ))}
              <button
                onClick={() => setTests(tests.map((x, i) => (i === ti ? { ...x, assertions: [...x.assertions, { type: "status_eq", expected: 200 }] } : x)))}
                className="text-[11px] text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
              >
                + 添加断言
              </button>
            </div>
            {result && (
              <div className="mt-2 space-y-0.5 border-t border-white/5 pt-2">
                {result.results.map((r, ri) => (
                  <div key={ri} className="flex items-center gap-1.5 text-[10px]">
                    <span className={`w-1.5 h-1.5 rounded-full ${r.pass ? "bg-emerald-400" : "bg-rose-400"}`} />
                    <span className="text-slate-400">{r.assertion}</span>
                    <span className="text-slate-500 ml-auto truncate">实际: {r.actual}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}
      {tests.length === 0 && <div className="text-[11px] text-slate-500">暂无单测，点击「添加断言组」创建（每个断言组执行一次请求，逐条校验断言）。也可在顶部「存为单测」从当前请求/响应自动生成。</div>}
    </div>
  );
}

// ─── 文档面板 ───
function DocsPanel({ draft, onSave }: { draft: ApiEndpoint; onSave: (md: string) => void }) {
  const [edit, setEdit] = useState(false);
  const [text, setText] = useState(draft.docs_md);
  useEffect(() => { setText(draft.docs_md); }, [draft.docs_md]);
  const save = async () => {
    await onSave(text);
    setEdit(false);
  };
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <button onClick={() => setEdit(!edit)} className="flex items-center gap-1 text-xs px-2.5 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
          <Pencil className="w-3.5 h-3.5" /> {edit ? "预览" : "编辑"}
        </button>
        {edit && (
          <button onClick={save} className="flex items-center gap-1 text-xs px-2.5 py-1.5 rounded-md font-semibold text-white cursor-pointer" style={{ background: "var(--module-accent)" }}>
            <Save className="w-3.5 h-3.5" /> 保存
          </button>
        )}
        <span className="text-[10px] text-slate-500">Markdown 格式，支持代码块、表格等</span>
      </div>
      {edit ? (
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={16}
          className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-xs font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
          placeholder={"# 接口说明\n\n## 请求参数\n\n| 参数 | 类型 | 必填 | 说明 |\n|------|------|------|------|\n| id | string | 是 | 用户 ID |\n"}
        />
      ) : (
        <div className="rounded-lg border border-white/10 bg-black/20 p-3 prose prose-invert prose-sm max-w-none">
          {draft.docs_md ? (
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{draft.docs_md}</ReactMarkdown>
          ) : (
            <div className="text-[11px] text-slate-500">暂无文档，点击「编辑」编写接口文档，或在顶部「存为文档」从当前请求/响应自动生成。</div>
          )}
        </div>
      )}
    </div>
  );
}

// ─── 导入弹窗 ───
function ImportModal({ projectId, modules, onClose, onImported }: {
  projectId: string;
  modules: ApiModule[];
  onClose: () => void;
  onImported: () => void;
}) {
  const [kind, setKind] = useState<"postman" | "swagger" | "framework">("postman");
  const [postmanJson, setPostmanJson] = useState("");
  const [swaggerSource, setSwaggerSource] = useState("");
  const [framework, setFramework] = useState("nest");
  const [frameworkDir, setFrameworkDir] = useState("");
  const [targetModule, setTargetModule] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const pickDir = async () => {
    const dir = await openDialog({ directory: true });
    if (dir) setFrameworkDir(String(dir));
  };

  const pickPostmanFile = async () => {
    const f = await openDialog({ multiple: false, filters: [{ name: "Postman Collection", extensions: ["json"] }] });
    if (!f) return;
    const content = await invoke<string>("read_text_file", { path: String(f) }).catch(() => "");
    if (content) setPostmanJson(content);
  };

  const doImport = async () => {
    setBusy(true);
    setMsg("");
    try {
      let count = 0;
      const moduleId = targetModule || null;
      if (kind === "postman") {
        count = await invoke<number>("api_import_postman", { json: postmanJson, projectId, moduleId });
      } else if (kind === "swagger") {
        count = await invoke<number>("api_import_swagger", { source: swaggerSource, projectId, moduleId });
      } else {
        count = await invoke<number>("api_scan_framework", { dir: frameworkDir, framework, projectId, moduleId });
      }
      setMsg(`成功导入 ${count} 个接口`);
      onImported();
    } catch (e) {
      setMsg(`导入失败: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-[560px] glass-panel rounded-2xl border border-white/10 shadow-2xl p-4 space-y-3" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Upload className="w-4 h-4" style={{ color: "var(--module-accent)" }} /> 导入接口
          </div>
          <button onClick={onClose} className="p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="flex gap-1">
          {([
            ["postman", "Postman 数据"],
            ["swagger", "Swagger"],
            ["framework", "框架扫描"],
          ] as const).map(([key, label]) => (
            <button
              key={key}
              onClick={() => setKind(key)}
              className={`px-3 py-1.5 text-xs rounded-md cursor-pointer ${kind === key ? "bg-white/10 text-white" : "text-slate-500 hover:text-slate-300"}`}
            >
              {label}
            </button>
          ))}
        </div>
        <label className="block">
          <span className="text-[10px] text-slate-500">导入到模块（留空自动创建）</span>
          <select value={targetModule} onChange={(e) => setTargetModule(e.target.value)} className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 cursor-pointer">
            <option value="">（自动按来源创建）</option>
            {modules.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
          </select>
        </label>
        {kind === "postman" && (
          <label className="block">
            <span className="text-[10px] text-slate-500">Postman Collection v2.1 JSON（集合变量将导入为变量集合）</span>
            <div className="flex gap-1.5 mb-1">
              <button onClick={pickPostmanFile} className="px-2.5 py-1 text-[11px] rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">选择文件</button>
            </div>
            <textarea
              value={postmanJson}
              onChange={(e) => setPostmanJson(e.target.value)}
              rows={9}
              placeholder='{"info":{"name":"..."},"variable":[...],"item":[...]}'
              className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs font-mono text-slate-200"
            />
          </label>
        )}
        {kind === "swagger" && (
          <label className="block">
            <span className="text-[10px] text-slate-500">Swagger/OpenAPI 地址或本地文件路径</span>
            <input
              value={swaggerSource}
              onChange={(e) => setSwaggerSource(e.target.value)}
              placeholder="https://api.example.com/swagger/v1/swagger.json 或 C:\\proj\\openapi.json"
              className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200"
            />
          </label>
        )}
        {kind === "framework" && (
          <div className="space-y-2">
            <label className="block">
              <span className="text-[10px] text-slate-500">框架类型</span>
              <select value={framework} onChange={(e) => setFramework(e.target.value)} className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 cursor-pointer">
                <option value="nest">NestJS（*.controller.ts）</option>
                <option value="nuxt">Nuxt3（server/api）</option>
                <option value="spring">Spring（*Controller.java）</option>
              </select>
            </label>
            <label className="block">
              <span className="text-[10px] text-slate-500">项目目录</span>
              <div className="flex gap-1.5">
                <input
                  value={frameworkDir}
                  onChange={(e) => setFrameworkDir(e.target.value)}
                  placeholder="选择或输入源码目录"
                  className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200"
                />
                <button onClick={pickDir} className="px-2.5 py-1.5 text-xs rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">选择</button>
              </div>
            </label>
          </div>
        )}
        {msg && <div className={`text-[11px] ${msg.startsWith("成功") ? "text-emerald-400" : "text-rose-400"}`}>{msg}</div>}
        <div className="flex justify-end gap-2 pt-1">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">关闭</button>
          <button
            onClick={doImport}
            disabled={busy || (kind === "postman" && !postmanJson.trim()) || (kind === "swagger" && !swaggerSource.trim()) || (kind === "framework" && !frameworkDir.trim())}
            className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer disabled:opacity-50"
            style={{ background: "var(--module-accent)" }}
          >
            {busy ? "导入中…" : "导入"}
          </button>
        </div>
      </div>
    </div>
  );
}
