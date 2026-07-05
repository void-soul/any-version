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

interface ModelEntry { id: string; name: string; }
interface AiProvider {
  id: string;
  name: string;
  category: string;
  api_key: string;
  website: string;
  openai_enabled: boolean;
  openai_url: string;
  openai_use_proxy: boolean;
  anthropic_enabled: boolean;
  anthropic_url: string;
  anthropic_use_proxy: boolean;
  google_enabled: boolean;
  google_url: string;
  model_aliases: Record<string, string>;
  default_model: string | null;
  models: ModelEntry[];
  active_model_id: string | null;
}
interface AiConfig {
  providers: AiProvider[];
  active_provider: string | null;
  active_model: string | null;
  proxy_port: number;
  default_project_path: string;
}

type Preset = {
  id: string; name: string; category: string;
  website: string; openai_url: string; anthropic_url: string;
  google_url?: string;
};

const PROVIDER_PRESETS: Preset[] = [
  { id: "openai", name: "OpenAI", category: "provider", website: "https://openai.com", openai_url: "https://api.openai.com/v1", anthropic_url: "" },
  { id: "anthropic", name: "Anthropic", category: "provider", website: "https://www.anthropic.com", openai_url: "", anthropic_url: "https://api.anthropic.com" },
  { id: "deepseek", name: "DeepSeek", category: "provider", website: "https://deepseek.com", openai_url: "https://api.deepseek.com", anthropic_url: "https://api.deepseek.com/anthropic" },
  { id: "volcengine", name: "火山引擎", category: "provider", website: "https://www.volcengine.com", openai_url: "https://ark.cn-beijing.volces.com/api/coding/v3", anthropic_url: "https://ark.cn-beijing.volces.com/api/coding" },
  { id: "qwen", name: "阿里百炼 Qwen", category: "provider", website: "https://bailian.aliyun.com", openai_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1", anthropic_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic" },
  { id: "kimi", name: "Kimi 月之暗面", category: "provider", website: "https://kimi.moonshot.cn", openai_url: "https://api.moonshot.cn/v1", anthropic_url: "https://api.moonshot.cn/anthropic" },
  { id: "glm", name: "GLM 智谱", category: "provider", website: "https://open.bigmodel.cn", openai_url: "https://open.bigmodel.cn/api/coding/paas/v4", anthropic_url: "https://open.bigmodel.cn/api/anthropic" },
  { id: "hunyuan", name: "腾讯混元", category: "provider", website: "https://hunyuan.tencent.com", openai_url: "https://api.lkeap.cloud.tencent.com/plan/v3", anthropic_url: "https://api.lkeap.cloud.tencent.com/plan/anthropic" },
  { id: "ernie", name: "百度千帆", category: "provider", website: "https://qianfan.cloud.baidu.com", openai_url: "https://qianfan.baidubce.com/v2/coding", anthropic_url: "https://qianfan.baidubce.com/anthropic/coding" },
  { id: "stepfun", name: "阶跃星辰", category: "provider", website: "https://stepfun.com", openai_url: "https://api.stepfun.com/v1", anthropic_url: "https://api.stepfun.com" },
  { id: "minimax", name: "MiniMax", category: "provider", website: "https://minimax.io", openai_url: "https://api.minimax.io/v1", anthropic_url: "https://api.minimax.io/anthropic" },
  { id: "xiaomi", name: "小米 MiMo", category: "provider", website: "https://mimo.xiaomi.com", openai_url: "https://token-plan-cn.xiaomimimo.com/v1", anthropic_url: "https://token-plan-cn.xiaomimimo.com/anthropic" },
  { id: "google", name: "Google Gemini", category: "provider", website: "https://gemini.google.com", openai_url: "https://generativelanguage.googleapis.com/v1beta/openai", anthropic_url: "" },
  { id: "xai", name: "xAI Grok", category: "provider", website: "https://x.ai", openai_url: "https://api.x.ai/v1", anthropic_url: "" },
  { id: "mistral", name: "Mistral AI", category: "provider", website: "https://mistral.ai", openai_url: "https://api.mistral.ai/v1", anthropic_url: "" },
  { id: "groq", name: "Groq", category: "provider", website: "https://groq.com", openai_url: "https://api.groq.com/openai/v1", anthropic_url: "" },
  { id: "nvidia", name: "NVIDIA", category: "provider", website: "https://build.nvidia.com", openai_url: "https://integrate.api.nvidia.com/v1", anthropic_url: "" },
  { id: "agnes", name: "Agnes AI", category: "provider", website: "https://agnes-ai.com", openai_url: "https://apihub.agnes-ai.com/v1", anthropic_url: "" },
  { id: "siliconflow", name: "SiliconFlow", category: "provider", website: "https://siliconflow.cn", openai_url: "https://api.siliconflow.cn/v1", anthropic_url: "" },
  { id: "longcat", name: "美团 LongCat", category: "provider", website: "https://longcat.chat", openai_url: "https://api.longcat.chat/openai", anthropic_url: "https://api.longcat.chat/anthropic" },
  { id: "sensenova", name: "商汤日日新", category: "provider", website: "https://platform.sensenova.cn", openai_url: "https://token.sensenova.cn/v1", anthropic_url: "https://token.sensenova.cn/v1/messages" },
];

const RELAY_PRESETS: Preset[] = [
  { id: "openrouter", name: "OpenRouter", category: "relay", website: "https://openrouter.ai", openai_url: "https://openrouter.ai/api/v1", anthropic_url: "" },
  { id: "worldrouter", name: "WorldRouter", category: "relay", website: "https://worldrouter.ai", openai_url: "https://inference-api.worldrouter.ai/v1", anthropic_url: "https://inference-api.worldrouter.ai" },
  { id: "bai", name: "B.ai", category: "relay", website: "https://theb.ai", openai_url: "https://api.theb.ai/v1", anthropic_url: "" },
  { id: "nekocode", name: "NekoCode", category: "relay", website: "https://nekocode.ai", openai_url: "", anthropic_url: "https://nekocode.ai" },
  { id: "code0", name: "Code0.ai", category: "relay", website: "https://code0.ai", openai_url: "https://code0.ai/v1", anthropic_url: "https://code0.ai" },
  { id: "amux", name: "Amux", category: "relay", website: "https://amux.ai", openai_url: "", anthropic_url: "https://api.amux.ai" },
  { id: "teamorouter", name: "TeamoRouter", category: "relay", website: "https://teamorouter.com", openai_url: "", anthropic_url: "https://api.teamorouter.com" },
  { id: "zetaapi", name: "ZetaAPI", category: "relay", website: "https://zetaapi.ai", openai_url: "", anthropic_url: "https://api.zetaapi.ai" },
  { id: "fennoai", name: "FennoAI", category: "relay", website: "https://api.fenno.ai", openai_url: "", anthropic_url: "https://api.fenno.ai" },
  { id: "qiniu", name: "七牛云", category: "relay", website: "https://s.qiniu.com/nMvAvy", openai_url: "https://api.qnaigc.com/v1", anthropic_url: "https://api.qnaigc.com" },
];

const EMPTY_PROVIDER: AiProvider = {
  id: "", name: "", category: "provider", api_key: "", website: "",
  openai_enabled: false, openai_url: "", openai_use_proxy: false,
  anthropic_enabled: false, anthropic_url: "", anthropic_use_proxy: false,
  google_enabled: false, google_url: "",
  model_aliases: {}, default_model: null,
  models: [], active_model_id: null,
};

export default function ModelConfig() {
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [showAddMenu, setShowAddMenu] = useState(false);

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
      const data = await invoke<AiConfig>("get_ai_config");
      setConfig(data);
    } catch {
      setConfig({ providers: [], active_provider: null, active_model: null, proxy_port: 15721, default_project_path: "" });
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

  const handleModalConfirm = () => {
    const err = validateForm();
    if (err) { setFormError(err); return; }

    if (!config) return;

    // 解析模型文本：每行一个 model id
    const models: ModelEntry[] = modelsText
      .split("\n")
      .map(line => line.trim())
      .filter(line => line.length > 0)
      .map(line => ({ id: line, name: line }));

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
      active_model: config.active_provider === id ? null : config.active_model,
    };
    saveConfig(next);
    setDeleteTarget(null);
    if (expandedId === id) setExpandedId(null);
  };

  // ─── 设为当前 ───

  const handleSetActive = (providerId: string, modelId: string) => {
    if (!config) return;
    saveConfig({ ...config, active_provider: providerId, active_model: modelId });
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
      {/* Active Model Banner */}
      {config?.active_provider && config.active_model && (() => {
        const ap = config.providers.find(p => p.id === config.active_provider);
        const am = ap?.models.find(m => m.id === config.active_model);
        return ap ? (
          <div className="p-3.5 rounded-xl bg-violet-500/10 border border-violet-500/20 flex items-center gap-3">
            <CheckCircle className="w-4 h-4 text-violet-400 flex-shrink-0" />
            <div className="text-xs">
              <span className="text-slate-400">当前模型：</span>
              <span className="text-violet-300 font-bold ml-1">{ap.name}</span>
              <span className="text-slate-500 mx-1">/</span>
              <span className="text-white font-semibold">{am?.name || config.active_model}</span>
            </div>
          </div>
        ) : null;
      })()}

      {/* Add Button */}
      <div className="relative">
        <button onClick={() => setShowAddMenu(!showAddMenu)} className="px-3.5 py-2 rounded-xl bg-violet-600 hover:bg-violet-500 text-white text-[11px] font-semibold flex items-center gap-1.5 cursor-pointer shadow-lg shadow-violet-500/10">
          <Plus className="w-3.5 h-3.5" /> 添加 Provider
        </button>
        {showAddMenu && (
          <div className="absolute top-full left-0 mt-1 w-72 bg-slate-900 border border-white/10 rounded-xl shadow-2xl z-50 overflow-hidden max-h-[70vh] overflow-y-auto">
            <div className="px-3 pt-2.5 pb-1 text-[9px] font-bold text-slate-500 uppercase tracking-wider">供应商</div>
            {PROVIDER_PRESETS.map((p) => (
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
            {RELAY_PRESETS.map((p) => (
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
                  {isActive && <span className="px-1.5 py-0.5 rounded text-[8px] font-bold bg-violet-500/20 text-violet-300">当前</span>}
                </div>
              </div>
              <div className="flex items-center gap-1.5 flex-shrink-0">
                <button onClick={(e) => { e.stopPropagation(); handleTest(provider); }} disabled={testing === provider.id || !provider.api_key}
                  className="px-2 py-1 rounded-md bg-white/5 hover:bg-white/10 text-[10px] text-slate-400 hover:text-white disabled:opacity-40 cursor-pointer transition-all flex items-center gap-1">
                  <Zap className={`w-3 h-3 ${testing === provider.id ? "animate-pulse text-yellow-400" : ""}`} />
                  {testing === provider.id ? "测试中" : "测速"}
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
                ) : provider.models.map((model) => {
                  const isModelActive = isActive && config.active_model === model.id;
                  return (
                    <button key={model.id} onClick={(e) => { e.stopPropagation(); handleSetActive(provider.id, model.id); }}
                      className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[10px] text-left transition-all cursor-pointer ${
                        isModelActive ? "bg-violet-500/10 border border-violet-500/20" : "bg-white/[0.02] border border-transparent hover:bg-white/5"
                      }`}>
                      <span className={`font-mono ${isModelActive ? "text-violet-300 font-bold" : "text-slate-300"}`}>
                        {model.id}
                      </span>
                      {isModelActive && <CheckCircle className="w-3 h-3 text-violet-400 ml-auto" />}
                    </button>
                  );
                })}
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

              {/* Model Aliases — 将 Claude 的模型角色关键词映射到实际模型 */}
              {(form.openai_use_proxy || form.anthropic_enabled) && (
                <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-2">
                  <label className="text-[10px] text-slate-300 font-semibold block">模型别名映射</label>
                  <p className="text-[9px] text-slate-600">
                    {form.openai_use_proxy || form.anthropic_use_proxy
                      ? "代理模式：Claude Code 发送 claude-sonnet-4 等模型名时，代理将其映射为你选择的实际上游模型"
                      : "直连模式：通过 ANTHROPIC_DEFAULT_XXX_MODEL 环境变量，让 Anthropic SDK 自动将角色关键词映射到指定模型"}
                  </p>
                  {["sonnet", "opus", "haiku", "fable"].map(key => (
                    <div key={key} className="flex items-center gap-2">
                      <span className="text-[10px] text-slate-400 font-mono w-16 flex-shrink-0">{key}</span>
                      <span className="text-[9px] text-slate-600 flex-shrink-0">→</span>
                      <select
                        value={form.model_aliases?.[key] || ""}
                        onChange={e => {
                          const next = { ...(form.model_aliases || {}) };
                          if (e.target.value) next[key] = e.target.value;
                          else delete next[key];
                          setForm({ ...form, model_aliases: next });
                        }}
                        className="flex-1 bg-slate-900 border border-white/10 rounded-md px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-violet-500"
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
                      value={form.default_model || ""}
                      onChange={e => setForm({ ...form, default_model: e.target.value || null })}
                      className="flex-1 bg-slate-900 border border-white/10 rounded-md px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-violet-500"
                    >
                      <option value="">不设置</option>
                      {modelsText.split("\n").filter(l => l.trim()).map(m => (
                        <option key={m.trim()} value={m.trim()}>{m.trim()}</option>
                      ))}
                    </select>
                  </div>
                </div>
              )}

              {/* Models Textarea */}
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
