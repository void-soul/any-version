// 任务编辑弹窗（新建与编辑共用）。从 TaskCalendarPanel 抽出：它完全自包含，
// 面板只负责开关与保存后刷新。
//
// 打开**已有任务**先只读浏览，点「编辑」才进可编辑态；新建任务直接就是编辑态。
// 为什么：点开一个任务就想改内容，是最容易误改的路径（顺手把字删了、进度拖错了）。
// 默认浏览把「看」和「改」分开，改动变得有明确意图。

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle, Archive, ArchiveRestore, Clock, Pencil, Trash2,
} from "lucide-react";
import { SharedButton, inputCls, labelCls } from "../shared/Button";
import { SharedModal } from "../shared/Modal";
import { MarkdownFieldEditor } from "../shared/markdown/MarkdownFieldEditor";
import { isOverdue } from "./calendarRange";
import { TaskLogsSection } from "./TaskLogsSection";
import {
  PRIORITY_META, addDays, deriveStatus, formatMinutes, progressColor, tasksApi, todayStr,
  type TaskItem, type TaskPriority,
} from "./types";

const PRIORITY_ORDER: TaskPriority[] = ["low", "medium", "high", "urgent"];
const PRIORITY_LABEL: Record<TaskPriority, string> = {
  low: "taskPlan.priorityLow",
  medium: "taskPlan.priorityMedium",
  high: "taskPlan.priorityHigh",
  urgent: "taskPlan.priorityUrgent",
};

export function TaskEditorModal({
  task, day, onClose, onSaved, onRequestDelete,
}: {
  task: TaskItem | null;
  day: string;
  onClose: () => void;
  onSaved: () => void;
  onRequestDelete: (task: TaskItem) => void;
}) {
  const { t } = useTranslation();
  const today = todayStr();
  const [title, setTitle] = useState(task?.title ?? "");
  const [date, setDate] = useState(task?.scheduledDate ?? day);
  const [priority, setPriority] = useState<TaskPriority>((task?.priority as TaskPriority) ?? "medium");
  const [progress, setProgress] = useState(task?.progress ?? 0);
  const [tags, setTags] = useState(task?.tags ?? "");
  const [description, setDescription] = useState(task?.description ?? "");
  const [detail, setDetail] = useState(task?.detail ?? "");
  const [estimate, setEstimate] = useState(task?.estimateMinutes ?? 0);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [editing, setEditing] = useState(!task);
  /** 只读浏览态 */
  const ro = !!task && !editing;

  const late = !!task && isOverdue({ progress, scheduledDate: date || null }, today);

  /** 选一个本地文件插入详细说明（绝对路径：任务不属于任何项目目录，
   *  相对路径没有可解析的根，所以一律存绝对路径）。 */
  const pickTaskFile = async () => {
    const sel = await open({ multiple: false, title: t("taskPlan.pickFileTitle") });
    if (!sel) return null;
    return typeof sel === "string" ? sel : null;
  };

  const save = async () => {
    const trimmed = title.trim();
    if (!trimmed) {
      setErr(t("taskPlan.titleRequired"));
      return;
    }
    setBusy(true);
    setErr("");
    try {
      if (task) {
        await tasksApi.update(task.id, {
          title: trimmed,
          scheduledDate: date || null,
          priority,
          tags,
          description,
          detail,
          estimateMinutes: estimate,
        });
        // 进度走专用命令：它会记 move record（进度变化也值得留痕）
        if (progress !== task.progress) await tasksApi.setProgress(task.id, { progress });
      } else {
        await tasksApi.create({
          title: trimmed, scheduledDate: date || null, priority, tags, description, detail, progress,
          estimateMinutes: estimate,
        });
      }
      onSaved();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const carryToToday = async () => {
    if (!task) return;
    setBusy(true);
    setErr("");
    try {
      await tasksApi.move(task.id, { toDate: today, reason: t("taskPlan.carryReason") });
      onSaved();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  /** 归档是软删除、可逆，所以不加确认弹窗；恢复入口在搜索（含已归档）找回的同一个弹窗里 */
  const toggleArchive = async () => {
    if (!task) return;
    setBusy(true);
    setErr("");
    try {
      await tasksApi.setArchived(task.id, !task.archived);
      onSaved();
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  return (
    <SharedModal
      open
      onClose={onClose}
      width={520}
      title={task ? (ro ? t("taskPlan.viewTask") : t("taskPlan.editTask")) : t("taskPlan.newTask")}
      headerActions={task?.archived ? (
        <span className="rounded-ctl border border-amber-400/30 bg-amber-400/10 px-1.5 py-0.5 text-tiny text-amber-200">
          {t("taskPlan.archivedBadge")}
        </span>
      ) : undefined}
      footer={
        <>
          {/* 删除是破坏性动作，只在编辑态露出来：浏览态不该摆着一个红按钮；
              归档可逆，两种态都给（打开归档任务主要就是为了恢复） */}
          {task && !ro && (
            <SharedButton variant="ghost" className="!text-rose-300 hover:!bg-rose-500/10" disabled={busy}
              onClick={() => onRequestDelete(task)}>
              <Trash2 className="h-3 w-3" />
              {t("taskPlan.delete")}
            </SharedButton>
          )}
          {task && (
            <SharedButton
              variant="ghost"
              disabled={busy}
              title={t("taskPlan.archiveHint")}
              className={task.archived
                ? "!text-emerald-300 hover:!bg-emerald-500/10"
                : "!text-amber-300 hover:!bg-amber-500/10"}
              onClick={() => void toggleArchive()}
            >
              {task.archived ? <ArchiveRestore className="h-3 w-3" /> : <Archive className="h-3 w-3" />}
              {task.archived ? t("taskPlan.unarchive") : t("taskPlan.archive")}
            </SharedButton>
          )}
          <div className="ml-auto flex items-center gap-2">
            {ro ? (
              <>
                <SharedButton variant="secondary" onClick={onClose}>{t("taskPlan.close")}</SharedButton>
                <SharedButton variant="primary" onClick={() => setEditing(true)}>
                  <Pencil className="h-3 w-3" />
                  {t("taskPlan.edit")}
                </SharedButton>
              </>
            ) : (
              <>
                <SharedButton variant="secondary" onClick={onClose}>{t("taskPlan.cancel")}</SharedButton>
                <SharedButton variant="primary" disabled={busy} onClick={() => void save()}>{t("taskPlan.save")}</SharedButton>
              </>
            )}
          </div>
        </>
      }
    >
      {err && (
        <div className="flex items-start gap-2 rounded-ctl border border-rose-500/25 bg-rose-500/10 px-3 py-2 text-caption text-rose-300">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1 break-all">{err}</span>
        </div>
      )}

      <div>
        <label className={labelCls}>{t("taskPlan.titleLabel")}</label>
        {ro ? (
          <div className="text-body font-semibold text-slate-100 break-words">{title || "—"}</div>
        ) : (
          <input className={inputCls} autoFocus value={title}
            placeholder={t("taskPlan.titlePlaceholder")}
            onChange={(e) => setTitle(e.target.value)} />
        )}
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls}>{t("taskPlan.dateLabel")}</label>
          {ro ? (
            <div className="text-caption text-slate-300">{date || t("taskPlan.unscheduled")}</div>
          ) : (
            <>
              <input type="date" className={inputCls} value={date} onChange={(e) => setDate(e.target.value)} />
              <div className="mt-1 flex items-center gap-1">
                <button type="button" onClick={() => setDate(todayStr())}
                  className="rounded border border-white/10 px-1.5 py-0.5 text-tiny text-slate-400 hover:bg-white/10">
                  {t("taskPlan.setToday")}
                </button>
                <button type="button" onClick={() => setDate(addDays(todayStr(), 1))}
                  className="rounded border border-white/10 px-1.5 py-0.5 text-tiny text-slate-400 hover:bg-white/10">
                  {t("taskPlan.setTomorrow")}
                </button>
                <button type="button" onClick={() => setDate("")}
                  className="rounded border border-white/10 px-1.5 py-0.5 text-tiny text-slate-400 hover:bg-white/10">
                  {t("taskPlan.unschedule")}
                </button>
              </div>
              {!date && <div className="mt-1 text-tiny text-slate-500">{t("taskPlan.unscheduledHint")}</div>}
            </>
          )}
        </div>
        <div>
          <label className={labelCls}>{t("taskPlan.priorityLabel")}</label>
          {ro ? (
            <span className={`inline-flex items-center rounded-ctl border px-2 py-1 text-tiny font-semibold ${PRIORITY_META[priority].bg} ${PRIORITY_META[priority].text}`}>
              {t(PRIORITY_LABEL[priority])}
            </span>
          ) : (
            <div className="flex items-center gap-1">
              {PRIORITY_ORDER.map((p) => (
                <button key={p} type="button" onClick={() => setPriority(p)}
                  className={`flex-1 rounded-ctl border px-1.5 py-1 text-tiny transition ${
                    priority === p ? `${PRIORITY_META[p].bg} ${PRIORITY_META[p].text} font-semibold` : "border-white/10 text-slate-400 hover:bg-white/5"
                  }`}>
                  {t(PRIORITY_LABEL[p])}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      <div>
        <label className={labelCls}>{t("taskPlan.estimateLabel")}</label>
        {ro ? (
          <div className="text-caption text-slate-300">{estimate > 0 ? formatMinutes(estimate) : "—"}</div>
        ) : (
          <div className="flex items-center gap-2">
            <input
              type="number"
              min={0}
              step={15}
              className={`${inputCls} !w-28`}
              value={estimate || ""}
              placeholder="0"
              onChange={(e) => setEstimate(Math.max(0, Number(e.target.value) || 0))}
            />
            {estimate > 0 && <span className="text-tiny text-slate-500">= {formatMinutes(estimate)}</span>}
          </div>
        )}
      </div>

      <div>
        <label className={labelCls}>
          {t("taskPlan.progressLabel")} · {progress}%
          <span className="ml-2 font-normal text-slate-500">{t(`taskPlan.status${deriveStatus(progress) === "done" ? "Done" : deriveStatus(progress) === "inProgress" ? "InProgress" : "Todo"}`)}</span>
        </label>
        {!ro && (
          <div className="flex items-center gap-2">
            <input type="range" min={0} max={100} step={5} value={progress}
              onChange={(e) => setProgress(Number(e.target.value))}
              className="h-1.5 flex-1 cursor-pointer accent-[var(--module-accent)]" />
            <div className="flex items-center gap-1">
              {[0, 50, 100].map((p) => (
                <button key={p} type="button" onClick={() => setProgress(p)}
                  className={`rounded border px-1.5 py-0.5 text-tiny transition ${
                    progress === p ? "border-white/25 bg-white/10 text-white" : "border-white/10 text-slate-400 hover:bg-white/5"
                  }`}>
                  {p}%
                </button>
              ))}
            </div>
          </div>
        )}
        <div className="mt-1.5 h-1 w-full overflow-hidden rounded-full bg-white/10">
          <div className={`h-full ${progressColor(progress)}`} style={{ width: `${progress}%` }} />
        </div>
      </div>

      <div>
        <label className={labelCls}>{t("taskPlan.tagsLabel")}</label>
        {ro ? (
          <div className="text-caption text-slate-300 break-words">{tags || "—"}</div>
        ) : (
          <input className={inputCls} value={tags} placeholder={t("taskPlan.tagsPlaceholder")}
            onChange={(e) => setTags(e.target.value)} />
        )}
      </div>

      <div>
        <label className={labelCls}>{t("taskPlan.descriptionLabel")}</label>
        {ro ? (
          <div className="whitespace-pre-wrap break-words text-caption leading-relaxed text-slate-300">
            {description || "—"}
          </div>
        ) : (
          <textarea className={`${inputCls} !h-16 resize-none py-1.5`} value={description}
            onChange={(e) => setDescription(e.target.value)} />
        )}
      </div>

      {/* 详细说明：Markdown + 本地文件链接。链接点在预览里即可打开，
          与思维导图里的文件链接是同一套做法（复用 MindmapMarkdown 渲染）。 */}
      <div className="flex min-h-0 flex-col">
        <label className={labelCls}>{t("taskPlan.detailLabel")}</label>
        <MarkdownFieldEditor
          value={detail}
          onChange={setDetail}
          minHeight="140px"
          pickFile={pickTaskFile}
          readOnly={ro}
        />
      </div>

      {/* 复盘记录：记日志不算改任务，浏览态同样可记 */}
      {task && <TaskLogsSection taskId={task.id} />}

      {task && (
        <div className="flex flex-wrap items-center gap-2 border-t border-white/10 pt-2 text-tiny text-slate-500">
          <Clock className="h-3 w-3" />
          {t("taskPlan.metaLine", {
            created: task.createdAt.slice(0, 10),
            updated: task.updatedAt.slice(0, 10),
          })}
          {!ro && late && (
            <button type="button" disabled={busy} onClick={() => void carryToToday()}
              className="ml-auto rounded-md border border-amber-400/30 bg-amber-400/10 px-2 py-0.5 text-tiny text-amber-200 transition hover:bg-amber-400/20 disabled:opacity-50">
              {t("taskPlan.carryToToday")}
            </button>
          )}
        </div>
      )}
    </SharedModal>
  );
}
