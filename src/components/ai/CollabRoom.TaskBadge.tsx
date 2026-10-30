import { useTranslation } from "react-i18next";
// ─── 任务状态徽章（E 任务流） ───
export function TaskBadge({ status }: { status: string }) {
  const { t } = useTranslation();
  const map: Record<string, string> = {
    open: "bg-slate-600",
    claimed: "bg-amber-600",
    in_progress: "bg-blue-600",
    in_review: "bg-[var(--module-accent)]",
    done: "bg-emerald-600",
  };
  const label: Record<string, string> = {
    open: "collab.open",
    claimed: "collab.claimed",
    in_progress: "collab.inProgress",
    in_review: "collab.inReview",
    done: "collab.done",
  };
  return (
    <span className={`px-1.5 py-0.5 rounded text-micro text-white ${map[status] || "bg-slate-600"}`}>
      {label[status] ? t(label[status]) : status}
    </span>
  );
}
