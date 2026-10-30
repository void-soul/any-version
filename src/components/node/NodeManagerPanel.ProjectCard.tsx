import { useState, useEffect, useRef } from "react";
import { Bot, Boxes, Package, Play, Square, Download, RefreshCw, ExternalLink, CheckCircle2, Loader2, GitBranch, Terminal, AlertTriangle, Copy, Eraser, Settings2, Trash2, Hammer, Sparkles } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { Note } from "../shared/Note";
import { ICONS, dockerDep, type DepCheckResult, type DockerEnv, type NodeProgress, type NodeProjectDef, type NodeProjectStatus, type NodeUpdateInfo } from "./NodeManagerPanel.model";
import { EnvBadge } from "./NodeManagerPanel.EnvBadge";
import { ActionButton } from "./NodeManagerPanel.ActionButton";
// ---- 项目卡片（复用于管理弹窗）----

export function ProjectCard({
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
  dockerEnv,
  onConfig,
  onAction,
  onExec,
  onInit,
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
  /// 本机 Docker 环境（docker 服务的环境条用它，替代 git/node/pm 三行）
  dockerEnv?: DockerEnv | null;
  onConfig: (p: NodeProjectDef) => void;
  onAction: (p: NodeProjectDef, action: string) => void;
  onExec: (p: NodeProjectDef, command: string) => void;
  onInit: (p: NodeProjectDef) => void;
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
  // Docker 运行方式：服务由 docker compose 托管（安装=拉镜像，启动=compose up -d）
  const isDocker = project.runtime === "docker";
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
    busy === `init:${project.id}` ||
    busy === `uninstall:${project.id}`;
  const isExecBusy = busy === `exec:${project.id}`;
  const isInitBusy = busy === `init:${project.id}`;
  // 装好了、配了初始化命令、但还没初始化 → 必须先跑一次才能用（如 wigolo 要先拉模型）
  const needsInit = !!installed && !!st?.initAvailable && !st?.initialized;
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
    <div className="rounded-panel border border-white/10 bg-white/[0.02] overflow-hidden">
      {/* 卡片头部 */}
      <div className="flex items-center gap-3 px-5 py-4">
        <div className="w-10 h-10 rounded-card bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)] border border-[var(--module-accent-ring)] flex items-center justify-center">
          <Icon className="w-5 h-5 text-[var(--module-accent)]" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[14px] font-bold text-white">
              {project.displayName}
            </span>
            {isNpx && (
              <span
                className="px-1.5 py-0.5 rounded-md text-micro font-mono font-semibold bg-cyan-500/10 text-cyan-400 border border-cyan-500/20"
                title={t("nodeproj.npxBadgeTitle")}
              >
                npx
              </span>
            )}
            {isPip && (
              <span
                className="px-1.5 py-0.5 rounded-md text-micro font-mono font-semibold bg-blue-500/10 text-blue-400 border border-blue-500/20"
                title={t("nodeproj.pipBadgeTitle")}
              >
                pip
              </span>
            )}
            <span
              className={`px-2 py-0.5 rounded-full text-tiny font-semibold ${
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
            <p className="text-caption text-slate-500 truncate">
              {project.description}
            </p>
          )}
        </div>
        <div className="flex items-center gap-1.5 text-tiny text-slate-500 font-mono flex-shrink-0">
          {isDocker ? (
            <>
              {/* 容器化服务没有本地 PID：「几个容器在跑」才是有用的信息 */}
              <span className="flex items-center gap-1">
                <Boxes className="w-3 h-3" /> {t("nodeproj.containerCount", { n: st?.containers ?? 0 })}
              </span>
              {project.docker?.project ? (
                <span className="text-slate-600">{project.docker.project}</span>
              ) : null}
            </>
          ) : (
            <>
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
            </>
          )}
        </div>
      </div>

      {/* 环境检测条：docker 服务看的是**本机 Docker**（CLI / compose / 引擎），
          而不是 git / node / pnpm —— 那三样对容器化服务没有任何意义 */}
      <div className="px-5 pb-2 flex flex-wrap items-center gap-3 text-tiny">
        {isDocker ? (
          <>
            <EnvBadge dep={dockerDep(dockerEnv?.cliVersion ?? "", !!dockerEnv?.cli)} label="docker" />
            <EnvBadge dep={dockerDep(dockerEnv?.composeVersion ?? "", !!dockerEnv?.compose)} label="compose" />
            <EnvBadge dep={dockerDep("", !!dockerEnv?.daemonRunning)} label={t("nodeproj.dockerEngine")} />
            {dockerEnv && !(dockerEnv.cli && dockerEnv.daemonRunning && dockerEnv.compose) && (
              // 未就绪时把结论（以及原文错误）摆出来：
              // Windows 上「装了 Docker Desktop 但没启动」是最常见的卡点
              <span
                className="min-w-0 truncate text-amber-400"
                title={dockerEnv.error || dockerEnv.summary}
              >
                {dockerEnv.summary}
              </span>
            )}
          </>
        ) : (
          <>
            {!isNpx && !isPip && <EnvBadge dep={d?.git} label="git" />}
            <EnvBadge
              dep={d?.node}
              label={`${project.runtime === "python" ? "python" : "node"} ${project.nodeRequirement || ""}`.trim()}
            />
            <EnvBadge
              dep={d?.packageManager}
              label={isNpx ? "npm" : isPip ? "pip" : project.packageManager}
            />
          </>
        )}
        {st?.port && <span className="text-slate-600">{t("nodeproj.portText", { port: st.port })}</span>}
        <label
          className="flex items-center gap-1 text-tiny text-slate-400 hover:text-slate-200 cursor-pointer select-none"
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
        <Note
          tone="warn"
          className="mx-5 mb-2 !p-2.5"
          title={t("nodeproj.conflictDetail", {
            port: st?.port ?? project.defaultPort,
            proc: st?.conflictProcess ?? "?",
            pid: st?.pid ?? "?",
          })}
          action={
            <button
              type="button"
              onClick={() => onKillPortOwner(st?.port ?? project.defaultPort)}
              className="px-2 py-1 rounded-md bg-red-600 hover:bg-red-500 text-tiny font-semibold text-white cursor-pointer transition-all flex items-center gap-1"
            >
              <Square className="w-3 h-3" /> {t("nodeproj.forceKill")}
            </button>
          }
        />
      )}

      {/* 更新检查：git 模式对比 commit；npx 模式对比本地版本与 npm registry 远程版本 */}
      {installed && (
        <div className="px-5 py-1.5 flex items-center gap-2 text-caption">
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
        <div className="px-5 py-2 flex items-center gap-2 text-caption text-sky-300">
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
        <div className="px-5 py-2 flex items-start gap-2 text-caption text-red-400 break-all">
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
              className="flex items-center gap-1.5 text-tiny text-slate-500 hover:text-slate-300 cursor-pointer"
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
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-tiny text-slate-500 hover:text-slate-300 hover:bg-white/5 cursor-pointer transition-all"
              title={t("nodeproj.copyLogs")}
            >
              <Copy className="w-3 h-3" />
              {copied ? t("nodeproj.copied") : t("nodeproj.copyLogs")}
            </button>
            <button
              onClick={() => onClearLogs(project.id)}
              className="flex items-center gap-1 px-1.5 py-0.5 rounded text-tiny text-slate-500 hover:text-rose-400 hover:bg-rose-500/10 cursor-pointer transition-all"
              title={t("nodeproj.clearLogsBtn")}
            >
              <Eraser className="w-3 h-3" />
              {t("nodeproj.clearLogsBtn")}
            </button>
          </div>
          {logOpen && (
            <div
              className="mt-1 max-h-56 overflow-y-auto rounded-ctl bg-black/40 border border-white/5 p-2 font-mono text-tiny leading-relaxed"
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
      {/* 首次初始化提示：装完还不能直接用，要先拉取运行时资源（wigolo 约 1.5GB） */}
      {needsInit && (
        <div className="px-5 pt-2">
          <div className="flex items-center gap-2 rounded-ctl border border-amber-500/20 bg-amber-500/10 px-3 py-2">
            <Sparkles className="w-3.5 h-3.5 text-amber-300 flex-shrink-0" />
            <div className="flex-1 text-caption text-amber-200/90 leading-snug">
              {t("nodeproj.initBanner")}
            </div>
            <div className="w-24 flex-shrink-0">
              <ActionButton
                disabled={isBusy}
                busy={isInitBusy}
                onClick={() => onInit(project)}
                icon={Sparkles}
                color="bg-amber-600 hover:bg-amber-500"
                label={t("nodeproj.init")}
                title={t("nodeproj.initTitle")}
              />
            </div>
          </div>
        </div>
      )}

      {/* docker 服务：只在定义里声明了 execService 时才给「执行命令」
          （没声明就说明作者没打算开放容器壳，猜一个容器名会在错误的容器里执行） */}
      {installed && (!isDocker || !!project.docker?.execService?.trim()) && (
        <div className="px-5 py-2 flex items-center gap-2">
          <div className="flex-1 flex items-center gap-1.5 bg-black/20 border border-white/10 rounded-ctl px-2.5 py-1.5 focus-within:border-[var(--module-accent)] transition-all">
            <Terminal className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
            <input
              value={cmdInput}
              onChange={(e) => setCmdInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submitCommand();
              }}
              placeholder={t("nodeproj.execPlaceholder")}
              className="flex-1 bg-transparent outline-none text-body text-slate-200 placeholder:text-slate-600 font-mono"
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
            label={isDocker ? t("nodeproj.pullImage") : t("nodeproj.install")}
          />
          <ActionButton
            disabled={!canUpgrade || isBusy || running}
            busy={isBusy && busy === `upgrade:${project.id}`}
            onClick={() => onAction(project, "upgrade")}
            icon={RefreshCw}
            color="bg-slate-700 hover:bg-slate-600"
            label={isDocker ? t("nodeproj.updateImage") : t("nodeproj.upgrade")}
          />
          {isDocker && (
            <ActionButton
              disabled={isBusy}
              busy={false}
              onClick={() => onConfig(project)}
              icon={Settings2}
              color="bg-slate-700 hover:bg-slate-600"
              label={t("nodeproj.configBtn")}
              title={t("nodeproj.configTitle")}
            />
          )}
          {!isNpx && !isDocker && (
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
            color="bg-[var(--module-accent)] hover:bg-[var(--module-accent-strong)]"
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
                className="text-tiny text-emerald-400/80 flex items-center gap-1 flex-shrink-0"
                title={consoleUrl}
              >
                <CheckCircle2 className="w-3 h-3" /> {t("nodeproj.consoleUrlReady")}
              </span>
            )}
            {!d?.allReady && installed && (
              <span
                className="text-tiny text-amber-400 flex items-center gap-1 flex-shrink-0"
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
        desc={
          isDocker
            ? // 说清楚「删了什么、留了什么」：容器化服务的卸载是可恢复的，
              // 但用户看到「卸载」两个字时最容易慌的就是数据
              t("nodeproj.uninstallConfirmDescDocker", { name: project.displayName })
            : t("nodeproj.uninstallConfirmDesc", { name: project.displayName })
        }
        confirmText={t("nodeproj.uninstall")}
        danger
      />
    </div>
  );
}
