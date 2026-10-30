import { Loader2, Upload } from "lucide-react";
import { useTranslation } from "react-i18next";

interface ApplicationFontSelectorProps {
  appearance: { moduleThemeColors: Record<string, string>; globalFont: string; customFontPath: string; moduleOrder: string[]; toolbarModules: string[]; disabledModules: string[]; backgroundTexture: string; language: string; motionPreference: string; density: string; };
  handleSetGlobalFont: (font: string) => Promise<void>;
  filteredFonts: string[];
  handleClearCustomFont: () => Promise<void>;
  handleImportFont: () => Promise<void>;
  importingFont: boolean;
}

/** Pure ApplicationFontSelector view; its owning panel retains side effects and state. */
export function ApplicationFontSelector({ appearance, handleSetGlobalFont, filteredFonts, handleClearCustomFont, handleImportFont, importingFont }: ApplicationFontSelectorProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-2">
      <select
        value={appearance.globalFont}
        onChange={(e) => handleSetGlobalFont(e.target.value)}
        className="flex-1 min-w-0 glass-input text-body"
      >
        <option value="">{t("settings.defaultFont")}</option>
        {filteredFonts.map((f) => (
          <option key={f} value={f}>
            {f}
          </option>
        ))}
      </select>
      {appearance.customFontPath && (
        <button
          onClick={handleClearCustomFont}
          className="px-2.5 py-1.5 rounded-ctl text-tiny font-medium bg-[color-mix(in_srgb,var(--module-accent)_20%,transparent)] border border-[color-mix(in_srgb,var(--module-accent)_40%,transparent)] text-[var(--module-accent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_30%,transparent)] transition cursor-pointer whitespace-nowrap"
          title={t("settings.removeCustomFont")}
        >
          {t("settings.removeCustomFont")}
        </button>
      )}
      <button
        onClick={handleImportFont}
        disabled={importingFont}
        className="px-2.5 py-1.5 rounded-ctl text-tiny font-medium bg-cyan-600/20 border border-cyan-500/40 text-cyan-300 hover:bg-cyan-600/30 transition cursor-pointer whitespace-nowrap flex items-center gap-1 disabled:opacity-50"
        title={t("settings.importFontTitle")}
      >
        {importingFont ? <Loader2 className="w-3 h-3 animate-spin" /> : <Upload className="w-3 h-3" />}
        {t("settings.importFont")}
      </button>
    </div>
  );
}
