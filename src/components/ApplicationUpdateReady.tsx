import { ExternalLink, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

interface ApplicationUpdateReadyProps {
  latestVersion: string;
  updateBody: string | null;
  updateSource: "github" | "plugin" | null;
  handleInstallUpdate: () => Promise<void>;
  installing: boolean;
  handleDownloadUpdate: () => void;
}

/** Pure ApplicationUpdateReady view; its owning panel retains side effects and state. */
export function ApplicationUpdateReady({ latestVersion, updateBody, updateSource, handleInstallUpdate, installing, handleDownloadUpdate }: ApplicationUpdateReadyProps) {
  const { t } = useTranslation();
  return (
    <div className="p-4 bg-emerald-500/5 border border-emerald-500/15 rounded-card space-y-2">
      <div className="flex items-center gap-2">
        <span className="text-body font-semibold text-emerald-300">
          {t("settings.newVersionFound", { version: latestVersion })}
        </span>
        {updateBody && (
          <span className="text-tiny text-slate-400">
            ({updateBody.substring(0, 80)}...)
          </span>
        )}
      </div>
      {updateSource === "plugin" ? (
        <button
          onClick={handleInstallUpdate}
          disabled={installing}
          className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white rounded-ctl text-body font-semibold cursor-pointer transition-all flex items-center gap-1.5 disabled:opacity-60"
        >
          <Loader2
            className={`w-3 h-3 ${installing ? "animate-spin" : ""}`}
          />
          {installing ? t("settings.installingUpdate") : t("settings.downloadInstall")}
        </button>
      ) : (
        <button
          onClick={handleDownloadUpdate}
          className="px-4 py-2 bg-emerald-600 hover:bg-emerald-500 text-white rounded-ctl text-body font-semibold cursor-pointer transition-all flex items-center gap-1.5"
        >
          <ExternalLink className="w-3 h-3" />
          {t("settings.gotoDownloadPage")}
        </button>
      )}
    </div>
  );
}
