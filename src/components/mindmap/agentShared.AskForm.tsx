import { useState } from "react";
import { Send, Sparkles } from "lucide-react";
import { type AskField } from "./agentShared.state";
export function AskForm({ ask, onSubmit, t }: {
  ask: { at: number; question: string; fields: AskField[]; round: number; max: number };
  onSubmit: (answer: Record<string, unknown> | string) => void;
  t: (k: string, o?: any) => string;
}) {
  const [values, setValues] = useState<Record<string, string>>(() => {
    const init: Record<string, string> = {};
    for (const f of ask.fields) init[f.key] = f.default ?? "";
    return init;
  });
  const [freeText, setFreeText] = useState("");
  const setField = (key: string, v: string) => setValues((p) => ({ ...p, [key]: v }));
  const submit = () => {
    const hasFields = ask.fields.length > 0;
    const answer: Record<string, unknown> = { ...values };
    if (!hasFields) answer.text = freeText;
    onSubmit(answer);
  };
  const canSubmit = ask.fields.length === 0 ? true : true; // 允许留空（后端按未填写兜底）
  void canSubmit;
  return (
    <div className="rounded-ctl border border-amber-400/30 bg-amber-400/[0.05] p-2.5">
      <div className="mb-1.5 flex items-center gap-1.5">
        <span className="inline-flex h-4 w-4 items-center justify-center rounded-full bg-amber-400/20 text-amber-300">
          <Sparkles className="h-3 w-3" />
        </span>
        <span className="text-tiny font-semibold text-amber-200">{t("agent.askTitle")}</span>
        <span className="ml-auto text-[8px] text-amber-300/70">{t("agent.askRound", { n: ask.round, max: ask.max })}</span>
      </div>
      {ask.question && <p className="mb-2 text-tiny leading-4 text-slate-200">{ask.question}</p>}
      <div className="space-y-2">
        {ask.fields.map((f) => (
          <div key={f.key}>
            <label className="mb-0.5 block text-micro font-medium text-slate-400">{f.label}</label>
            {f.type === "select" ? (
              <select value={values[f.key] ?? ""} onChange={(e) => setField(f.key, e.target.value)}
                className="h-7 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-tiny text-slate-200 outline-none focus:border-amber-400/60">
                <option value="">{t("agent.askPick")}</option>
                {(f.options ?? []).map((o) => <option key={o} value={o}>{o}</option>)}
              </select>
            ) : f.type === "textarea" ? (
              <textarea value={values[f.key] ?? ""} onChange={(e) => setField(f.key, e.target.value)} rows={2}
                className="w-full resize-none rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-tiny text-slate-200 outline-none focus:border-amber-400/60" />
            ) : (
              <input value={values[f.key] ?? ""} onChange={(e) => setField(f.key, e.target.value)}
                className="h-7 w-full rounded-md border border-white/10 bg-slate-950/70 px-2 text-tiny text-slate-200 outline-none focus:border-amber-400/60" />
            )}
          </div>
        ))}
        {ask.fields.length === 0 && (
          <textarea value={freeText} onChange={(e) => setFreeText(e.target.value)} rows={2}
            placeholder={t("agent.askFreePh")}
            className="w-full resize-none rounded-md border border-white/10 bg-slate-950/70 px-2 py-1.5 text-tiny text-slate-200 outline-none focus:border-amber-400/60" />
        )}
      </div>
      <div className="mt-2 flex justify-end">
        <button type="button" onClick={submit}
          className="inline-flex cursor-pointer items-center gap-1 rounded-md border border-amber-400/40 bg-amber-400/15 px-3 py-1 text-tiny font-semibold text-amber-100 transition hover:bg-amber-400/25">
          <Send className="h-3 w-3" />{t("agent.askSubmit")}
        </button>
      </div>
    </div>
  );
}
