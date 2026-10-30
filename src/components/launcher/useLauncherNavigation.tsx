import type { LauncherSetting } from "./types";
import { useMemo } from "react";
import { Classification } from "./types";
import { DEFAULT_WEB_SOURCES } from "./LauncherPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the LauncherNavigation feature, independently of page rendering. */
export function useLauncherNavigation({ topCategories, activeParentId, classifications, settings, collapsedGroups, setCollapsedGroups }: {
  topCategories: Classification[];
  activeParentId: number | null;
  classifications: Classification[];
  settings: LauncherSetting;
  collapsedGroups: Set<number>;
  setCollapsedGroups: Dispatch<SetStateAction<Set<number>>>;
}) {


  // Current active top-level category
  const activeTopCategory = useMemo(() => {
    return topCategories.find((c) => c.id === activeParentId) || topCategories[0];
  }, [topCategories, activeParentId]);


  // Sub-categories under current active parent category
  const subCategories = useMemo(() => {
    if (!activeTopCategory) return [];
    return classifications.filter((c) => c.parentId === activeTopCategory.id);
  }, [classifications, activeTopCategory]);


  // 当前分类的特殊类型（1=关联文件夹，2=聚合分类）
  const isFolderCategory = activeTopCategory?.classificationType === 1;

  const isAggregateCategory = activeTopCategory?.classificationType === 2;

  const activeFolderPath = activeTopCategory?.data?.associateFolderPath?.trim() || "";

  const webSources =
    settings.webSearchSources && settings.webSearchSources.length > 0
      ? settings.webSearchSources
      : DEFAULT_WEB_SOURCES;


  // 当前大分类下的所有子分组 id（含嵌套递归），用于「收缩/展开全部」
  const activeSubIds = useMemo(() => {
    if (!activeTopCategory) return [];
    const result: number[] = [];
    const walk = (parentId: number) => {
      for (const c of classifications) {
        if (c.parentId === parentId) {
          result.push(c.id);
          walk(c.id);
        }
      }
    };
    walk(activeTopCategory.id);
    return result;
  }, [classifications, activeTopCategory]);


  // 是否当前大分类下的所有子分组均已收缩
  const isAllCollapsed = useMemo(() => {
    if (activeSubIds.length === 0) return false;
    return activeSubIds.every((id) => collapsedGroups.has(id));
  }, [activeSubIds, collapsedGroups]);


  // 切换：全部收缩 / 全部展开
  const handleToggleCollapseAll = () => {
    if (activeSubIds.length === 0) return;
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (isAllCollapsed) {
        activeSubIds.forEach((id) => next.delete(id));
      } else {
        activeSubIds.forEach((id) => next.add(id));
      }
      return next;
    });
  };
  return {
    activeTopCategory,
    subCategories,
    isFolderCategory,
    isAggregateCategory,
    activeFolderPath,
    webSources,
    activeSubIds,
    isAllCollapsed,
    handleToggleCollapseAll,
  };
}
