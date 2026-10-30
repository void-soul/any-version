import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle, Activity, Info } from "lucide-react";
import type { ServiceStatus } from "./types";
import { type SubTabProps } from "./ProjectSubTabs.model";
// ═══════════════════════════════════════
//  服务管理
// ═══════════════════════════════════════
export function ServicesTab({ project, def, serviceCtrlLoading, onServiceToggle, onActiveSubTabChange }: SubTabProps) {
  const { t } = useTranslation();
  const [isAdmin, setIsAdmin] = useState(true);

  // 当切换到 services 标签页时通知父组件
  useEffect(() => {
    onActiveSubTabChange?.("services");
    invoke<boolean>("is_admin").then(setIsAdmin).catch(() => setIsAdmin(true));
  }, []);

  const svc: ServiceStatus | null = project.service_status ?? null;
  if (!svc) {
    return (
      <div className="p-8 text-center text-slate-500">
        <Activity className="w-10 h-10 mx-auto text-slate-600 mb-3" />
        <p className="text-body font-medium text-slate-400">{t("projsub.noSvcInfo")}</p>
        <p className="text-title text-slate-500 mt-1">{t("projsub.noSvcDesc")}</p>
      </div>
    );
  }

  const status = svc.status || (svc.running ? "running" : "stopped");
  const externallyRunning = status === "external_running" || svc.external === true;
  const hasConflict = status === "port_conflict";
  const notInstalled = status === "not_installed";
  const canToggle = !serviceCtrlLoading && !hasConflict && !notInstalled && !externallyRunning;

  return (
    <div className="space-y-4">
      {!isAdmin && svc?.system_service_name && (
        <div className="flex items-start gap-2.5 p-3 rounded-card border border-amber-500/20 bg-amber-500/10 text-[12.5px] text-amber-200 animate-fadeIn">
          <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
          <span>
            <strong>{t("projsub.svcPermHint")}</strong>{t("projsub.svcPermDesc", { name: svc.system_service_name })}
          </span>
        </div>
      )}

      <div className="glass-panel border border-white/5 rounded-panel p-5 bg-white/2 space-y-4">
        <div className="flex items-center gap-2 border-b border-white/5 pb-3">
          <Activity className="w-4 h-4 text-[var(--module-accent)]" />
          <h4 className="text-body font-semibold text-white">{t("projsub.svcConsole")}</h4>
        </div>

        {externallyRunning && (
          <div className="p-3 rounded-card border border-sky-500/20 bg-sky-500/10 text-body text-sky-200 flex items-start gap-2">
            <Info className="w-4 h-4 text-sky-400 flex-shrink-0 mt-0.5" />
            <span>{t("projsub.svcExternal", { proc: svc.process_name ? t("projsub.procInfo", { name: svc.process_name, pid: svc.pid ? t("projsub.pidInfo", { pid: svc.pid }) : "" }) : "" })}</span>
          </div>
        )}

        {hasConflict && (
          <div className="p-3 rounded-card border border-amber-500/20 bg-amber-500/10 text-body text-amber-200 flex items-start gap-2">
            <AlertTriangle className="w-4 h-4 text-amber-400 flex-shrink-0 mt-0.5" />
            <span>{t("projsub.portOccupied", { port: svc.port || def?.default_port || "?", proc: svc.process_name || t("projsub.otherProcess") })}</span>
          </div>
        )}

        {notInstalled && (
          <div className="p-3 rounded-card border border-slate-500/20 bg-slate-500/10 text-body text-slate-300 flex items-start gap-2">
            <Info className="w-4 h-4 text-slate-400 flex-shrink-0 mt-0.5" />
            <span>{t("projsub.noInstallDir")}</span>
          </div>
        )}

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4 text-body">
          <div className="p-3 bg-black/20 rounded-card border border-white/5 space-y-1.5">
            <span className="text-title text-slate-400 font-semibold uppercase tracking-wider block">{t("projsub.curStatus")}</span>
            <div className="flex items-center gap-2">
              {externallyRunning ? (
                <span className="px-2.5 py-1 rounded-ctl bg-sky-500/10 text-sky-300 border border-sky-500/20 font-semibold flex items-center gap-1 animate-fadeIn">
                  <span className="w-1.5 h-1.5 rounded-full bg-sky-400 animate-ping" />
                  {t("projsub.extRunning", { pid: svc.pid ? t("projsub.runningPid", { pid: svc.pid }) : "" })}
                </span>
              ) : svc.running ? (
                <span className="px-2.5 py-1 rounded-ctl bg-emerald-500/10 text-emerald-400 border border-emerald-500/20 font-semibold flex items-center gap-1 animate-fadeIn">
                  <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-ping" />
                  {t("projsub.running", { pid: svc.pid ? t("projsub.runningPid", { pid: svc.pid }) : "" })}
                </span>
              ) : hasConflict ? (
                <span className="px-2.5 py-1 rounded-ctl bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">{t("projsub.portConflictBadge")}</span>
              ) : notInstalled ? (
                <span className="px-2.5 py-1 rounded-ctl bg-slate-500/10 text-slate-400 border border-white/5 font-semibold">{t("projsub.notSet")}</span>
              ) : (
                <span className="px-2.5 py-1 rounded-ctl bg-slate-500/10 text-slate-400 border border-white/5 font-semibold">{t("projsub.stopped")}</span>
              )}
            </div>
          </div>

          <div className="p-3 bg-black/20 rounded-card border border-white/5 space-y-1">
            <span className="text-title text-slate-400 font-semibold uppercase tracking-wider block">{t("projsub.runParams")}</span>
            <div className="text-slate-300 font-mono space-y-0.5">
              <p>{t("projsub.port", { port: svc.port || def?.default_port || "-" })}</p>
              <p>{t("projsub.process", { name: svc.process_name || t("projsub.notDetected") })}</p>
            </div>
          </div>

          <div className="p-3 bg-black/20 rounded-card border border-white/5 flex items-center justify-center gap-2">
            <button
              onClick={onServiceToggle}
              disabled={!canToggle}
              className={`px-4 py-2 ${svc.running ? "bg-red-600 hover:bg-red-500" : "bg-emerald-600 hover:bg-emerald-500"} disabled:opacity-50 disabled:cursor-not-allowed text-white font-semibold rounded-card text-body cursor-pointer shadow-md transition-all flex items-center gap-1`}
            >
              {serviceCtrlLoading ? t("projsub.operating") : externallyRunning ? t("projsub.externalRun") : svc.running ? t("projsub.stopSvc") : t("projsub.startSvc")}
            </button>
          </div>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-body pt-2">
          {svc.install_root && (
            <div className="p-3 bg-black/20 rounded-card border border-white/5">
              <span className="text-title text-slate-400 font-semibold block">{t("projsub.installDir")}</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.install_root}>{svc.install_root}</p>
            </div>
          )}
          {svc.config_file && (
            <div className="p-3 bg-black/20 rounded-card border border-white/5">
              <span className="text-title text-slate-400 font-semibold block">{t("projsub.configFile")}</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.config_file}>{svc.config_file}</p>
            </div>
          )}
          {svc.data_dir && (
            <div className="p-3 bg-black/20 rounded-card border border-white/5">
              <span className="text-title text-slate-400 font-semibold block">{t("projsub.dataDir")}</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.data_dir}>{svc.data_dir}</p>
            </div>
          )}
          {svc.log_dir && (
            <div className="p-3 bg-black/20 rounded-card border border-white/5">
              <span className="text-title text-slate-400 font-semibold block">{t("projsub.logDir")}</span>
              <p className="font-mono text-slate-300 truncate mt-1" title={svc.log_dir}>{svc.log_dir}</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
