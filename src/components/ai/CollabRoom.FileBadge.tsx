import { Trash2 } from "lucide-react";
export function FileBadge({
  path,
  onRemove,
  color = "slate",
}: {
  path: string;
  onRemove?: () => void;
  color?: "slate" | "cyan";
}) {
  const colorClass = color === "cyan" ? "text-cyan-300" : "text-slate-300";
  return (
    <div
      className={`flex items-center gap-1 px-1.5 py-0.5 rounded bg-slate-800/60 border border-white/10 text-micro ${colorClass} max-w-[200px]`}
      title={path}
    >
      <span className="truncate">@ {path.split(/[\\/]/).pop()}</span>
      {onRemove && (
        <button
          onClick={onRemove}
          className="text-slate-500 hover:text-red-300 cursor-pointer flex-shrink-0"
        >
          <Trash2 className="w-2.5 h-2.5" />
        </button>
      )}
    </div>
  );
}
