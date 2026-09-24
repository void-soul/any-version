import { useTranslation } from "react-i18next";

/** 思维导图节点表单的「颜色」块。
 *  画布内 DetailModal（ns="mindmap"）与速记悬浮窗（ns="mmdpop"）共用，
 *  差异仅翻译键命名空间与是否显示 hex 输入框。
 *  颜色精简为一行内联：自定义取色器 + 恢复默认，不再单独占一行。 */
export function NodeFormFields({
  ns, color, onColor, showHexInput = true,
}: {
  ns: "mindmap" | "mmdpop";
  color: string;
  onColor: (v: string) => void;
  /** DetailModal 显示 hex 输入框，速记悬浮窗隐藏（更简洁）。 */
  showHexInput?: boolean;
}) {
  const { t } = useTranslation();
  const isMm = ns === "mindmap";
  const focusCls = isMm ? "focus:border-cyan-400/60" : "focus:border-[var(--mm-accent)]";
  const colorKey = isMm ? "mindmap.colorLabel" : "mmdpop.color";
  const hex = (color && /^#[0-9a-fA-F]{6}$/.test(color) ? color : null) ?? "#22d3ee";

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-md border border-white/5 bg-white/[0.02] px-2.5 py-2">
      {/* 颜色：内联进同一设置行（自定义取色器 + 恢复默认） */}
      <div className="flex min-w-0 items-center gap-1.5">
        <span className="shrink-0 text-[9px] uppercase font-semibold text-slate-500">{t(colorKey)}</span>
        <label className="relative inline-flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center overflow-hidden rounded-full border border-white/30" title={t(`${ns}.customColor`)}>
          <input type="color" value={hex} className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
            onChange={(e) => onColor(e.target.value)} />
          <span className="h-2.5 w-2.5 rounded-full" style={{ background: "conic-gradient(#f87171,#fbbf24,#34d399,#22d3ee,#a78bfa,#f87171)" }} />
        </label>
        {showHexInput && (
          <input value={color || ""} onChange={(e) => onColor(e.target.value || "")} placeholder="#RRGGBB"
            className={`h-5 w-[70px] shrink-0 rounded border border-white/15 bg-slate-900 px-1.5 text-[9px] text-slate-300 outline-none ${focusCls}`} />
        )}
        <button type="button" className="shrink-0 rounded border border-white/15 px-1.5 py-0.5 text-[9px] text-slate-400 hover:text-white"
          onClick={() => onColor("")} title={t(`${ns}.autoColor`)}>{t(`${ns}.auto`)}</button>
      </div>
    </div>
  );
}
