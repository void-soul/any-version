import { Shield, Trash2, Edit2, ExternalLink, FolderOpen, Copy } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { Item } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface LauncherItemMenuProps {
  itemContextMenu: { x: number; y: number; item: Item; };
  handleExecuteItem: (item: Item) => Promise<void>;
  setItemContextMenu: Dispatch<SetStateAction<{ x: number; y: number; item: Item; } | null>>;
  showToast: (msg: string) => void;
  setEditingItem: Dispatch<SetStateAction<Item | null>>;
  setTargetClassificationId: Dispatch<SetStateAction<number>>;
  setItemModalOpen: Dispatch<SetStateAction<boolean>>;
  handleDeleteItem: (id: number) => Promise<void>;
}


/** Pure LauncherItemMenu view; its owning panel retains side effects and state. */
export function LauncherItemMenu({ itemContextMenu, handleExecuteItem, setItemContextMenu, showToast, setEditingItem, setTargetClassificationId, setItemModalOpen, handleDeleteItem }: LauncherItemMenuProps) {
  const { t } = useTranslation();
  return (
<div
          style={{
            left: itemContextMenu.x,
            ...(itemContextMenu.y > (typeof window !== "undefined" ? window.innerHeight : 0) - 280
              ? { bottom: (typeof window !== "undefined" ? window.innerHeight : 0) - itemContextMenu.y, top: "auto" }
              : { top: itemContextMenu.y }),
          }}
          className="fixed z-[200] bg-surface-panel border border-white/15 rounded-card shadow-2xl p-1.5 min-w-[160px] text-body space-y-0.5 animate-in fade-in zoom-in-95 duration-100"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            onClick={() => {
              handleExecuteItem(itemContextMenu.item);
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-200 hover:bg-[var(--module-accent)] hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <ExternalLink className="w-3.5 h-3.5" />
            <span>{t("launcher.open")}</span>
          </button>
          <button
            onClick={() => {
              handleExecuteItem({
                ...itemContextMenu.item,
                data: { ...itemContextMenu.item.data, runAsAdmin: true },
              });
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-amber-300 hover:bg-amber-600 hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Shield className="w-3.5 h-3.5" />
            <span>{t("launcher.runAsAdmin")}</span>
          </button>
          {itemContextMenu.item.data.target && (
            <button
              onClick={() => {
                invoke("launcher_open_file_location", { path: itemContextMenu.item.data.target });
                setItemContextMenu(null);
              }}
              className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
            >
              <FolderOpen className="w-3.5 h-3.5" />
              <span>{t("launcher.openFileLocation")}</span>
            </button>
          )}
          {itemContextMenu.item.data.target && (
            <button
              onClick={() => {
                navigator.clipboard.writeText(itemContextMenu.item.data.target || "");
                showToast(t("launcher.copiedPath"));
                setItemContextMenu(null);
              }}
              className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
            >
              <Copy className="w-3.5 h-3.5" />
              <span>{t("launcher.copyPath")}</span>
            </button>
          )}
          <div className="h-px bg-white/10 my-1" />
          <button
            onClick={() => {
              setEditingItem(itemContextMenu.item);
              setTargetClassificationId(itemContextMenu.item.classificationId);
              setItemModalOpen(true);
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <Edit2 className="w-3.5 h-3.5" />
            <span>{t("launcher.editProperties")}</span>
          </button>
          <button
            onClick={() => {
              handleDeleteItem(itemContextMenu.item.id);
              setItemContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-red-400 hover:bg-red-500 hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Trash2 className="w-3.5 h-3.5" />
            <span>{t("launcher.deleteItem")}</span>
          </button>
        </div>
  );
}
