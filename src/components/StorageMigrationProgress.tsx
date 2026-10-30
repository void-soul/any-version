import { Loader2, FileText } from "lucide-react";
import { type MigrateProgress } from "./GlobalSettings.model";

interface StorageMigrationProgressProps {
  progress: MigrateProgress;
}

/** Pure StorageMigrationProgress view; its owning panel retains side effects and state. */
export function StorageMigrationProgress({ progress }: StorageMigrationProgressProps) {
  return (
    <div className="p-3 bg-[var(--module-accent-soft)] border border-[var(--module-accent-ring)] rounded-card space-y-2 animate-fadeIn">
      <div className="flex items-center justify-between text-tiny">
        <span className="text-[var(--module-accent)] font-semibold flex items-center gap-1.5">
          <Loader2 className="w-3 h-3 animate-spin" />
          {progress.stage}
        </span>
        {progress.total > 0 && (
          <span className="text-[var(--module-accent)] font-mono">
            {progress.current}/{progress.total}
          </span>
        )}
      </div>
      {progress.total > 0 && (
        <div className="w-full bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] rounded-full h-1.5 overflow-hidden">
          <div
            className="bg-[var(--module-accent)] h-full rounded-full transition-all duration-300"
            style={{
              width: `${Math.round((progress.current / progress.total) * 100)}%`,
            }}
          />
        </div>
      )}
      {progress.file_name && (
        <div className="flex items-center gap-1 text-micro text-slate-400">
          <FileText className="w-2.5 h-2.5 flex-shrink-0" />
          <span className="truncate">{progress.file_name}</span>
        </div>
      )}
    </div>
  );
}
