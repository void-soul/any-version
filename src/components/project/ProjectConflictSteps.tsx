import { useTranslation } from "react-i18next";

import { CheckCircle } from "lucide-react";

export function ConflictMethodStep({ method: workflowMethod, onMethodChange: setWorkflowMethod, onNext: workflowNext, onClose: closeWorkflow }: { method: "junction" | "point"; onMethodChange: (method: "junction" | "point") => void; onNext: () => void; onClose: () => void }) {
 const { t }=useTranslation();
 const accentBg="bg-amber-500/10", accentBorder="border-amber-500/20", accentText="text-amber-400", btnBg="bg-amber-600 hover:bg-amber-500";
 const totalSteps=4, stepLabels={method:t("projsub.stepLabelsMethod")};

      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("projsub.wfTitle", { step: 1, total: totalSteps, label: stepLabels.method })}
            </span>
            <button onClick={closeWorkflow} className="text-caption text-slate-500 hover:text-slate-300 cursor-pointer">{t("projsub.cancel")}</button>
          </div>
          <div className="space-y-1.5">
            <p className="text-body text-slate-300">{t("projsub.pickMethod")}</p>
            <label className={`flex items-start gap-2 p-2.5 rounded-ctl cursor-pointer transition-all border ${workflowMethod === "junction"
              ? `${accentBorder} bg-white/5`
              : "border-white/5 hover:bg-white/[0.02]"
              }`}>
              <input type="radio" name="wf_method" value="junction" checked={workflowMethod === "junction"}
                onChange={() => setWorkflowMethod("junction")} className="mt-0.5" />
              <div>
                <span className="text-body font-semibold text-slate-200">{t("projsub.optAJunction")}</span>
                <p className="text-title text-slate-500 mt-0.5">
                  {t("projsub.optADesc")}
                </p>
              </div>
            </label>
            <label className={`flex items-start gap-2 p-2.5 rounded-ctl cursor-pointer transition-all border ${workflowMethod === "point"
              ? `${accentBorder} bg-white/5`
              : "border-white/5 hover:bg-white/[0.02]"
              }`}>
              <input type="radio" name="wf_method" value="point" checked={workflowMethod === "point"}
                onChange={() => setWorkflowMethod("point")} className="mt-0.5" />
              <div>
                <span className="text-body font-semibold text-[var(--module-accent)]">{t("projsub.optBPoint")}</span>
                <p className="text-title text-slate-500 mt-0.5">
                  {t("projsub.optBDesc")}
                </p>
              </div>
            </label>
          </div>
          <div className="flex justify-end">
            <button onClick={workflowNext}
              className={`px-3 py-1 ${btnBg} text-white rounded text-caption font-semibold cursor-pointer transition-colors`}>
              {t("projsub.nextStep")}
            </button>
          </div>
        </div>
      );

}

export function ConflictProgressStep({ progress: workflowProgress }: { progress: { stage: string; current: number; total: number; file_name: string } | null }) {
 const { t }=useTranslation();
 const accentBg="bg-amber-500/10", accentBorder="border-amber-500/20", accentText="text-amber-400", progressBarColor="bg-amber-500/60";
 const totalSteps=4, stepLabels={executing:t("projsub.stepLabelsExec")};

      const progressPercent = workflowProgress && workflowProgress.total > 0
        ? Math.round((workflowProgress.current / workflowProgress.total) * 100)
        : 0;

      return (
        <div className={`mt-3 p-3 rounded-card border ${accentBorder} ${accentBg} space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className={`text-body font-semibold ${accentText}`}>
              {t("projsub.wfTitle", { step: 4, total: totalSteps, label: stepLabels.executing })}
            </span>
          </div>

          <div className="p-3 bg-black/20 rounded-ctl border border-white/5 space-y-3">
            <div className="flex items-center justify-between text-body text-slate-300">
              <span>{workflowProgress?.stage || t("projsub.executingStage")}</span>
              <span className="font-mono">{progressPercent}%</span>
            </div>

            {/* 进度条 */}
            <div className="w-full bg-white/5 rounded-full h-2 overflow-hidden border border-white/5">
              <div className={`${progressBarColor} h-2 rounded-full transition-all duration-300`} style={{ width: `${progressPercent}%` }}></div>
            </div>

            {workflowProgress && (
              <div className="text-caption text-slate-500 font-mono space-y-0.5">
                <div className="truncate">{t("projsub.wfFile", { name: workflowProgress.file_name || "-" })}</div>
                <div>{t("projsub.wfProgress", { cur: workflowProgress.current, total: workflowProgress.total })}</div>
              </div>
            )}
          </div>
        </div>
      );

}

export function ConflictDoneStep({ onClose: closeWorkflow }: { onClose: () => void }) {
 const { t }=useTranslation();

 const stepLabels={done:t("projsub.stepLabelsDone")};

      return (
        <div className={`mt-3 p-3 rounded-card border border-emerald-500/20 bg-emerald-500/10 space-y-3 animate-fadeIn`}>
          <div className="flex items-center justify-between">
            <span className="text-body font-semibold text-emerald-400">
              {t("projsub.wfDoneTitle", { label: stepLabels.done })}
            </span>
          </div>

          <div className="p-3 bg-black/20 rounded-ctl border border-white/5 space-y-1">
            <p className="text-body text-emerald-300 font-semibold flex items-center gap-1">
              <CheckCircle className="w-3.5 h-3.5" />
              {t("projsub.wfDoneMsg")}
            </p>
            <p className="text-caption text-slate-500 mt-1">{t("projsub.wfDoneDesc")}</p>
          </div>

          <div className="flex justify-end">
            <button onClick={closeWorkflow}
              className="px-4 py-1.5 bg-emerald-600 hover:bg-emerald-500 text-white rounded text-caption font-semibold cursor-pointer transition-colors">
              {t("projsub.closeWizard")}
            </button>
          </div>
        </div>
      );

}
