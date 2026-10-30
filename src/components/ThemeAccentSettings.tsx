import { VEX_THEME_PRESETS } from "../utils/brand";
import { useTranslation } from "react-i18next";

interface ThemeAccentSettingsProps {
  themeAccent: string;
  handleResetThemeAccent: () => Promise<void>;
  handleSetThemeAccent: (color: string) => Promise<void>;
}

/** Pure ThemeAccentSettings view; its owning panel retains side effects and state. */
export function ThemeAccentSettings({ themeAccent, handleResetThemeAccent, handleSetThemeAccent }: ThemeAccentSettingsProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2.5">
      <div className="flex items-center justify-between pt-3 border-t border-white/5">
        <p className="text-caption font-medium text-slate-200">{t("settings.themeColor")}</p>
        <div className="flex items-center gap-2">
          <div
            className="flex items-center gap-1.5 rounded-md border border-white/10 bg-black/30 px-2 py-1 font-mono text-tiny text-slate-200"
            style={{ boxShadow: `0 0 8px ${themeAccent}55` }}
          >
            <span className="h-3 w-3 rounded-full" style={{ background: themeAccent }} />
            <span>{themeAccent}</span>
          </div>
          <button
            onClick={() => void handleResetThemeAccent()}
            className="rounded-md border border-white/10 px-2 py-1 text-tiny text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer"
            title={t("settings.resetTheme")}
          >
            {t("settings.resetThemeBtn")}
          </button>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {VEX_THEME_PRESETS.map((c) => (
          <button
            key={c}
            onClick={() => void handleSetThemeAccent(c)}
            title={c}
            className={`h-8 w-8 rounded-ctl border transition cursor-pointer ${
              c === themeAccent ? "border-white ring-2 ring-white/40" : "border-white/15 hover:border-white/50"
            }`}
            style={{ background: c }}
          />
        ))}
        <label
          className="relative h-8 w-8 rounded-ctl border border-white/15 hover:border-white/50 cursor-pointer flex items-center justify-center overflow-hidden transition"
          title={t("settings.customColor")}
        >
          <span className="h-full w-full" style={{ background: "conic-gradient(red, yellow, lime, cyan, blue, magenta, red)" }} />
          <input
            type="color"
            value={themeAccent}
            onChange={(e) => void handleSetThemeAccent(e.target.value)}
            className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
          />
        </label>
      </div>
      <p className="text-micro text-slate-600">{t("settings.themePresetsHint")}</p>
    </div>
  );
}
