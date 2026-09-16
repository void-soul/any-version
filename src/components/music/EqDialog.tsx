// 音效设置：10 段图形均衡器 + 预设 + 总增益 + 声道平衡。
//
// 改动遵循「即时预览 + 自动保存」：拖动推子立刻通过 music_preview_eq 作用于播放线程，
// 停止改动 500ms 后由父组件落盘（本组件只负责把新参数交给父组件）。
import { useTranslation } from "react-i18next";
import { RotateCcw } from "lucide-react";

import { SharedButton } from "../shared/Button";
import type { EqParams, EqPresetInfo } from "./types";

/** 与后端 `dsp::BAND_FREQS` 保持一致 */
export const BAND_FREQS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/** 频率显示（1kHz 以上用 k 单位） */
export function bandLabel(freq: number): string {
  return freq >= 1000 ? `${freq / 1000}k` : `${freq}`;
}

interface Props {
  eq: EqParams;
  presets: EqPresetInfo[];
  onChange: (eq: EqParams) => void;
}

/** 找出与当前频段完全匹配的预设 id，否则返回 custom */
export function matchPresetId(bands: number[], presets: EqPresetInfo[]): string {
  const hit = presets.find((p) => p.bands.every((v, i) => Math.abs(v - (bands[i] ?? 0)) < 0.05));
  return hit ? hit.id : "custom";
}

export function EqDialog({ eq, presets, onChange }: Props) {
  const { t } = useTranslation();

  const setBand = (index: number, value: number) => {
    const bands = [...eq.bands];
    bands[index] = value;
    onChange({ ...eq, bands, preset: matchPresetId(bands, presets) });
  };

  const applyPreset = (presetId: string) => {
    if (presetId === "custom") return;
    const preset = presets.find((p) => p.id === presetId);
    if (!preset) return;
    const bands = [...preset.bands];
    onChange({ ...eq, bands, preset: presetId });
  };

  const reset = () => {
    onChange({ ...eq, bands: BAND_FREQS.map(() => 0), gain_db: 0, balance: 0, preset: "flat" });
  };

  return (
    <div className="space-y-4">
      {/* 开关 + 预设 */}
      <div className="flex items-center gap-3 flex-wrap">
        <label className="flex items-center gap-2 cursor-pointer select-none">
          <input
            type="checkbox"
            checked={eq.enabled}
            onChange={(e) => onChange({ ...eq, enabled: e.target.checked })}
            className="rounded border-white/10 bg-slate-800 text-[var(--module-accent)]"
          />
          <span className="text-[12px] text-slate-200">{t("music.eqEnabled")}</span>
        </label>
        <div className="flex items-center gap-2 ml-auto">
          <span className="text-[11px] text-slate-400">{t("music.eqPreset")}</span>
          <select
            value={eq.preset}
            onChange={(e) => applyPreset(e.target.value)}
            className="glass-input px-2 h-7 text-[11px] cursor-pointer"
          >
            {presets.map((p) => (
              <option key={p.id} value={p.id}>
                {t(`music.preset.${p.id}`)}
              </option>
            ))}
            <option value="custom">{t("music.preset.custom")}</option>
          </select>
          <SharedButton variant="secondary" className="!h-7 !px-2" onClick={reset}>
            <RotateCcw className="w-3 h-3" />
            {t("music.eqReset")}
          </SharedButton>
        </div>
      </div>

      {/* 10 段推子 */}
      <div className={`space-y-1.5 ${eq.enabled ? "" : "opacity-40 pointer-events-none"}`}>
        {BAND_FREQS.map((freq, index) => (
          <div key={freq} className="flex items-center gap-2">
            <span className="w-10 text-[10px] text-slate-500 text-right font-mono">{bandLabel(freq)}</span>
            <input
              type="range"
              min={-12}
              max={12}
              step={0.5}
              value={eq.bands[index] ?? 0}
              onChange={(e) => setBand(index, Number(e.target.value))}
              onDoubleClick={() => setBand(index, 0)}
              className="flex-1 accent-[var(--module-accent)] cursor-pointer"
              title={t("music.eqBandHint")}
            />
            <span className="w-12 text-[10px] font-mono text-slate-300 text-right">
              {(eq.bands[index] ?? 0) > 0 ? "+" : ""}
              {(eq.bands[index] ?? 0).toFixed(1)}
            </span>
          </div>
        ))}
      </div>

      {/* 总增益 / 声道平衡 */}
      <div className={`space-y-2 pt-2 border-t border-white/5 ${eq.enabled ? "" : "opacity-40 pointer-events-none"}`}>
        <div className="flex items-center gap-2">
          <span className="w-16 text-[11px] text-slate-400">{t("music.eqGain")}</span>
          <input
            type="range"
            min={-12}
            max={12}
            step={0.5}
            value={eq.gain_db}
            onChange={(e) => onChange({ ...eq, gain_db: Number(e.target.value) })}
            className="flex-1 accent-[var(--module-accent)] cursor-pointer"
          />
          <span className="w-12 text-[10px] font-mono text-slate-300 text-right">
            {eq.gain_db > 0 ? "+" : ""}
            {eq.gain_db.toFixed(1)}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-16 text-[11px] text-slate-400">{t("music.eqBalance")}</span>
          <span className="text-[10px] text-slate-500">L</span>
          <input
            type="range"
            min={-1}
            max={1}
            step={0.05}
            value={eq.balance}
            onChange={(e) => onChange({ ...eq, balance: Number(e.target.value) })}
            onDoubleClick={() => onChange({ ...eq, balance: 0 })}
            className="flex-1 accent-[var(--module-accent)] cursor-pointer"
          />
          <span className="text-[10px] text-slate-500">R</span>
          <span className="w-12 text-[10px] font-mono text-slate-300 text-right">
            {eq.balance === 0 ? t("music.eqCenter") : eq.balance.toFixed(2)}
          </span>
        </div>
      </div>

      <p className="text-[10px] text-slate-500 leading-snug">{t("music.eqHint")}</p>
    </div>
  );
}
