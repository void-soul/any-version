/**
 * 画布高亮的共享样式层：数据库设计器 / 思维导图 / JSON 浏览三个画布共用一套语言。
 *
 * 为什么不各写一份：三处的需求完全一样（选中 = 最亮、关联 = 稍弱、其余压暗、
 * 关联边流光），但实现散在三个组件里时，任何一次「调亮一点」都要改三处，
 * 而且很容易漏掉某一处 —— 高亮是纯观感，漏了不会报错、只会显得某处没跟上。
 */

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

/**
 * 霓虹环（描边 + 外辉光）的 boxShadow。
 *
 * 档位靠**强度**区分（描边粗细 + 辉光层数 + 透明度），不靠色相 ——
 * 这样同一套代码在「节点有色」和「节点无色」两种情况下都成立。
 */
export function neonRing(color: string, level: HighlightLevel): string {
  if (level === "select") {
    return `0 0 0 2px ${color}, 0 0 6px color-mix(in srgb, ${color} 60%, transparent), 0 0 18px color-mix(in srgb, ${color} 42%, transparent), 0 0 40px color-mix(in srgb, ${color} 22%, transparent)`;
  }
  return `0 0 0 1px ${color}, 0 0 8px color-mix(in srgb, ${color} 48%, transparent), 0 0 26px color-mix(in srgb, ${color} 26%, transparent)`;
}

/** 文字霓虹辉光（选中更亮：两段叠加） */
export function neonTextGlow(color: string, level: HighlightLevel): string {
  if (level === "select") {
    return `0 0 6px color-mix(in srgb, ${color} 90%, transparent), 0 0 16px color-mix(in srgb, ${color} 55%, transparent)`;
  }
  return `0 0 8px color-mix(in srgb, ${color} 60%, transparent)`;
}

/** 表头/卡片头部的底色浓度（选中更浓，是「点亮」最显眼的一块） */
export function headerTint(color: string, level: HighlightLevel): string {
  return level === "select"
    ? `color-mix(in srgb, ${color} 26%, transparent)`
    : `color-mix(in srgb, ${color} 14%, transparent)`;
}

/** 关联边的同色辉光（与节点环同色，一套语言） */
export function edgeGlow(color: string): string {
  return `drop-shadow(0 0 3px ${color})`;
}

/** 自定义 edge 的内层实线用的 className（配合 App.css 的 graph-edge-flow 动画） */
export const EDGE_FLOW_CLASS = "graph-edge-flow";
/** 外层光晕 path：显式关掉 dasharray，否则会被 React Flow 的 `animated path` 规则打成虚线 */
export const EDGE_GLOW_CLASS = "graph-edge-glow";
