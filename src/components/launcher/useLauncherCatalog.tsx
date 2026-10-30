import { useState, useEffect, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Classification, Item, LauncherSetting } from "./types";
import type { Dispatch, SetStateAction, RefObject } from "react";

/** Owns the LauncherCatalog feature, independently of page rendering. */
export function useLauncherCatalog({ collapseInitRef, setCollapsedGroups }: {
  collapseInitRef: RefObject<boolean>;
  setCollapsedGroups: Dispatch<SetStateAction<Set<number>>>;
}) {

  const [classifications, setClassifications] = useState<Classification[]>([]);

  const [activeParentId, setActiveParentId] = useState<number | null>(null);

  const [allItems, setAllItems] = useState<Item[]>([]);

  // 拖拽落地时要用「最新一次 applyMoveLocal 之后」的完整列表计算序号，
  // 渲染闭包可能不是最新的，这里维护一份随渲染同步的引用。
  const allItemsRef = useRef<Item[]>([]);

  useEffect(() => {
    allItemsRef.current = allItems;
  }, [allItems]);


  // Settings
  const [settings, setSettings] = useState<LauncherSetting>({
    moduleHotkeys: { launcher: "Alt+Space" },
    itemIconSize: 32,
    itemColumnNumber: 0,
    cardDensity: "cozy",
    showItemName: true,
    iconBackgroundColor: false,
    itemFontSize: 12,
    itemRadius: 12,
    itemBorder: true,
    categoryFontSize: 12,
    categoryGap: 24,
  });


  // Top-level categories (left sidebar) —— 必须先于 topCategoryIdsRef 的 useEffect（避免 TDZ）
  const topCategories = useMemo(() => {
    return classifications.filter((c) => !c.parentId);
  }, [classifications]);


  // Load Data
  const loadData = async () => {
    try {
      const clsList = await invoke<Classification[]>("launcher_get_classifications");
      setClassifications(clsList);

      // 首次加载：按各分类「默认收缩全部子分组」设置初始化折叠状态
      if (!collapseInitRef.current && clsList.length > 0) {
        const defaults = new Set<number>();
        for (const c of clsList) {
          if (c.data?.defaultCollapsed) {
            for (const child of clsList) {
              if (child.parentId === c.id) defaults.add(child.id);
            }
          }
        }
        if (defaults.size > 0) {
          setCollapsedGroups(defaults);
        }
        collapseInitRef.current = true;
      }

      const items = await invoke<Item[]>("launcher_get_all_items");
      setAllItems(items);

      const loadedSettings = await invoke<LauncherSetting>("launcher_get_settings");
      setSettings(loadedSettings);

      // Default active parent category (first top-level classification)
      if (clsList.length > 0) {
        if (!activeParentId || !clsList.some((c) => c.id === activeParentId)) {
          setActiveParentId(clsList[0].id);
        }
      }
    } catch (e) {
      console.error("加载启动器数据失败:", e);
    }
  };


  useEffect(() => {
    loadData();
  }, []);
  return {
    classifications,
    activeParentId,
    setActiveParentId,
    allItems,
    setAllItems,
    allItemsRef,
    settings,
    setSettings,
    topCategories,
    loadData,
  };
}
