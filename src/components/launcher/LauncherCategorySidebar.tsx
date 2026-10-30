import { Plus, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Classification } from "./types";
import { Droppable } from "./LauncherPanel.Droppable";
import type { Dispatch, SetStateAction, RefObject } from "react";

interface LauncherCategorySidebarProps {
  topCategories: Classification[];
  activeTopCategory: Classification;
  justDraggedAtRef: RefObject<number>;
  setActiveParentId: Dispatch<SetStateAction<number | null>>;
  isSearchOpen: boolean;
  closeSearch: () => void;
  setIsDragOver: Dispatch<SetStateAction<boolean>>;
  setTargetDragSubId: Dispatch<SetStateAction<number | null>>;
  processDroppedPaths: (paths: string[], targetClassificationId: number) => Promise<void>;
  setCategoryContextMenu: Dispatch<SetStateAction<{ x: number; y: number; category: Classification; } | null>>;
  targetDragSubId: number | null;
  setEditingCategory: Dispatch<SetStateAction<Classification | null>>;
  setCategoryParentId: Dispatch<SetStateAction<number | null>>;
  setCategoryModalOpen: Dispatch<SetStateAction<boolean>>;
}


/** Pure LauncherCategorySidebar view; its owning panel retains side effects and state. */
export function LauncherCategorySidebar({ topCategories, activeTopCategory, justDraggedAtRef, setActiveParentId, isSearchOpen, closeSearch, setIsDragOver, setTargetDragSubId, processDroppedPaths, setCategoryContextMenu, targetDragSubId, setEditingCategory, setCategoryParentId, setCategoryModalOpen }: LauncherCategorySidebarProps) {
  const { t } = useTranslation();
  return (
<div className="w-32 flex-shrink-0 border-r border-white/5 bg-white/[0.02] flex flex-col justify-between py-2 overflow-y-auto">
          <div className="space-y-1 px-1.5">
            {topCategories.map((cat) => {
              const isActive = cat.id === activeTopCategory?.id;

              return (
                <Droppable
                  key={cat.id}
                  id={`sidebar:${cat.id}`}
                  dataCatId={cat.id}
                  onClick={() => {
                    if (Date.now() - justDraggedAtRef.current < 300) return; // 拖拽后的残留 click 不触发切换
                    setActiveParentId(cat.id);
                    if (isSearchOpen) closeSearch();
                  }}
                  onDragOver={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setIsDragOver(true);
                    setTargetDragSubId(cat.id);
                  }}
                  onDrop={async (e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setIsDragOver(false);
                    setTargetDragSubId(null);
                    // 左栏仅接收外部文件；内部项目拖拽由 dnd-kit 的 handleDragOver/handleDragEnd 处理
                    const files = e.dataTransfer.files;
                    const paths: string[] = [];
                    for (let i = 0; i < files.length; i++) {
                      // @ts-ignore
                      const fullPath = files[i].path || files[i].name;
                      if (fullPath) paths.push(fullPath);
                    }
                    if (paths.length > 0) {
                      await processDroppedPaths(paths, cat.id);
                      setActiveParentId(cat.id);
                    }
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setCategoryContextMenu({ x: e.clientX, y: e.clientY, category: cat });
                  }}
                  className={`w-full px-3 py-2 rounded-card text-body flex items-center justify-between transition cursor-pointer group ${
                    isActive
                      // 选中态全站一致：实心 accent（原来是亮底 bg-slate-200，在暗色 UI 里很突兀）
                      ? "ui-selected font-bold"
                      : "ui-row font-medium hover:bg-white/5"
                  } ${
                    targetDragSubId === cat.id
                      ? "!bg-[var(--module-accent)] !text-slate-50 shadow-md ring-2 ring-[var(--module-accent-ring)]"
                      : ""
                  }`}
                >
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="shrink-0 leading-none" style={{ fontSize: 14 }}>
                      {cat.data?.icon || "📁"}
                    </span>
                    <span className="truncate">{cat.name}</span>
                  </span>
                  <ChevronRight
                    className={`w-3.5 h-3.5 flex-shrink-0 transition ${
                      isActive ? "text-white" : "opacity-40 group-hover:opacity-100"
                    }`}
                  />
                </Droppable>
              );
            })}
          </div>

          {/* Add Category Button at bottom of sidebar */}
          <div className="px-1.5 pt-2 border-t border-white/5">
            <button
              onClick={() => {
                setEditingCategory(null);
                setCategoryParentId(null);
                setCategoryModalOpen(true);
              }}
              className="w-full py-1.5 px-2 rounded-card text-slate-400 hover:text-slate-200 hover:bg-white/5 text-caption font-medium flex items-center justify-center gap-1 border border-dashed border-white/10 hover:border-white/20 transition cursor-pointer"
            >
              <Plus className="w-3 h-3" />
              <span>{t("launcher.newCategory")}</span>
            </button>
          </div>
        </div>
  );
}
