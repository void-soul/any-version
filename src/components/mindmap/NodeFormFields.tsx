import { useTranslation } from "react-i18next";
import { PlanDateTimePicker } from "./MindmapPanel";
import { ColorGradientBar } from "./ColorGradientBar";

/** 思维导图节点表单的「进度 · 计划时间 · 重复 + 颜色」整块。
 *  画布内 DetailModal（ns="mindmap"）与速记悬浮窗（ns="mmdpop"）共用，
 *  差异仅翻译键命名空间与是否显示 hex 输入框。 */
export function NodeFormFields({
  ns, progress, planAt, repeat, color,
  onProgress, onPlanAt, onRepeat, onColor, showHexInput = true,
}: {
  ns: "mindmap" | "mmdpop";
  progress: number;
  planAt: string;
  repeat: string;
  color: string;
  onProgress: (v: number) => void;
  onPlanAt: (iso: string | null) => void;
  onRepeat: (v: string) => void;
  onColor: (v: string) => void;
  /** DetailModal 显示 hex 输入框，速记悬浮窗隐藏（更简洁）。 */
  showHexInput?: boolean;
}) {
  const { t } = useTranslation();
  const isMm = ns === "mindmap";
  const focusCls = isMm ? "focus:border-cyan-400/60" : "focus:border-[var(--mm-accent)]";
  const accentCls = isMm ? "accent-cyan-400" : "accent-[var(--mm-accent)]";
  // 两个命名空间的键名不同：mindmap 用 XxxLabel/planRepeat，mmdpop 用 短键。
  const planTimeKey = isMm ? "mindmap.planTimeLabel" : "mmdpop.planTime";
  const repeatKey = isMm ? "mindmap.planRepeat" : "mmdpop.repeat";
  const colorKey = isMm ? "mindmap.colorLabel" : "mmdpop.color";
  const hex = (color && /^#[0-9a-fA-F]{6}$/.test(color) ? color : null) ?? "#22d3ee";

  return (
    <>
      {/* 进度 · 计划时间 · 重复：标签内联，单行 */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-md border border-white/5 bg-white/[0.02] px-2.5 py-2">
        <label className="flex items-center gap-1.5 text-[9px] uppercase font-semibold text-slate-500">
          {isMm ? t("mindmap.progressLabel") : t("mmdpop.progress", { value: progress })}
          <input type="range" min={0} max={100} value={progress} onChange={(e) => onProgress(Number(e.target.value))}
            className={`h-6 w-20 cursor-pointer ${accentCls}`} />
          {isMm && <span className="text-[10px] text-slate-400">{progress}%</span>}
        </label>
        <label className="flex items-center gap-1.5 text-[9px] uppercase font-semibold text-slate-500">{t(planTimeKey)}
          <PlanDateTimePicker value={planAt} onChange={onPlanAt} />
        </label>
        <label className="flex items-center gap-1.5 text-[9px] uppercase font-semibold text-slate-500">{t(repeatKey)}
          <select value={repeat} onChange={(e) => onRepeat(e.target.value)}
            style={{ fontSize: 10 }}
            className={`h-7 min-w-[58px] cursor-pointer rounded-md border border-white/10 bg-slate-950/70 px-1 text-slate-200 outline-none ${focusCls}`}>
            <option value="none">{t(`${ns}.noRepeat`)}</option>
            <option value="daily">{t(`${ns}.daily`)}</option>
            <option value="weekly">{t(`${ns}.weekly`)}</option>
          </select>
        </label>
      </div>

      {/* 颜色：独立一行的渐变细条取色器（点击取色）+ 自定义取色器 + 恢复默认 */}
      <div className="rounded-md border border-white/5 bg-white/[0.02] px-2.5 py-2">
        <div className="mb-1 flex items-center justify-between">
          <span className="text-[9px] uppercase font-semibold text-slate-500">{t(colorKey)}</span>
          <span className="font-mono text-[9px] text-slate-400">{color || "auto"}</span>
        </div>
        <div className="flex items-center gap-2">
          <ColorGradientBar color={color} onChange={onColor} />
          <label className="relative inline-flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center overflow-hidden rounded border border-white/30" title={t(`${ns}.customColor`)}>
            <input type="color" value={hex} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
              onChange={(e) => onColor(e.target.value)} />
            <span className="h-3 w-3 rounded-sm" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
          </label>
          {showHexInput && (
            <input value={color || ""} onChange={(e) => onColor(e.target.value || "")} placeholder="#RRGGBB"
              className={`h-5 w-[70px] shrink-0 rounded border border-white/15 bg-slate-900 px-1.5 text-[9px] text-slate-300 outline-none ${focusCls}`} />
          )}
          <button type="button" className="shrink-0 rounded border border-white/15 px-1.5 py-0.5 text-[9px] text-slate-400 hover:text-white"
            onClick={() => onColor("")} title={t(`${ns}.autoColor`)}>{t(`${ns}.auto`)}</button>
        </div>
      </div>
    </>
  );
}
