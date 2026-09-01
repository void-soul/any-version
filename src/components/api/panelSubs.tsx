import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Plus, Save, Play, Loader2, Trash2, Pencil, Upload, X, Star, Link2,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { ApiEndpoint, ApiModule, UnitTest, UnitTestRunOutput } from "./types";
import { ASSERTION_TYPES } from "./types";
import { methodIcon, fmtTime } from "./panelParts";
import type { AiProvider } from "../ai/types";

// ─── 接口树行（方法图标 + 名称） ───
export function EndpointRow({ ep, selected, onSelect, onDelete, onToggleFavorite }: {
  ep: ApiEndpoint;
  selected: boolean;
  onSelect: () => void;
  onDelete: () => void;
  onToggleFavorite: () => void;
}) {
  const { t } = useTranslation();
  const Icon = methodIcon(ep.method);
  const color = ep.method === "GET" ? "text-emerald-400" : ep.method === "POST" ? "text-amber-400" : ep.method === "PUT" ? "text-sky-400" : ep.method === "DELETE" ? "text-rose-400" : ep.method === "PATCH" ? "text-violet-400" : "text-slate-400";
  // 继承自项目模板的参数总数（含 form-data）
  const tplCount =
    [ep.headers, ep.query_params, ep.path_params, ep.body_urlencoded, ep.cookies].reduce((n, a) => n + a.filter((x) => x.from_template).length, 0) +
    ep.body_form.filter((f) => f.from_template).length;
  return (
    <div
      onClick={onSelect}
      title={ep.description || undefined}
      className={`group flex items-center gap-1.5 rounded-md px-2 py-1 cursor-pointer ${selected ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)]" : "hover:bg-white/5"}`}
    >
      <Icon className={`w-3 h-3 shrink-0 ${color}`} />
      <Star className={`w-3 h-3 shrink-0 cursor-pointer ${ep.is_favorite ? "text-amber-400 fill-amber-400" : "text-slate-700 opacity-0 group-hover:opacity-100 hover:text-amber-400"}`} onClick={(e) => { e.stopPropagation(); onToggleFavorite(); }} />
      {tplCount > 0 && (
        <span
          title={t("apisubs.tplCountTip", { n: tplCount })}
          className={`flex items-center gap-0.5 shrink-0 ${selected ? "text-[var(--module-accent)]" : "text-slate-500 group-hover:text-slate-400"}`}
        >
          <Link2 className="w-2.5 h-2.5" />
          {tplCount > 1 && <span className="text-[9px] tabular-nums leading-none">{tplCount}</span>}
        </span>
      )}
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
export function UnitTestsPanel({ endpointId, tests, setTests, results, running, onRun }: {
  endpointId: string | null;
  tests: UnitTest[];
  setTests: (t: UnitTest[]) => void;
  results: Record<string, UnitTestRunOutput>;
  running: boolean;
  onRun: () => void;
}) {
  const { t } = useTranslation();
  const addTest = () => {
    setTests([...tests, { id: "", endpoint_id: endpointId ?? "", name: t("apisubs.newTestDefault", { n: tests.length + 1 }), assertions: [{ type: "status_eq", expected: 200 }], created_at: "" }]);
  };

  const saveAll = async () => {
    for (const t of tests) {
      await invoke("api_save_unit_test", { test: t });
    }
    window.alert(t("apisubs.saved"));
  };

  const deleteTest = async (id: string, i: number) => {
    if (id) await invoke("api_delete_unit_test", { testId: id });
    setTests(tests.filter((_, idx) => idx !== i));
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <button onClick={addTest} className="flex items-center gap-1 text-xs px-2.5 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
          <Plus className="w-3.5 h-3.5" /> {t("apisubs.addAssertGroup")}
        </button>
        <button onClick={saveAll} className="flex items-center gap-1 text-xs px-2.5 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
          <Save className="w-3.5 h-3.5" /> {t("common.save")}
        </button>
        <button
          onClick={onRun}
          disabled={running || tests.length === 0}
          className="flex items-center gap-1 text-xs px-3 py-1.5 rounded-md font-semibold text-white cursor-pointer disabled:opacity-50"
          style={{ background: "var(--module-accent)" }}
        >
          {running ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Play className="w-3.5 h-3.5" />} {t("apisubs.runAll")}
        </button>
      </div>
      {tests.map((ut, ti) => {
        const result = results[ut.id];
        return (
          <div key={ti} className={`rounded-xl border p-3 ${result ? (result.pass ? "border-emerald-500/30" : "border-rose-500/30") : "border-white/10"} bg-black/20`}>
            <div className="flex items-center gap-2 mb-2">
              <input
                value={ut.name}
                onChange={(e) => setTests(tests.map((x, i) => (i === ti ? { ...x, name: e.target.value } : x)))}
                className="flex-1 bg-transparent text-xs font-semibold text-slate-100 focus:outline-none border-b border-transparent focus:border-white/20"
              />
              {result && (
                <span className={`text-[10px] px-2 py-0.5 rounded-full border ${result.pass ? "text-emerald-300 border-emerald-500/40" : "text-rose-300 border-rose-500/40"}`}>
                  {result.pass ? t("apisubs.pass") : t("apisubs.fail")} · {fmtTime(result.time_ms)} · HTTP {result.status}
                </span>
              )}
              <button onClick={() => deleteTest(ut.id, ti)} className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"><Trash2 className="w-3.5 h-3.5" /></button>
            </div>
            <div className="space-y-1">
              {ut.assertions.map((a, ai) => (
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
                      placeholder={a.type === "body_contains" ? t("apisubs.expectedContains") : t("apisubs.expectedValue")}
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
                + {t("apisubs.addAssertion")}
              </button>
            </div>
            {result && (
              <div className="mt-2 space-y-0.5 border-t border-white/5 pt-2">
                {result.results.map((r, ri) => (
                  <div key={ri} className="flex items-center gap-1.5 text-[10px]">
                    <span className={`w-1.5 h-1.5 rounded-full ${r.pass ? "bg-emerald-400" : "bg-rose-400"}`} />
                    <span className="text-slate-400">{r.assertion}</span>
                    <span className="text-slate-500 ml-auto truncate">{t("apisubs.actual", { v: r.actual })}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}
      {tests.length === 0 && <div className="text-[11px] text-slate-500">{t("apisubs.noTests")}</div>}
    </div>
  );
}

// ─── 文档面板 ───
export function DocsPanel({ draft, onSave }: { draft: ApiEndpoint; onSave: (md: string) => void }) {
  const { t } = useTranslation();
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
          <Pencil className="w-3.5 h-3.5" /> {edit ? t("apisubs.preview") : t("apisubs.edit")}
        </button>
        {edit && (
          <button onClick={save} className="flex items-center gap-1 text-xs px-2.5 py-1.5 rounded-md font-semibold text-white cursor-pointer" style={{ background: "var(--module-accent)" }}>
            <Save className="w-3.5 h-3.5" /> {t("common.save")}
          </button>
        )}
        <span className="text-[10px] text-slate-500">{t("apisubs.mdHint")}</span>
      </div>
      {edit ? (
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={16}
          className="w-full bg-black/30 border border-white/10 rounded-lg px-3 py-2 text-xs font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
          placeholder={t("apisubs.docPh")}
        />
      ) : (
        <div className="rounded-lg border border-white/10 bg-black/20 p-3 prose prose-invert prose-sm max-w-none">
          {draft.docs_md ? (
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{draft.docs_md}</ReactMarkdown>
          ) : (
            <div className="text-[11px] text-slate-500">{t("apisubs.noDocs")}</div>
          )}
        </div>
      )}
    </div>
  );
}

// ─── 导入弹窗 ───
export function ImportModal({ projectId, modules, onClose, onImported }: {
  projectId: string;
  modules: ApiModule[];
  onClose: () => void;
  onImported: () => void;
}) {
  const [kind, setKind] = useState<"postman" | "swagger" | "framework" | "ai">("postman");
  const [postmanJson, setPostmanJson] = useState("");
  const [swaggerSource, setSwaggerSource] = useState("");
  const [framework, setFramework] = useState("nest");
  const [frameworkDir, setFrameworkDir] = useState("");
  // AI 分析
  const [aiDir, setAiDir] = useState("");
  const [aiProviders, setAiProviders] = useState<AiProvider[]>([]);
  const [aiProviderId, setAiProviderId] = useState("");
  const [aiModelId, setAiModelId] = useState("");
  const [targetModule, setTargetModule] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [msgOk, setMsgOk] = useState(true);
  const { t } = useTranslation();

  // 加载 AI 模块的供应商/模型配置（用于 AI 分析导入）
  useEffect(() => {
    invoke<any>("get_ai_config")
      .then((cfg) => {
        const list: AiProvider[] = (cfg?.providers ?? []).filter((p: AiProvider) => p.api_key && p.openai_url);
        setAiProviders(list);
        if (list.length > 0) {
          setAiProviderId(list[0].id);
          setAiModelId(list[0].models[0]?.id ?? list[0].active_model_id ?? "");
        }
      })
      .catch(() => {});
  }, []);

  const pickDir = async () => {
    const dir = await openDialog({ directory: true });
    if (dir) setFrameworkDir(String(dir));
  };

  const pickAiDir = async () => {
    const dir = await openDialog({ directory: true });
    if (dir) setAiDir(String(dir));
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
    setMsgOk(true);
    try {
      let count = 0;
      const moduleId = targetModule || null;
      if (kind === "postman") {
        count = await invoke<number>("api_import_postman", { json: postmanJson, projectId, moduleId });
      } else if (kind === "swagger") {
        count = await invoke<number>("api_import_swagger", { source: swaggerSource, projectId, moduleId });
      } else if (kind === "framework") {
        count = await invoke<number>("api_scan_framework", { dir: frameworkDir, framework, projectId, moduleId });
      } else {
        const [n, via] = await invoke<[number, string]>("api_import_with_ai", {
          dir: aiDir, projectId, moduleId, providerId: aiProviderId || null, modelId: aiModelId || null,
        });
        count = n;
        setMsg(t("apisubs.aiDone", { via, count }));
        onImported();
        return;
      }
      setMsg(t("apisubs.importedOk", { count }));
      onImported();
    } catch (e) {
      setMsgOk(false);
      setMsg(t("apisubs.importFail", { err: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 modal-mask flex items-center justify-center bg-black/60">
      <div className="w-[560px] glass-panel rounded-2xl border border-white/10 shadow-2xl p-4 space-y-3" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2 text-sm font-semibold text-white">
            <Upload className="w-4 h-4" style={{ color: "var(--module-accent)" }} /> {t("apisubs.importTitle")}
          </div>
          <button onClick={onClose} className="p-1 text-slate-500 hover:text-white cursor-pointer"><X className="w-4 h-4" /></button>
        </div>
        <div className="flex gap-1">
          {([
            ["postman", t("apisubs.tabPostman")],
            ["swagger", "Swagger"],
            ["framework", t("apisubs.tabFramework")],
            ["ai", t("apisubs.tabAi")],
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
          <span className="text-[10px] text-slate-500">{t("apisubs.importToModule")}</span>
          <select value={targetModule} onChange={(e) => setTargetModule(e.target.value)} className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 cursor-pointer">
            <option value="">{t("apisubs.autoCreate")}</option>
            {modules.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
          </select>
        </label>
        {kind === "postman" && (
          <label className="block">
            <span className="text-[10px] text-slate-500">{t("apisubs.postmanDesc")}</span>
            <div className="flex gap-1.5 mb-1">
              <button onClick={pickPostmanFile} className="px-2.5 py-1 text-[11px] rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">{t("apisubs.chooseFile")}</button>
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
            <span className="text-[10px] text-slate-500">{t("apisubs.swaggerDesc")}</span>
            <input
              value={swaggerSource}
              onChange={(e) => setSwaggerSource(e.target.value)}
              placeholder={t("apisubs.importPh")}
              className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200"
            />
          </label>
        )}
        {kind === "framework" && (
          <div className="space-y-2">
            <label className="block">
              <span className="text-[10px] text-slate-500">{t("apisubs.frameworkType")}</span>
              <select value={framework} onChange={(e) => setFramework(e.target.value)} className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 cursor-pointer">
                <option value="nest">{t("apisubs.nestOpt")}</option>
                <option value="nuxt">{t("apisubs.nuxtOpt")}</option>
                <option value="spring">{t("apisubs.springOpt")}</option>
              </select>
            </label>
            <label className="block">
              <span className="text-[10px] text-slate-500">{t("apisubs.projectDir")}</span>
              <div className="flex gap-1.5">
                <input
                  value={frameworkDir}
                  onChange={(e) => setFrameworkDir(e.target.value)}
                  placeholder={t("apisubs.chooseOrInputDir")}
                  className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200"
                />
                <button onClick={pickDir} className="px-2.5 py-1.5 text-xs rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">{t("apisubs.choose")}</button>
              </div>
            </label>
          </div>
        )}
        {kind === "ai" && (
          <div className="space-y-2">
            <label className="block">
              <span className="text-[10px] text-slate-500">{t("apisubs.aiDirDesc")}</span>
              <div className="flex gap-1.5">
                <input
                  value={aiDir}
                  onChange={(e) => setAiDir(e.target.value)}
                  placeholder={t("apisubs.chooseOrInputDir")}
                  className="flex-1 bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200"
                />
                <button onClick={pickAiDir} className="px-2.5 py-1.5 text-xs rounded-md bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">{t("apisubs.choose")}</button>
              </div>
            </label>
            <div className="grid grid-cols-2 gap-2">
              <label className="block">
                <span className="text-[10px] text-slate-500">{t("apisubs.aiProvider")}</span>
                <select
                  value={aiProviderId}
                  onChange={(e) => {
                    const p = aiProviders.find((x) => x.id === e.target.value);
                    setAiProviderId(e.target.value);
                    setAiModelId(p?.models[0]?.id ?? p?.active_model_id ?? "");
                  }}
                  className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 cursor-pointer"
                >
                  {aiProviders.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
                </select>
              </label>
              <label className="block">
                <span className="text-[10px] text-slate-500">{t("apisubs.model")}</span>
                <select
                  value={aiModelId}
                  onChange={(e) => setAiModelId(e.target.value)}
                  className="w-full bg-black/30 border border-white/10 rounded-md px-2 py-1.5 text-xs text-slate-200 cursor-pointer"
                >
                  {(aiProviders.find((p) => p.id === aiProviderId)?.models ?? []).map((m) => (
                    <option key={m.id} value={m.id}>{m.name}</option>
                  ))}
                </select>
              </label>
            </div>
            <div className="text-[10px] text-slate-500 space-y-0.5">
              <p>{t("apisubs.importAiHint")}</p>
              <p>{t("apisubs.providerHint")}</p>
            </div>
          </div>
        )}
        {msg && <div className={`text-[11px] ${msgOk ? "text-emerald-400" : "text-rose-400"}`}>{msg}</div>}
        <div className="flex justify-end gap-2 pt-1">
          <button onClick={onClose} className="px-3 py-1.5 text-xs rounded-lg bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">{t("common.close")}</button>
          <button
            onClick={doImport}
            disabled={busy || (kind === "postman" && !postmanJson.trim()) || (kind === "swagger" && !swaggerSource.trim()) || (kind === "framework" && !frameworkDir.trim()) || (kind === "ai" && (!aiDir.trim() || !aiProviderId))}
            className="px-4 py-1.5 text-xs rounded-lg font-semibold text-white cursor-pointer disabled:opacity-50"
            style={{ background: "var(--module-accent)" }}
          >
            {busy ? (kind === "ai" ? t("apisubs.aiAnalyzing") : t("apisubs.importing")) : (kind === "ai" ? t("apisubs.aiAnalyzeImport") : t("apisubs.import"))}
          </button>
        </div>
      </div>
    </div>
  );
}