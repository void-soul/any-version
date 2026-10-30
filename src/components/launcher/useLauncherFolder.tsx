import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Classification, Item, ScannedProgram } from "./types";

/** Owns the LauncherFolder feature, independently of page rendering. */
export function useLauncherFolder({ isFolderCategory, activeTopCategory, activeFolderPath, showToast, loadData }: {
  isFolderCategory: boolean;
  activeTopCategory: Classification;
  activeFolderPath: string;
  showToast: (msg: string) => void;
  loadData: () => Promise<void>;
}) {
  const { t } = useTranslation();


  // 关联文件夹分类：实时扫描出来的「虚拟项目」（不落库）
  const [folderItems, setFolderItems] = useState<ScannedProgram[]>([]);

  const [folderLoading, setFolderLoading] = useState(false);

  const [folderError, setFolderError] = useState<string | null>(null);

  const [folderReloadKey, setFolderReloadKey] = useState(0);

  const [folderContextMenu, setFolderContextMenu] = useState<{
    x: number;
    y: number;
    item: ScannedProgram;
  } | null>(null);


  // 关联文件夹分类：扫描结果转成只读卡片（id 为负数，仅用于渲染 key）
  const folderVirtualItems = useMemo<Item[]>(() => {
    if (!isFolderCategory) return [];
    return folderItems.map((p, idx) => ({
      id: -(idx + 1),
      classificationId: activeTopCategory?.id ?? 0,
      name: p.name,
      itemType: p.isDir ? 1 : 0,
      data: {
        target: p.target,
        params: p.params ?? undefined,
        icon: p.icon ?? undefined,
      },
      shortcutKey: null,
      globalShortcutKey: false,
      order: idx,
    }));
  }, [isFolderCategory, folderItems, activeTopCategory]);


  // 关联文件夹：进入分类时扫描目录；刷新按钮通过 folderReloadKey 重新触发
  useEffect(() => {
    if (!isFolderCategory || !activeFolderPath) {
      setFolderItems([]);
      setFolderError(null);
      setFolderLoading(false);
      return;
    }
    let cancelled = false;
    setFolderLoading(true);
    setFolderError(null);
    invoke<ScannedProgram[]>("launcher_scan_folder", {
      path: activeFolderPath,
      hiddenItems: activeTopCategory?.data?.associateFolderHiddenItems ?? null,
      showOnly: activeTopCategory?.data?.itemShowOnly ?? "default",
    })
      .then((list) => {
        if (!cancelled) setFolderItems(list);
      })
      .catch((e) => {
        if (!cancelled) {
          setFolderItems([]);
          setFolderError(String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setFolderLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    isFolderCategory,
    activeFolderPath,
    activeTopCategory?.id,
    activeTopCategory?.data?.associateFolderHiddenItems,
    activeTopCategory?.data?.itemShowOnly,
    folderReloadKey,
  ]);


  /** 把关联文件夹里的某个条目保存为当前分类下的正式项目。 */
  const addFolderItemToClassification = async (p: ScannedProgram) => {
    if (!activeTopCategory) return;
    try {
      await invoke("launcher_save_item", {
        item: {
          id: 0,
          classificationId: activeTopCategory.id,
          name: p.name,
          itemType: p.isDir ? 1 : 0,
          data: {
            target: p.target,
            params: p.params || undefined,
            icon: p.icon || undefined,
          },
          shortcutKey: null,
          globalShortcutKey: false,
          order: 0,
        },
      });
      showToast(t("launcher.itemSaved"));
      await loadData();
    } catch (e) {
      showToast(t("launcher.addFail", { err: String(e) }));
    }
  };
  return {
    folderItems,
    folderLoading,
    folderError,
    setFolderReloadKey,
    folderContextMenu,
    setFolderContextMenu,
    folderVirtualItems,
    addFolderItemToClassification,
  };
}
