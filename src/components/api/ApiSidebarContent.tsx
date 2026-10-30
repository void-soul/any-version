import type { ReactElement } from "react";
import { useTranslation } from "react-i18next";
import type { ApiHistoryEntry } from "./types";
import { methodIcon } from "./panelParts";

interface ApiSidebarContentProps {
  sideTab: "history" | "tree";
  renderTree: () => ReactElement;
  history: ApiHistoryEntry[];
  replayHistory: (h: ApiHistoryEntry) => Promise<void>;
}

/** Pure ApiSidebarContent view; its owning panel retains side effects and state. */
export function ApiSidebarContent({ sideTab, renderTree, history, replayHistory }: ApiSidebarContentProps) {
  const { t } = useTranslation();
  return (
    <div className="flex-1 min-h-0 overflow-y-auto p-2 mt-1">
      {sideTab === "tree" ? (
        renderTree()
      ) : (
        <div className="space-y-0.5">
          {history.slice(0, 50).map((h) => {
            const Icon = methodIcon(h.method);
            return (
              <button
                key={h.id}
                onClick={() => replayHistory(h)}
                className="group flex w-full items-center gap-1.5 rounded-md px-1.5 py-1 text-left hover:bg-white/5 cursor-pointer"
                title={`${h.method} ${h.url}\n${h.created_at.replace("T", " ").slice(0, 19)}`}
              >
                <Icon className={`w-3 h-3 shrink-0 ${h.method === "GET" ? "text-emerald-400" : h.method === "POST" ? "text-amber-400" : h.method === "DELETE" ? "text-rose-400" : "text-slate-400"}`} />
                <span className="flex-1 text-caption text-slate-400 truncate">{h.name || h.url}</span>
              </button>
            );
          })}
          {history.length === 0 && <div className="text-tiny text-slate-600 px-1.5 py-1">{t("api.historyEmpty")}</div>}
        </div>
      )}
    </div>
  );
}
