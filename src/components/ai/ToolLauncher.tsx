import { ToolCacheDirectoryList } from "./ToolCacheDirectoryList";
import { ToolCustomPathEditor } from "./ToolCustomPathEditor";
import { ToolLastLaunchSummary } from "./ToolLastLaunchSummary";
import { ToolInstallationHeader } from "./ToolInstallationHeader";
import { useToolOperations } from "./useToolOperations";
import { useToolModelConfiguration } from "./useToolModelConfiguration";
import { useToolMarketplace } from "./useToolMarketplace";
import { useToolSessions } from "./useToolSessions";
import React, { Fragment, useState, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ConfirmDialogHost } from "../shared/ConfirmDialog";
import { ResultNote } from "../shared/Note";
import { useToolListWidth } from "../shared/ai/paneWidth";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Rocket, FolderOpen, CheckCircle, RefreshCw, Bot, Play, Copy, ExternalLink, HardDrive, ChevronDown, Download, Shield, Terminal } from "lucide-react";
import type { AiConfig, LastLaunchConfig, DetectedAiTool, ToolSession, TerminalInfo } from "../../contracts/ai";
import { PROTOCOL_LABELS, formatRelativeTime, getProxyInfo, resolveAppliedModelRef, type AppliedModels, type ToolKindFilter } from "./ToolLauncher.model";
import { CollapsibleCard } from "./ToolLauncher.CollapsibleCard";

export default function ToolLauncher({ onAskAssistant }: { onAskAssistant?: (question: string) => void } = {}) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(true);
  const [projectPath, setProjectPath] = useState("");
  const [selectedTerminal, setSelectedTerminal] = useState("cmd");
  const [selectedToolId, setSelectedToolId] = useState<string | null>(null);
  const [tools, setTools] = useState<DetectedAiTool[]>([]);
  const [kindFilter, setKindFilter] = useState<ToolKindFilter>("all");
  // 左栏宽度：可拖动，宽度持久化在 localStorage（纯 UI 偏好，不惊动后端）
  const [listWidth, setListWidth] = useToolListWidth();
  const startListResize = (e: React.MouseEvent) => {
    e.preventDefault();
    const startX = e.clientX;
    const startWidth = listWidth;
    let next = startWidth;
    const onMove = (ev: MouseEvent) => {
      next = startWidth + (ev.clientX - startX);
      setListWidth(next);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      // 松手才落盘：拖动中每像素都写一次太浪费
      setListWidth(next);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const {
    setSessions,
    sessionMode,
    setSessionMode,
    selectedSession,
    setSelectedSession,
    setShowSessionPicker,
    renderSessionPicker,
  } = useToolSessions({
    selectedTool: tools.find(tool => tool.id === selectedToolId) || null,
    setProjectPath,
  });
  // 插件市场默认收起：它自带搜索框 + 插件清单，展开时占掉大半屏；
  // 收起后靠 CollapsibleCard 的 summary 仍能看到「已安装 · 几个插件」。
  const [marketOpen, setMarketOpen] = useState(false);
  // Codex 官方插件市场（写 ~/.codex/config.toml 的 [marketplaces.*]）。
  // CodexPlusPlus 同样的思路：市场内容我们自己落地 + 注册，单个插件由客户端自己装。
  const {
    marketplace,
    setMarketplace,
    marketplaceBusy,
    claudeStatus,
    claudePlugins,
    loadMarketplace,
    installMarketplace,
    removeMarketplace,
    marketKind,
    marketReady,
    renderMarketplaceView,
  } = useToolMarketplace({
    selectedTool: tools.find(tool => tool.id === selectedToolId) || null,
  });

  const [launching, setLaunching] = useState(false);
  const [launchResult, setLaunchResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const {
    upgradeResult,
    installResult,
    uninstallResult,
    opLogs,
    opLogRef,
    versionStatuses,
    checkingVersions,
    cacheInfos,
    showCacheManager,
    setShowCacheManager,
    migratingCache,
    cleaningCache,
    selectedToolCaches,
    checkVersions,
    handleUpgrade,
    handleInstall,
    confirmRequest,
    setConfirmRequest,
    askUninstall,
    loadCacheInfos,
    handleMigrateCache,
    handleCleanCache,
    handleOpenCacheDir,
    getVerStatus,
    getBusy,
  } = useToolOperations({
    selectedToolId,
    tools,
    setTools,
  });

  // 各工具的上次启动方式记录
  const [lastLaunchConfigs, setLastLaunchConfigs] = useState<Record<string, LastLaunchConfig>>({});

  const selectedTool = tools.find(t => t.id === selectedToolId) || null;
  const {
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
  } = useToolModelConfiguration({
    config,
    selectedTool,
  });

  const loadData = useCallback(async () => {
    try {
      const [t, c, term, lcs] = await Promise.all([
        invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []),
        invoke<AiConfig>("get_ai_config").catch(() => ({ providers: [], proxy_port: 15721, default_project_path: "", rectifier: { enabled: false, thinking_signature: false, thinking_budget: false, media_fallback: false, media_heuristic: false, protocol_mismatch: false, toolcall_dialect: false }, headroom: { enabled: false, port: 8791, on_unavailable: "failOpen", disable_kompress: false, timeout_ms: 1500 }, optimizer: { enabled: false, cache_injection: false, thinking_optimizer: false, deepseek_normalize: false }, skills_dir: "" })),
        invoke<TerminalInfo[]>("detect_terminals").catch(() => []),
        invoke<Record<string, LastLaunchConfig>>("get_all_last_launch_configs").catch(() => ({})),
      ]);
      setTools(t);
      setConfig(c);
      setTerminals(term);
      setProjectPath(c.default_project_path || "");
      setLastLaunchConfigs(lcs);
      // 启动页代理增强策略默认沿用全局配置
      setOptimizerEnabled(c.optimizer?.enabled !== false);
      setRectifierEnabled(c.rectifier?.enabled !== false);
      setOptimizerStrategies({
        cache_injection: c.optimizer?.cache_injection !== false,
        thinking_optimizer: c.optimizer?.thinking_optimizer !== false,
        deepseek_normalize: c.optimizer?.deepseek_normalize !== false,
      });
      setRectifierStrategies({
        thinking_signature: c.rectifier?.thinking_signature !== false,
        thinking_budget: c.rectifier?.thinking_budget !== false,
        media_fallback: c.rectifier?.media_fallback !== false,
        media_heuristic: c.rectifier?.media_heuristic !== false,
        protocol_mismatch: c.rectifier?.protocol_mismatch !== false,
        toolcall_dialect: c.rectifier?.toolcall_dialect !== false,
      });
    } catch (e) { console.error(e); }
    finally { setLoading(false); }
  }, []);

  // 重新拉取工具列表（保存资料后刷新头像/昵称）
  const reloadTools = useCallback(async () => {
    try {
      const t = await invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []);
      setTools(t);
    } catch { /* ignore */ }
  }, []);

  // 手动指定安装路径：自动检测认不出来（绿色安装包装在别的目录、自定义安装位置等）
  // 时由用户直接给出可执行文件或其所在目录，写进 ~/.any-version/tool-paths.json。
  const [pathInput, setPathInput] = useState("");
  const [pathMsg, setPathMsg] = useState<{ ok: boolean; msg: string } | null>(null);
  const [pathSaving, setPathSaving] = useState(false);

  // 切换工具时把输入框同步成该工具已存的手动路径
  useEffect(() => {
    setPathInput(selectedTool?.custom_path || "");
    setPathMsg(null);
  }, [selectedTool?.id, selectedTool?.custom_path]);

  const saveCustomPath = async (value: string | null) => {
    if (!selectedTool) return;
    setPathSaving(true);
    setPathMsg(null);
    try {
      await invoke("ai_set_tool_custom_path", { toolId: selectedTool.id, path: value });
      setPathMsg({ ok: true, msg: value ? t("toollaunch.pathSaved") : t("toollaunch.pathCleared") });
      // 重新探测：手动路径会直接影响「已安装 / 未安装」的判定
      await reloadTools();
    } catch (e) {
      setPathMsg({ ok: false, msg: String(e) });
    } finally {
      setPathSaving(false);
    }
  };

  const browseCustomPath = async () => {
    const picked = await open({ multiple: false, directory: false, title: t("toollaunch.pathBrowse") });
    if (typeof picked === "string" && picked.trim()) {
      setPathInput(picked);
      await saveCustomPath(picked);
    }
  };

  useEffect(() => { loadData(); }, [loadData]);

  useEffect(() => {
    const unlisten = listen<{ default_project_path?: string; skills_dir?: string; providers_changed?: boolean }>("ai-config-changed", (event) => {
      if (event.payload.default_project_path) setProjectPath(event.payload.default_project_path);
      // 模型配置变更时重新加载
      if (event.payload.providers_changed) {
        invoke<AiConfig>("get_ai_config").then(setConfig).catch(() => {});
      }
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  useEffect(() => {
    if (!selectedTool?.installed) { setSessions([]); return; }
    invoke<ToolSession[]>("scan_tool_sessions", { toolId: selectedTool.id })
      .then(setSessions).catch(() => setSessions([]));
  }, [selectedTool]);

  const handleBrowse = async () => {
    try {
      const selected = await open({ directory: true, title: t("toollaunch.pickProjectDir") });
      if (selected) setProjectPath(selected as string);
    } catch { /* ignore */ }
  };

  const handleLaunch = async () => {
    if (!selectedTool) return;
    setLaunching(true);
    setLaunchResult(null);
    try {
      // 启动即决定配置文件的去向，不再单独提供「只保存模型」：
      // - 勾选「使用官方模型」→ 先还原工具自己的官方配置（清掉 Kira 写进去的模型），再启动；
      // - 选了第三方模型 → 启动流程本身就会把该模型写进工具配置（后端 launch_ai_tool 已做）。
      if (useOfficialModel && selectedTool.config_file) {
        // 勾选「使用官方模型」→ 先清掉 Kira 写进去的模型再启动（失败就别启动了，
        // 否则启动出来的还是上一轮的模型）。
        await invoke<string>("restore_ai_tool_config", { toolId: selectedTool.id });
      }
      const result = await invoke<{ success: boolean; message: string }>("launch_ai_tool", {
        req: {
          tool_id: selectedTool.id,
          project_path: (sessionMode === "resume" || sessionMode === "fork") && selectedSession ? selectedSession.project_path : projectPath,
          model_id: useOfficialModel ? null : (selectedModel || null),
          provider_id: useOfficialModel ? null : (selectedModelProvider || null),
          fallback_model_id: useOfficialModel ? null : (selectedFallbackModel || null),
          fallback_masquerade_model: useOfficialModel ? null : (fallbackMasqueradeModel || null),
          session_id: selectedSession?.session_id || null,
          session_mode: sessionMode,
          terminal_id: selectedTerminal,
          one_m_context: selectedTool.support_one_m_context ? oneMContext : false,
          fallback_one_m_context: selectedTool.support_one_m_context ? (selectedFallbackModel ? fallbackOneMContext : false) : false,
          masquerade_model: useOfficialModel ? null : (masqueradeModel || null),
          optimizer_enabled: useOfficialModel ? null : optimizerEnabled,
          rectifier_enabled: useOfficialModel ? null : rectifierEnabled,
          optimizer_cache_injection: useOfficialModel ? null : optimizerStrategies.cache_injection,
          optimizer_thinking: useOfficialModel ? null : optimizerStrategies.thinking_optimizer,
          optimizer_deepseek: useOfficialModel ? null : optimizerStrategies.deepseek_normalize,
          rectifier_thinking_signature: useOfficialModel ? null : rectifierStrategies.thinking_signature,
          rectifier_thinking_budget: useOfficialModel ? null : rectifierStrategies.thinking_budget,
          rectifier_media_fallback: useOfficialModel ? null : rectifierStrategies.media_fallback,
          rectifier_media_heuristic: useOfficialModel ? null : rectifierStrategies.media_heuristic,
          rectifier_protocol_mismatch: useOfficialModel ? null : rectifierStrategies.protocol_mismatch,
          rectifier_toolcall_dialect: useOfficialModel ? null : rectifierStrategies.toolcall_dialect,
          web_search_enabled: useOfficialModel ? false : webSearchEnabled,
          custom_params: useOfficialModel ? [] : currentModelCustomParams,
          custom_param_values: useOfficialModel ? {} : customParamValues,
        },
      });
      setLaunchResult({ ok: result.success, msg: result.message });
      if (result.success) {
        const updated = await invoke<ToolSession[]>("scan_tool_sessions", { toolId: selectedTool.id }).catch(() => []);
        setSessions(updated);
        // 保存本次启动配置
        const providerName = config?.providers.find(p => p.id === selectedModelProvider)?.name || null;
        const lc: LastLaunchConfig = {
          providerId: useOfficialModel ? null : (selectedModelProvider || null),
          providerName: providerName,
          modelId: useOfficialModel ? null : (selectedModel || null),
          fallbackModelId: useOfficialModel ? null : (selectedFallbackModel || null),
          fallbackProviderId: useOfficialModel ? null : (selectedFallbackProvider || null),
          fallbackMasqueradeModel: useOfficialModel ? null : (fallbackMasqueradeModel || null),
          useOfficialModel: useOfficialModel,
          terminalId: selectedTerminal,
          oneMContext: selectedTool.support_one_m_context ? oneMContext : false,
          fallbackOneMContext: selectedTool.support_one_m_context ? (selectedFallbackModel ? fallbackOneMContext : false) : false,
          masqueradeModel: useOfficialModel ? null : (masqueradeModel || null),
          optimizerEnabled: useOfficialModel ? null : optimizerEnabled,
          rectifierEnabled: useOfficialModel ? null : rectifierEnabled,
          customParamValues: useOfficialModel ? {} : customParamValues,
          projectPath: (sessionMode === "resume" || sessionMode === "fork") && selectedSession ? selectedSession.project_path : projectPath,
          lastLaunchedAt: new Date().toISOString(),
        };
        await invoke("save_last_launch_config", { toolId: selectedTool.id, config: lc }).catch(() => {});
        setLastLaunchConfigs(prev => ({ ...prev, [selectedTool.id]: lc }));
      }
    } catch (e: any) {
      setLaunchResult({ ok: false, msg: String(e) });
    } finally { setLaunching(false); }
  };

  // 桌面工具与项目目录无关（后端会用 exe 所在目录当工作目录），不该因为它被卡住
  const canLaunch = !!selectedTool?.installed
    && (selectedTool.tool_kind === "desktop" || sessionMode === "resume" || sessionMode === "fork" || !!projectPath)
    // 分叉必须挑一条会话（没有源会话就无从复制）
    && (sessionMode !== "fork" || !!selectedSession);

  // 列表分组（抄 EchoBird 的维度：先按已装/未装分开，已装的排在前面）
  // 形态筛选对**两组都生效**：只筛未装那组的话，选「桌面端」时已装的 CLI 照样在列表里，
  // 筛选看起来就是坏的。
  const matchesKind = (t: DetectedAiTool) =>
    kindFilter === "all" || (t.tool_kind ?? "other") === kindFilter;
  const installedTools = tools.filter(t => t.installed);
  const notInstalledTools = tools.filter(t => !t.installed);
  const visibleInstalled = installedTools.filter(matchesKind);
  const visibleNotInstalled = notInstalledTools.filter(matchesKind);
  const visibleTools = [...visibleInstalled, ...visibleNotInstalled];

  if (loading) {
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-body">{t("toollaunch.loading")}</span></div>;
  }

  return (
    <div className="h-full flex min-h-0 select-none">
      {/* ── 左侧工具列表（宽度可拖动，见下方分隔条） ── */}
      <div
        style={{ width: listWidth }}
        className="flex-shrink-0 border-r border-white/5 py-3 px-2 overflow-y-auto space-y-0.5 flex flex-col"
      >
        <div className="flex items-center gap-1 px-1 mb-1">
          {/* 形态筛选：对已装 / 未装两组都生效。
              窄栏放不下「AI 工具」标题 + 三个 tab，标题去掉、tab 靠左铺满。 */}
          {tools.length > 0 && (
            <div className="flex items-center gap-0.5 flex-1 min-w-0">
              {(["all", "cli", "desktop"] as ToolKindFilter[]).map(k => (
                <button
                  key={k}
                  onClick={() => setKindFilter(k)}
                  className={`px-1 py-0.5 rounded text-[8px] cursor-pointer transition-all whitespace-nowrap ${
                    kindFilter === k
                      ? "bg-[var(--module-accent)]/25 text-white font-semibold"
                      : "text-slate-600 hover:text-slate-400"
                  }`}
                >
                  {k === "all" ? t("toollaunch.kindAll") : k === "cli" ? t("toollaunch.kindCli") : t("toollaunch.kindDesktop")}
                </button>
              ))}
            </div>
          )}
          <button onClick={checkVersions} disabled={checkingVersions}
            className="ml-auto p-0.5 rounded text-slate-600 hover:text-slate-400 cursor-pointer"
            title={t("toollaunch.checkVersion")}>
            <RefreshCw className={`w-3 h-3 ${checkingVersions ? "animate-spin" : ""}`} />
          </button>
        </div>
        {/* 不再打「已安装 / 未安装」分组标题：列表里每项自己就带状态（绿点 + 版本号），
            标题只是重复这句话，还占掉两行高度。 */}
        {/* 筛选后一个都不剩：明说原因，别让整段静默消失 */}
        {visibleTools.length === 0 && (
          <div className="px-1 py-1.5 text-micro text-slate-600">{t("toollaunch.kindEmpty")}</div>
        )}
        {visibleTools.map((tool) => {
          const vs = getVerStatus(tool.id);
          return (
            <Fragment key={tool.id}>
            <button
              onClick={async () => {
                setSelectedToolId(tool.id);
                // 重置默认值
                setSelectedModel("");
                setSelectedModelProvider("");
                setSelectedFallbackModel("");
                setSelectedFallbackProvider("");
                setFallbackMasqueradeModel("");
                setSessionMode("new");
                setSelectedSession(null);
                setShowSessionPicker(false);
                setLaunchResult(null);
                setShowCacheManager(false);
                setOneMContext(false);
                setFallbackOneMContext(false);
                setMasqueradeModel("");
                setOptimizerEnabled(config?.optimizer?.enabled !== false);
                setRectifierEnabled(config?.rectifier?.enabled !== false);
                setOptimizerStrategies({
                  cache_injection: config?.optimizer?.cache_injection !== false,
                  thinking_optimizer: config?.optimizer?.thinking_optimizer !== false,
                  deepseek_normalize: config?.optimizer?.deepseek_normalize !== false,
                });
                setRectifierStrategies({
                  thinking_signature: config?.rectifier?.thinking_signature !== false,
                  thinking_budget: config?.rectifier?.thinking_budget !== false,
                  media_fallback: config?.rectifier?.media_fallback !== false,
                  media_heuristic: config?.rectifier?.media_heuristic !== false,
                  protocol_mismatch: config?.rectifier?.protocol_mismatch !== false,
                  toolcall_dialect: config?.rectifier?.toolcall_dialect !== false,
                });
                setSelectedTerminal("cmd");
                setUseOfficialModel(tool.api_protocol === "none");
                // 加载上次启动配置并恢复 UI 状态
                let hasLastSelection = false;
                try {
                  const last = await invoke<LastLaunchConfig | null>("get_last_launch_config", { toolId: tool.id });
                  if (last) {
                    setLastLaunchConfigs(prev => ({ ...prev, [tool.id]: last }));
                    if (last.useOfficialModel) {
                      setUseOfficialModel(true);
                    } else {
                      // 先设置 provider，触发模型列表更新
                      if (last.providerId) {
                        setSelectedModelProvider(last.providerId);
                      }
                      // 再设置 model（React 会批量更新，下次渲染时模型列表已更新）
                      if (last.modelId) setSelectedModel(last.modelId);
                      hasLastSelection = !!(last.providerId && last.modelId);
                      if (last.customParamValues) setCustomParamValues(last.customParamValues);
                      if (last.fallbackModelId) setSelectedFallbackModel(last.fallbackModelId);
                      if (last.fallbackProviderId) setSelectedFallbackProvider(last.fallbackProviderId);
                      if (last.fallbackMasqueradeModel) setFallbackMasqueradeModel(last.fallbackMasqueradeModel);
                    }
                    if (last.terminalId && last.terminalId !== "cmd") setSelectedTerminal(last.terminalId);
                    if (last.oneMContext) setOneMContext(true);
                    if (last.fallbackOneMContext) setFallbackOneMContext(true);
                    if (last.masqueradeModel) setMasqueradeModel(last.masqueradeModel);
                    if (last.optimizerEnabled !== null && last.optimizerEnabled !== undefined) setOptimizerEnabled(last.optimizerEnabled);
                    if (last.rectifierEnabled !== null && last.rectifierEnabled !== undefined) setRectifierEnabled(last.rectifierEnabled);
                    if (last.projectPath) setProjectPath(last.projectPath);
                  }
                } catch { /* 无历史记录 */
                }
                // 再以**工具配置文件**为准回显一次。
                //
                // ⚠️ 只能用于「没有上次启动记录」的情况。**伪装生效时配置文件里写的是声明名**
                // （ChatGPT 桌面端写 `gpt-6-astra`，真实模型是 `space-bunny`），拿它去供应商
                // 列表里 `findModelRef` 反查会命中**另一个供应商的同名模型** —— 实测就撞上了
                // OpenCode 的 `gpt-6-astra`，把刚恢复好的「WorkBuddy2API / space-bunny」
                // 顶成了「OpenCode / gpt-6-astra」，伪装输入框也跟着清空。
                //
                // 「上次启动记录」才是「用户选了哪个供应商与模型」的权威来源；配置文件这一路
                // 仍然**始终**回读（`configAppliedModel`），用于提示当前实际生效的是什么。
                setConfigAppliedModel(null);
                setConfigAppliedFallback(null);
                if (tool.config_file) {
                  try {
                    const applied = await invoke<AppliedModels>("get_ai_tool_models", { toolId: tool.id });
                    setConfigAppliedModel(applied.model ?? null);
                    setConfigAppliedFallback(applied.fallback_model ?? null);
                    const providers = config?.providers ?? [];
                    const mainRef = resolveAppliedModelRef(providers, applied.model, hasLastSelection);
                    if (mainRef) {
                      setUseOfficialModel(false);
                      setSelectedModelProvider(mainRef.providerId);
                      setSelectedModel(mainRef.modelId);
                    }
                    const fbRef = resolveAppliedModelRef(providers, applied.fallback_model, hasLastSelection);
                    if (fbRef) {
                      setSelectedFallbackProvider(fbRef.providerId);
                      setSelectedFallbackModel(fbRef.modelId);
                    }
                  } catch { /* 读不到就当没有，不影响使用 */
                  }
                }
                // Codex 插件市场状态只对声明了该能力的工具查（写的是全局 ~/.codex）
                setMarketplace(null);
                if (tool.supports_plugin_marketplace) void loadMarketplace(tool);
              }}
              className={`w-full px-3 py-2.5 rounded-ctl text-left transition-all cursor-pointer ${
                selectedToolId === tool.id
                  // 选中态用**半透明**主题色：纯色块会盖掉整行的层次（形态徽标、
                  // 版本号、状态色全被吞掉），半透明既标得清又不压内容。
                  ? "bg-[var(--module-accent)]/25 text-white ring-1 ring-[var(--module-accent)]/40"
                  : tool.installed
                    ? "text-slate-300 hover:text-white hover:bg-white/5"
                    : "text-slate-600 hover:text-slate-400 hover:bg-white/[0.03]"
              }`}
            >
              <div className="flex items-center gap-2">
                <span className="w-4 h-4 flex-shrink-0 flex items-center justify-center text-body">{tool.avatar || '🤖'}</span>
                <div className="flex items-center gap-1 min-w-0 flex-1">
                  {/* 形态徽标：一行就能看出是 CLI 还是桌面工具。
                      选中态底色是纯 accent，所以徽标也走 text-white/70 + bg-white/10 */}
                  <span className={`text-[8px] px-1 py-px rounded flex-shrink-0 font-semibold ${
                    selectedToolId === tool.id
                      ? "bg-white/15 text-white/80"
                      : tool.tool_kind === "desktop"
                        ? "bg-sky-500/20 text-sky-300"
                        : tool.tool_kind === "cli"
                          ? "bg-slate-500/20 text-slate-400"
                          : "bg-white/5 text-slate-600"
                  }`}>
                    {tool.tool_kind === "desktop"
                      ? t("toollaunch.kindDesktop")
                      : tool.tool_kind === "cli"
                        ? t("toollaunch.kindCli")
                        : t("toollaunch.kindOther")}
                  </span>
                  <span className="text-caption font-semibold truncate">{tool.nickname || tool.display_name}</span>
                  {/* 真实名称：选中态背景就是纯 accent，再叠 accent 半透明等于看不见，
                      一律用白色半透明（任何主题色下都清晰） */}
                  {tool.nickname && tool.nickname !== tool.display_name && (
                    <span className={`text-micro truncate flex-shrink-0 ${
                      selectedToolId === tool.id ? "text-white/70" : "text-slate-500"
                    }`}>
                      ({tool.display_name})
                    </span>
                  )}
                </div>
                {getBusy(tool.id) ? (
                  <span className="text-micro font-semibold flex items-center gap-0.5 ml-auto flex-shrink-0 text-blue-300">
                    <RefreshCw className="w-2.5 h-2.5 animate-spin" />
                    {getBusy(tool.id) === "upgrading" ? t("toollaunch.upgrading") : getBusy(tool.id) === "installing" ? t("toollaunch.installing") : t("toollaunch.uninstalling")}
                  </span>
                ) : vs && (
                  <span className={`text-micro font-semibold flex items-center gap-0.5 ml-auto flex-shrink-0 ${vs.color}`}>
                    {vs.icon}
                    {vs.label}
                  </span>
                )}
                {/* 没有版本状态可比（桌面应用按路径识别，拿不到版本号）时给个绿色小勾：
                    这类工具的状态只有「已安装/未安装」两态，不显示就等于看不出来 */}
                {!getBusy(tool.id) && !vs && tool.installed && (
                  <CheckCircle className="w-3 h-3 ml-auto flex-shrink-0 text-emerald-400/80" />
                )}
              </div>
              <div className="flex items-center gap-1.5 mt-0.5 ml-5.5">
                {getBusy(tool.id) === "installing" ? (
                  <span className="text-micro text-blue-300 animate-pulse">{t("toollaunch.installing")}...</span>
                ) : getBusy(tool.id) === "upgrading" ? (
                  <span className="text-micro text-blue-300 animate-pulse">{t("toollaunch.upgrading")}...</span>
                ) : getBusy(tool.id) === "uninstalling" ? (
                  <span className="text-micro text-blue-300 animate-pulse">{t("toollaunch.uninstalling")}...</span>
                ) : tool.installed ? (
                  // 与真实名称同理：选中态底色就是 accent，文字不能再用 accent。
                  // 版本号取不到时**不写「已安装」**：装没装从头像的明暗/右侧状态就看得出来，
                  // 写一行字反而把列表塞满重复信息。
                  tool.version ? (
                    <span className={`text-micro ${selectedToolId === tool.id ? "text-white/70" : "text-slate-500"} font-mono`}>
                      {tool.version}
                    </span>
                  ) : null
                ) : (
                  <span className="flex items-center gap-1">
                    {/* 未安装也不再写字：灰掉的样式本身就在表达「没装」，
                        只留下真正有用的入口（问助手 / 官网） */}
                    {/* 没装的工具一键去问助手：跳转并预填问题，省得用户自己敲 */}
                    {onAskAssistant && (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          onAskAssistant(t("toollaunch.askInstallQuestion", { name: tool.display_name }));
                        }}
                        className="text-[8px] px-1 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer transition-colors"
                        title={t("toollaunch.askAssistant")}
                      >
                        {t("toollaunch.askAssistant")}
                      </button>
                    )}
                    {tool.website && (
                      <a href={tool.website} target="_blank" rel="noopener noreferrer"
                        onClick={(e) => { e.preventDefault(); e.stopPropagation(); void openUrl(tool.website); }}
                        className="text-blue-400/70 hover:text-blue-300 transition-colors flex items-center"
                        title={t("toollaunch.openSite")}>
                        <ExternalLink className="w-2.5 h-2.5" />
                      </a>
                    )}
                  </span>
                )}
                {lastLaunchConfigs[tool.id] && tool.installed && (
                  <div className={`flex items-center gap-1 mt-0.5 ml-5.5 flex-wrap ${selectedToolId === tool.id ? "text-white/70" : "text-slate-600"}`}>
                    {lastLaunchConfigs[tool.id].useOfficialModel ? (
                      <span className="text-micro">{t("toollaunch.official")}</span>
                    ) : (
                      <>
                        <span className="text-micro truncate max-w-[60px]">
                          {lastLaunchConfigs[tool.id].providerName || lastLaunchConfigs[tool.id].providerId || "-"}
                        </span>
                        {lastLaunchConfigs[tool.id].modelId && (
                          <span className="text-micro truncate max-w-[50px] opacity-70">
                            · {lastLaunchConfigs[tool.id].modelId}
                          </span>
                        )}
                        {lastLaunchConfigs[tool.id].fallbackModelId && (
                          <span className="text-micro text-amber-400/80 truncate max-w-[50px]">
                            ※ {lastLaunchConfigs[tool.id].fallbackModelId}
                          </span>
                        )}
                      </>
                    )}
                    {lastLaunchConfigs[tool.id].lastLaunchedAt && (
                      <span className="text-micro opacity-50 ml-auto">
                        {formatRelativeTime(lastLaunchConfigs[tool.id].lastLaunchedAt, t)}
                      </span>
                    )}
                  </div>
                )}
              </div>
            </button>
            </Fragment>
          );
        })}
      </div>

      {/* 分栏拖拽把手：拖动改变左栏宽度，松手落盘 */}
      <div
        role="separator"
        aria-orientation="vertical"
        title={t("toollaunch.dragResize")}
        onMouseDown={startListResize}
        className="w-1.5 flex-shrink-0 cursor-col-resize hover:bg-[var(--module-accent)]/30 active:bg-[var(--module-accent)]/50 transition-colors"
      />

      {/* ── 右侧设置面板 ── */}
      <div className="flex-1 min-h-0 overflow-y-auto p-6 space-y-4">
        {!selectedTool ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500">
            <Bot className="w-8 h-8 text-slate-700 mb-2" />
            <span className="text-body font-bold text-slate-400">{t("toollaunch.selectToolHint")}</span>
          </div>
        ) : (
          <>
            {/* 工具信息 + 版本详情 */}
            <div className="p-3 rounded-card bg-slate-900/30 border border-white/5">
              <ToolInstallationHeader
                selectedTool={selectedTool}
                getBusy={getBusy}
                versionStatuses={versionStatuses}
                handleUpgrade={handleUpgrade}
                askUninstall={askUninstall}
              />
              {/* 上次启动配置摘要 */}
              {lastLaunchConfigs[selectedTool.id] && (
                <ToolLastLaunchSummary
                  lastLaunchConfigs={lastLaunchConfigs}
                  selectedTool={selectedTool}
                />
              )}
              {!selectedTool.installed && (
                <div className="mt-3 flex items-center gap-2">
                  <code className="flex-1 text-tiny text-slate-300 bg-slate-900 rounded px-2 py-1.5 font-mono truncate">{selectedTool.install_cmd}</code>
                  <button
                    onClick={() => handleInstall(selectedTool)}
                    disabled={getBusy(selectedTool.id) === "installing"}
                    className="px-2 py-1.5 rounded-md bg-[var(--module-accent-soft)] hover:bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-tiny text-[var(--module-accent)] hover:text-[var(--module-accent-strong)] cursor-pointer transition-all flex items-center gap-1 flex-shrink-0 disabled:opacity-50"
                    title={t("toollaunch.installTitle")}
                  >
                    <Download className={`w-3.5 h-3.5 ${getBusy(selectedTool.id) === "installing" ? "animate-spin" : ""}`} />
                    {getBusy(selectedTool.id) === "installing" ? `${t("toollaunch.installing")}...` : t("toollaunch.install")}
                  </button>
                  <button onClick={() => navigator.clipboard.writeText(selectedTool.install_cmd)}
                    className="px-2 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-tiny text-slate-400 hover:text-white cursor-pointer transition-all flex-shrink-0">
                    <Copy className="w-3.5 h-3.5" />
                  </button>
                </div>
              )}
            </div>

            {/* 安装路径：自动检测认不出来时手动指定（可执行文件本身或其所在目录） */}
            <ToolCustomPathEditor
              selectedTool={selectedTool}
              pathInput={pathInput}
              setPathInput={setPathInput}
              browseCustomPath={browseCustomPath}
              pathSaving={pathSaving}
              saveCustomPath={saveCustomPath}
              pathMsg={pathMsg}
            />

            {/* CLI 工具配置面板 */}
            {selectedTool.installed && selectedTool.supports_model && (
              <>
                {/* 官方插件市场（两套后端共用这一块 UI）。
                    - Codex 系：市场 = 下载官方仓库落盘 + 注册进 config.toml；插件走 `codex plugin`。
                    - Claude Code：市场 = 添加一个仓库来源（写进 settings.json）；插件走 `claude plugin`。
                    装/卸一律经官方 CLI —— 只有它才会落插件缓存、维护 enabled 与市场快照，
                    自己写配置文件的话插件在客户端里是「看得见、用不了」。 */}
                {selectedTool.supports_plugin_marketplace && selectedTool.installed && (
                  <div className="rounded-card border border-white/5 bg-slate-900/30 p-3">
                    <CollapsibleCard
                      title={t("toollaunch.pluginMarketplace")}
                      open={marketOpen}
                      onToggle={() => setMarketOpen(o => !o)}
                      summary={
                        <span className="flex items-center gap-1.5 flex-wrap">
                          <span className={`text-micro px-1.5 py-px rounded font-semibold ${
                            marketReady
                              ? "bg-emerald-500/15 text-emerald-300"
                              : "bg-slate-500/15 text-slate-400"
                          }`}>
                            {marketReady ? t("toollaunch.pluginMarketplaceInstalled") : t("toollaunch.pluginMarketplaceNotInstalled")}
                          </span>
                          {marketReady && (
                            <span className="text-micro text-slate-500">
                              {marketKind === "claude"
                                ? `${t("toollaunch.pluginMarketplaceMarkets", { count: claudeStatus?.marketplaces.length ?? 0 })} · ${t("toollaunch.pluginMarketplaceCount", { count: claudePlugins.length })}`
                                : `${t("toollaunch.pluginMarketplaceCount", { count: marketplace?.pluginCount ?? 0 })} · ${t("toollaunch.pluginMarketplaceEnabled", { count: marketplace?.enabledCount ?? 0 })}`}
                            </span>
                          )}
                        </span>
                      }
                      action={
                        <>
                          <button onClick={() => void installMarketplace(selectedTool)} disabled={marketplaceBusy}
                            className="px-2.5 py-1 rounded-md text-tiny bg-[var(--module-accent)]/20 hover:bg-[var(--module-accent)]/30 text-white cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
                            {marketplaceBusy
                              ? <><RefreshCw className="w-3 h-3 inline animate-spin" /> {t("toollaunch.pluginMarketplaceWorking")}</>
                              : marketKind === "claude"
                                ? (marketReady ? t("toollaunch.pluginMarketplaceAddAnother") : t("toollaunch.pluginMarketplaceAddOfficial"))
                                : (marketplace?.installed ? t("toollaunch.pluginMarketplaceUpdate") : t("toollaunch.pluginMarketplaceInstall"))}
                          </button>
                          {marketReady && marketKind === "codex" && (
                            <button onClick={() => void removeMarketplace(selectedTool)} disabled={marketplaceBusy}
                              className="px-2.5 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed transition-colors">
                              {t("toollaunch.pluginMarketplaceRemove")}
                            </button>
                          )}
                        </>
                      }
                    >
                    {renderMarketplaceView()}
                    </CollapsibleCard>
                  </div>
                )}

                {/* 缓存路径（当前工具） */}
                <div>
                  <button
                    onClick={async () => {
                      if (!showCacheManager) { await loadCacheInfos(); }
                      setShowCacheManager(!showCacheManager);
                    }}
                    className="w-full flex items-center justify-between px-3 py-2 rounded-ctl bg-slate-900/30 border border-white/5 text-tiny text-slate-400 hover:text-slate-200 cursor-pointer transition-all"
                  >
                    <div className="flex items-center gap-2">
                      <HardDrive className="w-3.5 h-3.5" />
                      <span className="font-semibold">{t("toollaunch.cacheMgr")}</span>
                      {selectedToolCaches.length > 0 && (
                        <span className="text-[8px] text-slate-500">{t("toollaunch.cacheDirs", { count: selectedToolCaches.length })}</span>
                      )}
                    </div>
                    <ChevronDown className={`w-3.5 h-3.5 transition-transform ${showCacheManager ? "rotate-180" : ""}`} />
                  </button>

                  {showCacheManager && (
                    <ToolCacheDirectoryList
                      cacheInfos={cacheInfos}
                      selectedToolCaches={selectedToolCaches}
                      handleOpenCacheDir={handleOpenCacheDir}
                      handleMigrateCache={handleMigrateCache}
                      migratingCache={migratingCache}
                      handleCleanCache={handleCleanCache}
                      cleaningCache={cleaningCache}
                    />
                  )}
                </div>

                {renderModelOptions()}

                {/* 会话 */}
                {renderSessionPicker()}

                {/* 项目目录（桌面应用与项目目录无关，后端会用 exe 所在目录当工作目录） */}
                {sessionMode === "new" && selectedTool.tool_kind !== "desktop" && (
                  <div>
                    <label className="text-body font-bold text-slate-300 mb-2 block">{t("toollaunch.projectDir")}</label>
                    <div className="flex gap-2">
                      <input value={projectPath} onChange={e => setProjectPath(e.target.value)} placeholder={t("toollaunch.projectDirPh")}
                        className="flex-1 ui-input rounded-ctl px-3 py-2 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                      <button onClick={handleBrowse}
                        className="px-3 py-2 rounded-ctl bg-white/5 border border-white/10 text-slate-400 hover:text-white hover:bg-white/10 cursor-pointer transition-all">
                        <FolderOpen className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                )}

                {/* 终端（桌面应用直接拉起 GUI 进程，不经过终端包装） */}
                {terminals.length > 0 && selectedTool.tool_kind !== "desktop" && (
                  <div>
                    <label className="text-body font-bold text-slate-300 mb-2 block">{t("toollaunch.terminal")}</label>
                    <select value={selectedTerminal} onChange={e => setSelectedTerminal(e.target.value)}
                      className="w-full ui-input rounded-ctl px-3 py-2 text-body text-slate-200 focus:outline-none focus:border-[var(--module-accent)]">
                      {terminals.map(t => <option key={t.id} value={t.id}>{t.name}</option>)}
                    </select>
                  </div>
                )}

                {/* 代理启动信息条（入站→出站 / 统计 / 伪装） */}
                {(() => {
                  const proxyInfo = getProxyInfo(
                    selectedTool,
                    selectedModelProvider ? config?.providers.find(p => p.id === selectedModelProvider) ?? null : null,
                    useOfficialModel,
                    selectedModel,
                    // 用后端解析出的生效别名，不是手填的 masqueradeModel：
                    // Claude Desktop 留空时后端会自动给别名，这里要显示的就是那个
                    effectiveAlias,
                    selectedFallbackModel,
                    fallbackMasqueradeModel,
                  );
                  if (!proxyInfo) return null;
                  return (
                    <div className="p-2.5 rounded-ctl bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] text-tiny flex flex-col gap-1.5">
                      <div className="flex items-center gap-2 flex-wrap">
                        <Shield className="w-3.5 h-3.5 text-[var(--module-accent)] flex-shrink-0" />
                        <span className="text-slate-300">
                          {t("toollaunch.inbound")} <span className="font-semibold text-[var(--module-accent)]">{proxyInfo.inbound === "none" ? t("toollaunch.modelNone") : PROTOCOL_LABELS[proxyInfo.inbound]}</span>
                          <span className="mx-1 text-slate-500">→</span>
                          {t("toollaunch.outbound")} <span className="font-semibold text-[var(--module-accent)]">{proxyInfo.outbound === "none" ? t("toollaunch.modelNone") : PROTOCOL_LABELS[proxyInfo.outbound]}</span>
                        </span>
                        {proxyInfo.converted ? (
                          <span className="px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-300 text-micro font-semibold">{t("toollaunch.autoConvert")}</span>
                        ) : (
                          <span className="px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-300 text-micro font-semibold">{t("toollaunch.sameProtocol")}</span>
                        )}
                        <span className="px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-300 text-micro font-semibold">{t("toollaunch.statsOn")}</span>
                        {usingThirdPartyModel && selectedTool.supports_optimizer && optimizerEnabled && config?.optimizer.enabled && (
                          <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] text-micro font-semibold">{t("toollaunch.optimizerBadge")}</span>
                        )}
                        {usingThirdPartyModel && selectedTool.supports_rectifier && rectifierEnabled && config?.rectifier.enabled && (
                          <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] text-micro font-semibold">{t("toollaunch.rectifierBadge")}</span>
                        )}
                      </div>
                      {/* 模型 / 伪装：常驻显示。没配伪装时也要说清楚「未设置」——
                          之前这一行只在有映射时才渲染，于是最常见的场景下底部什么都不显示。 */}
                      <div className="flex items-center gap-1.5 flex-wrap text-slate-400">
                        <span className="text-slate-500">{t("toollaunch.modelLabel")}</span>
                        <span className="font-mono text-micro bg-slate-700/40 px-1.5 py-0.5 rounded text-slate-200">
                          {proxyInfo.model || "—"}
                        </span>
                        <span className="text-slate-500 ml-1.5">{t("toollaunch.masqueradeLabel2")}</span>
                        {proxyInfo.alias ? (
                          <span className="font-mono text-micro bg-slate-700/40 px-1.5 py-0.5 rounded">
                            {proxyInfo.alias}
                          </span>
                        ) : (
                          <span className="text-micro text-slate-500">{t("toollaunch.masqueradeNone")}</span>
                        )}
                        {proxyInfo.fallbackAliases.map(([k, v]) => (
                          <span key={k} className="font-mono text-micro bg-slate-700/40 px-1.5 py-0.5 rounded">
                            {k} → {v}
                          </span>
                        ))}
                      </div>
                    </div>
                  );
                })()}

                {/* 启动按钮 */}
                <button onClick={handleLaunch} disabled={launching || !canLaunch}
                  className="w-full py-3 rounded-card bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-bold flex items-center justify-center gap-2 cursor-pointer transition-all shadow-lg shadow-[var(--module-accent-ring)]">
                  {launching ? (
                    <><RefreshCw className="w-4 h-4 animate-spin" /> {t("toollaunch.starting")}</>
                  ) : sessionMode === "resume" && selectedSession ? (
                    <><Play className="w-4 h-4" /> {t("toollaunch.restoreSession")}</>
                  ) : (
                    <><Rocket className="w-4 h-4" /> {t("toollaunch.launch", { name: selectedTool.display_name })}</>
                  )}
                </button>
              </>
            )}

            {/* 安装 / 卸载 / 升级的实时输出：命令跑起来后逐行滚动。
                以前只有一句「安装中...」+ 转圈图标，用户无法判断是卡住了还是在下载；
                现在 npm/pip 的每一行输出都看得见，结束时上面再给结果卡片。 */}
            {getBusy(selectedTool.id) && opLogs[selectedTool.id] && (
              <div className="rounded-card border border-white/10 bg-black/40 p-2.5">
                <div className="mb-1.5 flex items-center gap-1.5 text-micro font-semibold text-slate-400">
                  <Terminal className="w-3 h-3" />
                  {t("toollaunch.opProgress")}
                  <span className="ml-auto text-slate-600">{opLogs[selectedTool.id].lines.length}</span>
                </div>
                <div ref={opLogRef} className="max-h-32 overflow-y-auto whitespace-pre-wrap break-all font-mono text-tiny leading-relaxed text-slate-300">
                  {opLogs[selectedTool.id].lines.length === 0
                    ? <div className="text-slate-600">{t("toollaunch.opWaiting")}</div>
                    : opLogs[selectedTool.id].lines.slice(-20).map((line, i) => <div key={i}>{line}</div>)}
                </div>
              </div>
            )}

            {/* 结果提示统一走共用组件：四处结果块原本逐字重复，改样式要改四遍 */}
            {launchResult && (
              <ResultNote ok={launchResult.ok} message={launchResult.msg} />
            )}

            {upgradeResult && upgradeResult.id === selectedTool?.id && (
              <ResultNote ok={upgradeResult.ok} message={upgradeResult.message} />
            )}

            {installResult && installResult.id === selectedTool?.id && (
              <ResultNote ok={installResult.ok} message={installResult.message} />
            )}

            {uninstallResult && uninstallResult.id === selectedTool?.id && (
              <ResultNote ok={uninstallResult.ok} message={uninstallResult.message} />
            )}
          </>
        )}
      </div>

      {/* 卸载确认（含「是否同时删除数据目录」）：替代原生 window.confirm */}
      <ConfirmDialogHost request={confirmRequest} onClose={() => setConfirmRequest(null)} />
    </div>
  );
}

export { isStrictMasqueradeTool, getProxyInfo, type AppliedModels, resolveAppliedModelRef } from "./ToolLauncher.model";
