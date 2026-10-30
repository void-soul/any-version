import { Power } from "lucide-react";
import { useTranslation } from "react-i18next";
import { type TrayMenuConfig } from "./GlobalSettings.model";

interface ApplicationStartupSettingsProps {
  handleToggleAutostart: () => Promise<void>;
  autostartBusy: boolean;
  autostartOn: boolean;
  trayCfg: TrayMenuConfig;
  saveTrayCfg: (patch: Partial<TrayMenuConfig>) => Promise<void>;
}

/** Pure ApplicationStartupSettings view; its owning panel retains side effects and state. */
export function ApplicationStartupSettings({ handleToggleAutostart, autostartBusy, autostartOn, trayCfg, saveTrayCfg }: ApplicationStartupSettingsProps) {
  const { t } = useTranslation();
  return (
    <div className="glass-panel rounded-panel p-6 border border-white/5 space-y-4">
      <div className="flex items-center gap-2 pb-3 border-b border-white/5">
        <Power className="w-4 h-4 text-[var(--module-accent)]" />
        <h3 className="text-body font-semibold text-white">{t("settings.behavior")}</h3>
      </div>

      <div className="flex items-center justify-between">
        <div className="space-y-0.5">
          <p className="text-body font-medium text-slate-200">{t("settings.autostart")}</p>
          <p className="text-micro text-slate-500">
            {t("settings.autostartHint")}
          </p>
        </div>
        <button
          onClick={handleToggleAutostart}
          disabled={autostartBusy}
          role="switch"
          aria-checked={autostartOn}
          className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer disabled:opacity-50 ${
            autostartOn ? "bg-[var(--module-accent)]" : "bg-white/10"
          }`}
          title={autostartOn ? t("settings.autostartOn") : t("settings.autostartOff")}
        >
          <span
            className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
              autostartOn ? "translate-x-4" : "translate-x-0.5"
            }`}
          />
        </button>
      </div>

      {/* 托盘右键菜单总开关（各模块的细分项在对应模块的设置弹窗中） */}
      <div className="pt-3 border-t border-white/5 space-y-3">
        <div className="flex items-center justify-between">
          <div className="space-y-0.5">
            <p className="text-body font-medium text-slate-200">{t("settings.trayMenuEnabled")}</p>
            <p className="text-micro text-slate-500">{t("settings.trayMenuEnabledHint")}</p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={trayCfg.enabled}
            onClick={() => saveTrayCfg({ enabled: !trayCfg.enabled })}
            className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors cursor-pointer ${
              trayCfg.enabled ? "bg-[var(--module-accent)]" : "bg-white/15"
            }`}
          >
            <span
              className={`inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm transition-transform ${
                trayCfg.enabled ? "translate-x-4" : "translate-x-0.5"
              }`}
            />
          </button>
        </div>
      </div>
    </div>
  );
}
