import { ChevronsUp, ChevronsDown } from "lucide-react";
import { useTranslation } from "react-i18next";

interface LauncherCollapseButtonProps {
  handleToggleCollapseAll: () => void;
  activeSubIds: number[];
  isAllCollapsed: boolean;
}


/** Pure LauncherCollapseButton view; its owning panel retains side effects and state. */
export function LauncherCollapseButton({ handleToggleCollapseAll, activeSubIds, isAllCollapsed }: LauncherCollapseButtonProps) {
  const { t } = useTranslation();
  return (
<button
            onClick={handleToggleCollapseAll}
            disabled={activeSubIds.length === 0}
            className={`px-2.5 py-1 rounded-ctl text-body flex items-center gap-1.5 border transition cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed ${
              isAllCollapsed
                ? "bg-[var(--module-accent)]/15 border-[var(--module-accent)]/40 text-[var(--module-accent)]"
                : "ui-btn"
            }`}
            title={isAllCollapsed ? t("launcher.expandAllTitle") : t("launcher.collapseAllTitle")}
          >
            {isAllCollapsed ? (
              <ChevronsDown className="w-3 h-3 text-cyan-400" />
            ) : (
              <ChevronsUp className="w-3 h-3 text-cyan-400" />
            )}
            <span className="text-caption">{isAllCollapsed ? t("launcher.expandAll") : t("launcher.collapseAll")}</span>
          </button>
  );
}
