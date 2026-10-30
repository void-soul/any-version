import { Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Item } from "./types";

interface LauncherCleanupButtonProps {
  cleanupInvalidItems: () => Promise<void>;
  checking: boolean;
  missingCount: number;
  allItems: Item[];
}


/** Pure LauncherCleanupButton view; its owning panel retains side effects and state. */
export function LauncherCleanupButton({ cleanupInvalidItems, checking, missingCount, allItems }: LauncherCleanupButtonProps) {
  const { t } = useTranslation();
  return (
<button
            onClick={() => void cleanupInvalidItems()}
            disabled={
              checking ||
              (missingCount === 0 && !allItems.some((it) => it.data.exists === false))
            }
            className={`px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border transition cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed ${
              missingCount > 0
                ? "bg-red-600/20 border-red-500/40 text-red-300 hover:bg-red-600/30"
                : "ui-btn"
            }`}
            title={t("launcher.cleanInvalidTitle")}
          >
            <Trash2 className="w-3 h-3 text-red-400" />
            <span className="text-caption">{t("launcher.cleanInvalid")}</span>
          </button>
  );
}
