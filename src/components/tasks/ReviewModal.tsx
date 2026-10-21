// 复盘统计弹窗：当前可见范围（月网格或周）的聚合视图 —— 每天完成度、实际投入 vs 预估。
//
// 数据：dayStats 给计数与投入分钟；预估分钟与每日任务清单从面板已加载的 items 现算
//（dayStats 不返回 estimate，避免为此改后端）。

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import { SharedModal } from "../shared/Modal";
import { SharedButton } from "../shared/Button";
import { PRIORITY_META, formatMinutes, tasksApi, todayStr, type DayStat, type TaskItem } from "./types";

export function ReviewModal({
  open, onClose, range, items,
}: {
  open: boolean;
  onClose: () => void;
  range: { start: string; end: string };
  items: TaskItem[];
}) {
  const { t } = useTranslation();
  const today = todayStr();
  const [stats, setStats] = useState<DayStat[]>([]);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState("");
  const [expandedDay, setExpandedDay] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setLoading(true);
    tasksApi
      .dayStats(range.start, range.end)
      .then((list) => {
        if (cancelled) return;
        setStats(list);
        setErr("");
      })
      .catch((e) => {
        if (!cancelled) setErr(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, range.start, range.end]);

  /** 每天的预估分钟合计（来自 items） */
  const estimateByDay = useMemo(() => {
    const map = new Map<string, number>();
    for (const task of items) {
      if (!task.scheduledDate) continue;
      map.set(task.scheduledDate, (map.get(task.scheduledDate) ?? 0) + task.estimateMinutes);
    }
    return map;
  }, [items]);

  const tasksByDay = useMemo(() => {
    const map = new Map<string, TaskItem[]>();
    for (const task of items) {
      if (!task.scheduledDate) continue;
      const list = map.get(task.scheduledDate);
      if (list) list.push(task);
      else map.set(task.scheduledDate, [task]);
    }
    return map;
  }, [items]);

  const totals = useMemo(() => {
    const sum = (pick: (s: DayStat) => number) => stats.reduce((acc, s) => acc + pick(s), 0);
    const total = sum((s) => s.total);
    const completed = sum((s) => s.completed);
    const spent = sum((s) => s.minutesSpent);
    const estimate = items.reduce((acc, task) => acc + task.estimateMinutes, 0);
    return { total, completed, spent, estimate, rate: total > 0 ? Math.round((completed / total) * 100) : 0 };
  }, [stats, items]);

  const cards: Array<{ label: string; value: string }> = [
    { label: t("taskPlan.reviewTotalTasks"), value: String(totals.total) },
    { label: t("taskPlan.reviewCompleted"), value: String(totals.completed) },
    { label: t("taskPlan.reviewRate"), value: `${totals.rate}%` },
    { label: t("taskPlan.reviewEstimate"), value: formatMinutes(totals.estimate) },
    { label: t("taskPlan.reviewSpent"), value: formatMinutes(totals.spent) },
  ];

  return (
    <SharedModal
      open={open}
      onClose={onClose}
      width={640}
      title={t("taskPlan.reviewTitle")}
      headerActions={
        <span className="text-tiny text-slate-500">{range.start} ~ {range.end}</span>
      }
      footer={<SharedButton variant="secondary" onClick={onClose}>{t("taskPlan.close")}</SharedButton>}
    >
      {err && <div className="text-tiny text-rose-300">{err}</div>}

      <div className="grid grid-cols-5 gap-2">
        {cards.map((card) => (
          <div key={card.label} className="rounded-ctl border border-white/10 bg-white/[0.03] px-2 py-1.5">
            <div className="text-micro text-slate-500">{card.label}</div>
            <div className="mt-0.5 text-body font-bold text-white">{card.value}</div>
          </div>
        ))}
      </div>

      {loading && <Loader2 className="h-4 w-4 animate-spin text-slate-500" />}
      {!loading && !err && stats.length === 0 && (
        <div className="py-4 text-center text-caption text-slate-600">{t("taskPlan.reviewEmpty")}</div>
      )}

      <div className="flex min-h-0 flex-col gap-0.5 overflow-y-auto">
        {stats.map((s) => {
          const expanded = expandedDay === s.date;
          const dayTasks = tasksByDay.get(s.date) ?? [];
          const estimate = estimateByDay.get(s.date) ?? 0;
          return (
            <div key={s.date} className="rounded-ctl border border-white/5">
              <button
                type="button"
                onClick={() => setExpandedDay(expanded ? null : s.date)}
                className={`flex w-full items-center gap-2 px-2 py-1 text-left transition hover:bg-white/5 ${
                  s.date === today ? "bg-[var(--module-accent-soft)]" : ""
                }`}
              >
                {expanded
                  ? <ChevronDown className="h-3 w-3 shrink-0 text-slate-500" />
                  : <ChevronRight className="h-3 w-3 shrink-0 text-slate-500" />}
                <span className={`w-12 shrink-0 text-tiny font-semibold ${
                  s.date === today ? "text-[var(--module-accent)]" : "text-slate-300"
                }`}>
                  {s.date.slice(5)}
                </span>
                {/* 三段式完成度条：完成 / 进行中 / 待开始 */}
                <div className="flex h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-white/10">
                  {s.completed > 0 && <div className="bg-emerald-500" style={{ flex: s.completed }} />}
                  {s.inProgress > 0 && <div className="bg-amber-400" style={{ flex: s.inProgress }} />}
                  {s.pending > 0 && <div className="bg-slate-600" style={{ flex: s.pending }} />}
                </div>
                <span className="shrink-0 text-micro text-slate-400">{s.completed}/{s.total}</span>
                <span className="w-14 shrink-0 text-right text-micro text-slate-500" title={t("taskPlan.reviewSpent")}>
                  {formatMinutes(s.minutesSpent)}
                </span>
                <span className="w-14 shrink-0 text-right text-micro text-slate-600" title={t("taskPlan.reviewEstimate")}>
                  {estimate > 0 ? formatMinutes(estimate) : "—"}
                </span>
              </button>
              {expanded && (
                <div className="flex flex-col gap-0.5 border-t border-white/5 px-3 py-1.5">
                  {dayTasks.map((task) => (
                    <div key={task.id} className="flex items-center gap-1.5 text-tiny">
                      <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${PRIORITY_META[task.priority]?.dot ?? "bg-slate-500"}`} />
                      <span className={`min-w-0 flex-1 truncate ${
                        task.progress >= 100 ? "text-slate-500 line-through" : "text-slate-300"
                      }`}>
                        {task.title || t("taskPlan.untitled")}
                      </span>
                      <span className="shrink-0 text-micro text-slate-500">{task.progress}%</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </SharedModal>
  );
}
