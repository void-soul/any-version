/**
 * 画布高亮的共享样式层：数据库设计器 / 思维导图 / JSON 浏览三个画布共用一套语言。
 *
 * 为什么不各写一份：三处的需求完全一样（选中 = 最亮、关联 = 稍弱、其余压暗、
 * 关联边流光），但实现散在三个组件里时，任何一次「调亮一点」都要改三处，
 * 而且很容易漏掉某一处 —— 高亮是纯观感，漏了不会报错、只会显得某处没跟上。
 */
import { useCallback, useState } from "react";

/** 高亮档位：`select` 选中本身，`related` 与它直接相连 */
export type HighlightLevel = "select" | "related";

/** 选中档的兜底色 = 模块强调色（全站紫红主色） */
export const HL_FALLBACK_SELECT = "var(--module-accent, var(--vex-fallback-primary))";
/** 关联档的兜底色 = 青色（duo-tone 的点缀色）；节点自己没有颜色时用它 */
export const HL_FALLBACK_RELATED = "var(--cyan, var(--vex-cyan, var(--vex-fallback-cyan)))";
/** 无关的连线：压成灰蓝，几乎看不见 */
export const EDGE_DIM_STROKE = "rgba(148,163,184,0.18)";
/** 无关节点的透明度（与三个画布的 className 对齐） */
export const GRAPH_DIMMED_CLASS = "opacity-35";

// ── 可调的两档强度（2026-10 起）────────────────────────────────────────
//
// 「高亮多强」「无关项淡到什么程度」以前是写死的常量，想调只能改代码。
// 现在做成**持久化偏好**：设计器画布上有悬浮滑块，边拖边看效果，
// 换台电脑也保留（localStorage，与边类型 `useEdgeStyle` 同一套做法）。
//
// **默认值刻意等于改动前的常量**（高亮 1 / 淡化 1），所以升级后
// 任何人的画面都一模一样 —— 偏好只对「主动调过的人」生效。

/** 高亮强度上限：150% —— 再高就成一片糊光，反而看不清结构 */
export const HL_BOOST_MAX = 1.5;
/**
 * 淡化强度上限：**200%**，不是 100%。
 *
 * 为什么不是 100%：100% 对应的是「改动前的观感」（节点 0.35 不透明度），
 * 而用户要的「再淡一点」是**调到看不见** —— 0.65 的不透明度只能降到 0.35 就到头了。
 * 透明度 0 出现在 dim ≈ 154%，所以上限必须留出这段，否则滑块到顶了却还是能看见。
 */
export const DIM_MAX = 2;

const HL_KEY = "kira.graph.dbd.highlight";
const DIM_KEY = "kira.graph.dbd.dim";

export interface GraphHighlightPrefs {
  /** 高亮强度倍率：1 = 改动前的观感 */
  highlight: number;
  /** 淡化强度：1 = 改动前的观感，0 = 不淡化 */
  dim: number;
}

export const DEFAULT_HL_PREFS: GraphHighlightPrefs = { highlight: 1, dim: 1 };

/** 把任意输入夹到 0..max；非有限值（NaN / undefined）回落到默认 1。 */
export function clampPref(v: number, max: number): number {
  if (!Number.isFinite(v)) return 1;
  return Math.min(max, Math.max(0, v));
}

/** 解析 localStorage 里的字符串；非法值回落默认（不报错，用户改坏了也不该白屏）。 */
export function readPref(raw: string | null, max: number): number {
  if (raw === null || raw.trim() === "") return 1;
  return clampPref(Number(raw), max);
}

/**
 * 无关节点的透明度：dim=1 → 0.35（与旧的 `opacity-35` 一致），dim=0 → 1（不淡化），
 * dim ≳ 1.54 → 0（**完全看不见**，这是上限留到 200% 的原因）。
 */
export function dimmedOpacity(dim = 1): number {
  const d = clampPref(dim, DIM_MAX);
  return round2(Math.max(0, 1 - 0.65 * d));
}

/**
 * 无关连线的颜色：**跟着节点的透明度一起淡**。
 *
 * 之前是另起一条公式（alpha = 0.15 + 0.25·dim），结果滑块往上推时连线反而更显眼 ——
 * 而用户的直觉是「整块一起淡下去」。现在按 opacity 等比缩放：dim=1 → alpha 0.4（改动前），
 * 淡到看不见时连线也归零。
 */
export function dimmedEdgeStroke(dim = 1): string {
  const op = dimmedOpacity(dim);
  // dim=1 时 opacity=0.35 → alpha 0.4；opacity=0 时 alpha 也必须是 0
  const alpha = Math.min(1, 0.4 * (op / 0.35));
  return `rgba(148,163,184,${round2(alpha)})`;
}

/**
 * 无关连线的线宽：dim=1 → **1.3**（改动前的值，这条不变量不能破）。
 *
 * 分两段是有意的：dim ≤ 1 时在 0.9-1.3 之间插值（越不淡化越细，视觉上更「让路」）；
 * dim > 1 时按透明度等比收细，收到 0.6 就停 —— 再细对一条已经看不见的线没有意义，
 * 而且太细的线在高分屏上会断成虚点。
 */
export function dimmedEdgeWidth(dim = 1): number {
  const d = clampPref(dim, DIM_MAX);
  if (d <= 1) return round2(0.9 + 0.4 * d);
  return round2(Math.max(0.6, 1.3 * (dimmedOpacity(d) / 0.35)));
}

/**
 * 高亮强度是否还能产生**可见**效果（0 = 完全不点亮）。
 *
 * 为什么需要这一个开关：节点与连线消费同一档强度，但两边的公式并不同族 ——
 * 节点侧是纯乘法（浓度 = 基准 × 强度），强度 0 时霓虹环 / 文字辉光 / 表头底色全部归零；
 * 连线侧原本是带下限的仿射（线宽 = 1.4 + 1.2 × 强度），强度 0 时仍有 1.4px 实心线
 * + 流光 + 鲜亮箭头。两边一高一低摆在一起，画出来就是**只亮线、表不亮**（用户实测）。
 *
 * 所以两端必须共用这一个判据：强度为 0 就是「两端都不点亮」，
 * 由节点侧让位回常态外观、连线侧把线宽与箭头一起收掉。
 */
export function hasVisibleHighlight(boost = 1): boolean {
  return clampPref(boost, HL_BOOST_MAX) > 0;
}

/**
 * 选中/关联连线的线宽：hl=1 → 2.6（改动前的值）。
 *
 * **以 1 为基准做纯乘法**（而不是 `1.4 + 1.2·hl` 那种带下限的仿射）：
 * 强度 0 时连线必须和节点一起归零，否则就成了「只亮线、表不亮」的元凶。
 * 1.4 那个下限当初是为了避免弱化档隐形（见 dimmedEdgeWidth 的注释），
 * 高亮档不需要 —— 它已经有流光、辉光、置顶三重提示。
 */
export function relatedEdgeWidth(highlight = 1): number {
  return round2(2.6 * clampPref(highlight, HL_BOOST_MAX));
}

function round2(v: number): number {
  return Math.round(v * 100) / 100;
}

/** 读偏好 + 改即持久化。与 `useEdgeStyle` 同一形态。 */
export function useGraphHighlightPrefs() {
  const [prefs, setPrefs] = useState<GraphHighlightPrefs>(() => ({
    highlight: readPref(localStorage.getItem(HL_KEY), HL_BOOST_MAX),
    dim: readPref(localStorage.getItem(DIM_KEY), DIM_MAX),
  }));
  const setHighlight = useCallback((v: number) => {
    const next = clampPref(v, HL_BOOST_MAX);
    setPrefs((p) => ({ ...p, highlight: next }));
    localStorage.setItem(HL_KEY, String(next));
  }, []);
  const setDim = useCallback((v: number) => {
    const next = clampPref(v, DIM_MAX);
    setPrefs((p) => ({ ...p, dim: next }));
    localStorage.setItem(DIM_KEY, String(next));
  }, []);
  const reset = useCallback(() => {
    setPrefs(DEFAULT_HL_PREFS);
    localStorage.setItem(HL_KEY, String(DEFAULT_HL_PREFS.highlight));
    localStorage.setItem(DIM_KEY, String(DEFAULT_HL_PREFS.dim));
  }, []);
  return { prefs, setHighlight, setDim, reset };
}

/**
 * 高亮色：**优先用节点自己的颜色**，没有才回退到状态色。
 *
 * 为什么不让状态色一票否决：固定主色/青会出现「红色的表被套一圈青霓虹」——
 * 两个色相对着干，看着就是错的。跟随节点色后，节点环、卡片、它的连线是同一个色。
 *
 * 选中档把表色往白里提 12%：暗色表（如深蓝、深紫）不提亮的话，描边在深色画布上
 * 看不出来，高亮就白做了。提升的是**亮度**而不是换成别的色相，色相冲突不会被引入。
 */
export function highlightColor(ownColor: string | null | undefined, level: HighlightLevel): string {
  const c = (ownColor ?? "").trim();
  if (level === "select") {
    return c ? `color-mix(in srgb, ${c} 88%, white 12%)` : HL_FALLBACK_SELECT;
  }
  return c || HL_FALLBACK_RELATED;
}

/** 把 0..max 的倍率夹好再算 alpha，避免 boost 调大后出现 `alpha > 100%` 这种废 CSS。 */
function mixPct(pct: number, boost: number): number {
  return round2(Math.min(100, pct * clampPref(boost, HL_BOOST_MAX)));
}

/**
 * 霓虹环（描边 + 外辉光）的 boxShadow。
 *
 * 档位靠**强度**区分（描边粗细 + 辉光层数 + 透明度），不靠色相 ——
 * 这样同一套代码在「节点有色」和「节点无色」两种情况下都成立。
 *
 * `boost` 是「高亮强度」偏好：描边宽度与各层辉光透明度都按它缩放，
 * 1 = 改动前的样子。
 *
 * 强度 0 返回**空串**（而非一串全透明的废 CSS）：`0 0 0 0px <color>` 是零半径
 * 零扩散的影子，画出来什么都没有，可它**非空** —— 调用方 `ring ? 高亮 : 默认外观`
 * 的三元判定为真，卡片原有的 border/shadow 就被顶掉了，比不高亮时还难看。
 * 返回空串才是诚实的「当前不高亮」。
 */
export function neonRing(color: string, level: HighlightLevel, boost = 1): string {
  const b = clampPref(boost, HL_BOOST_MAX);
  if (!hasVisibleHighlight(b)) return "";
  if (level === "select") {
    return `0 0 0 ${round2(2 * b)}px ${color}, 0 0 6px color-mix(in srgb, ${color} ${mixPct(60, b)}%, transparent), 0 0 18px color-mix(in srgb, ${color} ${mixPct(42, b)}%, transparent), 0 0 40px color-mix(in srgb, ${color} ${mixPct(22, b)}%, transparent)`;
  }
  return `0 0 0 ${round2(1 * b)}px ${color}, 0 0 8px color-mix(in srgb, ${color} ${mixPct(48, b)}%, transparent), 0 0 26px color-mix(in srgb, ${color} ${mixPct(26, b)}%, transparent)`;
}

/** 文字霓虹辉光（选中更亮：两段叠加）。强度 0 返回空串，理由同 {@link neonRing}。 */
export function neonTextGlow(color: string, level: HighlightLevel, boost = 1): string {
  const b = clampPref(boost, HL_BOOST_MAX);
  if (!hasVisibleHighlight(b)) return "";
  if (level === "select") {
    return `0 0 6px color-mix(in srgb, ${color} ${mixPct(90, b)}%, transparent), 0 0 16px color-mix(in srgb, ${color} ${mixPct(55, b)}%, transparent)`;
  }
  return `0 0 8px color-mix(in srgb, ${color} ${mixPct(60, b)}%, transparent)`;
}

/**
 * 表头/卡片头部的底色浓度（选中更浓，是「点亮」最显眼的一块）。
 *
 * **强度 0 必须返回空串**，且这一点比看上去重要：调用方是
 * `hlHeader || 未高亮时的常态底色`，`||` 只认「非空字符串」。
 * 若这里返回 `color-mix(in srgb, C 0%, transparent)`（全透明），它照样短路 ——
 * 表头底色被一条全透明色盖掉，**高亮强度往左拖，表反而比常态更暗**，
 * 滑块方向整个反了。返回空串才能让常态底色回来。
 */
export function headerTint(color: string, level: HighlightLevel, boost = 1): string {
  const b = clampPref(boost, HL_BOOST_MAX);
  if (!hasVisibleHighlight(b)) return "";
  return level === "select"
    ? `color-mix(in srgb, ${color} ${mixPct(26, b)}%, transparent)`
    : `color-mix(in srgb, ${color} ${mixPct(14, b)}%, transparent)`;
}

/**
 * 关联边的同色辉光（与节点环同色，一套语言）。
 *
 * 强度 0 返回 `none`：CSS 里 `drop-shadow(0 0 0px C)` 虽然合法，但会把整条
 * `filter` 变成一个恒等变换，React Flow 的边缘抗锯齿与 `animated` 的 dash
 * 动画都要过这一层，白付渲染成本还不换来任何像素。
 */
export function edgeGlow(color: string, boost = 1): string {
  const b = clampPref(boost, HL_BOOST_MAX);
  if (!hasVisibleHighlight(b)) return "none";
  return `drop-shadow(0 0 ${round2(3 * b)}px ${color})`;
}

/** 自定义 edge 的内层实线用的 className（配合 App.css 的 graph-edge-flow 动画） */
export const EDGE_FLOW_CLASS = "graph-edge-flow";
/** 外层光晕 path：显式关掉 dasharray，否则会被 React Flow 的 `animated path` 规则打成虚线 */
export const EDGE_GLOW_CLASS = "graph-edge-glow";
