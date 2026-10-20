// 设计器画布的「外观」浮层：三个共用滑块 + **本模块自己的**布局整理动作。
//
// 滑块那部分（高亮强度 / 淡化强度 / 高亮层级 + 恢复默认）已上移到
// `shared/GraphLookPrefs` —— 思维导图与 JSON 浏览的画布用的是同一块，
// 三处各养一份的话，「加第四个滑块」要改三处，漏一处只是某个画布少个开关，
// 不报错、很难发现。
//
// 这里只保留设计器特有的那一排：布局方式（放射 / 核心表左右分列）+ 自动布局 /
// 整理选中 / 自动配色三颗按钮，以 `extra` 挂进共用面板。
//
// 为什么是悬浮面板而不是工具栏下拉：这几个值看一眼就要立刻判断效果，
// 塞进菜单里就得「打开 → 拖 → 关 → 看」来回三步；浮在画布上拖完立刻可见。
// 为什么默认折叠成一个图标钮：调它是低频操作（大部分人用默认值就挺好），
// 但面板一旦展开就要一直挡着画布。
import { useTranslation } from "react-i18next";
import { LayoutGrid, Palette, Rows3 } from "lucide-react";
import GraphLookPrefs from "../shared/GraphLookPrefs";
import { LAYOUT_MODES, type LayoutMode } from "./layout";
import type { HighlightDepth } from "./series";

/** 一颗「把画面收拾一下」的动作钮（图标在上、文字在下，三颗并排不挤） */
function ActionButton({
  icon, label, title, onClick, disabled,
}: {
  icon: React.ReactNode;
  label: string;
  title: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className="flex cursor-pointer flex-col items-center gap-1 rounded-ctl bg-white/5 py-1.5 text-micro text-slate-300 transition hover:bg-white/10 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-40"
    >
      {icon}
      <span className="leading-tight">{label}</span>
    </button>
  );
}

export default function GraphLookPanel({
  highlight, dim, onHighlight, onDim, onReset,
  depth, onDepth,
  maxHighlight, maxDim,
  layoutMode, onLayoutMode,
  onAutoLayout, onArrange, onAutoColor,
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
  return (
    <GraphLookPrefs
      highlight={highlight}
      dim={dim}
      onHighlight={onHighlight}
      onDim={onDim}
      onReset={onReset}
      depth={depth}
      onDepth={onDepth}
      maxHighlight={maxHighlight}
      maxDim={maxDim}
      extra={
        <>
          {/* 布局方式：两种算法（放射 / 核心表左右分列）在这里切换。
              放在「布局整理」里而不是工具栏：它只对下面两个动作有意义。 */}
          <select
            value={layoutMode}
            onChange={(e) => onLayoutMode(e.target.value as LayoutMode)}
            title={t("dbd.layoutModeHint")}
            className="mb-1.5 w-full cursor-pointer rounded-ctl bg-white/5 px-1.5 py-1 text-micro text-slate-200 outline-none"
          >
            {LAYOUT_MODES.map((m) => (
              <option key={m.value} value={m.value}>
                {t(m.labelKey)}
              </option>
            ))}
          </select>
          <div className="grid grid-cols-3 gap-1">
            <ActionButton icon={<LayoutGrid className="h-3.5 w-3.5" />} label={t("dbd.autoLayout")} title={t("dbd.autoLayout")} onClick={onAutoLayout} />
            <ActionButton icon={<Rows3 className="h-3.5 w-3.5" />} label={t("dbd.lookArrangeSelected")} title={t("dbd.arrangeHint")} onClick={onArrange} disabled={arrangeDisabled} />
            <ActionButton icon={<Palette className="h-3.5 w-3.5" />} label={t("dbd.lookAutoColor")} title={t("dbd.autoColorHint")} onClick={onAutoColor} />
          </div>
        </>
      }
    />
  );
}
