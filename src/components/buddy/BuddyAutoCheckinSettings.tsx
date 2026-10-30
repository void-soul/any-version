import { useTranslation } from "react-i18next";
import { Loader2, CalendarCheck, Play, Save } from "lucide-react";
import { type BuddyAutoCheckinConfig } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface BuddyAutoCheckinSettingsProps {
  autoConfig: BuddyAutoCheckinConfig | null;
  setAutoConfig: Dispatch<SetStateAction<BuddyAutoCheckinConfig | null>>;
  saveAutoConfig: () => Promise<void>;
  autoBusy: boolean;
  runAutoCheckin: (force: boolean) => Promise<void>;
}

/** Pure BuddyAutoCheckinSettings view; its owning panel retains side effects and state. */
export function BuddyAutoCheckinSettings({ autoConfig, setAutoConfig, saveAutoConfig, autoBusy, runAutoCheckin }: BuddyAutoCheckinSettingsProps) {
  const { t } = useTranslation();
  return (
    <div className="rounded-card border border-white/10 bg-white/[0.03] p-4 space-y-3">
      <div className="flex items-center gap-2 mb-1">
        <CalendarCheck className="w-4 h-4 text-[var(--module-accent)]" />
        <span className="text-title font-bold text-white">{t("buddy.auto.title")}</span>
      </div>
      {autoConfig ? (
        <div className="space-y-3">
          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={autoConfig.enabled}
              onChange={(e) => setAutoConfig({ ...autoConfig, enabled: e.target.checked })}
              className="accent-[var(--module-accent)] w-3.5 h-3.5"
            />
            <span className="text-caption text-slate-300">{t("buddy.auto.enabled")}</span>
          </label>
          <div className="flex items-center gap-3">
            <label className="flex items-center gap-2">
              <span className="text-tiny text-slate-500">{t("buddy.auto.startTime")}</span>
              <input
                type="time"
                value={autoConfig.startTime}
                onChange={(e) => setAutoConfig({ ...autoConfig, startTime: e.target.value })}
                className="bg-black/30 border border-white/10 rounded-ctl px-2 py-1 text-caption text-white outline-none"
              />
            </label>
            <label className="flex items-center gap-2">
              <span className="text-tiny text-slate-500">{t("buddy.auto.endTime")}</span>
              <input
                type="time"
                value={autoConfig.endTime}
                onChange={(e) => setAutoConfig({ ...autoConfig, endTime: e.target.value })}
                className="bg-black/30 border border-white/10 rounded-ctl px-2 py-1 text-caption text-white outline-none"
              />
            </label>
          </div>
          <p className="text-micro text-slate-600">{t("buddy.auto.hint")}</p>
          <div className="flex gap-2">
            <button
              onClick={saveAutoConfig}
              disabled={autoBusy}
              className="px-3 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
            >
              {autoBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}
              {t("buddy.auto.save")}
            </button>
            <button
              onClick={() => void runAutoCheckin(true)}
              disabled={autoBusy}
              className="px-3 py-1.5 rounded-ctl text-caption bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
            >
              <Play className="w-3 h-3" /> {t("buddy.auto.runNow")}
            </button>
          </div>
        </div>
      ) : (
        <div className="text-tiny text-slate-600">{t("buddy.auto.loading")}</div>
      )}
    </div>
  );
}
