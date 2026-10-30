import { UploadCloud } from "lucide-react";
import { useTranslation } from "react-i18next";

interface LauncherDropIndicatorProps {
  currentDragTargetName: string;
}


/** Pure LauncherDropIndicator view; its owning panel retains side effects and state. */
export function LauncherDropIndicator({ currentDragTargetName }: LauncherDropIndicatorProps) {
  const { t } = useTranslation();
  return (
<div className="absolute inset-x-0 top-12 z-50 flex justify-center pointer-events-none animate-in fade-in slide-in-from-top-2 duration-150">
          <div className="bg-[color-mix(in_srgb,var(--module-accent)_90%,transparent)] backdrop-blur-md border border-[var(--module-accent)] text-white px-5 py-2.5 rounded-panel shadow-[0_0_16px_color-mix(in_srgb,var(--module-accent)_45%,transparent),0_0_40px_color-mix(in_srgb,var(--module-accent)_25%,transparent)] flex items-center gap-2.5">
            <UploadCloud className="w-5 h-5 text-[var(--module-accent)] animate-bounce" />
            <div className="text-body">
              <span className="font-bold">{t("launcher.dropAdd", { name: currentDragTargetName })}</span>
              <span className="text-[var(--module-accent)] text-tiny ml-1.5">{t("launcher.dropSupport")}</span>
            </div>
          </div>
        </div>
  );
}
