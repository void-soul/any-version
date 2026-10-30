import { ScanSearch, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Item } from "./types";

interface LauncherCheckButtonProps {
  runCheck: () => Promise<void>;
  checking: boolean;
  allItems: Item[];
  missingCount: number;
}


/** Pure LauncherCheckButton view; its owning panel retains side effects and state. */
export function LauncherCheckButton({ runCheck, checking, allItems, missingCount }: LauncherCheckButtonProps) {
  const { t } = useTranslation();
  return (
<button
            onClick={runCheck}
            disabled={checking || allItems.length === 0}
            className={`px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border transition cursor-pointer ${
              missingCount > 0
                ? "bg-red-600/20 border-red-500/40 text-red-300 hover:bg-red-600/30"
                : "ui-btn"
            } ${checking ? "opacity-60 cursor-wait" : ""}`}
            title={t("launcher.checkTitle")}
          >
            {checking ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <ScanSearch className="w-3 h-3 text-emerald-400" />
            )}
            <span className="text-caption">{checking ? t("launcher.checking") : t("launcher.check")}</span>
            {missingCount > 0 && !checking && (
              <span className="w-3.5 h-3.5 rounded-full bg-red-500 text-micro text-white flex items-center justify-center font-bold">
                {missingCount}
              </span>
            )}
          </button>
  );
}
