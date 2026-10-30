import { X, ArrowRightLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface LauncherMoveCategoriesDialogProps {
  moveItemsLoading: boolean;
  setMoveItemsModalOpen: Dispatch<SetStateAction<boolean>>;
  moveItemsSource: Classification;
  moveItemsSourceCount: number;
  moveItemsTargetList: Classification[];
  classifications: Classification[];
  moveItemsTarget: number | null;
  setMoveItemsTarget: Dispatch<SetStateAction<number | null>>;
  handleMoveItemsConfirm: () => Promise<void>;
}


/** Pure LauncherMoveCategoriesDialog view; its owning panel retains side effects and state. */
export function LauncherMoveCategoriesDialog({ moveItemsLoading, setMoveItemsModalOpen, moveItemsSource, moveItemsSourceCount, moveItemsTargetList, classifications, moveItemsTarget, setMoveItemsTarget, handleMoveItemsConfirm }: LauncherMoveCategoriesDialogProps) {
  const { t } = useTranslation();
  return (
<div
          className="fixed inset-0 z-[250] modal-mask bg-black/60 backdrop-blur-sm flex items-center justify-center p-4 animate-in fade-in duration-100"
        >
          <div
            className="w-full max-w-sm bg-surface-panel border border-white/15 rounded-panel p-5 shadow-2xl space-y-4 text-body"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between pb-2 border-b border-white/10">
              <div className="flex items-center gap-2">
                <ArrowRightLeft className="w-4 h-4 text-cyan-400" />
                <h3 className="text-sm font-bold text-white">{t("launcher.moveSubCategoryTitle")}</h3>
              </div>
              <button
                onClick={() => !moveItemsLoading && setMoveItemsModalOpen(false)}
                className="text-slate-400 hover:text-white p-1"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <p className="text-caption text-slate-400 leading-relaxed">
              {t("launcher.moveExplain1")}
              <span className="text-slate-200 font-medium">{moveItemsSource.name}</span>
              {t("launcher.moveExplain2")}
              <span className="text-cyan-300 font-medium">{moveItemsSourceCount}</span>
              {t("launcher.moveExplain3")}
            </p>

            <div>
              <label className="block text-caption text-slate-400 mb-1.5">{t("launcher.pickTargetCat")}</label>
              <div className="max-h-56 overflow-y-auto space-y-1 pr-1">
                {moveItemsTargetList.length === 0 ? (
                  <p className="text-caption text-slate-500">{t("launcher.noTargetCat")}</p>
                ) : (
                  moveItemsTargetList.map((c) => {
                    const depth = (() => {
                      let d = 0;
                      let p = c.parentId;
                      while (p) {
                        d++;
                        p = classifications.find((x) => x.id === p)?.parentId ?? null;
                      }
                      return d;
                    })();
                    const isSelected = moveItemsTarget === c.id;
                    return (
                      <button
                        type="button"
                        key={c.id}
                        onClick={() => setMoveItemsTarget(c.id)}
                        className={`w-full text-left px-3 py-1.5 rounded-ctl border transition cursor-pointer flex items-center gap-2 ${
                          isSelected
                            ? "ui-selected"
                            : "bg-white/[0.02] border-white/5 text-slate-300 hover:bg-white/[0.05]"
                        }`}
                      >
                        <span style={{ marginLeft: depth * 12 }} className={depth > 0 ? "text-slate-500" : ""}>
                          {depth > 0 ? "└ " : ""}
                        </span>
                        <span>{c.data?.icon || "📁"}</span>
                        <span className="truncate">{c.name}</span>
                      </button>
                    );
                  })
                )}
              </div>
            </div>

            <div className="flex items-center justify-end gap-2.5 pt-2">
              <button
                type="button"
                disabled={moveItemsLoading}
                onClick={() => setMoveItemsModalOpen(false)}
                className="px-4 py-2 rounded-card text-body font-medium text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer disabled:opacity-50"
              >
                {t("launcher.cancel")}
              </button>
              <button
                type="button"
                disabled={moveItemsLoading || moveItemsTarget === null}
                onClick={handleMoveItemsConfirm}
                className="ui-btn ui-btn-primary px-5 py-2 disabled:opacity-50 flex items-center gap-1.5"
              >
                <ArrowRightLeft className="w-3.5 h-3.5" />
                {moveItemsLoading ? t("launcher.moving") : t("launcher.confirmMove")}
              </button>
            </div>
          </div>
        </div>
  );
}
