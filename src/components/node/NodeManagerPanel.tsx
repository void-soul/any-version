// 通用「Node 项目管理器」面板
// 顶级功能：管理类似 deepseek-harness 的 Node 项目（安装/升级/启动/停止/打开主页）。
// 项目列表来自后端 node-projects/ 配置注册表，新增项目无需改前端代码。
//
// 布局说明：
// - 未打开任何服务时：全屏引导页，可点「打开服务管理」进入管理弹窗。
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
  CheckCircle2,
  XCircle,
  Loader2,
  GitBranch,
  Terminal,
  AlertTriangle,
  X,
  LayoutDashboard,
  Settings2,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import VexAvatar from "../VexAvatar";
import VexGreeting from "../VexGreeting";
import { useTranslation } from "react-i18next";

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
  packageManager: string;
  buildScript: string;
  startCmd: string[];
  managed: boolean;
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
  pid?: number | null;
  gitVersion?: string | null;
  error?: string | null;
}

interface NodeUpdateInfo {
  hasUpdate: boolean;
  currentCommit: string;
  latestCommit: string;
  behind: number;
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
  const [progress, setProgress] = useState<Record<string, NodeProgress>>({});
  const [error, setError] = useState<Record<string, string>>({});
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const [logOpen, setLogOpen] = useState<Record<string, boolean>>({});
  const logEndRefs = useRef<Record<string, HTMLDivElement | null>>({});
  // 内部主页 Tab 管理（在主窗口内 iframe 打开各 Node 应用界面，服务区全屏）
  const [tabs, setTabs] = useState<NodeProjectDef[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  // 服务管理弹窗
  const [manageOpen, setManageOpen] = useState(false);
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
    const setup = async () => {
      unlisten = await listen<NodeProgress>("npm-progress", (e) => {
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
    };
    setup();
    return () => unlisten?.();
  }, []);

  // 监听实时日志（git pull / install / build / start）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      unlisten = await listen<NodeLog>("npm-log", (e) => {
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
    };
    setup();
    return () => unlisten?.();
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
          error: typeof e === "string" ? e : String(e),
        },
      }));
    } finally {
      setCheckingUpdate(null);
    }
  };

  // 在主窗口内部打开应用主页（iframe 全屏）。打开前校验端口是否监听。
  const openWeb = async (project: NodeProjectDef) => {
    setError((prev) => ({ ...prev, [project.id]: "" }));
    if (tabs.some((t) => t.id === project.id)) {
      setActiveTabId(project.id);
      setManageOpen(false);
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
    setManageOpen(false);
  };

  const closeTab = (id: string) => {
    setTabs((prev) => {
      const idx = prev.findIndex((t) => t.id === id);
      if (idx === -1) return prev;
      const next = prev.filter((t) => t.id !== id);
      if (activeTabId === id) {
        setActiveTabId(next[idx] ? next[idx].id : (next[idx - 1]?.id ?? null));
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

  // 主界面：无 Tab 时显示全屏引导页
  if (tabs.length === 0 && !manageOpen) {
    return (
      <div className="h-full flex flex-col items-center justify-center select-none">
        <div className="text-center space-y-4">
          <VexAvatar size={64} className="mx-auto" />
          <div>
            <h1 className="text-lg font-bold text-white">{t("nodeproj.servicesTitle")}</h1>
            <p className="text-[12px] text-slate-500 mt-1">
              {t("nodeproj.servicesDesc")}
            </p>
            <p className="text-[11px] text-slate-400 mt-2">
              <VexGreeting seconds={9} />
            </p>
          </div>
          <button
            onClick={() => setManageOpen(true)}
            className="px-5 py-2.5 bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)] text-white rounded-xl text-[13px] font-semibold flex items-center gap-2 mx-auto cursor-pointer transition-all"
          >
            <Settings2 className="w-4 h-4" /> {t("nodeproj.openManage")}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col min-h-0 select-none">
      {/* 服务区全屏：Tab 栏 + iframe */}
      {tabs.length > 0 && (
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
            {/* 管理按钮 */}
            <button
              onClick={() => setManageOpen(true)}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-[11px] font-semibold text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex-shrink-0"
              title={t("nodeproj.openManage")}
            >
              <Settings2 className="w-3.5 h-3.5" /> {t("nodeproj.manage")}
            </button>
          </div>

          {/* 内容区（全屏 iframe） */}
          <div className="flex-1 min-h-0">
            {activeTab ? (
              <iframe
                key={activeTab.id}
                src={resolvedWebPath(activeTab)}
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

      {/* 服务管理弹窗 */}
      {manageOpen && (
        <div
          className="fixed inset-0 z-50 modal-mask flex items-center justify-center p-6 bg-black/60 backdrop-blur-sm"
        >
          <div
            className="w-full max-w-2xl max-h-[85vh] flex flex-col rounded-2xl border border-white/10 bg-[#0b0f1a] overflow-hidden shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            {/* 弹窗头部 */}
            <div className="flex items-center gap-2 px-5 py-3.5 border-b border-white/10 bg-white/[0.02]">
              <Settings2 className="w-4 h-4 text-[var(--module-accent)]" />
              <h2 className="text-sm font-bold text-white">{t("nodeproj.manageTitle")}</h2>
              <span className="text-[10px] text-slate-500 ml-1">
                {t("nodeproj.manageSub")}
              </span>
              <div className="flex-1" />
              <button
                onClick={() => setManageOpen(false)}
                className="p-1.5 rounded-lg hover:bg-white/10 text-slate-400 hover:text-white cursor-pointer transition-all"
                title={t("nodeproj.close")}
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* 项目卡片列表（可滚动） */}
            <div className="flex-1 min-h-0 overflow-y-auto p-4 space-y-3">
              {managedProjects.map((project) => {
                const st = statuses[project.id];
                const d = deps[project.id];
                const prog = progress[project.id];
                const isStarting = busy === `start:${project.id}`;
                const isStopping = busy === `stop:${project.id}`;
                return (
                  <ProjectCard
                    key={project.id}
                    project={project}
                    st={st}
                    d={d}
                    prog={prog}
                    busy={busy}
                    isStarting={isStarting}
                    isStopping={isStopping}
                    logs={logs[project.id] ?? []}
                    logOpen={!!logOpen[project.id]}
                    error={error[project.id]}
                    updateInfo={updateInfo[project.id]}
                    checkingUpdate={checkingUpdate === project.id}
                    onAction={runAction}
                    onOpenWeb={openWeb}
                    onCheckUpdate={checkUpdate}
                    onToggleLog={() =>
                      setLogOpen((prev) => ({
                        ...prev,
                        [project.id]: !prev[project.id],
                      }))
                    }
                  />
                );
              })}
              {managedProjects.length === 0 && (
                <div className="py-10 text-center text-slate-500 text-sm">
                  {t("nodeproj.noManaged")}
                </div>
              )}
            </div>
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
  onAction,
  onOpenWeb,
  onCheckUpdate,
  onToggleLog,
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
  onAction: (p: NodeProjectDef, action: string) => void;
  onOpenWeb: (p: NodeProjectDef) => void;
  onCheckUpdate: (p: NodeProjectDef) => void;
  onToggleLog: () => void;
}) {
  const { t } = useTranslation();
  const Icon = ICONS[project.icon] ?? Bot;
  const installed = st?.installed;
  const running = st?.status === "running";
  const portConflict = st?.status === "port_conflict";
  const isBusy =
    busy === `install:${project.id}` ||
    busy === `upgrade:${project.id}` ||
    busy === `install_deps:${project.id}`;
  const canInstallUpgrade = !!d?.allReady && !installed;
  const canUpgrade = !!d?.allReady && !!installed;
  // 安装依赖：已安装即可单独重装依赖（不依赖 allReady，依赖缺失时可补装）
  const canInstallDeps = !!installed;
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
          <span className="flex items-center gap-1">
            <GitBranch className="w-3 h-3" /> {st?.gitVersion ?? "—"}
          </span>
        </div>
      </div>

      {/* 环境检测条 */}
      <div className="px-5 pb-2 flex flex-wrap items-center gap-3 text-[10px]">
        <EnvBadge dep={d?.git} label="git" />
        <EnvBadge
          dep={d?.node}
          label={`node ${project.nodeRequirement || ""}`.trim()}
        />
        <EnvBadge dep={d?.packageManager} label={project.packageManager} />
        {st?.port && <span className="text-slate-600">{t("nodeproj.portText", { port: st.port })}</span>}
      </div>

      {/* git 更新检查 */}
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
                  : prog?.phase === "install"
                    ? t("nodeproj.phaseInstall")
                    : prog?.phase === "build"
                      ? t("nodeproj.phaseBuild")
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
          <button
            onClick={onToggleLog}
            className="flex items-center gap-1.5 text-[10px] text-slate-500 hover:text-slate-300 cursor-pointer"
          >
            <Terminal className="w-3 h-3" />
            {logOpen ? t("nodeproj.logsToggleOpen") : t("nodeproj.logsToggleClosed")}
            <span className="text-slate-600">{t("nodeproj.logLines", { count: logs.length })}</span>
          </button>
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

      {/* 操作按钮 */}
      <div className="px-5 py-3 border-t border-white/5 flex items-center gap-2">
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
        <ActionButton
          disabled={!canInstallDeps || isBusy || running}
          busy={isBusy && busy === `install_deps:${project.id}`}
          onClick={() => onAction(project, "install_deps")}
          icon={Package}
          color="bg-sky-700 hover:bg-sky-600"
          label={t("nodeproj.installDeps")}
          title={t("nodeproj.installDepsTitle")}
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
          disabled={isBusy}
          busy={false}
          onClick={() => onOpenWeb(project)}
          icon={ExternalLink}
          color="bg-violet-600 hover:bg-violet-500"
          label={t("nodeproj.openHome")}
        />
        <div className="flex-1" />
        {!d?.allReady && installed && (
          <span className="text-[10px] text-amber-400 flex items-center gap-1">
            <AlertTriangle className="w-3 h-3" /> {t("nodeproj.depsNotReady")}
          </span>
        )}
      </div>
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
      className={`px-3 py-1.5 rounded-lg text-[11px] font-semibold flex items-center gap-1.5 transition-all cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed ${color} text-white`}
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
