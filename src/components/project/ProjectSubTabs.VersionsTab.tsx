import React from "react";
import { useTranslation } from "react-i18next";
import { Check, Trash2, Loader, X } from "lucide-react";
import { type SubTabProps } from "./ProjectSubTabs.model";
import { RemoteVersionSelector } from "./ProjectSubTabs.RemoteVersionSelector";
// ═══════════════════════════════════════
//  版本管理
// ═══════════════════════════════════════
export function VersionsTab({
  project, remoteVersions, loadingRemote, installingVersion,
  onInstall, onUninstall, onUse,
  downloadProgress, installStep, onCancelInstall,
  versionsUpdatedAt, onRefreshRemoteVersions,
  isOperating,
}: SubTabProps) {
  const { t } = useTranslation();
  const currentVersionNumber = installingVersion
    ? (installingVersion.includes(" · ") ? installingVersion.split(" · ")[1] : installingVersion).trim().split(" ")[0]
    : "";

  return (
    <div className="space-y-6">
      {/* 安装进度面板 */}
      {installingVersion && (
        <div className="glass-panel rounded-panel p-5 border border-[var(--module-accent-ring)] bg-[color-mix(in_srgb,var(--module-accent)_5%,transparent)] space-y-4 animate-fadeIn">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Loader className="w-4 h-4 text-[var(--module-accent)] animate-spin" />
              <h4 className="text-body font-semibold text-[var(--module-accent)]">
                {t("projsub.installing", { name: project.display_name, ver: currentVersionNumber })}
              </h4>
            </div>
            {onCancelInstall && (
              <button
                onClick={onCancelInstall}
                className="flex items-center gap-1 px-2.5 py-1 rounded-ctl bg-red-500/10 hover:bg-red-500/20 text-red-400 hover:text-red-300 text-caption font-semibold border border-red-500/20 cursor-pointer transition-all"
                title={t("projsub.cancelInstall")}
              >
                <X className="w-3 h-3" /> {t("projsub.cancelInstall")}
              </button>
            )}
          </div>

          {/* 步骤指示器 */}
          <div className="flex items-center gap-1">
            {["stepDownload", "stepExtract", "stepLink", "stepDone"].map((stepKey, idx) => {
              const step = t(`projsub.${stepKey}`);
              const currentIdx = [t("projsub.stepDownload"), t("projsub.stepExtract"), t("projsub.stepLink"), t("projsub.stepDone")].indexOf(installStep);
              const isActive = step === installStep;
              const isCompleted = currentIdx > idx;
              return (
                <React.Fragment key={step}>
                  {idx > 0 && (
                    <div className={`flex-1 h-0.5 rounded-full ${isCompleted ? "bg-emerald-500" : isActive ? "bg-[var(--module-accent)]" : "bg-white/10"}`} />
                  )}
                  <div className="flex items-center gap-1.5">
                    <div className={`w-5 h-5 rounded-full flex items-center justify-center text-caption font-bold border ${isCompleted
                      ? "bg-emerald-500 text-white border-emerald-500"
                      : isActive
                        ? "bg-[var(--module-accent)] text-white border-[var(--module-accent)] animate-pulse"
                        : "bg-white/5 text-slate-500 border-white/10"
                      }`}>
                      {isCompleted ? <Check className="w-3 h-3" /> : idx + 1}
                    </div>
                    <span className={`text-title font-medium ${isActive ? "text-[var(--module-accent)]" : isCompleted ? "text-emerald-400" : "text-slate-500"}`}>
                      {step}
                    </span>
                  </div>
                </React.Fragment>
              );
            })}
          </div>

          {/* 下载进度条 */}
          {downloadProgress && installStep === t("projsub.stepDownload") && (
            <div className="space-y-2">
              <div className="flex items-center justify-between text-title">
                <span className="text-slate-400">{t("projsub.downloadProgress", { ver: currentVersionNumber })}</span>
                <div className="flex items-center gap-3">
                  {downloadProgress.speed_str && (
                    <span className="text-cyan-400 font-mono font-semibold text-caption">
                      ↓ {downloadProgress.speed_str}
                    </span>
                  )}
                  <span className="text-[var(--module-accent)] font-mono font-semibold">{downloadProgress.pct}%</span>
                </div>
              </div>
              <div className="w-full h-2 bg-white/5 rounded-full overflow-hidden">
                <div
                  className="h-full bg-gradient-to-r from-[var(--module-accent)] to-[var(--module-accent-strong)] rounded-full transition-all duration-300"
                  style={{ width: `${downloadProgress.pct}%` }}
                />
              </div>
              <div className="flex items-center justify-between text-title text-slate-500">
                <span>{(downloadProgress.downloaded / 1024 / 1024).toFixed(1)} MB</span>
                <span>{(downloadProgress.total / 1024 / 1024).toFixed(1)} MB</span>
              </div>
            </div>
          )}

          {/* 当前步骤文字说明 */}
          <p className="text-title text-slate-400">
            {installStep === t("projsub.stepDownload") && t("projsub.dlHint", { ver: currentVersionNumber })}
            {installStep === t("projsub.stepExtract") && t("projsub.extractHint", { ver: currentVersionNumber })}
            {installStep === t("projsub.stepLink") && t("projsub.linkHint", { ver: currentVersionNumber })}
            {installStep === t("projsub.stepDone") && t("projsub.doneHint", { ver: currentVersionNumber })}
          </p>
        </div>
      )}

      {/* 已安装版本 */}
      <div className="space-y-3">
        <div>
          <h4 className="text-body font-semibold text-slate-300">{t("projsub.localVersionsTitle")}</h4>
          <p className="text-title text-slate-500 mt-0.5">{t("projsub.localVersionsDesc")}</p>
        </div>
        {!project.installed_versions || project.installed_versions.length === 0 ? (
          <p className="text-caption text-slate-500">{t("projsub.noLocalVersions")}</p>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            {project.installed_versions.map((v) => {
              const isActive = project.active_version === v;
              return (
                <div
                  key={v}
                  className={`p-3 rounded-card border flex items-center justify-between transition-all ${isActive
                    ? "bg-[var(--module-accent-soft)] border-[var(--module-accent-ring)] text-white shadow-md shadow-[var(--module-accent-ring)]"
                    : "bg-black/20 border-white/5 text-slate-300"
                    }`}
                >
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-body font-medium">{v}</span>
                    {isActive && (
                      <span className="px-1.5 py-0.5 rounded text-caption bg-[var(--module-accent)] text-white font-bold">{t("projsub.current")}</span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5">
                    {!isActive && (
                      <button
                        onClick={() => onUse(v)}
                        disabled={isOperating || !project.managed || !project.delegation?.version_control}
                        className="p-1.5 hover:bg-white/10 disabled:opacity-40 disabled:cursor-not-allowed rounded-ctl text-slate-400 hover:text-slate-200 text-title cursor-pointer transition-all flex items-center gap-0.5"
                        title={
                          !project.managed
                            ? t("projsub.needHostToEnable")
                            : !project.delegation?.version_control
                            ? t("projsub.needVerCtrlEnable")
                            : t("projsub.enableThis")
                        }
                      >
                        <Check className="w-3.5 h-3.5" /> {t("projsub.enable")}
                      </button>
                    )}
                    <button
                      onClick={() => onUninstall(v)}
                      disabled={isOperating || !project.managed || !project.delegation?.version_control}
                      className="p-1.5 hover:bg-red-500/10 hover:text-red-400 disabled:opacity-40 disabled:cursor-not-allowed rounded-ctl text-slate-500 cursor-pointer transition-all"
                      title={
                        !project.managed
                          ? t("projsub.needHostToUninstall")
                          : !project.delegation?.version_control
                          ? t("projsub.needVerCtrlUninstall")
                          : t("projsub.uninstallThis")
                      }
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* 远程版本安装 */}
      <RemoteVersionSelector
        remoteVersions={remoteVersions}
        loadingRemote={loadingRemote}
        installingVersion={installingVersion}
        isOperating={isOperating}
        onInstall={onInstall}
        versionsUpdatedAt={versionsUpdatedAt}
        onRefresh={onRefreshRemoteVersions}
        disabled={!project.managed || !project.delegation?.version_control}
        disabledReason={
          !project.managed
            ? t("projsub.persistWarning1")
            : t("projsub.persistWarning2")
        }
      />


    </div>
  );
}
