import type { ScannedProgram } from "./types";
import { useEffect } from "react";
import { Classification, Item } from "./types";
import type { Dispatch, SetStateAction, RefObject } from "react";

/** Owns the LauncherMenuDismissal feature, independently of page rendering. */
export function useLauncherMenuDismissal({ setItemContextMenu, setCategoryContextMenu, setFolderContextMenu, viewSettingsRef, setViewSettingsOpen }: {
  setItemContextMenu: Dispatch<SetStateAction<{ x: number; y: number; item: Item; } | null>>;
  setCategoryContextMenu: Dispatch<SetStateAction<{ x: number; y: number; category: Classification; } | null>>;
  setFolderContextMenu: Dispatch<SetStateAction<{ x: number; y: number; item: ScannedProgram; } | null>>;
  viewSettingsRef: RefObject<HTMLDivElement | null>;
  setViewSettingsOpen: Dispatch<SetStateAction<boolean>>;
}) {


  // Keyboard shortcut listener (TEMPORARILY COMMENTED OUT FOR TESTING)

  // Close context menus on global click
  useEffect(() => {
    const closeMenu = () => {
      setItemContextMenu(null);
      setCategoryContextMenu(null);
      setFolderContextMenu(null);
    };
    const onClickOutside = (e: MouseEvent) => {
      if (viewSettingsRef.current && !viewSettingsRef.current.contains(e.target as Node)) {
        setViewSettingsOpen(false);
      }
    };
    window.addEventListener("click", closeMenu);
    document.addEventListener("mousedown", onClickOutside);
    return () => {
      window.removeEventListener("click", closeMenu);
      document.removeEventListener("mousedown", onClickOutside);
    };
  }, []);

}
