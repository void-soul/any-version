import React, { useState, useEffect, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { type ConfirmRequest } from "../shared/ConfirmDialog";
import { FolderOpen, CheckCircle, ArrowUpCircle } from "lucide-react";
import type { DetectedAiTool, AiToolCacheInfo, ToolOpResult } from "../../contracts/ai";
import { alertError } from "../shared/ThemedAlert";
import type { Dispatch, SetStateAction } from "react";

/** Owns the ToolOperations feature, independently of page rendering. */
export function useToolOperations({ selectedToolId, tools, setTools }: {
  selectedToolId: string | null;
  tools: DetectedAiTool[];
  setTools: Dispatch<SetStateAction<DetectedAiTool[]>>;
}) {
  const { t } = useTranslation();

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

  // 缓存管理
  const [cacheInfos, setCacheInfos] = useState<AiToolCacheInfo[]>([]);

  const [showCacheManager, setShowCacheManager] = useState(false);

  const [migratingCache, setMigratingCache] = useState<string | null>(null);

  const [cleaningCache, setCleaningCache] = useState<string | null>(null);

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
  return {
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
  };
}
