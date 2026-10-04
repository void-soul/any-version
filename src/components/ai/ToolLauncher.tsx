import React, { Fragment, useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { ConfirmDialogHost, type ConfirmRequest } from "../shared/ConfirmDialog";
import { Note, ResultNote } from "../shared/Note";
import { useToolListWidth } from "./paneWidth";
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
  Cpu, Check, History, Terminal, GitFork,
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
  CodexPluginMarketplaceStatus,
  CodexPluginInfo,
  ClaudePluginInfo,
  ClaudePluginStatus,
} from "./types";
import { alertError } from "../shared/ThemedAlert";

/**
 * 插件列表的统一行结构。
 * Codex 与 Claude 两套后端的字段名不同，逻辑层抹平后 UI 只认这一种（不用维护两套列表）。
 */
type PluginRow = {
  /** 安装 / 卸载的寻址键（codex = 插件名；claude = `插件@市场`） */
  key: string;
  title: string;
  group: string;
  desc: string;
  installed: boolean;
  enabled: boolean;
  version: string | null;
};

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
export function getProxyInfo(
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
  /** 真实模型名（常驻显示） */
  model: string;
  /** 实际生效的伪装名；null = 没配伪装 / 别名与真实名相同 */
  alias: string | null;
  /** fallback 小模型的伪装映射，只有真发生伪装才有 */
  fallbackAliases: [string, string][];
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
  // 伪装映射 C → B。
  //
  // `model` / `alias` 是**常驻**的：底部要一直告诉用户「用什么模型、伪装什么模型」。
  // 之前只渲染 `伪装映射` 徽章行，且别名等于真实名时整行隐藏 —— 于是「没配伪装」的
  // 常见场景下底部一个字都不显示，用户完全看不出当前用的是哪个模型。
  const real = selectedModel || "";
  // 别名由后端 `effective_claimed_model` 解析（Claude Desktop 留空时它会给一个合法别名）
  const alias = masqueradeModel && masqueradeModel !== real ? masqueradeModel : null;
  // fallback 小模型：只有真发生伪装才列出来，避免 `x → x` 噪音
  const fallbackAliases: [string, string][] = [];
  if (fallbackModel) {
    const claimedFb = fallbackMasqueradeModel || fallbackModel;
    if (claimedFb !== fallbackModel) {
      fallbackAliases.push([claimedFb, fallbackModel]);
    }
  }
  return {
    inbound,
    outbound,
    converted: inbound !== outbound,
    model: real,
    alias,
    fallbackAliases,
  };
}

/** 工具配置文件里当前写定的模型（后端 `get_ai_tool_models` 的返回）。 */
export interface AppliedModels {
  model: string | null;
  fallback_model: string | null;
}

/** 在仓库里按模型值反查 (供应商, 模型)。
 *
 * 先精确匹配 `m.id`；再容忍 `前缀/模型` 形态 —— 有些工具写的值是 `provider/model`，
 * 直接 `===` 会认不出来，界面就回显成「未选择」。 */
function findModelRef(
  providers: AiProvider[],
  modelValue: string | null | undefined,
): { providerId: string; modelId: string } | null {
  const wanted = (modelValue ?? "").trim();
  if (!wanted) return null;
  for (const p of providers) {
    for (const m of p.models) {
      if (m.id === wanted) return { providerId: p.id, modelId: m.id };
    }
  }
  const tail = wanted.split('/').pop() ?? "";
  if (tail && tail !== wanted) {
    for (const p of providers) {
      for (const m of p.models) {
        if (m.id === tail) return { providerId: p.id, modelId: m.id };
      }
    }
  }
  return null;
}

/** 未安装区的形态筛选：全部 / CLI / 桌面端（抄 EchoBird 的分组维度）。
 *  已安装区不分组——装了的就那么几个，分组只会让人多找一层。 */
type ToolKindFilter = "all" | "cli" | "desktop";

/** 可折叠的设置卡片。
 *
 * 折叠后必须靠 `summary` 保留关键信息（例如「当前选了哪个模型」）——
 * 收起来就看不到设了什么，用户还得展开确认，等于没省事。
 * `action` 用于标题行右侧的常驻控件（如优化器开关）：它不能放进折叠按钮里，
 * 否则 button 套 button 是非法 HTML。 */
function CollapsibleCard({
  title, hint, summary, open, onToggle, action, children,
}: {
  title: string;
  hint?: string;
  summary?: React.ReactNode;
  open: boolean;
  onToggle: () => void;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="flex items-center gap-1.5 mb-1.5">
        <button onClick={onToggle} className="flex items-center gap-1.5 min-w-0 cursor-pointer text-left">
          <ChevronRight className={`w-3 h-3 text-slate-500 transition-transform flex-shrink-0 ${open ? "rotate-90" : ""}`} />
          <span className="text-body font-bold text-slate-300 truncate">{title}</span>
        </button>
        {hint && open && <span className="text-micro text-slate-500 truncate">{hint}</span>}
        {!open && summary && (
          <span className="text-micro text-slate-500 ml-auto truncate">{summary}</span>
        )}
        {action && <div className={!open && summary ? "" : "ml-auto"}>{action}</div>}
      </div>
      {open && children}
    </div>
  );
}

export default function ToolLauncher({ onAskAssistant }: { onAskAssistant?: (question: string) => void } = {}) {
  const { t } = useTranslation();
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
  const [sessions, setSessions] = useState<ToolSession[]>([]);
  const [loading, setLoading] = useState(true);

  const [selectedToolId, setSelectedToolId] = useState<string | null>(null);
  const [selectedModel, setSelectedModel] = useState("");
  const [selectedModelProvider, setSelectedModelProvider] = useState("");
  // 后端解析出的**实际生效**声明名（Rust 的 effective_claimed_model）。
  // Claude Desktop 留空伪装名时后端会自动给合法别名，前端必须显示那个而不是空 ——
  // 别名规则只在 Rust 一处实现，前端复制就会漂移。取不到时退回手填值（旧行为）。
  const [effectiveAlias, setEffectiveAlias] = useState("");
  // 插件市场默认收起：它自带搜索框 + 插件清单，展开时占掉大半屏；
  // 收起后靠 CollapsibleCard 的 summary 仍能看到「已安装 · 几个插件」。
  const [marketOpen, setMarketOpen] = useState(false);
  // 当前模型自定义启动参数的取值（param key → 用户选的值）
  const [customParamValues, setCustomParamValues] = useState<Record<string, string>>({});
  const [projectPath, setProjectPath] = useState("");
  const [selectedTerminal, setSelectedTerminal] = useState("cmd");
  const [sessionMode, setSessionMode] = useState<"new" | "continue" | "resume" | "fork">("new");
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
    thinking_signature: true, thinking_budget: true, media_fallback: true,
    media_heuristic: true, protocol_mismatch: true,
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
  // Codex 官方插件市场（写 ~/.codex/config.toml 的 [marketplaces.*]）。
  // CodexPlusPlus 同样的思路：市场内容我们自己落地 + 注册，单个插件由客户端自己装。
  const [marketplace, setMarketplace] = useState<CodexPluginMarketplaceStatus | null>(null);
  const [marketplaceBusy, setMarketplaceBusy] = useState(false);
  // 插件市场里的插件清单（逐个安装/卸载走官方 CLI）
  const [marketplacePlugins, setMarketplacePlugins] = useState<CodexPluginInfo[]>([]);
  // Claude Code 那套（`claude plugin`）：市场要自己添加、插件 id 带 @市场
  const [claudeStatus, setClaudeStatus] = useState<ClaudePluginStatus | null>(null);
  const [claudePlugins, setClaudePlugins] = useState<ClaudePluginInfo[]>([]);
  // 单个插件的忙态：按插件名记，避免一个在装时把整列按钮都禁用
  const [pluginBusy, setPluginBusy] = useState<Record<string, boolean>>({});
  const [pluginQuery, setPluginQuery] = useState("");
  // 各设置卡片的折叠状态：默认只展开「模型供应商」（选模型是主操作），
  // 其余收起 —— 全部铺开会把启动按钮顶到看不见的地方。
  const [vendorOpen, setVendorOpen] = useState(true);
  const [fallbackOpen, setFallbackOpen] = useState(false);
  const [optimizerOpen, setOptimizerOpen] = useState(false);
  const [rectifierOpen, setRectifierOpen] = useState(false);

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
        invoke<AiConfig>("get_ai_config").catch(() => ({ providers: [], proxy_port: 15721, default_project_path: "", rectifier: { enabled: false, thinking_signature: false, thinking_budget: false, media_fallback: false, media_heuristic: false, protocol_mismatch: false }, headroom: { enabled: false, port: 8791, on_unavailable: "failOpen", disable_kompress: false, timeout_ms: 1500 }, optimizer: { enabled: false, cache_injection: false, thinking_optimizer: false, deepseek_normalize: false }, skills_dir: "" })),
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
          project_path: (sessionMode === "resume" || sessionMode === "fork") && selectedSession ? selectedSession.project_path : projectPath,
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

  /**
   * 读取插件市场状态（按工具声明的后端分派：`codex` 或 `claude`）。
   * 未安装 / 未注册都只是状态，不是错误。
   */
  const loadMarketplace = useCallback(async (tool: DetectedAiTool) => {
    if ((tool.plugin_marketplace_kind ?? "codex") === "claude") {
      try {
        const status = await invoke<ClaudePluginStatus>("claude_plugin_status");
        setClaudeStatus(status);
        // 一个市场都没配就列不出插件，跳过省一次调用
        setClaudePlugins(
          status.cliAvailable && status.marketplaces.length > 0
            ? await invoke<ClaudePluginInfo[]>("claude_list_plugins").catch(() => [])
            : []
        );
      } catch {
        setClaudeStatus(null);
        setClaudePlugins([]);
      }
      return;
    }
    try {
      const status = await invoke<CodexPluginMarketplaceStatus>("codex_plugin_marketplace_status");
      setMarketplace(status);
      // 市场没落盘时列插件只会得到空表，跳过省一次调用
      setMarketplacePlugins(
        status.installed
          ? await invoke<CodexPluginInfo[]>("codex_list_marketplace_plugins").catch(() => [])
          : []
      );
    } catch {
      setMarketplace(null);
      setMarketplacePlugins([]);
    }
  }, []);

  /**
   * 市场级操作。
   *
   * - **codex**：下载官方市场包 → 落盘并改名 → 注册进 config.toml（幂等更新）。
   * - **claude**：市场来自仓库，不存在「官方内置包」这回事 → 这里是「添加官方市场来源」。
   */
  const installMarketplace = async (tool: DetectedAiTool) => {
    setMarketplaceBusy(true);
    try {
      if ((tool.plugin_marketplace_kind ?? "codex") === "claude") {
        const source = claudeStatus?.officialMarketplaceSource ?? "anthropics/skills";
        await invoke<string[]>("claude_plugin_marketplace_add", { source });
      } else {
        // 不额外弹提示：卡片本身会切成「已安装 N 个插件」，那才是用户要看的反馈
        await invoke<CodexPluginMarketplaceStatus>("codex_install_plugin_marketplace");
      }
      await loadMarketplace(tool);
    } catch (e) {
      alertError(String(e));
    } finally {
      setMarketplaceBusy(false);
    }
  };

  /**
   * 撤销托管。
   * - **codex**：摘掉 config.toml 的注册并删除落盘目录。
   * - **claude**：移除指定的市场配置（不传 name 时移除第一个）。
   */
  const removeMarketplace = async (tool: DetectedAiTool, name?: string) => {
    setMarketplaceBusy(true);
    try {
      if ((tool.plugin_marketplace_kind ?? "codex") === "claude") {
        const target = name ?? claudeStatus?.marketplaces[0];
        if (!target) return;
        await invoke<string[]>("claude_plugin_marketplace_remove", { name: target });
      } else {
        await invoke<CodexPluginMarketplaceStatus>("codex_remove_plugin_marketplace");
      }
      await loadMarketplace(tool);
    } catch (e) {
      alertError(String(e));
    } finally {
      setMarketplaceBusy(false);
    }
  };

  /**
   * 单个插件的安装 / 卸载：一律走官方 CLI。
   *
   * CLI 除了写配置，还会把插件落到客户端的插件缓存并处理各自策略（Codex 的 `authPolicy`、
   * Claude 的 `enabledPlugins`）—— 只写配置文件的话插件在客户端里是「看得见、用不了」。
   */
  const togglePlugin = async (tool: DetectedAiTool, key: string, install: boolean) => {
    setPluginBusy((m) => ({ ...m, [key]: true }));
    try {
      if ((tool.plugin_marketplace_kind ?? "codex") === "claude") {
        const list = await invoke<ClaudePluginInfo[]>(
          install ? "claude_install_plugin" : "claude_uninstall_plugin",
          { plugin: key }
        );
        setClaudePlugins(list);
        setClaudeStatus(await invoke<ClaudePluginStatus>("claude_plugin_status"));
      } else {
        const list = await invoke<CodexPluginInfo[]>(
          install ? "codex_install_plugin" : "codex_uninstall_plugin",
          { name: key }
        );
        setMarketplacePlugins(list);
        setMarketplace(
          await invoke<CodexPluginMarketplaceStatus>("codex_plugin_marketplace_status")
        );
      }
    } catch (e) {
      alertError(String(e));
    } finally {
      setPluginBusy((m) => ({ ...m, [key]: false }));
    }
  };

  /** 当前工具用的是哪套插件后端（未声明按 codex）。 */
  const marketKind: "codex" | "claude" =
    (selectedTool?.plugin_marketplace_kind ?? "codex") === "claude" ? "claude" : "codex";

  /** 市场是否就绪（codex = 已落盘；claude = 至少配了一个市场）。 */
  const marketReady =
    marketKind === "claude"
      ? (claudeStatus?.marketplaces.length ?? 0) > 0
      : !!marketplace?.installed;

  /** CLI 是否可用 —— 不可用时装/卸按钮要置灰，而不是让用户点了报错。 */
  const marketCliAvailable =
    marketKind === "claude"
      ? claudeStatus?.cliAvailable !== false
      : marketplace?.cliAvailable !== false;

  /**
   * 插件清单统一成一种行结构后按「市场 / 分类」分组 + 关键字过滤。
   * 两套后端字段名不同（Codex 用 `name`+`category`，Claude 用 `id`+`marketplace`+`description`），
   * 在这里抹平，UI 只认一种。
   */
  const pluginGroups: [string, PluginRow[]][] = (() => {
    const rows: PluginRow[] =
      marketKind === "claude"
        ? claudePlugins.map((p) => ({
            key: p.id,
            title: p.name,
            group: p.marketplace || t("toollaunch.pluginCategoryOther"),
            desc: p.description,
            installed: p.installed,
            enabled: p.enabled,
            version: p.version ?? null,
          }))
        : marketplacePlugins.map((p) => ({
            key: p.name,
            title: p.name,
            group: p.category || t("toollaunch.pluginCategoryOther"),
            desc: "",
            installed: p.installed,
            enabled: p.enabled,
            version: p.version ?? null,
          }));
    const q = pluginQuery.trim().toLowerCase();
    const filtered = q
      ? rows.filter(
          (r) =>
            r.title.toLowerCase().includes(q) ||
            r.group.toLowerCase().includes(q) ||
            r.desc.toLowerCase().includes(q)
        )
      : rows;
    const groups = new Map<string, PluginRow[]>();
    for (const r of filtered) {
      const bucket = groups.get(r.group);
      if (bucket) bucket.push(r);
      else groups.set(r.group, [r]);
    }
    return [...groups.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  })();

  /** 打开卸载确认：先刷新数据目录清单，再把该工具实际占用的目录逐条列出来 */
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
              <div className="text-tiny text-slate-400">{t("toollaunch.dataDirsHint")}</div>
              <div className="max-h-32 overflow-y-auto rounded-ctl border border-white/10 bg-black/30 divide-y divide-white/5">
                {dirs.map((d) => (
                  <div key={d.dir_name} className="flex items-center gap-2 px-2 py-1.5 text-tiny">
                    <FolderOpen className="w-3 h-3 flex-shrink-0 text-slate-500" />
                    <span className="min-w-0 flex-1 break-all text-slate-300">{d.full_path}</span>
                    <span className="flex-shrink-0 text-slate-500">{d.size}</span>
                  </div>
                ))}
              </div>
              <label className="flex items-start gap-2 text-tiny text-slate-300 cursor-pointer select-none">
                <input
                  type="checkbox"
                  checked={removeDataDirs}
                  onChange={(e) => toggleRemoveDataDirs(e.target.checked)}
                  className="mt-0.5 cursor-pointer"
                />
                <span>{t("toollaunch.removeDataDirsCheckbox", { count: dirs.length })}</span>
              </label>
              <div className="text-micro text-slate-500">{t("toollaunch.removeDataDirsHint")}</div>
            </>
          ) : (
            <div className="text-tiny text-slate-500">{t("toollaunch.noDataDirs")}</div>
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
    } catch (e: any) { alertError(t("toollaunch.migrateFail", { err: String(e) })); }
    finally { setMigratingCache(null); }
  };

  const handleCleanCache = async (toolId: string, dirName: string) => {
    if (!confirm(t("toollaunch.clearCacheConfirm", { name: dirName }))) return;
    setCleaningCache(`${toolId}:${dirName}`);
    try {
      await invoke("clean_ai_tool_cache", { toolId, dirName });
      await loadCacheInfos();
    } catch (e: any) { alertError(t("toollaunch.clearFail", { err: String(e) })); }
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
    return <div className="h-full flex items-center justify-center text-slate-500"><RefreshCw className="w-5 h-5 animate-spin mr-2" /><span className="text-body">{t("toollaunch.loading")}</span></div>;
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

  // 桌面工具与项目目录无关（后端会用 exe 所在目录当工作目录），不该因为它被卡住
  const canLaunch = !!selectedTool?.installed
    && (selectedTool.tool_kind === "desktop" || sessionMode === "resume" || sessionMode === "fork" || !!projectPath)
    // 分叉必须挑一条会话（没有源会话就无从复制）
    && (sessionMode !== "fork" || !!selectedSession);

  // 本次启动**实际会用第三方模型**：没勾「使用官方模型」**且**确实选了一个模型。
  //
  // 只勾掉官方开关、但一个模型都没选时，后端拿到的 `model_id` 是 null，
  // 走的仍然是工具的官方配置 —— 此时优化器 / 整流器没有任何作用，不该显示。
  const usingThirdPartyModel = !useOfficialModel && !!selectedModel;

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
                  media_heuristic: config?.rectifier?.media_heuristic !== false,
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
                // 再以**工具配置文件**为准回显一次：它比「上次启动配置」权威 ——
                // 工具真正读的是自己的配置文件，用户也可能在别处改过它。
                // 只信自己的记录会出现「看着像用官方配置、实际还在用自定义模型」。
                setConfigAppliedModel(null);
                setConfigAppliedFallback(null);
                if (tool.config_file) {
                  try {
                    const applied = await invoke<AppliedModels>("get_ai_tool_models", { toolId: tool.id });
                    setConfigAppliedModel(applied.model ?? null);
                    setConfigAppliedFallback(applied.fallback_model ?? null);
                    const providers = config?.providers ?? [];
                    const mainRef = findModelRef(providers, applied.model);
                    if (mainRef) {
                      setUseOfficialModel(false);
                      setSelectedModelProvider(mainRef.providerId);
                      setSelectedModel(mainRef.modelId);
                    }
                    const fbRef = findModelRef(providers, applied.fallback_model);
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
                    {lastLaunchConfigs[tool.id].use_official_model ? (
                      <span className="text-micro">{t("toollaunch.official")}</span>
                    ) : (
                      <>
                        <span className="text-micro truncate max-w-[60px]">
                          {lastLaunchConfigs[tool.id].provider_name || lastLaunchConfigs[tool.id].provider_id || "-"}
                        </span>
                        {lastLaunchConfigs[tool.id].model_id && (
                          <span className="text-micro truncate max-w-[50px] opacity-70">
                            · {lastLaunchConfigs[tool.id].model_id}
                          </span>
                        )}
                        {lastLaunchConfigs[tool.id].fallback_model_id && (
                          <span className="text-micro text-amber-400/80 truncate max-w-[50px]">
                            ※ {lastLaunchConfigs[tool.id].fallback_model_id}
                          </span>
                        )}
                      </>
                    )}
                    {lastLaunchConfigs[tool.id].last_launched_at && (
                      <span className="text-micro opacity-50 ml-auto">
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
              <div className="flex items-center gap-3">
                <div className="p-2 rounded-ctl bg-[var(--module-accent-soft)]">
                  <Bot className="w-5 h-5 text-[var(--module-accent)]" />
                </div>
                <div>
                  <h3 className="text-sm font-bold text-white">{selectedTool.display_name}</h3>
                  <div className="flex items-center gap-2 mt-0.5">
                    {getBusy(selectedTool.id) && (
                      <span className="flex items-center gap-1 text-tiny font-semibold text-blue-300">
                        <RefreshCw className="w-3 h-3 animate-spin" />
                        {getBusy(selectedTool.id) === "upgrading" ? `${t("toollaunch.upgrading")}...` : getBusy(selectedTool.id) === "installing" ? `${t("toollaunch.installing")}...` : `${t("toollaunch.uninstalling")}...`}
                      </span>
                    )}
                    {selectedTool.installed ? (
                      <>
                        <span className="text-tiny text-emerald-400"><CheckCircle className="w-3 h-3 inline mr-0.5" />{selectedTool.version || t("toollaunch.installed")}</span>
                        {!selectedTool.pm_managed && (
                          // 只是提示，不拦操作：升级/卸载照旧可用（包管理器 → 官方渠道 → 按文件清理）
                          <span
                            className="text-tiny text-amber-400/80 cursor-help"
                            title={selectedTool.detected_path || selectedTool.uninstall_cmd || selectedTool.upgrade_cmd || undefined}
                          >
                            {t("toollaunch.externalInstallHint")}
                          </span>
                        )}
                        {!getBusy(selectedTool.id) && versionStatuses[selectedTool.id]?.latest && versionStatuses[selectedTool.id]?.status === "outdated" && (
                          <>
                            <span className="text-tiny text-amber-400 ml-1">→ {t("toollaunch.latest")}: {versionStatuses[selectedTool.id].latest}</span>
                            <button
                              onClick={() => handleUpgrade(selectedTool)}
                              disabled={getBusy(selectedTool.id) === "upgrading"}
                              className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-micro font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-50"
                              title={t("toollaunch.upgradeLatest")}
                            >
                              <Download className={`w-3 h-3 ${getBusy(selectedTool.id) === "upgrading" ? "animate-spin" : ""}`} />
                              {getBusy(selectedTool.id) === "upgrading" ? `${t("toollaunch.upgrading")}...` : t("toollaunch.upgrade")}
                            </button>
                            <button
                              onClick={() => void askUninstall(selectedTool)}
                              disabled={getBusy(selectedTool.id) === "uninstalling"}
                              className="px-2 py-0.5 rounded-md bg-red-500/10 hover:bg-red-500/20 text-micro font-semibold text-red-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-50"
                              title={t("toollaunch.uninstallTitle")}
                            >
                              <Trash2 className={`w-3 h-3 ${getBusy(selectedTool.id) === "uninstalling" ? "animate-spin" : ""}`} />
                              {getBusy(selectedTool.id) === "uninstalling" ? `${t("toollaunch.uninstalling")}...` : t("toollaunch.uninstall")}
                            </button>
                          </>
                        )}
                      </>
                    ) : (
                      <span className="text-tiny text-slate-500">{t("toollaunch.notInstalled")}</span>
                    )}
                    <span className="text-tiny text-slate-500">· {selectedTool.api_protocol === "none" ? t("toollaunch.modelNone") : PROTOCOL_LABELS[selectedTool.api_protocol]}</span>
                    <a href={selectedTool.website} target="_blank" rel="noopener noreferrer"
                      onClick={(e) => { e.preventDefault(); void openUrl(selectedTool.website); }}
                      className="text-tiny text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 ml-1"
                      title={t("toollaunch.openSite")}>
                      <ExternalLink className="w-3 h-3" /> {t("toollaunch.site")}
                    </a>
                  </div>
                </div>
              </div>
              {/* 上次启动配置摘要 */}
              {lastLaunchConfigs[selectedTool.id] && (
                <div className="mt-2 px-2 py-1.5 rounded-ctl bg-slate-800/50 border border-white/5">
                  <div className="flex items-center gap-1 mb-1">
                    <History className="w-3 h-3 text-slate-500" />
                    <span className="text-micro text-slate-500 font-semibold">{t("toollaunch.lastLaunch")}</span>
                    {lastLaunchConfigs[selectedTool.id].last_launched_at && (
                      <span className="text-micro text-slate-600 ml-auto">
                        {formatRelativeTime(lastLaunchConfigs[selectedTool.id].last_launched_at, t)}
                      </span>
                    )}
                  </div>
                  <div className="flex flex-wrap gap-x-2 gap-y-0.5 text-micro">
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
            <div className="p-3 rounded-card bg-slate-900/30 border border-white/5 space-y-2">
              <div className="flex items-center justify-between">
                <div className="text-body font-semibold text-slate-300 flex items-center gap-1.5">
                  <FolderOpen className="w-3.5 h-3.5" /> {t("toollaunch.installPath")}
                </div>
                {selectedTool.custom_path && (
                  <span className="text-micro px-1.5 py-px rounded bg-[var(--module-accent)]/20 text-[var(--module-accent)]">
                    {t("toollaunch.pathManual")}
                  </span>
                )}
              </div>
              <div className="flex items-center gap-1.5">
                <input
                  value={pathInput}
                  onChange={(e) => setPathInput(e.target.value)}
                  placeholder={selectedTool.detected_path || t("toollaunch.pathPlaceholder")}
                  className="flex-1 min-w-0 px-2 py-1 rounded-md bg-white/5 border border-white/10 text-caption text-slate-200 placeholder-slate-600 font-mono truncate focus:outline-none focus:border-[var(--module-accent)]/50"
                />
                <button onClick={() => void browseCustomPath()} disabled={pathSaving}
                  className="px-2 py-1 rounded-md bg-white/5 hover:bg-white/10 text-tiny text-slate-300 flex items-center gap-1 cursor-pointer disabled:opacity-50"
                  title={t("toollaunch.pathBrowse")}>
                  <FolderOpen className="w-3 h-3" />
                </button>
                <button onClick={() => void saveCustomPath(pathInput.trim() || null)} disabled={pathSaving}
                  className="px-2 py-1 rounded-md bg-emerald-500/15 hover:bg-emerald-500/25 text-tiny text-emerald-200 flex items-center gap-1 cursor-pointer disabled:opacity-50"
                  title={t("toollaunch.pathSaveHint")}>
                  <Check className="w-3 h-3" /> {t("toollaunch.save")}
                </button>
                <button onClick={() => { setPathInput(""); void saveCustomPath(null); }}
                  disabled={pathSaving || !selectedTool.custom_path}
                  className="px-2 py-1 rounded-md bg-white/5 hover:bg-white/10 text-tiny text-slate-300 cursor-pointer disabled:opacity-40"
                  title={t("toollaunch.pathClearHint")}>
                  {t("toollaunch.clear")}
                </button>
              </div>
              <div className="text-tiny text-slate-500 truncate">
                {selectedTool.detected_path ? (
                  `${t("toollaunch.pathDetected")}: ${selectedTool.detected_path}`
                ) : selectedTool.custom_path ? (
                  // 手动填了路径却在磁盘上找不到 → 明确说是「你填的那条不对」，
                  // 而不是笼统的「未检测到」让用户以为自动检测失灵了
                  <span className="text-amber-400/80">
                    {t("toollaunch.pathInvalid", { path: selectedTool.custom_path })}
                  </span>
                ) : selectedTool.tool_kind === "desktop" ? (
                  // Store / 桌面应用本来就没有常规 exe 安装路径（装在系统包注册里），
                  // 这不是检测失灵：启动走 `shell:AppsFolder\…` 包 URI，不依赖这条路径
                  <span className="text-slate-500">{t("toollaunch.pathStoreApp")}</span>
                ) : (
                  t("toollaunch.pathNotDetected")
                )}
              </div>
              {pathMsg && (
                <div className={`text-tiny ${pathMsg.ok ? "text-emerald-400" : "text-red-400"}`}>{pathMsg.msg}</div>
              )}
            </div>

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
                    <div className="space-y-2">
                    <p className="text-micro text-slate-500">{t("toollaunch.pluginMarketplaceHint")}</p>
                    {/* Claude 的市场是「一个个加进来的」，逐个列出、逐个可移除 */}
                    {marketKind === "claude" && (claudeStatus?.marketplaces.length ?? 0) > 0 && (
                      <div className="flex items-center gap-1.5 flex-wrap">
                        {claudeStatus!.marketplaces.map((m) => (
                          <span key={m}
                            className="inline-flex items-center gap-1 px-2 py-0.5 rounded-ctl bg-white/5 border border-white/5 text-micro text-slate-300">
                            <span className="font-mono">{m}</span>
                            <button onClick={() => void removeMarketplace(selectedTool, m)} disabled={marketplaceBusy}
                              className="text-slate-500 hover:text-red-400 cursor-pointer disabled:opacity-40 leading-none"
                              title={t("toollaunch.pluginMarketplaceRemove")}>
                              ×
                            </button>
                          </span>
                        ))}
                      </div>
                    )}
                    {marketKind === "codex" && marketplace?.root && (
                      <div className="text-micro text-slate-600 font-mono break-all">{marketplace.root}</div>
                    )}

                    {/* 插件清单：逐个安装 / 卸载（两套后端共用同一份渲染） */}
                    {marketReady && (
                      !marketCliAvailable ? (
                        <p className="text-micro text-amber-400/80">
                          {marketKind === "claude"
                            ? t("toollaunch.pluginCliMissingClaude")
                            : t("toollaunch.pluginCliMissing")}
                        </p>
                      ) : (
                        <div className="space-y-2 pt-1">
                          <input
                            value={pluginQuery}
                            onChange={(e) => setPluginQuery(e.target.value)}
                            placeholder={t("toollaunch.pluginSearch")}
                            className="w-full ui-input rounded-ctl px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
                          />
                          {pluginGroups.length === 0 ? (
                            <p className="text-micro text-slate-500">{t("toollaunch.pluginEmpty")}</p>
                          ) : (
                            pluginGroups.map(([group, items]) => (
                              <div key={group} className="space-y-1">
                                <div className="text-micro text-slate-500 font-semibold">{group}</div>
                                {items.map((p) => (
                                  <div
                                    key={p.key}
                                    className="flex items-start gap-2 px-2 py-1 rounded-ctl bg-slate-900/40 border border-white/5"
                                  >
                                    <div className="min-w-0 flex-1">
                                      <div className="flex items-center gap-2">
                                        <span className="text-tiny text-slate-200 truncate">{p.title}</span>
                                        {p.version && (
                                          <span className="text-micro text-slate-600 font-mono flex-shrink-0">{p.version}</span>
                                        )}
                                      </div>
                                      {p.desc && (
                                        <div className="text-micro text-slate-600 line-clamp-2">{p.desc}</div>
                                      )}
                                    </div>
                                    {p.installed && (
                                      <span className="text-micro px-1.5 py-px rounded bg-emerald-500/15 text-emerald-300 flex-shrink-0 mt-0.5">
                                        {t("toollaunch.pluginInstalledTag")}
                                      </span>
                                    )}
                                    <button
                                      onClick={() => void togglePlugin(selectedTool, p.key, !p.installed)}
                                      disabled={!!pluginBusy[p.key]}
                                      className={`px-2 py-0.5 rounded text-micro cursor-pointer transition-colors disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0 mt-0.5 ${
                                        p.installed
                                          ? "bg-white/5 hover:bg-white/10 text-slate-300"
                                          : "bg-[var(--module-accent)]/20 hover:bg-[var(--module-accent)]/30 text-white"
                                      }`}
                                    >
                                      {pluginBusy[p.key] ? (
                                        <RefreshCw className="w-3 h-3 animate-spin" />
                                      ) : p.installed ? (
                                        t("toollaunch.pluginUninstall")
                                      ) : (
                                        t("toollaunch.pluginInstall")
                                      )}
                                    </button>
                                  </div>
                                ))}
                              </div>
                            ))
                          )}
                        </div>
                      )
                    )}
                    </div>
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
                    <div className="mt-2 rounded-ctl border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="max-h-56 overflow-y-auto divide-y divide-white/[0.03]">
                        {cacheInfos.length === 0 ? (
                          <div className="px-3 py-4 text-tiny text-slate-600 text-center">{t("toollaunch.loading")}</div>
                        ) : selectedToolCaches.length === 0 ? (
                          <div className="px-3 py-4 text-tiny text-slate-600 text-center">{t("toollaunch.noCache")}</div>
                        ) : (
                          selectedToolCaches.map(cache => (
                            <div key={`${cache.tool_id}:${cache.dir_name}`} className="px-3 py-2 flex items-center gap-3">
                              <HardDrive className="w-3 h-3 text-slate-600 flex-shrink-0" />
                              <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2">
                                  <span className="text-tiny text-slate-300 font-mono truncate">{cache.dir_name}</span>
                                  {cache.is_junction && (
                                    <span className="text-[8px] text-blue-400 bg-blue-500/10 px-1 rounded">JUNCTION</span>
                                  )}
                                </div>
                                <div className="text-micro text-slate-500 font-mono truncate mt-0.5" title={cache.full_path}>
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
                                    className="p-1 rounded text-slate-600 hover:text-[var(--module-accent)] hover:bg-blue-500/10 cursor-pointer"
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
                        <div className="rounded-ctl border border-white/5 bg-slate-900/30">
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
                                  className="w-full flex items-center justify-between px-3 py-2 text-tiny hover:bg-white/[0.02] cursor-pointer transition-all"
                                >
                                  <div className="flex items-center gap-2 min-w-0">
                                    <ChevronRight className={`w-3 h-3 text-slate-500 transition-transform ${expanded ? "rotate-90" : ""}`} />
                                    <span className="font-semibold text-slate-400">{group.provider_name}</span>
                                    <span className="text-[8px] text-slate-600">{t("toollaunch.modelsCount", { count: group.models.length })}</span>
                                    {providerProtocolBadges(config?.providers.find(p => p.id === group.provider_id))}
                                  </div>
                                  {isSelected && selectedModel && (
                                    <span className="text-micro text-[var(--module-accent)] font-mono truncate ml-2">{selectedModel}</span>
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
                                          className={`w-full text-left px-5 py-1.5 text-caption transition-all cursor-pointer flex items-center gap-2 ${
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
                          <div className="mt-1 text-tiny text-[var(--module-accent)]">{t("toollaunch.selected")}<span className="font-mono">{selectedModel}</span> <span className="text-slate-500">（{config?.providers.find(p => p.id === selectedModelProvider)?.name}）</span></div>
                        )}

                        {/* 模型自定义启动参数（用户定义，运行时渲染为控件） */}
                        {currentModelCustomParams.length > 0 && (
                          <div className="mt-3 space-y-2">
                            <div className="text-tiny text-slate-500 font-semibold">{t("toollaunch.customParams")}</div>
                            {currentModelCustomParams.map(cp => (
                              <div key={cp.key} className="flex items-center gap-2">
                                <label className="text-tiny text-slate-400 w-28 flex-shrink-0 truncate" title={cp.key}>{cp.label || cp.key}</label>
                                {cp.paramType === "bool" ? (
                                  <input type="checkbox" checked={customParamValues[cp.key] !== "false"}
                                    onChange={e => setCustomParamValues(prev => ({ ...prev, [cp.key]: e.target.checked ? "true" : "false" }))}
                                    className="w-4 h-4 accent-[var(--module-accent)]" />
                                ) : cp.paramType === "text" ? (
                                  <input type="text" value={customParamValues[cp.key] || ""}
                                    onChange={e => setCustomParamValues(prev => ({ ...prev, [cp.key]: e.target.value }))}
                                    placeholder={cp.defaultValue || ""}
                                    className="flex-1 min-w-0 ui-input rounded px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                                ) : (
                                  <select value={customParamValues[cp.key] || cp.defaultValue || ""}
                                    onChange={e => setCustomParamValues(prev => ({ ...prev, [cp.key]: e.target.value }))}
                                    className="flex-1 min-w-0 ui-input rounded px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]">
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
                        <p className="text-micro text-slate-500 mb-1.5">{t("toollaunch.masqueradeHint", { model: selectedModel })}</p>
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
                {selectedTool.supports_fallback_model && selectedTool.installed && usingThirdPartyModel && fallbackGroups.length > 0 && (
                  <CollapsibleCard
                    title={t("toollaunch.fallbackLabel")}
                    hint={t("toollaunch.fallbackHint")}
                    open={fallbackOpen}
                    onToggle={() => setFallbackOpen(!fallbackOpen)}
                    summary={selectedFallbackModel || t("toollaunch.noFallbackShort")}
                  >
                    <div className="rounded-ctl border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="px-3 py-1.5 text-micro text-slate-600 font-mono cursor-pointer hover:bg-white/[0.05] border-b border-white/[0.03]"
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
                              className="w-full flex items-center justify-between px-3 py-1.5 text-tiny hover:bg-white/[0.02] cursor-pointer transition-all border-b border-white/[0.03]"
                            >
                              <div className="flex items-center gap-2">
                                <ChevronRight className={`w-3 h-3 text-slate-500 transition-transform ${expanded ? "rotate-90" : ""}`} />
                                <span className="font-semibold text-slate-400">{group.provider_name}</span>
                                <span className="text-[8px] text-slate-600">{t("toollaunch.itemsCount", { count: group.models.length })}</span>
                              </div>
                              {selectedInGroup && (
                                <span className="text-micro text-amber-400 font-mono truncate ml-2">{selectedFallbackModel}</span>
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
                                      className={`w-full text-left px-5 py-1.5 text-tiny transition-all cursor-pointer flex items-center gap-2 ${
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

                {/* 会话 */}
                <div>
                  <div className="flex items-center justify-between mb-2">
                    <label className="text-body font-bold text-slate-300">{t("toollaunch.sessions")}</label>
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
                      className={`px-3 py-1.5 rounded-ctl text-tiny font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                        sessionMode === "new" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                      }`}>
                      {t("toollaunch.newSession")}
                    </button>
                    {sessions.length > 0 && (
                      <button onClick={() => { setSessionMode("resume"); setShowSessionPicker(!showSessionPicker); setSelectedSession(null); }}
                        className={`px-3 py-1.5 rounded-ctl text-tiny font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                          sessionMode === "resume" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                        }`}>
                        <Clock className="w-3 h-3" /> {t("toollaunch.historySessions", { count: sessions.length })}
                      </button>
                    )}
                    {/* 分叉：只有工具自己实现了 fork（注册表 forkCmd）才显示——
                        复制一份会话再进入，原会话保持不动 */}
                    {sessions.length > 0 && selectedTool?.fork_cmd && (
                      <button onClick={() => { setSessionMode("fork"); setShowSessionPicker(true); setSelectedSession(null); }}
                        className={`px-3 py-1.5 rounded-ctl text-tiny font-semibold flex items-center gap-1 cursor-pointer transition-all ${
                          sessionMode === "fork" ? "bg-[var(--module-accent)] text-white" : "bg-white/5 text-slate-400 hover:text-slate-200"
                        }`}
                        title={t("toollaunch.forkSessionTip")}>
                        <GitFork className="w-3 h-3" /> {t("toollaunch.forkSession")}
                      </button>
                    )}
                  </div>

                  {showSessionPicker && (sessionMode === "resume" || sessionMode === "fork") && (
                    <div className="mb-2">
                      <div className="flex items-center gap-2">
                        <div className="flex-1 relative">
                          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-slate-500" />
                          <input value={sessionSearch} onChange={e => setSessionSearch(e.target.value)}
                            placeholder={t("toollaunch.searchSessions")} className="w-full ui-input rounded-ctl pl-7 pr-7 py-1.5 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                          {sessionSearch && (
                            <button onClick={() => setSessionSearch("")} className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300">
                              <X className="w-3 h-3" />
                            </button>
                          )}
                        </div>
                        {selectionMode && (
                          <>
                            <button onClick={handleSelectAll} className="px-2 py-1 rounded text-micro font-semibold bg-white/5 text-slate-400 hover:text-slate-200 cursor-pointer whitespace-nowrap">
                              {selectedSessionIds.size === filteredSessions.length ? t("toollaunch.cancelSelectAll") : t("toollaunch.selectAll")}
                            </button>
                            <button onClick={handleDeleteSessions} disabled={selectedSessionIds.size === 0}
                              className="px-2 py-1 rounded text-micro font-semibold bg-red-500/10 text-red-400 hover:bg-red-500/20 cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed whitespace-nowrap flex items-center gap-1">
                              <Trash2 className="w-3 h-3" /> {t("toollaunch.delete", { count: selectedSessionIds.size })}
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                  )}

                  {showSessionPicker && (sessionMode === "resume" || sessionMode === "fork") && (
                    <div className="rounded-ctl border border-white/5 bg-slate-900/30 overflow-hidden">
                      <div className="max-h-72 overflow-y-auto divide-y divide-white/[0.03]">
                        {filteredSessions.length === 0 ? (
                          <div className="px-3 py-6 text-tiny text-slate-600 text-center">
                            {sessionSearch ? t("toollaunch.noMatchSessions") : t("toollaunch.noHistorySessions")}
                          </div>
                        ) : sessionViewMode === "flat" ? (
                          filteredSessions.map(s => (
                            <div key={s.session_id}
                              className={`flex items-center px-3 py-2 text-tiny transition-all group ${
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
                                  {s.summary && <div className="text-micro text-slate-500 mt-0.5 truncate italic">{s.summary}</div>}
                                </div>
                                <span className="text-micro text-slate-600 flex-shrink-0 ml-3">{s.last_used}</span>
                              </button>
                            </div>
                          ))
                        ) : (
                          sessionDirGroups.map(group => (
                            <div key={group.dir}>
                              <button onClick={() => toggleDirExpand(group.dir)}
                                className="w-full flex items-center gap-2 px-3 py-2 text-tiny bg-white/[0.02] hover:bg-white/[0.04] text-slate-400 hover:text-slate-200 cursor-pointer sticky top-0 z-10">
                                <ChevronRight className={`w-3 h-3 flex-shrink-0 transition-transform ${expandedDirs.has(group.dir) ? "rotate-90" : ""}`} />
                                <Folder className="w-3 h-3 flex-shrink-0 text-amber-500/70" />
                                <span className="font-semibold truncate">{group.label}</span>
                                <span className="text-micro text-slate-600 ml-auto">{group.sessions.length}</span>
                              </button>
                              {expandedDirs.has(group.dir) && group.sessions.map(s => (
                                <div key={s.session_id}
                                  className={`flex items-center pl-9 pr-3 py-2 text-tiny transition-all group ${
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
                                        {s.summary && <span className="text-micro text-slate-500 ml-2 italic truncate">{s.summary}</span>}
                                      </span>
                                    </div>
                                    <span className="text-micro text-slate-600 flex-shrink-0 ml-3">{s.last_used}</span>
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
                    <div className="mt-2 p-2 rounded-ctl bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] border border-[var(--module-accent-ring)] text-tiny text-[var(--module-accent)] flex items-center gap-2">
                      <CheckCircle className="w-3 h-3 flex-shrink-0" />
                      <span className="truncate">{t("toollaunch.willRestore", { path: selectedSession.project_path })}</span>
                    </div>
                  )}
                </div>

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
