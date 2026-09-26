import { useState, useEffect, useCallback, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
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
  Search,
  Laptop,
  Wallet,
  X,
  Settings2,
  ExternalLink,
  Eye,
  EyeOff,
  FolderOpen,
} from "lucide-react";
import type { ModelEntry, AiProvider, AiConfig, ModelCustomParam, UpstreamHeader } from "./types";
import { filterProviders } from "./providerSearch";

type Preset = {
  id: string; name: string; category: string;
  website: string; openai_url: string; anthropic_url: string;
  google_url: string;
};

const EMPTY_PROVIDER: AiProvider = {
  id: "", name: "", category: "provider", api_key: "", website: "",
  openai_url: "", anthropic_url: "", google_url: "",
  models: [], active_model_id: null, custom_headers: [],
  openai_include_v1: null, anthropic_include_v1: null,
};

/// 「是否包含 /v1」三态 → 下拉框字符串（null 即自动）。
function v1ToSelect(v?: boolean | null): string {
  return v === true ? "yes" : v === false ? "no" : "auto";
}
function selectToV1(s: string): boolean | null {
  return s === "yes" ? true : s === "no" ? false : null;
}

/// 传输层 / 逐跳头：由 HTTP 客户端按实际报文决定，后端也会拒绝，这里提前拦。
const FORBIDDEN_HEADER_NAMES = new Set([
  "host", "content-length", "transfer-encoding", "connection", "keep-alive",
  "proxy-connection", "te", "trailer", "upgrade", "expect",
]);

/// RFC 7230 token 字符集（HTTP 头名称的合法字符）
const HEADER_NAME_RE = /^[A-Za-z0-9!#$%&'*+\-.^_`|~]+$/;

/// 从预设（可能含多个协议端点）取出全部协议 URL
function presetUrls(p: Preset): { openai_url: string; anthropic_url: string; google_url: string } {
  return { openai_url: p.openai_url, anthropic_url: p.anthropic_url, google_url: p.google_url };
}

export default function ModelConfig() {
  const { t } = useTranslation();
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [loading, setLoading] = useState(true);
  // 详情弹窗（单供应商）与余额查询状态
  const [detailId, setDetailId] = useState<string | null>(null);
  const [balanceState, setBalanceState] = useState<{
    pid: string; loading: boolean; error: string | null;
    items: { key: string; value: string }[];
  } | null>(null);
  const [showPresetPicker, setShowPresetPicker] = useState(false);
  const [presetSearch, setPresetSearch] = useState("");
  const [presetCategory, setPresetCategory] = useState<"all" | "provider" | "relay" | "local">("all");
  // 「已添加的供应商」列表的搜索关键词（与预设弹窗的 presetSearch 互不影响）
  const [providerSearch, setProviderSearch] = useState("");
  const [presets, setPresets] = useState<Preset[]>([]);

  // 弹框状态
  const [showModal, setShowModal] = useState(false);
  const [modalMode, setModalMode] = useState<"add" | "edit">("add");
  const [form, setForm] = useState<AiProvider>({ ...EMPTY_PROVIDER });
  const [formError, setFormError] = useState<string | null>(null);
  // API Key 明文显示开关（仅影响弹框输入框的 type，不影响保存值）
  const [showApiKey, setShowApiKey] = useState(false);
  // 模型批量录入文本（一行一个 model_id 或 "model_id | 显示名"）
  const [modelsText, setModelsText] = useState("");
  const [fetchingModels, setFetchingModels] = useState(false);
  // 模型自定义启动参数：model_id → 参数列表（与 form.models 中的 customParams 双向同步）
  const [modelParams, setModelParams] = useState<Record<string, ModelCustomParam[]>>({});

  // 同步：modelsText 每行一个 model id，保证 modelParams 对每个 id 都有入口（保留已有）
  useEffect(() => {
    const ids = modelsText.split("\n").map(l => l.trim()).filter(Boolean);
    setModelParams(prev => {
      const next: Record<string, ModelCustomParam[]> = {};
      let changed = false;
      for (const id of ids) {
        if (Object.prototype.hasOwnProperty.call(prev, id)) {
          next[id] = prev[id];
        } else {
          next[id] = [];
          changed = true;
        }
      }
      if (Object.keys(prev).length !== ids.length) changed = true;
      return changed ? next : prev;
    });
  }, [modelsText]);

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
      setConfig({ providers: [], proxy_port: 15721, default_project_path: "", rectifier: { enabled: false, thinking_signature: false, thinking_budget: false, media_fallback: false, media_heuristic: false, protocol_mismatch: false }, headroom: { enabled: false, port: 8791, on_unavailable: "failOpen", disable_kompress: false, timeout_ms: 1500 }, optimizer: { enabled: false, cache_injection: false, thinking_optimizer: false, deepseek_normalize: false }, skills_dir: "" });
    } finally { setLoading(false); }
  }, []);

  useEffect(() => { loadConfig(); }, [loadConfig]);

  // 预设选择弹窗：关键词 + 分类过滤（分类 all/provider/relay/local）
  const filteredPresets = useMemo(() => {
    const kw = presetSearch.trim().toLowerCase();
    return presets.filter((p) => {
      if (presetCategory !== "all" && p.category !== presetCategory) return false;
      if (!kw) return true;
      return (
        p.name.toLowerCase().includes(kw) ||
        p.id.toLowerCase().includes(kw) ||
        p.website.toLowerCase().includes(kw) ||
        p.openai_url.toLowerCase().includes(kw) ||
        p.anthropic_url.toLowerCase().includes(kw)
      );
    });
  }, [presets, presetSearch, presetCategory]);

  // 已添加供应商的搜索：与预设弹窗同一口径（显示名 / id / 官网 / 协议端点）
  const filteredProviders = useMemo(
    () => filterProviders(config?.providers ?? [], providerSearch),
    [config?.providers, providerSearch],
  );

  const presetCategoryCounts = useMemo(() => {
    const counts = { all: presets.length, provider: 0, relay: 0, local: 0 };
    for (const p of presets) {
      if (p.category === "provider") counts.provider += 1;
      else if (p.category === "relay") counts.relay += 1;
      else if (p.category === "local") counts.local += 1;
    }
    return counts;
  }, [presets]);

  const saveConfig = async (next: AiConfig) => {
    setConfig(next);
    try { await invoke("save_ai_config", { config: next }); } catch (e) { console.error(e); }
  };

  // 默认项目目录：输入期间本地缓冲，失焦时才落盘（避免每次按键都写配置文件）
  const [defaultProject, setDefaultProject] = useState("");
  useEffect(() => {
    setDefaultProject(config?.default_project_path ?? "");
  }, [config?.default_project_path]);

  const browseDefaultProject = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, title: t("settings.chooseFolder") });
      if (selected && config) {
        setDefaultProject(selected as string);
        await saveConfig({ ...config, default_project_path: selected as string });
      }
    } catch {
      alert(t("settings.folderPickerUnavailable"));
    }
  };

  const commitDefaultProject = () => {
    if (!config) return;
    if (defaultProject === (config.default_project_path ?? "")) return;
    void saveConfig({ ...config, default_project_path: defaultProject });
  };

  // ─── 弹框操作 ───

  const openAddModal = (preset?: Preset) => {
    setModalMode("add");
    const urls = preset ? presetUrls(preset) : { openai_url: "", anthropic_url: "", google_url: "" };
    setForm({
      ...EMPTY_PROVIDER,
      id: preset?.id || `custom_${Date.now()}`,
      name: preset?.name || "",
      category: preset?.category || "provider",
      website: preset?.website || "",
      openai_url: urls.openai_url,
      anthropic_url: urls.anthropic_url,
      google_url: urls.google_url,
    });
    setModelsText("");
    setFormError(null);
    setShowApiKey(false);
    setShowModal(true);
    setShowPresetPicker(false);
  };

  const openEditModal = (provider: AiProvider) => {
    setModalMode("edit");
    // 旧配置可能没有 custom_headers 字段
    setForm({ ...provider, custom_headers: provider.custom_headers ?? [] });
    // 模型列表转为文本：每行一个 id
    setModelsText(provider.models.map(m => m.id).join("\n"));
    // 初始化每个模型的自定义参数
    const mp: Record<string, ModelCustomParam[]> = {};
    for (const m of provider.models) mp[m.id] = m.customParams ? [...m.customParams] : [];
    setModelParams(mp);
    setFormError(null);
    setShowApiKey(false);
    setShowModal(true);
  };

  // ─── 自定义上游请求头编辑 ───

  const addCustomHeader = () =>
    setForm(f => ({ ...f, custom_headers: [...f.custom_headers, { key: "", value: "" }] }));

  const updateCustomHeader = (idx: number, patch: Partial<UpstreamHeader>) =>
    setForm(f => ({
      ...f,
      custom_headers: f.custom_headers.map((h, i) => (i === idx ? { ...h, ...patch } : h)),
    }));

  const removeCustomHeader = (idx: number) =>
    setForm(f => ({ ...f, custom_headers: f.custom_headers.filter((_, i) => i !== idx) }));

  /// 与后端 `proxy::headers::validate` 同规则：非法名称 / 传输层头 / 重名都拦在保存前，
  /// 让用户当场看到问题，而不是保存时才被后端拒绝。报错只显示头名称（值可能是凭据）。
  const validateCustomHeaders = (headers: UpstreamHeader[]): string | null => {
    const seen = new Set<string>();
    for (const h of headers) {
      const key = h.key.trim();
      // 未填写的占位行视为未配置
      if (!key) continue;
      if (!HEADER_NAME_RE.test(key)) return t("modelcfg.headerInvalid", { key });
      const lower = key.toLowerCase();
      if (FORBIDDEN_HEADER_NAMES.has(lower)) return t("modelcfg.headerForbidden", { key });
      if (seen.has(lower)) return t("modelcfg.headerDuplicate", { key });
      seen.add(lower);
    }
    return null;
  };

  const validateForm = (): string | null => {
    if (!form.name.trim()) return t("modelcfg.nameRequired");
    if (!form.openai_url.trim() && !form.anthropic_url.trim() && !form.google_url.trim())
      return t("modelcfg.urlRequired");
    if (!form.api_key.trim()) return t("modelcfg.keyRequired");
    return validateCustomHeaders(form.custom_headers);
  };

  // ─── 模型自定义启动参数编辑 ───
  const addModelParam = (mid: string) => {
    setModelParams(prev => ({
      ...prev,
      [mid]: [...(prev[mid] || []), { key: "", label: "", paramType: "enum", options: [], target: "env", envKey: "" }],
    }));
  };
  const updateModelParam = (mid: string, idx: number, patch: Partial<ModelCustomParam>) => {
    setModelParams(prev => ({
      ...prev,
      [mid]: (prev[mid] || []).map((cp, i) => i === idx ? { ...cp, ...patch } : cp),
    }));
  };
  const removeModelParam = (mid: string, idx: number) => {
    setModelParams(prev => ({
      ...prev,
      [mid]: (prev[mid] || []).filter((_, i) => i !== idx),
    }));
  };

  const handleModalConfirm = async () => {
    const err = validateForm();
    if (err) { setFormError(err); return; }

    if (!config) return;

    // 解析模型文本：每行一个 model id；保留已存在模型的 customParams
    const prevById = new Map(form.models.map(m => [m.id, m]));
    const manualModels: ModelEntry[] = modelsText
      .split("\n")
      .map(line => line.trim())
      .filter(line => line.length > 0)
      .map(line => {
        const prev = prevById.get(line);
        const custom = modelParams[line] || [];
        return prev ? { ...prev, customParams: custom } : { id: line, name: line, customParams: custom };
      });

    // 新建供应商时，如果用户未手动录入模型，自动从 API 获取模型列表
    let autoModels: ModelEntry[] = [];
    if (modalMode === "add" && manualModels.length === 0) {
      const url = form.openai_url || form.anthropic_url || form.google_url || "";
      if (url && form.api_key) {
        try {
          const fetched: string[] = await invoke("fetch_provider_models", {
            baseUrl: url,
            apiKey: form.api_key,
            headers: form.custom_headers,
          });
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
  };

  // ─── 删除 ───

  const handleDelete = (id: string) => {
    if (!config) return;
    const next: AiConfig = {
      ...config,
      providers: config.providers.filter(p => p.id !== id),
    };
    saveConfig(next);
    setDeleteTarget(null);
  };

  // ─── 自动获取模型列表 ───

  const handleFetchModels = async () => {
    const url = form.openai_url || form.anthropic_url || form.google_url || "";
    if (!url) {
      setFormError(t("modelcfg.fillUrl"));
      return;
    }
    if (!form.api_key) {
      setFormError(t("modelcfg.fillKey"));
      return;
    }
    setFetchingModels(true);
    setFormError(null);
    try {
      const models = await invoke<string[]>("fetch_provider_models", {
        baseUrl: url,
        apiKey: form.api_key,
        headers: form.custom_headers,
      });
      if (models.length === 0) {
        setFormError(t("modelcfg.noModels"));
      } else {
        setModelsText(models.join("\n"));
      }
    } catch (e: any) {
      setFormError(t("modelcfg.fetchModelsFail", { err: String(e) }));
    } finally {
      setFetchingModels(false);
    }
  };

  // ─── 测速 ───

  const handleTest = async (provider: AiProvider) => {
    setTesting(provider.id);
    setTestResult(null);
    try {
      const testUrl = provider.openai_url || provider.anthropic_url || provider.google_url || "";
      const testProtocol = provider.openai_url ? "openai" : provider.anthropic_url ? "anthropic" : "google";
      const result = await invoke<{ success: boolean; message: string; latency_ms: number }>("test_model_connection", {
        baseUrl: testUrl,
        protocol: testProtocol,
        apiKey: provider.api_key,
        headers: provider.custom_headers ?? [],
      });
      setTestResult({ id: provider.id, ok: result.success, msg: result.message });
    } catch (e: any) {
      setTestResult({ id: provider.id, ok: false, msg: String(e) });
    } finally { setTesting(null); }
  };

  // ─── 余额查询（仅支持官方余额端点的预设） ───

  const BALANCE_CAPABLE = new Set(["deepseek", "siliconflow", "openrouter"]);

  const balanceLabel = (key: string) => {
    if (key === "state") return t("modelcfg.balState");
    return t(`modelcfg.bal_${key}`);
  };

  const runBalance = async (provider: AiProvider) => {
    setBalanceState({ pid: provider.id, loading: true, error: null, items: [] });
    try {
      const resp = await invoke<{ items: { key: string; value: string }[] }>("query_provider_balance", {
        providerId: provider.id,
        apiKey: provider.api_key,
      });
      setBalanceState({ pid: provider.id, loading: false, error: null, items: resp.items });
    } catch (e: any) {
      setBalanceState({ pid: provider.id, loading: false, error: String(e), items: [] });
    }
  };

  const openDetail = (provider: AiProvider, autoBalance = false) => {
    setDetailId(provider.id);
    setBalanceState(null);
    if (autoBalance) void runBalance(provider);
  };

  const detailProvider = config?.providers.find(p => p.id === detailId) ?? null;

  if (loading) {
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-xs">{t("modelcfg.loading")}</span></div>;
  }

  return (
    <div className="h-full overflow-y-auto p-6 space-y-4">
      {/* AI 默认项目目录（模块专属设置，原属全局设置页） */}
      <div className="rounded-xl border border-white/5 bg-slate-900/30 p-3.5 space-y-2">
        <div className="flex items-center gap-2">
          <FolderOpen className="w-3.5 h-3.5 text-[var(--module-accent)]" />
          <span className="text-[11px] font-semibold text-slate-200">
            {t("modelcfg.defaultProject")}
          </span>
        </div>
        <div className="flex items-center gap-1.5">
          <input
            value={defaultProject}
            onChange={(e) => setDefaultProject(e.target.value)}
            onBlur={commitDefaultProject}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitDefaultProject();
            }}
            placeholder={t("modelcfg.defaultProjectPh")}
            className="flex-1 h-9 px-2.5 rounded-xl bg-white/5 border border-white/10 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
          />
          <button
            type="button"
            onClick={browseDefaultProject}
            className="h-9 px-3 rounded-xl bg-white/10 hover:bg-white/20 text-white transition-colors cursor-pointer flex items-center justify-center"
            title={t("settings.chooseFolder")}
          >
            <FolderOpen className="w-4 h-4" />
          </button>
        </div>
        <p className="text-[10px] text-slate-500 leading-relaxed">
          {t("modelcfg.defaultProjectHint")}
        </p>
      </div>

      {/* Add Button + 已添加供应商的搜索 */}
      <div className="flex items-center gap-2">
        <button onClick={() => {
          // 打开前重新拉取预设：「本地聚合」的端口取自聚合页设置，可能已改动
          void invoke<Preset[]>("get_provider_presets").then(list => setPresets(list)).catch(() => {});
          setPresetSearch("");
          setPresetCategory("all");
          setShowPresetPicker(true);
        }} className="px-3.5 py-2 rounded-xl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-[11px] font-semibold flex items-center gap-1.5 cursor-pointer shadow-lg shadow-[var(--module-accent-ring)] flex-shrink-0">
          <Plus className="w-3.5 h-3.5" /> {t("modelcfg.addProvider")}
        </button>
        {/* 一个供应商都没添加时不显示搜索框：没有东西可筛 */}
        {(config?.providers.length ?? 0) > 0 && (
          <div className="relative flex-1 max-w-xs">
            <Search className="w-3.5 h-3.5 text-slate-500 absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              value={providerSearch}
              onChange={e => setProviderSearch(e.target.value)}
              placeholder={t("modelcfg.searchAddedPh")}
              className="w-full h-9 rounded-xl bg-white/5 border border-white/10 pl-8 pr-7 text-xs text-white placeholder-slate-500 focus:outline-none focus:border-[var(--module-accent)]"
            />
            {providerSearch && (
              <button
                type="button"
                onClick={() => setProviderSearch("")}
                title={t("modelcfg.searchClear")}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300 cursor-pointer"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
        )}
      </div>

      {/* Provider List（紧凑行：点击行打开详情弹窗） */}
      {config?.providers.length === 0 ? (
        <div className="h-64 border border-dashed border-white/5 rounded-2xl flex flex-col items-center justify-center text-slate-500">
          <Key className="w-8 h-8 text-slate-700 mb-2" />
          <span className="text-xs font-bold text-slate-400">{t("modelcfg.noProviders")}</span>
        </div>
      ) : filteredProviders.length === 0 ? (
        // 搜索没命中：与预设弹窗的空态同一套视觉
        <div className="h-32 border border-dashed border-white/5 rounded-2xl flex flex-col items-center justify-center text-slate-600">
          <Search className="w-6 h-6 mb-2" />
          <span className="text-[10px] font-bold">{t("modelcfg.searchNoMatch")}</span>
        </div>
      ) : (
        <div className="rounded-xl border border-white/5 overflow-hidden divide-y divide-white/[0.04]">
          {filteredProviders.map((provider) => {
            const hasEndpoint = !!(provider.openai_url || provider.anthropic_url || provider.google_url);
            return (
              <div key={provider.id}>
                <div
                  className="flex items-center gap-2 px-2.5 py-1.5 bg-slate-900/20 hover:bg-white/[0.04] cursor-pointer transition-all"
                  onClick={() => openDetail(provider)}
                >
                  <span className="text-[11px] font-bold text-white truncate max-w-[160px]">{provider.name}</span>
                  {provider.website && (
                    <a href={provider.website} target="_blank" rel="noopener noreferrer"
                      onClick={(e) => { e.preventDefault(); e.stopPropagation(); void openUrl(provider.website); }}
                      className="text-blue-400/70 hover:text-blue-300 transition-colors flex-shrink-0" title={t("modelcfg.openSite")}>
                      <ExternalLink className="w-3 h-3" />
                    </a>
                  )}
                  <span className={`px-1.5 py-0.5 rounded text-[8px] font-bold flex-shrink-0 ${provider.category === "relay" ? "bg-cyan-500/15 text-cyan-400" : provider.category === "local" ? "bg-purple-500/15 text-purple-400" : "bg-emerald-500/15 text-emerald-400"}`}>
                    {provider.category === "relay" ? t("modelcfg.relay") : provider.category === "local" ? t("modelcfg.local") : t("modelcfg.vendor")}
                  </span>
                  {provider.openai_url && <span className="px-1 py-0.5 rounded text-[8px] font-bold bg-blue-500/15 text-blue-300/80 flex-shrink-0">OA</span>}
                  {provider.anthropic_url && <span className="px-1 py-0.5 rounded text-[8px] font-bold bg-amber-500/15 text-amber-300/80 flex-shrink-0">ANT</span>}
                  {provider.google_url && <span className="px-1 py-0.5 rounded text-[8px] font-bold bg-green-500/15 text-green-300/80 flex-shrink-0">GG</span>}
                  <span className="ml-auto text-[9px] text-slate-500 flex-shrink-0">{t("modelcfg.modelCount", { count: provider.models.length })}</span>
                  {BALANCE_CAPABLE.has(provider.id) && (
                    <button onClick={(e) => { e.stopPropagation(); openDetail(provider, true); }}
                      className="p-1 rounded-md text-slate-600 hover:text-emerald-400 hover:bg-emerald-500/10 cursor-pointer transition-all"
                      title={t("modelcfg.balanceQuery")}>
                      <Wallet className="w-3.5 h-3.5" />
                    </button>
                  )}
                  <button onClick={(e) => { e.stopPropagation(); handleTest(provider); }} disabled={testing === provider.id || !provider.api_key || !hasEndpoint}
                    className="p-1 rounded-md text-slate-600 hover:text-yellow-400 hover:bg-yellow-500/10 disabled:opacity-40 cursor-pointer transition-all"
                    title={t("modelcfg.testOk")}>
                    <Zap className={`w-3.5 h-3.5 ${testing === provider.id ? "animate-pulse text-yellow-400" : ""}`} />
                  </button>
                  <button onClick={(e) => { e.stopPropagation(); openEditModal(provider); }}
                    className="p-1 rounded-md text-slate-600 hover:text-blue-400 hover:bg-blue-500/10 cursor-pointer transition-all" title={t("modelcfg.edit")}>
                    <Settings2 className="w-3.5 h-3.5" />
                  </button>
                  <button onClick={(e) => { e.stopPropagation(); setDeleteTarget(provider.id); }}
                    className="p-1 rounded-md text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer transition-all" title={t("modelcfg.delete")}>
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
                {/* 测速结果：单行内联，省空间 */}
                {testResult?.id === provider.id && (
                  <div className={`px-2.5 pb-1.5 text-[9px] flex items-center gap-1 ${testResult.ok ? "text-emerald-400" : "text-red-400"}`}
                    title={testResult.msg}>
                    {testResult.ok ? <CheckCircle className="w-3 h-3 flex-shrink-0" /> : <AlertTriangle className="w-3 h-3 flex-shrink-0" />}
                    <span className="truncate">{testResult.ok ? t("modelcfg.testOk") : `${t("modelcfg.testFail")} — ${testResult.msg}`}</span>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* ─── 预设选择弹窗（关键词 + 分类过滤） ─── */}
      {showPresetPicker && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 modal-mask flex items-center justify-center p-4" onClick={() => setShowPresetPicker(false)}>
          <div className="w-full max-w-xl bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl flex flex-col max-h-[85vh] overflow-hidden" onClick={e => e.stopPropagation()}>
            {/* Header */}
            <div className="p-4 pb-3 border-b border-white/5 space-y-3">
              <div className="flex items-center justify-between">
                <h3 className="text-xs font-bold text-slate-200">{t("modelcfg.pickerTitle")}</h3>
                <button onClick={() => setShowPresetPicker(false)} className="text-slate-500 hover:text-slate-300 cursor-pointer"><X className="w-4 h-4" /></button>
              </div>
              {/* 关键词搜索 */}
              <div className="relative">
                <Search className="w-3.5 h-3.5 text-slate-500 absolute left-2.5 top-1/2 -translate-y-1/2 pointer-events-none" />
                <input autoFocus value={presetSearch} onChange={e => setPresetSearch(e.target.value)}
                  placeholder={t("modelcfg.pickerSearchPh")}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg pl-8 pr-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
              </div>
              {/* 分类过滤 */}
              <div className="flex items-center gap-1.5 flex-wrap">
                {([["all", t("modelcfg.filterAll")], ["provider", t("modelcfg.filterProvider")], ["relay", t("modelcfg.filterRelay")], ["local", t("modelcfg.filterLocal")]] as const).map(([key, label]) => (
                  <button key={key} type="button" onClick={() => setPresetCategory(key)}
                    className={`px-2.5 py-1 rounded-full text-[10px] font-semibold border transition-colors ${presetCategory === key ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white" : "bg-slate-900 border-white/10 text-slate-400 hover:border-white/20 hover:text-slate-200"}`}>
                    {label} <span className="opacity-60">{presetCategoryCounts[key]}</span>
                  </button>
                ))}
              </div>
            </div>

            {/* 预设网格 */}
            <div className="flex-grow overflow-y-auto p-3">
              {filteredPresets.length === 0 ? (
                <div className="h-32 flex flex-col items-center justify-center text-slate-600">
                  <Search className="w-6 h-6 mb-2" />
                  <span className="text-[10px]">{t("modelcfg.pickerNoMatch")}</span>
                </div>
              ) : (
                <div className="grid grid-cols-2 gap-2">
                  {filteredPresets.map((p) => {
                    const added = config?.providers.some(x => x.id === p.id);
                    const isLocal = p.category === "local";
                    const isRelay = p.category === "relay";
                    return (
                      <button key={p.id} onClick={() => openAddModal(p)} disabled={added}
                        className="text-left p-2.5 rounded-xl border border-white/5 bg-slate-900/40 hover:bg-white/5 hover:border-white/15 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer transition-all group">
                        <div className="flex items-center gap-2 min-w-0">
                          {isLocal ? <Laptop className="w-3.5 h-3.5 text-purple-400/70 flex-shrink-0" /> : isRelay ? <Server className="w-3.5 h-3.5 text-cyan-400/70 flex-shrink-0" /> : <Globe className="w-3.5 h-3.5 text-emerald-400/70 flex-shrink-0" />}
                          <span className="text-[11px] font-bold text-slate-200 truncate">{p.name}</span>
                          {added && <span className="ml-auto text-[8px] text-slate-600 flex-shrink-0">{t("modelcfg.added")}</span>}
                          {!added && p.website && (
                            <a href={p.website} target="_blank" rel="noopener noreferrer" onClick={(e) => { e.preventDefault(); e.stopPropagation(); void openUrl(p.website); }}
                              className="ml-auto text-slate-600 hover:text-blue-400 opacity-0 group-hover:opacity-100 transition-opacity"
                              title={t("modelcfg.openSite")}>
                              <ExternalLink className="w-3 h-3" />
                            </a>
                          )}
                        </div>
                        {/* 协议徽标 */}
                        <div className="flex items-center gap-1 mt-1.5 flex-wrap">
                          {p.openai_url && <span className="px-1.5 py-0.5 rounded text-[8px] font-bold bg-blue-500/20 text-blue-300">OpenAI</span>}
                          {p.anthropic_url && <span className="px-1.5 py-0.5 rounded text-[8px] font-bold bg-amber-500/20 text-amber-300">Anthropic</span>}
                          {p.google_url && <span className="px-1.5 py-0.5 rounded text-[8px] font-bold bg-green-500/20 text-green-300">Google</span>}
                          <span className="font-mono text-[8px] text-slate-600 truncate">{p.openai_url || p.anthropic_url || p.google_url}</span>
                        </div>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>

            {/* Footer：自定义入口 */}
            <div className="p-3 border-t border-white/5 bg-slate-900/20 flex justify-end gap-2">
              <button onClick={() => openAddModal()}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer flex items-center gap-1">
                <Plus className="w-3 h-3" />{t("modelcfg.customProvider")}
              </button>
              <button onClick={() => openAddModal({ id: "", name: "", category: "relay", website: "", openai_url: "", anthropic_url: "", google_url: "" })}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer flex items-center gap-1">
                <Plus className="w-3 h-3" />{t("modelcfg.customRelay")}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ─── 供应商详情弹窗 ─── */}
      {detailProvider && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 modal-mask flex items-center justify-center p-4" onClick={() => setDetailId(null)}>
          <div className="w-full max-w-md bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl flex flex-col max-h-[85vh] overflow-hidden" onClick={e => e.stopPropagation()}>
            {/* Header */}
            <div className="p-4 border-b border-white/5 flex items-center justify-between gap-2">
              <div className="flex items-center gap-2 min-w-0">
                <h3 className="text-xs font-bold text-slate-200 truncate">{detailProvider.name}</h3>
                <span className={`px-1.5 py-0.5 rounded text-[8px] font-bold flex-shrink-0 ${detailProvider.category === "relay" ? "bg-cyan-500/15 text-cyan-400" : detailProvider.category === "local" ? "bg-purple-500/15 text-purple-400" : "bg-emerald-500/15 text-emerald-400"}`}>
                  {detailProvider.category === "relay" ? t("modelcfg.relay") : detailProvider.category === "local" ? t("modelcfg.local") : t("modelcfg.vendor")}
                </span>
                {detailProvider.website && (
                  <a href={detailProvider.website} target="_blank" rel="noopener noreferrer"
                    onClick={(e) => { e.preventDefault(); void openUrl(detailProvider.website); }}
                    className="text-blue-400 hover:text-blue-300 transition-colors flex-shrink-0" title={t("modelcfg.openSite")}>
                    <ExternalLink className="w-3 h-3" />
                  </a>
                )}
              </div>
              <button onClick={() => setDetailId(null)} className="text-slate-500 hover:text-slate-300 cursor-pointer flex-shrink-0"><X className="w-4 h-4" /></button>
            </div>

            {/* Body */}
            <div className="flex-grow overflow-y-auto p-4 space-y-4">
              {/* 余额 */}
              {BALANCE_CAPABLE.has(detailProvider.id) && (
                <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-2">
                  <div className="flex items-center justify-between">
                    <label className="text-[10px] text-slate-400 font-semibold">{t("modelcfg.balance")}</label>
                    <button onClick={() => void runBalance(detailProvider)} disabled={balanceState?.pid === detailProvider.id && balanceState.loading}
                      className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-[9px] font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-40 disabled:cursor-not-allowed">
                      <RefreshCw className={`w-3 h-3 ${balanceState?.pid === detailProvider.id && balanceState.loading ? "animate-spin" : ""}`} />
                      {t("modelcfg.balanceQuery")}
                    </button>
                  </div>
                  {balanceState?.pid === detailProvider.id && balanceState.loading && (
                    <p className="text-[10px] text-slate-500">{t("modelcfg.loading")}</p>
                  )}
                  {balanceState?.pid === detailProvider.id && balanceState.error && (
                    <p className="text-[10px] text-red-400 whitespace-pre-line">{balanceState.error}</p>
                  )}
                  {balanceState?.pid === detailProvider.id && !balanceState.loading && !balanceState.error && (
                    <div className="space-y-1">
                      {balanceState.items.map((it) => (
                        <div key={it.key} className="flex items-center justify-between text-[10px]">
                          <span className="text-slate-500">{balanceLabel(it.key)}</span>
                          <span className="font-mono text-slate-200">{it.value === "ok" ? t("modelcfg.balOk") : it.value === "disabled" ? t("modelcfg.balDisabled") : it.value}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {/* 协议端点 */}
              <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-1.5">
                <label className="text-[10px] text-slate-400 font-semibold">{t("modelcfg.endpoints")}</label>
                {([
                  [t("modelcfg.openaiUrl"), detailProvider.openai_url, "text-blue-300", detailProvider.openai_include_v1 ?? null],
                  [t("modelcfg.anthropicUrl"), detailProvider.anthropic_url, "text-amber-300", detailProvider.anthropic_include_v1 ?? null],
                  [t("modelcfg.googleUrl"), detailProvider.google_url, "text-green-300", null],
                ] as const).map(([label, url, cls, includeV1]) => url ? (
                  <div key={label} className="flex items-start gap-2 text-[10px]">
                    <span className={`${cls} font-semibold flex-shrink-0 w-24`}>{label}</span>
                    <span className="font-mono text-slate-400 break-all">{url}</span>
                    {/* 只有显式改过才标出来：「自动」是默认行为，写出来反而像配置项丢了 */}
                    {includeV1 !== null && (
                      <span className="flex-shrink-0 px-1 py-px rounded bg-white/5 text-[8px] text-slate-400">
                        /v1 {includeV1 ? t("modelcfg.v1Yes") : t("modelcfg.v1No")}
                      </span>
                    )}
                  </div>
                ) : null)}
              </div>

              {/* 模型列表 */}
              <div>
                <label className="text-[10px] text-slate-500 font-semibold block mb-1.5">
                  {t("modelcfg.modelList", { count: detailProvider.models.length })}
                </label>
                {detailProvider.models.length === 0 ? (
                  <div className="text-[10px] text-slate-600 py-2 text-center">{t("modelcfg.noModelsHint")}</div>
                ) : (
                  <div className="max-h-48 overflow-y-auto rounded-lg border border-white/5 divide-y divide-white/[0.03]">
                    {detailProvider.models.map((model) => (
                      <div key={model.id} className="px-2.5 py-1 text-[10px] bg-white/[0.02]">
                        <span className="font-mono text-slate-300">{model.id}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>

            {/* Footer */}
            <div className="p-4 border-t border-white/5 bg-slate-900/20 flex justify-end gap-2">
              <button onClick={() => { setDetailId(null); openEditModal(detailProvider); }}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-300 hover:text-white text-[10px] font-semibold cursor-pointer">{t("modelcfg.edit")}</button>
              <button onClick={() => setDetailId(null)}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer">{t("modelcfg.cancel")}</button>
            </div>
          </div>
        </div>
      )}

      {/* ─── 编辑/新增弹框 ─── */}
      {showModal && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 modal-mask flex items-center justify-center p-4">
          <div className="w-full max-w-lg bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl flex flex-col max-h-[85vh] overflow-hidden" onClick={e => e.stopPropagation()}>
            {/* Header */}
            <div className="p-4 border-b border-white/5 flex items-center justify-between">
              <h3 className="text-xs font-bold text-slate-200">{modalMode === "add" ? t("modelcfg.modalAdd") : t("modelcfg.modalEdit")}</h3>
              <button onClick={() => setShowModal(false)} className="text-slate-500 hover:text-slate-300 cursor-pointer"><X className="w-4 h-4" /></button>
            </div>

            {/* Body */}
            <div className="flex-grow overflow-y-auto p-4 space-y-4">
              {/* Name */}
              <div>
                <label className="text-[10px] text-slate-500 font-semibold block mb-1">{t("modelcfg.name")}</label>
                <input value={form.name} onChange={e => setForm({ ...form, name: e.target.value })}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
              </div>

              {/* Website */}
              <div>
                <label className="text-[10px] text-slate-500 font-semibold block mb-1">{t("modelcfg.website")}</label>
                <input value={form.website} onChange={e => setForm({ ...form, website: e.target.value })} placeholder="https://..."
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-blue-500" />
              </div>

              {/* API Key */}
              <div>
                <label className="text-[10px] text-slate-500 font-semibold block mb-1">API Key</label>
                <div className="relative">
                  <input type={showApiKey ? "text" : "password"} value={form.api_key} onChange={e => setForm({ ...form, api_key: e.target.value })} placeholder="sk-..."
                    className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 pr-9 text-xs text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                  <button
                    type="button"
                    onClick={() => setShowApiKey(v => !v)}
                    disabled={!form.api_key}
                    title={showApiKey ? t("modelcfg.hideKey") : t("modelcfg.showKey")}
                    className="absolute right-1.5 top-1/2 -translate-y-1/2 p-1 rounded-md text-slate-500 hover:text-slate-200 disabled:opacity-30 disabled:cursor-not-allowed cursor-pointer transition-all"
                  >
                    {showApiKey ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
                  </button>
                </div>
              </div>

              {/* 自定义上游请求头 */}
              <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-[10px] text-slate-400 font-semibold">{t("modelcfg.customHeaders")}</label>
                  <button
                    type="button"
                    onClick={addCustomHeader}
                    className="flex items-center gap-1 text-[10px] text-slate-400 hover:text-slate-200 cursor-pointer transition-all"
                  >
                    <Plus className="w-3 h-3" /> {t("modelcfg.addHeader")}
                  </button>
                </div>
                <p className="text-[9px] text-slate-600">{t("modelcfg.customHeadersHint")}</p>
                {form.custom_headers.length === 0 ? (
                  <p className="text-[10px] text-slate-600">{t("modelcfg.noCustomHeaders")}</p>
                ) : (
                  <div className="space-y-1.5">
                    {form.custom_headers.map((h, idx) => (
                      <div key={idx} className="flex items-center gap-1.5">
                        <input
                          value={h.key}
                          onChange={e => updateCustomHeader(idx, { key: e.target.value })}
                          placeholder={t("modelcfg.headerName")}
                          className="flex-1 min-w-0 bg-slate-900 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
                        />
                        <input
                          value={h.value}
                          onChange={e => updateCustomHeader(idx, { value: e.target.value })}
                          placeholder={t("modelcfg.headerValue")}
                          className="flex-1 min-w-0 bg-slate-900 border border-white/10 rounded-lg px-2 py-1 text-[11px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
                        />
                        <button
                          type="button"
                          onClick={() => removeCustomHeader(idx)}
                          title={t("modelcfg.removeHeader")}
                          className="p-1 rounded-md text-slate-500 hover:text-red-400 cursor-pointer transition-all"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* 协议端点 URL（每个支持的协议一个地址） */}
              <div className="p-3 rounded-lg bg-slate-900/50 border border-white/5 space-y-3">
                <label className="text-[10px] text-slate-400 font-semibold block">{t("modelcfg.endpoints")}</label>
                <p className="text-[9px] text-slate-600">{t("modelcfg.endpointsHint")}</p>

                <div className="space-y-1">
                  <label className="text-[9px] text-blue-300 font-semibold block">{t("modelcfg.openaiUrl")}</label>
                  <input value={form.openai_url} onChange={e => setForm({ ...form, openai_url: e.target.value })}
                    placeholder="https://api.openai.com/v1"
                    className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-blue-500" />
                  {/* 兼容层差异：有的端点要 `{base}/v1/chat/completions`，有的是 `{base}/chat/completions` */}
                  <div className="flex items-center gap-1.5">
                    <span className="text-[9px] text-slate-500">{t("modelcfg.includeV1")}</span>
                    <select value={v1ToSelect(form.openai_include_v1)}
                      onChange={e => setForm({ ...form, openai_include_v1: selectToV1(e.target.value) })}
                      className="bg-slate-900 border border-white/10 rounded px-1.5 py-0.5 text-[9px] text-slate-300 cursor-pointer focus:outline-none focus:border-blue-500"
                      title={t("modelcfg.includeV1Hint")}>
                      <option value="auto">{t("modelcfg.v1Auto")}</option>
                      <option value="yes">{t("modelcfg.v1Yes")}</option>
                      <option value="no">{t("modelcfg.v1No")}</option>
                    </select>
                  </div>
                </div>

                <div className="space-y-1">
                  <label className="text-[9px] text-amber-300 font-semibold block">{t("modelcfg.anthropicUrl")}</label>
                  <input value={form.anthropic_url} onChange={e => setForm({ ...form, anthropic_url: e.target.value })}
                    placeholder="https://api.anthropic.com"
                    className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-amber-500" />
                  <div className="flex items-center gap-1.5">
                    <span className="text-[9px] text-slate-500">{t("modelcfg.includeV1")}</span>
                    <select value={v1ToSelect(form.anthropic_include_v1)}
                      onChange={e => setForm({ ...form, anthropic_include_v1: selectToV1(e.target.value) })}
                      className="bg-slate-900 border border-white/10 rounded px-1.5 py-0.5 text-[9px] text-slate-300 cursor-pointer focus:outline-none focus:border-amber-500"
                      title={t("modelcfg.includeV1Hint")}>
                      <option value="auto">{t("modelcfg.v1Auto")}</option>
                      <option value="yes">{t("modelcfg.v1Yes")}</option>
                      <option value="no">{t("modelcfg.v1No")}</option>
                    </select>
                  </div>
                </div>

                <div className="space-y-1">
                  <label className="text-[9px] text-green-300 font-semibold block">{t("modelcfg.googleUrl")}</label>
                  <input value={form.google_url} onChange={e => setForm({ ...form, google_url: e.target.value })}
                    placeholder="https://generativelanguage.googleapis.com"
                    className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-1.5 text-xs text-slate-200 font-mono focus:outline-none focus:border-green-500" />
                </div>
              </div>

              {/* 模型列表 */}
              <div>
                <div className="flex items-center justify-between mb-1">
                  <label className="text-[10px] text-slate-500 font-semibold">
                    {t("modelcfg.modelListLabel")} <span className="text-slate-600">{t("modelcfg.onePerLine")}</span>
                  </label>
                  <button
                    onClick={handleFetchModels}
                    disabled={fetchingModels || (!form.openai_url && !form.anthropic_url && !form.google_url) || !form.api_key}
                    className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-[9px] font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-40 disabled:cursor-not-allowed"
                  >
                    <RefreshCw className={`w-3 h-3 ${fetchingModels ? "animate-spin" : ""}`} />
                    {fetchingModels ? t("modelcfg.fetching") : t("modelcfg.autoFetch")}
                  </button>
                </div>
                <textarea
                  value={modelsText}
                  onChange={e => setModelsText(e.target.value)}
                  rows={6}
                  placeholder={"gpt-4o\ngpt-4o-mini\nclaude-sonnet-4-20250514\ndeepseek-chat\ndeepseek-v4-pro"}
                  className="w-full bg-slate-900 border border-white/10 rounded-lg px-2.5 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)] resize-y leading-5"
                />
                <div className="text-[9px] text-slate-600 mt-1">
                  {t("modelcfg.enteredModels", { count: modelsText.split("\n").filter(l => l.trim()).length })}
                </div>
              </div>

              {/* 模型自定义启动参数 */}
              {modelsText.split("\n").map(l => l.trim()).filter(Boolean).length > 0 && (
                <div className="rounded-lg border border-white/5 bg-slate-900/30 p-3 space-y-3">
                  <div className="text-[10px] text-slate-500 font-semibold">
                    {t("modelcfg.customParams")}
                    <span className="text-slate-600 font-normal">{t("modelcfg.customParamsHint")}</span>
                  </div>
                  {modelsText.split("\n").map(l => l.trim()).filter(Boolean).map((mid) => (
                    <div key={mid} className="rounded-md border border-white/5 bg-slate-900/40 p-2.5">
                      <div className="text-[10px] text-[var(--module-accent)] font-mono mb-2">{mid}</div>
                      {(modelParams[mid] || []).map((cp, ci) => (
                        <div key={ci} className="mb-2 p-2 rounded bg-slate-800/40 border border-white/5 space-y-1.5">
                          <div className="flex gap-1.5">
                            <input value={cp.label} onChange={e => updateModelParam(mid, ci, { label: e.target.value })}
                              placeholder={t("modelcfg.paramNamePh")} className="w-37 min-w-0 bg-slate-900 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                            <input value={cp.key} onChange={e => updateModelParam(mid, ci, { key: e.target.value })}
                              placeholder={t("modelcfg.paramKeyPh")} className="flex-1 min-w-0 bg-slate-900 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                            <button onClick={() => removeModelParam(mid, ci)}
                              className="shrink-0 w-6 h-6 flex items-center justify-center rounded bg-red-500/10 hover:bg-red-500/20 text-[11px] text-red-400">×</button>
                          </div>
                          <div className="flex gap-1.5 items-stretch">
                            <div className="flex items-center gap-1.5 shrink-0 rounded-md border border-cyan-500/20 bg-cyan-500/5 px-2 py-1">
                              <div className="flex items-center gap-1 mr-2">
                                {([["enum", t("modelcfg.paramEnum")],["text", t("modelcfg.paramText")],["bool", t("modelcfg.paramBool")]] as const).map(([v,l]) => (
                                  <button key={v} type="button" onClick={() => updateModelParam(mid, ci, { paramType: v })}
                                    className={`px-2 py-0.5 rounded-full text-[10px] border transition-colors ${cp.paramType === v ? "bg-cyan-500/20 border-cyan-500 text-cyan-200" : "bg-slate-900 border-white/10 text-slate-400 hover:border-white/20"}`}>
                                    {l}
                                  </button>
                                ))}
                              </div>
                            </div>
                            {cp.paramType === "enum" && (
                              <input value={(cp.options || []).join(",")} onChange={e => updateModelParam(mid, ci, { options: e.target.value.split(",").map(s => s.trim()).filter(Boolean) })}
                                placeholder={t("modelcfg.paramValuesPh")} className="flex-1 min-w-0 bg-slate-900 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                            )}
                            <input value={cp.defaultValue || ""} onChange={e => updateModelParam(mid, ci, { defaultValue: e.target.value })}
                              placeholder={t("modelcfg.paramDefaultPh")} className="w-24 bg-slate-900 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                          </div>
                          <div className="flex gap-1.5 items-stretch">
                            <div className="flex items-center gap-1.5 shrink-0 rounded-md border border-amber-500/20 bg-amber-500/5 px-2 py-1">
                              <div className="flex items-center gap-1">
                                {([["env", t("modelcfg.paramEnv")],["config", t("modelcfg.paramConfig")]] as const).map(([v,l]) => (
                                  <button key={v} type="button" onClick={() => updateModelParam(mid, ci, { target: v })}
                                    className={`px-2 py-0.5 rounded-full text-[10px] border transition-colors ${cp.target === v ? "bg-amber-500/20 border-amber-500 text-amber-200" : "bg-slate-900 border-white/10 text-slate-400 hover:border-white/20"}`}>
                                    {l}
                                  </button>
                                ))}
                              </div>
                            </div>
                            <input value={cp.target === "config" ? (cp.configPath || "") : (cp.envKey || "")}
                              onChange={e => cp.target === "config"
                                ? updateModelParam(mid, ci, { configPath: e.target.value })
                                : updateModelParam(mid, ci, { envKey: e.target.value })}
                              placeholder={cp.target === "config" ? t("modelcfg.paramTargetPh") : t("modelcfg.paramEnvPh")}
                              className="flex-1 min-w-0 bg-slate-900 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                          </div>
                        </div>
                      ))}
                      <button onClick={() => addModelParam(mid)}
                        className="text-[10px] text-[var(--module-accent)] hover:text-[var(--module-accent-strong)] cursor-pointer">{t("modelcfg.addParam")}</button>
                    </div>
                  ))}
                </div>
              )}

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
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer">{t("modelcfg.cancel")}</button>
              <button onClick={handleModalConfirm}
                className="px-3.5 py-1.5 rounded-lg bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white text-[10px] font-semibold cursor-pointer">{t("modelcfg.confirm")}</button>
            </div>
          </div>
        </div>
      )}

      {/* ─── 删除确认弹框 ─── */}
      {deleteTarget && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 modal-mask flex items-center justify-center p-4">
          <div className="w-full max-w-sm bg-slate-950/95 border border-white/10 rounded-2xl shadow-2xl p-5" onClick={e => e.stopPropagation()}>
            <div className="flex items-center gap-3 mb-4">
              <div className="p-2 rounded-lg bg-red-500/10"><Trash2 className="w-4 h-4 text-red-400" /></div>
              <div>
                <h3 className="text-xs font-bold text-slate-200">{t("modelcfg.deleteTitle")}</h3>
                <p className="text-[10px] text-slate-500 mt-0.5">{t("modelcfg.deleteHint", { name: config?.providers.find(p => p.id === deleteTarget)?.name ?? "" })}</p>
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <button onClick={() => setDeleteTarget(null)}
                className="px-3 py-1.5 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-[10px] font-semibold cursor-pointer">{t("modelcfg.cancel")}</button>
              <button onClick={() => handleDelete(deleteTarget)}
                className="px-3.5 py-1.5 rounded-lg bg-red-600 hover:bg-red-500 text-white text-[10px] font-semibold cursor-pointer">{t("modelcfg.deleteBtn")}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
