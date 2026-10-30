import { useState } from "react";
import { useTranslation } from "react-i18next";
// ─── 单条消息渲染 ───
export function PromptResponse({
  prompt,
  onRespond,
}: {
  prompt: { question: string; options: string[] };
  onRespond: (response: string) => void;
}) {
  const { t } = useTranslation();
  const [customResponse, setCustomResponse] = useState("");
  return (
    <div className="mt-2 rounded-ctl border border-amber-500/30 bg-amber-500/5 p-2.5 space-y-2">
      <div className="flex items-start gap-1.5">
        <span className="text-amber-400 text-tiny font-bold mt-0.5">{t("collab.askBadge")}</span>
        <span className="text-tiny text-slate-300 whitespace-pre-wrap break-words flex-1">
          {prompt.question}
        </span>
      </div>
      <div className="flex flex-wrap gap-1.5 items-center">
        {prompt.options.map((opt) => (
          <button
            key={opt}
            onClick={() => onRespond(opt)}
            className="px-2.5 py-1 rounded-md bg-amber-600/80 hover:bg-amber-500 text-white text-tiny font-semibold transition-colors cursor-pointer"
          >
            {opt}
          </button>
        ))}
        <input
          value={customResponse}
          onChange={(e) => setCustomResponse(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && customResponse.trim()) {
              onRespond(customResponse.trim());
              setCustomResponse("");
            }
          }}
          placeholder={t("collab.customReplyPh")}
          className="flex-1 min-w-[80px] bg-slate-800 border border-white/10 rounded-md px-2 py-1 text-tiny text-slate-200 focus:outline-none focus:border-amber-500"
        />
        {customResponse.trim() && (
          <button
            onClick={() => {
              onRespond(customResponse.trim());
              setCustomResponse("");
            }}
            className="px-2 py-1 rounded-md bg-slate-700 hover:bg-slate-600 text-slate-200 text-tiny cursor-pointer"
          >
            {t("collab.send")}
          </button>
        )}
      </div>
      <p className="text-[8px] text-slate-500">{t("collab.autoYesHint")}</p>
    </div>
  );
}
