import React, { Fragment, useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { Note, ResultNote } from "../shared/Note";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Rocket,
  FolderOpen,
  CheckCircle,
  RefreshCw,

  Bot,
  Clock,
  Play,

  Copy,
  ArrowUpCircle,
  ExternalLink,
  HardDrive,
  Trash2,
  FolderSync,
  ChevronDown,
  List,
  ListTree,
  Search,
  X,
  ChevronRight,
  Folder,
  ToggleLeft,
  ToggleRight,
  Download,
  Shield,
  Cpu, Pencil, Check, History, Terminal,
} from "lucide-react";
import type {
  AiProvider,
  AiConfig,
  LastLaunchConfig,
  DetectedAiTool,
  AiToolCacheInfo,
  ToolSession,
  TerminalInfo,
  ModelCustomParam,
  ModelEntry,
  ToolOpResult,
} from "./types";

const PROTOCOL_LABELS: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  both: "OpenAI + Anthropic",
  google: "Google",
  none: "", // 渲染时用 toollaunch.modelNone 翻译
};

/// 由供应商已配置的协议 URL 推导出站协议：若供应商支持工具原生协议则同协议直连，
/// 否则取供应商首个支持的协议（由代理做协议转换）。
function getOutboundProtocol(tool: DetectedAiTool | null, provider: AiProvider | null): string {
  if (!tool || !provider) return "openai";
  const inbound = tool.supports_anthropic ? "anthropic" : tool.supports_google ? "google" : "openai";
  const supported: string[] = [];
  if (provider.openai_url) supported.push("openai");
  if (provider.anthropic_url) supported.push("anthropic");
  if (provider.google_url) supported.push("google");
  if (supported.includes(inbound)) return inbound;
  return supported[0] || "openai";
}

/// 格式化相对时间（如 "3小时前", "昨天", "2天前"）
function formatRelativeTime(isoString: string, t: (k: string, o?: any) => string): string {
  try {
    const date = new Date(isoString);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMin = Math.floor(diffMs / 60000);
    const diffHour = Math.floor(diffMs / 3600000);
    const diffDay = Math.floor(diffMs / 86400000);
    if (diffMin < 1) return t("toollaunch.justNow");
    if (diffMin < 60) return t("toollaunch.minAgo", { count: diffMin });
    if (diffHour < 24) return t("toollaunch.hourAgo", { count: diffHour });
    if (diffDay === 1) return t("toollaunch.yesterday");
    if (diffDay < 7) return t("toollaunch.dayAgo", { count: diffDay });
    return date.toLocaleDateString("zh-CN", { month: "short", day: "numeric" });
  } catch {
    return "";
  }
}

/// 渲染供应商已配置协议的徽标（与模型配置页一致）
function providerProtocolBadges(p: AiProvider | null | undefined) {
  if (!p) return null;
  const items: { key: string; label: string; cls: string }[] = [];
  if (p.openai_url) items.push({ key: "openai", label: "OpenAI", cls: "bg-blue-500/20 text-blue-300" });
  if (p.anthropic_url) items.push({ key: "anthropic", label: "Anthropic", cls: "bg-amber-500/20 text-amber-300" });
  if (p.google_url) items.push({ key: "google", label: "Google", cls: "bg-green-500/20 text-green-300" });
  return items.map(i => (
    <span key={i.key} className={`text-[8px] text-slate-600 px-1.5 py-0.5 rounded ${i.cls}`}>{i.label}</span>
  ));
}

/// 计算代理启动信息条所需数据（与后端 launch.rs 逻辑对齐）。
/// 无 Provider / 官方模式 / 不支持模型 时不启动代理，返回 null。
function getProxyInfo(
  tool: DetectedAiTool | null,
  provider: AiProvider | null,
  useOfficial: boolean,
  selectedModel: string,
  masqueradeModel: string,
  fallbackModel: string,
  fallbackMasqueradeModel: string,
): {
  inbound: string;
  outbound: string;
  converted: boolean;
  aliasEntries: [string, string][];
} | null {
  if (!tool || !tool.installed || !tool.supports_model || useOfficial || !provider) {
    return null;
  }
  // 入站协议：工具支持的协议（anthropic 优先，其次 google，否则 openai）
  const inbound = tool.supports_anthropic
    ? "anthropic"
    : tool.supports_google
    ? "google"
    : "openai";
  // 出站协议：根据供应商已配置的协议 URL 推导（同协议优先，否则转换）
  const outbound = getOutboundProtocol(tool, provider);
  // 伪装映射 C → B（主模型 + fallback 小模型）
  const aliasEntries: [string, string][] = [];
  if (masqueradeModel) aliasEntries.push([masqueradeModel, selectedModel || ""]);
  if (fallbackModel) {
    const claimedFb = fallbackMasqueradeModel || fallbackModel;
    aliasEntries.push([claimedFb, fallbackModel]);
  }
  return {
    inbound,
    outbound,
    converted: inbound !== outbound,
    aliasEntries,
  };
}

/** 未安装区的形态筛选：全部 / CLI / 桌面端（抄 EchoBird 的分组维度）。
 *  已安装区不分组——装了的就那么几个，分组只会让人多找一层。 */
type ToolKindFilter = "all" | "cli" | "desktop";

export default function ToolLauncher({ onAskAssistant }: { onAskAssistant?: (question: string) => void } = {}) {
  const { t } = useTranslation();
  const [tools, setTools] = useState<DetectedAiTool[]>([]);
  const [kindFilter, setKindFilter] = useState<ToolKindFilter>("all");
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const [sessions, setSessions] = useState<ToolSession[]>([]);
  const [loading, setLoading] = useState(true);

  const [selectedToolId, setSelectedToolId] = useState<string | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [selectedModelProvider, setSelectedModelProvider] = useState("");
  // 当前模型自定义启动参数的取值（param key → 用户选的值）
  const [customParamValues, setCustomParamValues] = useState<Record<string, string>>({});
  const [projectPath, setProjectPath] = useState("");
  const [selectedTerminal, setSelectedTerminal] = useState("cmd");
  const [sessionMode, setSessionMode] = useState<"new" | "continue" | "resume">("new");
  const [selectedSession, setSelectedSession] = useState<ToolSession | null>(null);
  const [showSessionPicker, setShowSessionPicker] = useState(false);

  const [sessionViewMode, setSessionViewMode] = useState<"flat" | "grouped">("grouped");
  const [sessionSearch, setSessionSearch] = useState("");
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedSessionIds, setSelectedSessionIds] = useState<Set<string>>(new Set());
  const [expandedDirs, setExpandedDirs] = useState<Set<string>>(new Set());

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
    thinking_signature: true, thinking_budget: true, media_fallback: true, protocol_mismatch: true,
  });
  const [optimizerStrategies, setOptimizerStrategies] = useState({
    cache_injection: true, thinking_optimizer: true, deepseek_normalize: true,
  });
  // Codex web_search 开关：开启 → 写 config.toml `web_search = "live"`（真实实时检索）
  const [webSearchEnabled, setWebSearchEnabled] = useState(false);

  const [launching, setLaunching] = useState(false);
  const [launchResult, setLaunchResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const [upgradingTool, setUpgradingTool] = useState<string | null>(null);
  const [upgradeResult, setUpgradeResult] = useState<{ id: string; ok: boolean; message: string } | null>(null);
  const [installingTool, setInstallingTool] = useState<string | null>(null);
  const [installResult, setInstallResult] = useState<{ id: string; ok: boolean; message: string } | null>(null);
  const [uninstallingTool, setUninstallingTool] = useState<string | null>(null);
  const [uninstallResult, setUninstallResult] = useState<{ id: string; ok: boolean; message: string } | null>(null);
  // 安装/卸载/升级的实时输出（后端 ai-tool-progress 事件逐行推送，按工具 id 分开存）
  const [opLogs, setOpLogs] = useState<Record<string, { phase: string; lines: string[] }>>({});
  const opLogRef = useRef<HTMLDivElement | null>(null);
  const [versionStatuses, setVersionStatuses] = useState<Record<string, { latest: string; status: string; busy?: string | null }>>({});
  const [checkingVersions, setCheckingVersions] = useState(false);

  // 双模型（高级 + fallback 低级）
  const [selectedFallbackModel, setSelectedFallbackModel] = useState("");
  const [selectedFallbackProvider, setSelectedFallbackProvider] = useState("");
  // fallback 模型的伪装声明名（"" 表示不伪装，直接使用所选取的供应商模型）
  const [fallbackMasqueradeModel, setFallbackMasqueradeModel] = useState("");
  // 官方模型选择（对于 api_protocol="none" 或用户主动选择官方模型）
  const [useOfficialModel, setUseOfficialModel] = useState(false);

  // 缓存管理
  const [cacheInfos, setCacheInfos] = useState<AiToolCacheInfo[]>([]);
  const [showCacheManager, setShowCacheManager] = useState(false);
  const [migratingCache, setMigratingCache] = useState<string | null>(null);
  const [cleaningCache, setCleaningCache] = useState<string | null>(null);

  // 各工具的上次启动方式记录
  const [lastLaunchConfigs, setLastLaunchConfigs] = useState<Record<string, LastLaunchConfig>>({});

  const selectedTool = tools.find(t => t.id === selectedToolId) || null;

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

  // 缓存当前选中工具的缓存信息（避免重复 filter）
  const selectedToolCaches = React.useMemo(() => {
    if (!selectedToolId) return [];
    return cacheInfos.filter(c => c.tool_id === selectedToolId);
  }, [cacheInfos, selectedToolId]);

  // 检测工具版本（使用后端 check_all_tool_versions + check_ai_tool_versions）
  const checkVersions = useCallback(async () => {
    setCheckingVersions(true);
    try {
      const [regResults, aiResults] = await Promise.all([
        invoke<Array<{ project_id: string; current_version: string | null; latest_version: string | null; status: string }>>("check_all_tool_versions"),
        invoke<Array<{ tool_id: string; current_version: string | null; latest_version: string | null; status: string; busy?: string | null }>>("check_ai_tool_versions"),
      ]);
      const map: Record<string, { latest: string; status: string; busy?: string | null }> = {};
      for (const r of regResults) {
        map[r.project_id] = { latest: r.latest_version || "", status: r.status };
      }
      for (const r of aiResults) {
        map[r.tool_id] = { latest: r.latest_version || "", status: r.status, busy: r.busy ?? null };
      }
      setVersionStatuses(map);
    } catch { /* ignore */ }
    finally { setCheckingVersions(false); }
  }, []);

  useEffect(() => {
    if (tools.length > 0) checkVersions();
  }, [tools, checkVersions]);

  const loadData = useCallback(async () => {
    try {
      const [t, c, term, lcs] = await Promise.all([
        invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []),
        invoke<AiConfig>("get_ai_config").catch(() => ({ providers: [], proxy_port: 15721, default_project_path: "", rectifier: { enabled: false, thinking_signature: false, thinking_budget: false, media_fallback: false, protocol_mismatch: false }, headroom: { enabled: false, port: 8791, on_unavailable: "failOpen", disable_kompress: false, timeout_ms: 1500 }, optimizer: { enabled: false, cache_injection: false, thinking_optimizer: false, deepseek_normalize: false }, skills_dir: "" })),
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
        protocol_mismatch: c.rectifier?.protocol_mismatch !== false,
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

  // 协同身份（头像/昵称）编辑，维护在 AI 工具页，供协同对话使用
  const [editProfile, setEditProfile] = useState(false);
  const [pAvatar, setPAvatar] = useState("");
  const [pNick, setPNick] = useState("");
  const [profileMsg, setProfileMsg] = useState<{ ok: boolean; msg: string } | null>(null);

  const openProfile = () => {
    if (!selectedTool) return;
    setPAvatar(selectedTool.avatar || "");
    setPNick(selectedTool.nickname || "");
    setProfileMsg(null);
    setEditProfile(true);
  };
  const saveProfile = async () => {
    if (!selectedTool) return;
    setProfileMsg(null);
    try {
      await invoke("update_tool_profile", {
        toolId: selectedTool.id,
        avatar: pAvatar.trim() || null,
        nickname: pNick.trim() || null,
      });
      setProfileMsg({ ok: true, msg: t("toollaunch.profileSaved") });
      setEditProfile(false);
      await reloadTools();
    } catch (e) {
      setProfileMsg({ ok: false, msg: String(e) });
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

  // 安装/卸载/升级的实时输出：后端把 npm/pip 的每一行输出推来，
  // 逐行显示（保留最近 60 行）—— 否则用户只能盯着一个转圈图标猜命令跑到哪了。
  useEffect(() => {
    const unlisten = listen<{ toolId: string; phase: string; line: string }>("ai-tool-progress", (event) => {
      const { toolId, phase, line } = event.payload;
      setOpLogs((prev) => {
        const cur = prev[toolId];
        // 阶段变了（如先卸载后重装）就另起一段，不要把两次输出混在一起
        const lines = cur && cur.phase === phase ? [...cur.lines, line] : [line];
        return { ...prev, [toolId]: { phase, lines: lines.slice(-60) } };
      });
    });
    return () => { unlisten.then(fn => fn()); };
  }, []);

  // 输出追加后自动滚到底部（日志类 UI 的基本预期）
  useEffect(() => {
    const el = opLogRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [opLogs]);

  useEffect(() => {
    if (!selectedTool?.installed) { setSessions([]); return; }
    invoke<ToolSession[]>("scan_tool_sessions", { toolId: selectedTool.id })
      .then(setSessions).catch(() => setSessions([]));
  }, [selectedTool]);

  // 切换工具时回显「它配置文件里现在写的是哪个模型」（未声明 configFile 的工具清空）
  useEffect(() => {
    setApplyModelMsg(null);
    if (selectedTool?.config_file) void loadAppliedModel(selectedTool.id);
    else setAppliedModel(null);
  }, [selectedTool?.id, selectedTool?.config_file]);

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

  // 全部模型（含任意供应商），用于 Fallback 选择 — 按供应商分组
  const fallbackGroups = React.useMemo(() => {
    if (!config || !selectedTool) return [];
    if (!selectedTool.supports_fallback_model) return [];
    const groups: { provider_name: string; provider_id: string; models: ModelEntry[] }[] = [];
    for (const p of config.providers) {
      if (p.models.length === 0) continue;
      const filteredModels = selectedModel ? p.models.filter(m => m.id !== selectedModel) : p.models;
      if (filteredModels.length === 0) continue;
      groups.push({ provider_name: p.name, provider_id: p.id, models: filteredModels });
    }
    return groups;
  }, [config, selectedTool, selectedModel]);

  // fallback 的折叠状态
  const [expandedFallbackGroups, setExpandedFallbackGroups] = useState<Set<string>>(new Set());

  // 模型供应商折叠状态
  const [expandedModelGroups, setExpandedModelGroups] = useState<Set<string>>(new Set());

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
      const result = await invoke<{ success: boolean; message: string }>("launch_ai_tool", {
        req: {
          tool_id: selectedTool.id,
          project_path: sessionMode === "resume" && selectedSession ? selectedSession.project_path : projectPath,
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
          rectifier_protocol_mismatch: useOfficialModel ? null : rectifierStrategies.protocol_mismatch,
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
          provider_id: useOfficialModel ? null : (selectedModelProvider || null),
          provider_name: providerName,
          model_id: useOfficialModel ? null : (selectedModel || null),
          fallback_model_id: useOfficialModel ? null : (selectedFallbackModel || null),
          fallback_provider_id: useOfficialModel ? null : (selectedFallbackProvider || null),
          fallback_masquerade_model: useOfficialModel ? null : (fallbackMasqueradeModel || null),
          use_official_model: useOfficialModel,
          terminal_id: selectedTerminal,
          one_m_context: selectedTool.support_one_m_context ? oneMContext : false,
          fallback_one_m_context: selectedTool.support_one_m_context ? (selectedFallbackModel ? fallbackOneMContext : false) : false,
          masquerade_model: useOfficialModel ? null : (masqueradeModel || null),
          optimizer_enabled: useOfficialModel ? null : optimizerEnabled,
          rectifier_enabled: useOfficialModel ? null : rectifierEnabled,
          custom_param_values: useOfficialModel ? {} : customParamValues,
          project_path: sessionMode === "resume" && selectedSession ? selectedSession.project_path : projectPath,
          last_launched_at: new Date().toISOString(),
        };
        await invoke("save_last_launch_config", { toolId: selectedTool.id, config: lc }).catch(() => {});
        setLastLaunchConfigs(prev => ({ ...prev, [selectedTool.id]: lc }));
      }
    } catch (e: any) {
      setLaunchResult({ ok: false, msg: String(e) });
    } finally { setLaunching(false); }
  };

  /** 清空某工具的实时输出（操作开始时调用，避免上一轮的残留混进这一轮） */
  const startOpLog = (toolId: string, phase: string) => {
    setOpLogs((prev) => ({ ...prev, [toolId]: { phase, lines: [] } }));
  };

  const handleUpgrade = async (tool: DetectedAiTool) => {
    setUpgradingTool(tool.id);
    setUpgradeResult(null);
    startOpLog(tool.id, "upgrading");
    try {
      const res = await invoke<ToolOpResult>("upgrade_ai_tool", { toolId: tool.id });
      setUpgradeResult({ id: tool.id, ...res });
      const t = await invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []);
      setTools(t);
      await checkVersions();
    } catch (e: any) {
      setUpgradeResult({ id: tool.id, ok: false, message: String(e) });
    } finally { setUpgradingTool(null); }
  };

  const handleInstall = async (tool: DetectedAiTool) => {
    if (!tool.install_cmd) return;
    setInstallingTool(tool.id);
    setInstallResult(null);
    startOpLog(tool.id, "installing");
    try {
      const res = await invoke<ToolOpResult>("install_ai_tool", { toolId: tool.id });
      setInstallResult({ id: tool.id, ...res });
      const t = await invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []);
      setTools(t);
      await checkVersions();
    } catch (e: any) {
      setInstallResult({ id: tool.id, ok: false, message: String(e) });
    } finally { setInstallingTool(null); }
  };

  // 卸载确认弹窗：原生 confirm 说不清「会不会连数据一起删」，换成能逐条看清影响面的确认框
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [removeDataDirs, setRemoveDataDirs] = useState(false);
  // 勾选值同时写 ref：确认回调是弹窗创建时就捕获的闭包，直接读 state 会拿到旧值
  const removeDataDirsRef = useRef(false);
  const toggleRemoveDataDirs = (value: boolean) => {
    removeDataDirsRef.current = value;
    setRemoveDataDirs(value);
  };

  /** 打开卸载确认：先刷新数据目录清单，再把该工具实际占用的目录逐条列出来 */
  // 「只设置模型、不启动工具」：写进工具自己的配置文件
  const [applyModelBusy, setApplyModelBusy] = useState(false);
  const [appliedModel, setAppliedModel] = useState<string | null>(null);
  const [applyModelMsg, setApplyModelMsg] = useState<{ ok: boolean; text: string } | null>(null);

  /** 读回工具配置文件里当前写定的模型（切换工具时刷新一次） */
  const loadAppliedModel = async (toolId: string) => {
    try {
      const current = await invoke<string | null>("get_ai_tool_model", { toolId });
      setAppliedModel(current ?? null);
    } catch {
      setAppliedModel(null);
    }
  };

  const applyModelOnly = async () => {
    if (!selectedTool || !selectedModel || !selectedModelProvider) return;
    setApplyModelBusy(true);
    setApplyModelMsg(null);
    try {
      const msg = await invoke<string>("set_ai_tool_model", {
        toolId: selectedTool.id,
        providerId: selectedModelProvider,
        modelId: selectedModel,
        fallbackModelId: selectedFallbackModel || null,
        masqueradeModel: masqueradeModel || null,
        oneMContext: selectedTool.support_one_m_context ? oneMContext : false,
        webSearch: webSearchEnabled,
      });
      setApplyModelMsg({ ok: true, text: msg });
      await loadAppliedModel(selectedTool.id);
    } catch (e: any) {
      setApplyModelMsg({ ok: false, text: String(e) });
    } finally {
      setApplyModelBusy(false);
    }
  };

  const askUninstall = async (tool: DetectedAiTool) => {
    await loadCacheInfos();
    const dirs = cacheInfos.filter((c) => c.tool_id === tool.id && c.exists);
    toggleRemoveDataDirs(false);
    setConfirmRequest({
      title: t("toollaunch.uninstallConfirmTitle", { name: tool.display_name }),
      danger: true,
      width: 440,
      confirmText: t("toollaunch.uninstall"),
      desc: (
        <div className="space-y-2">
          <div>{t("toollaunch.uninstallConfirmDesc", { name: tool.display_name })}</div>
          {dirs.length > 0 ? (
            <>
              <div className="text-[10px] text-slate-400">{t("toollaunch.dataDirsHint")}</div>
              <div className="max-h-32 overflow-y-auto rounded-lg border border-white/10 bg-black/30 divide-y divide-white/5">
                {dirs.map((d) => (
                  <div key={d.dir_name} className="flex items-center gap-2 px-2 py-1.5 text-[10px]">
                    <FolderOpen className="w-3 h-3 flex-shrink-0 text-slate-500" />
                    <span className="min-w-0 flex-1 break-all text-slate-300">{d.full_path}</span>
                    <span className="flex-shrink-0 text-slate-500">{d.size}</span>
                  </div>
                ))}
              </div>
              <label className="flex items-start gap-2 text-[10px] text-slate-300 cursor-pointer select-none">
                <input
                  type="checkbox"
                  checked={removeDataDirs}
                  onChange={(e) => toggleRemoveDataDirs(e.target.checked)}
                  className="mt-0.5 cursor-pointer"
                />
                <span>{t("toollaunch.removeDataDirsCheckbox", { count: dirs.length })}</span>
              </label>
              <div className="text-[9px] text-slate-500">{t("toollaunch.removeDataDirsHint")}</div>
            </>
          ) : (
            <div className="text-[10px] text-slate-500">{t("toollaunch.noDataDirs")}</div>
          )}
        </div>
      ),
      onConfirm: () => void handleUninstall(tool, removeDataDirsRef.current),
    });
  };

  const handleUninstall = async (tool: DetectedAiTool, removeData: boolean) => {
    setUninstallingTool(tool.id);
    setUninstallResult(null);
    startOpLog(tool.id, "uninstalling");
    try {
      const res = await invoke<ToolOpResult>("uninstall_ai_tool", {
        toolId: tool.id,
        removeDataDirs: removeData,
      });
      setUninstallResult({ id: tool.id, ...res });
      const detected = await invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []);
      setTools(detected);
      await checkVersions();
      // 勾选删除时数据目录已进回收站，清单要跟着刷新
      if (removeData) void loadCacheInfos();
    } catch (e: any) {
      setUninstallResult({ id: tool.id, ok: false, message: String(e) });
    } finally { setUninstallingTool(null); }
  };

  const loadCacheInfos = useCallback(async () => {
    try {
      const infos = await invoke<AiToolCacheInfo[]>("get_ai_tool_cache_info");
      setCacheInfos(infos);
    } catch (e) { console.error(e); }
  }, []);

  const handleMigrateCache = async (toolId: string, dirName: string, _fullPath: string) => {
    try {
      const selected = await open({ directory: true, title: t("toollaunch.pickCacheDir") });
      if (!selected) return;
      setMigratingCache(`${toolId}:${dirName}`);
      await invoke("migrate_ai_tool_cache", { toolId, dirName, newPath: selected as string });
      await loadCacheInfos();
    } catch (e: any) { alert(t("toollaunch.migrateFail", { err: String(e) })); }
    finally { setMigratingCache(null); }
  };

  const handleCleanCache = async (toolId: string, dirName: string) => {
    if (!confirm(t("toollaunch.clearCacheConfirm", { name: dirName }))) return;
    setCleaningCache(`${toolId}:${dirName}`);
    try {
      await invoke("clean_ai_tool_cache", { toolId, dirName });
      await loadCacheInfos();
    } catch (e: any) { alert(t("toollaunch.clearFail", { err: String(e) })); }
    finally { setCleaningCache(null); }
  };

  const handleOpenCacheDir = async (fullPath: string) => {
    try { await invoke("open_ai_tool_cache_dir_path", { fullPath }); }
    catch (e) { console.error(e); }
  };

  // ── 会话分组 & 搜索 ──
  const filteredSessions = React.useMemo(() => {
    if (!sessionSearch.trim()) return sessions;
    const q = sessionSearch.toLowerCase();
    return sessions.filter(s =>
      s.project_path.toLowerCase().includes(q) ||
      (s.summary && s.summary.toLowerCase().includes(q)) ||
      s.session_id.toLowerCase().includes(q)
    );
  }, [sessions, sessionSearch]);

  const sessionDirGroups = React.useMemo(() => {
    const groups = new Map<string, { dir: string; label: string; sessions: ToolSession[] }>();
    for (const s of filteredSessions) {
      const dir = s.project_path || t("toollaunch.unknownDir");
      const label = dir.split(/[\\/]/).pop() || dir;
      if (!groups.has(dir)) groups.set(dir, { dir, label, sessions: [] });
      groups.get(dir)!.sessions.push(s);
    }
    return Array.from(groups.values()).sort((a, b) => a.label.localeCompare(b.label));
  }, [filteredSessions]);

  const handleDeleteSessions = async () => {
    if (selectedSessionIds.size === 0) return;
    if (!confirm(t("toollaunch.delSessionsConfirm", { count: selectedSessionIds.size }))) return;
    for (const sid of selectedSessionIds) {
      const s = sessions.find(x => x.session_id === sid);
      if (s) {
        try { await invoke("remove_ai_session", { toolId: selectedTool!.id, projectPath: s.project_path, sessionId: s.session_id }); }
        catch (e) { console.error(e); }
      }
    }
    setSelectedSessionIds(new Set());
    setSelectionMode(false);
    const updated = await invoke<ToolSession[]>("scan_tool_sessions", { toolId: selectedTool!.id }).catch(() => []);
    setSessions(updated);
  };

  const handleSelectAll = () => {
    if (selectedSessionIds.size === filteredSessions.length) setSelectedSessionIds(new Set());
    else setSelectedSessionIds(new Set(filteredSessions.map(s => s.session_id)));
  };

  const toggleSessionSelect = (sid: string) => {
    const next = new Set(selectedSessionIds);
    if (next.has(sid)) next.delete(sid); else next.add(sid);
    setSelectedSessionIds(next);
  };

  const toggleDirExpand = (dir: string) => {
    const next = new Set(expandedDirs);
    if (next.has(dir)) next.delete(dir); else next.add(dir);
    setExpandedDirs(next);
  };

  if (loading) {
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-xs">{t("toollaunch.loading")}</span></div>;
  }

  const getVerStatus = (toolId: string): { label: string; color: string; icon: React.ReactNode } | null => {
    const vs = versionStatuses[toolId];
    if (!vs) return null;
    switch (vs.status) {
      case "outdated": return { label: t("toollaunch.upgradable"), color: "text-amber-400", icon: <ArrowUpCircle className="w-2.5 h-2.5" /> };
      case "latest": return { label: t("toollaunch.latest"), color: "text-emerald-400", icon: <CheckCircle className="w-2.5 h-2.5" /> };
      case "unknown": return null;
      case "not_installed": return null;
      default: return null;
    }
  };

  // 合并“进行中”状态：优先取本地在途操作，其次取后端 detect/versions 返回的 busy 标记。
  // 这样即使切换 Agent、切换页面或组件重新挂载，仍能持续显示“升级中/安装中/卸载中”。
  const getBusy = (toolId: string): "upgrading" | "installing" | "uninstalling" | null => {
    if (upgradingTool === toolId) return "upgrading";
    if (installingTool === toolId) return "installing";
    if (uninstallingTool === toolId) return "uninstalling";
    const t = tools.find((x) => x.id === toolId);
    if (t && t.busy) return t.busy as "upgrading" | "installing" | "uninstalling";
    const vs = versionStatuses[toolId];
    if (vs && vs.busy) return vs.busy as "upgrading" | "installing" | "uninstalling";
    return null;
  };

  const canLaunch = selectedTool?.installed && (sessionMode === "resume" || projectPath);

  // 列表分组（抄 EchoBird 的维度：先按已装/未装分开，未装的再按 CLI / 桌面端筛）
  const installedTools = tools.filter(t => t.installed);
  const notInstalledTools = tools.filter(t => !t.installed);
  const visibleNotInstalled = kindFilter === "all"
    ? notInstalledTools
    : notInstalledTools.filter(t => (t.tool_kind ?? "other") === kindFilter);
  const visibleTools = [...installedTools, ...visibleNotInstalled];

  return (
    <div className="h-full flex min-h-0 select-none">
      {/* ── 左侧工具列表 ── */}
      <div className="w-52 flex-shrink-0 border-r border-white/5 py-3 px-2 overflow-y-auto space-y-0.5 flex flex-col">
        <div className="flex items-center justify-between px-1 mb-1">
          <span className="text-[9px] font-bold text-slate-500 uppercase">{t("toollaunch.aiTools")}</span>
          {/* 形态筛选：只作用于「未安装」那一组（已装的就几个，再分组只会多找一层） */}
          {notInstalledTools.length > 0 && (
            <div className="flex items-center gap-0.5">
              {(["all", "cli", "desktop"] as ToolKindFilter[]).map(k => (
                <button
                  key={k}
                  onClick={() => setKindFilter(k)}
                  className={`px-1 py-0.5 rounded text-[8px] cursor-pointer transition-all ${
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
            className="p-0.5 rounded text-slate-600 hover:text-slate-400 cursor-pointer"
            title={t("toollaunch.checkVersion")}>
            <RefreshCw className={`w-3 h-3 ${checkingVersions ? "animate-spin" : ""}`} />
          </button>
        </div>
        {visibleTools.map((tool, index) => {
          const vs = getVerStatus(tool.id);
          return (
            <Fragment key={tool.id}>
            {/* 分组标题：已安装区在前、未安装区在后，各一个抬头 */}
            {index === 0 && installedTools.length > 0 && (
              <div className="px-1 pt-1 pb-0.5 text-[9px] font-bold text-slate-600 uppercase">
                {t("toollaunch.installedGroup")}（{installedTools.length}）
              </div>
            )}
            {index === installedTools.length && visibleNotInstalled.length > 0 && (
              <div className="px-1 pt-2 pb-0.5 text-[9px] font-bold text-slate-600 uppercase">
                {t("toollaunch.notInstalledGroup")}（{visibleNotInstalled.length}）
              </div>
            )}
            <button
              onClick={async () => {
                setSelectedToolId(tool.id);
                // 重置默认值
                setSelectedModel("");
                setSelectedModelProvider("");
                setSelectedFallbackModel("");
                setSelectedFallbackProvider("");
                setFallbackMasqueradeModel("");
                setExpandedModelGroups(new Set());
                setExpandedFallbackGroups(new Set());
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
                  protocol_mismatch: config?.rectifier?.protocol_mismatch !== false,
                });
                setSelectedTerminal("cmd");
                setUseOfficialModel(tool.api_protocol === "none");
                // 加载上次启动配置并恢复 UI 状态
                try {
                  const last = await invoke<LastLaunchConfig | null>("get_last_launch_config", { toolId: tool.id });
                  if (last) {
                    setLastLaunchConfigs(prev => ({ ...prev, [tool.id]: last }));
                    if (last.use_official_model) {
                      setUseOfficialModel(true);
                    } else {
                      // 先设置 provider，触发模型列表更新
                      if (last.provider_id) {
                        setSelectedModelProvider(last.provider_id);
                      }
                      // 再设置 model（React 会批量更新，下次渲染时模型列表已更新）
                      if (last.model_id) setSelectedModel(last.model_id);
                      if (last.custom_param_values) setCustomParamValues(last.custom_param_values);
                      if (last.fallback_model_id) setSelectedFallbackModel(last.fallback_model_id);
                      if (last.fallback_provider_id) setSelectedFallbackProvider(last.fallback_provider_id);
                      if (last.fallback_masquerade_model) setFallbackMasqueradeModel(last.fallback_masquerade_model);
                    }
                    if (last.terminal_id && last.terminal_id !== "cmd") setSelectedTerminal(last.terminal_id);
                    if (last.one_m_context) setOneMContext(true);
                    if (last.fallback_one_m_context) setFallbackOneMContext(true);
                    if (last.masquerade_model) setMasqueradeModel(last.masquerade_model);
                    if (last.optimizer_enabled !== null && last.optimizer_enabled !== undefined) setOptimizerEnabled(last.optimizer_enabled);
                    if (last.rectifier_enabled !== null && last.rectifier_enabled !== undefined) setRectifierEnabled(last.rectifier_enabled);
                    if (last.project_path) setProjectPath(last.project_path);
                  }
                } catch { /* 无历史记录 */
                }
              }}
              className={`w-full px-3 py-2.5 rounded-lg text-left transition-all cursor-pointer ${
                selectedToolId === tool.id
                  ? "bg-[var(--module-accent)] text-white shadow-md shadow-[var(--module-accent-ring)]"
                  : tool.installed
                    ? "text-slate-300 hover:text-white hover:bg-white/5"
                    : "text-slate-600 hover:text-slate-400 hover:bg-white/[0.03]"
              }`}
            >
              <div className="flex items-center gap-2">
                <span className="w-4 h-4 flex-shrink-0 flex items-center justify-center text-xs">{tool.avatar || '🤖'}</span>
                <div className="flex items-center gap-1 min-w-0 flex-1">
                  <span className="text-[11px] font-semibold truncate">{tool.nickname || tool.display_name}</span>
                  {/* 真实名称：选中态背景就是纯 accent，再叠 accent 半透明等于看不见，
                      一律用白色半透明（任何主题色下都清晰） */}
                  {tool.nickname && tool.nickname !== tool.display_name && (
                    <span className={`text-[9px] truncate flex-shrink-0 ${
                      selectedToolId === tool.id ? "text-white/70" : "text-slate-500"
                    }`}>
                      ({tool.display_name})
                    </span>
                  )}
                </div>
                {getBusy(tool.id) ? (
                  <span className="text-[9px] font-semibold flex items-center gap-0.5 ml-auto flex-shrink-0 text-blue-300">
                    <RefreshCw className="w-2.5 h-2.5 animate-spin" />
                    {getBusy(tool.id) === "upgrading" ? t("toollaunch.upgrading") : getBusy(tool.id) === "installing" ? t("toollaunch.installing") : t("toollaunch.uninstalling")}
                  </span>
                ) : vs && (
                  <span className={`text-[9px] font-semibold flex items-center gap-0.5 ml-auto flex-shrink-0 ${vs.color}`}>
                    {vs.icon}
                    {vs.label}
                  </span>
                )}
              </div>
              <div className="flex items-center gap-1.5 mt-0.5 ml-5.5">
                {getBusy(tool.id) === "installing" ? (
                  <span className="text-[9px] text-blue-300 animate-pulse">{t("toollaunch.installing")}...</span>
                ) : getBusy(tool.id) === "upgrading" ? (
                  <span className="text-[9px] text-blue-300 animate-pulse">{t("toollaunch.upgrading")}...</span>
                ) : getBusy(tool.id) === "uninstalling" ? (
                  <span className="text-[9px] text-blue-300 animate-pulse">{t("toollaunch.uninstalling")}...</span>
                ) : tool.installed ? (
                  // 与真实名称同理：选中态底色就是 accent，文字不能再用 accent
                  <span className={`text-[9px] ${selectedToolId === tool.id ? "text-white/70" : "text-slate-500"} font-mono`}>
                    {tool.version || t("toollaunch.installed")}
                  </span>
                ) : (
                  <span className="flex items-center gap-1">
                    <span className="text-[9px] text-slate-600">{t("toollaunch.notInstalled")}</span>
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
                    {lastLaunchConfigs[tool.id].use_official_model ? (
                      <span className="text-[9px]">{t("toollaunch.official")}</span>
                    ) : (
                      <>
                        <span className="text-[9px] truncate max-w-[60px]">
                          {lastLaunchConfigs[tool.id].provider_name || lastLaunchConfigs[tool.id].provider_id || "-"}
                        </span>
                        {lastLaunchConfigs[tool.id].model_id && (
                          <span className="text-[9px] truncate max-w-[50px] opacity-70">
                            · {lastLaunchConfigs[tool.id].model_id}
                          </span>
                        )}
                        {lastLaunchConfigs[tool.id].fallback_model_id && (
                          <span className="text-[9px] text-amber-400/80 truncate max-w-[50px]">
                            ※ {lastLaunchConfigs[tool.id].fallback_model_id}
                          </span>
                        )}
                      </>
                    )}
                    {lastLaunchConfigs[tool.id].last_launched_at && (
                      <span className="text-[9px] opacity-50 ml-auto">
                        {formatRelativeTime(lastLaunchConfigs[tool.id].last_launched_at, t)}
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

      {/* ── 右侧设置面板 ── */}
      <div className="flex-1 min-h-0 overflow-y-auto p-6 space-y-4">
        {!selectedTool ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500">
            <Bot className="w-8 h-8 text-slate-700 mb-2" />
            <span className="text-xs font-bold text-slate-400">{t("toollaunch.selectToolHint")}</span>
          </div>
        ) : (
          <>
            {/* 工具信息 + 版本详情 */}
            <div className="p-3 rounded-xl bg-slate-900/30 border border-white/5">
              <div className="flex items-center gap-3">
                <div className="p-2 rounded-lg bg-[var(--module-accent-soft)]">
                  <Bot className="w-5 h-5 text-[var(--module-accent)]" />
                </div>
                <div>
                  <h3 className="text-sm font-bold text-white">{selectedTool.display_name}</h3>
                  <div className="flex items-center gap-2 mt-0.5">
                    {getBusy(selectedTool.id) && (
                      <span className="flex items-center gap-1 text-[10px] font-semibold text-blue-300">
                        <RefreshCw className="w-3 h-3 animate-spin" />
                        {getBusy(selectedTool.id) === "upgrading" ? `${t("toollaunch.upgrading")}...` : getBusy(selectedTool.id) === "installing" ? `${t("toollaunch.installing")}...` : `${t("toollaunch.uninstalling")}...`}
                      </span>
                    )}
                    {selectedTool.installed ? (
                      <>
                        <span className="text-[10px] text-emerald-400"><CheckCircle className="w-3 h-3 inline mr-0.5" />{selectedTool.version || t("toollaunch.installed")}</span>
                        {!selectedTool.pm_managed && (
                          // 只是提示，不拦操作：升级/卸载照旧可用（包管理器 → 官方渠道 → 按文件清理）
                          <span
                            className="text-[10px] text-amber-400/80 cursor-help"
                            title={selectedTool.detected_path || selectedTool.uninstall_cmd || selectedTool.upgrade_cmd || undefined}
                          >
                            {t("toollaunch.externalInstallHint")}
                          </span>
                        )}
                        {!getBusy(selectedTool.id) && versionStatuses[selectedTool.id]?.latest && versionStatuses[selectedTool.id]?.status === "outdated" && (
                          <>
                            <span className="text-[10px] text-amber-400 ml-1">→ {t("toollaunch.latest")}: {versionStatuses[selectedTool.id].latest}</span>
                            <button
                              onClick={() => handleUpgrade(selectedTool)}
                              disabled={getBusy(selectedTool.id) === "upgrading"}
                              className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-[9px] font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-50"
                              title={t("toollaunch.upgradeLatest")}
                            >
                              <Download className={`w-3 h-3 ${getBusy(selectedTool.id) === "upgrading" ? "animate-spin" : ""}`} />
                              {getBusy(selectedTool.id) === "upgrading" ? `${t("toollaunch.upgrading")}...` : t("toollaunch.upgrade")}
                            </button>
                            <button
                              onClick={() => void askUninstall(selectedTool)}
                              disabled={getBusy(selectedTool.id) === "uninstalling"}
                              className="px-2 py-0.5 rounded-md bg-red-500/10 hover:bg-red-500/20 text-[9px] font-semibold text-red-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-50"
                              title={t("toollaunch.uninstallTitle")}
                            >
                              <Trash2 className={`w-3 h-3 ${getBusy(selectedTool.id) === "uninstalling" ? "animate-spin" : ""}`} />
                              {getBusy(selectedTool.id) === "uninstalling" ? `${t("toollaunch.uninstalling")}...` : t("toollaunch.uninstall")}
                            </button>
                          </>
                        )}
                      </>
                    ) : (
                      <span className="text-[10px] text-slate-500">{t("toollaunch.notInstalled")}</span>
                    )}
                    <span className="text-[10px] text-slate-500">· {selectedTool.api_protocol === "none" ? t("toollaunch.modelNone") : PROTOCOL_LABELS[selectedTool.api_protocol]}</span>
                    <a href={selectedTool.website} target="_blank" rel="noopener noreferrer"
                      onClick={(e) => { e.preventDefault(); void openUrl(selectedTool.website); }}
                      className="text-[10px] text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 ml-1"
                      title={t("toollaunch.openSite")}>
                      <ExternalLink className="w-3 h-3" /> {t("toollaunch.site")}
                    </a>
                  </div>
                </div>
              </div>
              {/* 上次启动配置摘要 */}
              {lastLaunchConfigs[selectedTool.id] && (
                <div className="mt-2 px-2 py-1.5 rounded-lg bg-slate-800/50 border border-white/5">
                  <div className="flex items-center gap-1 mb-1">
                    <History className="w-3 h-3 text-slate-500" />
                    <span className="text-[9px] text-slate-500 font-semibold">{t("toollaunch.lastLaunch")}</span>
                    {lastLaunchConfigs[selectedTool.id].last_launched_at && (
                      <span className="text-[9px] text-slate-600 ml-auto">
                        {formatRelativeTime(lastLaunchConfigs[selectedTool.id].last_launched_at, t)}
                      </span>
                    )}
                  </div>
                  <div className="flex flex-wrap gap-x-2 gap-y-0.5 text-[9px]">
                    {lastLaunchConfigs[selectedTool.id].use_official_model ? (
                      <span className="text-slate-400">{t("toollaunch.officialModel")}</span>
                    ) : (
                      <>
                        <span className="text-slate-400">
                          {lastLaunchConfigs[selectedTool.id].provider_name || lastLaunchConfigs[selectedTool.id].provider_id || "-"}
                        </span>
                        {lastLaunchConfigs[selectedTool.id].model_id && (
                          <span className="text-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] truncate max-w-[120px]" title={lastLaunchConfigs[selectedTool.id].model_id ?? undefined}>
                            {lastLaunchConfigs[selectedTool.id].model_id}
                          </span>
                        )}
                        {lastLaunchConfigs[selectedTool.id].fallback_model_id && (
                          <span className="text-amber-400/80 truncate max-w-[120px]" title={t("toollaunch.fallbackModel", { name: lastLaunchConfigs[selectedTool.id].fallback_model_id })}>
                            ※ {lastLaunchConfigs[selectedTool.id].fallback_model_id}
                          </span>
                        )}
                      </>
                    )}
                    {lastLaunchConfigs[selectedTool.id].masquerade_model && (
                      <span className="text-cyan-400/60" title={t("toollaunch.masquerade")}>
                        🎭 {lastLaunchConfigs[selectedTool.id].masquerade_model}
                      </span>
                    )}
                    {lastLaunchConfigs[selectedTool.id].one_m_context && (
                      <span className="text-emerald-400/60" title={t("toollaunch.oneM")}>1M</span>
                    )}
                  </div>
                </div>
              )}
              {!selectedTool.installed && (
                <div className="mt-3 flex items-center gap-2">
                  <code className="flex-1 text-[10px] text-slate-300 bg-slate-900 rounded px-2 py-1.5 font-mono truncate">{selectedTool.install_cmd}</code>
                  <button
                    onClick={() => handleInstall(selectedTool)}
                    disabled={getBusy(selectedTool.id) === "installing"}
                    className="px-2 py-1.5 rounded-md bg-[var(--module-accent-soft)] hover:bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] text-[10px] text-[var(--module-accent)] hover:text-[var(--module-accent-strong)] cursor-pointer transition-all flex items-center gap-1 flex-shrink-0 disabled:opacity-50"
                    title={t("toollaunch.installTitle")}
                  >
                    <Download className={`w-3.5 h-3.5 ${getBusy(selectedTool.id) === "installing" ? "animate-spin" : ""}`} />
                    {getBusy(selectedTool.id) === "installing" ? `${t("toollaunch.installing")}...` : t("toollaunch.install")}
                  </button>
                  <button onClick={() => navigator.clipboard.writeText(selectedTool.install_cmd)}
                    className="px-2 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex-shrink-0">
                    <Copy className="w-3.5 h-3.5" />
                  </button>
                </div>
              )}
            </div>

            {/* 协同身份（头像/昵称，用于协同对话），维护在 AI 工具页 */}
            <div className="p-3 rounded-xl bg-slate-900/30 border border-white/5 space-y-2">
              <div className="flex items-center justify-between">
                <div className="text-xs font-semibold text-slate-300 flex items-center gap-1.5">
                  <Bot className="w-3.5 h-3.5" /> {t("toollaunch.coopIdentity")}
                </div>
                {!editProfile && (
                  <button onClick={openProfile}
                    className="px-2 py-0.5 rounded-md bg-white/5 hover:bg-white/10 text-[10px] text-slate-300 flex items-center gap-1">
                    <Pencil className="w-3 h-3" /> {t("toollaunch.edit")}
                  </button>
                )}
              </div>
              {!editProfile ? (
                <div className="flex items-center gap-2">
                  <span className="w-8 h-8 rounded-md bg-white/5 border border-white/10 flex items-center justify-center text-lg">
                    {selectedTool.avatar || '🤖'}
                  </span>
                  <div className="min-w-0">
                    <div className="text-sm text-slate-200 truncate">{selectedTool.nickname || selectedTool.display_name}</div>
                    <div className="text-[10px] text-slate-500">{t("toollaunch.coopHint")}</div>
                  </div>
                </div>
              ) : (
                <div className="space-y-2">
                  <div className="flex items-center gap-2">
                    <input value={pAvatar} onChange={(e) => setPAvatar(e.target.value)} maxLength={4} placeholder="🤖"
                      className="w-12 px-2 py-1 rounded-md bg-white/5 border border-white/10 text-sm text-center focus:outline-none focus:border-emerald-500/50" />
                    <input value={pNick} onChange={(e) => setPNick(e.target.value)} placeholder={t("toollaunch.nickPh")}
                      className="flex-1 px-2 py-1 rounded-md bg-white/5 border border-white/10 text-[11px] text-slate-200 placeholder-slate-500 focus:outline-none focus:border-emerald-500/50" />
                  </div>
                  <div className="flex items-center gap-2">
                    <button onClick={saveProfile}
                      className="px-2 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 text-[10px] text-emerald-200 flex items-center gap-1">
                      <Check className="w-3 h-3" /> {t("toollaunch.save")}
                    </button>
                    <button onClick={() => setEditProfile(false)}
                      className="px-2 py-1 rounded-md bg-white/5 hover:bg-white/10 text-[10px] text-slate-300">{t("toollaunch.cancel")}</button>
                  </div>
                  {profileMsg && (
                    <div className={`text-[10px] ${profileMsg.ok ? "text-emerald-400" : "text-red-400"}`}>{profileMsg.msg}</div>
                  )}
                </div>
              )}
            </div>

            {/* CLI 工具配置面板 */}
            {selectedTool.installed && selectedTool.supports_model && (
              <>
                {/* 缓存路径（当前工具） */}
                <div>
                  <button
                    onClick={async () => {
                      if (!showCacheManager) { await loadCacheInfos(); }
                      setShowCacheManager(!showCacheManager);
                    }}
                    className="w-full flex items-center justify-between px-3 py-2 rounded-lg bg-slate-900/30 border border-white/5 text-[10px] text-slate-400 hover:text-slate-200 cursor-pointer transition-all"
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
                    <div className="mt-2 rounded-lg border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="max-h-56 overflow-y-auto divide-y divide-white/[0.03]">
                        {cacheInfos.length === 0 ? (
                          <div className="px-3 py-4 text-[10px] text-slate-600 text-center">{t("toollaunch.loading")}</div>
                        ) : selectedToolCaches.length === 0 ? (
                          <div className="px-3 py-4 text-[10px] text-slate-600 text-center">{t("toollaunch.noCache")}</div>
                        ) : (
                          selectedToolCaches.map(cache => (
                            <div key={`${cache.tool_id}:${cache.dir_name}`} className="px-3 py-2 flex items-center gap-3">
                              <HardDrive className="w-3 h-3 text-slate-600 flex-shrink-0" />
                              <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2">
                                  <span className="text-[10px] text-slate-300 font-mono truncate">{cache.dir_name}</span>
                                  {cache.is_junction && (
                                    <span className="text-[8px] text-blue-400 bg-blue-500/10 px-1 rounded">JUNCTION</span>
                                  )}
                                </div>
                                <div className="text-[9px] text-slate-500 font-mono truncate mt-0.5" title={cache.full_path}>
                                  {cache.exists ? cache.full_path : t("toollaunch.notExists")}
                                </div>
                                {cache.is_junction && cache.junction_target && (
                                  <div className="text-[8px] text-blue-400/70 font-mono truncate mt-0.5" title={cache.junction_target}>
                                    ↳ {cache.junction_target}
                                  </div>
                                )}
                                <div className="text-[8px] text-slate-600 mt-0.5">{cache.exists ? cache.size : "0 B"}</div>
                              </div>
                              {cache.exists && (
                                <div className="flex items-center gap-1 flex-shrink-0">
                                  <button onClick={() => handleOpenCacheDir(cache.full_path)}
                                    className="p-1 rounded text-slate-600 hover:text-blue-400 hover:bg-blue-500/10 cursor-pointer"
                                    title={t("toollaunch.openDir")}>
                                    <FolderOpen className="w-3 h-3" />
                                  </button>
                                  <button onClick={() => handleMigrateCache(cache.tool_id, cache.dir_name, cache.full_path)}
                                    disabled={migratingCache === `${cache.tool_id}:${cache.dir_name}`}
                                    className="p-1 rounded text-slate-600 hover:text-emerald-400 hover:bg-emerald-500/10 cursor-pointer disabled:opacity-50"
                                    title={t("toollaunch.migrateCache")}>
                                    <FolderSync className="w-3 h-3" />
                                  </button>
                                  <button onClick={() => handleCleanCache(cache.tool_id, cache.dir_name)}
                                    disabled={cleaningCache === `${cache.tool_id}:${cache.dir_name}`}
                                    className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer disabled:opacity-50"
                                    title={t("toollaunch.clearCache")}>
                                    <Trash2 className="w-3 h-3" />
                                  </button>
                                </div>
                              )}
                            </div>
                          ))
                        )}
                      </div>
                    </div>
                  )}
                </div>

                {/* 官方模型开关（适用于有独立 API key 的工具） */}
                {selectedTool.api_protocol !== "none" && selectedTool.supports_model && (
                  <div className="flex items-center justify-between p-2.5 rounded-lg bg-blue-500/5 border border-blue-500/10">
                    <div className="flex items-center gap-2">
                      <Cpu className="w-3.5 h-3.5 text-blue-400" />
                      <div>
                        <span className="text-[10px] font-semibold text-blue-300">{t("toollaunch.useOfficial")}</span>
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
                    {/* 模型供应商 — 统一列表（代理自动转换协议，任意供应商可选） */}
                    {eligibleProviders.length > 0 && (
                      <div>
                        <label className="text-xs font-bold text-slate-300 mb-1.5 block">{t("toollaunch.modelVendor")}</label>
                        <div className="rounded-lg border border-white/5 bg-slate-900/30">
                          {eligibleProviders.map(group => {
                            const isSelected = selectedModelProvider === group.provider_id;
                            const expanded = expandedModelGroups.has(group.provider_id);
                            return (
                              <div key={group.provider_id}>
                                <button
                                  onClick={() => {
                                    const next = new Set(expandedModelGroups);
                                    if (expanded) next.delete(group.provider_id); else next.add(group.provider_id);
                                    setExpandedModelGroups(next);
                                  }}
                                  className="w-full flex items-center justify-between px-3 py-2 text-[10px] hover:bg-white/[0.02] cursor-pointer transition-all"
                                >
                                  <div className="flex items-center gap-2 min-w-0">
                                    <ChevronRight className={`w-3 h-3 text-slate-500 transition-transform ${expanded ? "rotate-90" : ""}`} />
                                    <span className="font-semibold text-slate-400">{group.provider_name}</span>
                                    <span className="text-[8px] text-slate-600">{t("toollaunch.modelsCount", { count: group.models.length })}</span>
                                    {providerProtocolBadges(config?.providers.find(p => p.id === group.provider_id))}
                                  </div>
                                  {isSelected && selectedModel && (
                                    <span className="text-[9px] text-[var(--module-accent)] font-mono truncate ml-2">{selectedModel}</span>
                                  )}
                                </button>
                                {expanded && (
                                  <div className="border-t border-white/[0.03]">
                                    {group.models.map(m => {
                                      const isSelModel = selectedModel === m.id && selectedModelProvider === group.provider_id;
                                      return (
                                        <button key={`${group.provider_id}:${m.id}`}
                                          onClick={() => {
                                            if (isSelModel) { setSelectedModel(""); setSelectedModelProvider(""); resetCustomParamValues([]); }
                                            else { setSelectedModel(m.id); setSelectedModelProvider(group.provider_id); resetCustomParamValues(m.customParams || []); }
                                          }}
                                          className={`w-full text-left px-5 py-1.5 text-[11px] transition-all cursor-pointer flex items-center gap-2 ${
                                            isSelModel
                                              ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)] font-semibold"
                                              : "text-slate-400 hover:bg-white/5 hover:text-slate-200"
                                          }`}>
                                          <span className="w-1.5 h-1.5 rounded-full flex-shrink-0" style={{ backgroundColor: isSelModel ? "#a78bfa" : "#334155" }} />
                                          <span className="font-mono">{m.id}</span>
                                        </button>
                                      );
                                    })}
                                  </div>
                                )}
                              </div>
                            );
                          })}
                        </div>
                        {selectedModel && (
                          <div className="mt-1 text-[10px] text-[var(--module-accent)]">{t("toollaunch.selected")}<span className="font-mono">{selectedModel}</span> <span className="text-slate-500">（{config?.providers.find(p => p.id === selectedModelProvider)?.name}）</span></div>
                        )}

                        {/* 模型自定义启动参数（用户定义，运行时渲染为控件） */}
                        {currentModelCustomParams.length > 0 && (
                          <div className="mt-3 space-y-2">
                            <div className="text-[10px] text-slate-500 font-semibold">{t("toollaunch.customParams")}</div>
                            {currentModelCustomParams.map(cp => (
                              <div key={cp.key} className="flex items-center gap-2">
                                <label className="text-[10px] text-slate-400 w-28 flex-shrink-0 truncate" title={cp.key}>{cp.label || cp.key}</label>
                                {cp.paramType === "bool" ? (
                                  <input type="checkbox" checked={customParamValues[cp.key] !== "false"}
                                    onChange={e => setCustomParamValues(prev => ({ ...prev, [cp.key]: e.target.checked ? "true" : "false" }))}
                                    className="w-4 h-4 accent-[var(--module-accent)]" />
                                ) : cp.paramType === "text" ? (
                                  <input type="text" value={customParamValues[cp.key] || ""}
                                    onChange={e => setCustomParamValues(prev => ({ ...prev, [cp.key]: e.target.value }))}
                                    placeholder={cp.defaultValue || ""}
                                    className="flex-1 min-w-0 bg-slate-900 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                                ) : (
                                  <select value={customParamValues[cp.key] || cp.defaultValue || ""}
                                    onChange={e => setCustomParamValues(prev => ({ ...prev, [cp.key]: e.target.value }))}
                                    className="flex-1 min-w-0 bg-slate-900 border border-white/10 rounded px-2 py-1 text-[10px] text-slate-200 focus:outline-none focus:border-[var(--module-accent)]">
                                    {(cp.options && cp.options.length > 0 ? cp.options : [cp.defaultValue || ""]).filter(Boolean).map(o => (
                                      <option key={o} value={o}>{o}</option>
                                    ))}
                                  </select>
                                )}
                                <span className="text-[8px] text-slate-600 font-mono flex-shrink-0 w-16 text-right">
                                  {cp.target === "config" ? (cp.configPath || "config") : (cp.envKey || "env")}
                                </span>
                              </div>
                            ))}
                          </div>
                        )}
                      </div>
                    )}

                    {/* 没有可用的供应商/模型时的警告 */}
                    {eligibleProviders.length === 0 && (
                      <Note tone="warn">{t("toollaunch.noModelsWarn")}</Note>
                    )}

                    {/* 模型伪装（仅当工具内置模型名列表非空） */}
                    {selectedModel && selectedTool.builtin_models.length > 0 && (
                      <div className="mt-3">
                        <label className="text-xs font-bold text-slate-300 mb-1.5 block">{t("toollaunch.masqueradeLabel")} <span className="text-[9px] text-slate-500 font-normal">{t("toollaunch.optional")}</span></label>
                        <p className="text-[9px] text-slate-500 mb-1.5">{t("toollaunch.masqueradeHint", { model: selectedModel })}</p>
                        <input type="text" list={`masq-list-${selectedTool.id}`} value={masqueradeModel}
                          onChange={e => setMasqueradeModel(e.target.value)}
                          placeholder={t("toollaunch.noMasqueradePh", { model: selectedModel })}
                          className="w-full bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                        <datalist id={`masq-list-${selectedTool.id}`}>
                          {selectedTool.builtin_models.map(c => (
                            <option key={c} value={c} />
                          ))}
                        </datalist>
                      </div>
                    )}

                    {/* 只设置模型、不启动工具：把模型写进工具自己的配置文件。
                        启动会顺带写，但很多人只想先配好（之后直接双击工具图标用）。 */}
                    {selectedTool.config_file && (
                      <div className="mt-3 rounded-lg border border-white/5 bg-slate-900/30 p-2.5">
                        <div className="flex items-center gap-2">
                          <span className="text-[10px] text-slate-400 flex-1 min-w-0">
                            {t("toollaunch.applyModelHint")}
                          </span>
                          <button
                            onClick={() => void applyModelOnly()}
                            disabled={!selectedModel || !selectedModelProvider || applyModelBusy}
                            className="px-2.5 py-1 rounded-md text-[10px] font-semibold bg-[var(--module-accent)]/20 hover:bg-[var(--module-accent)]/30 text-white cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
                          >
                            {applyModelBusy ? t("toollaunch.applyingModel") : t("toollaunch.applyModel")}
                          </button>
                        </div>
                        <div className="mt-1.5 text-[9px] text-slate-500 break-all">
                          {t("toollaunch.configFileLabel")}
                          <span className="font-mono text-slate-400">{selectedTool.config_file.path}</span>
                        </div>
                        {appliedModel !== null && (
                          <div className="mt-1 text-[9px] text-slate-500">
                            {t("toollaunch.currentConfigModel")}
                            <span className="font-mono text-slate-300">{appliedModel || t("toollaunch.unknownModel")}</span>
                          </div>
                        )}
                        {applyModelMsg && (
                          <div className={`mt-1 text-[9px] break-all ${applyModelMsg.ok ? "text-emerald-400" : "text-rose-400"}`}>
                            {applyModelMsg.text}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                )}

                {/* Fallback 模型 — 按供应商分组，可折叠 */}
                {selectedTool.supports_fallback_model && selectedTool.installed && !useOfficialModel && fallbackGroups.length > 0 && (
                  <div>
                    <label className="text-xs font-bold text-slate-300 mb-2 block">
                      {t("toollaunch.fallbackLabel")}
                      <span className="text-[9px] text-slate-500 font-normal ml-1">{t("toollaunch.fallbackHint")}</span>
                    </label>
                    <div className="rounded-lg border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="px-3 py-1.5 text-[9px] text-slate-600 font-mono cursor-pointer hover:bg-white/[0.05] border-b border-white/[0.03]"
                        onClick={() => { setSelectedFallbackModel(""); setSelectedFallbackProvider(""); setFallbackOneMContext(false); }}>
                        {t("toollaunch.noFallback")}
                      </div>
                      {fallbackGroups.map(group => {
                        const expanded = expandedFallbackGroups.has(group.provider_id);
                        const selectedInGroup = selectedFallbackProvider === group.provider_id && selectedFallbackModel !== "";
                        return (
                          <div key={`fbg:${group.provider_id}`}>
                            <button
                              onClick={() => {
                                const next = new Set(expandedFallbackGroups);
                                if (expanded) next.delete(group.provider_id); else next.add(group.provider_id);
                                setExpandedFallbackGroups(next);
                              }}
                              className="w-full flex items-center justify-between px-3 py-1.5 text-[10px] hover:bg-white/[0.02] cursor-pointer transition-all border-b border-white/[0.03]"
                            >
                              <div className="flex items-center gap-2">
                                <ChevronRight className={`w-3 h-3 text-slate-500 transition-transform ${expanded ? "rotate-90" : ""}`} />
                                <span className="font-semibold text-slate-400">{group.provider_name}</span>
                                <span className="text-[8px] text-slate-600">{t("toollaunch.itemsCount", { count: group.models.length })}</span>
                              </div>
                              {selectedInGroup && (
                                <span className="text-[9px] text-amber-400 font-mono truncate ml-2">{selectedFallbackModel}</span>
                              )}
                            </button>
                            {expanded && (
                              <div className="border-t border-white/[0.03]">
                                {group.models.map(m => {
                                  const isSelected = selectedFallbackModel === m.id && selectedFallbackProvider === group.provider_id;
                                  return (
                                    <button key={`fb:${group.provider_id}:${m.id}`}
                                      onClick={() => {
                                        if (isSelected) { setSelectedFallbackModel(""); setSelectedFallbackProvider(""); setFallbackOneMContext(false); }
                                        else { setSelectedFallbackModel(m.id); setSelectedFallbackProvider(group.provider_id); }
                                      }}
                                      className={`w-full text-left px-5 py-1.5 text-[10px] transition-all cursor-pointer flex items-center gap-2 ${
                                        isSelected ? "bg-amber-500/10 text-amber-300 font-semibold" : "text-slate-400 hover:bg-white/5 hover:text-slate-300"
                                      }`}>
                                      <span className="w-1.5 h-1.5 rounded-full flex-shrink-0" style={{ backgroundColor: isSelected ? "#f59e0b" : "#334155" }} />
                                      <span className="font-mono">{m.id}</span>
                                    </button>
                                  );
                                })}
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                    {selectedFallbackModel && selectedTool.builtin_models.length > 0 && (
                      <div className="mt-3">
                        <label className="text-[11px] font-bold text-slate-300 mb-1.5 block">{t("toollaunch.fallbackMqLabel")} <span className="text-[9px] text-slate-500 font-normal">{t("toollaunch.optional")}</span></label>
                        <p className="text-[9px] text-slate-500 mb-1.5">{t("toollaunch.fallbackMqHint", { model: selectedFallbackModel })}</p>
                        <input type="text" list={`fb-masq-list-${selectedTool.id}`} value={fallbackMasqueradeModel}
                          onChange={e => setFallbackMasqueradeModel(e.target.value)}
                          placeholder={t("toollaunch.noFallbackMqPh", { model: selectedFallbackModel })}
                          className="w-full bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
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
                          <label className="flex items-center gap-2 mt-2 text-[10px] text-slate-400 cursor-pointer select-none">
                            <input type="checkbox" checked={fallbackOneMContext} onChange={e => setFallbackOneMContext(e.target.checked)}
                              className="accent-[var(--module-accent)]" />
                            {t("toollaunch.fallbackOneM")}
                          </label>
                        )}
                        <div className="mt-1 text-[10px] text-amber-400">{t("toollaunch.fallbackPreview", { model: `${selectedFallbackModel}${fallbackOneMContext ? "[1m]" : ""}` })}{fallbackMasqueradeModel && <>{t("toollaunch.masqueradeAs", { model: `${fallbackMasqueradeModel}${fallbackOneMContext ? "[1m]" : ""}` })}</>}</div>
                      </>
                    )}
                  </div>
                )}

                {/* 1M Context Toggle — 由 config.json 的 supportOneMContext 字段驱动 */}
                {selectedTool.supports_model && selectedTool.support_one_m_context && (
                  <div className="flex items-center justify-between p-2.5 rounded-lg bg-slate-900/30 border border-white/5">
                    <div className="flex items-center gap-2">
                      <span className="text-[10px] font-semibold text-slate-300">1M Context</span>
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
                  <div className="rounded-lg bg-slate-900/30 border border-white/5 overflow-hidden">
                    <div className="flex items-center justify-between p-2.5">
                      <div className="flex items-center gap-2">
                        <span className="text-[10px] font-semibold text-slate-300">{t("toollaunch.liveSearch")}</span>
                        <span className="text-[8px] text-slate-500 hidden sm:inline">{t("toollaunch.liveSearchHint")}</span>
                      </div>
                      <button onClick={() => setWebSearchEnabled(!webSearchEnabled)}
                        className={`p-1 rounded-md cursor-pointer transition-all ${webSearchEnabled ? "text-emerald-400" : "text-slate-600 hover:text-slate-400"}`}>
                        {webSearchEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                      </button>
                    </div>
                  </div>
                )}

                {/* 代理增强能力（优化器 / 整流器）— 默认沿用全局配置，可在此按启动覆盖 */}
                {selectedTool.supports_model && !useOfficialModel && (selectedTool.supports_optimizer || selectedTool.supports_rectifier) && (
                  <div className="space-y-2">
                    {selectedTool.supports_optimizer && (
                      <div className="rounded-lg bg-slate-900/30 border border-white/5 overflow-hidden">
                        <div className="flex items-center justify-between p-2.5">
                          <div className="flex items-center gap-2">
                            <span className="text-[10px] font-semibold text-slate-300">{t("toollaunch.optimizer")}</span>
                            <span className="text-[8px] text-slate-500 hidden sm:inline">{t("toollaunch.optimizerHint")}</span>
                          </div>
                          <button onClick={() => setOptimizerEnabled(!optimizerEnabled)}
                            className={`p-1 rounded-md cursor-pointer transition-all ${optimizerEnabled ? "text-[var(--module-accent)]" : "text-slate-600 hover:text-slate-400"}`}>
                            {optimizerEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                          </button>
                        </div>
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
                                <span className="text-[10px] text-slate-300">{item.label}</span>
                                <span className="text-[9px] text-slate-600">{item.desc}</span>
                              </label>
                            ))}
                          </div>
                        )}
                      </div>
                    )}
                    {selectedTool.supports_rectifier && (
                      <div className="rounded-lg bg-slate-900/30 border border-white/5 overflow-hidden">
                        <div className="flex items-center justify-between p-2.5">
                          <div className="flex items-center gap-2">
                            <span className="text-[10px] font-semibold text-slate-300">{t("toollaunch.rectifier")}</span>
                            <span className="text-[8px] text-slate-500 hidden sm:inline">{t("toollaunch.rectifierHint")}</span>
                          </div>
                          <button onClick={() => setRectifierEnabled(!rectifierEnabled)}
                            className={`p-1 rounded-md cursor-pointer transition-all ${rectifierEnabled ? "text-[var(--module-accent)]" : "text-slate-600 hover:text-slate-400"}`}>
                            {rectifierEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                          </button>
                        </div>
                        {rectifierEnabled && (
                          <div className="px-3 pb-2.5 space-y-1.5 border-t border-white/5 pt-2">
                            {[
                              { key: "thinking_signature" as const, label: t("toollaunch.recThinkingSig"), desc: t("toollaunch.recThinkingSigDesc") },
                              { key: "thinking_budget" as const, label: t("toollaunch.recThinkingBudget"), desc: t("toollaunch.recThinkingBudgetDesc") },
                              { key: "media_fallback" as const, label: t("toollaunch.recMedia"), desc: t("toollaunch.recMediaDesc") },
                              { key: "protocol_mismatch" as const, label: t("toollaunch.recProtocol"), desc: t("toollaunch.recProtocolDesc") },
                            ].map(item => (
                              <label key={item.key} className="flex items-center gap-2 cursor-pointer">
                                <input
                                  type="checkbox"
                                  checked={rectifierStrategies[item.key]}
                                  onChange={() => setRectifierStrategies(prev => ({ ...prev, [item.key]: !prev[item.key] }))}
                                  className="accent-[var(--module-accent)]"
                                />
                                <span className="text-[10px] text-slate-300">{item.label}</span>
                                <span className="text-[9px] text-slate-600">{item.desc}</span>
                              </label>
                            ))}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                )}

                {/* 会话 */}
                <div>
                  <div className="flex items-center justify-between mb-2">
                    <label className="text-xs font-bold text-slate-300">{t("toollaunch.sessions")}</label>
                    {sessions.length > 0 && (
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => setSessionViewMode(sessionViewMode === "flat" ? "grouped" : "flat")}
                          className="p-1 rounded text-slate-500 hover:text-slate-300 cursor-pointer transition-all"
                          title={sessionViewMode === "flat" ? t("toollaunch.groupView") : t("toollaunch.listView")}
                        >
                          {sessionViewMode === "flat" ? <ListTree className="w-3.5 h-3.5" /> : <List className="w-3.5 h-3.5" />}
                        </button>
                        <button
                          onClick={() => { setSelectionMode(!selectionMode); setSelectedSessionIds(new Set()); }}
                          className={`p-1 rounded cursor-pointer transition-all ${selectionMode ? "text-[var(--module-accent)]" : "text-slate-500 hover:text-slate-300"}`}
                        >
                          <CheckCircle className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    )}
                  </div>

                  <div className="flex gap-2 flex-wrap mb-2">
                    <button onClick={() => { setSessionMode("new"); setSelectedSession(null); setShowSessionPicker(false); }}
                      className={`px-3 py-1.5 rounded-lg text-[10px] font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                        sessionMode === "new" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                      }`}>
                      {t("toollaunch.newSession")}
                    </button>
                    {sessions.length > 0 && (
                      <button onClick={() => { setSessionMode("resume"); setShowSessionPicker(!showSessionPicker); setSelectedSession(null); }}
                        className={`px-3 py-1.5 rounded-lg text-[10px] font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                          sessionMode === "resume" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                        }`}>
                        <Clock className="w-3 h-3" /> {t("toollaunch.historySessions", { count: sessions.length })}
                      </button>
                    )}
                  </div>

                  {showSessionPicker && sessionMode === "resume" && (
                    <div className="mb-2">
                      <div className="flex items-center gap-2">
                        <div className="flex-1 relative">
                          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500" />
                          <input value={sessionSearch} onChange={e => setSessionSearch(e.target.value)}
                            placeholder={t("toollaunch.searchSessions")} className="w-full bg-slate-900 border border-white/10 rounded-lg pl-7 pr-7 py-1.5 text-[10px] text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                          {sessionSearch && (
                            <button onClick={() => setSessionSearch("")} className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300">
                              <X className="w-3 h-3" />
                            </button>
                          )}
                        </div>
                        {selectionMode && (
                          <>
                            <button onClick={handleSelectAll} className="px-2 py-1 rounded text-[9px] font-semibold bg-white/5 text-slate-400 hover:text-slate-200 cursor-pointer whitespace-nowrap">
                              {selectedSessionIds.size === filteredSessions.length ? t("toollaunch.cancelSelectAll") : t("toollaunch.selectAll")}
                            </button>
                            <button onClick={handleDeleteSessions} disabled={selectedSessionIds.size === 0}
                              className="px-2 py-1 rounded text-[9px] font-semibold bg-red-500/10 text-red-400 hover:bg-red-500/20 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed whitespace-nowrap flex items-center gap-1">
                              <Trash2 className="w-3 h-3" /> {t("toollaunch.delete", { count: selectedSessionIds.size })}
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                  )}

                  {showSessionPicker && sessionMode === "resume" && (
                    <div className="rounded-lg border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="max-h-72 overflow-y-auto divide-y divide-white/[0.03]">
                        {filteredSessions.length === 0 ? (
                          <div className="px-3 py-6 text-[10px] text-slate-600 text-center">
                            {sessionSearch ? t("toollaunch.noMatchSessions") : t("toollaunch.noHistorySessions")}
                          </div>
                        ) : sessionViewMode === "flat" ? (
                          filteredSessions.map(s => (
                            <div key={s.session_id}
                              className={`flex items-center px-3 py-2 text-[10px] transition-all group ${
                                selectedSession?.session_id === s.session_id ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "text-slate-400 hover:bg-white/[0.03] hover:text-slate-200"
                              }`}>
                              {selectionMode && (
                                <button onClick={() => toggleSessionSelect(s.session_id)}
                                  className={`mr-2 w-4 h-4 rounded border flex-shrink-0 flex items-center justify-center cursor-pointer ${
                                    selectedSessionIds.has(s.session_id) ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white" : "border-slate-700 hover:border-slate-500"
                                  }`}>
                                  {selectedSessionIds.has(s.session_id) && <CheckCircle className="w-3 h-3" />}
                                </button>
                              )}
                              <button onClick={() => { if (!selectionMode) { setSelectedSession(s); setProjectPath(s.project_path); } }}
                                className="flex-1 text-left flex items-center justify-between min-w-0">
                                <div className="flex-1 min-w-0">
                                  <span className="font-mono text-slate-300 break-all block truncate">{s.project_path}</span>
                                  {s.summary && <div className="text-[9px] text-slate-500 mt-0.5 truncate italic">{s.summary}</div>}
                                </div>
                                <span className="text-[9px] text-slate-600 flex-shrink-0 ml-3">{s.last_used}</span>
                              </button>
                            </div>
                          ))
                        ) : (
                          sessionDirGroups.map(group => (
                            <div key={group.dir}>
                              <button onClick={() => toggleDirExpand(group.dir)}
                                className="w-full flex items-center gap-2 px-3 py-2 text-[10px] bg-white/[0.02] hover:bg-white/[0.04] text-slate-400 hover:text-slate-200 cursor-pointer sticky top-0 z-10">
                                <ChevronRight className={`w-3 h-3 flex-shrink-0 transition-transform ${expandedDirs.has(group.dir) ? "rotate-90" : ""}`} />
                                <Folder className="w-3 h-3 flex-shrink-0 text-amber-500/70" />
                                <span className="font-semibold truncate">{group.label}</span>
                                <span className="text-[9px] text-slate-600 ml-auto">{group.sessions.length}</span>
                              </button>
                              {expandedDirs.has(group.dir) && group.sessions.map(s => (
                                <div key={s.session_id}
                                  className={`flex items-center pl-9 pr-3 py-2 text-[10px] transition-all group ${
                                    selectedSession?.session_id === s.session_id ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]" : "text-slate-400 hover:bg-white/[0.03] hover:text-slate-200"
                                  }`}>
                                  {selectionMode && (
                                    <button onClick={() => toggleSessionSelect(s.session_id)}
                                      className={`mr-2 w-3.5 h-3.5 rounded border flex-shrink-0 flex items-center justify-center cursor-pointer ${
                                        selectedSessionIds.has(s.session_id) ? "bg-[var(--module-accent)] border-[var(--module-accent)] text-white" : "border-slate-700 hover:border-slate-500"
                                      }`}>
                                      {selectedSessionIds.has(s.session_id) && <CheckCircle className="w-2.5 h-2.5" />}
                                    </button>
                                  )}
                                  <button onClick={() => { if (!selectionMode) { setSelectedSession(s); setProjectPath(s.project_path); } }}
                                    className="flex-1 text-left flex items-center justify-between min-w-0">
                                    <div className="flex-1 min-w-0">
                                      <span className="text-slate-400 truncate block">
                                        {s.session_id.slice(0, 8)}...
                                        {s.summary && <span className="text-[9px] text-slate-500 ml-2 italic truncate">{s.summary}</span>}
                                      </span>
                                    </div>
                                    <span className="text-[9px] text-slate-600 flex-shrink-0 ml-3">{s.last_used}</span>
                                  </button>
                                </div>
                              ))}
                            </div>
                          ))
                        )}
                      </div>
                    </div>
                  )}

                  {sessionMode === "resume" && selectedSession && (
                    <div className="mt-2 p-2 rounded-lg bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] text-[10px] text-[var(--module-accent)] flex items-center gap-2">
                      <CheckCircle className="w-3 h-3 flex-shrink-0" />
                      <span className="truncate">{t("toollaunch.willRestore", { path: selectedSession.project_path })}</span>
                    </div>
                  )}
                </div>

                {/* 项目目录 */}
                {sessionMode === "new" && (
                  <div>
                    <label className="text-xs font-bold text-slate-300 mb-2 block">{t("toollaunch.projectDir")}</label>
                    <div className="flex gap-2">
                      <input value={projectPath} onChange={e => setProjectPath(e.target.value)} placeholder={t("toollaunch.projectDirPh")}
                        className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                      <button onClick={handleBrowse}
                        className="px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-slate-400 hover:text-white hover:bg-white/10 cursor-pointer transition-all">
                        <FolderOpen className="w-4 h-4" />
                      </button>
                    </div>
                  </div>
                )}

                {/* 终端 */}
                {terminals.length > 0 && (
                  <div>
                    <label className="text-xs font-bold text-slate-300 mb-2 block">{t("toollaunch.terminal")}</label>
                    <select value={selectedTerminal} onChange={e => setSelectedTerminal(e.target.value)}
                      className="w-full bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-[var(--module-accent)]">
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
                    masqueradeModel,
                    selectedFallbackModel,
                    fallbackMasqueradeModel,
                  );
                  if (!proxyInfo) return null;
                  return (
                    <div className="p-2.5 rounded-lg bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] text-[10px] flex flex-col gap-1.5">
                      <div className="flex items-center gap-2 flex-wrap">
                        <Shield className="w-3.5 h-3.5 text-[var(--module-accent)] flex-shrink-0" />
                        <span className="text-slate-300">
                          {t("toollaunch.inbound")} <span className="font-semibold text-[var(--module-accent)]">{proxyInfo.inbound === "none" ? t("toollaunch.modelNone") : PROTOCOL_LABELS[proxyInfo.inbound]}</span>
                          <span className="mx-1 text-slate-500">→</span>
                          {t("toollaunch.outbound")} <span className="font-semibold text-[var(--module-accent)]">{proxyInfo.outbound === "none" ? t("toollaunch.modelNone") : PROTOCOL_LABELS[proxyInfo.outbound]}</span>
                        </span>
                        {proxyInfo.converted ? (
                          <span className="px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-300 text-[9px] font-semibold">{t("toollaunch.autoConvert")}</span>
                        ) : (
                          <span className="px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-300 text-[9px] font-semibold">{t("toollaunch.sameProtocol")}</span>
                        )}
                        <span className="px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-300 text-[9px] font-semibold">{t("toollaunch.statsOn")}</span>
                        {selectedTool.supports_optimizer && optimizerEnabled && config?.optimizer.enabled && (
                          <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] text-[9px] font-semibold">{t("toollaunch.optimizerBadge")}</span>
                        )}
                        {selectedTool.supports_rectifier && rectifierEnabled && config?.rectifier.enabled && (
                          <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] text-[9px] font-semibold">{t("toollaunch.rectifierBadge")}</span>
                        )}
                      </div>
                      {proxyInfo.aliasEntries.length > 0 && (
                        <div className="flex items-center gap-1.5 flex-wrap text-slate-400">
                          <span className="text-slate-500">{t("toollaunch.masqueradeLabel2")}</span>
                          {proxyInfo.aliasEntries.map(([k, v]) => (
                            <span key={k} className="font-mono text-[9px] bg-slate-700/40 px-1.5 py-0.5 rounded">{k} → {v}</span>
                          ))}
                        </div>
                      )}
                    </div>
                  );
                })()}

                {/* 启动按钮 */}
                <button onClick={handleLaunch} disabled={launching || !canLaunch}
                  className="w-full py-3 rounded-xl bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-bold flex items-center justify-center gap-2 cursor-pointer transition-all shadow-lg shadow-[var(--module-accent-ring)]">
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
              <div className="rounded-xl border border-white/10 bg-black/40 p-2.5">
                <div className="mb-1.5 flex items-center gap-1.5 text-[9px] font-semibold text-slate-400">
                  <Terminal className="w-3 h-3" />
                  {t("toollaunch.opProgress")}
                  <span className="ml-auto text-slate-600">{opLogs[selectedTool.id].lines.length}</span>
                </div>
                <div ref={opLogRef} className="max-h-32 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[10px] leading-relaxed text-slate-300">
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
