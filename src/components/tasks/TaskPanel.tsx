import React, { useState, useEffect } from "react";
import { CalendarCheck, CalendarDays, ListTodo, BarChart3, GanttChartSquare } from "lucide-react";
import TaskToday from "./TaskToday";
import TaskCalendar from "./TaskCalendar";
import TaskAll from "./TaskAll";
import TaskReview from "./TaskReview";
import TaskTimeline from "./TaskTimeline";
import { tasksApi } from "./types";

type TaskSubTab = "today" | "calendar" | "all" | "review" | "timeline";

const TABS = [
  { key: "today" as TaskSubTab, label: "今日", icon: CalendarCheck },
  { key: "calendar" as TaskSubTab, label: "日历", icon: CalendarDays },
  { key: "all" as TaskSubTab, label: "全部", icon: ListTodo },
  { key: "review" as TaskSubTab, label: "复盘", icon: BarChart3 },
  { key: "timeline" as TaskSubTab, label: "时间轴", icon: GanttChartSquare },
];

export default function TaskPanel() {
  const [activeTab, setActiveTab] = useState<TaskSubTab>("today");
  const [ready, setReady] = useState(false);

  // 懒挂载：仅渲染至少被访问过一次的 tab
  const [mountedTabs, setMountedTabs] = useState<Set<TaskSubTab>>(new Set(["today"]));
  const switchTab = (tab: TaskSubTab) => {
    setActiveTab(tab);
    setMountedTabs((prev) => {
      if (prev.has(tab)) return prev;
      const next = new Set(prev);
      next.add(tab);
      return next;
    });
  };

  // 首次进入时初始化 sqlite（建库建表，幂等）
  useEffect(() => {
    tasksApi
      .init()
      .catch((e) => console.error("任务数据库初始化失败:", e))
      .finally(() => setReady(true));
  }, []);

  if (!ready) {
    return (
      <div className="h-full flex items-center justify-center text-slate-500 text-xs">
        正在初始化任务数据库…
      </div>
    );
  }

  return (
    <div className="h-full flex min-h-0 select-none">
      {/* 左侧竖向菜单 */}
      <div className="w-25 flex-shrink-0 border-r border-white/5 py-3 px-2 space-y-0.5 overflow-y-auto">
        {TABS.map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            onClick={() => switchTab(key)}
            className={`w-full px-3 py-2 rounded-lg text-[11px] font-semibold flex items-center gap-2 transition-all cursor-pointer text-left ${
              activeTab === key
                ? "bg-[var(--module-accent)] text-white shadow-md shadow-[var(--module-accent-ring)]"
                : "text-slate-400 hover:text-slate-200 hover:bg-white/5"
            }`}
          >
            <Icon className="w-3.5 h-3.5 flex-shrink-0" />
            {label}
          </button>
        ))}
      </div>

      {/* 右侧内容区域 */}
      <div className="flex-1 min-h-0 overflow-hidden">
        {mountedTabs.has("today") && (
          <div className={activeTab === "today" ? "h-full" : "hidden"}>
            <TaskToday />
          </div>
        )}
        {mountedTabs.has("calendar") && (
          <div className={activeTab === "calendar" ? "h-full" : "hidden"}>
            <TaskCalendar />
          </div>
        )}
        {mountedTabs.has("all") && (
          <div className={activeTab === "all" ? "h-full" : "hidden"}>
            <TaskAll />
          </div>
        )}
        {mountedTabs.has("review") && (
          <div className={activeTab === "review" ? "h-full" : "hidden"}>
            <TaskReview />
          </div>
        )}
        {mountedTabs.has("timeline") && (
          <div className={activeTab === "timeline" ? "h-full" : "hidden"}>
            <TaskTimeline />
          </div>
        )}
      </div>
    </div>
  );
}
