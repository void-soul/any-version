import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, ShieldCheck } from "lucide-react";
import type { ProjectStatus, ProjectDef } from "./types";
import { categoryLabel } from "./types";

interface ProjectIdentityHeaderProps {
  status: ProjectStatus;
  def: ProjectDef | null;
  handleSelectCustomPath: () => Promise<void>;
  handleClearCustomPath: () => Promise<void>;
}

/** Pure ProjectIdentityHeader view; its owning panel retains side effects and state. */
export function ProjectIdentityHeader({ status, def, handleSelectCustomPath, handleClearCustomPath }: ProjectIdentityHeaderProps) {
  const { t } = useTranslation();
  return (
    <div>
      <div className="flex items-center gap-2">
        <h3 className="text-base font-semibold text-white">{status.display_name}</h3>
        <span className={`px-1.5 py-0.5 rounded text-micro font-semibold border ${
          status.category === "language"
            ? "bg-blue-500/10 text-blue-400 border-blue-500/20"
            : status.category === "tool"
            ? "bg-amber-500/10 text-amber-400 border-amber-500/20"
            : "bg-[var(--module-accent-soft)] text-[var(--module-accent)] border-[var(--module-accent-ring)]"
        }`}>
          {categoryLabel(status.category)}
        </span>
        {status.managed ? (
          status.is_simple_managed ? (
            <span className="px-1.5 py-0.5 rounded text-micro bg-amber-500/10 text-amber-400 border border-amber-500/20 font-bold flex items-center gap-0.5">
              <ShieldCheck className="w-2.5 h-2.5" /> {t("projdetail.simpleManaging")}
            </span>
          ) : (
            <span className="px-1.5 py-0.5 rounded text-micro bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-bold flex items-center gap-0.5">
              <ShieldCheck className="w-2.5 h-2.5" /> {t("projdetail.managing")}
            </span>
          )
        ) : (
          <span className="px-1.5 py-0.5 rounded text-micro bg-slate-500/10 text-slate-400 border border-slate-500/20 font-medium">
            {t("projdetail.notManaged")}
          </span>
        )}
        {status.installed ? (
          status.active_version ? (
            <span className="px-1.5 py-0.5 rounded text-micro font-mono font-bold bg-emerald-500/15 text-emerald-400 border border-emerald-500/25">
              v{status.active_version}
            </span>
          ) : (
            <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-amber-500/10 text-amber-400 border border-amber-500/20">
              {t("projdetail.installed")}
            </span>
          )
        ) : (
          <span className="px-1.5 py-0.5 rounded text-micro font-semibold bg-red-500/10 text-red-400 border border-red-500/20">
            {t("projdetail.notInstalled")}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2 mt-0.5 flex-wrap">
        {def?.official_website && (
          <button onClick={() => openUrl(def.official_website)} className="text-tiny text-blue-400 hover:text-blue-300 transition-colors flex items-center gap-0.5 cursor-pointer mr-1">
            {t("projdetail.officialSite")} <ExternalLink className="w-2.5 h-2.5" />
          </button>
        )}
        {status.install_source && (
          <><span className="text-slate-600 text-tiny">.</span><span className="text-tiny text-slate-400">{t("projdetail.installMethod")}: {status.install_source}</span></>
        )}
        {status.install_root && (
          <><span className="text-slate-600 text-tiny">.</span><span className="text-tiny text-slate-400 font-mono truncate max-w-[300px]" title={status.install_root}>{t("projdetail.installPath")}: {status.install_root}</span></>
        )}
        {(!status.managed || status.is_simple_managed) && (!status.install_root || status.install_source === "手动指定") && (
          <>
            <span className="text-slate-600 text-tiny">.</span>
            <button
              onClick={handleSelectCustomPath}
              className="text-tiny ui-link transition-colors flex items-center gap-0.5 cursor-pointer font-medium"
            >
              {status.install_root ? t("projdetail.modifyPath") : t("projdetail.manualSpecifyDir")}
            </button>
            {status.install_source === "手动指定" && (
              <>
                <span className="text-slate-600 text-tiny">.</span>
                <button
                  onClick={handleClearCustomPath}
                  className="text-tiny text-amber-500 hover:text-amber-400 hover:underline transition-colors flex items-center gap-0.5 cursor-pointer font-medium"
                >
                  {t("projdetail.restoreAutoDetect")}
                </button>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}
