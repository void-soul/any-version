import { useState, useEffect, useCallback, useRef } from "react";
import { Bot, Boxes, RefreshCw, Code2, Loader2, X, LayoutDashboard, Settings2 } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { ICONS, MANAGE_TAB, MAX_LOG_LINES, isPortListening, resolvedWebPath, type DepCheckResult, type DockerEnv, type NodeConsoleUrl, type NodeLog, type NodeProgress, type NodeProjectDef, type NodeProjectStatus, type NodeUpdateInfo } from "./NodeManagerPanel.model";
import { ProjectCard } from "./NodeManagerPanel.ProjectCard";
import { DockerConfigDialog } from "./NodeManagerPanel.DockerConfigDialog";
export default function NodeManagerPanel() {
  const { t } = useTranslation();
  const [projects, setProjects] = useState<NodeProjectDef[]>([]);
  const [loaded, setLoaded] = useState(false); // 首次列表是否已加载完成
  const [deps, setDeps] = useState<Record<string, DepCheckResult>>({});
  /// 本机 Docker 环境（含 docker 服务时才有值）
  const [dockerEnv, setDockerEnv] = useState<DockerEnv | null>(null);
  /// 正在编辑端口 / 数据库密码的 docker 服务
  const [configTarget, setConfigTarget] = useState<NodeProjectDef | null>(null);
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

  /// Docker 环境：探测要跑 3 条 docker 命令（约数百毫秒），所以只在
  /// 「列表里确有 docker 服务」或「刚操作过 docker 服务」时刷新，不挂在 4s 轮询上。
  const refreshDockerEnv = useCallback(async () => {
    try {
      setDockerEnv(await invoke<DockerEnv>("docker_env_check"));
    } catch {
      setDockerEnv(null);
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
    // 有 docker 服务才探测本机 Docker 环境（否则每次刷新都白跑几条 docker 命令）
    if (list.some((p) => p.runtime === "docker")) void refreshDockerEnv();
  }, [refreshDeps, refreshStatus, refreshDockerEnv]);

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
      // 操作可能改变了 Docker 环境（刚装/刚启动 Docker Desktop 等），顺手重探一次
      if (project.runtime === "docker") void refreshDockerEnv();
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

  // 首次初始化（如 wigolo 的 `init`：下载浏览器引擎与端上模型，约 1.5GB）。
  // 耗时与输出都只在日志里可见，所以先自动展开日志区。
  const initProject = async (project: NodeProjectDef) => {
    setBusy(`init:${project.id}`);
    setError((prev) => ({ ...prev, [project.id]: "" }));
    setLogOpen((prev) => ({ ...prev, [project.id]: true }));
    try {
      await invoke("npm_init", { projectId: project.id });
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
              path: resolvedWebPath(project, st?.port),
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
        [project.id]: t("nodeproj.serviceStateHint", { label: stateLabel, path: resolvedWebPath(project, st?.port) }),
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
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-2 px-5 py-3 border-b border-white/10 bg-white/[0.02]">
        <Settings2 className="w-4 h-4 text-[var(--module-accent)]" />
        <h2 className="text-sm font-bold text-white">{t("nodeproj.manageTitle")}</h2>
        <span className="text-tiny text-slate-500 ml-1">{t("nodeproj.manageSub")}</span>
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
                  className={`text-caption font-semibold truncate flex-1 ${
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
            <div className="px-3 py-6 text-center text-tiny text-slate-500">
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
              dockerEnv={dockerEnv}
              onConfig={(p) => setConfigTarget(p)}
              onAction={runAction}
              onExec={execCommand}
              onInit={initProject}
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
          <div className="flex items-center gap-1 px-2 pt-1.5 pb-0 bg-white/[0.02] border-b border-white/10 overflow-x-auto">
            <LayoutDashboard className="w-3.5 h-3.5 text-[var(--module-accent)] ml-1 flex-shrink-0" />
            {tabs.map((tab) => {
              const Icon = ICONS[tab.icon] ?? Bot;
              const active = tab.id === activeTabId;
              return (
                <button
                  key={tab.id}
                  onClick={() => setActiveTabId(tab.id)}
                  className={`group flex items-center gap-1.5 px-3 py-1.5 rounded-t-ctl text-caption font-semibold transition-all cursor-pointer flex-shrink-0 ${
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
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-ctl text-caption font-semibold text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex-shrink-0 disabled:opacity-40 disabled:cursor-not-allowed"
              title={t("nodeproj.refreshHomeTitle")}
            >
              <RefreshCw className="w-3.5 h-3.5" /> {t("nodeproj.refreshHome")}
            </button>
            <button
              onClick={() => activeTab && void openDevTools(activeTab)}
              disabled={!activeTab}
              className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-ctl text-caption font-semibold text-slate-300 hover:text-white hover:bg-white/10 cursor-pointer transition-all flex-shrink-0 disabled:opacity-40 disabled:cursor-not-allowed"
              title={t("nodeproj.devToolsTitle")}
            >
              <Code2 className="w-3.5 h-3.5" /> {t("nodeproj.devTools")}
            </button>
            {/* 服务管理入口（超链接样式，区别于两侧的刷新/开发者工具按钮） */}
            <button
              onClick={() => setActiveTabId(MANAGE_TAB)}
              className={`group flex items-center gap-1 px-1 py-1 text-caption cursor-pointer transition-colors flex-shrink-0 ${
                activeTabId === MANAGE_TAB
                  ? "text-[var(--module-accent)]"
                  : "text-slate-400 hover:text-[var(--module-accent)]"
              }`}
              title={t("nodeproj.openManage")}
            >
              <Settings2 className="w-3.5 h-3.5" />
              <span
                className={`underline-offset-2 decoration-[var(--module-accent)]/70 ${
                  activeTabId === MANAGE_TAB ? "underline" : "group-hover:underline"
                }`}
              >
                {t("nodeproj.manage")}
              </span>
            </button>
          </div>

          {/* 内容区：管理页 / 全屏 iframe */}
          <div className="flex-1 min-h-0">
            {activeTabId === MANAGE_TAB ? (
              managePage
            ) : activeTab ? (
              <iframe
                src={consoleUrls[activeTab.id] || resolvedWebPath(activeTab, statuses[activeTab.id]?.port)}
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

      {/* docker 服务的端口 / 数据库密码配置 */}
      {configTarget && (
        <DockerConfigDialog
          project={configTarget}
          onClose={() => setConfigTarget(null)}
          onSaved={() => {
            void refreshStatus(configTarget.id);
            void refreshDockerEnv();
          }}
        />
      )}
    </div>
  );
}
