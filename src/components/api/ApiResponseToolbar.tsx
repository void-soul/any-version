import type { SendRequestOutput, ExtractResult } from "./types";
import { useTranslation } from "react-i18next";
import { Copy, Check, Terminal, FileDown } from "lucide-react";
import { fmtTime } from "./panelParts";
import type { Dispatch, SetStateAction } from "react";

interface ApiResponseToolbarProps {
  response: SendRequestOutput;
  setBodyMode: Dispatch<SetStateAction<"pretty" | "raw">>;
  bodyMode: "pretty" | "raw";
  copyBody: () => Promise<void>;
  copied: boolean;
  saveResponseBody: () => Promise<void>;
  copyAsCurl: () => Promise<void>;
  curlCopied: boolean;
  extractResults: ExtractResult[];
}

/** Pure ApiResponseToolbar view; its owning panel retains side effects and state. */
export function ApiResponseToolbar({ response, setBodyMode, bodyMode, copyBody, copied, saveResponseBody, copyAsCurl, curlCopied, extractResults }: ApiResponseToolbarProps) {
  const { t } = useTranslation();
  return (
    <>
          <span className="text-tiny text-slate-500">{fmtTime(response.time_ms)}</span>
      <span className="text-tiny text-slate-500">{response.size_bytes > 1024 * 1024 ? `${(response.size_bytes / 1024 / 1024).toFixed(1)}MB` : `${(response.size_bytes / 1024).toFixed(1)}KB`}</span>
      {response.body_truncated && <span className="text-tiny text-amber-400">{t("api.bodyTruncated")}</span>}
      <div className="ml-auto flex items-center gap-2">
        <div className="flex gap-0.5 bg-black/30 rounded p-0.5">
          {(["pretty", "raw"] as const).map((m) => (
            <button
              key={m}
              onClick={() => setBodyMode(m)}
              className={`px-1.5 py-0.5 rounded text-tiny cursor-pointer ${bodyMode === m ? "bg-white/10 text-cyan-300" : "text-slate-500 hover:text-slate-300"}`}
            >
              {m === "pretty" ? t("api.pretty") : t("api.raw")}
            </button>
          ))}
        </div>
        <button onClick={copyBody} className="flex items-center gap-1 text-tiny px-2 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
          {copied ? <Check className="w-3 h-3 text-emerald-400" /> : <Copy className="w-3 h-3" />} {copied ? t("api.copied") : t("api.copyBody")}
        </button>
        <button onClick={saveResponseBody} title={t("api.saveBodyTip")} className="flex items-center gap-1 text-tiny px-2 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
          <FileDown className="w-3 h-3" /> {t("api.saveBody")}
        </button>
        <button onClick={copyAsCurl} title={t("api.copyCurlTip")} className="flex items-center gap-1 text-tiny px-2 py-0.5 rounded bg-white/5 hover:bg-white/10 text-slate-400 cursor-pointer">
          {curlCopied ? <Check className="w-3 h-3 text-emerald-400" /> : <Terminal className="w-3 h-3" />} {curlCopied ? t("api.copiedCurl") : t("api.copyCurl")}
        </button>
        {extractResults.length > 0 && (
          <span className="text-tiny text-emerald-400" title={t("api.extractBadgeTip")}>
            {t("api.extractBadge", { n: extractResults.filter((r) => r.ok).length })}
          </span>
        )}
      </div>
    </>
  );
}
