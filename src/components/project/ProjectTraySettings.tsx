import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Settings } from "lucide-react";
import type { ProjectStatus, ProjectDef } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface ProjectTraySettingsProps {
  setShowMenuConfig: Dispatch<SetStateAction<boolean>>;
  status: ProjectStatus;
  pid: string;
  refreshSingle: (id: string) => Promise<void>;
  def: ProjectDef | null;
  autoStartServices: string[];
  setAutoStartServices: Dispatch<SetStateAction<string[]>>;
}

/** Pure ProjectTraySettings view; its owning panel retains side effects and state. */
export function ProjectTraySettings({ setShowMenuConfig, status, pid, refreshSingle, def, autoStartServices, setAutoStartServices }: ProjectTraySettingsProps) {
  const { t } = useTranslation();
  return (
    <div className="mx-5 mt-4 p-4 glass-panel border border-white/5 rounded-panel bg-white/2 space-y-3 animate-fadeIn flex-shrink-0">
      <div className="flex items-center justify-between border-b border-white/5 pb-2">
        <span className="text-body font-semibold text-white flex items-center gap-1.5">
          <Settings className="w-3.5 h-3.5 text-[var(--module-accent)]" />
          {t("projdetail.trayConfigTitle")}
        </span>
        <button
          onClick={() => setShowMenuConfig(false)}
          className="text-tiny text-slate-400 hover:text-slate-200 cursor-pointer"
        >
          {t("projdetail.close")}
        </button>
      </div>
      <div className="flex flex-wrap gap-6 py-1">
        <label className="flex items-center gap-2 cursor-pointer text-body text-slate-300 select-none">
          <input
            type="checkbox"
            checked={status.show_version !== false}
            onChange={async (e) => {
              if (pid) {
                await invoke("update_project_menu_config", {
                  id: pid,
                  showVersion: e.target.checked,
                  showService: status.show_service !== false,
                });
                await refreshSingle(pid);
              }
            }}
            className="rounded border-white/10 bg-black/40 text-[var(--module-accent)] focus:ring-[var(--module-accent)] w-3.5 h-3.5 cursor-pointer"
          />
          {t("projdetail.showVersionSwitch")}
        </label>
        {(def?.category === "service" || def?.is_service) && (
          <>
            <label className="flex items-center gap-2 cursor-pointer text-body text-slate-300 select-none">
              <input
                type="checkbox"
                checked={status.show_service !== false}
                onChange={async (e) => {
                  if (pid) {
                    await invoke("update_project_menu_config", {
                      id: pid,
                      showVersion: status.show_version !== false,
                      showService: e.target.checked,
                    });
                    await refreshSingle(pid);
                  }
                }}
                className="rounded border-white/10 bg-black/40 text-[var(--module-accent)] focus:ring-[var(--module-accent)] w-3.5 h-3.5 cursor-pointer"
              />
              {t("projdetail.showServiceControl")}
            </label>
            <label className="flex items-center gap-2 cursor-pointer text-body text-slate-300 select-none">
              <input
                type="checkbox"
                checked={pid ? autoStartServices.includes(pid) : false}
                onChange={async (e) => {
                  if (pid) {
                    const enabled = e.target.checked;
                    await invoke("set_auto_start_service", { serviceId: pid, enabled });
                    setAutoStartServices((prev) =>
                      enabled ? [...prev.filter((id) => id !== pid), pid] : prev.filter((id) => id !== pid)
                    );
                  }
                }}
                className="rounded border-white/10 bg-black/40 text-red-500 focus:ring-red-500 w-3.5 h-3.5 cursor-pointer"
              />
              <span className={pid && autoStartServices.includes(pid) ? "text-red-400 font-semibold" : ""}>
                {t("projdetail.autoStartService")}
              </span>
            </label>
          </>
        )}
      </div>
      <p className="text-tiny text-slate-500 leading-normal">
        {t("projdetail.trayHint")}
      </p>
    </div>
  );
}
