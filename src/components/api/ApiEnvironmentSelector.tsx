import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Settings2, ChevronDown, Database } from "lucide-react";
import type { ApiEnvironment } from "./types";
import { POPOVER_BG } from "./ApiPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface ApiEnvironmentSelectorProps {
  setEnvPop: Dispatch<SetStateAction<boolean>>;
  setProjectPop: Dispatch<SetStateAction<boolean>>;
  activeEnv: ApiEnvironment | null;
  envPop: boolean;
  envs: ApiEnvironment[];
  setActiveEnvId: Dispatch<SetStateAction<string | null>>;
  activeProjectId: string | null;
  activeEnvId: string | null;
  setEnvModal: Dispatch<SetStateAction<boolean>>;
}

/** Pure ApiEnvironmentSelector view; its owning panel retains side effects and state. */
export function ApiEnvironmentSelector({ setEnvPop, setProjectPop, activeEnv, envPop, envs, setActiveEnvId, activeProjectId, activeEnvId, setEnvModal }: ApiEnvironmentSelectorProps) {
  const { t } = useTranslation();
  return (
    <div className="px-2 pt-2">
      <div className="flex items-center gap-1">
        <div className="relative flex-1 min-w-0">
          <button
            onClick={() => { setEnvPop((v) => !v); setProjectPop(false); }}
            className="flex w-full items-center gap-1.5 rounded-md border border-white/10 bg-black/30 px-2 py-1 text-caption text-slate-200 hover:border-white/25 cursor-pointer"
            title={t("api.switchEnvTip")}
          >
            <Database className="w-3 h-3 shrink-0 text-slate-500" />
            <span className="flex-1 truncate text-left">{activeEnv?.name ?? t("api.noEnv")}</span>
            <ChevronDown className="w-3 h-3 shrink-0 text-slate-500" />
          </button>
          {envPop && (
            <>
              <div className="fixed inset-0 z-30" onClick={() => setEnvPop(false)} />
              <div className="absolute left-0 top-full z-40 mt-1.5 min-w-full w-max max-w-[260px] overflow-hidden rounded-card border border-white/10 shadow-2xl" style={{ background: POPOVER_BG }}>
                <div className="px-3 py-1.5 text-tiny font-semibold text-slate-500">{t("api.envSwitchTitle")}</div>
                <div className="max-h-52 overflow-y-auto p-1 space-y-0.5">
                  {envs.length === 0 && <div className="px-2 py-1 text-tiny text-slate-600">{t("api.noEnvHint")}</div>}
                  {envs.map((e) => (
                    <button
                      key={e.id}
                      onClick={() => { setActiveEnvId(e.id); setEnvPop(false); invoke("api_set_active_env", { projectId: activeProjectId, envId: e.id }); }}
                      className={`flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-caption text-left cursor-pointer transition-colors ${e.id === activeEnvId ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)] text-white" : "text-slate-300 hover:bg-white/5"}`}
                    >
                      {e.id === activeEnvId && <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ background: "var(--module-accent)" }} />}
                      <span className={`flex-1 truncate ${e.id === activeEnvId ? "" : "pl-3"}`}>{e.name}</span>
                    </button>
                  ))}
                </div>
              </div>
            </>
          )}
        </div>
        <button onClick={() => setEnvModal(true)} className="flex items-center gap-1 rounded-md border border-white/10 bg-black/30 px-1.5 py-1.5 text-tiny text-slate-400 hover:text-white hover:border-white/25 cursor-pointer" title={t("api.manageEnvsTip")}>
          <Settings2 className="w-3 h-3" />
          {t("api.envMaintain")}
        </button>
      </div>
    </div>
  );
}
