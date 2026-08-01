import React, { useEffect, useState } from "react";
import { Bell, X, AlertTriangle, ListChecks, CheckCircle2, ChevronRight } from "lucide-react";
import { ReminderData, TaskBrief, tasksApi, PRIORITY_META, humanDate } from "./types";

interface Props {
  onClose: () => void;
  onOpenTasks: () => void;
}

const PRIORITY_DOT: Record<string, string> = {
  urgent: "bg-red-400",
  high: "bg-orange-400",
  medium: "bg-amber-400",
  low: "bg-slate-400",
};

function Row({ t, overdue }: { t: TaskBrief; overdue?: boolean }) {
  const dot = PRIORITY_DOT[t.priority] ?? PRIORITY_DOT.medium;
  return (
    <div className="flex items-center gap-2 px-3 py-1.5 rounded-lg hover:bg-white/5 transition-all">
      <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${dot}`} />
      <span className="flex-1 min-w-0 text-[11px] text-slate-200 truncate">
        {t.title}
      </span>
      {overdue && t.scheduledDate && (
        <span className="text-[9px] text-red-300/80 font-mono flex-shrink-0">
          {humanDate(t.scheduledDate)}
        </span>
      )}
      {t.progress > 0 && (
        <span className="text-[9px] font-mono text-slate-500 flex-shrink-0">
          {t.progress}%
        </span>
      )}
    </div>
  );
}

export default function TaskReminderToast({ onClose, onOpenTasks }: Props) {
  const [data, setData] = useState<ReminderData | null>(null);
  const [loading, setLoading] = useState(true);
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    let alive = true;
    tasksApi
      .todayReminder()
      .then((d) => alive && setData(d))
      .catch(() => {})
      .finally(() => alive && setLoading(false));
    const t = setTimeout(() => handleClose(), 15000); // 15s 自动消失
    return () => {
      alive = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleClose = () => {
    setLeaving(true);
    setTimeout(onClose, 200);
  };

  if (loading) return null;
  const overdue = data?.overdue ?? [];
  const todayPending = data?.todayPending ?? [];
  const total = overdue.length + todayPending.length;
  if (total === 0) {
    // 无待办也简短提示一下"今日清空"，然后自动消失
    return (
      <ToastShell leaving={leaving} onClose={handleClose}>
        <div className="flex items-center gap-2 px-4 py-3">
          <CheckCircle2 className="w-4 h-4 text-emerald-400 flex-shrink-0" />
          <span className="text-[12px] text-slate-200">今日暂无待办，干得漂亮 🎉</span>
          <button onClick={handleClose} className="ml-2 text-slate-500 hover:text-white">
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      </ToastShell>
    );
  }

  const showOverdue = overdue.slice(0, 5);
  const showToday = todayPending.slice(0, 8);

  return (
    <ToastShell leaving={leaving} onClose={handleClose}>
      <div className="w-80">
        {/* 头部 */}
        <div className="flex items-center gap-2 px-4 py-3 border-b border-white/10">
          <Bell className="w-4 h-4 text-amber-400 flex-shrink-0" />
          <div className="flex-1 min-w-0">
            <p className="text-[12px] font-bold text-white leading-tight">今日待办</p>
            <p className="text-[9px] text-slate-500">
              共 {total} 项 · 今日 {todayPending.length} · 逾期 {overdue.length}
            </p>
          </div>
          <button
            onClick={handleClose}
            className="text-slate-500 hover:text-white transition-all flex-shrink-0"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        {/* 内容区（可滚动） */}
        <div className="max-h-72 overflow-y-auto py-2 space-y-2">
          {overdue.length > 0 && (
            <div>
              <div className="flex items-center gap-1 px-3 mb-1">
                <AlertTriangle className="w-3 h-3 text-red-400" />
                <span className="text-[9px] font-bold text-red-300 uppercase tracking-wider">
                  逾期 {overdue.length}
                </span>
              </div>
              {showOverdue.map((t) => (
                <Row key={t.id} t={t} overdue />
              ))}
            </div>
          )}
          {todayPending.length > 0 && (
            <div>
              <div className="flex items-center gap-1 px-3 mb-1">
                <ListChecks className="w-3 h-3 text-amber-400" />
                <span className="text-[9px] font-bold text-amber-300 uppercase tracking-wider">
                  今日未完 {todayPending.length}
                </span>
              </div>
              {showToday.map((t) => (
                <Row key={t.id} t={t} />
              ))}
            </div>
          )}
        </div>

        {/* 底部操作 */}
        <div className="px-4 py-2.5 border-t border-white/10 flex items-center justify-between">
          <span className="text-[9px] text-slate-600">启动后自动提醒</span>
          <button
            onClick={onOpenTasks}
            className="flex items-center gap-1 text-[10px] font-semibold text-amber-300 hover:text-amber-200 transition-all"
          >
            去处理 <ChevronRight className="w-3 h-3" />
          </button>
        </div>
      </div>
    </ToastShell>
  );
}

function ToastShell({
  children,
  leaving,
  onClose,
}: {
  children: React.ReactNode;
  leaving: boolean;
  onClose: () => void;
}) {
  return (
    <div
      className={`fixed bottom-4 right-4 z-[9999] rounded-2xl border border-white/10 shadow-2xl shadow-black/50 bg-slate-900/95 backdrop-blur-md text-slate-100 overflow-hidden transition-all duration-200 ${
        leaving ? "opacity-0 translate-y-3" : "opacity-100 translate-y-0"
      }`}
    >
      {children}
    </div>
  );
}
