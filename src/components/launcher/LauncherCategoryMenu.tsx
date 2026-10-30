import { Plus, FolderPlus, Trash2, Edit2, ArrowRightLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification, Item } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface LauncherCategoryMenuProps {
  categoryContextMenu: { x: number; y: number; category: Classification; };
  setEditingCategory: Dispatch<SetStateAction<Classification | null>>;
  setCategoryParentId: Dispatch<SetStateAction<number | null>>;
  setCategoryModalOpen: Dispatch<SetStateAction<boolean>>;
  setCategoryContextMenu: Dispatch<SetStateAction<{ x: number; y: number; category: Classification; } | null>>;
  setEditingItem: Dispatch<SetStateAction<Item | null>>;
  setTargetClassificationId: Dispatch<SetStateAction<number>>;
  setItemModalOpen: Dispatch<SetStateAction<boolean>>;
  openMoveItemsModal: (cat: Classification) => void;
  handleDeleteCategory: (cat: Classification) => void;
}


/** Pure LauncherCategoryMenu view; its owning panel retains side effects and state. */
export function LauncherCategoryMenu({ categoryContextMenu, setEditingCategory, setCategoryParentId, setCategoryModalOpen, setCategoryContextMenu, setEditingItem, setTargetClassificationId, setItemModalOpen, openMoveItemsModal, handleDeleteCategory }: LauncherCategoryMenuProps) {
  const { t } = useTranslation();
  return (
<div
          style={{
            left: categoryContextMenu.x,
            ...(categoryContextMenu.y > (typeof window !== "undefined" ? window.innerHeight : 0) - 200
              ? { bottom: (typeof window !== "undefined" ? window.innerHeight : 0) - categoryContextMenu.y, top: "auto" }
              : { top: categoryContextMenu.y }),
          }}
          className="fixed z-[200] bg-surface-panel border border-white/15 rounded-card shadow-2xl p-1.5 min-w-[150px] text-body space-y-0.5 animate-in fade-in zoom-in-95 duration-100"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            onClick={() => {
              setEditingCategory(categoryContextMenu.category);
              setCategoryParentId(categoryContextMenu.category.parentId);
              setCategoryModalOpen(true);
              setCategoryContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-200 hover:bg-[var(--module-accent)] hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Edit2 className="w-3.5 h-3.5" />
            <span>{t("launcher.editCategory")}</span>
          </button>
          <button
            onClick={() => {
              setEditingCategory(null);
              setCategoryParentId(categoryContextMenu.category.id);
              setCategoryModalOpen(true);
              setCategoryContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <FolderPlus className="w-3.5 h-3.5" />
            <span>{t("launcher.addSubCategory")}</span>
          </button>
          <button
            onClick={() => {
              setEditingItem(null);
              setTargetClassificationId(categoryContextMenu.category.id);
              setItemModalOpen(true);
              setCategoryContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>{t("launcher.addItemToCategory")}</span>
          </button>
          <button
            onClick={() => openMoveItemsModal(categoryContextMenu.category)}
            title={t("launcher.moveSubCategoriesTitle")}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <ArrowRightLeft className="w-3.5 h-3.5 text-cyan-400" />
            <span>{t("launcher.moveSubCategories")}</span>
          </button>
          <div className="h-px bg-white/10 my-1" />
          <button
            onClick={() => {
              setCategoryContextMenu(null);
              handleDeleteCategory(categoryContextMenu.category);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-red-400 hover:bg-red-500 hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
            <span>{t("launcher.deleteCategory")}</span>
          </button>
        </div>
  );
}
