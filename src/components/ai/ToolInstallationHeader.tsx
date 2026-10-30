import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CheckCircle, RefreshCw, Bot, ExternalLink, Trash2, Download } from "lucide-react";
import type { DetectedAiTool } from "../../contracts/ai";
import { PROTOCOL_LABELS } from "./ToolLauncher.model";

interface ToolInstallationHeaderProps {
  selectedTool: DetectedAiTool;
  getBusy: (toolId: string) => "installing" | "uninstalling" | "upgrading" | null;
  versionStatuses: Record<string, { latest: string; status: string; busy?: string | null | undefined; }>;
  handleUpgrade: (tool: DetectedAiTool) => Promise<void>;
  askUninstall: (tool: DetectedAiTool) => Promise<void>;
}

/** Pure ToolInstallationHeader view; its owning panel retains side effects and state. */
export function ToolInstallationHeader({ selectedTool, getBusy, versionStatuses, handleUpgrade, askUninstall }: ToolInstallationHeaderProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-3">
      <div className="p-2 rounded-ctl bg-[var(--module-accent-soft)]">
        <Bot className="w-5 h-5 text-[var(--module-accent)]" />
      </div>
      <div>
        <h3 className="text-sm font-bold text-white">{selectedTool.display_name}</h3>
        <div className="flex items-center gap-2 mt-0.5">
          {getBusy(selectedTool.id) && (
            <span className="flex items-center gap-1 text-tiny font-semibold text-blue-300">
              <RefreshCw className="w-3 h-3 animate-spin" />
              {getBusy(selectedTool.id) === "upgrading" ? `${t("toollaunch.upgrading")}...` : getBusy(selectedTool.id) === "installing" ? `${t("toollaunch.installing")}...` : `${t("toollaunch.uninstalling")}...`}
            </span>
          )}
          {selectedTool.installed ? (
            <>
              <span className="text-tiny text-emerald-400"><CheckCircle className="w-3 h-3 inline mr-0.5" />{selectedTool.version || t("toollaunch.installed")}</span>
              {!selectedTool.pm_managed && (
                // 只是提示，不拦操作：升级/卸载照旧可用（包管理器 → 官方渠道 → 按文件清理）
                <span
                  className="text-tiny text-amber-400/80 cursor-help"
                  title={selectedTool.detected_path || selectedTool.uninstall_cmd || selectedTool.upgrade_cmd || undefined}
                >
                  {t("toollaunch.externalInstallHint")}
                </span>
              )}
              {!getBusy(selectedTool.id) && versionStatuses[selectedTool.id]?.latest && versionStatuses[selectedTool.id]?.status === "outdated" && (
                <>
                  <span className="text-tiny text-amber-400 ml-1">→ {t("toollaunch.latest")}: {versionStatuses[selectedTool.id].latest}</span>
                  <button
                    onClick={() => handleUpgrade(selectedTool)}
                    disabled={getBusy(selectedTool.id) === "upgrading"}
                    className="px-2 py-0.5 rounded-md bg-emerald-500/10 hover:bg-emerald-500/20 text-micro font-semibold text-emerald-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-50"
                    title={t("toollaunch.upgradeLatest")}
                  >
                    <Download className={`w-3 h-3 ${getBusy(selectedTool.id) === "upgrading" ? "animate-spin" : ""}`} />
                    {getBusy(selectedTool.id) === "upgrading" ? `${t("toollaunch.upgrading")}...` : t("toollaunch.upgrade")}
                  </button>
                  <button
                    onClick={() => void askUninstall(selectedTool)}
                    disabled={getBusy(selectedTool.id) === "uninstalling"}
                    className="px-2 py-0.5 rounded-md bg-red-500/10 hover:bg-red-500/20 text-micro font-semibold text-red-400 cursor-pointer transition-all flex items-center gap-0.5 disabled:opacity-50"
                    title={t("toollaunch.uninstallTitle")}
                  >
                    <Trash2 className={`w-3 h-3 ${getBusy(selectedTool.id) === "uninstalling" ? "animate-spin" : ""}`} />
                    {getBusy(selectedTool.id) === "uninstalling" ? `${t("toollaunch.uninstalling")}...` : t("toollaunch.uninstall")}
                  </button>
                </>
              )}
            </>
          ) : (
            <span className="text-tiny text-slate-500">{t("toollaunch.notInstalled")}</span>
          )}
          <span className="text-tiny text-slate-500">· {selectedTool.api_protocol === "none" ? t("toollaunch.modelNone") : PROTOCOL_LABELS[selectedTool.api_protocol]}</span>
          <a href={selectedTool.website} target="_blank" rel="noopener noreferrer"
            onClick={(e) => { e.preventDefault(); void openUrl(selectedTool.website); }}
            className="text-tiny text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 ml-1"
            title={t("toollaunch.openSite")}>
            <ExternalLink className="w-3 h-3" /> {t("toollaunch.site")}
          </a>
        </div>
      </div>
    </div>
  );
}
