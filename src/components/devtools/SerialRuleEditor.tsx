import { useTranslation } from "react-i18next";
import { Trash2 } from "lucide-react";
import { type SimRule } from "./serialUtils";

interface SerialRuleEditorProps {
  rules: SimRule[];
  updateRule: (i: number, patch: Partial<SimRule>) => void;
  selectCls: string;
  inputCls: string;
  removeRule: (i: number) => void;
}


/** Pure SerialRuleEditor view; its owning panel retains side effects and state. */
export function SerialRuleEditor({ rules, updateRule, selectCls, inputCls, removeRule }: SerialRuleEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1.5 max-h-44 overflow-auto pr-1">
      {rules.map((r, i) => (
        <div key={i} className="grid grid-cols-[7rem_1fr_9rem_5rem_2rem] gap-2 items-start">
          <select
            value={r.match_type}
            onChange={(e) => updateRule(i, { match_type: e.target.value as SimRule["match_type"] })}
            className={selectCls}
          >
            <option value="contains">{t("serial.optContains")}</option>
            <option value="prefix">{t("serial.optPrefix")}</option>
            <option value="exact">{t("serial.optExact")}</option>
            <option value="regex">{t("serial.optRegex")}</option>
          </select>
          <div className="space-y-1">
            <textarea
              value={r.pattern}
              onChange={(e) => updateRule(i, { pattern: e.target.value })}
              rows={1}
              placeholder={r.pattern_hex ? t("serial.hexPh") : t("serial.textPh")}
              className={`${inputCls} w-full resize-y min-h-[2rem]`}
            />
            <label className="flex items-center gap-1 text-caption text-slate-500 cursor-pointer">
              <input type="checkbox" checked={r.pattern_hex} onChange={(e) => updateRule(i, { pattern_hex: e.target.checked })} className="accent-[var(--module-accent)]" /> {t("serial.hexMatch")}
            </label>
          </div>
          <div className="space-y-1">
            <textarea
              value={r.response}
              onChange={(e) => updateRule(i, { response: e.target.value })}
              rows={1}
              placeholder={r.response_hex ? t("serial.respHexPh") : t("serial.respTextPh")}
              className={`${inputCls} w-full resize-y min-h-[2rem]`}
            />
            <div className="flex items-center gap-2 text-caption text-slate-500">
              <label className="flex items-center gap-1 cursor-pointer">
                <input type="checkbox" checked={r.response_hex} onChange={(e) => updateRule(i, { response_hex: e.target.checked })} className="accent-[var(--module-accent)]" /> HEX
              </label>
              <label className="flex items-center gap-1 cursor-pointer" title={t("serial.appendNlTip")}>
                <input type="checkbox" checked={r.append_newline} onChange={(e) => updateRule(i, { append_newline: e.target.checked })} className="accent-[var(--module-accent)]" /> \n
              </label>
            </div>
          </div>
          <input
            type="number"
            min={0}
            step={10}
            value={r.delay_ms}
            onChange={(e) => updateRule(i, { delay_ms: Math.max(0, Number(e.target.value) || 0) })}
            className={`${inputCls} w-full`}
          />
          <button
            onClick={() => removeRule(i)}
            disabled={rules.length <= 1}
            title={t("serial.delRule")}
            className="p-1.5 rounded text-slate-500 hover:text-red-400 hover:bg-white/10 disabled:opacity-30 cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
