import type { ExtractRule, ExtractResult } from "./types";
import { useTranslation } from "react-i18next";
import { Plus, Trash2, Check, X } from "lucide-react";
import type { ApiEndpoint } from "./types";

interface ApiExtractRulesEditorProps {
  draft: ApiEndpoint;
  updateExtractRule: (index: number, patch: Partial<ExtractRule>) => void;
  removeExtractRule: (index: number) => void;
  addExtractRule: () => void;
  extractResults: ExtractResult[];
}

/** Pure ApiExtractRulesEditor view; its owning panel retains side effects and state. */
export function ApiExtractRulesEditor({ draft, updateExtractRule, removeExtractRule, addExtractRule, extractResults }: ApiExtractRulesEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="space-y-2">
      <div className="text-tiny text-slate-500 leading-relaxed">{t("api.extractHint")}</div>
      <div className="space-y-1.5">
        {(draft.extract_rules ?? []).map((rule, i) => (
          <div key={i} className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={rule.enabled}
              onChange={(e) => updateExtractRule(i, { enabled: e.target.checked })}
              className="accent-[var(--module-accent)]"
            />
            <input
              value={rule.path}
              onChange={(e) => updateExtractRule(i, { path: e.target.value })}
              placeholder={t("api.extractPathPh")}
              className="flex-1 min-w-0 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-caption font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
            />
            <span className="text-tiny text-slate-500">→</span>
            <input
              value={rule.variable}
              onChange={(e) => updateExtractRule(i, { variable: e.target.value })}
              placeholder={t("api.extractVarPh")}
              className="w-32 bg-black/30 border border-white/10 rounded-md px-2 py-1 text-caption font-mono text-slate-200 focus:outline-none focus:border-[var(--module-accent)]/60"
            />
            <button onClick={() => removeExtractRule(i)} className="p-1 text-slate-500 hover:text-rose-400 cursor-pointer">
              <Trash2 className="w-3 h-3" />
            </button>
          </div>
        ))}
        {(draft.extract_rules ?? []).length === 0 && (
          <div className="text-caption text-slate-600">{t("api.extractEmpty")}</div>
        )}
      </div>
      <button onClick={addExtractRule} className="flex items-center gap-1 text-tiny px-2 py-1 rounded bg-white/5 hover:bg-white/10 text-slate-300 cursor-pointer">
        <Plus className="w-3 h-3" /> {t("api.extractAddRule")}
      </button>
      {extractResults.length > 0 && (
        <div className="rounded-ctl border border-white/10 bg-black/20 p-2 space-y-1">
          <div className="text-tiny font-semibold text-slate-400">{t("api.extractResultTitle")}</div>
          {extractResults.map((r, i) => (
            <div key={i} className="flex items-center gap-1.5 text-tiny min-w-0">
              {r.ok ? <Check className="w-3 h-3 shrink-0 text-emerald-400" /> : <X className="w-3 h-3 shrink-0 text-rose-400" />}
              <code className="shrink-0 text-slate-300">{r.variable}</code>
              <span className="text-slate-600">=</span>
              <code className={`truncate ${r.ok ? "text-emerald-300" : "text-rose-300"}`} title={r.value}>
                {r.ok ? r.value : t("api.extractMissing")}
              </code>
            </div>
          ))}
          <div className="text-micro text-slate-500">{t("api.extractSaved")}</div>
        </div>
      )}
    </div>
  );
}
