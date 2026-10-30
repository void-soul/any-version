import { useTranslation } from "react-i18next";

interface BackgroundTextureSettingsProps {
  appearance: { moduleThemeColors: Record<string, string>; globalFont: string; customFontPath: string; moduleOrder: string[]; toolbarModules: string[]; disabledModules: string[]; backgroundTexture: string; language: string; motionPreference: string; density: string; };
  handleSetBackgroundTexture: (texture: string) => Promise<void>;
}

/** Pure BackgroundTextureSettings view; its owning panel retains side effects and state. */
export function BackgroundTextureSettings({ appearance, handleSetBackgroundTexture }: BackgroundTextureSettingsProps) {
  const { t } = useTranslation();
  return (
    <div className="pt-3 border-t border-white/5 space-y-2.5">
      <p className="text-caption font-medium text-slate-200">{t("settings.backgroundTexture")}</p>
      <p className="text-micro text-slate-500">{t("settings.backgroundTextureHint")}</p>
      <div className="grid grid-cols-5 gap-1.5">
        {[
          { value: "", label: "settings.textureGrid", cls: "cyber-grid" },
          { value: "dots", label: "settings.textureDots", cls: "app-bg-dots" },
          { value: "scanline", label: "settings.textureScanline", cls: "app-bg-scanline" },
          { value: "aurora", label: "settings.textureAurora", cls: "app-bg-aurora" },
          { value: "solid", label: "settings.textureSolid", cls: "app-bg-solid" },
          { value: "diagonal", label: "settings.textureDiagonal", cls: "app-bg-diagonal" },
          { value: "cross", label: "settings.textureCross", cls: "app-bg-cross" },
          { value: "paper", label: "settings.texturePaper", cls: "app-bg-paper" },
          { value: "hex", label: "settings.textureHex", cls: "app-bg-hex" },
          { value: "circuit", label: "settings.textureCircuit", cls: "app-bg-circuit" },
          { value: "waves", label: "settings.textureWaves", cls: "app-bg-waves" },
          { value: "noise", label: "settings.textureNoise", cls: "app-bg-noise" },
          { value: "vignette", label: "settings.textureVignette", cls: "app-bg-vignette" },
          { value: "neonline", label: "settings.textureNeonline", cls: "app-bg-neonline" },
        ].map((tc) => {
          const active = (appearance.backgroundTexture || "") === tc.value;
          return (
            <button
              key={tc.value}
              onClick={() => handleSetBackgroundTexture(tc.value)}
              className={`flex flex-col items-center gap-1 rounded-ctl border p-1.5 transition cursor-pointer ${
                active
                  ? "border-[var(--module-accent)] bg-[color-mix(in_srgb,var(--module-accent)_18%,transparent)]"
                  : "border-white/10 bg-white/[0.03] hover:border-white/25"
              }`}
              title={t(tc.label)}
            >
              <span className={`h-7 w-full rounded ${tc.cls} bg-black/40`} />
              <span className={`text-micro ${active ? "text-[var(--module-accent)]" : "text-slate-400"}`}>
                {t(tc.label)}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
