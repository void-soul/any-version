import { Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification } from "./types";
import CategoryTreeSelect from "./CategoryTreeSelect";
import { NEW_CATEGORY_VALUE } from "./LauncherPanel.model";
import type { Dispatch, SetStateAction } from "react";

interface LauncherDeleteCategoryDialogProps {
  pendingDeleteCategory: Classification;
  scopeOf: (cat: Classification) => { cats: number; items: number; };
  deleteCascade: boolean;
  setDeleteCascade: Dispatch<SetStateAction<boolean>>;
  classifications: Classification[];
  deleteTarget: string;
  setDeleteTarget: Dispatch<SetStateAction<string>>;
  setDeleteNewName: Dispatch<SetStateAction<string>>;
  deleteNewName: string;
  deletingCategory: boolean;
  setPendingDeleteCategory: Dispatch<SetStateAction<Classification | null>>;
  confirmDeleteCategory: () => Promise<void>;
}


/** Pure LauncherDeleteCategoryDialog view; its owning panel retains side effects and state. */
export function LauncherDeleteCategoryDialog({ pendingDeleteCategory, scopeOf, deleteCascade, setDeleteCascade, classifications, deleteTarget, setDeleteTarget, setDeleteNewName, deleteNewName, deletingCategory, setPendingDeleteCategory, confirmDeleteCategory }: LauncherDeleteCategoryDialogProps) {
  const { t } = useTranslation();
  return (
<div
          className="fixed inset-0 z-[110] modal-mask flex items-center justify-center bg-black/60 backdrop-blur-sm p-4"
        >
          <div
            className="bg-surface-panel border rounded-panel w-full max-w-md shadow-2xl overflow-hidden animate-in fade-in zoom-in-95 duration-150 text-slate-100"
            style={{ borderColor: "var(--module-accent-ring)" }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center gap-2.5 px-5 py-4 border-b border-white/10 bg-white/[0.02]">
              <span className="text-xl">{pendingDeleteCategory.data?.icon || "📁"}</span>
              <h3 className="text-sm font-semibold text-white">{t("launcher.delCatTitle")}</h3>
            </div>
            <div className="p-5 space-y-3.5">
              <p className="text-body leading-relaxed text-slate-300">
                {t("launcher.delCatConfirm")}
                <span
                  className="mx-1 px-1.5 py-0.5 rounded font-medium"
                  style={{
                    backgroundColor: "var(--module-accent-soft)",
                    color: "var(--module-accent)",
                  }}
                >
                  {pendingDeleteCategory.name}
                </span>
                {t("launcher.delCatConfirmTail")}
              </p>
              <p className="text-caption text-slate-400">
                {t("launcher.delCatScope", {
                  cats: scopeOf(pendingDeleteCategory).cats,
                  items: scopeOf(pendingDeleteCategory).items,
                })}
              </p>

              {/* 选项一：连内容一起删 */}
              <label className="flex items-start gap-2 cursor-pointer">
                <input
                  type="radio"
                  name="del-cat-mode"
                  checked={deleteCascade}
                  onChange={() => setDeleteCascade(true)}
                  className="mt-0.5 accent-red-500"
                />
                <span className="text-body text-slate-200">{t("launcher.delCatModeDelete")}</span>
              </label>

              {/* 选项二：保留内容，但必须指定新上级 */}
              <div className="space-y-2">
                <label className="flex items-start gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="del-cat-mode"
                    checked={!deleteCascade}
                    onChange={() => setDeleteCascade(false)}
                    className="mt-0.5 accent-[var(--module-accent)]"
                  />
                  <span className="text-body text-slate-200">{t("launcher.delCatModeMove")}</span>
                </label>
                {!deleteCascade && (
                  <div className="ml-6 space-y-2">
                    <div className="flex items-center gap-2">
                      <span className="text-caption text-slate-400 shrink-0">
                        {t("launcher.delCatMoveTarget")}
                      </span>
                      <CategoryTreeSelect
                        classifications={classifications}
                        value={deleteTarget === NEW_CATEGORY_VALUE ? 0 : Number(deleteTarget) || 0}
                        onChange={(id) => {
                          setDeleteTarget(String(id));
                          setDeleteNewName("");
                        }}
                        excludeId={pendingDeleteCategory.id}
                        hideDescendantsOfExclude
                      />
                    </div>
                    <div className="flex items-center gap-2">
                      <input
                        type="text"
                        value={deleteNewName}
                        onChange={(e) => {
                          setDeleteNewName(e.target.value);
                          if (e.target.value.trim()) setDeleteTarget(NEW_CATEGORY_VALUE);
                        }}
                        placeholder={t("launcher.delCatNewName")}
                        className="glass-input flex-1 px-2 py-1 text-caption"
                      />
                    </div>
                    <p className="text-tiny text-slate-500">{t("launcher.delCatMoveHint")}</p>
                  </div>
                )}
              </div>

              <div className="flex items-center justify-end gap-2.5 pt-1">
                <button
                  type="button"
                  disabled={deletingCategory}
                  onClick={() => setPendingDeleteCategory(null)}
                  className="px-4 py-2 rounded-card text-body font-medium text-slate-400 hover:text-white hover:bg-white/5 transition cursor-pointer disabled:opacity-50"
                >
                  {t("common.cancel")}
                </button>
                <button
                  type="button"
                  disabled={deletingCategory}
                  onClick={confirmDeleteCategory}
                  className={`px-5 py-2 rounded-card text-body font-semibold text-white shadow-lg transition cursor-pointer disabled:opacity-50 flex items-center gap-1.5 ${
                    deleteCascade
                      ? "bg-red-600 hover:bg-red-500 shadow-red-600/30"
                      : "bg-[var(--module-accent)] hover:brightness-110"
                  }`}
                >
                  <Trash2 className="w-3.5 h-3.5" />
                  {deletingCategory
                    ? t("launcher.deleting")
                    : deleteCascade
                      ? t("common.delete")
                      : t("launcher.delCatMoveAction")}
                </button>
              </div>
            </div>
          </div>
        </div>
  );
}
