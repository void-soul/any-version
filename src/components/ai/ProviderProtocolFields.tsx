import { useTranslation } from "react-i18next";
import type { AiProvider } from "../../contracts/ai";
import { selectToV1, v1ToSelect } from "./ModelConfig.model";
import type { Dispatch, SetStateAction } from "react";

interface ProviderProtocolFieldsProps {
  form: AiProvider;
  setForm: Dispatch<SetStateAction<AiProvider>>;
}


/** Pure ProviderProtocolFields view; its owning panel retains side effects and state. */
export function ProviderProtocolFields({ form, setForm }: ProviderProtocolFieldsProps) {
  const { t } = useTranslation();
  return (
    <div className="p-3 rounded-ctl bg-slate-900/50 border border-white/5 space-y-3">
      <label className="text-tiny text-slate-400 font-semibold block">{t("modelcfg.endpoints")}</label>
      <p className="text-micro text-slate-600">{t("modelcfg.endpointsHint")}</p>

      <div className="space-y-1">
        <label className="text-micro text-blue-300 font-semibold block">{t("modelcfg.openaiUrl")}</label>
        <input value={form.openai_url} onChange={e => setForm({ ...form, openai_url: e.target.value })}
          placeholder="https://api.openai.com/v1"
          className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-blue-500" />
        {/* 兼容层差异：有的端点要 `{base}/v1/chat/completions`，有的是 `{base}/chat/completions` */}
        <div className="flex items-center gap-1.5">
          <span className="text-micro text-slate-500">{t("modelcfg.includeV1")}</span>
          <select value={v1ToSelect(form.openai_include_v1)}
            onChange={e => setForm({ ...form, openai_include_v1: selectToV1(e.target.value) })}
            className="ui-input rounded px-1.5 py-0.5 text-micro text-slate-300 cursor-pointer focus:outline-none focus:border-blue-500"
            title={t("modelcfg.includeV1Hint")}>
            <option value="auto">{t("modelcfg.v1Auto")}</option>
            <option value="yes">{t("modelcfg.v1Yes")}</option>
            <option value="no">{t("modelcfg.v1No")}</option>
          </select>
        </div>
      </div>

      <div className="space-y-1">
        <label className="text-micro text-amber-300 font-semibold block">{t("modelcfg.anthropicUrl")}</label>
        <input value={form.anthropic_url} onChange={e => setForm({ ...form, anthropic_url: e.target.value })}
          placeholder="https://api.anthropic.com"
          className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-amber-500" />
        <div className="flex items-center gap-1.5">
          <span className="text-micro text-slate-500">{t("modelcfg.includeV1")}</span>
          <select value={v1ToSelect(form.anthropic_include_v1)}
            onChange={e => setForm({ ...form, anthropic_include_v1: selectToV1(e.target.value) })}
            className="ui-input rounded px-1.5 py-0.5 text-micro text-slate-300 cursor-pointer focus:outline-none focus:border-amber-500"
            title={t("modelcfg.includeV1Hint")}>
            <option value="auto">{t("modelcfg.v1Auto")}</option>
            <option value="yes">{t("modelcfg.v1Yes")}</option>
            <option value="no">{t("modelcfg.v1No")}</option>
          </select>
        </div>
      </div>

      <div className="space-y-1">
        <label className="text-micro text-green-300 font-semibold block">{t("modelcfg.googleUrl")}</label>
        <input value={form.google_url} onChange={e => setForm({ ...form, google_url: e.target.value })}
          placeholder="https://generativelanguage.googleapis.com"
          className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-green-500" />
      </div>
    </div>
  );
}
