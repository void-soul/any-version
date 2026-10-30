import { LayoutGrid, Settings2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { LauncherSetting } from "./types";
import type { Dispatch, SetStateAction, RefObject } from "react";

interface LauncherViewSettingsProps {
  viewSettingsRef: RefObject<HTMLDivElement | null>;
  setViewSettingsOpen: Dispatch<SetStateAction<boolean>>;
  viewSettingsOpen: boolean;
  settings: LauncherSetting;
  saveViewSettings: (patch: Partial<LauncherSetting>) => Promise<void>;
}


/** Pure LauncherViewSettings view; its owning panel retains side effects and state. */
export function LauncherViewSettings({ viewSettingsRef, setViewSettingsOpen, viewSettingsOpen, settings, saveViewSettings }: LauncherViewSettingsProps) {
  const { t } = useTranslation();
  return (
<div className="relative" ref={viewSettingsRef}>
            <button
              onClick={() => setViewSettingsOpen((v) => !v)}
              className={`px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border transition cursor-pointer ${
                viewSettingsOpen
                  ? "ui-selected"
                  : "ui-btn"
              }`}
              title={t("launcher.viewTitle")}
            >
              <LayoutGrid className="w-3 h-3 text-cyan-400" />
              <span className="text-caption">{t("launcher.view")}</span>
            </button>

            {viewSettingsOpen && (
              <div className="absolute right-0 mt-1 z-[150] w-64 bg-surface-panel border border-white/15 rounded-card shadow-2xl shadow-black/50 p-3 space-y-3 max-h-[75vh] overflow-y-auto animate-in fade-in zoom-in-95 duration-100">
                <div className="flex items-center justify-between">
                  <span className="text-caption font-semibold text-slate-200 flex items-center gap-1">
                    <Settings2 className="w-3 h-3 text-[var(--module-accent)]" />
                    {t("launcher.globalViewSettings")}
                  </span>
                </div>

                {/* 图标大小 */}
                <div>
                  <label className="block text-tiny text-slate-400 mb-1">
                    {t("launcher.iconSize", { size: settings.itemIconSize ?? 32 })}
                  </label>
                  <input
                    type="range"
                    min={16}
                    max={64}
                    step={4}
                    value={settings.itemIconSize ?? 32}
                    onChange={(e) => saveViewSettings({ itemIconSize: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 网格列数 */}
                <div>
                  <label className="block text-tiny text-slate-400 mb-1">{t("launcher.colsPerRow")}</label>
                  <div className="flex flex-wrap gap-1">
                    {[0, 3, 4, 5, 6, 7, 8].map((col) => (
                      <button
                        key={col}
                        onClick={() => saveViewSettings({ itemColumnNumber: col })}
                        className={`px-2 py-1 rounded-ctl text-tiny border transition cursor-pointer ${
                          (settings.itemColumnNumber ?? 0) === col
                            ? "ui-selected"
                            : "bg-white/5 border-white/10 text-slate-400 hover:bg-white/10"
                        }`}
                      >
                        {col === 0 ? t("launcher.adaptive") : t("launcher.colCount", { count: col })}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 卡片密度 */}
                <div>
                  <label className="block text-tiny text-slate-400 mb-1">{t("launcher.cardDensity")}</label>
                  <div className="grid grid-cols-3 gap-1">
                    {[
                      { key: "compact", label: "launcher.compact" },
                      { key: "cozy", label: "launcher.cozy" },
                      { key: "spacious", label: "launcher.spacious" },
                    ].map((opt) => (
                      <button
                        key={opt.key}
                        onClick={() => saveViewSettings({ cardDensity: opt.key as LauncherSetting["cardDensity"] })}
                        className={`px-2 py-1.5 rounded-ctl text-tiny border transition cursor-pointer ${
                          (settings.cardDensity ?? "cozy") === opt.key
                            ? "ui-selected"
                            : "bg-white/5 border-white/10 text-slate-400 hover:bg-white/10"
                        }`}
                      >
                        {t(opt.label)}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 项目文字大小 */}
                <div>
                  <label className="block text-tiny text-slate-400 mb-1">
                    {t("launcher.itemFontSize", { size: settings.itemFontSize ?? 12 })}
                  </label>
                  <input
                    type="range"
                    min={9}
                    max={20}
                    step={1}
                    value={settings.itemFontSize ?? 12}
                    onChange={(e) => saveViewSettings({ itemFontSize: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 分类文字大小 */}
                <div>
                  <label className="block text-tiny text-slate-400 mb-1">
                    {t("launcher.catFontSize", { size: settings.categoryFontSize ?? 12 })}
                  </label>
                  <input
                    type="range"
                    min={9}
                    max={20}
                    step={1}
                    value={settings.categoryFontSize ?? 12}
                    onChange={(e) => saveViewSettings({ categoryFontSize: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 项目圆角 */}
                <div>
                  <label className="block text-tiny text-slate-400 mb-1">
                    {t("launcher.itemRadius", { size: settings.itemRadius ?? 12 })}
                  </label>
                  <input
                    type="range"
                    min={0}
                    max={20}
                    step={1}
                    value={settings.itemRadius ?? 12}
                    onChange={(e) => saveViewSettings({ itemRadius: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 分类间距 */}
                <div>
                  <label className="block text-tiny text-slate-400 mb-1">
                    {t("launcher.catGap", { size: settings.categoryGap ?? 24 })}
                  </label>
                  <input
                    type="range"
                    min={0}
                    max={32}
                    step={1}
                    value={settings.categoryGap ?? 24}
                    onChange={(e) => saveViewSettings({ categoryGap: Number(e.target.value) })}
                    className="w-full accent-[var(--module-accent)]"
                  />
                </div>

                {/* 开关 */}
                <div className="space-y-1.5">
                  <label className="flex items-center justify-between cursor-pointer">
                    <span className="text-tiny text-slate-400">{t("launcher.validOnly")}</span>
                    <button
                      onClick={() => saveViewSettings({ showOnlyValid: !(settings.showOnlyValid ?? false) })}
                      className={`w-7 h-4 rounded-full transition relative cursor-pointer ${
                        settings.showOnlyValid ?? false ? "bg-[var(--module-accent)]" : "bg-white/15"
                      }`}
                      title={t("launcher.hideMissingTitle")}
                    >
                      <span
                        className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-all ${
                          settings.showOnlyValid ?? false ? "left-3.5" : "left-0.5"
                        }`}
                      />
                    </button>
                  </label>
                  <label className="flex items-center justify-between cursor-pointer">
                    <span className="text-tiny text-slate-400">{t("launcher.itemBorder")}</span>
                    <button
                      onClick={() => saveViewSettings({ itemBorder: !(settings.itemBorder ?? true) })}
                      className={`w-7 h-4 rounded-full transition relative cursor-pointer ${
                        settings.itemBorder ?? true ? "bg-[var(--module-accent)]" : "bg-white/15"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-all ${
                          settings.itemBorder ?? true ? "left-3.5" : "left-0.5"
                        }`}
                      />
                    </button>
                  </label>
                  <label className="flex items-center justify-between cursor-pointer">
                    <span className="text-tiny text-slate-400">{t("launcher.showItemName")}</span>
                    <button
                      onClick={() => saveViewSettings({ showItemName: !(settings.showItemName ?? true) })}
                      className={`w-7 h-4 rounded-full transition relative cursor-pointer ${
                        settings.showItemName ?? true ? "bg-[var(--module-accent)]" : "bg-white/15"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-all ${
                          settings.showItemName ?? true ? "left-3.5" : "left-0.5"
                        }`}
                      />
                    </button>
                  </label>
                  <label className="flex items-center justify-between cursor-pointer">
                    <span className="text-tiny text-slate-400">{t("launcher.iconBgColor")}</span>
                    <button
                      onClick={() => saveViewSettings({ iconBackgroundColor: !(settings.iconBackgroundColor ?? false) })}
                      className={`w-7 h-4 rounded-full transition relative cursor-pointer ${
                        settings.iconBackgroundColor ?? false ? "bg-[var(--module-accent)]" : "bg-white/15"
                      }`}
                    >
                      <span
                        className={`absolute top-0.5 w-3 h-3 rounded-full bg-white transition-all ${
                          settings.iconBackgroundColor ?? false ? "left-3.5" : "left-0.5"
                        }`}
                      />
                    </button>
                  </label>
                </div>
              </div>
            )}
          </div>
  );
}
