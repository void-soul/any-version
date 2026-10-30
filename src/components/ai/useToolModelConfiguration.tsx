import React, { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Note } from "../shared/Note";
import { ToggleLeft, ToggleRight, Cpu } from "lucide-react";
import type { AiConfig, DetectedAiTool, ModelCustomParam, ModelEntry } from "../../contracts/ai";
import { ModelSelector, CustomParamControls } from "../shared/ai/ModelSelector";
import { isStrictMasqueradeTool } from "./ToolLauncher.model";
import { CollapsibleCard } from "./ToolLauncher.CollapsibleCard";

/** Owns the ToolModelConfiguration feature, independently of page rendering. */
export function useToolModelConfiguration({ config, selectedTool }: {
  config: AiConfig | null;
  selectedTool: DetectedAiTool | null;
}) {
  const { t } = useTranslation();

  const [customParamValues, setCustomParamValues] = useState<Record<string, string>>({});

  const [vendorOpen, setVendorOpen] = useState(true);

  const [fallbackOpen, setFallbackOpen] = useState(false);

  const [optimizerOpen, setOptimizerOpen] = useState(false);

  const [selectedModel, setSelectedModel] = useState("");

  const [selectedModelProvider, setSelectedModelProvider] = useState("");

  // 后端解析出的**实际生效**声明名（Rust 的 effective_claimed_model）。
  // Claude Desktop 留空伪装名时后端会自动给合法别名，前端必须显示那个而不是空 ——
  // 别名规则只在 Rust 一处实现，前端复制就会漂移。取不到时退回手填值（旧行为）。
  const [effectiveAlias, setEffectiveAlias] = useState("");

  const [oneMContext, setOneMContext] = useState(false);

  // fallback 模型是否同样追加 [1m]（可与主模型独立勾选）
  const [fallbackOneMContext, setFallbackOneMContext] = useState(false);

  // 伪装模型名（"" 表示不伪装，直接使用所选取的供应商模型）
  const [masqueradeModel, setMasqueradeModel] = useState("");

  // 代理增强能力开关（由工具能力 + 全局配置共同决定是否实际生效）
  const [optimizerEnabled, setOptimizerEnabled] = useState(true);

  const [rectifierEnabled, setRectifierEnabled] = useState(true);

  // 整流器 / 优化器各策略（默认沿用全局配置 AiConfig.rectifier / optimizer）
  const [rectifierStrategies, setRectifierStrategies] = useState({
    thinking_signature: true, thinking_budget: true, media_fallback: true,
    media_heuristic: true, protocol_mismatch: true,
    // 工具调用方言优化：上游不把 tools 翻回标准 tool_calls 时，模型会把自己的写法当正文吐回来，
    // 客户端看不到 function_call → 判定本轮结束 → 「说了段计划就停住」。详见 proxy/xml_tools.rs。
    toolcall_dialect: true,
  });

  const [optimizerStrategies, setOptimizerStrategies] = useState({
    cache_injection: true, thinking_optimizer: true, deepseek_normalize: true,
  });

  // Codex web_search 开关：开启 → 写 config.toml `web_search = "live"`（真实实时检索）
  const [webSearchEnabled, setWebSearchEnabled] = useState(false);

  // 工具配置文件里**当前写定**的模型：切换工具时回读，用来把上次的选择回显出来。
  // 不回显会出现「界面显示没选模型（看着像在用官方配置），实际工具还在用我们写进去的
  // 自定义模型」—— 用户看到的和生效的完全是两回事。
  const [configAppliedModel, setConfigAppliedModel] = useState<string | null>(null);

  const [configAppliedFallback, setConfigAppliedFallback] = useState<string | null>(null);

  const [rectifierOpen, setRectifierOpen] = useState(false);

  // 双模型（高级 + fallback 低级）
  const [selectedFallbackModel, setSelectedFallbackModel] = useState("");

  const [selectedFallbackProvider, setSelectedFallbackProvider] = useState("");

  // fallback 模型的伪装声明名（"" 表示不伪装，直接使用所选取的供应商模型）
  const [fallbackMasqueradeModel, setFallbackMasqueradeModel] = useState("");

  // 官方模型选择（对于 api_protocol="none" 或用户主动选择官方模型）
  const [useOfficialModel, setUseOfficialModel] = useState(false);

  // 当前选中模型的自定义启动参数模板（来自该模型定义）
  const currentModelCustomParams = React.useMemo<ModelCustomParam[]>(() => {
    if (!selectedModelProvider || !selectedModel) return [];
    return config?.providers
      .find(p => p.id === selectedModelProvider)?.models
      .find(m => m.id === selectedModel)?.customParams || [];
  }, [config, selectedModelProvider, selectedModel]);

  // 切换模型时，将自定义参数取值重置为该模型的默认值
  const resetCustomParamValues = React.useCallback((params: ModelCustomParam[]) => {
    const defs: Record<string, string> = {};
    for (const cp of params) if (cp.defaultValue) defs[cp.key] = cp.defaultValue;
    setCustomParamValues(defs);
  }, []);

  // 底部「伪装映射」要显示**实际生效**的声明名，而它由后端规则决定（Claude Desktop
  // 留空时会给一个合法别名）。所以问后端要，不在前端复刻那份黑名单。
  // 取不到就退回手填值 —— 展示性功能不能把面板搞挂。
  useEffect(() => {
    let cancelled = false;
    const toolId = selectedTool?.id ?? "";
    const fallback = masqueradeModel || "";
    if (!toolId) { setEffectiveAlias(""); return; }
    // invoke 在没有后端时是**同步抛错**（读 window.__TAURI__），`.catch()` 抓不到 ——
    // 所以整个调用包进 async IIFE + try/catch，兑现「取不到就退回手填值」。
    (async () => {
      try {
        const v = await invoke<string>("resolve_claimed_model", {
          toolId,
          realModel: selectedModel || "",
          preferred: masqueradeModel || "",
        });
        if (!cancelled) setEffectiveAlias(v ?? fallback);
      } catch {
        if (!cancelled) setEffectiveAlias(fallback);
      }
    })();

  return () => { cancelled = true; };
  }, [selectedTool?.id, selectedModel, masqueradeModel]);

  // ── 模型供应商（统一列表）──
  // 新设计下代理会自动做协议转换，因此 ANY 提供模型列表的供应商都可选；
  // 协议差异由代理的入站/出站转换负责。这里合并为单一列表（按供应商分组）。
  const eligibleProviders = React.useMemo(() => {
    if (!config || !selectedTool) return [];
    if (!selectedTool.supports_model) return [];
    const groups: { provider_name: string; provider_id: string; models: ModelEntry[] }[] = [];
    for (const p of config.providers) {
      if (p.models.length === 0) continue;
      groups.push({ provider_name: p.name, provider_id: p.id, models: p.models });
    }
    return groups;
  }, [config, selectedTool]);

  // Fallback 可选供应商：排除已选的主模型（副模型不能与主模型重复），供 ModelSelector 用
  const fallbackProviders = React.useMemo(() => {
    if (!config) return [];
    return config.providers
      .map((p) => ({
        ...p,
        models: selectedModel ? p.models.filter((m) => m.id !== selectedModel) : p.models,
      }))
      .filter((p) => p.models.length > 0);
  }, [config, selectedModel]);

  // 本次启动**实际会用第三方模型**：没勾「使用官方模型」**且**确实选了一个模型。
  //
  // 只勾掉官方开关、但一个模型都没选时，后端拿到的 `model_id` 是 null，
  // 走的仍然是工具的官方配置 —— 此时优化器 / 整流器没有任何作用，不该显示。
  const usingThirdPartyModel = !useOfficialModel && !!selectedModel;

  const renderModelOptions = () => selectedTool && (<>
                {/* 官方模型开关（适用于有独立 API key 的工具） */}
                {selectedTool.api_protocol !== "none" && selectedTool.supports_model && (
                  <div className="flex items-center justify-between p-2.5 rounded-ctl bg-blue-500/5 border border-blue-500/10">
                    <div className="flex items-center gap-2">
                      <Cpu className="w-3.5 h-3.5 text-blue-400" />
                      <div>
                        <span className="text-tiny font-semibold text-blue-300">{t("toollaunch.useOfficial")}</span>
                        <p className="text-[8px] text-slate-500 mt-0.5">{t("toollaunch.useOfficialHint")}</p>
                      </div>
                    </div>
                    <button
                      onClick={() => setUseOfficialModel(!useOfficialModel)}
                      className={`p-1 rounded-md cursor-pointer transition-all ${useOfficialModel ? "text-blue-400" : "text-slate-600 hover:text-slate-400"}`}
                      title={useOfficialModel ? t("toollaunch.useOfficialTitle") : t("toollaunch.useKiraModel")}
                    >
                      {useOfficialModel ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                    </button>
                  </div>
                )}

                {/* ─── 模型选择 ─── */}
                {selectedTool.supports_model && !useOfficialModel && (
                  <div>
                    {/* 配置文件里有模型、但仓库里查不到（比如写了伪装名，或那个供应商/模型
                        已被删）：明说它还在生效，别静默显示成「未选择」—— 那会让人以为
                        工具在用官方配置。 */}
                    {configAppliedModel && !selectedModel && (
                      <Note tone="warn">
                        {t("toollaunch.configModelFromFile", { model: configAppliedModel })}
                      </Note>
                    )}
                    {configAppliedFallback && !selectedFallbackModel && (
                      <Note tone="warn">
                        {t("toollaunch.configFallbackFromFile", { model: configAppliedFallback })}
                      </Note>
                    )}
                    {/* 有上次记录、但配置文件里的值与面板选择不一致：多半是伪装名（声明名）。
                        说清楚「谁在生效」，别让面板与工具实际状态悄悄分叉。 */}
                    {configAppliedModel && selectedModel && configAppliedModel !== selectedModel && (
                      <Note tone="warn">
                        {t("toollaunch.configModelDiffers", { model: configAppliedModel, selected: selectedModel })}
                      </Note>
                    )}

                    {/* 模型供应商 — 统一列表（代理自动转换协议，任意供应商可选） */}
                    {eligibleProviders.length > 0 && (
                      <CollapsibleCard
                        title={t("toollaunch.modelVendor")}
                        open={vendorOpen}
                        onToggle={() => setVendorOpen(!vendorOpen)}
                        summary={selectedModel
                          ? `${selectedModel}（${config?.providers.find(p => p.id === selectedModelProvider)?.name ?? "—"}）`
                          : t("toollaunch.noModelSelected")}
                      >
                        <ModelSelector
                          providers={config?.providers ?? []}
                          providerId={selectedModelProvider}
                          modelId={selectedModel}
                          onSelect={(pid, mid) => {
                            if (!mid) {
                              setSelectedModel("");
                              setSelectedModelProvider("");
                              resetCustomParamValues([]);
                            } else {
                              setSelectedModel(mid);
                              setSelectedModelProvider(pid);
                              const m = config?.providers.find(p => p.id === pid)?.models.find(x => x.id === mid);
                              resetCustomParamValues(m?.customParams || []);
                            }
                          }}
                          allowClear
                        />
                        {selectedModel && (
                          <div className="mt-1 text-tiny text-[var(--module-accent)]">{t("toollaunch.selected")}<span className="font-mono">{selectedModel}</span> <span className="text-slate-500">（{config?.providers.find(p => p.id === selectedModelProvider)?.name}）</span></div>
                        )}

                        {/* 模型自定义启动参数（用户定义，运行时渲染为控件） */}
                        {currentModelCustomParams.length > 0 && (
                          <div className="mt-3 space-y-2">
                            <div className="text-tiny text-slate-500 font-semibold">{t("toollaunch.customParams")}</div>
                            <CustomParamControls
                              params={currentModelCustomParams}
                              values={customParamValues}
                              onChange={(key, value) => setCustomParamValues(prev => ({ ...prev, [key]: value }))}
                            />
                          </div>
                        )}
                      </CollapsibleCard>
                    )}

                    {/* 没有可用的供应商/模型时的警告 */}
                    {eligibleProviders.length === 0 && (
                      <Note tone="warn">{t("toollaunch.noModelsWarn")}</Note>
                    )}

                    {/* 模型伪装（仅当工具内置模型名列表非空） */}
                    {selectedModel && selectedTool.builtin_models.length > 0 && (
                      <div className="mt-3">
                        <label className="text-body font-bold text-slate-300 mb-1.5 block">{t("toollaunch.masqueradeLabel")} <span className="text-micro text-slate-500 font-normal">{t("toollaunch.optional")}</span></label>
                        <p className="text-micro text-slate-500 mb-1.5">
                          {/* 桌面端的声明名会被后端按「官方名校验」过滤，填非法名是无效的 ——
                              提示语必须说实话，不能沿用「任意模型名」那句。 */}
                          {t(isStrictMasqueradeTool(selectedTool.id) ? "toollaunch.masqueradeHintStrict" : "toollaunch.masqueradeHint", { model: selectedModel })}
                        </p>
                        <input type="text" list={`masq-list-${selectedTool.id}`} value={masqueradeModel}
                          onChange={e => setMasqueradeModel(e.target.value)}
                          placeholder={t("toollaunch.noMasqueradePh", { model: selectedModel })}
                          className="w-full ui-input rounded-ctl px-3 py-2 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                        <datalist id={`masq-list-${selectedTool.id}`}>
                          {selectedTool.builtin_models.map(c => (
                            <option key={c} value={c} />
                          ))}
                        </datalist>
                      </div>
                    )}

                  </div>
                )}

                {/* Fallback 模型 — 按供应商分组，可折叠。
                    与优化器 / 整流器同理：没选主模型时等于还是官方配置，
                    fallback 也不会生效，摆出来是误导。 */}
                {selectedTool.supports_fallback_model && selectedTool.installed && usingThirdPartyModel && fallbackProviders.length > 0 && (
                  <CollapsibleCard
                    title={t("toollaunch.fallbackLabel")}
                    hint={t("toollaunch.fallbackHint")}
                    open={fallbackOpen}
                    onToggle={() => setFallbackOpen(!fallbackOpen)}
                    summary={selectedFallbackModel || t("toollaunch.noFallbackShort")}
                  >
                    <ModelSelector
                      providers={fallbackProviders}
                      providerId={selectedFallbackProvider}
                      modelId={selectedFallbackModel}
                      onSelect={(pid, mid) => {
                        if (!mid) {
                          setSelectedFallbackModel("");
                          setSelectedFallbackProvider("");
                          setFallbackOneMContext(false);
                        } else {
                          setSelectedFallbackModel(mid);
                          setSelectedFallbackProvider(pid);
                        }
                      }}
                      allowClear
                    />
                    {selectedFallbackModel && selectedTool.builtin_models.length > 0 && (
                      <div className="mt-3">
                        <label className="text-caption font-bold text-slate-300 mb-1.5 block">{t("toollaunch.fallbackMqLabel")} <span className="text-micro text-slate-500 font-normal">{t("toollaunch.optional")}</span></label>
                        <p className="text-micro text-slate-500 mb-1.5">{t("toollaunch.fallbackMqHint", { model: selectedFallbackModel })}</p>
                        <input type="text" list={`fb-masq-list-${selectedTool.id}`} value={fallbackMasqueradeModel}
                          onChange={e => setFallbackMasqueradeModel(e.target.value)}
                          placeholder={t("toollaunch.noFallbackMqPh", { model: selectedFallbackModel })}
                          className="w-full ui-input rounded-ctl px-3 py-2 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                        <datalist id={`fb-masq-list-${selectedTool.id}`}>
                          {selectedTool.builtin_models.map(c => (
                            <option key={c} value={c} />
                          ))}
                        </datalist>
                      </div>
                    )}
                    {selectedFallbackModel && (
                      <>
                        {selectedTool.support_one_m_context && (
                          <label className="flex items-center gap-2 mt-2 text-tiny text-slate-400 cursor-pointer select-none">
                            <input type="checkbox" checked={fallbackOneMContext} onChange={e => setFallbackOneMContext(e.target.checked)}
                              className="accent-[var(--module-accent)]" />
                            {t("toollaunch.fallbackOneM")}
                          </label>
                        )}
                        <div className="mt-1 text-tiny text-amber-400">{t("toollaunch.fallbackPreview", { model: `${selectedFallbackModel}${fallbackOneMContext ? "[1m]" : ""}` })}{fallbackMasqueradeModel && <>{t("toollaunch.masqueradeAs", { model: `${fallbackMasqueradeModel}${fallbackOneMContext ? "[1m]" : ""}` })}</>}</div>
                      </>
                    )}
                  </CollapsibleCard>
                )}

                {/* 1M Context Toggle — 由 config.json 的 supportOneMContext 字段驱动 */}
                {selectedTool.supports_model && selectedTool.support_one_m_context && (
                  <div className="flex items-center justify-between p-2.5 rounded-ctl bg-slate-900/30 border border-white/5">
                    <div className="flex items-center gap-2">
                      <span className="text-tiny font-semibold text-slate-300">1M Context</span>
                      <span className="text-[8px] text-slate-500 hidden sm:inline">{t("toollaunch.oneMHint")}</span>
                    </div>
                    <button
                      onClick={() => setOneMContext(!oneMContext)}
                      className={`p-1 rounded-md cursor-pointer transition-all ${oneMContext ? "text-[var(--module-accent)]" : "text-slate-600 hover:text-slate-400"}`}
                    >
                      {oneMContext ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                    </button>
                  </div>
                )}

                {/* Codex web_search：默认关；开启 → 写 config.toml `web_search = "live"` */}
                {selectedTool.id === "codex-cli" && !useOfficialModel && (
                  <div className="rounded-ctl bg-slate-900/30 border border-white/5 overflow-hidden">
                    <div className="flex items-center justify-between p-2.5">
                      <div className="flex items-center gap-2">
                        <span className="text-tiny font-semibold text-slate-300">{t("toollaunch.liveSearch")}</span>
                        <span className="text-[8px] text-slate-500 hidden sm:inline">{t("toollaunch.liveSearchHint")}</span>
                      </div>
                      <button onClick={() => setWebSearchEnabled(!webSearchEnabled)}
                        className={`p-1 rounded-md cursor-pointer transition-all ${webSearchEnabled ? "text-emerald-400" : "text-slate-600 hover:text-slate-400"}`}>
                        {webSearchEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                      </button>
                    </div>
                  </div>
                )}

                {/* 代理增强能力（优化器 / 整流器）— 默认沿用全局配置，可在此按启动覆盖。
                    只有真的会走第三方模型时才显示：没选模型等于还是用官方配置，
                    代理根本不会介入，这两个开关摆出来是误导。 */}
                {selectedTool.supports_model && usingThirdPartyModel && (selectedTool.supports_optimizer || selectedTool.supports_rectifier) && (
                  <div className="space-y-2">
                    {selectedTool.supports_optimizer && (
                      <CollapsibleCard
                        title={t("toollaunch.optimizer")}
                        hint={t("toollaunch.optimizerHint")}
                        open={optimizerOpen}
                        onToggle={() => setOptimizerOpen(!optimizerOpen)}
                        summary={optimizerEnabled ? t("toollaunch.stateOn") : t("toollaunch.stateOff")}
                        action={
                          <button onClick={() => setOptimizerEnabled(!optimizerEnabled)}
                            className={`p-1 rounded-md cursor-pointer transition-all ${optimizerEnabled ? "text-[var(--module-accent)]" : "text-slate-600 hover:text-slate-400"}`}>
                            {optimizerEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                          </button>
                        }
                      >
                      <div className="rounded-ctl bg-slate-900/30 border border-white/5 overflow-hidden">
                        {optimizerEnabled && (
                          <div className="px-3 pb-2.5 space-y-1.5 border-t border-white/5 pt-2">
                            {[
                              { key: "cache_injection" as const, label: t("toollaunch.optCacheInjection"), desc: t("toollaunch.optCacheInjectionDesc") },
                              { key: "thinking_optimizer" as const, label: t("toollaunch.optThinking"), desc: t("toollaunch.optThinkingDesc") },
                              { key: "deepseek_normalize" as const, label: t("toollaunch.optDeepseek"), desc: t("toollaunch.optDeepseekDesc") },
                            ].map(item => (
                              <label key={item.key} className="flex items-center gap-2 cursor-pointer">
                                <input
                                  type="checkbox"
                                  checked={optimizerStrategies[item.key]}
                                  onChange={() => setOptimizerStrategies(prev => ({ ...prev, [item.key]: !prev[item.key] }))}
                                  className="accent-[var(--module-accent)]"
                                />
                                <span className="text-tiny text-slate-300">{item.label}</span>
                                <span className="text-micro text-slate-600">{item.desc}</span>
                              </label>
                            ))}
                          </div>
                        )}
                        </div>
                      </CollapsibleCard>
                    )}
                    {selectedTool.supports_rectifier && (
                      <CollapsibleCard
                        title={t("toollaunch.rectifier")}
                        hint={t("toollaunch.rectifierHint")}
                        open={rectifierOpen}
                        onToggle={() => setRectifierOpen(!rectifierOpen)}
                        summary={rectifierEnabled ? t("toollaunch.stateOn") : t("toollaunch.stateOff")}
                        action={
                          <button onClick={() => setRectifierEnabled(!rectifierEnabled)}
                            className={`p-1 rounded-md cursor-pointer transition-all ${rectifierEnabled ? "text-[var(--module-accent)]" : "text-slate-600 hover:text-slate-400"}`}>
                            {rectifierEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                          </button>
                        }
                      >
                      <div className="rounded-ctl bg-slate-900/30 border border-white/5 overflow-hidden">
                        {rectifierEnabled && (
                          <div className="px-3 pb-2.5 space-y-1.5 border-t border-white/5 pt-2">
                            {[
                              { key: "thinking_signature" as const, label: t("toollaunch.recThinkingSig"), desc: t("toollaunch.recThinkingSigDesc") },
                              { key: "thinking_budget" as const, label: t("toollaunch.recThinkingBudget"), desc: t("toollaunch.recThinkingBudgetDesc") },
                              { key: "media_fallback" as const, label: t("toollaunch.recMedia"), desc: t("toollaunch.recMediaDesc") },
                              { key: "media_heuristic" as const, label: t("toollaunch.recMediaHeuristic"), desc: t("toollaunch.recMediaHeuristicDesc") },
                              { key: "protocol_mismatch" as const, label: t("toollaunch.recProtocol"), desc: t("toollaunch.recProtocolDesc") },
                              { key: "toolcall_dialect" as const, label: t("toollaunch.recToolcallDialect"), desc: t("toollaunch.recToolcallDialectDesc") },
                            ].map(item => (
                              <label key={item.key} className="flex items-center gap-2 cursor-pointer">
                                <input
                                  type="checkbox"
                                  checked={rectifierStrategies[item.key]}
                                  onChange={() => setRectifierStrategies(prev => ({ ...prev, [item.key]: !prev[item.key] }))}
                                  className="accent-[var(--module-accent)]"
                                />
                                <span className="text-tiny text-slate-300">{item.label}</span>
                                <span className="text-micro text-slate-600">{item.desc}</span>
                              </label>
                            ))}
                          </div>
                        )}
                        </div>
                      </CollapsibleCard>
                    )}
                  </div>
                )}

  </>);
  return {
    customParamValues,
    setCustomParamValues,
    selectedModel,
    setSelectedModel,
    selectedModelProvider,
    setSelectedModelProvider,
    effectiveAlias,
    oneMContext,
    setOneMContext,
    fallbackOneMContext,
    setFallbackOneMContext,
    masqueradeModel,
    setMasqueradeModel,
    optimizerEnabled,
    setOptimizerEnabled,
    rectifierEnabled,
    setRectifierEnabled,
    rectifierStrategies,
    setRectifierStrategies,
    optimizerStrategies,
    setOptimizerStrategies,
    webSearchEnabled,
    setConfigAppliedModel,
    setConfigAppliedFallback,
    selectedFallbackModel,
    setSelectedFallbackModel,
    selectedFallbackProvider,
    setSelectedFallbackProvider,
    fallbackMasqueradeModel,
    setFallbackMasqueradeModel,
    useOfficialModel,
    setUseOfficialModel,
    currentModelCustomParams,
    usingThirdPartyModel,
    renderModelOptions,
  };
}
