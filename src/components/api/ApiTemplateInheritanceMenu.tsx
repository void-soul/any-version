import { useTranslation } from "react-i18next";
import { Settings2, Link2, AlertTriangle, Lock, X } from "lucide-react";
import type { KeyValueItem } from "./types";
import { POPOVER_BG } from "./ApiPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface ApiTemplateInheritanceMenuProps {
  setShowTplPanel: Dispatch<SetStateAction<boolean>>;
  showTplPanel: boolean;
  tplItems: { total: number; outOfSync: number; groups: { label: string; items: (KeyValueItem & { synced: boolean; })[]; }[]; };
  reloadCurrent: () => Promise<void>;
  openProjectTpl: () => void;
}

/** Pure ApiTemplateInheritanceMenu view; its owning panel retains side effects and state. */
export function ApiTemplateInheritanceMenu({ setShowTplPanel, showTplPanel, tplItems, reloadCurrent, openProjectTpl }: ApiTemplateInheritanceMenuProps) {
  const { t } = useTranslation();
  return (
    <div className="relative">
      <button
        onClick={() => setShowTplPanel((v) => !v)}
        className={`flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-body cursor-pointer transition-colors ${showTplPanel ? "border-[var(--module-accent)]/50 bg-[color-mix(in_srgb,var(--module-accent)_12%,transparent)] text-[var(--module-accent)]" : "border-[color-mix(in_srgb,var(--module-accent)_35%,transparent)] bg-[color-mix(in_srgb,var(--module-accent)_8%,transparent)] text-[var(--module-accent)] hover:bg-[color-mix(in_srgb,var(--module-accent)_14%,transparent)]"} ${tplItems.outOfSync > 0 ? "!border-rose-500/60" : ""}`}
        title={tplItems.outOfSync > 0 ? t("api.tplOutOfSyncTip", { n: tplItems.outOfSync }) : t("api.tplViewTip")}
      >
        <Link2 className="w-3.5 h-3.5" />
        {t("api.tplInherit")}
        <span
          className="rounded-full px-1.5 py-px text-tiny font-bold text-white tabular-nums"
          style={{ background: tplItems.outOfSync > 0 ? "#f43f5e" : "var(--module-accent)" }}
        >
          {tplItems.total}
        </span>
        {tplItems.outOfSync > 0 && (
          <span
            className="absolute -right-1.5 -top-1.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-micro font-bold text-white shadow"
            style={{ background: "#e11d48" }}
            title={t("api.tplPendingTip", { n: tplItems.outOfSync })}
          >
            {tplItems.outOfSync}
          </span>
        )}
      </button>
      {showTplPanel && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setShowTplPanel(false)} />
          <div className="absolute right-0 top-full z-40 mt-1.5 w-80 overflow-hidden rounded-card border border-white/10 shadow-2xl" style={{ background: POPOVER_BG }}>
            <div className="flex items-center justify-between border-b border-white/10 px-3 py-2">
              <span className="flex items-center gap-1.5 text-caption font-semibold text-white">
                <Link2 className="w-3 h-3" style={{ color: "var(--module-accent)" }} />
                {t("api.tplPanelTitle")}
              </span>
              <button onClick={() => setShowTplPanel(false)} className="p-0.5 text-slate-500 hover:text-white cursor-pointer">
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
            <div className="max-h-64 overflow-y-auto p-2 space-y-2.5">
              {tplItems.outOfSync > 0 && (
                <div className="flex items-center gap-2 rounded-md border border-rose-500/40 bg-rose-500/10 px-2.5 py-2">
                  <AlertTriangle className="w-3.5 h-3.5 shrink-0 text-rose-400" />
                  <span className="flex-1 text-tiny leading-snug text-rose-300">
                    {t("api.tplOutOfSyncDesc", { n: tplItems.outOfSync })}
                  </span>
                  <button
                    onClick={reloadCurrent}
                    className="shrink-0 rounded-md px-2 py-1 text-tiny font-semibold text-white cursor-pointer hover:opacity-85"
                    style={{ background: "#e11d48" }}
                  >
                    {t("api.reload")}
                  </button>
                </div>
              )}
              {tplItems.groups.map((g) => (
                <div key={g.label}>
                  <div className="px-1 pb-1 text-micro font-semibold uppercase tracking-wider text-slate-500">{g.label}</div>
                  <div className="space-y-1">
                    {g.items.map((it, i) => (
                      <div key={i} className={`flex items-center gap-1.5 rounded-md border px-2 py-1 ${it.synced ? "border-white/5 bg-black/25" : "border-rose-500/40 bg-rose-500/10"}`}>
                        <Lock className={`w-3 h-3 shrink-0 ${it.synced ? "text-[var(--module-accent)]" : "text-rose-400"}`} />
                        <code className="font-mono text-tiny text-slate-200 truncate">{it.key}</code>
                        <span className="text-tiny text-slate-500">=</span>
                        <code className={`font-mono text-tiny truncate ${it.synced ? "text-[var(--module-accent)]/90" : "text-rose-300"}`} title={it.value}>{it.value || t("api.emptyVal")}</code>
                        {!it.synced && <span className="ml-auto shrink-0 text-micro font-semibold text-rose-400">{t("api.pendingSync")}</span>}
                      </div>
                    ))}
                  </div>
                </div>
              ))}
              <div className="px-1 pt-1 space-y-1.5">
                <button
                  onClick={openProjectTpl}
                  className="flex w-full items-center justify-center gap-1.5 rounded-ctl border border-white/10 bg-white/5 px-2 py-1.5 text-tiny font-semibold text-slate-200 hover:bg-white/10 hover:text-white cursor-pointer transition-colors"
                >
                  <Settings2 className="w-3 h-3" style={{ color: "var(--module-accent)" }} />
                  {t("api.openProjectTpl")}
                </button>
                <div className="text-center text-micro text-slate-500">{t("api.tplSyncHint")}</div>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
