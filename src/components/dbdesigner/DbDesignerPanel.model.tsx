import "@xyflow/react/dist/style.css";
import { groupByPrefix } from "./series";
import DesignNodeCard from "./DesignNodeCard";
import { NODE_COLORS } from "./nodeColors";
import { createEventBuffer } from "../../utils/eventBuffer";
import { type DbDesignDocument, type DbField } from "./types";
// nodeTypes 必须是组件外的常量：内联新建会让 React Flow 每次渲染都重挂载节点（官方文档警告的卡顿源）。
export const nodeTypes = { designNode: DesignNodeCard };

/** dock 高度（px）：拖分隔线调整并记住。上下限 = 至少能看到表头+页签+一行字段，
 *  且必须给画布留 MIN_CANVAS_HEIGHT，否则窗口小的时候画布会被挤没。 */
export const DOCK_HEIGHT_KEY = "kira.dbd.dockHeight";

export const FIELD_ROWS_KEY = "kira.dbd.fieldRows";

/** 模块级进度缓冲：与思维导图同款（keep-alive 切换页面也不会丢事件）。
 *  data：stream/reasoning 帧携带 length（累计字符）+ text（末尾 200 字预览）；
 *  reasoning 收尾帧带 done=true。 */
export const dbdAiProgressBuffer = createEventBuffer<{
  step?: string;
  data?: {
    length?: number;
    text?: string;
    done?: boolean;
    /** step === "repair" 时：自校验重修的进度 */
    round?: number;
    max?: number;
    errors?: number;
  };
}>("dbd-ai-progress", { owner: "dbdesigner" });

/**
 * 流式构图：后端每写完一张表就推一份「到目前为止画得出来的图」。
 * 单独一个事件而不是塞进 progress —— 载荷是整份部分文档，混在进度事件里
 * 会让 UI 那边每次都要判断是不是文档。
 */
// limit 8：只消费最新那份，旧的部分文档留着没用还占内存（每份都是全量节点）。
export const dbdAiPartialBuffer = createEventBuffer<{ runId?: string; doc?: DbDesignDocument }>(
  "dbd-ai-partial",
  { limit: 8, owner: "dbdesigner" },
);

/** 流式落图时一行放几张表（只影响生成过程中的临时排布，最终仍走自动布局） */
export const AI_STREAM_COLS = 5;

/** AI 模块 `get_ai_config` 返回的供应商子集（本面板只要挑供应商 + 模型） */
export interface AiProviderLike {
  id: string;
  name: string;
  api_key?: string;
  openai_url?: string;
  active_model_id?: string;
  models?: { id: string; name?: string }[];
}

/**
 * 同系列自动配色：给每个节点挑一个颜色。
 *
 * 分组规则复用 series.ts 的 groupByPrefix（表名前缀 ∪ 外键）—— 与自动布局是同一份「系列」定义。
 * 两处若各写一套，就会出现「同色却离得很远」这种自相矛盾的画面。
 */
export function assignSeriesColors(doc: DbDesignDocument): Map<string, string> {
  // 与自动布局**同一份系列口径**（前缀 ∪ 外键）：口径分叉就会出现「颜色成对、位置分开」
  const groups = groupByPrefix(doc);
  const byNode = new Map<string, string>();
  groups.forEach((group, i) => {
    // 颜色按系列在文档中的出现顺序分配（不按 hash、不随机）：同一份设计每次打开配色都一致。
    // 随机色会让「重新点一次自动配色，全场颜色全变」，用户会以为手滑了。
    const color = NODE_COLORS[i % NODE_COLORS.length];
    for (const n of group) byNode.set(n.id, color);
  });
  return byNode;
}

/** 高亮层级的偏好键（1-5 跳）。属「摆放/观感」偏好，不进设计文件。 */
export const HIGHLIGHT_DEPTH_KEY = "kira.dbd.highlightDepth";

/** 布局方式的偏好键（放射 / 核心左右），与连线样式同一套持久化做法 */
export const LAYOUT_MODE_KEY = "kira.graph.dbd.layoutMode";

export const MIN_DOCK_HEIGHT = 150;

export const MIN_CANVAS_HEIGHT = 120;

export const DEFAULT_DOCK_HEIGHT = 320;

export const FK_ACTIONS = ["RESTRICT", "CASCADE", "SET NULL", "NO ACTION"];

export const REL_KINDS = ["1-1", "1-n", "n-n"];

/**
 * 连线样式。React Flow 内置三种，ER 图各有适用场景：
 *   bezier     曲线 —— 关系少时最顺眼（默认）
 *   smoothstep 直角折线 —— 表多时不互相穿插，接近 PowerDesigner 的观感
 *   straight   直线 —— 极简，节点排整齐时最清爽
 *
 * 本面板用的是**内置** edge，所以样式直接当 `type` 用；取值 / 持久化 / 文案
 * 与思维导图、JSON 图共用 `utils/graphEdgeStyle`（那边是自定义 edge，走 `edgePath()`）。
 */

export function newId(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;
}

export function emptyField(base = "bigint"): DbField {
  // 字段名默认 `field` 而不是 `id`：裸 id 会被命名校验判为 error
  //（应用侧 validate_naming 要求主键用 <实体>_id，且 id 命中通用裸名黑名单）
  return { name: "field", type: { base }, nullable: false, pk: false, autoIncrement: false, unique: false, comment: "" };
}
