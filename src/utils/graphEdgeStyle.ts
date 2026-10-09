import { useCallback, useState } from "react";
import {
  ConnectionLineType,
  getBezierPath,
  getSmoothStepPath,
  getStraightPath,
  type EdgeProps,
} from "@xyflow/react";

/**
 * 画布连线样式：ER 设计 / 思维导图 / JSON 图三处共用一套。
 *
 * 三档对应 React Flow 内置的三种边：
 *   bezier     曲线    —— 关系少时最顺眼（默认）
 *   smoothstep 直角折线 —— 表多时不互相穿插，接近 PowerDesigner 的观感
 *   straight   直线    —— 节点排整齐时最清爽
 *
 * ⚠ 两种接线方式，别混：
 *   - 用**内置** edge 的画布（ER 设计）→ 直接把值当 `type`：`{ type: style }`
 *   - 用**自定义** edge 的画布（思维导图 / JSON 图）→ 走 `edgePath()` 自己算路径。
 *     自定义 edge 拿不到组件 state，样式要由 `data.style` 传进去（见各画布的 edges useMemo）。
 */

/** 一份偏好三处共享：切视图不用重新挑，换台电脑也保留（localStorage，不进业务数据）。 */
const EDGE_STYLE_KEY = "kira.graph.edgeStyle";

export const EDGE_STYLES = ["bezier", "smoothstep", "straight"] as const;
export type EdgeStyle = (typeof EDGE_STYLES)[number];

function isEdgeStyle(value: string | null): value is EdgeStyle {
  return value !== null && (EDGE_STYLES as readonly string[]).includes(value);
}

function loadEdgeStyle(): EdgeStyle {
  const saved = localStorage.getItem(EDGE_STYLE_KEY);
  return isEdgeStyle(saved) ? saved : "bezier";
}

/** 读当前样式 + 切换即持久化。切换只重建 edges，不动节点位置。 */
export function useEdgeStyle(): [EdgeStyle, (next: EdgeStyle) => void] {
  const [style, setStyle] = useState<EdgeStyle>(loadEdgeStyle);
  const change = useCallback((next: EdgeStyle) => {
    setStyle(next);
    localStorage.setItem(EDGE_STYLE_KEY, next);
  }, []);
  return [style, change];
}

/**
 * 拖线过程中那根实时预览线用 `connectionLineType`，它的类型是 **enum**
 * （`ConnectionLineType.Bezier = "default"`），既不认 `"bezier"` 这个名字，
 * 也不是字符串联合 —— 写字符串会直接类型报错。
 */
export const CONNECTION_LINE_BY_STYLE: Record<EdgeStyle, ConnectionLineType> = {
  bezier: ConnectionLineType.Bezier,
  smoothstep: ConnectionLineType.SmoothStep,
  straight: ConnectionLineType.Straight,
};

/** 自定义 edge 算路径要用的那组坐标（正好是 EdgeProps 里现成的字段）。 */
export type EdgePathParams = Pick<
  EdgeProps,
  "sourceX" | "sourceY" | "targetX" | "targetY" | "sourcePosition" | "targetPosition"
>;

/**
 * 按样式算出一条边的路径。三个 React Flow 函数都返回 5 元组
 * `[path, labelX, labelY, offsetX, offsetY]`，所以标签位置可以直接取用。
 *
 * 折线走 `borderRadius: 4`：小圆角比硬 90° 好看，又不像曲线那样大面积糊在一起。
 */
export function edgePath(
  style: EdgeStyle,
  params: EdgePathParams,
  curvature = 0.28,
): [string, number, number, number, number] {
  if (style === "smoothstep") return getSmoothStepPath({ ...params, borderRadius: 4 });
  if (style === "straight") return getStraightPath(params);
  return getBezierPath({ ...params, curvature });
}
