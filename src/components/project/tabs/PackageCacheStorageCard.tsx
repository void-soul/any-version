import type { ReactNode } from "react";
import type { PackageStorageInfo } from "./packageStorageTypes";
import { useTranslation } from "react-i18next";
import { Trash2, FolderSync } from "lucide-react";
import type { ProjectStatus } from "../types";

interface PackageCacheStorageCardProps {
  cacheInfo: PackageStorageInfo;
  openWorkflow: (type: "cache" | "data", extraId?: string | undefined) => void;
  projectStatus: ProjectStatus | null | undefined;
  workflowType: "cache" | "data" | null;
  handleCleanCache: () => Promise<void>;
  cleaningCache: boolean;
  cleanProgress: { stage: string; current: number; total: number; file_name: string; } | null;
  workflowExtraCacheId: string | null;
  renderWorkflow: () => ReactNode;
}

/** Pure PackageCacheStorageCard view; its owning panel retains side effects and state. */
export function PackageCacheStorageCard({ cacheInfo, openWorkflow, projectStatus, workflowType, handleCleanCache, cleaningCache, cleanProgress, workflowExtraCacheId, renderWorkflow }: PackageCacheStorageCardProps) {
  const { t } = useTranslation();
  return (
    <div className="p-4 bg-black/20 rounded-card border border-white/5 space-y-3">
      <div className="flex items-start justify-between">
        <div className="space-y-1 flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            {cacheInfo.detect_source && (
              <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-tiny inline-flex items-center font-mono">
                {cacheInfo.detect_source}
              </span>
            )}
            {cacheInfo.real_target ? (
              <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-tiny inline-flex items-center font-semibold">
                {t("pkgmgr.migratedJunction")}
              </span>
            ) : (
              <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-tiny inline-flex items-center">
                {t("pkgmgr.defaultPath")}
              </span>
            )}
          </div>
          <p className="font-mono text-body text-slate-400 break-all">{cacheInfo.path}</p>
          {cacheInfo.real_target && (
            <p className="font-mono text-caption text-slate-500 break-all">
              {t("pkgmgr.actualTarget", { path: cacheInfo.real_target })}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          <span className="text-slate-300 font-mono text-title font-semibold bg-white/5 px-2.5 py-1 rounded-ctl">
            {cacheInfo.size}
          </span>
        </div>
      </div>

      {/* 操作行 */}
      <div className="pt-2 border-t border-white/5 flex items-center gap-2">
        <button onClick={() => openWorkflow("cache")} disabled={!projectStatus?.managed || workflowType !== null}
          className="px-3 py-1.5 bg-amber-600/80 hover:bg-amber-600 disabled:opacity-40 text-white rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-all" title={!projectStatus?.managed ? t("pkgmgr.managedFirst") : ""}>
          <FolderSync className="w-3.5 h-3.5" />{t("pkgmgr.startChange")}
        </button>
        <button onClick={handleCleanCache} disabled={!projectStatus?.managed || cleaningCache || workflowType !== null}
          className="px-3 py-1.5 bg-red-600/10 hover:bg-red-600/20 text-red-400 disabled:opacity-40 text-white rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-all" title={!projectStatus?.managed ? t("pkgmgr.managedFirst") : ""}>
          <Trash2 className="w-3.5 h-3.5" />{cleaningCache ? t("pkgmgr.cleaning") : t("pkgmgr.cleanCache")}
        </button>
      </div>
      {cleanProgress && (
        <div className="space-y-1 pt-1">
          <div className="flex items-center justify-between text-title text-slate-400">
            <span>{cleanProgress.stage}</span>
            <span>{cleanProgress.current}/{cleanProgress.total}</span>
          </div>
          <div className="w-full h-1 bg-white/5 rounded-full overflow-hidden">
            <div className="h-full bg-red-500/60 rounded-full transition-all duration-200"
              style={{ width: `${cleanProgress.total > 0 ? (cleanProgress.current / cleanProgress.total) * 100 : 0}%` }} />
          </div>
          {cleanProgress.file_name && (
            <p className="text-title text-slate-500 truncate font-mono">{cleanProgress.file_name}</p>
          )}
        </div>
      )}

      {/* 工作流面板 — 缓存变更 */}
      {workflowType === "cache" && !workflowExtraCacheId && renderWorkflow()}
    </div>
  );
}
