import { useState, useMemo, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Classification, DeleteClassificationResult, Item } from "./types";
import { NEW_CATEGORY_VALUE, flattenCategories } from "./LauncherPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the LauncherCategoryActions feature, independently of page rendering. */
export function useLauncherCategoryActions({ showToast, loadData, classifications, setCategoryContextMenu }: {
  showToast: (msg: string) => void;
  loadData: () => Promise<void>;
  classifications: Classification[];
  setCategoryContextMenu: Dispatch<SetStateAction<{ x: number; y: number; category: Classification; } | null>>;
}) {
  const { t } = useTranslation();


  // 批量转移项目：把某分类（含子分类）下的项目移动到目标分类
  const [moveItemsModalOpen, setMoveItemsModalOpen] = useState(false);

  const [moveItemsSource, setMoveItemsSource] = useState<Classification | null>(null);

  const [moveItemsTarget, setMoveItemsTarget] = useState<number | null>(null);

  const [moveItemsLoading, setMoveItemsLoading] = useState(false);


  // Save Category
  const handleSaveCategory = async (cat: Classification) => {
    await invoke("launcher_save_classification", { classification: cat });
    showToast(t("launcher.catSaved"));
    await loadData();
  };


  // Delete Category（确认弹框）
  //
  // 删除分类会产生孤儿：子分类的 parent_id、项目的 classification_id 都指向已删除的
  // 分类，它们在界面上不可达（看不见也删不掉）。所以这里必须让用户二选一：
  // ① 连同子分类与项目一起删；② 保留内容 —— 但**必须**指定新的上级（可新建）。
  const [pendingDeleteCategory, setPendingDeleteCategory] = useState<Classification | null>(null);

  const [deletingCategory, setDeletingCategory] = useState(false);

  const [deleteCascade, setDeleteCascade] = useState(true);

  const [deleteTarget, setDeleteTarget] = useState("");

  const [deleteNewName, setDeleteNewName] = useState("");


  /** 收集一棵分类树的全部子孙 id（含自己），用于排除非法迁移目标与统计影响范围。 */
  const subtreeIds = useCallback((cat: Classification): number[] => {
    const out: number[] = [cat.id];
    const walk = (list?: Classification[]) => {
      for (const c of list ?? []) {
        out.push(c.id);
        walk(c.childList);
      }
    };
    walk(cat.childList);
    return out;
  }, []);


  /** 影响范围：子分类数（含孙级）与项目数（含孙级分类里的）。 */
  const scopeOf = useCallback(
    (cat: Classification) => {
      let cats = 0;
      let items = 0;
      const walk = (list?: Classification[]) => {
        for (const c of list ?? []) {
          cats += 1;
          items += c.itemCount ?? 0;
          walk(c.childList);
        }
      };
      walk(cat.childList);
      return { cats, items: items + (cat.itemCount ?? 0) };
    },
    [],
  );


  const handleDeleteCategory = (cat: Classification) => {
    setPendingDeleteCategory(cat);
    setDeleteCascade(true);
    setDeleteNewName("");
    // 默认给一个合法目标：同级里第一个「不是自己、也不是自己子孙」的分类
    const banned = new Set(subtreeIds(cat));
    const first = flattenCategories(classifications).find((c) => !banned.has(c.id));
    setDeleteTarget(first ? String(first.id) : NEW_CATEGORY_VALUE);
  };


  const confirmDeleteCategory = async () => {
    if (!pendingDeleteCategory) return;
    setDeletingCategory(true);
    try {
      let newParentId: number | null = null;
      if (!deleteCascade) {
        if (deleteTarget === NEW_CATEGORY_VALUE) {
          if (!deleteNewName.trim()) {
            showToast(t("launcher.delCatNeedTarget"));
            setDeletingCategory(false);
            return;
          }
          // 「新建上级」：建在被删分类原来的位置上，再把内容迁进去
          newParentId = await invoke<number>("launcher_save_classification", {
            classification: {
              id: 0,
              parentId: pendingDeleteCategory.parentId ?? null,
              name: deleteNewName.trim(),
              classificationType: 0,
              data: {},
              shortcutKey: null,
              globalShortcutKey: false,
              order: pendingDeleteCategory.order ?? 0,
            },
          });
        } else {
          newParentId = Number(deleteTarget);
        }
      }
      const res = await invoke<DeleteClassificationResult>("launcher_delete_classification", {
        id: pendingDeleteCategory.id,
        cascade: deleteCascade,
        newParentId,
      });
      showToast(
        deleteCascade
          ? t("launcher.catDeletedCascade", {
              cats: res.deletedCategories,
              items: res.deletedItems,
            })
          : t("launcher.catDeletedMoved", {
              cats: res.movedCategories,
              items: res.movedItems,
            }),
      );
      setPendingDeleteCategory(null);
      await loadData();
    } catch (e: any) {
      showToast(t("launcher.deleteFail", { err: String(e) }));
    } finally {
      setDeletingCategory(false);
    }
  };


  // Save Item
  const handleSaveItem = async (it: Item) => {
    await invoke("launcher_save_item", { item: it });
    showToast(t("launcher.itemSaved"));
    await loadData();
  };


  // Delete Item
  const handleDeleteItem = async (id: number) => {
    await invoke("launcher_delete_item", { id });
    showToast(t("launcher.itemDeleted"));
    await loadData();
  };


  // ---- 批量移动子分类（保留层级）----

  // 递归收集某分类及其所有子孙分类的 id
  const collectCatIds = (cat: Classification): Set<number> => {
    const ids = new Set<number>([cat.id]);
    const walk = (parentId: number) => {
      for (const c of classifications) {
        if (c.parentId === parentId && !ids.has(c.id)) {
          ids.add(c.id);
          walk(c.id);
        }
      }
    };
    walk(cat.id);
    return ids;
  };


  // 源分类下的直接子分类数量（这些会整体搬到目标分类下）
  const moveItemsSourceCount = useMemo(() => {
    if (!moveItemsSource) return 0;
    return classifications.filter((c) => c.parentId === moveItemsSource.id).length;
  }, [moveItemsSource, classifications]);


  // 可用的目标分类：排除源分类及其所有子孙分类（避免循环 / 嵌套目标）
  const moveItemsTargetList = useMemo(() => {
    if (!moveItemsSource) return [];
    const excluded = collectCatIds(moveItemsSource);
    // 计算分类深度用于排序（防御性：父分类找不到或 parentId 为 null 时按 0 层处理，避免崩溃）
    const getDepth = (c: Classification): number => {
      let depth = 0;
      let pid = c.parentId;
      let guard = 0;
      while (pid != null && guard < 100) {
        const parent = classifications.find((x) => x.id === pid);
        if (!parent) break;
        depth++;
        pid = parent.parentId;
        guard++;
      }
      return depth;
    };
    return classifications
      .filter((c) => !excluded.has(c.id))
      .sort((a, b) => getDepth(a) - getDepth(b));
  }, [moveItemsSource, classifications]);


  // 打开移动子分类弹窗
  const openMoveItemsModal = (cat: Classification) => {
    setMoveItemsSource(cat);
    setMoveItemsTarget(null);
    setMoveItemsModalOpen(true);
    setCategoryContextMenu(null);
  };


  // 确认批量移动子分类
  const handleMoveItemsConfirm = async () => {
    if (!moveItemsSource || moveItemsTarget === null) return;
    setMoveItemsLoading(true);
    try {
      const moved = await invoke<number>("launcher_move_subcategories_to_classification", {
        sourceId: moveItemsSource.id,
        targetId: moveItemsTarget,
      });
      showToast(t("launcher.movedSubcats", { count: moved }));
      setMoveItemsModalOpen(false);
      setMoveItemsSource(null);
      setMoveItemsTarget(null);
      await loadData();
    } catch (e: any) {
      showToast(t("launcher.moveFail", { err: String(e) }));
    } finally {
      setMoveItemsLoading(false);
    }
  };
  return {
    moveItemsModalOpen,
    setMoveItemsModalOpen,
    moveItemsSource,
    moveItemsTarget,
    setMoveItemsTarget,
    moveItemsLoading,
    handleSaveCategory,
    pendingDeleteCategory,
    setPendingDeleteCategory,
    deletingCategory,
    deleteCascade,
    setDeleteCascade,
    deleteTarget,
    setDeleteTarget,
    deleteNewName,
    setDeleteNewName,
    scopeOf,
    handleDeleteCategory,
    confirmDeleteCategory,
    handleSaveItem,
    handleDeleteItem,
    moveItemsSourceCount,
    moveItemsTargetList,
    openMoveItemsModal,
    handleMoveItemsConfirm,
  };
}
