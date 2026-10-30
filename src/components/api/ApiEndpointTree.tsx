import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Star, ChevronDown, ChevronRight, Folder, Pencil, Plus, FolderInput, Trash2 } from "lucide-react";
import type { ApiEndpoint, ApiModule } from "./types";
import { EndpointRow } from "./panelSubs";

interface ApiEndpointTreeProps {
  endpoints: ApiEndpoint[];
  modules: ApiModule[];
  selectedId: string | null;
  setSelectedId: (id: string) => void;
  onEndpointAction: (action: "delete" | "favorite", endpoint: ApiEndpoint) => void;
  onModuleAction: (action: "edit" | "createEndpoint" | "move" | "delete", module: ApiModule) => void;
}

export function ApiEndpointTree({ endpoints, modules, selectedId, setSelectedId, onEndpointAction, onModuleAction }: ApiEndpointTreeProps) {
  const { t } = useTranslation();
const [expanded, setExpanded] = useState<Set<string>>(new Set());
const toggleExpand = (id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

    const loose = endpoints.filter((e) => !e.module_id);
    const favs = endpoints.filter((e) => e.is_favorite);
    const row = (ep: ApiEndpoint) => (
      <EndpointRow
        key={ep.id}
        ep={ep}
        selected={selectedId === ep.id}
        onSelect={() => setSelectedId(ep.id)}
        onDelete={() => onEndpointAction("delete", ep)}
        onToggleFavorite={() => onEndpointAction("favorite", ep)}
      />
    );
    return (
      <div className="space-y-0.5">
        {favs.length > 0 && (
          <div className="mb-1">
            <div className="flex items-center gap-1 px-1.5 pt-1 pb-0.5">
              <Star className="w-3 h-3 text-amber-400 fill-amber-400" />
              <span className="text-tiny font-semibold text-slate-500">{t("api.fav")}</span>
            </div>
            {favs.map(row)}
          </div>
        )}
        {modules.map((m) => {
          const children = endpoints.filter((e) => e.module_id === m.id);
          const isOpen = expanded.has(m.id);
          return (
            <div key={m.id}>
              <div className="group flex items-center gap-1 rounded-md px-1.5 py-1 hover:bg-white/5 cursor-pointer" onClick={() => toggleExpand(m.id)}>
                {isOpen ? <ChevronDown className="w-3.5 h-3.5 text-slate-500" /> : <ChevronRight className="w-3.5 h-3.5 text-slate-500" />}
                <Folder className="w-3.5 h-3.5 text-amber-400/80 shrink-0" />
                <span className="flex-1 text-body text-slate-300 truncate">{m.name}</span>
                {m.description && <span className="text-tiny text-slate-600 truncate max-w-28 hidden group-hover:block" title={m.description}>{m.description}</span>}
                <span className="text-tiny px-1.5 py-px rounded-full bg-white/5 border border-white/5 text-slate-500 tabular-nums">{children.length}</span>
                <button
                  onClick={(e) => { e.stopPropagation(); onModuleAction("edit", m); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title={t("api.editModuleTip")}
                >
                  <Pencil className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); onModuleAction("createEndpoint", m); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title={t("api.addEndpoint")}
                >
                  <Plus className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); onModuleAction("move", m); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-[var(--module-accent)] cursor-pointer"
                  title={t("api.moveToModule")}
                >
                  <FolderInput className="w-3 h-3" />
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); onModuleAction("delete", m); }}
                  className="hidden group-hover:block p-0.5 text-slate-500 hover:text-rose-400 cursor-pointer"
                  title={t("api.deleteModuleTip")}
                >
                  <Trash2 className="w-3 h-3" />
                </button>
              </div>
              {/* 折叠动画容器（grid-rows 过渡） */}
              <div
                className="ml-4 grid transition-[grid-template-rows] duration-200 ease-out"
                style={{ gridTemplateRows: isOpen ? "1fr" : "0fr" }}
              >
                <div className="overflow-hidden min-h-0">
                  <div className="space-y-0.5 py-0.5">
                    {m.description && (
                      <div className="text-tiny text-slate-500/90 leading-snug px-1.5 py-0.5 border-l border-white/10 ml-1">{m.description}</div>
                    )}
                    {children.map(row)}
                    {children.length === 0 && <div className="text-tiny text-slate-600 px-2 py-0.5">{t("api.emptyModule")}</div>}
                  </div>
                </div>
              </div>
            </div>
          );
        })}
        {loose.map(row)}
        {modules.length === 0 && endpoints.length === 0 && (
          <div className="text-tiny text-slate-600 px-2 py-2">{t("api.noEndpoints")}</div>
        )}
      </div>
    );

}
