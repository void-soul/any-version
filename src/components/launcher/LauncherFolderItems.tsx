import type { ScannedProgram } from "./types";
import type { LauncherAppearance } from "./buildLauncherAppearance";
import { FolderOpen, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification, Item } from "./types";
import VexEmptyState from "../VexEmptyState";
import { cardClass } from "./LauncherPanel.model";
import { ItemCardBody } from "./LauncherPanel.ItemCardBody";
import type { Dispatch, SetStateAction } from "react";

interface LauncherFolderItemsProps {
 appearance: LauncherAppearance;
  activeFolderPath: string;
  folderItems: ScannedProgram[];
  folderLoading: boolean;
  folderError: string | null;
  setEditingCategory: Dispatch<SetStateAction<Classification | null>>;
  activeTopCategory: Classification;
  setCategoryParentId: Dispatch<SetStateAction<number | null>>;
  setCategoryModalOpen: Dispatch<SetStateAction<boolean>>;
  folderVirtualItems: Item[];
  launchVirtual: (name: string, target: string, itemType: number, params?: string | null | undefined, icon?: string | null | undefined) => Promise<void>;
  setFolderContextMenu: Dispatch<SetStateAction<{ x: number; y: number; item: ScannedProgram; } | null>>;
}


/** Pure LauncherFolderItems view; its owning panel retains side effects and state. */
export function LauncherFolderItems({ appearance, activeFolderPath, folderItems, folderLoading, folderError, setEditingCategory, activeTopCategory, setCategoryParentId, setCategoryModalOpen, folderVirtualItems, launchVirtual, setFolderContextMenu }: LauncherFolderItemsProps) {
const { gridClass,densityGap,gridStyle,itemView,view } = appearance;
  const { t } = useTranslation();
  return (
<div className="space-y-2.5">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1.5 text-caption text-slate-400 min-w-0">
                  <FolderOpen className="w-3.5 h-3.5 text-amber-400 flex-shrink-0" />
                  <span className="truncate" title={activeFolderPath}>
                    {activeFolderPath || t("launcher.folderNotSet")}
                  </span>
                  {activeFolderPath && (
                    <span className="text-slate-600 flex-shrink-0">
                      {t("launcher.folderItemCount", { count: folderItems.length })}
                    </span>
                  )}
                </div>
                {folderLoading && <Loader2 className="w-3.5 h-3.5 animate-spin text-[var(--module-accent)]" />}
              </div>

              {folderError ? (
                <div className="rounded-card border border-red-500/30 bg-red-500/10 px-3 py-2 text-caption text-red-300 break-all">
                  {t("launcher.folderScanFail", { err: folderError })}
                </div>
              ) : !activeFolderPath ? (
                <VexEmptyState
                  title={t("launcher.folderNotSetTitle")}
                  desc={t("launcher.folderNotSetHint")}
                  avatarSize={44}
                  className="!py-12"
                  action={{
                    label: t("launcher.editCategory"),
                    onClick: () => {
                      setEditingCategory(activeTopCategory || null);
                      setCategoryParentId(activeTopCategory?.parentId ?? null);
                      setCategoryModalOpen(true);
                    },
                  }}
                />
              ) : folderVirtualItems.length === 0 && !folderLoading ? (
                <VexEmptyState
                  title={t("launcher.folderEmpty")}
                  desc={t("launcher.folderEmptyHint")}
                  avatarSize={44}
                  className="!py-12"
                />
              ) : (
                <div className={`${gridClass} ${densityGap}`} style={gridStyle}>
                  {folderVirtualItems.map((item) => (
                    <div
                      key={item.id}
                      className={cardClass(item, itemView, {})}
                      style={{ borderRadius: view.itemRadius }}
                      title={`${item.name}\n${item.data.target || ""}`}
                      onClick={() =>
                        launchVirtual(
                          item.name,
                          item.data.target || "",
                          item.itemType,
                          item.data.params,
                          item.data.icon
                        )
                      }
                      onContextMenu={(e) => {
                        e.preventDefault();
                        const program = folderItems[-item.id - 1];
                        if (program) {
                          setFolderContextMenu({ x: e.clientX, y: e.clientY, item: program });
                        }
                      }}
                    >
                      <ItemCardBody item={item} view={itemView} checkResults={{}} />
                    </div>
                  ))}
                </div>
              )}
            </div>
  );
}
