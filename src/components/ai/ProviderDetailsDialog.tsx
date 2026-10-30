import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { RefreshCw, X, ExternalLink } from "lucide-react";
import type { AiProvider } from "../../contracts/ai";
import type { Dispatch, SetStateAction } from "react";

interface ProviderDetailsDialogProps {
  detailProvider: AiProvider;
  setDetailId: Dispatch<SetStateAction<string | null>>;
  BALANCE_CAPABLE: Set<string>;
  runBalance: (provider: AiProvider) => Promise<void>;
  balanceState: { pid: string; loading: boolean; error: string | null; items: { key: string; value: string; }[]; } | null;
  balanceLabel: (key: string) => string;
  openEditModal: (provider: AiProvider) => void;
}


/** Pure ProviderDetailsDialog view; its owning panel retains side effects and state. */
export function ProviderDetailsDialog({ detailProvider, setDetailId, BALANCE_CAPABLE, runBalance, balanceState, balanceLabel, openEditModal }: ProviderDetailsDialogProps) {
  const { t } = useTranslation();
  return (
    <div className="w-full max-w-md bg-slate-950/95 border border-white/10 rounded-panel shadow-2xl flex flex-col max-h-[85vh] overflow-hidden" onClick={e => e.stopPropagation()}>
      {/* Header */}
      <div className="p-4 border-b border-white/5 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <h3 className="text-body font-bold text-slate-200 truncate">{detailProvider.name}</h3>
          <span className={`px-1.5 py-0.5 rounded text-[8px] font-bold flex-shrink-0 ${detailProvider.category === "relay" ? "bg-cyan-500/15 text-cyan-400" : detailProvider.category === "local" ? "bg-[var(--module-accent)]/15 text-[var(--module-accent)]" : "bg-emerald-500/15 text-emerald-400"}`}>
            {detailProvider.category === "relay" ? t("modelcfg.relay") : detailProvider.category === "local" ? t("modelcfg.local") : t("modelcfg.vendor")}
          </span>
          {detailProvider.website && (
            <a href={detailProvider.website} target="_blank" rel="noopener noreferrer"
              onClick={(e) => { e.preventDefault(); void openUrl(detailProvider.website); }}
              className="text-blue-400 hover:text-blue-300 transition-colors flex-shrink-0" title={t("modelcfg.openSite")}>
              <ExternalLink className="w-3 h-3" />
            </a>
          )}
        </div>
        <button onClick={() => setDetailId(null)} className="text-slate-500 hover:text-slate-300 cursor-pointer flex-shrink-0"><X className="w-4 h-4" /></button>
      </div>

      {/* Body */}
      <div className="flex-grow overflow-y-auto p-4 space-y-4">
        {/* 余额 */}
        {BALANCE_CAPABLE.has(detailProvider.id) && (
          <div className="p-3 rounded-ctl bg-slate-900/50 border border-white/5 space-y-2">
            <div className="flex items-center justify-between">
              <label className="text-tiny text-slate-400 font-semibold">{t("modelcfg.balance")}</label>
              <button onClick={() => void runBalance(detailProvider)} disabled={balanceState?.pid === detailProvider.id && balanceState.loading}
                className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-micro font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-40 disabled:cursor-not-allowed">
                <RefreshCw className={`w-3 h-3 ${balanceState?.pid === detailProvider.id && balanceState.loading ? "animate-spin" : ""}`} />
                {t("modelcfg.balanceQuery")}
              </button>
            </div>
            {balanceState?.pid === detailProvider.id && balanceState.loading && (
              <p className="text-tiny text-slate-500">{t("modelcfg.loading")}</p>
            )}
            {balanceState?.pid === detailProvider.id && balanceState.error && (
              <p className="text-tiny text-red-400 whitespace-pre-line">{balanceState.error}</p>
            )}
            {balanceState?.pid === detailProvider.id && !balanceState.loading && !balanceState.error && (
              <div className="space-y-1">
                {balanceState.items.map((it) => (
                  <div key={it.key} className="flex items-center justify-between text-tiny">
                    <span className="text-slate-500">{balanceLabel(it.key)}</span>
                    <span className="font-mono text-slate-200">{it.value === "ok" ? t("modelcfg.balOk") : it.value === "disabled" ? t("modelcfg.balDisabled") : it.value}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* 协议端点 */}
        <div className="p-3 rounded-ctl bg-slate-900/50 border border-white/5 space-y-1.5">
          <label className="text-tiny text-slate-400 font-semibold">{t("modelcfg.endpoints")}</label>
          {([
            [t("modelcfg.openaiUrl"), detailProvider.openai_url, "text-blue-300", detailProvider.openai_include_v1 ?? null],
            [t("modelcfg.anthropicUrl"), detailProvider.anthropic_url, "text-amber-300", detailProvider.anthropic_include_v1 ?? null],
            [t("modelcfg.googleUrl"), detailProvider.google_url, "text-green-300", null],
          ] as const).map(([label, url, cls, includeV1]) => url ? (
            <div key={label} className="flex items-start gap-2 text-tiny">
              <span className={`${cls} font-semibold flex-shrink-0 w-24`}>{label}</span>
              <span className="font-mono text-slate-400 break-all">{url}</span>
              {/* 只有显式改过才标出来：「自动」是默认行为，写出来反而像配置项丢了 */}
              {includeV1 !== null && (
                <span className="flex-shrink-0 px-1 py-px rounded bg-white/5 text-[8px] text-slate-400">
                  /v1 {includeV1 ? t("modelcfg.v1Yes") : t("modelcfg.v1No")}
                </span>
              )}
            </div>
          ) : null)}
        </div>

        {/* 模型列表 */}
        <div>
          <label className="text-tiny text-slate-500 font-semibold block mb-1.5">
            {t("modelcfg.modelList", { count: detailProvider.models.length })}
          </label>
          {detailProvider.models.length === 0 ? (
            <div className="text-tiny text-slate-600 py-2 text-center">{t("modelcfg.noModelsHint")}</div>
          ) : (
            <div className="max-h-48 overflow-y-auto rounded-ctl border border-white/5 divide-y divide-white/[0.03]">
              {detailProvider.models.map((model) => (
                <div key={model.id} className="px-2.5 py-1 text-tiny bg-white/[0.02]">
                  <span className="font-mono text-slate-300">{model.id}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* Footer */}
      <div className="p-4 border-t border-white/5 bg-slate-900/20 flex justify-end gap-2">
        <button onClick={() => { setDetailId(null); openEditModal(detailProvider); }}
          className="px-3 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-slate-300 hover:text-white text-tiny font-semibold cursor-pointer">{t("modelcfg.edit")}</button>
        <button onClick={() => setDetailId(null)}
          className="px-3 py-1.5 rounded-ctl bg-white/5 border border-white/10 text-slate-400 hover:text-slate-200 text-tiny font-semibold cursor-pointer">{t("modelcfg.cancel")}</button>
      </div>
    </div>
  );
}
