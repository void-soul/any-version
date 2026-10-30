import { FileText, X, BarChart3 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification, Item } from "./types";
import VexEmptyState from "../VexEmptyState";
import type { Dispatch, SetStateAction } from "react";

interface LauncherUsageStatsProps {
  setUsageStatsOpen: Dispatch<SetStateAction<boolean>>;
  usageStats: Item[];
  classificationMap: Map<number, Classification>;
  setActiveParentId: Dispatch<SetStateAction<number | null>>;
  activeParentId: number | null;
  handleExecuteItem: (item: Item) => Promise<void>;
}


/** Pure LauncherUsageStats view; its owning panel retains side effects and state. */
export function LauncherUsageStats({ setUsageStatsOpen, usageStats, classificationMap, setActiveParentId, activeParentId, handleExecuteItem }: LauncherUsageStatsProps) {
  const { t } = useTranslation();
  return (
<div className="absolute inset-y-0 left-0 right-0 z-30 bg-surface-panel/80 backdrop-blur-md flex flex-col overflow-hidden">
            <div className="flex items-center justify-between border-b border-white/10 px-5 py-3">
              <div className="flex items-center gap-2">
                <BarChart3 className="w-4 h-4 text-amber-400" />
                <div>
                  <h3 className="text-sm font-bold text-white">{t("launcher.usageStats")}</h3>
                  <p className="text-tiny text-slate-500">{t("launcher.usageStatsHint")}</p>
                </div>
              </div>
              <button type="button" onClick={() => setUsageStatsOpen(false)} className="rounded-ctl p-1.5 text-slate-400 hover:bg-white/10 hover:text-white" title={t("launcher.closeUsageStats")}>
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto p-5">
              {usageStats.length === 0 ? (
                <VexEmptyState
                  title={t("launcher.noUsageStats")}
                  tick={t("launcher.noUsageStatsTick")}
                  avatarSize={40}
                  className="h-full"
                />
              ) : (
                <div className="mx-auto grid max-w-3xl grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3">
                  {usageStats.map((item, index) => {
                    const cat = classificationMap.get(item.classificationId);
                    return (
                      <button key={item.id} type="button" onClick={() => { setUsageStatsOpen(false); setActiveParentId(cat?.parentId ?? cat?.id ?? activeParentId); handleExecuteItem(item); }} className="group flex items-center gap-3 rounded-card border border-white/10 bg-white/[0.03] px-3 py-2.5 text-left transition hover:border-[var(--module-accent-ring)] hover:bg-[var(--module-accent-soft)]">
                        <span className="w-6 shrink-0 text-center font-mono text-tiny text-slate-600">{index + 1}</span>
                        <div className="flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-ctl bg-white/10">
                          {item.data.icon ? <img src={item.data.icon} className="h-7 w-7 object-contain" alt="" /> : item.data.htmlIcon ? <span className="text-lg">{item.data.htmlIcon}</span> : <FileText className="h-5 w-5 text-[var(--module-accent)]" />}
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-body font-medium text-slate-200 group-hover:text-white">{item.name}</div>
                          <div className="truncate text-micro text-slate-500">{cat?.data.icon ?? "📁"} {cat?.name ?? t("launcher.currentCat")}</div>
                        </div>
                        <span className="shrink-0 rounded-md bg-amber-400/10 px-1.5 py-1 font-mono text-tiny font-bold text-amber-300">{t("launcher.openCount", { count: item.data.openNumber ?? 0 })}</span>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          </div>
  );
}
