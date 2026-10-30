import { useTranslation } from "react-i18next";
import { ListChecks, ChevronDown } from "lucide-react";
import type { CollabTask } from "../../contracts/ai";
import { TaskBadge } from "./CollabRoom.TaskBadge";
import type { Dispatch, SetStateAction } from "react";

interface CollabRoomTasksProps {
  setTasksOpen: Dispatch<SetStateAction<boolean>>;
  tasks: CollabTask[];
  tasksOpen: boolean;
  taskAction: (op: string, taskId: string) => void;
  newTaskTitle: string;
  setNewTaskTitle: Dispatch<SetStateAction<string>>;
  createTask: () => void;
}

/** Pure CollabRoomTasks view; its owning panel retains side effects and state. */
export function CollabRoomTasks({ setTasksOpen, tasks, tasksOpen, taskAction, newTaskTitle, setNewTaskTitle, createTask }: CollabRoomTasksProps) {
  const { t } = useTranslation();
  return (
    <div className="flex-shrink-0 border-t border-white/5 bg-slate-900/30">
      <button
        onClick={() => setTasksOpen((o) => !o)}
        className="w-full flex items-center justify-between px-3 py-1.5 text-micro text-slate-400 hover:text-slate-200 cursor-pointer transition-all"
      >
        <span className="flex items-center gap-1.5 font-semibold">
          <ListChecks className="w-3 h-3" /> {t("collab.tasksTitle", { count: tasks.length })}
        </span>
        <ChevronDown className={`w-3 h-3 transition-transform ${tasksOpen ? "rotate-180" : ""}`} />
      </button>
      {tasksOpen && (
        <div className="px-3 pb-3 space-y-2">
          {tasks.length === 0 && (
            <p className="text-tiny text-slate-500">{t("collab.noTasks")}</p>
          )}
          {tasks.map((task) => (
            <div key={task.id} className="rounded border border-white/5 bg-slate-800/40 p-2 space-y-1">
              <div className="flex items-center justify-between gap-2">
                <span className="text-caption text-slate-200 font-medium truncate">{task.title}</span>
                <TaskBadge status={task.status} />
              </div>
              {task.description && <p className="text-tiny text-slate-400 line-clamp-2">{task.description}</p>}
              <div className="flex items-center justify-between text-micro text-slate-500">
                <span>{task.assignee ? t("collab.assignedTo", { name: task.assignee }) : t("collab.unassigned")}</span>
                <div className="flex gap-1">
                  {task.status !== "done" && (
                    <button
                      onClick={() => taskAction("claim", task.id)}
                      className="px-1.5 py-0.5 rounded bg-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] text-white hover:bg-[var(--module-accent-strong)] cursor-pointer"
                    >
                      {t("collab.claim")}
                    </button>
                  )}
                  {task.status !== "done" && (
                    <button
                      onClick={() => taskAction("complete", task.id)}
                      className="px-1.5 py-0.5 rounded bg-emerald-600/80 text-white hover:bg-emerald-500 cursor-pointer"
                    >
                      {t("collab.complete")}
                    </button>
                  )}
                </div>
              </div>
            </div>
          ))}
          <div className="flex gap-1">
            <input
              value={newTaskTitle}
              onChange={(e) => setNewTaskTitle(e.target.value)}
              placeholder={t("collab.taskTitlePh")}
              className="flex-1 bg-slate-800 border border-white/10 rounded px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]"
            />
            <button
              onClick={createTask}
              className="px-2 py-1 rounded bg-[var(--module-accent)] text-white text-tiny hover:bg-[var(--module-accent-strong)] cursor-pointer"
            >
              {t("collab.create")}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
