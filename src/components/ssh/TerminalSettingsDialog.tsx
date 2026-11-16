import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { FONT_FAMILIES, loadPrefs, savePrefs, type TerminalPrefs } from "./terminalSettings";

const btn = "inline-flex items-center gap-1.5 rounded px-2.5 py-1.5 text-tiny border border-white/10 bg-black/30 text-slate-200 hover:bg-white/10";
const field = "w-full rounded border border-white/10 bg-black/30 px-2 py-1.5 text-tiny text-slate-100 outline-none focus:border-sky-500/60";

/** 终端外观设置：保存后通过事件广播，已打开的终端即时生效。 */
export function TerminalSettingsDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [prefs, setPrefs] = useState<TerminalPrefs>(() => loadPrefs());

  const patch = (key: keyof TerminalPrefs, value: string | number) =>
    setPrefs((prev) => ({ ...prev, [key]: value }));

  const save = () => {
    savePrefs(prefs);
    onClose();
  };

  return (
    <div className="fixed inset-0 z-[140] flex items-center justify-center bg-black/70 p-4">
      <div className="w-[380px] max-w-full rounded border border-white/10 bg-slate-900 p-4 text-slate-200 shadow-2xl">
        <div className="mb-3 flex items-center justify-between">
          <h3 className="text-caption font-semibold">{t("terminalSettings.title")}</h3>
          <button className={btn} onClick={onClose}><X className="h-3.5 w-3.5" /></button>
        </div>
        <div className="space-y-3">
          <label className="block text-tiny text-slate-400">{t("terminalSettings.fontSize")}
            <input className={field} type="number" min={8} max={32} value={prefs.fontSize}
              onChange={(e) => patch("fontSize", Number(e.target.value))} />
          </label>
          <label className="block text-tiny text-slate-400">{t("terminalSettings.fontFamily")}
            <select className={field} value={prefs.fontFamily} onChange={(e) => patch("fontFamily", e.target.value)}>
              {FONT_FAMILIES.map((f) => <option key={f.label} value={f.value}>{f.label}</option>)}
            </select>
          </label>
          <label className="block text-tiny text-slate-400">{t("terminalSettings.theme")}
            <select className={field} value={prefs.theme} onChange={(e) => patch("theme", e.target.value)}>
              <option value="dark">{t("terminalSettings.themeDark")}</option>
              <option value="light">{t("terminalSettings.themeLight")}</option>
            </select>
          </label>
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <button className={btn} onClick={onClose}>{t("rdp.cancel")}</button>
          <button className={btn} onClick={save}>{t("rdp.save")}</button>
        </div>
      </div>
    </div>
  );
}
