import { Save, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { Dispatch, SetStateAction } from "react";

interface StorageMigrationConfirmProps {
  handleSave: () => Promise<void>;
  saving: boolean;
  setShowMigrateConfirm: Dispatch<SetStateAction<boolean>>;
}

/** Pure StorageMigrationConfirm view; its owning panel retains side effects and state. */
export function StorageMigrationConfirm({ handleSave, saving, setShowMigrateConfirm }: StorageMigrationConfirmProps) {
  const { t } = useTranslation();
  return (
    <div className="p-4 bg-amber-500/10 border border-amber-500/20 rounded-card space-y-3 animate-fadeIn">
      <h4 className="text-body font-semibold text-amber-400 flex items-center gap-1.5">
        <AlertTriangle className="w-4 h-4" />
        {t("settings.confirmMigrate")}
      </h4>
      <div className="text-tiny text-slate-300 space-y-1.5">
        <p>{t("settings.migrateNotice")}</p>
        <p className="text-amber-300">{t("settings.migrateStep1")}</p>
        <p className="text-amber-300">{t("settings.migrateStep2")}</p>
        <p className="text-amber-300">{t("settings.migrateStep3")}</p>
        <p className="text-slate-400 mt-1">{t("settings.migrateHint")}</p>
        <p className="text-amber-300">{t("settings.migrateRestartNote")}</p>
      </div>
      <div className="flex items-center gap-2 pt-1">
        <button
          onClick={handleSave}
          disabled={saving}
          className="px-4 py-2 bg-amber-600 hover:bg-amber-500 disabled:opacity-50 text-white rounded-card text-body font-semibold cursor-pointer transition-all flex items-center gap-1.5"
        >
          <Save className="w-3 h-3" />
          {saving ? t("settings.migrating") : t("settings.confirmMigrateSave")}
        </button>
        <button
          onClick={() => setShowMigrateConfirm(false)}
          className="px-4 py-2 bg-white/5 hover:bg-white/10 text-slate-300 rounded-card text-body font-medium cursor-pointer border border-white/10"
        >
          {t("common.cancel")}
        </button>
      </div>
    </div>
  );
}
