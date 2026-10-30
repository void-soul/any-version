import type { AiProvider, DetectedAiTool } from "../../contracts/ai";
/**
 * 插件列表的统一行结构。
 * Codex 与 Claude 两套后端的字段名不同，逻辑层抹平后 UI 只认这一种（不用维护两套列表）。
 */
export type PluginRow = {
  /** 安装 / 卸载的寻址键（codex = 插件名；claude = `插件@市场`） */
  key: string;
  title: string;
  group: string;
  desc: string;
  installed: boolean;
  enabled: boolean;
  version: string | null;
};

export const PROTOCOL_LABELS: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  both: "OpenAI + Anthropic",
  google: "Google",
  none: "", // 渲染时用 toollaunch.modelNone 翻译
};

/// 由供应商已配置的协议 URL 推导出站协议：若供应商支持工具原生协议则同协议直连，
/// 否则取供应商首个支持的协议（由代理做协议转换）。
export function getOutboundProtocol(tool: DetectedAiTool | null, provider: AiProvider | null): string {
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
export function formatRelativeTime(isoString: string, t: (k: string, o?: any) => string): string {
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

/// 哪些工具的伪装名会被后端按「官方模型名校验」过滤。
///
/// 与 Rust `tool_config_custom::effective_claimed_model` 的分派保持一致：只有这两个
/// 桌面端（模型选择器只认自家官方名）会把手填的非法声明名**自动替换**成 `builtinModels`
/// 清单里第一个合法项；其余工具原样使用用户填的名字。
export function isStrictMasqueradeTool(toolId: string | undefined | null): boolean {
  return toolId === "claudedesktop" || toolId === "chatgptdesktop";
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
export function findModelRef(
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

/** 工具配置文件里回读到的模型，要不要拿去回填「供应商 + 模型」选择。
 *
 * **只有「没有上次启动记录」时才回填。**
 *
 * 伪装生效时，工具配置文件里写的是**声明名 C** 而不是真实模型 —— ChatGPT 桌面端写
 * `gpt-6-astra`，真实模型是 `space-bunny`。拿 C 去供应商列表里 [`findModelRef`] 反查，
 * 会命中**另一个供应商的同名模型**：实测把「WorkBuddy2API / space-bunny」顶成
 * 「OpenCode / gpt-6-astra」，伪装输入框也跟着被清空，看起来像「伪装值变成了原始模型」。
 *
 * 「上次启动记录」才是「用户选了哪个供应商+模型」的权威来源；配置文件那一路仍然始终
 * 回读（用于提示当前实际生效的是什么），只是不再夺走选择权。
 */
export function resolveAppliedModelRef(
  providers: AiProvider[],
  appliedModel: string | null | undefined,
  hasLastSelection: boolean,
): { providerId: string; modelId: string } | null {
  if (hasLastSelection) return null;
  return findModelRef(providers, appliedModel);
}

/** 未安装区的形态筛选：全部 / CLI / 桌面端（抄 EchoBird 的分组维度）。
 *  已安装区不分组——装了的就那么几个，分组只会让人多找一层。 */
export type ToolKindFilter = "all" | "cli" | "desktop";
