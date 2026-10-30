import { Plus, Shield, ExternalLink, FolderOpen, Copy } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { ScannedProgram } from "./types";
import type { Dispatch, SetStateAction } from "react";

interface LauncherFolderMenuProps {
  folderContextMenu: { x: number; y: number; item: ScannedProgram; };
  launchVirtual: (name: string, target: string, itemType: number, params?: string | null | undefined, icon?: string | null | undefined) => Promise<void>;
  setFolderContextMenu: Dispatch<SetStateAction<{ x: number; y: number; item: ScannedProgram; } | null>>;
  showToast: (msg: string) => void;
  addFolderItemToClassification: (p: ScannedProgram) => Promise<void>;
}


/** Pure LauncherFolderMenu view; its owning panel retains side effects and state. */
export function LauncherFolderMenu({ folderContextMenu, launchVirtual, setFolderContextMenu, showToast, addFolderItemToClassification }: LauncherFolderMenuProps) {
  const { t } = useTranslation();
  return (
<div
          style={{
            left: folderContextMenu.x,
            ...(folderContextMenu.y > (typeof window !== "undefined" ? window.innerHeight : 0) - 240
              ? { bottom: (typeof window !== "undefined" ? window.innerHeight : 0) - folderContextMenu.y, top: "auto" }
              : { top: folderContextMenu.y }),
          }}
          className="fixed z-[200] bg-surface-panel border border-white/15 rounded-card shadow-2xl p-1.5 min-w-[170px] text-body space-y-0.5 animate-in fade-in zoom-in-95 duration-100"
          onClick={(e) => e.stopPropagation()}
        >
          <button
            onClick={() => {
              const p = folderContextMenu.item;
              launchVirtual(p.name, p.target, p.isDir ? 1 : 0, p.params, p.icon);
              setFolderContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-200 hover:bg-[var(--module-accent)] hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <ExternalLink className="w-3.5 h-3.5" />
            <span>{t("launcher.open")}</span>
          </button>
          {!folderContextMenu.item.isDir && (
            <button
              onClick={async () => {
                const p = folderContextMenu.item;
                try {
                  await invoke("launcher_execute_raw", {
                    file: p.target,
                    params: p.params || null,
                    runAsAdmin: true,
                    startLocation: null,
                  });
                  setFolderContextMenu(null);
                } catch (e) {
                  showToast(t("launcher.launchFail", { err: String(e) }));
                }
              }}
              className="w-full px-3 py-1.5 rounded-ctl text-left text-amber-300 hover:bg-amber-600 hover:text-white flex items-center gap-2 cursor-pointer"
            >
              <Shield className="w-3.5 h-3.5" />
              <span>{t("launcher.runAsAdmin")}</span>
            </button>
          )}
          <button
            onClick={() => {
              invoke("launcher_open_file_location", { path: folderContextMenu.item.target }).catch(
                (e) => showToast(t("launcher.launchFail", { err: String(e) }))
              );
              setFolderContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <FolderOpen className="w-3.5 h-3.5" />
            <span>{t("launcher.openFileLocation")}</span>
          </button>
          <button
            onClick={() => {
              navigator.clipboard.writeText(folderContextMenu.item.target);
              showToast(t("launcher.copiedPath"));
              setFolderContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-slate-300 hover:bg-white/10 flex items-center gap-2 cursor-pointer"
          >
            <Copy className="w-3.5 h-3.5" />
            <span>{t("launcher.copyPath")}</span>
          </button>
          <div className="h-px bg-white/10 my-1" />
          <button
            onClick={() => {
              addFolderItemToClassification(folderContextMenu.item);
              setFolderContextMenu(null);
            }}
            className="w-full px-3 py-1.5 rounded-ctl text-left text-[var(--module-accent)] hover:bg-[var(--module-accent)] hover:text-white flex items-center gap-2 cursor-pointer"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>{t("launcher.addAsItem")}</span>
          </button>
        </div>
  );
}
