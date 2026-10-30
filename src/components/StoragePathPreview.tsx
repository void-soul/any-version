import { useTranslation } from "react-i18next";

interface StoragePathPreviewProps {
  dataDir: string;
}

/** Pure StoragePathPreview view; its owning panel retains side effects and state. */
export function StoragePathPreview({ dataDir }: StoragePathPreviewProps) {
  const { t } = useTranslation();
  return (
    <div className="rounded-card border border-white/5 bg-white/[0.02] p-3 space-y-1.5">
      <p className="text-tiny text-slate-500 uppercase font-semibold">
        {t("settings.derivedDirs")}
      </p>
      <div className="text-tiny font-mono text-slate-400 space-y-1">
        <div className="flex items-center gap-2">
          <span className="text-cyan-400 flex-shrink-0">{t("settings.sdkDir")}</span>
          <span className="truncate">
            {(dataDir || "…").replace(/[\\/]+$/, "")}\sdk
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-emerald-400 flex-shrink-0">
            {t("settings.nodeServices")}
          </span>
          <span className="truncate">
            {(dataDir || "…").replace(/[\\/]+$/, "")}\node-projects
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-amber-400 flex-shrink-0">{t("settings.certs")}</span>
          <span className="truncate">
            {(dataDir || "…").replace(/[\\/]+$/, "")}\certs
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="text-slate-400 flex-shrink-0">
            {t("settings.cacheDb")}
          </span>
          <span className="truncate">
            {(dataDir || "…").replace(/[\\/]+$/, "")}
            {t("settings.derivedTail")}
          </span>
        </div>
      </div>
    </div>
  );
}
