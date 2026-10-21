// 任务的复盘记录区：日志（每日推进/投入）+ 顺延记录（为什么一再改期）。
//
// 挂在编辑弹窗里、只读浏览态也可用——记日志不算「改任务」，不该要求先点编辑。
// 默认收起，首次展开才拉数据，避免打开弹窗就发两个请求。

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, History, Loader2, NotebookPen } from "lucide-react";
import { SharedButton, inputCls } from "../shared/Button";
import {
  formatMinutes, tasksApi, todayStr,
  type TaskLog, type TaskMoveRecord,
} from "./types";

export function TaskLogsSection({ taskId }: { taskId: string }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [logs, setLogs] = useState<TaskLog[]>([]);
  const [moves, setMoves] = useState<TaskMoveRecord[]>([]);
  const [logContent, setLogContent] = useState("");
  const [logDate, setLogDate] = useState(todayStr());
  const [logMinutes, setLogMinutes] = useState(0);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");

  const toggle = async () => {
    const next = !expanded;
    setExpanded(next);
    if (!next || loaded) return;
    setBusy(true);
    try {
      const [logList, moveList] = await Promise.all([
        tasksApi.listLogs(taskId),
        tasksApi.listMoves(taskId),
      ]);
      setLogs(logList);
      setMoves(moveList);
      setLoaded(true);
      setErr("");
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const addLog = async () => {
    const content = logContent.trim();
    if (!content) return;
    setBusy(true);
    setErr("");
    try {
      const created = await tasksApi.addLog(taskId, logDate || todayStr(), content, logMinutes, []);
      setLogs((prev) => [created, ...prev]);
      setLogContent("");
      setLogMinutes(0);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="rounded-ctl border border-white/10">
      <button
        type="button"
        onClick={() => void toggle()}
        className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-tiny font-semibold text-slate-300 transition hover:bg-white/5"
      >
        {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        <NotebookPen className="h-3 w-3 text-[var(--module-accent)]" />
        {t("taskPlan.logsTitle")}
        {loaded && logs.length > 0 && (
          <span className="rounded-full bg-white/10 px-1.5 text-micro text-slate-400">{logs.length}</span>
        )}
        {busy && <Loader2 className="ml-auto h-3 w-3 animate-spin text-slate-500" />}
      </button>

      {expanded && (
        <div className="flex flex-col gap-2 border-t border-white/10 px-2.5 py-2">
          {err && <div className="text-tiny text-rose-300">{err}</div>}

          {/* 添加日志：日期默认今天；分钟=当日实际投入，会进复盘统计 */}
          <textarea
            className={`${inputCls} !h-14 resize-none py-1.5`}
            value={logContent}
            placeholder={t("taskPlan.logsContentPlaceholder")}
            onChange={(e) => setLogContent(e.target.value)}
          />
          <div className="flex items-center gap-2">
            <input
              type="date"
              className={`${inputCls} !w-auto`}
              value={logDate}
              title={t("taskPlan.logsDateLabel")}
              onChange={(e) => setLogDate(e.target.value)}
            />
            <input
              type="number"
              min={0}
              step={15}
              className={`${inputCls} !w-20`}
              value={logMinutes || ""}
              placeholder="0"
              title={t("taskPlan.logsMinutesLabel")}
              onChange={(e) => setLogMinutes(Math.max(0, Number(e.target.value) || 0))}
            />
            <SharedButton
              variant="primary"
              className="ml-auto !h-7 !px-2 !text-tiny"
              disabled={busy || !logContent.trim()}
              title={!logContent.trim() ? t("taskPlan.logsContentRequired") : undefined}
              onClick={() => void addLog()}
            >
              {t("taskPlan.logsAdd")}
            </SharedButton>
          </div>

          {loaded && logs.length === 0 && (
            <div className="text-tiny text-slate-600">{t("taskPlan.logsEmpty")}</div>
          )}
          {logs.map((log) => (
            <div key={log.id} className="rounded-md bg-white/[0.03] px-2 py-1.5">
              <div className="flex items-center gap-2 text-micro text-slate-500">
                <span>{log.logDate}</span>
                {log.minutesSpent > 0 && (
                  <span className="rounded-full bg-[var(--module-accent-soft)] px-1.5 text-[var(--module-accent)]">
                    {formatMinutes(log.minutesSpent)}
                  </span>
                )}
              </div>
              <div className="mt-0.5 whitespace-pre-wrap break-words text-caption text-slate-300">{log.content}</div>
            </div>
          ))}

          {/* 顺延记录：回答「这个任务为什么一再改期」 */}
          {loaded && (
            <div className="mt-1">
              <div className="flex items-center gap-1 text-tiny font-semibold text-slate-400">
                <History className="h-3 w-3" />
                {t("taskPlan.movesTitle")}
                {moves.length > 0 && (
                  <span className="rounded-full bg-white/10 px-1.5 text-micro text-slate-500">{moves.length}</span>
                )}
              </div>
              {moves.length === 0 ? (
                <div className="mt-0.5 text-tiny text-slate-600">{t("taskPlan.movesEmpty")}</div>
              ) : (
                <div className="mt-1 flex flex-col gap-0.5">
                  {moves.map((mv) => (
                    <div key={mv.id} className="text-micro text-slate-500">
                      {(mv.fromDate ?? t("taskPlan.unscheduled")).slice(5)}
                      {" → "}
                      {(mv.toDate ?? t("taskPlan.unscheduled")).slice(5)}
                      {mv.reason && <span className="text-slate-600"> · {mv.reason}</span>}
                      <span className="text-slate-700"> · {mv.movedAt.slice(0, 16).replace("T", " ")}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
