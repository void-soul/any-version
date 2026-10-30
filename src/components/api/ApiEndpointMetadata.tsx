import { useTranslation } from "react-i18next";
import { Folder } from "lucide-react";
import type { ApiModule, ApiEndpoint } from "./types";

interface ApiEndpointMetadataProps {
  modules: ApiModule[];
  draft: ApiEndpoint;
  updateDraft: (patch: Partial<ApiEndpoint>) => void;
}

/** Pure ApiEndpointMetadata view; its owning panel retains side effects and state. */
export function ApiEndpointMetadata({ modules, draft, updateDraft }: ApiEndpointMetadataProps) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-2 px-3 py-1.5 border-b border-white/5 bg-black/10">
      {(() => {
        const mod = modules.find((m) => m.id === draft.module_id);
        return (
          <>
            {mod && (
              <div className="flex items-center gap-1 shrink-0 text-tiny text-amber-300/80">
                <Folder className="w-3 h-3" />
                <span className="font-semibold">{mod.name}</span>
                {mod.description && <span className="text-slate-500 max-w-40 truncate" title={mod.description}>· {mod.description}</span>}
              </div>
            )}
            <input
              value={draft.description}
              onChange={(e) => updateDraft({ description: e.target.value })}
              placeholder={t("api.descPh")}
              className="flex-1 min-w-0 bg-transparent border border-transparent hover:border-white/10 focus:border-[var(--module-accent)]/40 rounded-md px-2 py-1 text-caption text-slate-300 placeholder:text-slate-600 focus:outline-none"
            />
          </>
        );
      })()}
    </div>
  );
}
