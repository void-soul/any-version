import { CheckCircle2, AlertTriangle, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { type MigrateResult } from "./GlobalSettings.model";

interface StorageMigrationResultProps {
  migrateResult: MigrateResult;
  deletedOldDirs: string[] | null;
  handleDeleteOldDirs: () => Promise<void>;
  deletingOldDirs: boolean;
}

/** Pure StorageMigrationResult view; its owning panel retains side effects and state. */
export function StorageMigrationResult({ migrateResult, deletedOldDirs, handleDeleteOldDirs, deletingOldDirs }: StorageMigrationResultProps) {
  const { t } = useTranslation();
  return (
    <div className="p-4 bg-emerald-500/5 border border-emerald-500/15 rounded-card space-y-2 text-tiny">
      <h4 className="text-body font-semibold text-emerald-400">
        {t("settings.migrateDone")}
      </h4>
      {migrateResult.moved_versions && (
        <p className="text-slate-300">{t("settings.migMovedVersions")}</p>
      )}
      {migrateResult.moved_links && (
        <p className="text-slate-300">{t("settings.migMovedLinks")}</p>
      )}
      {migrateResult.recreated_junctions.length > 0 && (
        <p className="text-slate-300">
          {t("settings.migRecreatedJunctions", { count: migrateResult.recreated_junctions.length })}{" "}
          {migrateResult.recreated_junctions.join(", ")}
        </p>
      )}
      {migrateResult.updated_env_vars.length > 0 && (
        <p className="text-slate-300">
          {t("settings.migUpdatedEnv")}{" "}
          {migrateResult.updated_env_vars.join(", ")}
        </p>
      )}
      {migrateResult.updated_path_entries.length > 0 && (
        <p className="text-slate-300">
          {t("settings.migUpdatedPath", { count: migrateResult.updated_path_entries.length })}
        </p>
      )}

      {/* 旧目录清理提示 */}
      {migrateResult.old_dirs_remain.length > 0 && (
        <div className="pt-2 mt-2 border-t border-amber-500/15 space-y-2">
          <div className="flex items-start gap-1.5 text-amber-300">
            <AlertTriangle className="w-3 h-3 mt-0.5 flex-shrink-0" />
            <span>
              {t("settings.oldDirsRemain")}
            </span>
          </div>
          {migrateResult.old_dirs_remain.map((dir, i) => (
            <p
              key={i}
              className="font-mono text-micro text-slate-400 pl-5"
            >
              {dir}
            </p>
          ))}
          {deletedOldDirs ? (
            <p className="text-emerald-400 text-tiny flex items-center gap-1">
              <CheckCircle2 className="w-3 h-3" />
              {t("settings.deletedOldDirs", { count: deletedOldDirs.length })}
            </p>
          ) : (
            <button
              onClick={handleDeleteOldDirs}
              disabled={deletingOldDirs}
              className="px-3 py-1.5 bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_40%,transparent)] disabled:opacity-50 text-[var(--module-accent)] rounded-ctl text-tiny font-medium cursor-pointer transition-all flex items-center gap-1.5 border border-[var(--module-accent-ring)]"
            >
              <Trash2 className="w-3 h-3" />
              {deletingOldDirs ? t("settings.deletingOld") : t("settings.deleteOldDirs")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
