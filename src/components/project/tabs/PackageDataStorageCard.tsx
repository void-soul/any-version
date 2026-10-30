import type { ReactNode } from "react";
import type { PackageStorageInfo } from "./packageStorageTypes";
import { useTranslation } from "react-i18next";
import { AlertTriangle, HardDrive, FolderSync } from "lucide-react";
import type { ProjectStatus, PackageManagerDef } from "../types";

interface PackageDataStorageCardProps {
  dataInfo: PackageStorageInfo | null;
  pm: PackageManagerDef;
  openWorkflow: (type: "cache" | "data", extraId?: string | undefined) => void;
  projectStatus: ProjectStatus | null | undefined;
  workflowType: "cache" | "data" | null;
  renderWorkflow: () => ReactNode;
}

/** Pure PackageDataStorageCard view; its owning panel retains side effects and state. */
export function PackageDataStorageCard({ dataInfo, pm, openWorkflow, projectStatus, workflowType, renderWorkflow }: PackageDataStorageCardProps) {
  const { t } = useTranslation();
  return (
    <div className="glass-panel rounded-panel p-4 border border-white/5 bg-white/2 space-y-3">
      <div className="flex items-center gap-2">
        <HardDrive className="w-4 h-4 text-red-400" />
        <h4 className="text-body font-semibold text-white">{t("pkgmgr.dataTitle")}</h4>
        <span className="text-caption px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 border border-red-500/20">{t("pkgmgr.dataTag")}</span>
      </div>
      <div className="p-4 bg-black/20 rounded-card border border-white/5 space-y-3">
        {/* 数据状态 */}
        {dataInfo ? (
          <div className="flex items-start justify-between">
            <div className="space-y-1 flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                {dataInfo.detect_source && (
                  <span className="px-1.5 py-0.5 rounded bg-[var(--module-accent-soft)] text-[var(--module-accent)] border border-[var(--module-accent-ring)] text-tiny inline-flex items-center font-mono">
                    {dataInfo.detect_source}
                  </span>
                )}
                {dataInfo.real_target ? (
                  <span className="px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 text-tiny inline-flex items-center font-semibold">
                    {t("pkgmgr.migratedJunction")}
                  </span>
                ) : (
                  <span className="px-1.5 py-0.5 rounded bg-slate-500/10 text-slate-400 border border-slate-500/20 text-tiny inline-flex items-center">
                    {t("pkgmgr.defaultPath")}
                  </span>
                )}
              </div>
              <p className="font-mono text-body text-slate-400 break-all">{dataInfo.path}</p>
              {dataInfo.real_target && (
                <p className="font-mono text-caption text-slate-500 break-all">
                  {t("pkgmgr.actualTarget", { path: dataInfo.real_target })}
                </p>
              )}
            </div>
            <div className="flex items-center gap-2 flex-shrink-0">
              <span className="text-slate-300 font-mono text-title font-semibold bg-white/5 px-2.5 py-1 rounded-ctl">
                {dataInfo.size}
              </span>
            </div>
          </div>
        ) : (
          <div className="space-y-1">
            <p className="text-body text-slate-500 flex items-center gap-1.5">
              <AlertTriangle className="w-3 h-3 text-yellow-400" />
              <span>{t("pkgmgr.notSetDetect")} </span>
              <span className="text-caption font-mono text-slate-400">{pm.data_detect_cmd}</span>
              <span> {t("pkgmgr.noValidPath")}</span>
            </p>
          </div>
        )}

        <p className="text-caption text-red-400/70">{t("pkgmgr.dataSafeWarn")}</p>

        {/* 操作行 */}
        <div className="pt-2 border-t border-white/5 flex items-center gap-2">
          <button onClick={() => openWorkflow("data")} disabled={!projectStatus?.managed || workflowType !== null}
            className="px-3 py-1.5 bg-red-600 hover:bg-red-500 disabled:opacity-40 text-white rounded-ctl text-body font-semibold cursor-pointer flex items-center gap-1 transition-colors" title={!projectStatus?.managed ? t("pkgmgr.managedFirst") : ""}>
            <FolderSync className="w-3.5 h-3.5" />{dataInfo ? t("pkgmgr.startMigrate") : t("pkgmgr.setDataDir")}
          </button>
        </div>

        {/* 工作流面板 — 数据迁移 */}
        {workflowType === "data" && renderWorkflow()}
      </div>
    </div>
  );
}
