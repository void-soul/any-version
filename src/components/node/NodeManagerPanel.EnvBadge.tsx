import { CheckCircle2, XCircle } from "lucide-react";
import { type DepCheck } from "./NodeManagerPanel.model";
export function EnvBadge({ dep, label }: { dep?: DepCheck; label: string }) {
  const ok = dep?.exists && dep.satisfies;
  return (
    <span
      className={`inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md border font-mono ${
        ok
          ? "bg-emerald-500/10 border-emerald-500/20 text-emerald-400"
          : "bg-red-500/10 border-red-500/20 text-red-400"
      }`}
    >
      {ok ? (
        <CheckCircle2 className="w-3 h-3" />
      ) : (
        <XCircle className="w-3 h-3" />
      )}
      <span>{label}</span>
      {dep?.version && <span className="text-slate-500">({dep.version})</span>}
    </span>
  );
}
