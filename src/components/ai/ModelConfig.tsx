import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Plus,
  Trash2,
  Zap,
  CheckCircle,
  AlertTriangle,
  RefreshCw,
  Globe,
  Key,
  Server,
  ChevronDown,
  ChevronRight,
  X,
  Settings2,
  ExternalLink,
} from "lucide-react";
import type { ModelEntry, AiProvider, AiConfig } from "./types";

type Preset = {
  id: string; name: string; category: string;
  website: string; openai_url: string; anthropic_url: string;
  google_url: string;
};

const EMPTY_PROVIDER: AiProvider = {
  id: "", name: "", category: "provider", api_key: "", website: "",
  openai_enabled: false, openai_url: "", openai_use_proxy: false,
  anthropic_enabled: false, anthropic_url: "", anthropic_use_proxy: false,
  google_enabled: false, google_url: "",
  anthropic_model_aliases: {}, anthropic_default_model: null,
  openai_model_aliases: {}, openai_default_model: null,
  google_model_aliases: {}, google_default_model: null,
  models: [], active_model_id: null,
};

export default function ModelConfig() {
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [showAddMenu, setShowAddMenu] = useState(false);
  const [presets, setPresets] = useState<Preset[]>([]);

  // 弹框状态
  const [showModal, setShowModal] = useState(false);
  const [modalMode, setModalMode] = useState<"add" | "edit">("add");
  const [form, setForm] = useState<AiProvider>({ ...EMPTY_PROVIDER });
  const [formError, setFormError] = useState<string | null>(null);
  // 模型批量录入文本（一行一个 model_id 或 "model_id | 显示名"）
  const [modelsText, setModelsText] = useState("");
  const [fetchingModels, setFetchingModels] = useState(false);

  // 删除确认
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  // 测速
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{ id: string; ok: boolean; msg: string } | null>(null);

  const loadConfig = useCallback(async () => {
    try {
      const [data, presetData] = await Promise.all([
        invoke<AiConfig>("get_ai_config"),
        invoke<Preset[]>("get_provider_presets"),
      ]);
      setConfig(data);
      setPresets(presetData);
    } catch {
      setConfig({ providers: [], active_provider: null, proxy_port: 15721, default_project_path: "", rectifier: { enabled: false, thinking_signature: false, thinking_budget: false, media_fallback: false }, optimizer: { enabled: false, cache_injection: false, thinking_optimizer: false, deepseek_normalize: false }, skills_dir: "" });
    } finally { setLoading(false); }
  }, []);

  useEffect(() => { loadConfig(); }, [loadConfig]);

  const saveConfig = async (next: AiConfig) => {
    setConfig(next);
    try { await invoke("save_ai_config", { config: next }); } catch (e) { console.error(e); }
  };

  // ─── 弹框操作 ───

  const openAddModal = (preset?: Preset) => {
    setModalMode("add");
    setForm({
      ...EMPTY_PROVIDER,
      id: preset?.id || `custom_${Date.now()}`,
      name: preset?.name || "",
      category: preset?.category || "provider",
      website: preset?.website || "",
      openai_enabled: !!preset?.openai_url,
      openai_url: preset?.openai_url || "",
      anthropic_enabled: !!preset?.anthropic_url,
      anthropic_url: preset?.anthropic_url || "",
      google_enabled: !!preset?.google_url,
      google_url: preset?.google_url || "",
    });
    setModelsText("");
    setFormError(null);
    setShowModal(true);
    setShowAddMenu(false);
  };

  const openEditModal = (provider: AiProvider) => {
    setModalMode("edit");
    setForm({ ...provider });
    // 模型列表转为文本：每行一个 id
    setModelsText(provider.models.map(m => m.id).join("\n"));
    setFormError(null);
    setShowModal(true);
  };

  const validateForm = (): string | null => {
    if (!form.name.trim()) return "名称不能为空";
    if (!form.openai_enabled && !form.anthropic_enabled && !form.google_enabled) return "至少启用一种协议（OpenAI / Anthropic / Google）";
    if (form.openai_enabled && !form.openai_url.trim() && !form.openai_use_proxy) return "OpenAI 已启用但未配置 URL，请填写 URL 或启用转换代理";
    if (form.anthropic_enabled && !form.anthropic_url.trim() && !form.anthropic_use_proxy) return "Anthropic 已启用但未配置 URL，请填写 URL 或启用转换代理";
    if (form.openai_use_proxy && !form.anthropic_url.trim()) return "OpenAI 转换代理需要 Anthropic URL 作为上游（OpenAI→Anthropic）";
    if (form.anthropic_use_proxy && !form.openai_url.trim()) return "Anthropic 转换代理需要 OpenAI URL 作为上游（Anthropic→OpenAI）";
    if (!form.api_key.trim()) return "API Key 不能为空";
    return null;
  };

  const handleModalConfirm = async () => {
    const err = validateForm();
    if (err) { setFormError(err); return; }

    if (!config) return;

    // 解析模型文本：每行一个 model id
    const manualModels: ModelEntry[] = modelsText
      .split("\n")
      .map(line => line.trim())
      .filter(line => line.length > 0)
      .map(line => ({ id: line, name: line }));

    // 新建供应商时，如果用户未手动录入模型，自动从 API 获取模型列表
    let autoModels: ModelEntry[] = [];
    if (modalMode === "add" && manualModels.length === 0) {
      const url = form.openai_url || form.anthropic_url || form.google_url || "";
      if (url && form.api_key) {
        try {
          const fetched: string[] = await invoke("fetch_provider_models", { baseUrl: url, apiKey: form.api_key });
          autoModels = fetched.map(id => ({ id, name: id }));
        } catch {
          // 自动获取失败不阻塞保存，用户后续可手动点"自动获取"
        }
      }
    }

    const models = autoModels.length > 0 ? autoModels : manualModels;
    const saved = { ...form, models };

    let next: AiConfig;
    if (modalMode === "add") {
      next = { ...config, providers: [...config.providers, saved] };
    } else {
      next = { ...config, providers: config.providers.map(p => p.id === saved.id ? saved : p) };
    }
    saveConfig(next);
    setShowModal(false);
    setExpandedId(saved.id);
  };

  // ─── 删除 ───

  const handleDelete = (id: string) => {
    if (!config) return;
    const next: AiConfig = {
      ...config,
      providers: config.providers.filter(p => p.id !== id),
      active_provider: config.active_provider === id ? null : config.active_provider,
    };
    saveConfig(next);
    setDeleteTarget(null);
    if (expandedId === id) setExpandedId(null);
  };

  // ─── 设为当前供应商（仅标记，不设置默认模型）───

  const handleSetActiveProvider = (providerId: string) => {
    if (!config) return;
    saveConfig({ ...config, active_provider: providerId });
  };

  // ─── 自动获取模型列表 ───
  const handleFetchModels = async () => {
    if (!form.openai_url && !form.anthropic_url && !form.google_url) {
      setFormError("请先填写至少一个 URL");
      return;
    }
    if (!form.api_key) {
      setFormError("请先填写 API Key");
      return;
    }
    setFetchingModels(true);
    setFormError(null);
    try {
      // 优先用 OpenAI URL，没有则用 Anthropic URL，最后用 Google URL
      const url = form.openai_url || form.anthropic_url || form.google_url;
      const models = await invoke<string[]>("fetch_provider_models", {
        baseUrl: url,
        apiKey: form.api_key,
      });
      if (models.length === 0) {
        setFormError("未获取到任何模型");
      } else {
        setModelsText(models.join("\n"));
      }
    } catch (e: any) {
      setFormError(`获取模型失败: ${e}`);
    } finally {
      setFetchingModels(false);
    }
  };

  // ─── 测速 ───

  const handleTest = async (provider: AiProvider) => {
    setTesting(provider.id);
    setTestResult(null);
    try {
      const result = await invoke<{ success: boolean; message: string; latency_ms: number }>("test_model_connection", {
        openaiUrl: provider.openai_url || null,
        anthropicUrl: provider.anthropic_url || null,
        apiKey: provider.api_key,
      });
      setTestResult({ id: provider.id, ok: result.success, msg: result.message });
    } catch (e: any) {
      setTestResult({ id: provider.id, ok: false, msg: String(e) });
    } finally { setTesting(null); }
  };

  if (loading) {
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-xs">加载中...</span></div>;
  }

  return (
    <div className="h-full overflow-y-auto p-6 space-y-4">
      {/* Add Button */}
      <div className="relative">
        <button onClick={() => setShowAddMenu(!showAddMenu)} className="px-3.5 py-2 rounded-xl bg-violet-600 hover:bg-violet-500 text-white text-[11px] font-semibold flex items-center gap-1.5 cursor-pointer shadow-lg shadow-violet-500/10">
          <Plus className="w-3.5 h-3.5" /> 添加 Provider
        </button>
        {showAddMenu && (
          <div className="absolute top-full left-0 mt-1 w-72 bg-slate-900 border border-white/10 rounded-xl shadow-2xl z-50 overflow-hidden max-h-[70vh] overflow-y-auto">
            <div className="px-3 pt-2.5 pb-1 text-[9px] font-bold text-slate-500 uppercase tracking-wider">供应商</div>
            {presets.filter(p => p.category === "provider").map((p) => (
              <button key={p.id} onClick={() => openAddModal(p)} disabled={config?.providers.some(x => x.id === p.id)}
                className="w-full px-3.5 py-2 text-left text-[11px] text-slate-300 hover:bg-white/5 hover:text-white flex items-center gap-2 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer transition-all">
                <Globe className="w-3.5 h-3.5 text-slate-500" />{p.name}
                {config?.providers.some(x => x.id === p.id) && <span className="ml-auto text-[9px] text-slate-600">已添加</span>}
              </button>
            ))}
            <button onClick={() => openAddModal()} className="w-full px-3.5 py-2 text-left text-[11px] text-slate-500 hover:bg-white/5 hover:text-slate-300 flex items-center gap-2 cursor-pointer transition-all">
              <Plus className="w-3.5 h-3.5" />自定义供应商
            </button>
            <div className="border-t border-white/5 mx-3 my-1" />
            <div className="px-3 pt-1 pb-1 text-[9px] font-bold text-slate-500 uppercase tracking-wider">中转站</div>
            {presets.filter(p => p.category === "relay").map((p) => (
              <button key={p.id} onClick={() => openAddModal(p)} disabled={config?.providers.some(x => x.id === p.id)}
                className="w-full px-3.5 py-2 text-left text-[11px] text-slate-300 hover:bg-white/5 hover:text-white flex items-center gap-2 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer transition-all">
                <Server className="w-3.5 h-3.5 text-slate-500" />{p.name}
                {config?.providers.some(x => x.id === p.id) && <span className="ml-auto text-[9px] text-slate-600">已添加</span>}
              </button>
            ))}
            <button onClick={() => openAddModal({ id: "", name: "", category: "relay", website: "", openai_url: "", anthropic_url: "", google_url: "" })}
              className="w-full px-3.5 py-2 text-left text-[11px] text-slate-500 hover:bg-white/5 hover:text-slate-300 flex items-center gap-2 cursor-pointer transition-all">
              <Plus className="w-3.5 h-3.5" />自定义中转站
            </button>
          </div>
        )}
      </div>

      {/* Provider List */}
      {config?.providers.length === 0 ? (
        <div className="h-64 border border-dashed border-white/5 rounded-2xl flex flex-col items-center justify-center text-slate-500">
          <Key className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-xs font-bold text-slate-400">尚未配置任何 Provider</span>
        </div>
      ) : config?.providers.map((provider) => {
        const isExpanded = expandedId === provider.id;
        const isActive = config.active_provider === provider.id;
        return (
          <div key={provider.id} className={`rounded-xl border transition-all ${isActive ? "border-violet-500/30 bg-violet-500/5" : "border-white/5 bg-slate-900/30"}`}>
            {/* Header */}
            <div className="p-3.5 flex items-center gap-3 cursor-pointer hover:bg-white/[0.02] transition-all" onClick={() => setExpandedId(isExpanded ? null : provider.id)}>
              {isExpanded ? <ChevronDown className="w-4 h-4 text-slate-500" /> : <ChevronRight className="w-4 h-4 text-slate-500" />}
              <div className="flex-grow min-w-0">
                <div className="flex items-center gap-2 flex-wrap">
                  <span className="text-xs font-bold text-white">{provider.name}</span>
                  {provider.website && (
                    <a href={provider.website} target="_blank" rel="noopener noreferrer"
                      className="text-blue-400 hover:text-blue-300 transition-colors" title="打开官方网站">
                      <ExternalLink className="w-3 h-3" />
                    </a>
                  )}
                  <span className={`px-1.5 py-0.5 rounded text-[8px] font-bold ${provider.category === "relay" ? "bg-cyan-500/15 text-cyan-400" : "bg-emerald-500/15 text-emerald-400"}`}>
                    {provider.category === "relay" ? "中转站" : "供应商"}
                  </span>
                  {/* 原生协议：高亮 */}
                  {provider.openai_enabled && provider.openai_url && !provider.openai_use_proxy && <span className="px-1.5 py-0.5 rounded text-[8px] font-bold bg-blue-500/20 text-blue-300">OpenAI</span>}
                  {provider.anthropic_enabled && provider.anthropic_url && !provider.anthropic_use_proxy && <span className="px-1.5 py-0.5 rounded text-[8px] font-bold bg-amber-500/20 text-amber-300">Anthropic</span>}
                  {provider.google_enabled && <span className="px-1.5 py-0.5 rounded text-[8px] font-bold bg-green-500/20 text-green-300">Google</span>}
                  {/* 代理协议：胶囊形式 */}
                  {provider.anthropic_use_proxy && <span className="px-1.5 py-0.5 rounded-full text-[8px] font-bold bg-pink-500/15 text-pink-300">代理 → Anthropic</span>}
                  {provider.openai_use_proxy && <span className="px-1.5 py-0.5 rounded-full text-[8px] font-bold bg-pink-500/15 text-pink-300">代理 → OpenAI</span>}
                </div>
              </div>
              <div className="flex items-center gap-1.5 flex-shrink-0">
                <button onClick={(e) => { e.stopPropagation(); handleTest(provider); }} disabled={testing === provider.id || !provider.api_key}
                  className="px-2 py-1 rounded-md  hover:bg-white/10 text-[10px] text-slate-400 hover:text-white disabled:opacity-40 cursor-pointer transition-all flex items-center gap-1">
                  <Zap className={`w-3 h-3 ${testing === provider.id ? "animate-pulse text-yellow-400" : ""}`} />
                  {/* {testing === provider.id ? "测试中" : "测速"} */}
                </button>
                <button onClick={(e) => { e.stopPropagation(); handleSetActiveProvider(provider.id); }}
                  className={`px-2 py-1 rounded-md text-[10px] cursor-pointer transition-all ${isActive ? "text-violet-400 bg-violet-500/10" : "text-slate-500 hover:text-violet-300 hover:bg-white/5"}`}
                  title="设为当前供应商">
                  {isActive ? "当前" : "设为当前"}
                </button>
                <button onClick={(e) => { e.stopPropagation(); openEditModal(provider); }}
                  className="p-1 rounded-md text-slate-600 hover:text-blue-400 hover:bg-blue-500/10 cursor-pointer transition-all" title="编辑">
                  <Settings2 className="w-3.5 h-3.5" />
                </button>
                <button onClick={(e) => { e.stopPropagation(); setDeleteTarget(provider.id); }}
                  className="p-1 rounded-md text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all" title="删除">
                  <Trash2 className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>

            {/* Test Result */}
            {testResult?.id === provider.id && (
              <div className={`mx-3.5 mb-2 p-2 rounded-lg text-[10px] font-medium ${testResult.ok ? "bg-emerald-500/10 text-emerald-400" : "bg-red-500/10 text-red-400"}`}>
                <div className="flex items-center gap-1.5 mb-0.5">
                  {testResult.ok ? <CheckCircle className="w-3 h-3" /> : <AlertTriangle className="w-3 h-3" />}
                  <span>{testResult.ok ? "连接成功" : "连接失败"}</span>
                </div>
                <div className="text-[9px] text-slate-400 pl-4 whitespace-pre-line">{testResult.msg}</div>
              </div>
            )}

            {/* Expanded: Models quick view + select */}
            {isExpanded && (
              <div className="px-3.5 pb-3.5 border-t border-white/5 pt-3">
                <div className="flex items-center justify-between mb-1.5">
                  <label className="text-[10px] text-slate-500 font-semibold">模型列表 ({provider.models.length})</label>
                </div>
                {provider.models.length === 0 ? (
                  <div className="text-[10px] text-slate-600 py-2 text-center">暂无模型，点击卡片右上角编辑</div>
                ) : provider.models.map((model) => (
                  <div key={model.id}
                    className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[10px] bg-white/[0.02] border border-transparent">
                    <span className="font-mono text-slate-300">{model.id}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        );
      })}

      {/* ─── 编辑/新增弹框 ─── */}
      {showModal && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4" onClick={() => setShowModal(false)}>
          <div className="w-full max-w-lg bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl flex flex-col max-h-[85vh] overflow-hidden" onClick={e => e.stopPropagation()}>
            {/* Header */}
            <div className="p-4 border-b border-white/5 flex items-center justify-between">
              <h3 className="text-xs font-bold text-slate-200">{modalMode === "add" ? "添加 Provider" : "编辑 Provider"}</h3>
              <button onClick={() => setShowModal(false)} className="text-slate-500 hover:text-slate-300 cursor-pointer"><X className="w-4 h-4" /></button>
            </div>

            {/* Body */}
            <div className="flex-grow overflow-y-auto p-4 space-y-4">
              {/* Name */}
              <div>
                <label className="text-[10px] text-slate-500 font-semibold block mb-1">名称</label>
                <input value={form.name} onChange={e => setForm({ ...form, name: e.target.value })}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-violet-500" />
              </div>

              {/* Website */}
              <div>
                <label className="text-[10px] text-slate-500 font-semibold block mb-1">官方网站</label>
                <input value={form.website} onChange={e => setForm({ ...form, website: e.target.value })} placeholder="https://..."
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-blue-500" />
              </div>

              {/* API Key */}
              <div>
                <label className="text-[10px] text-slate-500 font-semibold block mb-1">API Key</label>
                <input type="password" value={form.api_key} onChange={e => setForm({ ...form, api_key: e.target.value })} placeholder="sk-..."
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500" />
              </div>

              {/* OpenAI Section */}
              <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-[10px] text-blue-300 font-semibold">OpenAI 协议</label>
                  <button onClick={() => setForm({ ...form, openai_enabled: !form.openai_enabled })}
                    className={`relative w-9 h-5 rounded-full transition-all cursor-pointer ${form.openai_enabled ? "bg-blue-600" : "bg-slate-700"}`}>
                    <div className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-all ${form.openai_enabled ? "left-[18px]" : "left-0.5"}`} />
                  </button>
                </div>
                {form.openai_enabled && (
                  <>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input type="checkbox" checked={form.openai_use_proxy} onChange={e => setForm({ ...form, openai_use_proxy: e.target.checked, openai_url: e.target.checked ? "" : form.openai_url })} className="accent-pink-500" />
                      <span className="text-[10px] text-slate-400">启用转换代理（Anthropic → OpenAI）</span>
                    </label>
                    {!form.openai_use_proxy && (
                      <input value={form.openai_url} onChange={e => setForm({ ...form, openai_url: e.target.value })} placeholder="https://api.openai.com/v1"
                        className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-blue-500" />
                    )}
                  </>
                )}
              </div>

              {/* Anthropic Section */}
              <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-[10px] text-amber-300 font-semibold">Anthropic 协议</label>
                  <button onClick={() => setForm({ ...form, anthropic_enabled: !form.anthropic_enabled })}
                    className={`relative w-9 h-5 rounded-full transition-all cursor-pointer ${form.anthropic_enabled ? "bg-amber-600" : "bg-slate-700"}`}>
                    <div className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-all ${form.anthropic_enabled ? "left-[18px]" : "left-0.5"}`} />
                  </button>
                </div>
                {form.anthropic_enabled && (
                  <>
                    <label className="flex items-center gap-2 cursor-pointer">
                      <input type="checkbox" checked={form.anthropic_use_proxy} onChange={e => setForm({ ...form, anthropic_use_proxy: e.target.checked, anthropic_url: e.target.checked ? "" : form.anthropic_url })} className="accent-pink-500" />
                      <span className="text-[10px] text-slate-400">启用转换代理（OpenAI → Anthropic）</span>
                    </label>
                    {!form.anthropic_use_proxy && (
                      <input value={form.anthropic_url} onChange={e => setForm({ ...form, anthropic_url: e.target.value })} placeholder="https://api.anthropic.com"
                        className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-amber-500" />
                    )}
                  </>
                )}
              </div>

              {/* Google Section (Gemini CLI) */}
              <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-[10px] text-green-300 font-semibold">Google 协议（Gemini CLI）</label>
                  <button onClick={() => setForm({ ...form, google_enabled: !form.google_enabled })}
                    className={`relative w-9 h-5 rounded-full transition-all cursor-pointer ${form.google_enabled ? "bg-green-600" : "bg-slate-700"}`}>
                    <div className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-all ${form.google_enabled ? "left-[18px]" : "left-0.5"}`} />
                  </button>
                </div>
                {form.google_enabled && (
                  <input value={form.google_url} onChange={e => setForm({ ...form, google_url: e.target.value })}
                    placeholder="留空使用官方端点，或填写自定义 GOOGLE_GEMINI_BASE_URL"
                    className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-green-500" />
                )}
              </div>

              {/* ─── 模型别名映射：按协议分组 ─── */}
              {/* Anthropic 协议别名（role → model） */}
              {((form.anthropic_enabled || form.openai_use_proxy) && form.anthropic_enabled) && (
                <div className="p-3 rounded-lg bg-slate-900/50 border border-amber-500/20 space-y-2">
                  <label className="text-[10px] text-amber-300 font-semibold block">Anthropic 模型映射</label>
                  <p className="text-[9px] text-slate-600">
                    {form.anthropic_use_proxy
                      ? "代理模式：Claude Code 发送 claude-sonnet-4 等模型名时，代理自动映射"
                      : form.openai_use_proxy
                      ? "代理模式（OpenAI→Anthropic）：模型名映射"
                      : "直连模式：通过 ANTHROPIC_DEFAULT_XXX_MODEL 环境变量，Anthropic SDK 自动将角色映射到指定模型"}
                  </p>
                  {["sonnet", "opus", "haiku", "fable"].map(key => (
                    <div key={key} className="flex items-center gap-2">
                      <span className="text-[10px] text-slate-400 font-mono w-16 flex-shrink-0">{key}</span>
                      <span className="text-[9px] text-slate-600 flex-shrink-0">→</span>
                      <select
                        value={form.anthropic_model_aliases?.[key] || ""}
                        onChange={e => {
                          const next = { ...(form.anthropic_model_aliases || {}) };
                          if (e.target.value) next[key] = e.target.value;
                          else delete next[key];
                          setForm({ ...form, anthropic_model_aliases: next });
                        }}
                        className="flex-1 bg-slate-900 border border-white/10 rounded-md px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-amber-500"
                      >
                        <option value="">不映射</option>
                        {modelsText.split("\n").filter(l => l.trim()).map(m => (
                          <option key={m.trim()} value={m.trim()}>{m.trim()}</option>
                        ))}
                      </select>
                    </div>
                  ))}
                  <div className="flex items-center gap-2 pt-1 border-t border-white/5">
                    <span className="text-[10px] text-slate-400 font-mono w-16 flex-shrink-0">默认</span>
                    <span className="text-[9px] text-slate-600 flex-shrink-0">→</span>
                    <select
                      value={form.anthropic_default_model || ""}
                      onChange={e => setForm({ ...form, anthropic_default_model: e.target.value || null })}
                      className="flex-1 bg-slate-900 border border-white/10 rounded-md px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-amber-500"
                    >
                      <option value="">不设置</option>
                      {modelsText.split("\n").filter(l => l.trim()).map(m => (
                        <option key={m.trim()} value={m.trim()}>{m.trim()}</option>
                      ))}
                    </select>
                  </div>
                </div>
              )}

              {/* OpenAI 协议别名（未来扩展） */}
              {form.openai_enabled && !form.openai_use_proxy && (
                <div className="p-3 rounded-lg bg-slate-900/50 border border-blue-500/20 space-y-2">
                  <label className="text-[10px] text-blue-300 font-semibold block">OpenAI 模型映射</label>
                  <p className="text-[9px] text-slate-600">预留：未来 OpenAI 协议工具的角色映射（当前暂未生效）</p>
                </div>
              )}

              {/* Google 协议别名（未来扩展） */}
              <div>
                <div className="flex items-center justify-between mb-1">
                  <label className="text-[10px] text-slate-500 font-semibold">
                    模型列表 <span className="text-slate-600">（一行一个模型 ID）</span>
                  </label>
                  <button
                    onClick={handleFetchModels}
                    disabled={fetchingModels || (!form.openai_url && !form.anthropic_url) || !form.api_key}
                    className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-[9px] font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-40 disabled:cursor-not-allowed"
                  >
                    <RefreshCw className={`w-3 h-3 ${fetchingModels ? "animate-spin" : ""}`} />
                    {fetchingModels ? "获取中..." : "自动获取"}
                  </button>
                </div>
                <textarea
                  value={modelsText}
                  onChange={e => setModelsText(e.target.value)}
                  rows={6}
                  placeholder={"gpt-4o\ngpt-4o-mini\nclaude-sonnet-4-20250514\ndeepseek-chat"}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500 resize-y leading-5"
                />
                <div className="text-[9px] text-slate-600 mt-1">
                  已录入 {modelsText.split("\n").filter(l => l.trim()).length} 个模型
                </div>
              </div>

              {/* Error */}
              {formError && (
                <div className="p-2 rounded-lg bg-red-500/10 border border-red-500/20 text-[10px] text-red-400 flex items-center gap-1.5">
                  <AlertTriangle className="w-3.5 h-3.5 flex-shrink-0" />{formError}
                </div>
              )}
            </div>

            {/* Footer */}
            <div className="p-4 border-t border-white/5 bg-slate-900/20 flex justify-end gap-2">
              <button onClick={() => setShowModal(false)}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer">取消</button>
              <button onClick={handleModalConfirm}
                className="px-3.5 py-1.5 rounded-lg bg-violet-600 hover:bg-violet-500 text-white text-[10px] font-semibold cursor-pointer">确定</button>
            </div>
          </div>
        </div>
      )}

      {/* ─── 删除确认弹框 ─── */}
      {deleteTarget && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4" onClick={() => setDeleteTarget(null)}>
          <div className="w-full max-w-sm bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl p-5" onClick={e => e.stopPropagation()}>
            <div className="flex items-center gap-3 mb-4">
              <div className="p-2 rounded-lg bg-red-500/10"><Trash2 className="w-4 h-4 text-red-400" /></div>
              <div>
                <h3 className="text-xs font-bold text-slate-200">确认删除</h3>
                <p className="text-[10px] text-slate-500 mt-0.5">删除后不可恢复，确定要删除 {config?.providers.find(p => p.id === deleteTarget)?.name} 吗？</p>
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <button onClick={() => setDeleteTarget(null)}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer">取消</button>
              <button onClick={() => handleDelete(deleteTarget)}
                className="px-3.5 py-1.5 rounded-lg bg-red-600 hover:bg-red-500 text-white text-[10px] font-semibold cursor-pointer">删除</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
