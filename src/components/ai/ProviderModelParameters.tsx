import { useTranslation } from "react-i18next";
import type { ModelCustomParam } from "../../contracts/ai";
import { PARAM_PRESETS } from "./paramPresets";

interface ProviderModelParametersProps {
  modelsText: string;
  modelParams: Record<string, ModelCustomParam[]>;
  updateModelParam: (mid: string, idx: number, patch: Partial<ModelCustomParam>) => void;
  removeModelParam: (mid: string, idx: number) => void;
  addModelParam: (mid: string) => void;
  addModelParamFromPreset: (mid: string, preset: ModelCustomParam) => void;
}


/** Pure ProviderModelParameters view; its owning panel retains side effects and state. */
export function ProviderModelParameters({ modelsText, modelParams, updateModelParam, removeModelParam, addModelParam, addModelParamFromPreset }: ProviderModelParametersProps) {
  const { t } = useTranslation();
  return (
    <div className="rounded-ctl border border-white/5 bg-slate-900/30 p-3 space-y-3">
      <div className="text-tiny text-slate-500 font-semibold">
        {t("modelcfg.customParams")}
        <span className="text-slate-600 font-normal">{t("modelcfg.customParamsHint")}</span>
      </div>
      {modelsText.split("\n").map(l => l.trim()).filter(Boolean).map((mid) => (
        <div key={mid} className="rounded-md border border-white/5 bg-slate-900/40 p-2.5">
          <div className="text-tiny text-[var(--module-accent)] font-mono mb-2">{mid}</div>
          {(modelParams[mid] || []).map((cp, ci) => (
            <div key={ci} className="mb-2 p-2 rounded bg-slate-800/40 border border-white/5 space-y-1.5">
              <div className="flex gap-1.5">
                <input value={cp.label} onChange={e => updateModelParam(mid, ci, { label: e.target.value })}
                  placeholder={t("modelcfg.paramNamePh")} className="w-37 min-w-0 ui-input rounded px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
                <input value={cp.key} onChange={e => updateModelParam(mid, ci, { key: e.target.value })}
                  placeholder={t("modelcfg.paramKeyPh")} className="flex-1 min-w-0 ui-input rounded px-2 py-1 text-tiny text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                <button onClick={() => removeModelParam(mid, ci)}
                  className="shrink-0 w-6 h-6 flex items-center justify-center rounded bg-red-500/10 hover:bg-red-500/20 text-caption text-red-400">×</button>
              </div>
              <div className="flex gap-1.5 items-stretch">
                <div className="flex items-center gap-1.5 shrink-0 rounded-md border border-cyan-500/20 bg-cyan-500/5 px-2 py-1">
                  <div className="flex items-center gap-1 mr-2">
                    {([["enum", t("modelcfg.paramEnum")],["text", t("modelcfg.paramText")],["bool", t("modelcfg.paramBool")]] as const).map(([v,l]) => (
                      <button key={v} type="button" onClick={() => updateModelParam(mid, ci, { paramType: v })}
                        className={`px-2 py-0.5 rounded-full text-tiny border transition-colors ${cp.paramType === v ? "bg-cyan-500/20 border-cyan-500 text-cyan-200" : "bg-slate-900 border-white/10 text-slate-400 hover:border-white/20"}`}>
                        {l}
                      </button>
                    ))}
                  </div>
                </div>
                {cp.paramType === "enum" && (
                  <input value={(cp.options || []).join(",")} onChange={e => updateModelParam(mid, ci, { options: e.target.value.split(",").map(s => s.trim()).filter(Boolean) })}
                    placeholder={t("modelcfg.paramValuesPh")} className="flex-1 min-w-0 ui-input rounded px-2 py-1 text-tiny text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
                )}
                <input value={cp.defaultValue || ""} onChange={e => updateModelParam(mid, ci, { defaultValue: e.target.value })}
                  placeholder={t("modelcfg.paramDefaultPh")} className="w-24 ui-input rounded px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-[var(--module-accent)]" />
              </div>
              <div className="flex gap-1.5 items-stretch">
                <div className="flex items-center gap-1.5 shrink-0 rounded-md border border-amber-500/20 bg-amber-500/5 px-2 py-1">
                  <div className="flex items-center gap-1">
                    {([["env", t("modelcfg.paramEnv")],["config", t("modelcfg.paramConfig")]] as const).map(([v,l]) => (
                      <button key={v} type="button" onClick={() => updateModelParam(mid, ci, { target: v })}
                        className={`px-2 py-0.5 rounded-full text-tiny border transition-colors ${cp.target === v ? "bg-amber-500/20 border-amber-500 text-amber-200" : "bg-slate-900 border-white/10 text-slate-400 hover:border-white/20"}`}>
                        {l}
                      </button>
                    ))}
                  </div>
                </div>
                <input value={cp.target === "config" ? (cp.configPath || "") : (cp.envKey || "")}
                  onChange={e => cp.target === "config"
                    ? updateModelParam(mid, ci, { configPath: e.target.value })
                    : updateModelParam(mid, ci, { envKey: e.target.value })}
                  placeholder={cp.target === "config" ? t("modelcfg.paramTargetPh") : t("modelcfg.paramEnvPh")}
                  className="flex-1 min-w-0 ui-input rounded px-2 py-1 text-tiny text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]" />
              </div>
            </div>
          ))}
          <div className="flex items-center gap-2">
            <button onClick={() => addModelParam(mid)}
              className="text-tiny text-[var(--module-accent)] hover:text-[var(--module-accent-strong)] cursor-pointer">{t("modelcfg.addParam")}</button>
            {/* 从预设添加：把高频参数做成模板，选中即生成一条完整参数（见 paramPresets.ts） */}
            <select
              value=""
              onChange={(e) => {
                const id = e.target.value;
                if (!id) return;
                const preset = PARAM_PRESETS.find(p => p.id === id);
                if (preset) addModelParamFromPreset(mid, preset.param);
              }}
              className="text-tiny text-slate-400 bg-slate-900 border border-white/10 rounded px-1.5 py-0.5 focus:outline-none focus:border-[var(--module-accent)] cursor-pointer"
            >
              <option value="">{t("modelcfg.presetAdd")}</option>
              {PARAM_PRESETS.map(p => (
                <option key={p.id} value={p.id}>{t(p.nameKey)}（{t(p.scopeKey)}）</option>
              ))}
            </select>
          </div>
        </div>
      ))}
    </div>
  );
}
