import { useTranslation } from "react-i18next";
import { History } from "lucide-react";
import type { LastLaunchConfig, DetectedAiTool } from "../../contracts/ai";
import { formatRelativeTime } from "./ToolLauncher.model";

interface ToolLastLaunchSummaryProps {
  lastLaunchConfigs: Record<string, LastLaunchConfig>;
  selectedTool: DetectedAiTool;
}

/** Pure ToolLastLaunchSummary view; its owning panel retains side effects and state. */
export function ToolLastLaunchSummary({ lastLaunchConfigs, selectedTool }: ToolLastLaunchSummaryProps) {
  const { t } = useTranslation();
  return (
    <div className="mt-2 px-2 py-1.5 rounded-ctl bg-slate-800/50 border border-white/5">
      <div className="flex items-center gap-1 mb-1">
        <History className="w-3 h-3 text-slate-500" />
        <span className="text-micro text-slate-500 font-semibold">{t("toollaunch.lastLaunch")}</span>
        {lastLaunchConfigs[selectedTool.id].lastLaunchedAt && (
          <span className="text-micro text-slate-600 ml-auto">
            {formatRelativeTime(lastLaunchConfigs[selectedTool.id].lastLaunchedAt, t)}
          </span>
        )}
      </div>
      <div className="flex flex-wrap gap-x-2 gap-y-0.5 text-micro">
        {lastLaunchConfigs[selectedTool.id].useOfficialModel ? (
          <span className="text-slate-400">{t("toollaunch.officialModel")}</span>
        ) : (
          <>
            <span className="text-slate-400">
              {lastLaunchConfigs[selectedTool.id].providerName || lastLaunchConfigs[selectedTool.id].providerId || "-"}
            </span>
            {lastLaunchConfigs[selectedTool.id].modelId && (
              <span className="text-[color-mix(in_srgb,var(--module-accent)_80%,transparent)] truncate max-w-[120px]" title={lastLaunchConfigs[selectedTool.id].modelId ?? undefined}>
                {lastLaunchConfigs[selectedTool.id].modelId}
              </span>
            )}
            {lastLaunchConfigs[selectedTool.id].fallbackModelId && (
              <span className="text-amber-400/80 truncate max-w-[120px]" title={t("toollaunch.fallbackModel", { name: lastLaunchConfigs[selectedTool.id].fallbackModelId })}>
                ※ {lastLaunchConfigs[selectedTool.id].fallbackModelId}
              </span>
            )}
          </>
        )}
        {lastLaunchConfigs[selectedTool.id].masqueradeModel && (
          <span className="text-cyan-400/60" title={t("toollaunch.masquerade")}>
            🎭 {lastLaunchConfigs[selectedTool.id].masqueradeModel}
          </span>
        )}
        {lastLaunchConfigs[selectedTool.id].oneMContext && (
          <span className="text-emerald-400/60" title={t("toollaunch.oneM")}>1M</span>
        )}
      </div>
    </div>
  );
}
