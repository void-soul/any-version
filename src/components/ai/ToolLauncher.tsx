import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Rocket,
  FolderOpen,
  CheckCircle,
  AlertTriangle,
  RefreshCw,
  Terminal,
  Bot,
  Clock,
  Play,
  Plus,
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
  Cpu,
} from "lucide-react";
import type {
  AiProvider,
  AiConfig,
  LastLaunchConfig,
  DetectedAiTool,
  AiToolCacheInfo,
  ToolSession,
  TerminalInfo,
} from "./types";

const PROTOCOL_LABELS: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  both: "OpenAI + Anthropic",
  google: "Google",
  none: "仅支持官方模型",
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

export default function ToolLauncher() {
  const [tools, setTools] = useState<DetectedAiTool[]>([]);
  const [config, setConfig] = useState<AiConfig | null>(null);
  const [terminals, setTerminals] = useState<TerminalInfo[]>([]);
  const [sessions, setSessions] = useState<ToolSession[]>([]);
  const [loading, setLoading] = useState(true);

  const [selectedToolId, setSelectedToolId] = useState<string | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [selectedModelProvider, setSelectedModelProvider] = useState("");
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

  const [launching, setLaunching] = useState(false);
  const [launchResult, setLaunchResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const [upgradingTool, setUpgradingTool] = useState<string | null>(null);
  const [upgradeResult, setUpgradeResult] = useState<{ id: string; msg: string } | null>(null);
  const [versionStatuses, setVersionStatuses] = useState<Record<string, { latest: string; status: string }>>({});
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
        invoke<Array<{ tool_id: string; current_version: string | null; latest_version: string | null; status: string }>>("check_ai_tool_versions"),
      ]);
      const map: Record<string, { latest: string; status: string }> = {};
      for (const r of regResults) {
        map[r.project_id] = { latest: r.latest_version || "", status: r.status };
      }
      for (const r of aiResults) {
        map[r.tool_id] = { latest: r.latest_version || "", status: r.status };
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
        invoke<AiConfig>("get_ai_config").catch(() => ({ providers: [], active_provider: null, proxy_port: 15721, default_project_path: "", rectifier: { enabled: false, thinking_signature: false, thinking_budget: false, media_fallback: false, protocol_mismatch: false }, optimizer: { enabled: false, cache_injection: false, thinking_optimizer: false, deepseek_normalize: false }, skills_dir: "" })),
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

  // ── 模型供应商（统一列表）──
  // 新设计下代理会自动做协议转换，因此 ANY 提供模型列表的供应商都可选；
  // 协议差异由代理的入站/出站转换负责。这里合并为单一列表（按供应商分组）。
  const eligibleProviders = React.useMemo(() => {
    if (!config || !selectedTool) return [];
    if (!selectedTool.supports_model) return [];
    const groups: { provider_name: string; provider_id: string; models: { id: string }[] }[] = [];
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
    const groups: { provider_name: string; provider_id: string; models: { id: string }[] }[] = [];
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
      const selected = await open({ directory: true, title: "选择项目目录" });
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
          masquerade_model: useOfficialModel ? null : (masqueradeModel || null),
          optimizer_enabled: useOfficialModel ? null : optimizerEnabled,
          rectifier_enabled: useOfficialModel ? null : rectifierEnabled,
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

  const handleUpgrade = async (tool: DetectedAiTool) => {
    setUpgradingTool(tool.id);
    setUpgradeResult(null);
    try {
      const msg = await invoke<string>("upgrade_ai_tool", { toolId: tool.id });
      setUpgradeResult({ id: tool.id, msg });
      const t = await invoke<DetectedAiTool[]>("detect_ai_tools").catch(() => []);
      setTools(t);
      await checkVersions();
    } catch (e: any) {
      setUpgradeResult({ id: tool.id, msg: String(e) });
    } finally { setUpgradingTool(null); }
  };

  const loadCacheInfos = useCallback(async () => {
    try {
      const infos = await invoke<AiToolCacheInfo[]>("get_ai_tool_cache_info");
      setCacheInfos(infos);
    } catch (e) { console.error(e); }
  }, []);

  const handleMigrateCache = async (toolId: string, dirName: string, fullPath: string) => {
    try {
      const selected = await open({ directory: true, title: "选择新的缓存目录" });
      if (!selected) return;
      setMigratingCache(`${toolId}:${dirName}`);
      await invoke("migrate_ai_tool_cache", { toolId, dirName, newPath: selected as string });
      await loadCacheInfos();
    } catch (e: any) { alert(`迁移失败: ${e}`); }
    finally { setMigratingCache(null); }
  };

  const handleCleanCache = async (toolId: string, dirName: string) => {
    if (!confirm(`确定要清理 ${dirName} 的所有缓存数据吗？此操作不可恢复。`)) return;
    setCleaningCache(`${toolId}:${dirName}`);
    try {
      await invoke("clean_ai_tool_cache", { toolId, dirName });
      await loadCacheInfos();
    } catch (e: any) { alert(`清理失败: ${e}`); }
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
      const dir = s.project_path || "未知目录";
      const label = dir.split(/[\\/]/).pop() || dir;
      if (!groups.has(dir)) groups.set(dir, { dir, label, sessions: [] });
      groups.get(dir)!.sessions.push(s);
    }
    return Array.from(groups.values()).sort((a, b) => a.label.localeCompare(b.label));
  }, [filteredSessions]);

  const handleDeleteSessions = async () => {
    if (selectedSessionIds.size === 0) return;
    if (!confirm(`确定要删除 ${selectedSessionIds.size} 个会话记录吗？此操作不可恢复。`)) return;
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
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-xs">加载中...</span></div>;
  }

  const getVerStatus = (toolId: string): { label: string; color: string; icon: React.ReactNode } | null => {
    const vs = versionStatuses[toolId];
    if (!vs) return null;
    switch (vs.status) {
      case "outdated": return { label: "可升级", color: "text-amber-400", icon: <ArrowUpCircle className="w-2.5 h-2.5" /> };
      case "latest": return { label: "最新", color: "text-emerald-400", icon: <CheckCircle className="w-2.5 h-2.5" /> };
      case "unknown": return null;
      case "not_installed": return null;
      default: return null;
    }
  };

  const canLaunch = selectedTool?.installed && (sessionMode === "resume" || projectPath);

  return (
    <div className="h-full flex min-h-0 select-none">
      {/* ── 左侧工具列表 ── */}
      <div className="w-52 flex-shrink-0 border-r border-white/5 py-3 px-2 overflow-y-auto space-y-0.5 flex flex-col">
        <div className="flex items-center justify-between px-1 mb-1">
          <span className="text-[9px] font-bold text-slate-500 uppercase">AI 工具</span>
          <button onClick={checkVersions} disabled={checkingVersions}
            className="p-0.5 rounded text-slate-600 hover:text-slate-400 cursor-pointer"
            title="检测版本">
            <RefreshCw className={`w-3 h-3 ${checkingVersions ? "animate-spin" : ""}`} />
          </button>
        </div>
        {tools.map(tool => {
          const vs = getVerStatus(tool.id);
          return (
            <button
              key={tool.id}
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
                      if (last.provider_id) setSelectedModelProvider(last.provider_id);
                      if (last.model_id) setSelectedModel(last.model_id);
                      if (last.fallback_model_id) setSelectedFallbackModel(last.fallback_model_id);
                      if (last.fallback_provider_id) setSelectedFallbackProvider(last.fallback_provider_id);
                      if (last.fallback_masquerade_model) setFallbackMasqueradeModel(last.fallback_masquerade_model);
                    }
                    if (last.terminal_id && last.terminal_id !== "cmd") setSelectedTerminal(last.terminal_id);
                    if (last.one_m_context) setOneMContext(true);
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
                  ? "bg-violet-600 text-white shadow-md shadow-violet-500/10"
                  : tool.installed
                    ? "text-slate-300 hover:text-white hover:bg-white/5"
                    : "text-slate-600 hover:text-slate-400 hover:bg-white/[0.03]"
              }`}
            >
              <div className="flex items-center gap-2">
                <Bot className={`w-3.5 h-3.5 flex-shrink-0 ${selectedToolId === tool.id ? "text-white" : tool.installed ? "text-slate-400" : "text-slate-700"}`} />
                <span className="text-[11px] font-semibold truncate">{tool.display_name}</span>
                {vs && (
                  <span className={`text-[9px] font-semibold flex items-center gap-0.5 ml-auto flex-shrink-0 ${vs.color}`}>
                    {vs.icon}
                    {vs.label}
                  </span>
                )}
              </div>
              <div className="flex items-center gap-1.5 mt-0.5 ml-5.5">
                {tool.installed ? (
                  <span className={`text-[9px] ${selectedToolId === tool.id ? "text-violet-200" : "text-slate-500"} font-mono`}>
                    {tool.version || "已安装"}
                  </span>
                ) : (
                  <span className="text-[9px] text-slate-600">未安装</span>
                )}
                {lastLaunchConfigs[tool.id] && tool.installed && (
                  <span className={`text-[9px] truncate max-w-[80px] ${selectedToolId === tool.id ? "text-violet-300/70" : "text-slate-600"}`}>
                    {lastLaunchConfigs[tool.id].use_official_model
                      ? "官方"
                      : (lastLaunchConfigs[tool.id].provider_name || lastLaunchConfigs[tool.id].provider_id || "")}
                    {(lastLaunchConfigs[tool.id].fallback_model_id && !lastLaunchConfigs[tool.id].use_official_model) ? " ※" : ""}
                  </span>
                )}
              </div>
            </button>
          );
        })}
      </div>

      {/* ── 右侧设置面板 ── */}
      <div className="flex-1 min-h-0 overflow-y-auto p-6 space-y-4">
        {!selectedTool ? (
          <div className="h-full flex flex-col items-center justify-center text-slate-500">
            <Bot className="w-8 h-8 text-slate-700 mb-2" />
            <span className="text-xs font-bold text-slate-400">在左侧选择一个 AI 工具</span>
          </div>
        ) : (
          <>
            {/* 工具信息 + 版本详情 */}
            <div className="p-3 rounded-xl bg-slate-900/30 border border-white/5">
              <div className="flex items-center gap-3">
                <div className="p-2 rounded-lg bg-violet-500/10">
                  <Bot className="w-5 h-5 text-violet-400" />
                </div>
                <div>
                  <h3 className="text-sm font-bold text-white">{selectedTool.display_name}</h3>
                  <div className="flex items-center gap-2 mt-0.5">
                    {selectedTool.installed ? (
                      <>
                        <span className="text-[10px] text-emerald-400"><CheckCircle className="w-3 h-3 inline mr-0.5" />{selectedTool.version || "已安装"}</span>
                        {versionStatuses[selectedTool.id]?.latest && versionStatuses[selectedTool.id]?.status === "outdated" && (
                          <>
                            <span className="text-[10px] text-amber-400 ml-1">→ 最新: {versionStatuses[selectedTool.id].latest}</span>
                            <button
                              onClick={() => handleUpgrade(selectedTool)}
                              disabled={upgradingTool === selectedTool.id}
                              className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-[9px] font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-50"
                              title="升级到最新版"
                            >
                              <Download className={`w-3 h-3 ${upgradingTool === selectedTool.id ? "animate-spin" : ""}`} />
                              {upgradingTool === selectedTool.id ? "升级中..." : "升级"}
                            </button>
                          </>
                        )}
                      </>
                    ) : (
                      <span className="text-[10px] text-slate-500">未安装</span>
                    )}
                    <span className="text-[10px] text-slate-500">· {PROTOCOL_LABELS[selectedTool.api_protocol]}</span>
                    <a href={selectedTool.website} target="_blank" rel="noopener noreferrer"
                      className="text-[10px] text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 ml-1"
                      title="打开官方网站">
                      <ExternalLink className="w-3 h-3" /> 官网
                    </a>
                  </div>
                </div>
              </div>
              {!selectedTool.installed && (
                <div className="mt-3 flex items-center gap-2">
                  <code className="flex-1 text-[10px] text-slate-300 bg-slate-900 rounded px-2 py-1.5 font-mono truncate">{selectedTool.install_cmd}</code>
                  <button onClick={() => navigator.clipboard.writeText(selectedTool.install_cmd)}
                    className="px-2 py-1.5 rounded-md bg-white/5 hover:bg-white/10 text-[10px] text-slate-400 hover:text-white cursor-pointer transition-all flex-shrink-0">
                    <Copy className="w-3.5 h-3.5" />
                  </button>
                </div>
              )}
            </div>

            {/* 仅已安装工具显示以下配置 */}
            {selectedTool.installed && (
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
                      <span className="font-semibold">缓存管理</span>
                      {selectedToolCaches.length > 0 && (
                        <span className="text-[8px] text-slate-500">({selectedToolCaches.length} 个缓存目录)</span>
                      )}
                    </div>
                    <ChevronDown className={`w-3.5 h-3.5 transition-transform ${showCacheManager ? "rotate-180" : ""}`} />
                  </button>

                  {showCacheManager && (
                    <div className="mt-2 rounded-lg border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="max-h-56 overflow-y-auto divide-y divide-white/[0.03]">
                        {cacheInfos.length === 0 ? (
                          <div className="px-3 py-4 text-[10px] text-slate-600 text-center">加载中...</div>
                        ) : selectedToolCaches.length === 0 ? (
                          <div className="px-3 py-4 text-[10px] text-slate-600 text-center">此工具无缓存目录</div>
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
                                  {cache.exists ? cache.full_path : "不存在"}
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
                                    title="打开目录">
                                    <FolderOpen className="w-3 h-3" />
                                  </button>
                                  <button onClick={() => handleMigrateCache(cache.tool_id, cache.dir_name, cache.full_path)}
                                    disabled={migratingCache === `${cache.tool_id}:${cache.dir_name}`}
                                    className="p-1 rounded text-slate-600 hover:text-emerald-400 hover:bg-emerald-500/10 cursor-pointer disabled:opacity-50"
                                    title="迁移缓存">
                                    <FolderSync className="w-3 h-3" />
                                  </button>
                                  <button onClick={() => handleCleanCache(cache.tool_id, cache.dir_name)}
                                    disabled={cleaningCache === `${cache.tool_id}:${cache.dir_name}`}
                                    className="p-1 rounded text-slate-600 hover:text-red-400 hover:bg-red-500/10 cursor-pointer disabled:opacity-50"
                                    title="清理缓存">
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
                        <span className="text-[10px] font-semibold text-blue-300">使用官方模型</span>
                        <p className="text-[8px] text-slate-500 mt-0.5">使用工具的官方 API Key，而不是 AnyVersion 配置的模型</p>
                      </div>
                    </div>
                    <button
                      onClick={() => setUseOfficialModel(!useOfficialModel)}
                      className={`p-1 rounded-md cursor-pointer transition-all ${useOfficialModel ? "text-blue-400" : "text-slate-600 hover:text-slate-400"}`}
                      title={useOfficialModel ? "使用官方模型" : "使用 AnyVersion 模型"}
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
                        <label className="text-xs font-bold text-slate-300 mb-1.5 block">模型供应商</label>
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
                                    <span className="text-[8px] text-slate-600">{group.models.length} 个模型</span>
                                    {providerProtocolBadges(config?.providers.find(p => p.id === group.provider_id))}
                                  </div>
                                  {isSelected && selectedModel && (
                                    <span className="text-[9px] text-violet-400 font-mono truncate ml-2">{selectedModel}</span>
                                  )}
                                </button>
                                {expanded && (
                                  <div className="border-t border-white/[0.03]">
                                    {group.models.map(m => {
                                      const isSelModel = selectedModel === m.id && selectedModelProvider === group.provider_id;
                                      return (
                                        <button key={`${group.provider_id}:${m.id}`}
                                          onClick={() => {
                                            if (isSelModel) { setSelectedModel(""); setSelectedModelProvider(""); }
                                            else { setSelectedModel(m.id); setSelectedModelProvider(group.provider_id); }
                                          }}
                                          className={`w-full text-left px-5 py-1.5 text-[11px] transition-all cursor-pointer flex items-center gap-2 ${
                                            isSelModel
                                              ? "bg-violet-500/10 text-violet-300 font-semibold"
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
                          <div className="mt-1 text-[10px] text-violet-400">已选: <span className="font-mono">{selectedModel}</span> <span className="text-slate-500">（{config?.providers.find(p => p.id === selectedModelProvider)?.name}）</span></div>
                        )}
                      </div>
                    )}

                    {/* 没有可用的供应商/模型时的警告 */}
                    {eligibleProviders.length === 0 && (
                      <div className="p-3 rounded-xl border border-amber-500/20 bg-amber-500/5 text-[10px] text-amber-400 flex items-center gap-2">
                        <AlertTriangle className="w-3.5 h-3.5 flex-shrink-0" />
                        <span>没有可用的模型，请在模型配置中添加 Provider</span>
                      </div>
                    )}

                    {/* 模型伪装（仅当工具内置模型名列表非空） */}
                    {selectedModel && selectedTool.builtin_models.length > 0 && (
                      <div className="mt-3">
                        <label className="text-xs font-bold text-slate-300 mb-1.5 block">伪装模型名称 <span className="text-[9px] text-slate-500 font-normal">（可选）</span></label>
                        <p className="text-[9px] text-slate-500 mb-1.5">让工具以为自己调用以下内置模型，代理实际转发到 <span className="font-mono text-slate-400">{selectedModel}</span></p>
                        <select value={masqueradeModel} onChange={e => setMasqueradeModel(e.target.value)}
                          className="w-full bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-violet-500">
                          <option value="">不伪装（直接使用 {selectedModel}）</option>
                          {selectedTool.builtin_models.map(c => (
                            <option key={c} value={c}>{c}</option>
                          ))}
                        </select>
                      </div>
                    )}
                  </div>
                )}

                {/* Fallback 模型 — 按供应商分组，可折叠 */}
                {selectedTool.supports_fallback_model && selectedTool.installed && !useOfficialModel && fallbackGroups.length > 0 && (
                  <div>
                    <label className="text-xs font-bold text-slate-300 mb-2 block">
                      Fallback 模型
                      <span className="text-[9px] text-slate-500 font-normal ml-1">（处理简单任务，节省费用）</span>
                    </label>
                    <div className="rounded-lg border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="px-3 py-1.5 text-[9px] text-slate-600 font-mono cursor-pointer hover:bg-white/[0.05] border-b border-white/[0.03]"
                        onClick={() => { setSelectedFallbackModel(""); setSelectedFallbackProvider(""); }}>
                        不使用 fallback 模型
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
                                <span className="text-[8px] text-slate-600">{group.models.length} 个</span>
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
                                        if (isSelected) { setSelectedFallbackModel(""); setSelectedFallbackProvider(""); }
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
                        <label className="text-[11px] font-bold text-slate-300 mb-1.5 block">Fallback 伪装名称 <span className="text-[9px] text-slate-500 font-normal">（可选）</span></label>
                        <p className="text-[9px] text-slate-500 mb-1.5">让工具以为 fallback 调用以下内置模型，代理实际转发到 <span className="font-mono text-slate-400">{selectedFallbackModel}</span></p>
                        <select value={fallbackMasqueradeModel} onChange={e => setFallbackMasqueradeModel(e.target.value)}
                          className="w-full bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-violet-500">
                          <option value="">不伪装（直接使用 {selectedFallbackModel}）</option>
                          {selectedTool.builtin_models.map(c => (
                            <option key={c} value={c}>{c}</option>
                          ))}
                        </select>
                      </div>
                    )}
                    {selectedFallbackModel && (
                      <div className="mt-1 text-[10px] text-amber-400">Fallback: <span className="font-mono">{selectedFallbackModel}</span>{fallbackMasqueradeModel && <>(伪装为 <span className="font-mono">{fallbackMasqueradeModel}</span>)</>}</div>
                    )}
                  </div>
                )}

                {/* 1M Context Toggle — 由 config.json 的 supportOneMContext 字段驱动 */}
                {selectedTool.supports_model && selectedTool.support_one_m_context && (
                  <div className="flex items-center justify-between p-2.5 rounded-lg bg-slate-900/30 border border-white/5">
                    <div className="flex items-center gap-2">
                      <span className="text-[10px] font-semibold text-slate-300">1M Context</span>
                      <span className="text-[8px] text-slate-500 hidden sm:inline">给模型 ID 追加 [1m] 后缀</span>
                    </div>
                    <button
                      onClick={() => setOneMContext(!oneMContext)}
                      className={`p-1 rounded-md cursor-pointer transition-all ${oneMContext ? "text-violet-400" : "text-slate-600 hover:text-slate-400"}`}
                    >
                      {oneMContext ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                    </button>
                  </div>
                )}

                {/* 代理增强能力（优化器 / 整流器）— 默认沿用全局配置，可在此按启动覆盖 */}
                {selectedTool.supports_model && !useOfficialModel && (selectedTool.supports_optimizer || selectedTool.supports_rectifier) && (
                  <div className="space-y-2">
                    {selectedTool.supports_optimizer && (
                      <div className="rounded-lg bg-slate-900/30 border border-white/5 overflow-hidden">
                        <div className="flex items-center justify-between p-2.5">
                          <div className="flex items-center gap-2">
                            <span className="text-[10px] font-semibold text-slate-300">请求优化器</span>
                            <span className="text-[8px] text-slate-500 hidden sm:inline">缓存注入 / 思考优化 / DeepSeek 归一</span>
                          </div>
                          <button onClick={() => setOptimizerEnabled(!optimizerEnabled)}
                            className={`p-1 rounded-md cursor-pointer transition-all ${optimizerEnabled ? "text-violet-400" : "text-slate-600 hover:text-slate-400"}`}>
                            {optimizerEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                          </button>
                        </div>
                        {optimizerEnabled && (
                          <div className="px-3 pb-2.5 space-y-1.5 border-t border-white/5 pt-2">
                            {[
                              { key: "cache_injection" as const, label: "Prompt 缓存注入", desc: "注入 cache_control 断点降低 API 费用" },
                              { key: "thinking_optimizer" as const, label: "Thinking 参数优化", desc: "按模型自适应配置 thinking 参数" },
                              { key: "deepseek_normalize" as const, label: "DeepSeek 兼容", desc: "为 DeepSeek/Moonshot 等端点规范化 thinking 块" },
                            ].map(item => (
                              <label key={item.key} className="flex items-center gap-2 cursor-pointer">
                                <input
                                  type="checkbox"
                                  checked={optimizerStrategies[item.key]}
                                  onChange={() => setOptimizerStrategies(prev => ({ ...prev, [item.key]: !prev[item.key] }))}
                                  className="accent-violet-500"
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
                            <span className="text-[10px] font-semibold text-slate-300">协议整流器</span>
                            <span className="text-[8px] text-slate-500 hidden sm:inline">抹平协议差异</span>
                          </div>
                          <button onClick={() => setRectifierEnabled(!rectifierEnabled)}
                            className={`p-1 rounded-md cursor-pointer transition-all ${rectifierEnabled ? "text-violet-400" : "text-slate-600 hover:text-slate-400"}`}>
                            {rectifierEnabled ? <ToggleRight className="w-6 h-6" /> : <ToggleLeft className="w-6 h-6" />}
                          </button>
                        </div>
                        {rectifierEnabled && (
                          <div className="px-3 pb-2.5 space-y-1.5 border-t border-white/5 pt-2">
                            {[
                              { key: "thinking_signature" as const, label: "Thinking 签名修复", desc: "thinking block 签名无效时自动剥离并重试" },
                              { key: "thinking_budget" as const, label: "Thinking 预算修复", desc: "budget_tokens 太小时自动修正为 32000" },
                              { key: "media_fallback" as const, label: "图片降级", desc: "上游不支持图片时自动替换为文本标记" },
                              { key: "protocol_mismatch" as const, label: "协议不匹配修复", desc: "协议转换后残留专有字段被上游拒绝时自动剥离并重试" },
                            ].map(item => (
                              <label key={item.key} className="flex items-center gap-2 cursor-pointer">
                                <input
                                  type="checkbox"
                                  checked={rectifierStrategies[item.key]}
                                  onChange={() => setRectifierStrategies(prev => ({ ...prev, [item.key]: !prev[item.key] }))}
                                  className="accent-violet-500"
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
                    <label className="text-xs font-bold text-slate-300">会话</label>
                    {sessions.length > 0 && (
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => setSessionViewMode(sessionViewMode === "flat" ? "grouped" : "flat")}
                          className="p-1 rounded text-slate-500 hover:text-slate-300 cursor-pointer transition-all"
                          title={sessionViewMode === "flat" ? "分组视图" : "列表视图"}
                        >
                          {sessionViewMode === "flat" ? <ListTree className="w-3.5 h-3.5" /> : <List className="w-3.5 h-3.5" />}
                        </button>
                        <button
                          onClick={() => { setSelectionMode(!selectionMode); setSelectedSessionIds(new Set()); }}
                          className={`p-1 rounded cursor-pointer transition-all ${selectionMode ? "text-violet-400" : "text-slate-500 hover:text-slate-300"}`}
                        >
                          <CheckCircle className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    )}
                  </div>

                  <div className="flex gap-2 flex-wrap mb-2">
                    <button onClick={() => { setSessionMode("new"); setSelectedSession(null); setShowSessionPicker(false); }}
                      className={`px-3 py-1.5 rounded-lg text-[10px] font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                        sessionMode === "new" ? "bg-violet-600 text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                      }`}>
                      使用新会话
                    </button>
                    {sessions.length > 0 && (
                      <button onClick={() => { setSessionMode("resume"); setShowSessionPicker(!showSessionPicker); setSelectedSession(null); }}
                        className={`px-3 py-1.5 rounded-lg text-[10px] font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                          sessionMode === "resume" ? "bg-violet-600 text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                        }`}>
                        <Clock className="w-3 h-3" /> 历史会话 ({sessions.length})
                      </button>
                    )}
                  </div>

                  {showSessionPicker && sessionMode === "resume" && (
                    <div className="mb-2">
                      <div className="flex items-center gap-2">
                        <div className="flex-1 relative">
                          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500" />
                          <input value={sessionSearch} onChange={e => setSessionSearch(e.target.value)}
                            placeholder="搜索会话..." className="w-full bg-slate-900 border border-white/10 rounded-lg pl-7 pr-7 py-1.5 text-[10px] text-slate-200 focus:outline-none focus:border-violet-500" />
                          {sessionSearch && (
                            <button onClick={() => setSessionSearch("")} className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300">
                              <X className="w-3 h-3" />
                            </button>
                          )}
                        </div>
                        {selectionMode && (
                          <>
                            <button onClick={handleSelectAll} className="px-2 py-1 rounded text-[9px] font-semibold bg-white/5 text-slate-400 hover:text-slate-200 cursor-pointer whitespace-nowrap">
                              {selectedSessionIds.size === filteredSessions.length ? "取消全选" : "全选"}
                            </button>
                            <button onClick={handleDeleteSessions} disabled={selectedSessionIds.size === 0}
                              className="px-2 py-1 rounded text-[9px] font-semibold bg-red-500/10 text-red-400 hover:bg-red-500/20 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed whitespace-nowrap flex items-center gap-1">
                              <Trash2 className="w-3 h-3" /> 删除 ({selectedSessionIds.size})
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
                            {sessionSearch ? "无匹配的会话" : "暂无历史会话"}
                          </div>
                        ) : sessionViewMode === "flat" ? (
                          filteredSessions.map(s => (
                            <div key={s.session_id}
                              className={`flex items-center px-3 py-2 text-[10px] transition-all group ${
                                selectedSession?.session_id === s.session_id ? "bg-violet-500/10 text-violet-300" : "text-slate-400 hover:bg-white/[0.03] hover:text-slate-200"
                              }`}>
                              {selectionMode && (
                                <button onClick={() => toggleSessionSelect(s.session_id)}
                                  className={`mr-2 w-4 h-4 rounded border flex-shrink-0 flex items-center justify-center cursor-pointer ${
                                    selectedSessionIds.has(s.session_id) ? "bg-violet-500 border-violet-500 text-white" : "border-slate-700 hover:border-slate-500"
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
                                    selectedSession?.session_id === s.session_id ? "bg-violet-500/10 text-violet-300" : "text-slate-400 hover:bg-white/[0.03] hover:text-slate-200"
                                  }`}>
                                  {selectionMode && (
                                    <button onClick={() => toggleSessionSelect(s.session_id)}
                                      className={`mr-2 w-3.5 h-3.5 rounded border flex-shrink-0 flex items-center justify-center cursor-pointer ${
                                        selectedSessionIds.has(s.session_id) ? "bg-violet-500 border-violet-500 text-white" : "border-slate-700 hover:border-slate-500"
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
                    <div className="mt-2 p-2 rounded-lg bg-violet-500/5 border border-violet-500/15 text-[10px] text-violet-300 flex items-center gap-2">
                      <CheckCircle className="w-3 h-3 flex-shrink-0" />
                      <span className="truncate">将恢复 <span className="font-mono">{selectedSession.project_path}</span></span>
                    </div>
                  )}
                </div>

                {/* 项目目录 */}
                {sessionMode === "new" && (
                  <div>
                    <label className="text-xs font-bold text-slate-300 mb-2 block">项目目录</label>
                    <div className="flex gap-2">
                      <input value={projectPath} onChange={e => setProjectPath(e.target.value)} placeholder="选择或输入项目目录..."
                        className="flex-1 bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-violet-500" />
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
                    <label className="text-xs font-bold text-slate-300 mb-2 block">终端</label>
                    <select value={selectedTerminal} onChange={e => setSelectedTerminal(e.target.value)}
                      className="w-full bg-slate-900 border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 focus:outline-none focus:border-violet-500">
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
                    <div className="p-2.5 rounded-lg bg-violet-500/5 border border-violet-500/15 text-[10px] flex flex-col gap-1.5">
                      <div className="flex items-center gap-2 flex-wrap">
                        <Shield className="w-3.5 h-3.5 text-violet-400 flex-shrink-0" />
                        <span className="text-slate-300">
                          入站 <span className="font-semibold text-violet-300">{PROTOCOL_LABELS[proxyInfo.inbound]}</span>
                          <span className="mx-1 text-slate-500">→</span>
                          出站 <span className="font-semibold text-violet-300">{PROTOCOL_LABELS[proxyInfo.outbound]}</span>
                        </span>
                        {proxyInfo.converted ? (
                          <span className="px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-300 text-[9px] font-semibold">自动转换</span>
                        ) : (
                          <span className="px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-300 text-[9px] font-semibold">同协议直连</span>
                        )}
                        <span className="px-1.5 py-0.5 rounded bg-blue-500/15 text-blue-300 text-[9px] font-semibold">统计已开启</span>
                        {selectedTool.supports_optimizer && optimizerEnabled && config?.optimizer.enabled && (
                          <span className="px-1.5 py-0.5 rounded bg-violet-500/15 text-violet-300 text-[9px] font-semibold">优化器</span>
                        )}
                        {selectedTool.supports_rectifier && rectifierEnabled && config?.rectifier.enabled && (
                          <span className="px-1.5 py-0.5 rounded bg-violet-500/15 text-violet-300 text-[9px] font-semibold">整流器</span>
                        )}
                      </div>
                      {proxyInfo.aliasEntries.length > 0 && (
                        <div className="flex items-center gap-1.5 flex-wrap text-slate-400">
                          <span className="text-slate-500">伪装:</span>
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
                  className="w-full py-3 rounded-xl bg-violet-600 hover:bg-violet-500 disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-bold flex items-center justify-center gap-2 cursor-pointer transition-all shadow-lg shadow-violet-500/20">
                  {launching ? (
                    <><RefreshCw className="w-4 h-4 animate-spin" /> 启动中...</>
                  ) : sessionMode === "resume" && selectedSession ? (
                    <><Play className="w-4 h-4" /> 恢复会话</>
                  ) : (
                    <><Rocket className="w-4 h-4" /> 启动 {selectedTool.display_name}</>
                  )}
                </button>
              </>
            )}

            {launchResult && (
              <div className={`p-3 rounded-xl text-xs flex items-start gap-2 ${
                launchResult.ok ? "bg-emerald-500/10 border border-emerald-500/20 text-emerald-400" : "bg-red-500/10 border border-red-500/20 text-red-400"
              }`}>
                {launchResult.ok ? <CheckCircle className="w-4 h-4 flex-shrink-0 mt-0.5" /> : <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />}
                <span className="whitespace-pre-line">{launchResult.msg}</span>
              </div>
            )}

            {upgradeResult && upgradeResult.id === selectedTool?.id && (
              <div className={`p-3 rounded-xl text-xs flex items-start gap-2 ${
                upgradeResult.msg.includes("成功") ? "bg-emerald-500/10 border border-emerald-500/20 text-emerald-400" : "bg-red-500/10 border border-red-500/20 text-red-400"
              }`}>
                {upgradeResult.msg.includes("成功") ? <CheckCircle className="w-4 h-4 flex-shrink-0 mt-0.5" /> : <AlertTriangle className="w-4 h-4 flex-shrink-0 mt-0.5" />}
                <span className="whitespace-pre-line">{upgradeResult.msg}</span>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
