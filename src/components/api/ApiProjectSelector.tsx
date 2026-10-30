import { useTranslation } from "react-i18next";
import { Plus, ChevronDown, FlaskConical } from "lucide-react";
import type { ApiProject } from "./types";
import { ACCENT } from "./panelParts";
import { POPOVER_BG } from "./ApiPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface ApiProjectSelectorProps {
  setProjectPop: Dispatch<SetStateAction<boolean>>;
  setEnvPop: Dispatch<SetStateAction<boolean>>;
  currentProject: ApiProject | null;
  projectPop: boolean;
  openCreateProject: () => void;
  projects: ApiProject[];
  setActiveProjectId: Dispatch<SetStateAction<string | null>>;
  activeProjectId: string | null;
}

/** Pure ApiProjectSelector view; its owning panel retains side effects and state. */
export function ApiProjectSelector({ setProjectPop, setEnvPop, currentProject, projectPop, openCreateProject, projects, setActiveProjectId, activeProjectId }: ApiProjectSelectorProps) {
  const { t } = useTranslation();
  return (
    <div className="relative flex-1 min-w-0">
      <button
        onClick={() => { setProjectPop((v) => !v); setEnvPop(false); }}
        className="flex w-full items-center gap-1.5 rounded-md border border-white/10 bg-black/30 px-2 py-1.5 text-body text-slate-200 hover:border-white/25 cursor-pointer"
        title={t("api.switchProjectTip")}
      >
        <FlaskConical className="w-3.5 h-3.5 shrink-0" style={{ color: ACCENT }} />
        <span className="flex-1 truncate text-left">{currentProject?.name ?? t("api.selectProject")}</span>
        <ChevronDown className="w-3 h-3 shrink-0 text-slate-500" />
      </button>
      {projectPop && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setProjectPop(false)} />
          <div className="absolute left-0 top-full z-40 mt-1.5 w-56 overflow-hidden rounded-card border border-white/10 shadow-2xl" style={{ background: POPOVER_BG }}>
            <div className="flex items-center justify-between border-b border-white/10 px-3 py-1.5">
              <span className="text-tiny font-semibold text-slate-500">{t("api.apiProjects")}</span>
              <button onClick={openCreateProject} className="p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer" title={t("api.newProjectTip")}>
                <Plus className="w-3 h-3" />
              </button>
            </div>
            <div className="max-h-56 overflow-y-auto p-1 space-y-0.5">
              {projects.length === 0 && <div className="px-2 py-1 text-tiny text-slate-600">{t("api.noProjects")}</div>}
              {projects.map((p) => (
                <div
                  key={p.id}
                  onClick={() => { setActiveProjectId(p.id); setProjectPop(false); }}
                  className={`flex items-center gap-1.5 rounded-md px-2 py-1 cursor-pointer ${p.id === activeProjectId ? "bg-[color-mix(in_srgb,var(--module-accent)_15%,transparent)] text-white" : "text-slate-300 hover:bg-white/5"}`}
                >
                  <FlaskConical className="w-3 h-3 shrink-0" style={{ color: p.id === activeProjectId ? ACCENT : undefined }} />
                  <span className="flex-1 text-body truncate" title={p.description || undefined}>{p.name}</span>
                  {p.id === activeProjectId && (
                    <span className="rounded-full px-1.5 py-px text-micro font-semibold" style={{ background: "color-mix(in srgb, var(--module-accent) 25%, transparent)", color: ACCENT }}>{t("api.current")}</span>
                  )}
                </div>
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
