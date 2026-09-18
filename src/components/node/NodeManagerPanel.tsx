// 通用「Node 项目管理器」面板
// 顶级功能：管理类似 deepseek-harness 的 Node 项目（安装/升级/启动/停止/打开主页）。
// 项目列表来自后端 node-projects/ 配置注册表，新增项目无需改前端代码。
//
// 布局说明：
// - 无已打开服务 Tab 时：自动弹出服务管理弹窗（无需再手动点一次）；弹窗可关闭查看引导页。
// - 打开服务后：iframe 全屏占满页面；顶部 Tab 栏含「管理」按钮，可随时弹出服务管理弹窗。
import { useState, useEffect, useCallback, useRef } from "react";
import {
  Bot,
  Boxes,
  Package,
  Play,
  Square,
  Download,
  RefreshCw,
  ExternalLink,
  Code2,
  CheckCircle2,
  XCircle,
  Loader2,
  GitBranch,
  Terminal,
  AlertTriangle,
  Copy,
  Eraser,
  X,
  LayoutDashboard,
  Settings2,
  Trash2,
  Hammer,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "../shared/ConfirmDialog";

// ---- 类型（与后端 node_manager.rs 对应，serde camelCase）----

interface NodeProjectDef {
  id: string;
  displayName: string;
  repo: string;
  website: string;
  icon: string;
  description: string;
  defaultPort: number;
  webPath: string;
  nodeRequirement: string;
  /** 运行时：node（默认）/ python（venv + pip） */
  runtime?: string;
  packageManager: string;
  buildScript: string;
  startCmd: string[];
  managed: boolean;
  npxPackage: string;
  npxBin: string;
  /** pip 包模式：PyPI 包名（如 headroom-ai[proxy]），直接从 PyPI 安装、无需 clone */
  pipPackage?: string;
  /** pip 包模式下的 -m 模块路径（如 headroom.cli） */
  pipModule?: string;
  /// 控制台 URL 提取正则（第 1 捕获组 = 带凭据主页地址）；空 = 不提取。
  consoleUrlPattern: string;
  /// 捕获到控制台 URL 后是否自动用系统浏览器打开。
  autoOpenConsoleUrl: boolean;
}

interface DepCheck {
  name: string;
  exists: boolean;
  path?: string | null;
  version?: string | null;
  satisfies: boolean;
  requirement?: string | null;
}

interface DepCheckResult {
  git: DepCheck;
  node: DepCheck;
  packageManager: DepCheck;
  allReady: boolean;
}

interface NodeProjectStatus {
  id: string;
  displayName: string;
  installed: boolean;
  status: string; // "running" | "stopped" | "not_installed" | "port_conflict"
  port?: number | null;
  /** port_conflict 时占用进程名 */
  conflictProcess?: string | null;
  pid?: number | null;
  gitVersion?: string | null;
  localVersion?: string | null;
  error?: string | null;
}

interface NodeUpdateInfo {
  hasUpdate: boolean;
  currentCommit: string;
  latestCommit: string;
  behind: number;
  currentVersion?: string | null;
  latestVersion?: string | null;
  error?: string | null;
}

interface NodeProgress {
  projectId: string;
  phase: string;
  detail: string;
}

interface NodeLog {
  projectId: string;
  phase: string;
  line: string;
}

/// 控制台 URL 捕获事件（npm-console-url）。
interface NodeConsoleUrl {
  projectId: string;
  url: string;
}

/// 每个项目日志保留的最大行数。
const MAX_LOG_LINES = 800;

const ICONS: Record<string, React.ComponentType<{ className?: string }>> = {
  bot: Bot,
  boxes: Boxes,
};

/// 渲染 webPath，替换 {port} 占位符（与后端 NodeProjectDef::resolved_web_path 一致）。
function resolvedWebPath(p: NodeProjectDef): string {
  return p.webPath.replace("{port}", String(p.defaultPort));
}

/// 服务管理固定标签页 id（不可关闭）
const MANAGE_TAB = "__manage__";

/// 判断服务端口是否处于监听（running 或 port_conflict 均视为有进程占用端口）。
function isPortListening(st?: NodeProjectStatus): boolean {
  return st?.status === "running" || st?.status === "port_conflict";
}

export default function NodeManagerPanel() {
  const { t } = useTranslation();
  const [projects, setProjects] = useState<NodeProjectDef[]>([]);
  const [loaded, setLoaded] = useState(false); // 首次列表是否已加载完成
  const [deps, setDeps] = useState<Record<string, DepCheckResult>>({});
  const [statuses, setStatuses] = useState<Record<string, NodeProjectStatus>>(
    {},
  );
  const [busy, setBusy] = useState<string>(""); // "install:harness" / "upgrade:harness" / ...
  // 管理弹窗当前选中的服务（左侧竖向选项卡，一服务一页）
  const [manageSelectedId, setManageSelectedId] = useState<string>("");
  const [progress, setProgress] = useState<Record<string, NodeProgress>>({});
  const [error, setError] = useState<Record<string, string>>({});
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const [logOpen, setLogOpen] = useState<Record<string, boolean>>({});
  const logEndRefs = useRef<Record<string, HTMLDivElement | null>>({});
  // 各项目最近捕获的控制台 URL（带凭据主页地址，随服务重启更新）
  const [consoleUrls, setConsoleUrls] = useState<Record<string, string>>({});
  // 内部主页 Tab 管理（在主窗口内 iframe 打开各 Node 应用界面，服务区全屏）
  const [tabs, setTabs] = useState<NodeProjectDef[]>([]);
  // 默认打开服务管理页（Q-0120）：进入面板直接可用，无需点按钮
  const [activeTabId, setActiveTabId] = useState<string | null>(MANAGE_TAB);
  const [tabReload, setTabReload] = useState<Record<string, number>>({});
  // git 更新检查
  const [updateInfo, setUpdateInfo] = useState<Record<string, NodeUpdateInfo>>(
    {},
  );
  const [checkingUpdate, setCheckingUpdate] = useState<string | null>(null);

  const refreshDeps = useCallback(async (id: string) => {
    try {
      const d = await invoke<DepCheckResult>("npm_deps", { projectId: id });
      setDeps((prev) => ({ ...prev, [id]: d }));
    } catch (err) {
      console.error("检查项目依赖失败:", err);
    }
  }, []);

  const refreshStatus = useCallback(async (id: string) => {
    try {
      const s = await invoke<NodeProjectStatus>("npm_status", {
        projectId: id,
      });
      setStatuses((prev) => ({ ...prev, [id]: s }));
    } catch (err) {
      console.error("查询项目状态失败:", err);
    }
  }, []);

  const refreshAll = useCallback(async () => {
    const list = await invoke<NodeProjectDef[]>("npm_list_projects").catch(
      () => [],
    );
    setProjects(list);
    setLoaded(true);
    for (const p of list) {
      refreshDeps(p.id);
      refreshStatus(p.id);
    }
  }, [refreshDeps, refreshStatus]);

  useEffect(() => {
    refreshAll();
    const timer = setInterval(() => {
      setProjects((prev) => {
        for (const p of prev) refreshStatus(p.id);
        return prev;
      });
    }, 4000);
    return () => clearInterval(timer);
  }, [refreshAll, refreshStatus]);

  // 监听安装/升级/启动进度；phase=done 时自动清除该项目的进度动画
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false; // StrictMode 双挂载下，异步注册完成前可能已卸载 → 必须补退订
    const setup = async () => {
      const fn = await listen<NodeProgress>("npm-progress", (e) => {
        const { projectId, phase } = e.payload;
        if (phase === "done") {
          setProgress((prev) => {
            const next = { ...prev };
            delete next[projectId];
            return next;
          });
        } else {
          setProgress((prev) => ({ ...prev, [projectId]: e.payload }));
        }
      });
      if (disposed) fn();
      else unlisten = fn;
    };
    setup();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // 监听实时日志（git pull / install / build / start）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const setup = async () => {
      const fn = await listen<NodeLog>("npm-log", (e) => {
        const { projectId, line } = e.payload;
        if (!line) return;
        setLogs((prev) => {
          const cur = prev[projectId] ?? [];
          return { ...prev, [projectId]: [...cur, line].slice(-MAX_LOG_LINES) };
        });
        setLogOpen((prev) =>
          prev[projectId] ? prev : { ...prev, [projectId]: true },
        );
      });
      if (disposed) fn();
      else unlisten = fn;
    };
    setup();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // 监听控制台 URL 捕获（dsh web 等在启动输出里打印带 token 的认证地址）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    const setup = async () => {
      const fn = await listen<NodeConsoleUrl>("npm-console-url", (e) => {
        const { projectId, url } = e.payload;
        if (!projectId || !url) return;
        setConsoleUrls((prev) => ({ ...prev, [projectId]: url }));
      });
      if (disposed) fn();
      else unlisten = fn;
    };
    setup();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // 日志区自动滚动到底部
  useEffect(() => {
    for (const el of Object.values(logEndRefs.current)) {
      el?.scrollIntoView({ block: "end" });
    }
  }, [logs]);

  const runAction = async (project: NodeProjectDef, action: string) => {
    const key = `${action}:${project.id}`;
    setBusy(key);
    setError((prev) => ({ ...prev, [project.id]: "" }));
    try {
      await invoke(`npm_${action}`, { projectId: project.id });
      await refreshStatus(project.id);
    } catch (e) {
      setError((prev) => ({
        ...prev,
        [project.id]: typeof e === "string" ? e : String(e),
      }));
    } finally {
      setBusy("");
      // 操作结束后清除进度动画（避免"启动中/安装中"残留）
      setProgress((prev) => {
        if (!prev[project.id]) return prev;
        const next = { ...prev };
        delete next[project.id];
        return next;
      });
    }
  };

  // 在服务运行目录中执行任意命令行（如 `dsh plugin --profile web add`）
  const execCommand = async (project: NodeProjectDef, command: string) => {
    const key = `exec:${project.id}`;
    setBusy(key);
    setError((prev) => ({ ...prev, [project.id]: "" }));
    // 执行命令必然产生日志，自动展开日志区
    setLogOpen((prev) => ({ ...prev, [project.id]: true }));
    try {
      await invoke("npm_exec", { projectId: project.id, command });
      await refreshStatus(project.id);
    } catch (e) {
      setError((prev) => ({
        ...prev,
        [project.id]: typeof e === "string" ? e : String(e),
      }));
    } finally {
      setBusy("");
      setProgress((prev) => {
        if (!prev[project.id]) return prev;
        const next = { ...prev };
        delete next[project.id];
        return next;
      });
    }
  };

  // 检查 git 是否有新版（fetch 后比较）
  const checkUpdate = async (project: NodeProjectDef) => {
    setCheckingUpdate(project.id);
    setUpdateInfo((prev) => {
      const next = { ...prev };
      delete next[project.id];
      return next;
    });
    try {
      const info = await invoke<NodeUpdateInfo>("npm_check_update", {
        projectId: project.id,
      });
      setUpdateInfo((prev) => ({ ...prev, [project.id]: info }));
    } catch (e) {
      setUpdateInfo((prev) => ({
        ...prev,
        [project.id]: {
          hasUpdate: false,
          currentCommit: "",
          latestCommit: "",
          behind: 0,
          currentVersion: null,
          latestVersion: null,
          error: typeof e === "string" ? e : String(e),
        },
      }));
    } finally {
      setCheckingUpdate(null);
    }
  };

  // 打开应用主页。
  // - 普通服务：主窗口内 iframe 全屏（打开前校验端口是否监听）。
  // - 配置了 consoleUrlPattern 的服务（如 dsh web）：控制台打印的带 token 地址
  //   是唯一可用入口（直接访问 webPath 会 401），同样在 Kira 内部 iframe 打开
  //   后端捕获的最新地址；尚未捕获时提示稍候。
  const openWeb = async (project: NodeProjectDef) => {
    setError((prev) => ({ ...prev, [project.id]: "" }));
    if (project.consoleUrlPattern?.trim()) {
      // 始终回查后端取「最新」捕获地址（内存 → 持久化文件）：
      // token 随每次服务启动变化，前端 state 里可能残留旧值，直接用会打开失效地址。
      const fromBackend = await invoke<string | null>("npm_console_url", {
        projectId: project.id,
      }).catch(() => null);
      let captured = fromBackend ?? consoleUrls[project.id];
      if (fromBackend) {
        setConsoleUrls((prev) => ({ ...prev, [project.id]: fromBackend }));
      }
      if (!captured) {
        const st = await invoke<NodeProjectStatus>("npm_status", {
          projectId: project.id,
        }).catch(() => undefined);
        if (st && !isPortListening(st)) {
          setError((prev) => ({
            ...prev,
            [project.id]: t("nodeproj.serviceStateHint", {
              label: t("nodeproj.notRunning"),
              path: resolvedWebPath(project),
            }),
          }));
          return;
        }
        setError((prev) => ({
          ...prev,
          [project.id]: t("nodeproj.consoleUrlPending"),
        }));
        return;
      }
      // 独立 Kira 窗口打开（非外部浏览器）：dsh 等服务的鉴权 Cookie 是 SameSite=Strict，
      // 主窗口 iframe 的第三方上下文会拦截 Cookie 导致 401；顶级窗口可正常握手。
      try {
        await invoke("npm_open_window", { projectId: project.id });
      } catch (e) {
        setError((prev) => ({
          ...prev,
          [project.id]: typeof e === "string" ? e : String(e),
        }));
      }
      return;
    }
    if (tabs.some((t) => t.id === project.id)) {
      setActiveTabId(project.id);
      return;
    }
    const st = await invoke<NodeProjectStatus>("npm_status", {
      projectId: project.id,
    }).catch(() => undefined);
    if (st) setStatuses((prev) => ({ ...prev, [project.id]: st }));
    if (!isPortListening(st)) {
      const stateLabel =
        st?.status === "not_installed"
          ? t("nodeproj.notInstalled")
          : st?.status === "port_conflict"
            ? t("nodeproj.portConflict")
            : t("nodeproj.notRunning");
      setError((prev) => ({
        ...prev,
        [project.id]: t("nodeproj.serviceStateHint", { label: stateLabel, path: resolvedWebPath(project) }),
      }));
      return;
    }
    setTabs((prev) =>
      prev.some((t) => t.id === project.id) ? prev : [...prev, project],
    );
    setActiveTabId(project.id);
  };

  const reloadTab = (id: string) => {
    setTabReload((prev) => ({ ...prev, [id]: (prev[id] ?? 0) + 1 }));
  };

  const openDevTools = async (project: NodeProjectDef) => {
    try {
      await invoke("npm_open_devtools", { projectId: project.id });
    } catch (e) {
      setError((prev) => ({ ...prev, [project.id]: typeof e === "string" ? e : String(e) }));
    }
  };

  // 强制关闭占用服务端口的进程（端口冲突时前端按钮触发）
  const killPortOwner = async (project: NodeProjectDef, port: number) => {
    try {
      await invoke<string>("kill_port_owner", { portStr: String(port) });
      setError((prev) => ({ ...prev, [project.id]: "" }));
    } catch (e) {
      setError((prev) => ({
        ...prev,
        [project.id]: typeof e === "string" ? e : String(e),
      }));
    }
    await refreshStatus(project.id);
  };

  const closeTab = (id: string) => {
    setTabs((prev) => {
      const idx = prev.findIndex((t) => t.id === id);
      if (idx === -1) return prev;
      const next = prev.filter((t) => t.id !== id);
      if (activeTabId === id) {
        // 全部服务标签关闭后回到服务管理页
        setActiveTabId(next[idx] ? next[idx].id : (next[idx - 1]?.id ?? MANAGE_TAB));
      }
      return next;
    });
  };

  if (projects.length === 0) {
    if (!loaded) {
      return (
        <div className="h-full flex items-center justify-center text-slate-500 text-sm gap-2">
          <Loader2 className="w-4 h-4 animate-spin" /> {t("nodeproj.loading")}
        </div>
      );
    }
    return (
      <div className="h-full flex items-center justify-center text-slate-500 text-sm gap-2">
        <Boxes className="w-4 h-4" /> {t("nodeproj.noProjects")}
        {t("nodeproj.noProjects2")}
      </div>
    );
  }

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;
  const managedProjects = projects.filter((p) => p.managed);
  // 选中项兜底：列表变化后保持有效（默认第一个）
  const manageSelected = managedProjects.find((p) => p.id === manageSelectedId) ?? managedProjects[0] ?? null;

  // 服务管理页（固定标签页内容）：左侧竖向服务列表（带状态标签）+ 右侧详情卡片
  const managePage = (
    <div className="h-full flex flex-col bg-[#0b0f1a]">
      <div className="flex items-center gap-2 px-5 py-3 border-b border-white/10 bg-white/[0.02]">
        <Settings2 className="w-4 h-4 text-[var(--module-accent)]" />
        <h2 className="text-sm font-bold text-white">{t("nodeproj.manageTitle")}</h2>
        <span className="text-[10px] text-slate-500 ml-1">{t("nodeproj.manageSub")}</span>
      </div>
      <div className="flex-1 min-h-0 flex overflow-hidden">
        {/* 服务选项卡列表 */}
        <div className="w-52 flex-shrink-0 border-r border-white/10 bg-white/[0.02] overflow-y-auto py-2">
          {managedProjects.map((project) => {
            const st = statuses[project.id];
            const status = st?.status ?? "not_installed";
            const selected = manageSelected?.id === project.id;
            const statusCls =
              status === "running"
                ? "bg-emerald-500/15 text-emerald-400"
                : status === "port_conflict"
                  ? "bg-amber-500/15 text-amber-400"
                  : "bg-slate-500/15 text-slate-500";
            const statusLabel =
              status === "running"
                ? t("nodeproj.running")
                : status === "port_conflict"
                  ? t("nodeproj.portConflict")
                  : status === "stopped"
                    ? t("nodeproj.stopped")
                    : t("nodeproj.notInstalled");
            return (
              <button
                key={project.id}
                type="button"
                onClick={() => setManageSelectedId(project.id)}
                className={`w-full text-left px-3 py-2.5 flex items-center gap-2 border-l-2 transition-colors cursor-pointer ${
                  selected
                    ? "border-[var(--module-accent)] bg-white/[0.06]"
                    : "border-transparent hover:bg-white/[0.03]"
                }`}
              >
                <span
                  className={`text-[11px] font-semibold truncate flex-1 ${
                    selected ? "text-white" : "text-slate-300"
                  }`}
                  title={project.displayName}
                >
                  {project.displayName}
                </span>
                <span
                  className={`px-1.5 py-0.5 rounded text-[8px] font-bold flex-shrink-0 ${statusCls}`}
                >
                  {statusLabel}
                </span>
              </button>
            );
          })}
          {managedProjects.length === 0 && (
            <div className="px-3 py-6 text-center text-[10px] text-slate-500">
              {t("nodeproj.noManaged")}
            </div>
          )}
        </div>

        {/* 选中服务的详情卡片 */}
        <div className="flex-1 min-w-0 overflow-y-auto p-4">
          {manageSelected ? (
            <ProjectCard
              project={manageSelected}
              st={statuses[manageSelected.id]}
              d={deps[manageSelected.id]}
              prog={progress[manageSelected.id]}
              busy={busy}
              isStarting={busy === `start:${manageSelected.id}`}
              isStopping={busy === `stop:${manageSelected.id}`}
              logs={logs[manageSelected.id] ?? []}
              logOpen={!!logOpen[manageSelected.id]}
              error={error[manageSelected.id]}
              updateInfo={updateInfo[manageSelected.id]}
              checkingUpdate={checkingUpdate === manageSelected.id}
              consoleUrl={consoleUrls[manageSelected.id]}
              onAction={runAction}
              onExec={execCommand}
              onOpenWeb={openWeb}
              onCheckUpdate={checkUpdate}
              onToggleLog={() =>
                setLogOpen((prev) => ({
                  ...prev,
                  [manageSelected.id]: !prev[manageSelected.id],
                }))
              }
              onClearLogs={(pid) => setLogs((prev) => ({ ...prev, [pid]: [] }))}
              onKillPortOwner={(port) => void killPortOwner(manageSelected, port)}
            />
          ) : (
            <div className="h-full flex items-center justify-center text-slate-500 text-sm">
              {t("nodeproj.noManaged")}
            </div>
          )}
        </div>
      </div>
    </div>
  );

  return (
    <div className="h-full flex flex-col min-h-0 select-none">
      {/* 服务区全屏：Tab 栏 + iframe / 管理页 */}
      {(tabs.length > 0 || activeTabId === MANAGE_TAB) && (
        <div className="flex-1 min-h-0 flex flex-col">
          {/* Tab 栏 */}
          <div className="flex items-center gap-1 px-2 pt-1.5 pb-0 bg-[#0b0f1a] border-b border-white/10 overflow-x-auto">
            <LayoutDashboard className="w-3.5 h-3.5 text-[var(--module-accent)] ml-1 flex-shrink-0" />
            {tabs.map((tab) => {
              const Icon = ICONS[tab.icon] ?? Bot;
              const active = tab.id === activeTabId;
              return (
                <button
                  key={tab.id}
                  onClick={() => setActiveTabId(tab.id)}
                  className={`group flex items-center gap-1.5 px-3 py-1.5 rounded-t-lg text-[11px] font-semibold transition-all cursor-pointer flex-shrink-0 ${
                    active
                      ? "bg-white/10 text-white border-b-2 border-[var(--module-accent)]"
                      : "text-slate-400 hover:text-slate-200 hover:bg-white/5 border-b-2 border-transparent"
                  }`}
                >
                  <Icon className="w-3 h-3" />
                  <span>{tab.displayName}</span>
                  <span
                    role="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      closeTab(tab.id);
                    }}
                    className="ml-0.5 p-0.5 rounded hover:bg-white/15 text-slate-500 hover:text-white cursor-pointer"
                    title={t("nodeproj.close")}
                  >
                    <X className="w-3 h-3" />
                  </span>
                </button>
              );
            })}
            <div className="flex-1" />
            <button
              onClick={() => activeTab && reloadTab(activeTab.id)}
              disabled={!activeTab}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-semibold text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex-shrink-0 disabled:opacity-40 disabled:cursor-not-allowed"
              title={t("nodeproj.refreshHomeTitle")}
            >
              <RefreshCw className="w-3.5 h-3.5" /> {t("nodeproj.refreshHome")}
            </button>
            <button
              onClick={() => activeTab && void openDevTools(activeTab)}
              disabled={!activeTab}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-semibold text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex-shrink-0 disabled:opacity-40 disabled:cursor-not-allowed"
              title={t("nodeproj.devToolsTitle")}
            >
              <Code2 className="w-3.5 h-3.5" /> {t("nodeproj.devTools")}
            </button>
            {/* 服务管理入口 */}
            <button
              onClick={() => setActiveTabId(MANAGE_TAB)}
              className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-semibold cursor-pointer transition-all flex-shrink-0 ${
                activeTabId === MANAGE_TAB
                  ? "bg-white/10 text-white"
                  : "text-slate-300 hover:text-white hover:bg-white/10"
              }`}
              title={t("nodeproj.openManage")}
            >
              <Settings2 className="w-3.5 h-3.5" /> {t("nodeproj.manage")}
            </button>
          </div>

          {/* 内容区：管理页 / 全屏 iframe */}
          <div className="flex-1 min-h-0">
            {activeTabId === MANAGE_TAB ? (
              managePage
            ) : activeTab ? (
              <iframe
                src={consoleUrls[activeTab.id] || resolvedWebPath(activeTab)}
                key={`${activeTab.id}:${tabReload[activeTab.id] ?? 0}`}
                className="w-full h-full border-0 bg-white"
                title={activeTab.displayName}
              />
            ) : (
              <div className="h-full flex items-center justify-center text-slate-500 text-sm gap-2">
                <LayoutDashboard className="w-4 h-4" /> {t("nodeproj.pickTab")}
              </div>
            )}
          </div>
        </div>
      )}

    </div>
  );
}

// ---- 项目卡片（复用于管理弹窗）----

function ProjectCard({
  project,
  st,
  d,
  prog,
  busy,
  isStarting,
  isStopping,
  logs,
  logOpen,
  error,
  updateInfo,
  checkingUpdate,
  consoleUrl,
  onAction,
  onExec,
  onOpenWeb,
  onCheckUpdate,
  onToggleLog,
  onClearLogs,
  onKillPortOwner,
}: {
  project: NodeProjectDef;
  st?: NodeProjectStatus;
  d?: DepCheckResult;
  prog?: NodeProgress;
  busy: string;
  isStarting: boolean;
  isStopping: boolean;
  logs: string[];
  logOpen: boolean;
  error?: string;
  updateInfo?: NodeUpdateInfo;
  checkingUpdate: boolean;
  /// 最近捕获的控制台 URL（带凭据主页地址）；未捕获为 undefined。
  consoleUrl?: string;
  onAction: (p: NodeProjectDef, action: string) => void;
  onExec: (p: NodeProjectDef, command: string) => void;
  onOpenWeb: (p: NodeProjectDef) => void;
  onCheckUpdate: (p: NodeProjectDef) => void;
  onToggleLog: () => void;
  onClearLogs: (projectId: string) => void;
  onKillPortOwner: (port: number) => void;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const Icon = ICONS[project.icon] ?? Bot;
  // npx 模式：配置了 npxPackage，安装/升级/启动直接用 npm install --prefix / npx --prefix
  const isNpx = !!project.npxPackage?.trim();
  // pip 包模式：配置了 pipPackage，直接从 PyPI 安装（无需 clone）
  const isPip = !!project.pipPackage?.trim();
  // 控制台 URL 模式：服务通过启动输出里打印的带 token 地址访问（iframe 不可用）
  const consoleUrlMode = !!project.consoleUrlPattern?.trim();
  const installed = st?.installed;
  const running = st?.status === "running";
  const portConflict = st?.status === "port_conflict";
  const isBusy =
    busy === `install:${project.id}` ||
    busy === `upgrade:${project.id}` ||
    busy === `install_deps:${project.id}` ||
    busy === `build_native:${project.id}` ||
    busy === `exec:${project.id}` ||
    busy === `uninstall:${project.id}`;
  const isExecBusy = busy === `exec:${project.id}`;
  // 命令输入框内容（每个服务独立）
  const [cmdInput, setCmdInput] = useState("");
  // 随应用启动（kira 启动后自动拉起该服务）
  const autoStartId = `node:${project.id}`;
  const [autoStart, setAutoStart] = useState(false);
  useEffect(() => {
    void invoke<string[]>("get_auto_start_services")
      .then(list => setAutoStart(list.includes(autoStartId)))
      .catch(() => {});
  }, [autoStartId]);
  const toggleAutoStart = async () => {
    const next = !autoStart;
    setAutoStart(next);
    try {
      await invoke("set_auto_start_service", { serviceId: autoStartId, enabled: next });
    } catch {
      setAutoStart(!next);
    }
  };
  const submitCommand = () => {
    const cmd = cmdInput.trim();
    if (!cmd || isExecBusy) return;
    setCmdInput("");
    onExec(project, cmd);
  };
  const canInstallUpgrade = !!d?.allReady && !installed;
  const canUpgrade = !!d?.allReady && !!installed;
  // 安装依赖：已安装即可单独重装依赖（不依赖 allReady，依赖缺失时可补装）
  const canInstallDeps = !!installed;
  // 卸载确认弹窗状态
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const endRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [logs]);

  return (
    <div className="rounded-2xl border border-white/10 bg-white/[0.02] overflow-hidden">
      {/* 卡片头部 */}
      <div className="flex items-center gap-3 px-5 py-4">
        <div className="w-10 h-10 rounded-xl bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)] border border-[var(--module-accent-ring)] flex items-center justify-center">
          <Icon className="w-5 h-5 text-[var(--module-accent)]" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-bold text-white">
              {project.displayName}
            </span>
            {isNpx && (
              <span
                className="px-1.5 py-0.5 rounded-md text-[9px] font-mono font-semibold bg-cyan-500/10 text-cyan-400 border border-cyan-500/20"
                title={t("nodeproj.npxBadgeTitle")}
              >
                npx
              </span>
            )}
            {isPip && (
              <span
                className="px-1.5 py-0.5 rounded-md text-[9px] font-mono font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/20"
                title={t("nodeproj.pipBadgeTitle")}
              >
                pip
              </span>
            )}
            <span
              className={`px-2 py-0.5 rounded-full text-[10px] font-semibold ${
                portConflict
                  ? "bg-red-500/10 text-red-400 border border-red-500/20"
                  : running
                    ? "bg-emerald-500/10 text-emerald-400 border border-emerald-500/20"
                    : installed
                      ? "bg-slate-500/10 text-slate-400 border border-slate-500/20"
                      : "bg-amber-500/10 text-amber-400 border border-amber-500/20"
              }`}
            >
              {portConflict
                ? t("nodeproj.portConflict")
                : running
                  ? t("nodeproj.running")
                  : installed
                    ? t("nodeproj.stopped")
                    : t("nodeproj.notInstalled")}
            </span>
          </div>
          {project.description && (
            <p className="text-[11px] text-slate-500 truncate">
              {project.description}
            </p>
          )}
        </div>
        <div className="flex items-center gap-1.5 text-[10px] text-slate-500 font-mono flex-shrink-0">
          {st?.pid && (
            <span className="flex items-center gap-1">
              <Terminal className="w-3 h-3" /> PID {st.pid}
            </span>
          )}
          {isNpx ? (
            <span className="flex items-center gap-1" title={project.npxPackage}>
              <Package className="w-3 h-3" /> {project.npxPackage}
              {st?.localVersion && (
                <span className="text-slate-600">v{st.localVersion}</span>
              )}
            </span>
          ) : (
            <span className="flex items-center gap-1">
              <GitBranch className="w-3 h-3" /> {st?.gitVersion ?? "—"}
            </span>
          )}
        </div>
      </div>

      {/* 环境检测条 */}
      <div className="px-5 pb-2 flex flex-wrap items-center gap-3 text-[10px]">
        {!isNpx && !isPip && <EnvBadge dep={d?.git} label="git" />}
        <EnvBadge
          dep={d?.node}
          label={`${project.runtime === "python" ? "python" : "node"} ${project.nodeRequirement || ""}`.trim()}
        />
        <EnvBadge
          dep={d?.packageManager}
          label={isNpx ? "npm" : isPip ? "pip" : project.packageManager}
        />
        {st?.port && <span className="text-slate-600">{t("nodeproj.portText", { port: st.port })}</span>}
        <label
          className="flex items-center gap-1 text-[10px] text-slate-400 hover:text-slate-200 cursor-pointer select-none"
          title={t("nodeproj.autoStartTitle")}
        >
          <input
            type="checkbox"
            checked={autoStart}
            onChange={() => void toggleAutoStart()}
            className="w-3 h-3 accent-[var(--module-accent)] cursor-pointer"
          />
          {t("nodeproj.autoStart")}
        </label>
      </div>

      {/* 端口冲突：明确端口 / 占用进程 / PID，并提供强制关闭 */}
      {portConflict && (
        <div className="mx-5 mb-2 p-2.5 rounded-lg bg-amber-500/10 border border-amber-500/20 flex items-center gap-2 flex-wrap">
          <AlertTriangle className="w-3.5 h-3.5 text-amber-400 flex-shrink-0" />
          <span className="text-[10px] text-amber-300 font-semibold">
            {t("nodeproj.conflictDetail", {
              port: st?.port ?? project.defaultPort,
              proc: st?.conflictProcess ?? "?",
              pid: st?.pid ?? "?",
            })}
          </span>
          <button
            type="button"
            onClick={() => onKillPortOwner(st?.port ?? project.defaultPort)}
            className="ml-auto px-2 py-1 rounded-md bg-red-600 hover:bg-red-500 text-[10px] font-semibold text-white cursor-pointer transition-all flex items-center gap-1"
          >
            <Square className="w-3 h-3" /> {t("nodeproj.forceKill")}
          </button>
        </div>
      )}

      {/* 更新检查：git 模式对比 commit；npx 模式对比本地版本与 npm registry 远程版本 */}
      {installed && (
        <div className="px-5 py-1.5 flex items-center gap-2 text-[11px]">
          {checkingUpdate ? (
            <span className="flex items-center gap-1.5 text-slate-400">
              <Loader2 className="w-3.5 h-3.5 animate-spin" /> {t("nodeproj.checkingUpdate")}
            </span>
          ) : updateInfo ? (
            updateInfo.error ? (
              <span className="flex items-center gap-1.5 text-amber-400">
                <AlertTriangle className="w-3.5 h-3.5" /> {t("nodeproj.checkFail")}
                {updateInfo.error}
              </span>
            ) : isNpx ? (
              updateInfo.hasUpdate ? (
                <span className="flex items-center gap-1.5 text-[var(--module-accent)]">
                  <RefreshCw className="w-3.5 h-3.5" />
                  {t("nodeproj.npxHasUpdate", {
                    local: updateInfo.currentVersion ?? "?",
                    latest: updateInfo.latestVersion ?? "?",
                  })}
                </span>
              ) : (
                <span className="flex items-center gap-1.5 text-emerald-400">
                  <CheckCircle2 className="w-3.5 h-3.5" />
                  {t("nodeproj.npxUpToDate", {
                    version: updateInfo.currentVersion ?? "?",
                  })}
                </span>
              )
            ) : updateInfo.hasUpdate ? (
              <span className="flex items-center gap-1.5 text-[var(--module-accent)]">
                <RefreshCw className="w-3.5 h-3.5" />
                {t("nodeproj.hasUpdate", { behind: updateInfo.behind })}
                <span className="text-slate-600">
                  {updateInfo.currentCommit} → {updateInfo.latestCommit}
                </span>
              </span>
            ) : (
              <span className="flex items-center gap-1.5 text-emerald-400">
                <CheckCircle2 className="w-3.5 h-3.5" /> {t("nodeproj.upToDate")}
                <span className="text-slate-600">
                  ({updateInfo.currentCommit})
                </span>
              </span>
            )
          ) : (
            <button
              onClick={() => onCheckUpdate(project)}
              disabled={isBusy || running}
              className="flex items-center gap-1.5 text-slate-400 hover:text-[var(--module-accent)] cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <RefreshCw className="w-3 h-3" /> {t("nodeproj.checkUpdate")}
            </button>
          )}
        </div>
      )}

      {/* 进度 / 错误 */}
      {(isBusy || prog) && (
        <div className="px-5 py-2 flex items-center gap-2 text-[11px] text-sky-300">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          <span>
            {prog?.phase === "done"
              ? t("nodeproj.phaseDone")
              : prog?.phase === "clone"
                ? t("nodeproj.phaseClone")
                : prog?.phase === "pull"
                  ? t("nodeproj.phasePull")
                  : prog?.phase === "npx"
                    ? t("nodeproj.phaseNpx")
                    : prog?.phase === "install"
                      ? t("nodeproj.phaseInstall")
                      : prog?.phase === "build"
                        ? t("nodeproj.phaseBuild")
                        : prog?.phase === "native"
                          ? t("nodeproj.phaseNative")
                          : prog?.phase === "exec"
                            ? t("nodeproj.phaseExec")
                              : prog?.phase === "running"
                                ? t("nodeproj.phaseStart")
                                : prog?.phase === "starting"
                                  ? t("nodeproj.phaseStart")
                                  : t("nodeproj.phaseOther")}
            {prog?.detail ? `：${prog.detail}` : ""}
          </span>
        </div>
      )}
      {error && (
        <div className="px-5 py-2 flex items-start gap-2 text-[11px] text-red-400 break-all">
          <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* 实时日志区 */}
      {logs.length > 0 && (
        <div className="px-5 py-1">
          <div className="flex items-center gap-1">
            <button
              onClick={onToggleLog}
              className="flex items-center gap-1.5 text-[10px] text-slate-500 hover:text-slate-300 cursor-pointer"
            >
              <Terminal className="w-3 h-3" />
              {logOpen ? t("nodeproj.logsToggleOpen") : t("nodeproj.logsToggleClosed")}
              <span className="text-slate-600">{t("nodeproj.logLines", { count: logs.length })}</span>
            </button>
            <div className="flex-1" />
            <button
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(logs.join("\n"));
                  setCopied(true);
                  window.setTimeout(() => setCopied(false), 1500);
                } catch {
                  /* 剪贴板不可用时静默忽略 */
                }
              }}
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] text-slate-500 hover:text-slate-300 hover:bg-white/5 cursor-pointer transition-all"
              title={t("nodeproj.copyLogs")}
            >
              <Copy className="w-3 h-3" />
              {copied ? t("nodeproj.copied") : t("nodeproj.copyLogs")}
            </button>
            <button
              onClick={() => onClearLogs(project.id)}
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] text-slate-500 hover:text-rose-400 hover:bg-rose-500/10 cursor-pointer transition-all"
              title={t("nodeproj.clearLogsBtn")}
            >
              <Eraser className="w-3 h-3" />
              {t("nodeproj.clearLogsBtn")}
            </button>
          </div>
          {logOpen && (
            <div
              className="mt-1 max-h-56 overflow-y-auto rounded-lg bg-black/40 border border-white/5 p-2 font-mono text-[10px] leading-relaxed"
              onClick={() => endRef.current?.scrollIntoView({ block: "end" })}
            >
              {logs.map((l, i) => (
                <div
                  key={i}
                  className={
                    l.startsWith("error") ||
                    l.includes("ERR!") ||
                    l.startsWith("fatal:")
                      ? "text-red-400"
                      : "text-slate-300"
                  }
                >
                  {l}
                </div>
              ))}
              <div
                ref={(el) => {
                  endRef.current = el;
                }}
              />
            </div>
          )}
        </div>
      )}

      {/* 命令执行：在服务运行目录内执行任意命令（PATH 已含 node_modules/.bin） */}
      {installed && (
        <div className="px-5 py-2 flex items-center gap-2">
          <div className="flex-1 flex items-center gap-1.5 bg-black/20 border border-white/10 rounded-lg px-2.5 py-1.5 focus-within:border-[var(--module-accent)] transition-all">
            <Terminal className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
            <input
              value={cmdInput}
              onChange={(e) => setCmdInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submitCommand();
              }}
              placeholder={t("nodeproj.execPlaceholder")}
              className="flex-1 bg-transparent outline-none text-[12px] text-slate-200 placeholder:text-slate-600 font-mono"
              spellCheck={false}
            />
          </div>
          <ActionButton
            disabled={!cmdInput.trim() || isBusy}
            busy={isExecBusy}
            onClick={submitCommand}
            icon={Terminal}
            color="bg-slate-700 hover:bg-slate-600"
            label={t("nodeproj.exec")}
            title={t("nodeproj.execTitle")}
          />
        </div>
      )}

      {/* 操作按钮：两行 4 列 Grid（上行=安装维护类，下行=运行类），列严格对齐 */}
      <div className="px-5 py-3 border-t border-white/5 grid grid-cols-4 gap-2">
          <ActionButton
            disabled={!canInstallUpgrade || isBusy || running || portConflict}
            busy={isBusy && busy === `install:${project.id}`}
            onClick={() => onAction(project, "install")}
            icon={Download}
            color="bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)]"
            label={t("nodeproj.install")}
          />
          <ActionButton
            disabled={!canUpgrade || isBusy || running}
            busy={isBusy && busy === `upgrade:${project.id}`}
            onClick={() => onAction(project, "upgrade")}
            icon={RefreshCw}
            color="bg-slate-700 hover:bg-slate-600"
            label={t("nodeproj.upgrade")}
          />
          {!isNpx && (
            <ActionButton
              disabled={!canInstallDeps || isBusy || running}
              busy={isBusy && busy === `install_deps:${project.id}`}
              onClick={() => onAction(project, "install_deps")}
              icon={Package}
              color="bg-sky-700 hover:bg-sky-600"
              label={t("nodeproj.installDeps")}
              title={t("nodeproj.installDepsTitle")}
            />
          )}
          {isNpx && installed && (
            <ActionButton
              disabled={!installed || isBusy || running}
              busy={isBusy && busy === `build_native:${project.id}`}
              onClick={() => onAction(project, "build_native")}
              icon={Hammer}
              color="bg-amber-700 hover:bg-amber-600"
              label={t("nodeproj.buildNative")}
              title={t("nodeproj.buildNativeTitle")}
            />
          )}
          <ActionButton
            disabled={!installed || isBusy}
            busy={isBusy && busy === `uninstall:${project.id}`}
            onClick={() => setConfirmUninstall(true)}
            icon={Trash2}
            color="bg-red-950/70 hover:bg-red-900/80"
            label={t("nodeproj.uninstall")}
            title={t("nodeproj.uninstallTitle")}
          />
          <ActionButton
            disabled={!installed || isBusy || running || portConflict}
            busy={isStarting}
            onClick={() => onAction(project, "start")}
            icon={Play}
            color="bg-emerald-600 hover:bg-emerald-500"
            label={t("nodeproj.start")}
          />
          <ActionButton
            disabled={!running || isBusy}
            busy={isStopping}
            onClick={() => onAction(project, "stop")}
            icon={Square}
            color="bg-red-600 hover:bg-red-500"
            label={t("nodeproj.stop")}
          />
          <ActionButton
            disabled={isBusy || (consoleUrlMode && !running)}
            busy={false}
            onClick={() => onOpenWeb(project)}
            icon={ExternalLink}
            color="bg-violet-600 hover:bg-violet-500"
            label={t("nodeproj.openHome")}
            title={
              consoleUrlMode
                ? consoleUrl
                  ? t("nodeproj.openHomeConsoleTitle", { url: consoleUrl })
                  : t("nodeproj.consoleUrlPending")
                : undefined
            }
          />
          {/* 第 4 列：状态提示（右侧对齐，保持与上行按钮列对齐） */}
          <div className="flex items-center justify-end gap-2 min-w-0">
            {consoleUrlMode && consoleUrl && (
              <span
                className="text-[10px] text-emerald-400/80 flex items-center gap-1 flex-shrink-0"
                title={consoleUrl}
              >
                <CheckCircle2 className="w-3 h-3" /> {t("nodeproj.consoleUrlReady")}
              </span>
            )}
            {!d?.allReady && installed && (
              <span
                className="text-[10px] text-amber-400 flex items-center gap-1 flex-shrink-0"
                title={t("nodeproj.depsNotReady")}
              >
                <AlertTriangle className="w-3 h-3 flex-shrink-0" />
              </span>
            )}
          </div>
      </div>

      {/* 卸载确认弹窗 */}
      <ConfirmDialog
        open={confirmUninstall}
        onCancel={() => setConfirmUninstall(false)}
        onConfirm={() => {
          setConfirmUninstall(false);
          onAction(project, "uninstall");
        }}
        title={t("nodeproj.uninstallConfirmTitle")}
        desc={t("nodeproj.uninstallConfirmDesc", { name: project.displayName })}
        confirmText={t("nodeproj.uninstall")}
        danger
      />
    </div>
  );
}

function EnvBadge({ dep, label }: { dep?: DepCheck; label: string }) {
  const ok = dep?.exists && dep.satisfies;
  return (
    <span
      className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md border font-mono ${
        ok
          ? "bg-emerald-500/10 border-emerald-500/20 text-emerald-400"
          : "bg-red-500/10 border-red-500/20 text-red-400"
      }`}
    >
      {ok ? (
        <CheckCircle2 className="w-3 h-3" />
      ) : (
        <XCircle className="w-3 h-3" />
      )}
      <span>{label}</span>
      {dep?.version && <span className="text-slate-500">({dep.version})</span>}
    </span>
  );
}

function ActionButton({
  disabled,
  busy,
  onClick,
  icon: Icon,
  color,
  label,
  title,
}: {
  disabled: boolean;
  busy: boolean;
  onClick: () => void;
  icon: React.ComponentType<{ className?: string }>;
  color: string;
  label: string;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`flex-1 justify-center px-2 py-1 rounded-lg text-[10px] font-semibold flex items-center gap-1.5 transition-all cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed ${color} text-white`}
    >
      {busy ? (
        <Loader2 className="w-3.5 h-3.5 animate-spin" />
      ) : (
        <Icon className="w-3.5 h-3.5" />
      )}
      {label}
    </button>
  );
}
