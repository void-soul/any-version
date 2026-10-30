import type { CSSProperties } from "react";
import { Clock } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification, Item } from "./types";
import VexEmptyState from "../VexEmptyState";
import { cardClass, type ItemView } from "./LauncherPanel.model";
import { ItemCardBody } from "./LauncherPanel.ItemCardBody";
import type { Dispatch, SetStateAction } from "react";

interface LauncherAggregateItemsProps {
  activeTopCategory: Classification;
  aggregateItems: Item[];
  gridClass: string;
  densityGap: string;
  gridStyle: CSSProperties;
  itemView: ItemView;
  checkResults: Record<number, { exists: boolean; }>;
  view: { iconSize: number; columns: number; density: "compact" | "cozy" | "spacious"; showName: boolean; iconBg: boolean; itemFontSize: number; itemRadius: number; itemBorder: boolean; categoryFontSize: number; categoryGap: number; };
  handleExecuteItem: (item: Item) => Promise<void>;
  setItemContextMenu: Dispatch<SetStateAction<{ x: number; y: number; item: Item; } | null>>;
}


/** Pure LauncherAggregateItems view; its owning panel retains side effects and state. */
export function LauncherAggregateItems({ activeTopCategory, aggregateItems, gridClass, densityGap, gridStyle, itemView, checkResults, view, handleExecuteItem, setItemContextMenu }: LauncherAggregateItemsProps) {
  const { t } = useTranslation();
  return (
<div className="space-y-2.5">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 text-caption text-slate-400">
                  <Clock className="w-3.5 h-3.5 text-[var(--module-accent)]" />
                  <span>
                    {activeTopCategory?.data?.aggregateSort === "lastOpen"
                      ? t("launcher.aggregateByRecent")
                      : activeTopCategory?.data?.aggregateSort === "name"
                        ? t("launcher.aggregateByName")
                        : t("launcher.aggregateByCount")}
                  </span>
                  <span className="text-slate-600">·</span>
                  <span>{t("launcher.aggregateTotal", { count: aggregateItems.length })}</span>
                </div>
              </div>
              {aggregateItems.length > 0 ? (
                <div className={`${gridClass} ${densityGap}`} style={gridStyle}>
                  {aggregateItems.map((item) => (
                    <div
                      key={item.id}
                      className={cardClass(item, itemView, checkResults)}
                      style={{ borderRadius: view.itemRadius }}
                      title={item.name}
                      onClick={() => handleExecuteItem(item)}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        setItemContextMenu({ x: e.clientX, y: e.clientY, item });
                      }}
                    >
                      <ItemCardBody item={item} view={itemView} checkResults={checkResults} />
                    </div>
                  ))}
                </div>
              ) : (
                <VexEmptyState
                  title={t("launcher.aggregateEmpty")}
                  desc={t("launcher.aggregateEmptyHint")}
                  avatarSize={46}
                  className="!py-16"
                />
              )}
            </div>
  );
}
