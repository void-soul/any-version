import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Activity, Columns2, Files, FolderOpen, GitBranch, Network, PanelRightClose, PanelRightOpen, Play, TerminalSquare, X } from "lucide-react";
import { SshTerminal, type RunCommand } from "../ssh/SshTerminal";
import { SftpPanel } from "../ssh/SftpPanel";
import { ForwardPanel } from "../ssh/ForwardPanel";
import { SshMonitorPanel } from "../ssh/SshMonitorPanel";
import { SshSnippetPanel } from "../ssh/SshSnippetPanel";
import { ServerStatusPanel } from "../ssh/ServerStatusPanel";
import { CommandHistoryPanel, loadHistory, pushHistory } from "../ssh/CommandHistoryPanel";
import { splitTabs, type SessionTab, type TabsState } from "./sessionTabs";

const tabBtn = "inline-flex items-center gap-1.5 rounded-t px-3 py-1.5 text-tiny border border-b-0 border-white/10 text-slate-300 hover:bg-white/5";
const iconBtn = "rounded p-1 text-slate-400 hover:bg-white/10 hover:text-slate-100";
const iconBtnWide = "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-micro text-slate-400 hover:bg-white/10 hover:text-slate-100";

function kindIcon(kind: SessionTab["kind"]) {
  if (kind === "sftp") return <Files className="h-3.5 w-3.5" />;
  if (kind === "forward") return <GitBranch className="h-3.5 w-3.5" />;
  if (kind === "process") return <Activity className="h-3.5 w-3.5" />;
  if (kind === "net") return <Network className="h-3.5 w-3.5" />;
  return <TerminalSquare className="h-3.5 w-3.5" />;
}

/**
 * 多会话工作区，布局对齐 FinalShell：
 * 终端占主区，右侧常驻「服务器状态 + 命令缓存」，下方可展开文件管理。
 * 所有标签都保持挂载（只隐藏非可见项），否则切换标签会断开 SSH 会话。
 */
export function SessionWorkspace({ state, titleFor, onActivate, onClose, onToggleSplit, onSplitBlocked, onOpenMonitor }: {
  state: TabsState;
  titleFor: (tab: SessionTab) => string;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onToggleSplit: () => void;
  /** 会话数不足两个时点击分屏的反馈（禁用按钮的「没反应」用户看不出来） */
  onSplitBlocked?: () => void;
  /** 从状态面板/底部按钮打开进程或网络列表 */
  onOpenMonitor?: (connectionId: string, view: "process" | "net") => void;
}) {
  const { t } = useTranslation();
  const visible = new Set(splitTabs(state).map((t) => t.id));
  const split = Boolean(state.splitId);
  const canSplit = state.tabs.length >= 2;
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [bottomTab, setBottomTab] = useState<Record<string, "files" | "snippets" | undefined>>({});
  const [history, setHistory] = useState<Record<string, string[]>>({});
  const [runCmd, setRunCmd] = useState<Record<string, RunCommand>>({});

  const recordCommand = (tab: SessionTab, cmd: string) => {
    const next = pushHistory(tab.connectionId, cmd);
    setHistory((prev) => ({ ...prev, [tab.id]: next }));
  };

  const runFromHistory = (tab: SessionTab, text: string) => {
    setRunCmd((prev) => ({ ...prev, [tab.id]: { text, seq: (prev[tab.id]?.seq ?? 0) + 1 } }));
  };

  const historyFor = (tab: SessionTab) => history[tab.id] ?? loadHistory(tab.connectionId);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-end gap-1 overflow-x-auto border-b border-white/10 pb-0">
        {state.tabs.map((tab) => (
          <div
            key={tab.id}
            className={`${tabBtn} ${state.activeId === tab.id ? "bg-white/10 text-slate-100" : ""}`}
          >
            <button className="inline-flex items-center gap-1.5" onClick={() => onActivate(tab.id)}>
              {kindIcon(tab.kind)}
              <span className="max-w-[16rem] truncate">{tab.title}</span>
            </button>
            <button className={iconBtn} title={t("tabs.close")} onClick={() => onClose(tab.id)}>
              <X className="h-3 w-3" />
            </button>
          </div>
        ))}
        <button
          className={`${iconBtn} mb-1 ml-1 ${canSplit ? "" : "opacity-50"}`}
          title={canSplit ? (split ? t("tabs.unsplit") : t("tabs.split")) : t("tabs.splitNeedsTwo")}
          aria-disabled={!canSplit}
          onClick={() => (canSplit ? onToggleSplit() : onSplitBlocked?.())}
        >
          <Columns2 className={`h-4 w-4 ${split ? "text-sky-400" : ""}`} />
        </button>
        <button
          className={`${iconBtn} mb-1`}
          title={sidebarOpen ? t("layout.hideSidebar") : t("layout.showSidebar")}
          onClick={() => setSidebarOpen((v) => !v)}
        >
          {sidebarOpen ? <PanelRightClose className="h-4 w-4" /> : <PanelRightOpen className="h-4 w-4" />}
        </button>
      </div>

      <div className="flex min-h-0 flex-1 gap-1 pt-1">
        {state.tabs.map((tab) => {
          const shown = visible.has(tab.id);
          const isTerminal = tab.kind === "terminal";
          // 分屏时同样保留侧栏与文件区，只把宽度/高度收窄，避免功能被切掉
          const withSidebar = isTerminal && sidebarOpen;
          const showBottom = isTerminal ? bottomTab[tab.id] : undefined;
          return (
            <div
              key={tab.id}
              className={shown ? "flex min-w-0 flex-1 flex-col overflow-hidden rounded border border-white/10" : "hidden"}
            >
              {tab.kind === "sftp" ? (
                <SftpPanel connectionId={tab.connectionId} title={titleFor(tab)} onClose={() => onClose(tab.id)} />
              ) : tab.kind === "forward" ? (
                <div className="p-2">
                  <ForwardPanel connectionId={tab.connectionId} title={titleFor(tab)} onClose={() => onClose(tab.id)} />
                </div>
              ) : tab.kind === "process" || tab.kind === "net" ? (
                <SshMonitorPanel
                  connectionId={tab.connectionId}
                  view={tab.kind}
                  title={titleFor(tab)}
                  onClose={() => onClose(tab.id)}
                />
              ) : (
                <>
                  <div className="flex min-h-0 flex-1">
                    <div className="min-w-0 flex-1">
                      <SshTerminal
                        connectionId={tab.connectionId}
                        title={titleFor(tab)}
                        onClose={() => onClose(tab.id)}
                        onCommand={(cmd) => recordCommand(tab, cmd)}
                        runCommand={runCmd[tab.id]}
                      />
                    </div>
                    {withSidebar && (
                      <aside className={`flex shrink-0 flex-col gap-2 overflow-auto border-l border-white/10 p-2 ${split ? "w-40" : "w-60"}`}>
                        <ServerStatusPanel
                          connectionId={tab.connectionId}
                          active={shown}
                          onOpenProcess={onOpenMonitor ? () => onOpenMonitor(tab.connectionId, "process") : undefined}
                          onOpenNet={onOpenMonitor ? () => onOpenMonitor(tab.connectionId, "net") : undefined}
                        />
                        <CommandHistoryPanel
                          connectionId={tab.connectionId}
                          items={historyFor(tab)}
                          onRun={(cmd) => runFromHistory(tab, cmd)}
                          onChanged={(next) => setHistory((prev) => ({ ...prev, [tab.id]: next }))}
                        />
                      </aside>
                    )}
                  </div>
                  {isTerminal && (
                    <div className="flex items-center gap-2 border-t border-white/10 px-2 py-1">
                      <button
                        className={iconBtnWide}
                        onClick={() => setBottomTab((prev) => ({
                          ...prev,
                          [tab.id]: prev[tab.id] === "files" ? undefined : "files",
                        }))}
                      >
                        <FolderOpen className="h-3 w-3" />
                        {t("layout.files")}
                      </button>
                      <button
                        className={iconBtnWide}
                        onClick={() => setBottomTab((prev) => ({
                          ...prev,
                          [tab.id]: prev[tab.id] === "snippets" ? undefined : "snippets",
                        }))}
                      >
                        <Play className="h-3 w-3" />
                        {t("snippets.title")}
                      </button>
                      <button
                        className={iconBtnWide}
                        onClick={() => onOpenMonitor?.(tab.connectionId, "process")}
                      >
                        <Activity className="h-3 w-3" />
                        {t("ssh.process")}
                      </button>
                      <button
                        className={iconBtnWide}
                        onClick={() => onOpenMonitor?.(tab.connectionId, "net")}
                      >
                        <Network className="h-3 w-3" />
                        {t("ssh.netlinks")}
                      </button>
                    </div>
                  )}
                  {showBottom && (
                    <div className={`border-t border-white/10 p-2 ${split ? "h-[26%] min-h-[7rem]" : "h-[38%] min-h-[9rem]"}`}>
                      {showBottom === "files" ? (
                        <SftpPanel
                          connectionId={tab.connectionId}
                          title={titleFor(tab)}
                          onClose={() => setBottomTab((prev) => ({ ...prev, [tab.id]: undefined }))}
                        />
                      ) : (
                        <SshSnippetPanel onRun={(command) => runFromHistory(tab, command)} />
                      )}
                    </div>
                  )}
                </>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
