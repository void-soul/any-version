import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { RefreshCw, Trash2, Check, Loader2, MessageSquareText, ClipboardCopy, FolderOpen, Search, X, ChevronDown, ChevronRight, Folder, GitFork, FolderInput } from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import { theamedConfirm } from "../shared/ThemedAlert";
import { buildSessionGroups, formatCwd, formatRelative, resolveGroupLabel, sessionActivityAt, type BuddySessionDeleteReport, type BuddySessionForkReport, type BuddySessionMoveReport, type BuddySessionRecord, type Tab } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction, RefObject } from "react";

/** Owns the BuddySessions feature, independently of page rendering. */
export function useBuddySessions({ platformEpochRef, setMessage, platform, tab, showMsg }: {
  platformEpochRef: RefObject<number>;
  setMessage: Dispatch<SetStateAction<{ ok: boolean; text: string; } | null>>;
  platform: string;
  tab: Tab;
  showMsg: (ok: boolean, text: string) => void;
}) {
  const { t } = useTranslation();

  // 会话
  const [sessions, setSessions] = useState<BuddySessionRecord[]>([]);

  const [sessionKeyword, setSessionKeyword] = useState("");

  const [sessionStatus, setSessionStatus] = useState("");

  const [sessionsBusy, setSessionsBusy] = useState(false);

  const [copiedId, setCopiedId] = useState<string | null>(null);

  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());

  // 待删除的会话（确认弹窗，支持批量）
  const [sessionDelete, setSessionDelete] = useState<{ ids: string[]; label: string } | null>(null);

  // 会话多选（批量删除）
  const [selectedSessionIds, setSelectedSessionIds] = useState<Set<string>>(new Set());

  // 上一次「改目录」的备份目录：留个入口让用户能打开/核对，确认无误后自己忽略掉
  const [lastMoveBackup, setLastMoveBackup] = useState("");

  // 会话加载（关键字防抖 300ms，与参考实现一致）
  const loadSessions = useCallback(async () => {
    const epoch = platformEpochRef.current;
    setSessionsBusy(true);
    try {
      const list = await invoke<BuddySessionRecord[]>("buddy_list_sessions", {
        platform,
        keyword: sessionKeyword || null,
        status: sessionStatus || null,
      });
      if (platformEpochRef.current !== epoch) return;
      setSessions(list ?? []);
      // 加载后默认展开全部分组
      setExpandedGroups(new Set((list ?? []).map((s) => s.cwd)));
    } catch (e) {
      if (platformEpochRef.current !== epoch) return;
      setMessage({ ok: false, text: String(e) });
    } finally {
      if (platformEpochRef.current === epoch) setSessionsBusy(false);
    }
  }, [platform, sessionKeyword, sessionStatus]);

  useEffect(() => {
    if (tab === "sessions") loadSessions();
  }, [tab, loadSessions]);

  // 关键字防抖：输入停止 300ms 后再触发查询
  useEffect(() => {
    const timer = setTimeout(() => {
      if (tab === "sessions") loadSessions();
    }, 300);
    return () => clearTimeout(timer);
  }, [sessionKeyword, tab, loadSessions]);

  // ─── 会话操作 ───

  const copySessionId = async (id: string) => {
    try {
      await navigator.clipboard.writeText(id);
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 1500);
    } catch {
      // ignore
    }
  };

  // 删除会话（数据库记录 + 本地会话文件，支持批量）
  const deleteSessionsByIds = async (ids: string[]) => {
    if (ids.length === 0) return;
    setSessionsBusy(true);
    try {
      const report = await invoke<BuddySessionDeleteReport>("buddy_delete_sessions", {
        platform,
        conversationIds: ids,
      });
      if (report.errors.length > 0) {
        showMsg(false, `${t("buddy.sessions.deletePartial")}: ${report.errors.join("；")}`);
      } else {
        showMsg(true, t("buddy.sessions.deleteDone"));
      }
      setSelectedSessionIds(new Set());
      await loadSessions();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setSessionsBusy(false);
      setSessionDelete(null);
    }
  };

  // 分叉会话：复制正文 + 用新 id 注册一条记录（原会话保持不动）
  const forkSessionById = async (id: string) => {
    setSessionsBusy(true);
    try {
      const report = await invoke<BuddySessionForkReport>("buddy_fork_session", {
        platform,
        conversationId: id,
      });
      if (report.contentMissing) {
        showMsg(false, t("buddy.sessions.forkNoContent"));
      } else if (report.errors.length > 0) {
        showMsg(false, `${t("buddy.sessions.forkPartial")}: ${report.errors.join("；")}`);
      } else {
        showMsg(true, t("buddy.sessions.forkDone", { title: report.title }));
      }
      await loadSessions();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setSessionsBusy(false);
    }
  };

  /**
   * 改会话所属目录：连正文一起搬到目标目录对应的工作区（不是只改列表里的分组文字）。
   * 搬之前后端会整批备份一次，备份路径回传到这里给用户留个入口。
   */
  const moveSessionsToDir = async (ids: string[]) => {
    let dir: string | null = null;
    try {
      const picked = await openDialog({
        directory: true,
        title: t("buddy.sessions.movePickTitle"),
      });
      dir = typeof picked === "string" ? picked : null;
    } catch {
      dir = null;
    }
    if (!dir) return;
    const ok = await theamedConfirm(t("buddy.sessions.moveConfirm", { count: ids.length, dir }));
    if (!ok) return;
    setSessionsBusy(true);
    try {
      const report = await invoke<BuddySessionMoveReport>("buddy_move_sessions", {
        platform,
        conversationIds: ids,
        targetDir: dir,
      });
      if (report.skipped.length > 0) {
        showMsg(
          false,
          `${t("buddy.sessions.movePartial", { count: report.moved })}：${report.skipped.join("；")}`,
        );
      } else {
        showMsg(true, t("buddy.sessions.moveDone", { count: report.moved, dir }));
      }
      if (report.backupDir) setLastMoveBackup(report.backupDir);
      await loadSessions();
    } catch (e) {
      showMsg(false, String(e));
    } finally {
      setSessionsBusy(false);
    }
  };

  const toggleSessionSelect = (id: string) => {
    setSelectedSessionIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleSelectAllSessions = () => {
    setSelectedSessionIds((prev) =>
      sessions.length > 0 && prev.size === sessions.length
        ? new Set()
        : new Set(sessions.map((s) => s.conversationId))
    );
  };

  // 切换平台时清空会话多选（不同平台的会话 id 空间不同）
  useEffect(() => {
    setSelectedSessionIds(new Set());
  }, [platform]);

const renderSessionsWorkspace = () => (<>
          <div className="flex items-center gap-2 px-4 py-2 border-b border-white/5 flex-shrink-0">
            <div className="flex items-center gap-1.5 bg-black/30 rounded-ctl border border-white/10 px-2 py-1 flex-1 max-w-xs">
              <Search className="w-3 h-3 text-slate-500" />
              <input
                value={sessionKeyword}
                onChange={(e) => setSessionKeyword(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && loadSessions()}
                placeholder={t("buddy.sessions.searchPlaceholder")}
                className="bg-transparent text-caption text-white outline-none w-full placeholder:text-slate-600"
              />
            </div>
            <select
              value={sessionStatus}
              onChange={(e) => setSessionStatus(e.target.value)}
              className="bg-black/30 border border-white/10 rounded-ctl px-2 py-1 text-caption text-slate-300 outline-none cursor-pointer"
            >
              <option value="">{t("buddy.sessions.statusAll")}</option>
              <option value="Completed">{t("buddy.sessions.statusCompleted")}</option>
              <option value="InProgress">{t("buddy.sessions.statusInProgress")}</option>
            </select>
            <div className="flex-1" />
            <button
              onClick={toggleSelectAllSessions}
              disabled={sessions.length === 0}
              className="px-2 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition disabled:opacity-50"
            >
              {sessions.length > 0 && selectedSessionIds.size === sessions.length
                ? t("buddy.sessions.selectAllClear")
                : t("buddy.sessions.selectAll")}
            </button>
            <button
              onClick={() =>
                setSessionDelete({
                  ids: [...selectedSessionIds],
                  label: t("buddy.sessions.deleteBatchLabel", { count: selectedSessionIds.size }),
                })
              }
              disabled={selectedSessionIds.size === 0 || sessionsBusy}
              className="px-2 py-1 rounded-md text-tiny bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 border border-rose-500/20 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
            >
              <Trash2 className="w-3 h-3" />
              {t("buddy.sessions.deleteSelected", { count: selectedSessionIds.size })}
            </button>
            <button
              onClick={loadSessions}
              disabled={sessionsBusy}
              className="px-2 py-1 rounded-md text-tiny bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-40"
            >
              {sessionsBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <RefreshCw className="w-3 h-3" />}
              {t("buddy.refresh")}
            </button>
          </div>
          <div className="flex-1 overflow-y-auto p-4">
            {sessions.length === 0 ? (
              <div className="h-full flex flex-col items-center justify-center text-slate-500 gap-2">
                <MessageSquareText className="w-10 h-10 text-slate-600" />
                <p className="text-body">{t("buddy.sessions.noSessions")}</p>
              </div>
            ) : (
              <div className="space-y-2">
                {/* 上次改目录留下的备份：搬完先别急着删，客户端里打开确认过再说 */}
                {lastMoveBackup ? (
                  <div className="flex items-center gap-2 rounded-card border border-amber-500/25 bg-amber-500/10 px-3 py-2">
                    <FolderOpen className="w-3.5 h-3.5 text-amber-300 flex-shrink-0" />
                    <span
                      className="text-tiny text-amber-200 truncate min-w-0 flex-1"
                      title={lastMoveBackup}
                    >
                      {t("buddy.sessions.moveBackupHint")}：{lastMoveBackup}
                    </span>
                    <button
                      onClick={() => void openPath(lastMoveBackup)}
                      className="px-1.5 py-0.5 rounded bg-white/5 hover:bg-white/10 text-amber-200 text-tiny cursor-pointer transition flex-shrink-0"
                    >
                      {t("buddy.sessions.moveOpenBackup")}
                    </button>
                    <button
                      onClick={() => setLastMoveBackup("")}
                      className="p-0.5 rounded hover:bg-white/10 text-amber-300/70 cursor-pointer transition flex-shrink-0"
                      title={t("common.dialogClose")}
                    >
                      <X className="w-3 h-3" />
                    </button>
                  </div>
                ) : null}
                {buildSessionGroups(sessions).map((group) => {
                  const isExpanded = expandedGroups.has(group.cwd);
                  return (
                    <div
                      key={group.cwd || "__empty__"}
                      className="rounded-card border border-white/10 bg-white/[0.03] overflow-hidden"
                    >
                      {/* 项目分组头 */}
                      <div
                        className="flex items-center gap-2 px-3 py-2 hover:bg-white/[0.04] cursor-pointer transition select-none"
                        onClick={() =>
                          setExpandedGroups((prev) => {
                            const next = new Set(prev);
                            if (next.has(group.cwd)) next.delete(group.cwd);
                            else next.add(group.cwd);
                            return next;
                          })
                        }
                      >
                        {isExpanded ? (
                          <ChevronDown className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
                        ) : (
                          <ChevronRight className="w-3.5 h-3.5 text-slate-500 flex-shrink-0" />
                        )}
                        <Folder className="w-3.5 h-3.5 text-[var(--module-accent)] flex-shrink-0" />
                        <span className="text-body font-semibold text-white truncate">
                          {resolveGroupLabel(group.cwd)}
                        </span>
                        <span className="inline-flex items-center text-micro px-1.5 py-0.5 rounded bg-white/5 text-slate-400 border border-white/10 flex-shrink-0">
                          {group.sessions.length}
                        </span>
                        {group.cwd && (
                          <span className="text-micro text-slate-600 truncate hidden sm:inline">
                            {formatCwd(group.cwd)}
                          </span>
                        )}
                        <span className="ml-auto text-micro text-slate-500 flex-shrink-0">
                          {formatRelative(group.latestUpdatedAt)}
                        </span>
                      </div>
                      {/* 分组内会话 */}
                      {isExpanded && (
                        <div className="border-t border-white/5 divide-y divide-white/5">
                          {group.sessions.map((s) => (
                            <div
                              key={s.conversationId}
                              className="px-3 py-2.5 hover:bg-white/[0.03] transition flex items-start gap-3"
                            >
                              <input
                                type="checkbox"
                                checked={selectedSessionIds.has(s.conversationId)}
                                onChange={() => toggleSessionSelect(s.conversationId)}
                                className="accent-[var(--module-accent)] w-3 h-3 cursor-pointer flex-shrink-0 mt-0.5"
                                title={t("buddy.sessions.selectOne")}
                              />
                              <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2">
                                  <span className="text-caption font-semibold text-white truncate">
                                    {s.title || "（无标题）"}
                                  </span>
                                  <span
                                    className={`inline-flex items-center text-[8px] px-1 py-0.5 rounded border flex-shrink-0 ${
                                      s.status === "Completed"
                                        ? "bg-emerald-500/15 text-emerald-300 border-emerald-500/25"
                                        : s.status === "InProgress"
                                          ? "bg-sky-500/15 text-sky-300 border-sky-500/25"
                                          : "bg-slate-500/15 text-slate-400 border-slate-500/25"
                                    }`}
                                  >
                                    {s.status}
                                  </span>
                                  {s.isPlayground && (
                                    <span className="inline-flex items-center text-[8px] px-1 py-0.5 rounded bg-[var(--module-accent-soft)] text-violet-300 border border-violet-500/25 flex-shrink-0">
                                      Playground
                                    </span>
                                  )}
                                </div>
                                <div className="text-tiny text-slate-500 mt-0.5 truncate" title={s.cwd}>
                                  <FolderOpen className="w-2.5 h-2.5 inline mr-1" />
                                  {formatCwd(s.cwd) || "—"}
                                </div>
                                <div className="text-micro text-slate-600 mt-0.5 flex items-center gap-2 flex-wrap">
                                  <span>ID: {s.conversationId.slice(0, 12)}…</span>
                                  <span>
                                    {t("buddy.sessions.updated")}: {formatRelative(sessionActivityAt(s))}
                                  </span>
                                  {s.contentMissing && (
                                    <span
                                      className="px-1 py-0.5 rounded bg-amber-500/10 border border-amber-500/30 text-amber-300"
                                      title={t("buddy.sessions.contentMissingHint")}
                                    >
                                      {t("buddy.sessions.contentMissing")}
                                    </span>
                                  )}
                                  {s.locations.length > 0 && (
                                    <span>{s.locations.map((l) => l.instanceName).join(", ")}</span>
                                  )}
                                </div>
                              </div>
                              <button
                                onClick={() => void copySessionId(s.conversationId)}
                                className="p-1.5 rounded-md bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 cursor-pointer transition flex-shrink-0"
                                title={t("buddy.sessions.copyId")}
                              >
                                {copiedId === s.conversationId ? (
                                  <Check className="w-3 h-3 text-emerald-400" />
                                ) : (
                                  <ClipboardCopy className="w-3 h-3" />
                                )}
                              </button>
                              <button
                                onClick={() => void forkSessionById(s.conversationId)}
                                disabled={sessionsBusy}
                                className="p-1.5 rounded-md bg-white/5 hover:bg-cyan-500/15 text-slate-300 hover:text-cyan-300 border border-white/10 cursor-pointer transition flex-shrink-0 disabled:opacity-40"
                                title={t("buddy.sessions.forkTitle")}
                              >
                                <GitFork className="w-3 h-3" />
                              </button>
                              <button
                                onClick={() => void moveSessionsToDir([s.conversationId])}
                                disabled={sessionsBusy}
                                className="p-1.5 rounded-md bg-white/5 hover:bg-violet-500/15 text-slate-300 hover:text-violet-300 border border-white/10 cursor-pointer transition flex-shrink-0 disabled:opacity-40"
                                title={t("buddy.sessions.moveTitle")}
                              >
                                <FolderInput className="w-3 h-3" />
                              </button>
                              <button
                                onClick={() => setSessionDelete({ ids: [s.conversationId], label: s.title || s.conversationId })}
                                className="p-1.5 rounded-md bg-white/5 hover:bg-rose-500/15 text-rose-300 border border-white/10 cursor-pointer transition flex-shrink-0"
                                title={t("buddy.sessions.deleteTitle")}
                              >
                                <Trash2 className="w-3 h-3" />
                              </button>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </>);

const renderSessionDeleteDialog = () => sessionDelete && (<div className="fixed inset-0 z-[130] modal-mask flex items-center justify-center bg-black/70 backdrop-blur-sm p-4">
          <div className="w-[360px] max-w-[95vw] rounded-panel border border-white/10 bg-slate-900/95 shadow-2xl p-5">
            <div className="flex items-center gap-2.5 mb-4">
              <div className="w-9 h-9 rounded-card bg-rose-500/15 border border-rose-500/30 flex items-center justify-center">
                <Trash2 className="w-4 h-4 text-rose-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-sm font-bold text-white">{t("buddy.sessions.deleteTitle")}</h3>
              </div>
              <button onClick={() => setSessionDelete(null)} className="p-1.5 rounded-ctl hover:bg-white/10 text-slate-400 cursor-pointer">
                ✕
              </button>
            </div>
            <p className="text-body text-slate-300 leading-relaxed mb-5 break-all">
              {sessionDelete.ids.length === 1
                ? t("buddy.sessions.deleteConfirm", { title: sessionDelete.label })
                : t("buddy.sessions.deleteConfirmBatch", { count: sessionDelete.ids.length })}
            </p>
            <div className="flex justify-end gap-2">
              <button onClick={() => setSessionDelete(null)} className="px-3 py-1.5 rounded-ctl text-caption text-slate-400 hover:bg-white/5 cursor-pointer">
                {t("buddy.cancel")}
              </button>
              <button
                onClick={() => void deleteSessionsByIds(sessionDelete.ids)}
                disabled={sessionsBusy}
                className="px-4 py-1.5 rounded-ctl text-caption bg-rose-600 hover:bg-rose-500 text-white font-semibold cursor-pointer disabled:opacity-50"
              >
                {t("buddy.deleteConfirm")}
              </button>
            </div>
          </div>
        </div>);
  return {
    setSessions,
    sessionDelete,
    renderSessionsWorkspace,
    renderSessionDeleteDialog,
  };
}
