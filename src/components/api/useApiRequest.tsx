import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Send, Loader2, StickyNote, Search } from "lucide-react";
import type { ApiEnvironment, ApiEndpoint, SendRequestInput, SendRequestOutput, ExtractResult, ExtractRule } from "./types";
import { ResponseBody, jsonGetPath, toCurl } from "./panelParts";
import { theamedAlert } from "../shared/ThemedAlert";
import type { Dispatch, SetStateAction } from "react";

/** Owns the ApiRequest feature, independently of page rendering. */
export function useApiRequest({ envs, activeEnvId, draft, setEnvs, activeProjectId, selectedId, loadHistory, updateDraft, saveDraft }: {
  envs: ApiEnvironment[];
  activeEnvId: string | null;
  draft: ApiEndpoint | null;
  setEnvs: Dispatch<SetStateAction<ApiEnvironment[]>>;
  activeProjectId: string | null;
  selectedId: string | null;
  loadHistory: () => Promise<void>;
  updateDraft: (patch: Partial<ApiEndpoint>) => void;
  saveDraft: (patch?: Partial<ApiEndpoint> | undefined) => Promise<void>;
}) {
  const { t } = useTranslation();

  const [response, setResponse] = useState<SendRequestOutput | null>(null);

  const [sending, setSending] = useState(false);

  const [copied, setCopied] = useState(false);

  const [bodyMode, setBodyMode] = useState<"pretty" | "raw">("pretty");

  const [commentDraft, setCommentDraft] = useState("");

  // 响应操作：变量提取结果 / 响应头筛选 / 复制 cURL 状态 / 导出菜单
  const [extractResults, setExtractResults] = useState<ExtractResult[]>([]);

  const [headerFilter, setHeaderFilter] = useState("");

  const [curlCopied, setCurlCopied] = useState(false);

  const variables = useMemo(() => {
    const env = envs.find((e) => e.id === activeEnvId);
    return env?.variables ?? {};
  }, [envs, activeEnvId]);

  // 响应头筛选结果（避免在 JSX 里重复过滤）
  const visibleResponseHeaders = useMemo(() => {
    const q = headerFilter.trim().toLowerCase();
    const list = response?.headers ?? [];
    return q ? list.filter((h) => h.key.toLowerCase().includes(q) || h.value.toLowerCase().includes(q)) : list;
  }, [response, headerFilter]);

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

  // 停止当前请求（后端丢弃 reqwest future，立即中断连接）
  const cancelRequest = async () => {
    try {
      await invoke("api_cancel_request");
    } catch {
      // 忽略取消失败
    }
  };

  // 响应变量提取：按规则读取 JSON 并写回当前环境变量，供后续接口引用
  const applyExtractRules = async (out: SendRequestOutput, ep: ApiEndpoint) => {
    const rules = (ep.extract_rules ?? []).filter((r) => r.enabled && r.path.trim() && r.variable.trim());
    if (rules.length === 0) {
      setExtractResults([]);
      return;
    }
    let json: unknown;
    try {
      json = JSON.parse(out.body);
    } catch {
      json = undefined;
    }
    const results: ExtractResult[] = [];
    const updates: Record<string, string | number | boolean> = {};
    for (const rule of rules) {
      const value = json === undefined ? undefined : jsonGetPath(json, rule.path);
      if (value === undefined || value === null) {
        results.push({ variable: rule.variable, path: rule.path, ok: false, value: "" });
        continue;
      }
      const text = typeof value === "object" ? JSON.stringify(value) : String(value);
      results.push({ variable: rule.variable, path: rule.path, ok: true, value: text });
      updates[rule.variable] = typeof value === "string" || typeof value === "number" || typeof value === "boolean" ? value : text;
    }
    setExtractResults(results);
    if (!activeEnvId || Object.keys(updates).length === 0) return;
    const env = envs.find((e) => e.id === activeEnvId);
    if (!env) return;
    const next: ApiEnvironment = { ...env, variables: { ...env.variables, ...updates } };
    try {
      await invoke("api_update_environment", { env: next });
      setEnvs((prev) => prev.map((e) => (e.id === env.id ? next : e)));
    } catch {
      // 写回环境失败不影响响应展示
    }
  };

  const sendRequest = async () => {
    if (!draft || !currentInput) return;
    setSending(true);
    try {
      const out = await invoke<SendRequestOutput>("api_send_request", { input: currentInput });
      setResponse(out);
      await applyExtractRules(out, draft);
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
      const msg = String(e);
      const cancelled = msg.includes("请求已取消");
      setResponse({ ok: false, status: 0, status_text: cancelled ? t("api.cancelled") : t("api.statusError"), headers: [], body: msg, body_truncated: false, time_ms: 0, size_bytes: 0 });
    } finally {
      setSending(false);
    }
  };

  // 响应另存为文件（按 Content-Type 推断扩展名）
  const saveResponseBody = async () => {
    if (!response) return;
    const ct = response.headers.find((h) => h.key.toLowerCase() === "content-type")?.value.toLowerCase() ?? "";
    const ext = ct.includes("json") ? "json" : ct.includes("html") ? "html" : ct.includes("xml") ? "xml" : "txt";
    try {
      const target = await saveDialog({ defaultPath: `response.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
      if (target) await invoke("write_text_file", { path: target, content: response.body });
    } catch (e) {
      theamedAlert(String(e));
    }
  };

  // 当前请求复制为 cURL 命令
  const copyAsCurl = async () => {
    if (!draft) return;
    await navigator.clipboard.writeText(toCurl(draft));
    setCurlCopied(true);
    setTimeout(() => setCurlCopied(false), 1200);
  };

  // ─── 响应变量提取规则编辑 ───
  const addExtractRule = () => {
    if (!draft) return;
    updateDraft({ extract_rules: [...(draft.extract_rules ?? []), { path: "", variable: "", enabled: true }] });
  };

  const updateExtractRule = (index: number, patch: Partial<ExtractRule>) => {
    if (!draft) return;
    updateDraft({ extract_rules: (draft.extract_rules ?? []).map((r, i) => (i === index ? { ...r, ...patch } : r)) });
  };

  const removeExtractRule = (index: number) => {
    if (!draft) return;
    updateDraft({ extract_rules: (draft.extract_rules ?? []).filter((_, i) => i !== index) });
  };

  // 保存响应注释
  const saveComment = async () => {
    if (!draft) return;
    await saveDraft({ response_comment: commentDraft });
  };

  const copyBody = async () => {
    if (!response) return;
    await navigator.clipboard.writeText(response.body);
    setCopied(true);
    setTimeout(() => setCopied(false), 1200);
  };

const renderResponseContent = () => response && (<div className="flex-1 min-h-0 grid grid-cols-[minmax(180px,24%)_1fr_220px]">
              <div className="border-r border-white/10 overflow-y-auto p-2">
                {response && response.headers.length > 0 && (
                  <div className="relative mb-1.5">
                    <Search className="w-3 h-3 absolute left-1.5 top-1/2 -translate-y-1/2 text-slate-600" />
                    <input
                      value={headerFilter}
                      onChange={(e) => setHeaderFilter(e.target.value)}
                      placeholder={t("api.headerFilterPh")}
                      className="w-full bg-black/30 border border-white/10 rounded-md pl-6 pr-1.5 py-0.5 text-tiny text-slate-300 placeholder:text-slate-600 focus:outline-none focus:border-[var(--module-accent)]/60"
                    />
                  </div>
                )}
                {visibleResponseHeaders.map((h, i) => (
                  <div key={i} className="flex text-tiny py-0.5">
                    <span className="w-1/2 text-slate-500 truncate" title={h.key}>{h.key}</span>
                    <span className="w-1/2 text-slate-300 truncate" title={h.value}>{h.value}</span>
                  </div>
                ))}
                {response && response.headers.length === 0 && <div className="text-tiny text-slate-600">{t("api.noHeaders")}</div>}
                {response && response.headers.length > 0 && visibleResponseHeaders.length === 0 && (
                  <div className="text-tiny text-slate-600 pt-1">{t("api.headerNoMatch")}</div>
                )}
              </div>
              {sending ? (
                <div className="flex flex-col items-center justify-center h-full gap-2 text-slate-500 select-none">
                  <Loader2 className="w-5 h-5 animate-spin text-cyan-400" />
                  <span className="text-caption">{t("api.sendingWait")}</span>
                </div>
              ) : response ? (
                <ResponseBody body={response.body} mode={bodyMode} />
              ) : (
                <div className="h-full flex flex-col items-center justify-center text-slate-600 gap-2 select-none">
                  <Send className="w-6 h-6 opacity-40" />
                  <span className="text-caption">{t("api.noResponseYet")}</span>
                </div>
              )}
              {/* 响应注释 */}
              <div className="border-l border-white/10 flex flex-col">
                <div className="flex items-center gap-1 px-2 py-1 border-b border-white/10">
                  <StickyNote className="w-3 h-3 text-slate-500" />
                  <span className="text-tiny text-slate-500">{t("api.comment")}</span>
                  <button onClick={saveComment} className="ml-auto text-tiny px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">{t("common.save")}</button>
                </div>
                <textarea
                  value={commentDraft}
                  onChange={(e) => setCommentDraft(e.target.value)}
                  placeholder={t("api.commentPh")}
                  className="flex-1 bg-transparent p-2 text-caption text-slate-300 resize-none focus:outline-none"
                />
              </div>
            </div>);
  return {
    response,
    setResponse,
    sending,
    copied,
    bodyMode,
    setBodyMode,
    setCommentDraft,
    extractResults,
    setExtractResults,
    setHeaderFilter,
    curlCopied,
    variables,
    currentInput,
    cancelRequest,
    sendRequest,
    saveResponseBody,
    copyAsCurl,
    addExtractRule,
    updateExtractRule,
    removeExtractRule,
    copyBody,
    renderResponseContent,
  };
}
