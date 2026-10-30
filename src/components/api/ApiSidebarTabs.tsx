import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Braces, History, Eraser } from "lucide-react";
import type { ApiEndpoint, ApiHistoryEntry } from "./types";
import { ACCENT } from "./panelParts";
import { theamedConfirm } from "../shared/ThemedAlert";
import type { Dispatch, SetStateAction } from "react";

interface ApiSidebarTabsProps {
  setSideTab: Dispatch<SetStateAction<"history" | "tree">>;
  sideTab: "history" | "tree";
  endpoints: ApiEndpoint[];
  history: ApiHistoryEntry[];
  activeProjectId: string | null;
  setHistory: Dispatch<SetStateAction<ApiHistoryEntry[]>>;
}

/** Pure ApiSidebarTabs view; its owning panel retains side effects and state. */
export function ApiSidebarTabs({ setSideTab, sideTab, endpoints, history, activeProjectId, setHistory }: ApiSidebarTabsProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-0.5 px-2 pt-2">
      <button
        onClick={() => setSideTab("tree")}
        className={`flex flex-1 items-center gap-1 rounded-md px-2 py-1 text-caption cursor-pointer transition-colors ${sideTab === "tree" ? "bg-[color-mix(in_srgb,var(--module-accent)_14%,transparent)] text-white" : "text-slate-400 hover:bg-white/5 hover:text-slate-300"}`}
      >
        <Braces className="w-3 h-3" style={sideTab === "tree" ? { color: ACCENT } : undefined} />
        {t("api.tabTree")}
        <span className="ml-auto text-micro tabular-nums text-slate-500">{endpoints.length}</span>
      </button>
      <button
        onClick={() => setSideTab("history")}
        className={`flex flex-1 items-center gap-1 rounded-md px-2 py-1 text-caption cursor-pointer transition-colors ${sideTab === "history" ? "bg-[color-mix(in_srgb,var(--module-accent)_14%,transparent)] text-white" : "text-slate-400 hover:bg-white/5 hover:text-slate-300"}`}
      >
        <History className="w-3 h-3" style={sideTab === "history" ? { color: ACCENT } : undefined} />
        {t("api.tabHistory")}
        <span className="ml-auto text-micro tabular-nums text-slate-500">{history.length}</span>
      </button>
      {sideTab === "history" && history.length > 0 && (
        <button
          onClick={async () => {
            if (!(await theamedConfirm(t("api.clearHistoryConfirm"), { danger: true }))) return;
            await invoke("api_clear_history", { projectId: activeProjectId });
            setHistory([]);
          }}
          className="p-1 text-slate-600 hover:text-rose-400 cursor-pointer"
          title={t("api.clearHistoryTip")}
        >
          <Eraser className="w-3 h-3" />
        </button>
      )}
    </div>
  );
}
