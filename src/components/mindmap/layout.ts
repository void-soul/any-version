import type { MindmapNode } from "./types";
import { isTreeLayoutDir, treeLayout, type TreeLayoutDir } from "../../utils/treeLayout";

// ════════════ 自动布局（纯函数，可独立测试）════════════
//
// 算法本体已上移到 `utils/treeLayout.ts` —— JSON 浏览的画布用的是同一份。
// 早先那边是 `depth * 260` 的硬编码网格：父节点与子节点在堆叠轴上毫无关系，
// 键一多整张图就散成一片竖条，看不出谁是谁的父。现在两个画布口径一致。
//
// 本文件只保留**思维导图自己的尺寸常量**：它们要跟 FlowNode 的 `w-[200px]` 对齐，
// 所以属于本模块而不是共享层。

/** 布局方向：lr=左→右（默认，根在左） rl=右→左 tb=上→下 bt=下→上 */
export type LayoutDir = TreeLayoutDir;

export const isLayoutDir = isTreeLayoutDir;

/** 节点卡片宽度，与 FlowNode 的 w-[200px] 一致。 */
const CARD_W = 200;
/** 节点卡片高度的**兜底**值（React Flow 还没测到时的估计）。 */
const CARD_H = 90;
/**
 * 层间距：沿展开方向，卡片之间留出的「连线走廊」。
 * 24 太窄 —— 卡片宽 200，走廊只剩 24px，连线的箭头几乎贴着下一张卡片的边框。
 */
const LEVEL_GAP = 56;
/**
 * 兄弟间距：堆叠方向上卡片之间的呼吸空间。
 *
 * 早先固定 30（配合固定行高 90 → 步长 120）。但卡片高度是**内容驱动**的
 * （标题栏 33 + 0~3 行证据文件，实测 56~110px），一律按 90 留白的结果是
 * 矮卡片之间空一大截、而超过 90 的卡片反而会互相压住。
 * 现在行高由**该行卡片的实测高度**决定，间隙只需保证不粘连。
 */
const STACK_GAP = 22;

export function layoutTree(
  nodes: MindmapNode[],
  dir: LayoutDir = "lr",
  /**
   * React Flow 实测的节点尺寸。给了就按**真实尺寸**排行高 —— 卡片高度是内容驱动的
   * （标题栏 + 0~3 行证据文件），用固定 90 估会白留空隙、超高的卡片还会压住邻居。
   * 缺省时退回 CARD_W / CARD_H（首帧 RF 还没测量时就是这条路）。
   */
  measured?: Map<string, { width: number; height: number }>,
): Map<string, { x: number; y: number }> {
  return treeLayout(nodes, dir, measured, { cardW: CARD_W, cardH: CARD_H, levelGap: LEVEL_GAP, stackGap: STACK_GAP });
}
