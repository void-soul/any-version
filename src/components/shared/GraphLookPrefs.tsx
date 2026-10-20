// 画布「外观」悬浮调参面板：**三个画布共用这一块**。
//
// 为什么抽出来：数据库设计器先有它（`dbdesigner/GraphLookPanel.tsx`），
// 而思维导图 / JSON 浏览的画布要的是同一件事 —— 两个强度滑块 + 一个层级滑块，
// 边拖边看效果。复制一份的话，将来「加第四个滑块」要改三处，
// 而漏掉某一处只是「某个画布少个开关」，不报错、很难发现。
//
// 与 `GraphLookPanel` 的分工：**这里只有三个滑块 + 恢复默认**；
// 「布局整理」那一排动作由 `extra` 传进来 —— 设计器有自动布局/整理选中/自动配色三颗，
// 导图与 JSON 只有自动布局/整理选中两颗，硬编码进来就得给某处传空按钮。
//
// 为什么不收进工具栏下拉：这几个值看一眼就要立刻判断效果，
// 塞进菜单里就得「打开 → 拖 → 关 → 看」来回三步；浮在画布上拖完立刻可见。
//
// 为什么默认折叠成一个图标钮：调它是低频操作（大部分人用默认值就挺好），
// 但面板一旦展开就要一直挡着画布。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { RotateCcw, SlidersHorizontal } from "lucide-react";
import { GRAPH_DEPTHS, type GraphDepth } from "../../utils/graphScope";

/** 滑块 → 百分比显示 */
function pct(v: number): string {
  return `${Math.round(v * 100)}%`;
}

function Row({
  label, value, min, max, step, onChange, display,
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

/** 层级值合法（来自 range 控件，但仍过一次校验：本地偏好里的旧值不该直接当层级用） */
function toDepth(v: number): GraphDepth {
  return (GRAPH_DEPTHS as readonly number[]).includes(v) ? (v as GraphDepth) : 1;
}

export default function GraphLookPrefs({
  highlight, dim, onHighlight, onDim, onReset,
  depth, onDepth,
  maxHighlight, maxDim,
  extra,
}: {
  /** 高亮强度倍率：1 = 改动前的观感，0 = 完全不点亮 */
  highlight: number;
  /** 淡化强度：1 = 改动前的观感，0 = 不淡化，2 = 完全看不见 */
  dim: number;
  onHighlight: (v: number) => void;
  onDim: (v: number) => void;
  onReset: () => void;
  /** 高亮层级 1-5：几跳以内的节点算关联。整数指标，不用百分比显示。 */
  depth: GraphDepth;
  onDepth: (d: GraphDepth) => void;
  /** 上限由偏好层给（高亮可到 150%，淡化可调至完全看不见） */
  maxHighlight: number;
  maxDim: number;
  /** 「把画面收拾一下」的那排动作（各画布不一样，见文件头注释） */
  extra?: React.ReactNode;
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
        <Row label={t("dbd.lookHighlight")} value={highlight} min={0} max={maxHighlight} step={0.05} onChange={onHighlight} />
        <Row label={t("dbd.lookDim")} value={dim} min={0} max={maxDim} step={0.05} onChange={onDim} />
        {/* 层级：放在强度之后、分隔线之前。一谱业务设计里表外键全连通，
            层级抬高会把几乎整份图都点亮（适合看局部），低层级则只点直接邻居。
            它同时决定「整理选中」整理多少个节点。 */}
        <Row
          label={t("dbd.lookDepth")}
          value={depth}
          min={GRAPH_DEPTHS[0]}
          max={GRAPH_DEPTHS[GRAPH_DEPTHS.length - 1]}
          step={1}
          display={t("dbd.lookDepthUnit", { count: depth })}
          onChange={(v) => onDepth(toDepth(v))}
        />
      </div>
      {/* 布局整理：各画布自己的动作（设计器三颗、导图/JSON 两颗）由 extra 传进来。
          它们和上面的滑块是同一类需求（调整画面），放一起少一个入口、也少一层点击。 */}
      {extra ? (
        <div className="mt-2.5 border-t border-white/10 pt-2">
          <div className="mb-1 text-micro uppercase tracking-wide text-slate-600">
            {t("dbd.lookArrangeSection")}
          </div>
          {extra}
        </div>
      ) : null}
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
