// 搜索结果区：有查询词时替换日历网格（表头保留，搜索框在表头）。
//
// 「含已归档」开关是归档任务的唯一找回入口 —— 归档任务从日历与收集箱消失后，
// 在这里勾上开关才能看到（灰显 + 徽标），点开即可取消归档。

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { PRIORITY_META, tasksApi, type TaskItem } from "./types";

export function TaskSearchPanel({
  query, onOpenTask, refreshNonce,
}: {
  query: string;
  onOpenTask: (task: TaskItem) => void;
  /** 编辑弹窗保存后递增，触发重搜（归档状态等可能已变） */
  refreshNonce: number;
}) {
  const { t } = useTranslation();
  const [results, setResults] = useState<TaskItem[]>([]);
  const [searching, setSearching] = useState(false);
  const [includeArchived, setIncludeArchived] = useState(false);
  const [err, setErr] = useState("");

  useEffect(() => {
    let cancelled = false;
    setSearching(true);
    tasksApi
      .search(query, includeArchived)
      .then((list) => {
        if (cancelled) return;
        setResults(list);
        setErr("");
      })
      .catch((e) => {
        if (cancelled) return;
        setErr(String(e));
      })
      .finally(() => {
        if (!cancelled) setSearching(false);
      });
    return () => {
      cancelled = true;
    };
  }, [query, includeArchived, refreshNonce]);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-card border border-white/10">
      <div className="flex flex-shrink-0 items-center gap-2 border-b border-white/10 bg-white/[0.02] px-3 py-1.5">
        <span className="text-tiny font-semibold text-slate-300">
          {t("taskPlan.searchResults", { count: results.length })}
        </span>
        {searching && <Loader2 className="h-3 w-3 animate-spin text-slate-500" />}
        <label className="ml-auto flex cursor-pointer items-center gap-1 text-tiny text-slate-400">
          <input
            type="checkbox"
            checked={includeArchived}
            onChange={(e) => setIncludeArchived(e.target.checked)}
            className="accent-[var(--module-accent)]"
          />
          {t("taskPlan.searchIncludeArchived")}
        </label>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto p-2">
        {err && <div className="px-1 py-0.5 text-tiny text-rose-300">{err}</div>}
        {!searching && !err && results.length === 0 && (
          <div className="px-1 py-2 text-tiny text-slate-600">{t("taskPlan.searchEmpty")}</div>
        )}
        {results.length >= 300 && (
          <div className="px-1 py-0.5 text-tiny text-slate-500">{t("taskPlan.searchLimitHint")}</div>
        )}
        {results.map((task) => (
          <button
            key={task.id}
            type="button"
            onClick={() => onOpenTask(task)}
            className={`flex items-center gap-2 rounded-md border border-white/5 bg-white/[0.03] px-2 py-1 text-left transition hover:bg-white/[0.08] ${
              task.archived ? "opacity-60" : ""
            }`}
          >
            <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${PRIORITY_META[task.priority]?.dot ?? "bg-slate-500"}`} />
            <span className={`min-w-0 flex-1 truncate text-caption ${
              task.progress >= 100 ? "text-slate-500 line-through" : "text-slate-200"
            }`}>
              {task.title || t("taskPlan.untitled")}
            </span>
            {task.archived && (
              <span className="shrink-0 rounded-ctl border border-amber-400/30 bg-amber-400/10 px-1.5 text-micro text-amber-200">
                {t("taskPlan.archivedBadge")}
              </span>
            )}
            <span className="shrink-0 text-micro text-slate-500">
              {task.scheduledDate ?? t("taskPlan.unscheduled")}
            </span>
            <span className="w-9 shrink-0 text-right text-micro text-slate-500">{task.progress}%</span>
          </button>
        ))}
      </div>
    </div>
  );
}
