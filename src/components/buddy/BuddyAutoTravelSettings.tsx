import { useTranslation } from "react-i18next";
import { Loader2, Play, Save, Cat } from "lucide-react";
import { TRAVEL_LOCATIONS, type BuddyAutoTravelConfig } from "./BuddyPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface BuddyAutoTravelSettingsProps {
  travelConfig: BuddyAutoTravelConfig | null;
  setTravelConfig: Dispatch<SetStateAction<BuddyAutoTravelConfig | null>>;
  saveTravelConfig: () => Promise<void>;
  autoBusy: boolean;
  runAutoTravel: () => Promise<void>;
}

/** Pure BuddyAutoTravelSettings view; its owning panel retains side effects and state. */
export function BuddyAutoTravelSettings({ travelConfig, setTravelConfig, saveTravelConfig, autoBusy, runAutoTravel }: BuddyAutoTravelSettingsProps) {
  const { t } = useTranslation();
  return (
    <div className="rounded-card border border-white/10 bg-white/[0.03] p-4 space-y-3">
      <div className="flex items-center gap-2 mb-1">
        <Cat className="w-4 h-4 text-[var(--module-accent)]" />
        <span className="text-title font-bold text-white">{t("buddy.travel.title")}</span>
      </div>
      {travelConfig ? (
        <div className="space-y-3">
          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={travelConfig.enabled}
              onChange={(e) => setTravelConfig({ ...travelConfig, enabled: e.target.checked })}
              className="accent-[var(--module-accent)] w-3.5 h-3.5"
            />
            <span className="text-caption text-slate-300">{t("buddy.travel.enabled")}</span>
          </label>
          <div className="flex items-center gap-3 flex-wrap">
            <label className="flex items-center gap-2">
              <span className="text-tiny text-slate-500">{t("buddy.travel.startTime")}</span>
              <input
                type="time"
                value={travelConfig.startTime}
                onChange={(e) => setTravelConfig({ ...travelConfig, startTime: e.target.value })}
                className="bg-black/30 border border-white/10 rounded-ctl px-2 py-1 text-caption text-white outline-none"
              />
            </label>
            <label className="flex items-center gap-2">
              <span className="text-tiny text-slate-500">{t("buddy.travel.endTime")}</span>
              <input
                type="time"
                value={travelConfig.endTime}
                onChange={(e) => setTravelConfig({ ...travelConfig, endTime: e.target.value })}
                className="bg-black/30 border border-white/10 rounded-ctl px-2 py-1 text-caption text-white outline-none"
              />
            </label>
            <label className="flex items-center gap-2">
              <span className="text-tiny text-slate-500">{t("buddy.travel.location")}</span>
              <select
                value={travelConfig.locationId}
                onChange={(e) =>
                  setTravelConfig({ ...travelConfig, locationId: Number(e.target.value) })
                }
                className="bg-black/30 border border-white/10 rounded-ctl px-2 py-1 text-caption text-white outline-none"
              >
                {TRAVEL_LOCATIONS.map((loc) => (
                  <option key={loc.id} value={loc.id}>
                    {t(`buddy.travel.${loc.key}`)}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <p className="text-micro text-slate-600">{t("buddy.travel.hint")}</p>
          <div className="flex gap-2">
            <button
              onClick={saveTravelConfig}
              disabled={autoBusy}
              className="px-3 py-1.5 rounded-ctl text-caption ui-btn-primary text-white font-semibold flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
            >
              {autoBusy ? <Loader2 className="w-3 h-3 animate-spin" /> : <Save className="w-3 h-3" />}
              {t("buddy.auto.save")}
            </button>
            <button
              onClick={() => void runAutoTravel()}
              disabled={autoBusy}
              className="px-3 py-1.5 rounded-ctl text-caption bg-white/5 hover:bg-white/10 text-slate-300 border border-white/10 flex items-center gap-1 cursor-pointer transition disabled:opacity-50"
            >
              <Play className="w-3 h-3" /> {t("buddy.travel.runNow")}
            </button>
          </div>
        </div>
      ) : (
        <div className="text-tiny text-slate-600">{t("buddy.auto.loading")}</div>
      )}
    </div>
  );
}
