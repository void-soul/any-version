// 画布「外观」悬浮调面板：高亮强度 + 淡化强度两个滑块，边拖边看效果。
//
// 为什么是悬浮面板而不是工具栏下拉：这两个值看一眼就要立刻判断效果，
// 塞进菜单里就得「打开 → 拖 → 关 → 看」，来回三步；而浮在画布上拖完立刻可见。
//
// 为什么默认折叠成一个图标钮：调它是低频操作（大部分人用默认值就挺好），
// 但面板一旦展开就要一直挡着画布 —— 所以折叠态只留一个 Sliders 图标，
// 位置在画布右下角，避开左下的 Controls、左侧表导航、右侧 AI 栏。
//
// 偏好本体在 `utils/graphHighlight.ts`（localStorage 持久化，与边类型同一套做法），
// 这里只负责交互；将来思维导图 / JSON 图要接上，复制这个面板即可（纯逻辑不用动）。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LayoutGrid, Palette, RotateCcw, Rows3, SlidersHorizontal } from "lucide-react";
import { LAYOUT_MODES, type LayoutMode } from "./layout";
import { HIGHLIGHT_DEPTHS, type HighlightDepth } from "./series";

/** 层级值合法（来自 range 控件，但仍过一次校验：保存本地偏好里的旧值不应该直接当层级用） */
function toDepth(v: number): HighlightDepth {
  return (HIGHLIGHT_DEPTHS as readonly number[]).includes(v) ? (v as HighlightDepth) : 1;
}

/** 滑块 → 百分比显示 */
function pct(v: number): string {
  return `${Math.round(v * 100)}%`;
}

function Row({
  label,
  value,
  min,
  max,
  step,
  onChange,
  display,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  /** 右上角的显示文字；不传则按百分比显示。
   *  层级这类整数指标不能当百分比（“3跳”变成“300%”会让人误解为强度）。 */
  display?: string;
}) {
  return (
    <label className="block">
      <span className="flex items-baseline justify-between gap-2 text-micro text-slate-400">
        <span>{label}</span>
        <span className="font-mono text-slate-300">{display ?? pct(value)}</span>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        // onChange（而不是 onInput/onPointerUp）：range 的 onChange 在拖动中逐帧触发，
        // 这正是「边调整边看」要的。
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-1 w-full accent-[var(--module-accent)]"
      />
    </label>
  );
}

export default function GraphLookPanel({
  highlight,
  dim,
  onHighlight,
  onDim,
  onReset,
  depth,
  onDepth,
  maxHighlight,
  maxDim,
  layoutMode,
  onLayoutMode,
  onAutoLayout,
  onArrange,
  onAutoColor,
  arrangeDisabled,
}: {
  highlight: number;
  dim: number;
  onHighlight: (v: number) => void;
  onDim: (v: number) => void;
  /** 高亮层级 1-5：几跳内的表不被淡化。整数指标，不用百分比显示。 */
  depth: HighlightDepth;
  onDepth: (d: HighlightDepth) => void;
  onReset: () => void;
  /** 上限由偏好层给（高亮可到 150%，淡化可调至完全看不见） */
  maxHighlight: number;
  maxDim: number;
  /** 布局方式：放射 / 核心表左右分列（决定自动布局与整理选中用哪套算法） */
  layoutMode: LayoutMode;
  onLayoutMode: (m: LayoutMode) => void;
  onAutoLayout: () => void;
  /** 「整理选中的表」：未选中或关联不足 2 张时禁用 */
  onArrange: () => void;
  onAutoColor: () => void;
  arrangeDisabled: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        title={t("dbd.lookTitle")}
        className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-ctl border border-white/10 bg-surface-panel/90 text-slate-400 shadow backdrop-blur transition hover:text-slate-100"
      >
        <SlidersHorizontal className="h-3.5 w-3.5" />
      </button>
    );
  }

  return (
    <div className="w-[220px] rounded-ctl border border-white/10 bg-surface-panel/95 p-2.5 shadow-xl backdrop-blur">
      <div className="mb-2 flex items-center justify-between gap-2">
        <span className="text-tiny font-semibold text-slate-200">{t("dbd.lookTitle")}</span>
        {/* 恢复默认：滑块很容易调乱，没有一键回到「改动前的样子」就只能去删 localStorage */}
        <button
          type="button"
          onClick={onReset}
          title={t("dbd.lookReset")}
          className="flex cursor-pointer items-center gap-1 text-micro text-slate-500 transition hover:text-slate-200"
        >
          <RotateCcw className="h-3 w-3" />
          {t("dbd.lookReset")}
        </button>
      </div>
      <div className="space-y-2.5">
        <Row
          label={t("dbd.lookHighlight")}
          value={highlight}
          min={0}
          max={maxHighlight}
          step={0.05}
          onChange={onHighlight}
        />
        <Row label={t("dbd.lookDim")} value={dim} min={0} max={maxDim} step={0.05} onChange={onDim} />
      {/* 层级：放在强度之后、分隔线之前。一谱业务设计里表外键全连通，
          层级抬高会把几乎整份图都点亮（适合看局部），低层级则只点直接邻居（适合干脆的事）。
          它同时决定「整理选中」整理多少张表。 */}
      <Row
        label={t("dbd.lookDepth")}
        value={depth}
        min={HIGHLIGHT_DEPTHS[0]}
        max={HIGHLIGHT_DEPTHS[HIGHLIGHT_DEPTHS.length - 1]}
        step={1}
        display={`${depth} 跳`}
        onChange={(v) => onDepth(toDepth(v))}
      />
      </div>
      {/* 布局整理：三个「把画面收拾一下」的动作收在这里，而不是工具栏再开一个下拉。
          它们和两个滑块是同一类需求（调整画面），放一起少一个入口、也少一层点击。 */}
      <div className="mt-2.5 border-t border-white/10 pt-2">
        <div className="mb-1 text-micro uppercase tracking-wide text-slate-600">
          {t("dbd.lookArrangeSection")}
        </div>
        {/* 布局方式：两种算法（放射 / 核心表左右分列）在这里切换。
            放在「布局整理」里而不是工具栏：它只对下面两个动作有意义。 */}
        <select
          value={layoutMode}
          onChange={(e) => onLayoutMode(e.target.value as LayoutMode)}
          title={t("dbd.layoutModeHint")}
          className={"mb-1.5 w-full cursor-pointer rounded-ctl bg-white/5 px-1.5 py-1 text-micro text-slate-200 outline-none"}
        >
          {LAYOUT_MODES.map((m) => (
            <option key={m.value} value={m.value}>
              {t(m.labelKey)}
            </option>
          ))}
        </select>
        <div className="grid grid-cols-3 gap-1">
          <button
            type="button"
            onClick={onAutoLayout}
            title={t("dbd.autoLayout")}
            className="flex cursor-pointer flex-col items-center gap-1 rounded-ctl bg-white/5 py-1.5 text-micro text-slate-300 transition hover:bg-white/10 hover:text-slate-100"
          >
            <LayoutGrid className="h-3.5 w-3.5" />
            <span className="leading-tight">{t("dbd.autoLayout")}</span>
          </button>
          <button
            type="button"
            onClick={onArrange}
            disabled={arrangeDisabled}
            title={t("dbd.arrangeHint")}
            className="flex cursor-pointer flex-col items-center gap-1 rounded-ctl bg-white/5 py-1.5 text-micro text-slate-300 transition hover:bg-white/10 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Rows3 className="h-3.5 w-3.5" />
            <span className="leading-tight">{t("dbd.lookArrangeSelected")}</span>
          </button>
          <button
            type="button"
            onClick={onAutoColor}
            title={t("dbd.autoColorHint")}
            className="flex cursor-pointer flex-col items-center gap-1 rounded-ctl bg-white/5 py-1.5 text-micro text-slate-300 transition hover:bg-white/10 hover:text-slate-100"
          >
            <Palette className="h-3.5 w-3.5" />
            <span className="leading-tight">{t("dbd.lookAutoColor")}</span>
          </button>
        </div>
      </div>
      {/* 用「收起」而不是「点外面就关」：调参时手会移出面板，点外面即关等于没法连续调两个滑块 */}
      <button
        type="button"
        onClick={() => setOpen(false)}
        className="mt-2 w-full cursor-pointer rounded-ctl bg-white/5 py-1 text-micro text-slate-400 transition hover:bg-white/10 hover:text-slate-200"
      >
        {t("dbd.lookCollapse")}
      </button>
    </div>
  );
}