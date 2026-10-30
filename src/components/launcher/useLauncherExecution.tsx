import { useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { Classification, Item } from "./types";
import { comboFromKeyboardEvent, normalizeCombo } from "./hotkey";
import { topAncestorId } from "./LauncherPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the LauncherExecution feature, independently of page rendering. */
export function useLauncherExecution({ loadData, showToast, classificationMap, setActiveParentId, classifications, setIsSearchOpen, setSearchQuery, isSearchOpen, allItems, activeTopCategory }: {
  loadData: () => Promise<void>;
  showToast: (msg: string) => void;
  classificationMap: Map<number, Classification>;
  setActiveParentId: Dispatch<SetStateAction<number | null>>;
  classifications: Classification[];
  setIsSearchOpen: Dispatch<SetStateAction<boolean>>;
  setSearchQuery: Dispatch<SetStateAction<string>>;
  isSearchOpen: boolean;
  allItems: Item[];
  activeTopCategory: Classification;
}) {
  const { t } = useTranslation();


  // Execute Item launch (单击秒开，打开后隐藏窗口)
  const handleExecuteItem = async (item: Item) => {
    try {
      await invoke("launcher_execute_item", {
        itemId: item.id > 0 ? item.id : null,
        item,
      });
      loadData();
      try {
        const win = getCurrentWindow();
        await win.hide();
      } catch (hideErr) {
        console.warn("Hide window on launch:", hideErr);
      }
    } catch (e) {
      showToast(t("launcher.launchFail", { err: String(e) }));
    }
  };


  /** 切到某个分类所在的顶级分类（分类快捷键 / 聚合分类跳转共用）。 */
  const focusCategoryById = useCallback(
    (id: number) => {
      if (!classificationMap.has(id)) return;
      setActiveParentId(topAncestorId(classifications, id));
      setIsSearchOpen(false);
      setSearchQuery("");
    },
    [classificationMap, classifications]
  );


  // 分类全局快捷键：后端唤起窗口后通知前端定位；面板重挂载时再取一次 pending 兜底
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    const setup = async () => {
      try {
        const pending = await invoke<number | null>("launcher_take_focus_category");
        if (!cancelled && pending != null) focusCategoryById(pending);
      } catch {
        /* 忽略：没有 pending 分类 */
      }
      unlisten = await listen<number>("launcher-focus-category", (event) => {
        const id = Number(event.payload);
        if (Number.isFinite(id)) focusCategoryById(id);
      });
    };
    setup().catch((e) => console.error("监听分类快捷键失败:", e));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [focusCategoryById]);


  // 窗口内快捷键（未勾选「全局」的项目/分类快捷键）
  const executeItemRef = useRef(handleExecuteItem);

  useEffect(() => {
    executeItemRef.current = handleExecuteItem;
  });

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (isSearchOpen || e.repeat || e.defaultPrevented) return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      const combo = comboFromKeyboardEvent(e);
      if (!combo) return;
      const item = allItems.find(
        (it) =>
          it.shortcutKey &&
          !it.globalShortcutKey &&
          normalizeCombo(it.shortcutKey) === combo
      );
      if (item) {
        e.preventDefault();
        executeItemRef.current(item);
        return;
      }
      const cat = classifications.find(
        (c) =>
          c.shortcutKey &&
          !c.globalShortcutKey &&
          normalizeCombo(c.shortcutKey) === combo
      );
      if (cat) {
        e.preventDefault();
        focusCategoryById(cat.id);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [allItems, classifications, isSearchOpen, focusCategoryById]);


  /** 启动一个「虚拟项目」（关联文件夹里的文件、搜索直达的网址）。 */
  const launchVirtual = async (
    name: string,
    target: string,
    itemType: number,
    params?: string | null,
    icon?: string | null
  ) => {
    try {
      const item: Item = {
        id: 0,
        classificationId: activeTopCategory?.id ?? 0,
        name,
        itemType,
        data: { target, params: params || undefined, icon: icon || undefined },
        shortcutKey: null,
        globalShortcutKey: false,
        order: 0,
      };
      await invoke("launcher_execute_item", { itemId: null, item });
      try {
        await getCurrentWindow().hide();
      } catch {
        /* 忽略隐藏失败 */
      }
    } catch (e) {
      showToast(t("launcher.launchFail", { err: String(e) }));
    }
  };
  return {
    handleExecuteItem,
    launchVirtual,
  };
}
