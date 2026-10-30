import { useTranslation } from "react-i18next";
import VexBusy from "../VexBusy";

interface LauncherCheckProgressProps {
  checkProgress: { done: number; total: number; name: string; };
  checking: boolean;
  stopCheck: () => void;
}


/** Pure LauncherCheckProgress view; its owning panel retains side effects and state. */
export function LauncherCheckProgress({ checkProgress, checking, stopCheck }: LauncherCheckProgressProps) {
  const { t } = useTranslation();
  return (
<div className="flex items-center gap-2 px-4 py-1.5 bg-white/[0.03] border-b border-white/5 flex-shrink-0 animate-in slide-in-from-top-2 duration-150">
          <VexBusy
            avatarSize={26}
            text={checkProgress.name ? <>{t("launcher.checkingItem")} <span className="text-[var(--module-accent)]">{checkProgress.name}</span> · {checkProgress.done}/{checkProgress.total}</> : `${t("launcher.checkingItem")} ${checkProgress.done}/${checkProgress.total}…`}
          />
          {checking && (
            <button
              onClick={stopCheck}
              className="shrink-0 px-2 py-0.5 rounded-md text-tiny font-medium bg-red-600/20 border border-red-500/40 text-red-300 hover:bg-red-600/30 transition cursor-pointer"
              title={t("launcher.stopCheckTitle")}
            >
              {t("launcher.stop")}
            </button>
          )}
        </div>
  );
}
