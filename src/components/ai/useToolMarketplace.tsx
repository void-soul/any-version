import { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import type { DetectedAiTool, CodexPluginMarketplaceStatus, CodexPluginInfo, ClaudePluginInfo, ClaudePluginStatus } from "../../contracts/ai";
import { alertError } from "../shared/ThemedAlert";
import { type PluginRow } from "./ToolLauncher.model";

/** Owns the ToolMarketplace feature, independently of page rendering. */
export function useToolMarketplace({ selectedTool }: {
  selectedTool: DetectedAiTool | null;
}) {
  const { t } = useTranslation();

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

  const renderMarketplaceView = () => selectedTool && (<div className="space-y-2">
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
                    </div>);
  return {
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
  };
}
