import { useTranslation } from "react-i18next";
import { type McpManagedSource } from "./McpManager.model";
import type { Dispatch, SetStateAction } from "react";

interface McpManagedSourceEditorProps {
  setFManaged: Dispatch<SetStateAction<McpManagedSource | null>>;
  fManaged: McpManagedSource | null;
  patchManaged: (patch: Partial<McpManagedSource>) => void;
}


/** Pure McpManagedSourceEditor view; its owning panel retains side effects and state. */
export function McpManagedSourceEditor({ setFManaged, fManaged, patchManaged }: McpManagedSourceEditorProps) {
  const { t } = useTranslation();
  return (
    <div className="rounded-ctl border border-white/10 bg-black/20 p-2.5 space-y-2">
      <div className="flex items-center justify-between gap-2">
        <label className="text-micro text-slate-500">{t("mcp.managedTitle")}</label>
        <button
          type="button"
          onClick={() =>
            setFManaged(
              fManaged
                ? null
                : {
                    repo: "",
                    tagTemplate: "v{version}",
                    assetPrefix: "",
                    assetSuffix: ".zip",
                    extractSubdir: "",
                    executable: "",
                  },
            )
          }
          className="px-2 py-0.5 rounded text-micro bg-white/5 border border-white/10 text-slate-300 hover:text-white cursor-pointer"
        >
          {fManaged ? t("mcp.managedDisable") : t("mcp.managedEnable")}
        </button>
      </div>
      {fManaged ? (
        <>
          <input
            value={fManaged.repo}
            onChange={(e) => patchManaged({ repo: e.target.value })}
            placeholder="owner/repo"
            className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
          />
          <div className="grid grid-cols-2 gap-1.5">
            <input
              value={fManaged.tagTemplate}
              onChange={(e) => patchManaged({ tagTemplate: e.target.value })}
              placeholder="v{version}"
              className="ui-input rounded-ctl px-2.5 py-1.5 text-tiny text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
            />
            <input
              value={fManaged.assetSuffix}
              onChange={(e) => patchManaged({ assetSuffix: e.target.value })}
              placeholder=".zip"
              className="ui-input rounded-ctl px-2.5 py-1.5 text-tiny text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
            />
            <input
              value={fManaged.assetPrefix}
              onChange={(e) => patchManaged({ assetPrefix: e.target.value })}
              placeholder="name-{version}"
              className="ui-input rounded-ctl px-2.5 py-1.5 text-tiny text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
            />
            <input
              value={fManaged.extractSubdir}
              onChange={(e) => patchManaged({ extractSubdir: e.target.value })}
              placeholder={t("mcp.managedExtractPh")}
              className="ui-input rounded-ctl px-2.5 py-1.5 text-tiny text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
            />
          </div>
          <input
            value={fManaged.executable}
            onChange={(e) => patchManaged({ executable: e.target.value })}
            placeholder="bin/xxx.bat"
            className="w-full ui-input rounded-ctl px-2.5 py-1.5 text-body text-slate-200 font-mono focus:outline-none focus:border-[var(--module-accent)]"
          />
          <p className="text-micro text-slate-500 leading-relaxed">{t("mcp.managedHint")}</p>
        </>
      ) : (
        <p className="text-micro text-slate-500 leading-relaxed">{t("mcp.managedOffHint")}</p>
      )}
    </div>
  );
}
