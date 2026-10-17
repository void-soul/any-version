// 数据库设计器面板（P2 画布）。
//
// 形态：**纯文件编辑器** —— 顶部工具栏管文件（新建/打开/保存/导出），
// 中间 React Flow 画布画「表 / 视图」，右侧检查器改字段与关联。
// 文档只在前端内存里，保存才落盘；校验交给后端 `dbd_validate`（单一真源）。
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  Background,
  ConnectionMode,
  MarkerType,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
  type Connection,
  type Edge,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { CONNECTION_LINE_BY_STYLE, useEdgeStyle } from "../../utils/graphEdgeStyle";
import { mergeDecisions, trimHistory } from "./aiMemory";
import { modelChoiceSettings, resolveModelChoice } from "./modelChoice";
import {
  createLoadSeq,
  createSessionId,
  isUsableMessage,
  resolveSessionId,
  sessionTitle,
  type SessionEntry,
} from "./sessionKey";
import type { ProviderLike } from "../ai/ModelSelector";
import { cardClickAction, clickRoleOf } from "./cardClick";
import { DEFAULT_LAYOUT_MODE, computeLayout, type LayoutMode } from "./layout";
import {
  buildArrangeSubDoc,
  clampHighlightDepth,
  groupByPrefix,
  relatedEdgeIds,
  relatedTableHops,
  relatedTableIds,
  type HighlightDepth,
} from "./series";
import EdgeStyleSelect from "../shared/EdgeStyleSelect";
import { Menu } from "../shared/Menu";
import {
  ArrowRight,
  ChevronDown,
  Loader2,
  ChevronUp,
  Database,
  Eye,
  FileCode,
  FileDown,
  FilePlus2,
  Plus,
  ListTree,
  Sparkles,
  SlidersHorizontal,
  Trash2,
} from "lucide-react";

import DesignNodeCard, { type DesignNodeData } from "./DesignNodeCard";
import FieldTable from "./FieldTable";
import GraphLookPanel from "./GraphLookPanel";
import AiSidebar from "./AiSidebar";
import { patchCounts, patchLast, startTurn, type ChatMsg } from "./chatState";
import CommonFieldsDialog from "./CommonFieldsDialog";
import ExportDialog from "./ExportDialog";
import { predictSqlPath, type ExportChoice } from "./exportOptions";
import { NODE_COLORS, isPresetColor } from "./nodeColors";
import { EDGE_FLOW_CLASS, edgeGlow } from "../../utils/graphHighlight";
import {
  DIM_MAX,
  HL_BOOST_MAX,
  dimmedEdgeStroke,
  dimmedEdgeWidth,
  dimmedOpacity,
  hasVisibleHighlight,
  relatedEdgeWidth,
  useGraphHighlightPrefs,
} from "../../utils/graphHighlight";
import { useAiPanelWidth } from "../ai/paneWidth";
import { createEventBuffer, useEventBufferSnapshot } from "../../utils/eventBuffer";
import { ResultNote } from "../shared/Note";
import { HoverPreview, useHoverPreview } from "../shared/HoverPreview";
import { theamedConfirm } from "../shared/ThemedAlert";
import { toast } from "../shared/Toast";
import {
  DIALECTS,
  FIELD_ROW_MODES,
  HANDLE_TABLE,
  HANDLE_TABLE_IN,
  edgeAnchors,
  fkFieldsOf,
  handleAllPk,
  relationFieldsOf,
  typeLabel,
  type FieldRowMode,
  type DbDesignDocument,
  type DbDesignNode,
  type DbDesignRelation,
  type AiOutcome,
  type ChatTurn,
  type DbField,
  type DbIndex,
  type DbTableBody,
  type Dialect,
  type ValidationReport,
} from "./types";

// nodeTypes 必须是组件外的常量：内联新建会让 React Flow 每次渲染都重挂载节点（官方文档警告的卡顿源）。
const nodeTypes = { designNode: DesignNodeCard };

/** 画布内的「适应视野」触发器：signal 每 +1 就重新 fitView 一次。
 *  必须待在 ReactFlowProvider 里面（useReactFlow 依赖它），所以单独拆个小组件。
 *
 *  `ids` 给定时只把这批节点纳入视野：「整理选中」后要定位到刚整理的那几张表，
 *  而不是把整份设计（几十张表）缩到全览 —— 那等于没定位。ids 走 ref 传，
 *  这样 effect 只依赖 signal：ids 与 signal 同一次更新里写入，不会触发多余的适配。 */
function FitOnSignal({ signal, ids }: { signal: number; ids?: string[] }) {
  const { fitView, getNodes } = useReactFlow();
  const first = useRef(true);
  const idsRef = useRef(ids);
  idsRef.current = ids;
  useEffect(() => {
    if (first.current) {
      first.current = false; // 首挂载交给 <ReactFlow fitView>，别动初始视野
      return;
    }
    if (signal > 0) {
      const want = idsRef.current;
      const nodes =
        want && want.length > 0 ? getNodes().filter((n) => want.includes(n.id)) : undefined;
      // 指定了 ids 但一个都没匹配上（节点刚好被删）→ 退回全图，别让视野卡在空处
      void fitView({
        nodes: nodes && nodes.length > 0 ? nodes : undefined,
        padding: 0.15,
        duration: 260,
      });
    }
  }, [signal, fitView, getNodes]);
  return null;
}

/**
 * 把画布平移到某个节点并居中。
 *
 * 表一多，在画布上「找到那张表」就得拖来拖去 —— 这是导航列表存在的原因。
 * 单独做成信号组件：定位需要 `useReactFlow`，而它只能在 ReactFlowProvider 内调用，
 * 面板本体在 Provider 之外。
 */
function CenterOnSignal({ signal, nodeId }: { signal: number; nodeId: string | null }) {
  const { setCenter, getNode } = useReactFlow();
  useEffect(() => {
    if (signal <= 0 || !nodeId) return;
    const n = getNode(nodeId);
    if (!n) return;
    const w = n.measured?.width ?? n.width ?? 288;
    const h = n.measured?.height ?? n.height ?? 200;
    void setCenter(n.position.x + w / 2, n.position.y + h / 2, { zoom: 1, duration: 320 });
  }, [signal, nodeId, setCenter, getNode]);
  return null;
}

/** dock 高度（px）：拖分隔线调整并记住。上下限 = 至少能看到表头+页签+一行字段，
 *  且必须给画布留 MIN_CANVAS_HEIGHT，否则窗口小的时候画布会被挤没。 */
const DOCK_HEIGHT_KEY = "kira.dbd.dockHeight";
const FIELD_ROWS_KEY = "kira.dbd.fieldRows";
/** 模块级进度缓冲：与思维导图同款（keep-alive 切换页面也不会丢事件）。
 *  data：stream/reasoning 帧携带 length（累计字符）+ text（末尾 200 字预览）；
 *  reasoning 收尾帧带 done=true。 */
const dbdAiProgressBuffer = createEventBuffer<{
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
}>("dbd-ai-progress");

/**
 * 流式构图：后端每写完一张表就推一份「到目前为止画得出来的图」。
 * 单独一个事件而不是塞进 progress —— 载荷是整份部分文档，混在进度事件里
 * 会让 UI 那边每次都要判断是不是文档。
 */
// limit 8：只消费最新那份，旧的部分文档留着没用还占内存（每份都是全量节点）。
const dbdAiPartialBuffer = createEventBuffer<{ runId?: string; doc?: DbDesignDocument }>(
  "dbd-ai-partial",
  { limit: 8 },
);

/** 流式落图时一行放几张表（只影响生成过程中的临时排布，最终仍走自动布局） */
const AI_STREAM_COLS = 5;

/** AI 模块 `get_ai_config` 返回的供应商子集（本面板只要挑供应商 + 模型） */
interface AiProviderLike {
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
function assignSeriesColors(doc: DbDesignDocument): Map<string, string> {
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
const HIGHLIGHT_DEPTH_KEY = "kira.dbd.highlightDepth";

/** 布局方式的偏好键（放射 / 核心左右），与连线样式同一套持久化做法 */
const LAYOUT_MODE_KEY = "kira.graph.dbd.layoutMode";

const MIN_DOCK_HEIGHT = 150;
const MIN_CANVAS_HEIGHT = 120;
const DEFAULT_DOCK_HEIGHT = 320;

const FK_ACTIONS = ["RESTRICT", "CASCADE", "SET NULL", "NO ACTION"];
const REL_KINDS = ["1-1", "1-n", "n-n"];

/**
 * 连线样式。React Flow 内置三种，ER 图各有适用场景：
 *   bezier     曲线 —— 关系少时最顺眼（默认）
 *   smoothstep 直角折线 —— 表多时不互相穿插，接近 PowerDesigner 的观感
 *   straight   直线 —— 极简，节点排整齐时最清爽
 *
 * 本面板用的是**内置** edge，所以样式直接当 `type` 用；取值 / 持久化 / 文案
 * 与思维导图、JSON 图共用 `utils/graphEdgeStyle`（那边是自定义 edge，走 `edgePath()`）。
 */

function newId(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;
}

function emptyField(base = "bigint"): DbField {
  // 字段名默认 `field` 而不是 `id`：裸 id 会被命名校验判为 error
  //（应用侧 validate_naming 要求主键用 <实体>_id，且 id 命中通用裸名黑名单）
  return { name: "field", type: { base }, nullable: false, pk: false, autoIncrement: false, unique: false, comment: "" };
}

/**
 * 表列表悬浮预览的正文：注释 + 完整字段清单 + 统计。
 *
 * **用普通 DOM 而不是 markdown**：字段天然是三列对齐的表（名 / 类型 / 标记 / 注释），
 * 走 markdown 反而要 `await invoke` 调后端的 `dbd_export_outline`（那是整文档级的、
 * 异步 IPC，每次悬浮发一次不可接受），或者把那段 Rust 逻辑复制一份到 TS。
 */
function TablePreviewBody({ row }: { row: { comment: string; fields: DbField[]; indexCount: number; relCount: number } }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-1.5">
      {row.comment && <p className="text-micro leading-relaxed text-slate-400">{row.comment}</p>}
      {row.fields.length === 0 ? (
        <p className="text-micro text-slate-600">{t("dbd.viewNoFields")}</p>
      ) : (
        <div className="space-y-px">
          {/* 字段用**真表格**展示，不用 flex 行。
              flex 行里字段名长短不一，类型与注释就跟着参差不齐，扫一眼对不上是哪一列；
              表格靠固定列宽天然对齐，跨表看字段清单时能直接按列比较
              （用户要求）。宽度按 320px 气泡分配：名字可截断、类型与注释各留够，
              完整内容都在 title 里。 */}
          <table className="w-full table-fixed text-micro">
            <colgroup>
              <col className="w-[38%]" />
              <col className="w-[26%]" />
              <col className="w-[11%]" />
              <col />
            </colgroup>
            <tbody>
              {row.fields.map((f) => (
                <tr key={f.name} className="align-top odd:bg-white/[0.015]">
                  <td
                    className="truncate py-0.5 pr-1.5 font-mono text-slate-200"
                    title={f.name}
                  >
                    {f.name}
                  </td>
                  <td className="truncate py-0.5 pr-1.5 font-mono text-slate-500" title={typeLabel(f.type)}>
                    {typeLabel(f.type)}
                  </td>
                  <td className="whitespace-nowrap py-0.5 pr-1.5">
                    {f.pk ? (
                      <span className="text-amber-400/90">PK</span>
                    ) : f.unique ? (
                      <span className="text-emerald-400/80">UQ</span>
                    ) : null}
                  </td>
                  <td className="truncate py-0.5 text-slate-500" title={f.comment || undefined}>
                    {f.comment}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <div className="flex items-center gap-2 border-t border-white/10 pt-1.5 text-micro text-slate-500">
        <span>{t("dbd.fieldCount", { count: row.fields.length })}</span>
        {row.indexCount > 0 && <span>{t("dbd.indexCount", { count: row.indexCount })}</span>}
        {row.relCount > 0 && <span>{t("dbd.relCount", { count: row.relCount })}</span>}
      </div>
    </div>
  );
}

export default function DbDesignerPanel() {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<DbDesignDocument | null>(null);
  const [path, setPath] = useState<string>("");
  const [dirty, setDirty] = useState(false);
  /**
   * 两个「选中」必须分开，否则没法实现「点表头只高亮、点字段才开面板」：
   *
   * - `selectedId`：**底部面板正在编辑哪张表**。点字段区域设置它，面板由它驱动
   *   （字段/索引/关联三个页签 + 表名/注释/标签/颜色都挂在它上面）。
   * - `highlightId`：**画布上哪张表高亮**（含一跳关联的高亮与无关项的淡化）。
   *   点表头设置它。整理选中、左侧导航的高亮也看它。
   *
   * 早先只有一个 selectedId，点哪都是「选中 + 开面板」；拆开之后面板的三十几处渲染
   * 一行都不用改，只是「高亮」不再由面板目标决定。
   */
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [highlightId, setHighlightId] = useState<string | null>(null);
  // 表列表的悬浮字段预览：定位 / 延迟关闭 / 移入本体不关，都在与思维导图共用的 hook 里
  const hp = useHoverPreview();
  /** 布局变化计数：+1 就让画布重新适应视野（自动布局后必须做，否则停在角落） */
  /**
   * 两个分开的适应视野信号，不能共用：
   *   - `fitSignal`：适应**整窗**（AI 生成落图、自动布局、自动配色）；
   *   - `fitFocusSignal`：适应**指定的那几张表**（整理选中），依赖 `fitFocusIds`。
   * 若共用，整理过后再点「自动布局」就会只适应那几张表而不是全图。
   */
  const [fitSignal, setFitSignal] = useState(0);
  const [fitFocusSignal, setFitFocusSignal] = useState(0);
  /** 整理选中后要定位到那几张表：下标这张。与 fitSignal 同次写入，
   *  避免两个状态分开更新导致视野与坐标不匹配。 */
  const [arrangeFitIds, setArrangeFitIds] = useState<string[]>([]);
  /** 表导航：点表名 → 选中它 + 画布平移过去（表一多，画布上「找到那张表」全靠拖） */
  const [centerSignal, setCenterSignal] = useState(0);
  const [centerTargetId, setCenterTargetId] = useState<string | null>(null);
  const [tableNavOpen, setTableNavOpen] = useState(false);
  /** 连线样式（曲线 / 直角折线 / 直线）：与思维导图、JSON 图共用一份偏好 */
  const [edgeStyle, setEdgeStyle] = useEdgeStyle();

  /**
   * 布局方式（核心左右 / 放射）。localStorage 持久化，形态与连线样式一致：
   * 两者都只影响观感、不进设计文件，但用户会希望「上次用哪个」。
   * 没有偏好记录时读 `DEFAULT_LAYOUT_MODE`，与 `computeLayout` 的缺省同一个来源 ——
   * 曾经这里写死`radial` 而函数默认 `radial`，改默认时漏了一处，
   * 会出现「下拉显示核心左右、实际排出来是放射」。
   */
  const [layoutMode, setLayoutMode] = useState<LayoutMode>(() => {
    const saved = localStorage.getItem(LAYOUT_MODE_KEY);
    // 认得的值才采纳；存了别的东西（改版后废弃的选项）回落到默认
    return saved === "radial" || saved === "coreLeftRight" ? saved : DEFAULT_LAYOUT_MODE;
  });
  /**
   * 高亮强度 / 淡化强度：持久化偏好（localStorage），默认值 = 改动前的观感。
   * 画布右下角的悬浮面板直接改它，rfNodes / rfEdges 的 useMemo 都依赖它 → 实时重画。
   */
  const {
    prefs: lookPrefs,
    setHighlight: setHighlightStrength,
    setDim: setDimStrength,
    reset: resetLookPrefs,
  } = useGraphHighlightPrefs();
  /** 高亮层级：1 = 只直接上级/下级，可调到五跳。与连线样式一样属观感偏好，
   *  记 localStorage；「整理选中」的范围也跟它走（两者必须是同一个集合）。 */
  const [highlightDepth, setHighlightDepthRaw] = useState<HighlightDepth>(() =>
    clampHighlightDepth(localStorage.getItem(HIGHLIGHT_DEPTH_KEY)),
  );
  const setHighlightDepth = useCallback((d: HighlightDepth) => {
    setHighlightDepthRaw(d);
    localStorage.setItem(HIGHLIGHT_DEPTH_KEY, String(d));
  }, []);
  const [report, setReport] = useState<ValidationReport | null>(null);
  /** 校验结果条是否展开：默认展开（有问题就该看得见），收起只留一行摘要把画布还回去 */
  const [reportOpen, setReportOpen] = useState(true);
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null);
  /**
   * 成功条幅 3 秒后自动消失，**错误条幅一律保留**。
   *
   * 分两种处理不是偷懒：成功消息（「已保存 xxx」）只是确认动作做完了，读完即无用；
   * 错误消息里带路径、校验清单这种要读两遍以上的内容，自动收掉就来不及看 ——
   * 而失败恰恰是最需要看清的时候。下一个动作（再存一次、导入、导出）会直接覆盖它。
   */
  useEffect(() => {
    if (!result || !result.ok) return;
    const timer = window.setTimeout(() => setResult(null), 3000);
    // 期间若来了新消息（成功或失败），撤掉这个定时器，否则会把新消息误关掉
    return () => window.clearTimeout(timer);
  }, [result]);
  /** 导出弹框：只在打开时挂载（这样每次打开都是默认选项，不记忆上次选择） */
  const [exportOpen, setExportOpen] = useState(false);
  const [commonFieldsOpen, setCommonFieldsOpen] = useState(false);
  const [exportBusy, setExportBusy] = useState(false);
  /** 底部 dock：页签（字段 / 索引 / 关联）+ 折叠。属性编辑全在这里，画布只负责摆表。 */
  const [dockTab, setDockTab] = useState<"fields" | "indexes" | "relations">("fields");
  /** 关联页签筛选：全部 / 我是主表 / 我是子表 */
  const [relFilter, setRelFilter] = useState<"all" | "parent" | "child">("all");
  /** 标签筛选（画布）：命中的表才显示；空 = 全显示 */
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  /** 新标签输入框 */
  const [newTag, setNewTag] = useState("");
  // ── AI 助手 ──
  // 右侧栏（与思维导图 AI 栏同构）：aiOpen=展开；宽度持久化，拖拽范围 300~640
  const [aiOpen, setAiOpen] = useState(false);
  const [aiPanelW, setAiPanelW] = useAiPanelWidth();
  const [aiText, setAiText] = useState("");
  const [aiBusy, setAiBusy] = useState(false);
  /** 本轮是主动停止的：控制台收尾显示「已停止」而不是「已完成」 */
  const [, setAiAborted] = useState(false);
  const [, setAiErr] = useState("");
  /**
   * 会话历史（滚动窗口，最近 3 轮）。
   *
   * **不进设计文件**：它是对话过程的脚手架，不是设计内容。放进文件会让它越滚越大、
   * 分享文件时把两个 AI 的来回对话也发给了对方。真正的记忆是 `doc.decisions`。
   */
  const [aiHistory, setAiHistory] = useState<ChatTurn[]>([]);
  /**
   * 对话气泡（临时，关掉面板就丢）。
   *
   * 为什么不进设计文件：它是对话过程的脚手架，不是设计内容，放进文件会让它越滚越大。
   * 真正的长期记忆是 `doc.decisions`（在顶部那个按钮里）。
   */
  const [chatMsgs, setChatMsgs] = useState<ChatMsg[]>([]);
  /** 模型停下来问问题时的状态（非空 = 交洽卡片应渲染出来） */
  const [, setAiClarify] = useState<{ question: string; options: string[] } | null>(null);
  /** 已追问几轮：传给后端写进提示词，让模型知道自己问过几次 */
  const [aiClarifyRound, setAiClarifyRound] = useState(0);
  /** 本轮新增的决策：生成完在侧栏里展示给你看，不对即归入文档形式的“已记入” */
  const [, setAiNewDecisions] = useState<string[]>([]);
  /** 本轮开始前的文档：追问时用它把画布恢复回来（追问不落图） */
  const [docSnapshot, setDocSnapshot] = useState<DbDesignDocument | null>(null);
  /** 决策记录区的展开状态：默认收起——不收起就是一块常驻的字符迹 */
  const [decisionsOpen, setDecisionsOpen] = useState(false);
  /** 决策记录的输入草稿（回车即提交） */
  const [decisionDraft, setDecisionDraft] = useState("");
  const [aiProviders, setAiProviders] = useState<AiProviderLike[]>([]);
  const [aiProviderId, setAiProviderId] = useState("");
  const [aiModelId, setAiModelId] = useState("");
  /**
   * 当前设计绑着的会话 id。
   *
   * 它就是 `doc.id`（外部文件缺 id 时会在 loadChatForDoc 里分配一个补上），
   * 这里另存一份是为了**读盘期间 doc 还是原值**时也能拿到正确的 key 去写。
   */
  const [sessionId, setSessionId] = useState("");
  /** 会话索引（弹框列表 + resolveSessionId 的输入） */
  const [sessions, setSessions] = useState<SessionEntry[]>([]);
  /**
   * 「读盘中」—— 换文档 / 切会话后，会话要从磁盘异步读回来，这段窗口里
   * `sessionId` 与 `aiHistory` 还是上一份的。用户此时回车就是把「新表 + 旧对话」
   * 发给模型、并把这一轮存进旧会话，**串台换个入口复活**。
   *
   * 所以这段时间禁用发送。`chatLoadSeq` 同时是那道竞态闸门
   * （后发起的接管先发起的），状态机与不变量见 `sessionKey.ts::createLoadSeq`。
   */
  const [chatLoading, setChatLoading] = useState(false);
  const chatLoadSeqRef = useRef(createLoadSeq((loading) => setChatLoading(loading)));
  /**
   * 最新的 `chatMsgs`，供**写盘**时读。
   *
   * 为什么需要 ref：React state 是异步的，一轮生成刚跑完时闭包里的 `chatMsgs`
   * 还是「补上最终气泡之前」的那份。写盘要的是后者。
   * （不能改在 `setChatMsgs` 的 updater 里读——那里面做副作用会被严格模式调用两次。）
   */
  const chatMsgsRef = useRef<ChatMsg[]>(chatMsgs);
  useEffect(() => {
    chatMsgsRef.current = chatMsgs;
  }, [chatMsgs]);
  /** 本次运行的 id：发给后端，用于「停止」 */
  const aiRunIdRef = useRef("");
  /** 流式落图时已经排好位的节点（id → 坐标）：同一个节点不会被后来的批次挪动 */
  const aiPlacedRef = useRef<Record<string, { x: number; y: number }>>({});
  const aiProgress = useEventBufferSnapshot(dbdAiProgressBuffer);
  /** 思考的完整文本与是否还在想：供 <ThinkingBlock> 展示。
   *  `text` 从这一版起是**累积全文**（后端以前只给尾部 200 字），展开能看到完整推理链条。
   *  与下面 aiStatusText 用的 aiThinkFinal 的区别：它**不因 done 而变 null** ——
   *  思考完成后那块还在，只是从「思考中」变成可展开的成品，随时能回头看。 */
  const aiThinking = useMemo(() => {
    for (let i = aiProgress.length - 1; i >= 0; i--) {
      const e = aiProgress[i];
      if (e.step === "reasoning") {
        return { text: e.data?.text ?? "", done: !!e.data?.done };
      }
    }
    return null;
  }, [aiProgress]);
  /** 思考块最终状态：最后一帧 reasoning（done=true 即思考结束，length 为总字数） */
  const aiThinkFinal = useMemo(() => {
    for (let i = aiProgress.length - 1; i >= 0; i--) {
      const e = aiProgress[i];
      if (e.step === "reasoning") return { length: e.data?.length ?? 0, done: !!e.data?.done };
    }
    return null;
  }, [aiProgress]);
  /** 正文输出的最新帧：运行中是实时预览；跑完后缓冲里最后一帧 = 最终字数 */
  const aiStream = useMemo(() => {
    for (let i = aiProgress.length - 1; i >= 0; i--) {
      const e = aiProgress[i];
      if (e.step === "stream") return { length: e.data?.length ?? 0, text: e.data?.text ?? "" };
    }
    return null;
  }, [aiProgress]);
  /** 进度变化时把当轮气泡也更新一下：思考中→正在写、字数不断增长。
      不跟的话气泡会一直卡在「思考中」，用户以为卡死了。*/
  useEffect(() => {
    if (!aiBusy) return;
    setChatMsgs((m) =>
      patchLast(m, {
        status: aiStream ? "writing" : "thinking",
        thinking: aiThinking?.text,
        chars: aiStream?.length,
      }),
    );
  }, [aiBusy, aiThinking, aiStream]);

  /**
 * 自校验重修进度：最后一帧 repair 事件。
 * 重修期间没有 stream 帧，所以状态条会掉回「思考中」——用它把「正在自动修复 1/2」显示出来，
 * 否则用户只看到卡住不动，不知道系统在替它改错。
 */
const aiRepair = useMemo(() => {
  for (let i = aiProgress.length - 1; i >= 0; i--) {
    const e = aiProgress[i];
    if (e.step === "repair") {
      return { round: e.data?.round ?? 1, max: e.data?.max ?? 2, errors: e.data?.errors ?? 0 };
    }
  }
  return null;
}, [aiProgress]);

/** 状态条 / 胶囊文案：思考中 → 写设计中 → 自动修复中 → 已完成 */
const aiStatusText = aiBusy
  ? aiRepair
    ? t("dbd.aiRepairing", { round: aiRepair.round, max: aiRepair.max })
    : aiStream
      ? t("dbd.aiWritingChars", { count: aiStream.length })
      : aiThinkFinal
        ? t("dbd.aiThinkingChars", { count: aiThinkFinal.length })
        : t("dbd.aiThinking")
    : t("dbd.aiDoneShort");

  // 供应商 / 模型列表来自 AI 模块的现有配置（与思维导图、API 模块同一份）
  useEffect(() => {
    // 记下「上次用的模型」，与设置读**并行**发起：串行会让空态闪一下
    const remembered = invoke<{ lastProviderId?: string; lastModelId?: string }>(
      "dbd_get_settings",
    ).catch(() => null);
    invoke<any>("get_ai_config")
      .then(async (cfg) => {
        const list: AiProviderLike[] = (cfg?.providers ?? []).filter(
          (p: AiProviderLike) => p.api_key && p.openai_url,
        );
        setAiProviders(list);
        const saved = await remembered;
        // 沿用规则（逐级回落 + 先验证还存不存在）挪到 modelChoice.ts：
        // 它连同「切换后写回 dbd_save_settings」是一条完整的契约，拆散在两个
        // effect 里时，其中一半很容易被面板重写顺手带走又没人发现。
        const choice = resolveModelChoice({ providers: list, remembered: saved });
        if (!choice) return;
        setAiProviderId(choice.providerId);
        setAiModelId(choice.modelId);
      })
      .catch(() => {});
  }, []);

  /**
   * 切换 AI 模型。
   *
   * **必须持久化**：主模块切换是 `<Activity>` keep-alive，切回时上面那个挂载
   * effect 会重跑一遍「读设置 + 决定用哪个模型」。不写回的话，你刚选好的模型
   * 会在切回时被覆盖回默认 —— 这段代码曾经随 UI 一起被删掉过，后果正是
   * 「选不了模型、而且选的也记不住」。
   *
   * 载荷字段名由后端按名读，不能改（见 modelChoice.modelChoiceSettings）。
   */
  const onAiModelSelect = useCallback((providerId: string, modelId: string) => {
    setAiProviderId(providerId);
    setAiModelId(modelId);
    void invoke("dbd_save_settings", {
      settings: modelChoiceSettings({ providerId, modelId }),
    }).catch(() => {});
  }, []);
  const [dockOpen, setDockOpen] = useState(true);
  /** 用户手动收起后上锁：即使有选中也不自动展开（否则一点「收起」立刻被弹回来）。
   *  解锁时机 = 再次点展开按钮，或点画布取消选中（那条规则本身就是「收缩」）。 */
  const dockLockedRef = useRef(false);
  /** dock 高度（px）。拖分隔线调整，记住到 localStorage —— 属于「摆放」偏好，不进设计文件。 */
  const [dockHeight, setDockHeight] = useState<number>(() => {
    const raw = Number(localStorage.getItem(DOCK_HEIGHT_KEY));
    return Number.isFinite(raw) && raw >= MIN_DOCK_HEIGHT ? Math.round(raw) : DEFAULT_DOCK_HEIGHT;
  });
  const [dockDragging, setDockDragging] = useState(false);
  /** 节点卡片里显示哪些字段行：只主键+外键 / 全部。观感偏好，记 localStorage。 */
  const [fieldRows, setFieldRows] = useState<FieldRowMode>(() => {
    const saved = localStorage.getItem(FIELD_ROWS_KEY);
    return FIELD_ROW_MODES.includes(saved as FieldRowMode) ? (saved as FieldRowMode) : "keys";
  });
  const rootRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ startY: number; startHeight: number } | null>(null);
  const dockHeightRef = useRef(dockHeight);
  dockHeightRef.current = dockHeight;

  // 选中表 → 自动展开；点画布取消选中 → 自动收缩。
  // 用「节点是否真的还在文档里」判断：逆向替换整个文档后 selectedId 可能已失效，
  // 那时不该显示成「选中了」的样子。
  const selectedAlive = !!selectedId && !!doc?.nodes.some((n) => n.id === selectedId);
  useEffect(() => {
    if (selectedAlive) {
      if (dockLockedRef.current) return;
      setDockOpen(true);
    } else {
      dockLockedRef.current = false;
      setDockOpen(false);
    }
  }, [selectedAlive]);

  /** 拖分隔线：向上拖变高。clamp 住，别把画布挤到 0 或把 dock 压到看不见表头。 */
  const clampDockHeight = useCallback((px: number) => {
    const total = rootRef.current?.clientHeight ?? 0;
    const max = Math.max(MIN_DOCK_HEIGHT, total - MIN_CANVAS_HEIGHT);
    return Math.min(Math.max(Math.round(px), MIN_DOCK_HEIGHT), max);
  }, []);

  const onSplitterDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    dragRef.current = { startY: e.clientY, startHeight: dockHeightRef.current };
    setDockDragging(true);
  };
  const onSplitterMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    setDockHeight(clampDockHeight(drag.startHeight + (drag.startY - e.clientY)));
  };
  const onSplitterUp = (e: React.PointerEvent<HTMLDivElement>) => {
    e.currentTarget.releasePointerCapture(e.pointerId);
    dragRef.current = null;
    setDockDragging(false);
  };
  /** 拖完/键盘调完再落盘：拖动过程中每帧都写 localStorage 太浪费。 */
  useEffect(() => {
    if (dockDragging) return;
    localStorage.setItem(DOCK_HEIGHT_KEY, String(dockHeight));
  }, [dockHeight, dockDragging]);

  // 文档一变就问后端要校验结果（后端是唯一真源，前端不复制校验规则）
  useEffect(() => {
    if (!doc) {
      setReport(null);
      return;
    }
    let alive = true;
    void invoke<ValidationReport>("dbd_validate", { doc }).then((r) => {
      if (alive) setReport(r);
    });
    return () => {
      alive = false;
    };
  }, [doc]);

  const patchDoc = useCallback((next: DbDesignDocument) => {
    setDoc(next);
    setDirty(true);
  }, []);

  /** 保存 / 导出前的校验闸门：返回 false 表示「有阻断级问题，别往下走」。
   *
   *  为什么要有这道闸门：后端 `dbd_export_sql` / `dbd_save_file` 校验不过时会把
   *  **整份错误清单**（`设计文档校验未通过：\n- …\n- …`，几十行）当作错误字符串抛回来。
   *  早先直接 `setResult({ ok: false, msg })` 塞进顶部的 ResultNote —— 那是个无上限的
   *  内联块，几十行一换行就把画布整个顶没了（用户看到的是「整个界面被错误盖住」，
   *  而且不能滚动）。而面板里本来就有一条**限高可滚动、可折叠**的校验条，
   *  错误清单天生该进那里。
   *
   *  所以这里先自己问一次后端 `dbd_validate`（单一真源，不在前端复制校验规则），
   *  把结果塞进那条校验条并展开它，只用 toast 报一句「有几项未通过」，
   *  错误详情去校验条里看 —— 画布始终拿得回来。
   */
  const ensureValid = async (): Promise<boolean> => {
    if (!doc) return false;
    const r = await invoke<ValidationReport>("dbd_validate", { doc });
    setReport(r);
    if (r.errors.length === 0) return true;
    setReportOpen(true);
    toast(t("dbd.blockedByValidation", { count: r.errors.length }), "err");
    return false;
  };

  // 同系列自动配色：只在**节点集合或关系端点**变化时重算，拖动节点/切页签不会触发，
  // 否则每次微动都可能去改文档，把文档标记成「已修改」。
  const autoColorSigRef = useRef("");
  useEffect(() => {
    if (!doc) return;
    const sig =
      doc.nodes.map((n) => n.id).sort().join(",") +
      "|" +
      doc.relations.map((r) => `${r.from.node}>${r.to.node}`).sort().join(",");
    if (sig === autoColorSigRef.current) return;
    autoColorSigRef.current = sig;

    const bySeries = assignSeriesColors(doc);
    let changed = false;
    const nodes = doc.nodes.map((n) => {
      // 手选色优先：用户手动挑过的颜色绝不覆盖
      if (n.color) return n;
      const c = bySeries.get(n.id);
      if (!c || c === n.color) return n;
      changed = true;
      return { ...n, color: c };
    });
    if (changed) patchDoc({ ...doc, nodes });
  }, [doc, patchDoc]);

  /** 交回自动配色：清掉所有手选色，让同系列重新按分组上色 */
  const autoColorAll = useCallback(() => {
    if (!doc) return;
    autoColorSigRef.current = ""; // 强制下一轮 effect 重算
    patchDoc({ ...doc, nodes: doc.nodes.map((n) => ({ ...n, color: "" })) });
  }, [doc, patchDoc]);

  // ── 文件 ──
  /**
   * 把某个会话登记进索引（upsert，不是整份覆盖）。
   *
   * 为什么 upsert：前端手里那份 `sessions` 可能已经过期（另一个窗口改过），
   * 整份写回会把别人的条目抹掉。交给后端按 id 合并。
   */
  const touchSession = useCallback(
    async (id: string, filePath: string) => {
      if (!id) return;
      try {
        const title = sessionTitle(filePath, doc?.name?.trim() || t("dbd.untitled"));
        const next = await invoke<{ sessions: SessionEntry[] }>("dbd_upsert_session_entry", {
          id,
          title,
          path: filePath,
        });
        setSessions(next.sessions);
      } catch {
        // 索引写不进去不该挡住对话：顶多列表里少一条，对话本身还在
      }
    },
    [doc?.name, t],
  );

  /**
   * **唯一的会话写盘出口**：`runAi` 成功轮次与切换文档前的 `saveChatNow` 都走它。
   *
   * 两件事一起做，缺一不可：
   * 1. 写正文 `chats/<id>.json`；
   * 2. `touchSession` 刷新索引里的 `updatedAt` —— 少了它，列表排序停在「最近打开」
   *    而不是「最近活跃」，而且**删掉当前会话后再聊一轮会把文件写回来、
   *    索引里却没有**，于是这个会话在列表里彻底消失（用户看得见的孤儿）。
   *
   * 写盘失败一律静默：对话照常显示，只是没存住 —— 不值得为缓存失败打断界面。
   */
  const commitChat = useCallback(
    async (chat: { msgs: ChatMsg[]; history: ChatTurn[]; clarifyRound: number }) => {
      const id = sessionId;
      if (!id) return;
      try {
        await invoke("dbd_save_chat", {
          id,
          chat: {
            chatMsgs: chat.msgs,
            aiHistory: chat.history,
            aiClarifyRound: chat.clarifyRound,
          },
        });
        void touchSession(id, path);
      } catch {
        // 同上：存不住不是致命错误
      }
    },
    [sessionId, path, touchSession],
  );

  /** 把当前对话写进当前会话（切换文档前调，避免这一轮的对话随组件状态丢掉）。 */
  const saveChatNow = useCallback(async () => {
    if (!sessionId) return;
    await commitChat({
      msgs: chatMsgsRef.current,
      history: aiHistory,
      clarifyRound: aiClarifyRound,
    });
  }, [sessionId, commitChat, aiHistory, aiClarifyRound]);

  /**
   * 换文档时读回它自己的会话。
   *
   * **取代了原来那串手写的清空 setter**（`setAiHistory([])` / `setAiClarify(null)` …）：
   * 读取结果直接覆盖，语义上更对，也就不会再像 `chatMsgs` 那样漏掉一个字段
   * （漏掉的后果就是「新建文档后还留着上一份的对话」）。
   */
  const loadChatForDoc = useCallback(
    async (d: DbDesignDocument, filePath: string) => {
      const token = chatLoadSeqRef.current.begin();
      let index: SessionEntry[] = [];
      try {
        const got = await invoke<{ sessions: SessionEntry[] }>("dbd_get_session_index");
        index = got.sessions ?? [];
      } catch {
        // 读不到索引就按「没有记录」处理，仍然会分配一个新 id
      }
      const r = resolveSessionId({ docId: d.id, path: filePath, index });
      let chat: { chatMsgs?: unknown[]; aiHistory?: unknown[]; aiClarifyRound?: number } = {};
      try {
        chat = await invoke("dbd_get_chat", { id: r.id });
      } catch {
        // 会话读不出来 → 当成空会话（后端已做同样兜底，这里防的是 invoke 本身失败）
      }
      // 期间用户又切了文档 → 这次结果作废
      if (!chatLoadSeqRef.current.alive(token)) return;

      setSessionId(r.id);
      setChatMsgs((chat.chatMsgs ?? []).filter(isUsableMessage) as ChatMsg[]);
      setAiHistory((chat.aiHistory ?? []) as ChatTurn[]);
      setAiClarifyRound(chat.aiClarifyRound ?? 0);
      // 追问状态不进会话缓存：它只在单次追问流程内有意义
      setAiClarify(null);
      setAiNewDecisions([]);
      setDocSnapshot(null);
      // 外部文件没有 id 而被分配了新 id → 写回文档，文档因此变脏。
      // 用函数式更新并校验来源：那两次 IPC 期间用户可能拖过节点，
      // 拿调用时刻的整份快照回去覆盖会把他的拖动回滚掉。
      if (d.id !== r.id) {
        setDoc((prev) => (prev && prev.id === d.id ? { ...prev, id: r.id } : prev));
        setDirty(true);
      }
      // 索引以 touchSession 的返回为准（它是后端给的权威列表）。
      // 刻意**不**在这里 setSessions(index)：读索引失败时 index 是空数组，
      // 那会把内存里已有的列表清空 —— 一次读失败让用户看到「会话全没了」。
      void touchSession(r.id, filePath);
      // 读到会话、状态已归位 → 「读盘中」解除。上面那条 `return` 是被后继者
      // 接管的情况，此时**不** settle：收尾归接管者管（见 createLoadSeq 的契约）。
      chatLoadSeqRef.current.settle(token);
    },
    [touchSession],
  );

  const newDoc = async () => {
    const d = await invoke<DbDesignDocument>("dbd_new_document", { name: t("dbd.untitled"), dialect: "mysql" });
    // 先把当前会话存住，再切走 —— 否则这一轮的对话就随组件状态一起丢了
    void saveChatNow();
    setDoc(d);
    setPath("");
    setDirty(true);
    setSelectedId(null);
    setHighlightId(null);
    setPositions({});
    // 草稿随文档一起换，**同步清**：它的语义是「你正在写什么」，绑定的是当前这份设计。
    // 放在 loadChatForDoc 里等读取完成再清太晚 —— 那几毫秒里敲的字会被一起抹掉。
    setAiText("");
    void loadChatForDoc(d, "");
  };

  /**
   * 切到某个会话。
   *
   * **改 `doc.id` → 文档变脏**：换了个身份就该落盘，否则下次打开又认回旧会话。
   * **旧会话的内容不动** —— 切走时已经存过了。
   *
   * **刻意不清输入草稿**（只有 `newDoc` / `openFile` 清）：草稿只在**换文档**时
   * 才会"主动说错话"—— 它写的是另一批表。而会话切换不改表，草稿仍然切题；
   * `sessionId` 与 `aiHistory` 在这里已被正确刷新，所以保留草稿**不构成串台**。
   * 清掉反而会吃掉用户正在写的东西。
   */
  const switchSession = useCallback(
    async (id: string) => {
      if (!doc || id === sessionId) return;
      // 领 token 作废在飞的读取：否则 openFile 尚未返回的那次 loadChatForDoc
      // 会在切换之后才到，把用户刚切好的会话又回滚回去（设计文档第三节的要求）。
      const token = chatLoadSeqRef.current.begin();
      void saveChatNow();
      setDoc({ ...doc, id });
      setDirty(true);
      setSessionId(id);
      try {
        const chat = await invoke<{ chatMsgs?: unknown[]; aiHistory?: unknown[]; aiClarifyRound?: number }>(
          "dbd_get_chat",
          { id },
        );
        if (!chatLoadSeqRef.current.alive(token)) return;
        setChatMsgs((chat.chatMsgs ?? []).filter(isUsableMessage) as ChatMsg[]);
        setAiHistory((chat.aiHistory ?? []) as ChatTurn[]);
        setAiClarifyRound(chat.aiClarifyRound ?? 0);
      } catch {
        if (!chatLoadSeqRef.current.alive(token)) return;
        setChatMsgs([]);
        setAiHistory([]);
        setAiClarifyRound(0);
      }
      setAiClarify(null);
      setAiNewDecisions([]);
      setDocSnapshot(null);
      void touchSession(id, path);
      chatLoadSeqRef.current.settle(token);
    },
    [doc, sessionId, path, saveChatNow, touchSession],
  );

  /** 新建会话：生成新 id 并切过去（对话为空）。旧会话定格不动。 */
  const createSession = useCallback(() => {
    if (!doc) return;
    const id = createSessionId();
    // 同样要作废在飞的读取：不然它在返回后会把刚建的新会话 id 覆盖回去。
    // 它是同步完成的（不读盘），所以自己 settle —— 不 settle 就把「读盘中」卡死了。
    const token = chatLoadSeqRef.current.begin();
    void saveChatNow();
    setDoc({ ...doc, id });
    setDirty(true);
    setSessionId(id);
    setChatMsgs([]);
    setAiHistory([]);
    setAiClarifyRound(0);
    setAiClarify(null);
    setAiNewDecisions([]);
    setDocSnapshot(null);
    void touchSession(id, path);
    chatLoadSeqRef.current.settle(token);
  }, [doc, path, saveChatNow, touchSession]);

  /**
   * 删掉一个会话的缓存。
   *
   * **删的是当前会话时不报错、不补救**：文档的 `doc.id` 还在，下次
   * `resolveSessionId` 会走第 3 条重新登记它、读到空 —— 对话自然变空，文档不脏。
   * (删完顺手把列表里那条摘掉，否则弹框里还留着一条点不动的死条目。)
   */
  const deleteSession = useCallback(
    async (id: string) => {
      try {
        await invoke("dbd_delete_session", { id });
      } catch {
        return;
      }
      setSessions((prev) => prev.filter((s) => s.id !== id));
      if (id !== sessionId) return;
      const token = chatLoadSeqRef.current.begin();
      try {
        const chat = await invoke<{ chatMsgs?: unknown[]; aiHistory?: unknown[]; aiClarifyRound?: number }>(
          "dbd_get_chat",
          { id },
        );
        if (!chatLoadSeqRef.current.alive(token)) return;
        setChatMsgs((chat.chatMsgs ?? []).filter(isUsableMessage) as ChatMsg[]);
        setAiHistory((chat.aiHistory ?? []) as ChatTurn[]);
        setAiClarifyRound(chat.aiClarifyRound ?? 0);
      } catch {
        if (!chatLoadSeqRef.current.alive(token)) return;
        setChatMsgs([]);
        setAiHistory([]);
        setAiClarifyRound(0);
      }
      // 追问状态一并清：卡指向的是一条已经不存在的气泡，留着会点不动
      setAiClarify(null);
      setAiNewDecisions([]);
      setDocSnapshot(null);
      chatLoadSeqRef.current.settle(token);
    },
    [sessionId],
  );

  const openFile = async () => {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: t("dbd.openTitle"),
      filters: [{ name: "dbdesign", extensions: ["json"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const d = await invoke<DbDesignDocument>("dbd_open_file", { path: picked });
      void saveChatNow();
      setDoc(d);
      setPath(picked);
      setDirty(false);
      setSelectedId(null);
      setHighlightId(null);
      setPositions({});
      setAiText("");
      void loadChatForDoc(d, picked);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  const saveFile = async (as: boolean) => {
    if (!doc) return;
    if (!(await ensureValid())) return;
    let target = path;
    if (!target || as) {
      const picked = await saveDialog({
        title: t("dbd.saveTitle"),
        defaultPath: `${doc.name}.dbdesign.json`,
        filters: [{ name: "dbdesign", extensions: ["json"] }],
      });
      if (typeof picked !== "string") return;
      target = picked.endsWith(".dbdesign.json") ? picked : picked.replace(/\.json$/i, "") + ".dbdesign.json";
    }
    try {
      await invoke("dbd_save_file", { path: target, doc });
      setPath(target);
      setDirty(false);
      // 索引里的 path 停在旧值（首次保存前是空串）→ 列表不显示位置，
      // 而且「靠 path 认回原会话」这条规则对刚存盘的设计实际失效。
      void touchSession(sessionId, target);
      setResult({ ok: true, msg: t("dbd.saved", { path: target }) });
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  const exportSql = async (choice: ExportChoice) => {
    if (!doc) return;
    setExportBusy(true);
    try {
      // options 的键名必须是 camelCase（后端 ExportOptions 是 rename_all=camelCase）：
      // 写成 foreign_keys 会被 serde 静默忽略 → 开关看着没生效却仍导出带外键的 SQL。
      const options = { foreignKeys: choice.foreignKeys };
      if (choice.target === "clipboard") {
        const sql = await invoke<string>("dbd_export_sql", { doc, options });
        await navigator.clipboard.writeText(sql);
        toast(t("dbd.sqlCopied"), "ok");
        setExportOpen(false);
        return;
      }
      if (choice.target === "saveAs") {
        const picked = await saveDialog({
          title: t("dbd.exportSaveAsTitle"),
          defaultPath: predictSqlPath(path) || `${doc.name}.sql`,
          filters: [{ name: "sql", extensions: ["sql"] }],
        });
        if (typeof picked !== "string") return;
        const out = await invoke<string>("dbd_export_sql_to", { doc, target: picked, options });
        setResult({ ok: true, msg: t("dbd.sqlExported", { path: out }) });
        setExportOpen(false);
        return;
      }
      if (!path) {
        toast(t("dbd.exportNeedSaveFirst"), "err");
        return;
      }
      const out = await invoke<string>("dbd_export_sql_file", { doc, path, options });
      setResult({ ok: true, msg: t("dbd.sqlExported", { path: out }) });
      setExportOpen(false);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    } finally {
      setExportBusy(false);
    }
  };

  /** 打开导出弹框前先过校验闸门：不让后端那份几十行的错误清单落进内联块 */
  const openExportDialog = async () => {
    if (!doc) return;
    if (!(await ensureValid())) return;
    setExportOpen(true);
  };

  const exportOutline = async () => {
    if (!doc) return;
    if (!path) {
      const md = await invoke<string>("dbd_export_outline", { doc });
      await navigator.clipboard.writeText(md);
      toast(t("dbd.outlineCopied"), "ok");
      return;
    }
    try {
      const out = await invoke<string>("dbd_export_outline_file", { path });
      setResult({ ok: true, msg: t("dbd.outlineExported", { path: out }) });
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  // ── 反向工程 ──
  /** 用反推结果替换当前文档（有未保存内容时先问一句，别把人刚画的东西冲掉） */
  const adoptReversed = async (next: DbDesignDocument, from: string) => {
    if (doc && doc.nodes.length > 0) {
      const ok = await theamedConfirm(t("dbd.replaceConfirm", { from }), {
        title: t("dbd.replaceTitle"),
        confirmText: t("dbd.replaceBtn"),
        danger: true,
      });
      if (!ok) return;
    }
    setDoc(next);
    setPath("");
    setDirty(true);
    setSelectedId(null);
    setHighlightId(null);
    // 切换文档就清会话历史：旧文档的对话抵到新文档上会让模型拿不上位置的上下文。
    setAiHistory([]);
    setAiClarify(null);
    setAiClarifyRound(0);
    setAiNewDecisions([]);
    setDocSnapshot(null);
    setPositions({});
    setResult({ ok: true, msg: t("dbd.reversed", { count: next.nodes.length, from }) });
  };

  const reverseSqlite = async () => {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: t("dbd.pickSqlite"),
      filters: [{ name: "sqlite", extensions: ["db", "sqlite", "sqlite3", "db3"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const d = await invoke<DbDesignDocument>("dbd_reverse_sqlite", { path: picked });
      await adoptReversed(d, picked);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  const reverseDdl = async () => {
    const picked = await openDialog({
      multiple: false,
      directory: false,
      title: t("dbd.pickDdl"),
      filters: [{ name: "sql", extensions: ["sql", "dump", "txt"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const d = await invoke<DbDesignDocument>("dbd_reverse_ddl_file", {
        path: picked,
        dialect: doc?.dialect ?? "mysql",
      });
      await adoptReversed(d, picked);
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  // ── 节点 ──
  const addNode = (kind: "table" | "view") => {
    if (!doc) return;
    const node: DbDesignNode = {
      id: newId(kind === "table" ? "t" : "v"),
      kind,
      name: kind === "table" ? t("dbd.newTable") : t("dbd.newView"),
      comment: "",
      x: 80 + (doc.nodes.length % 5) * 260,
      y: 80 + Math.floor(doc.nodes.length / 5) * 220,
      table: kind === "table" ? { fields: [emptyField()], indexes: [] } : null,
      view: kind === "view" ? { sql: "SELECT 1" } : null,
    };
    patchDoc({ ...doc, nodes: [...doc.nodes, node] });
    setSelectedId(node.id);
    // 新建表沿用「选中即聚焦」的观感：既开面板也点亮，跟用户点表头的效果一致
    setHighlightId(node.id);
  };

  const updateNode = (id: string, patch: Partial<DbDesignNode>) => {
    if (!doc) return;
    patchDoc({ ...doc, nodes: doc.nodes.map((n) => (n.id === id ? { ...n, ...patch } : n)) });
  };

  const deleteNode = (id: string) => {
    if (!doc) return;
    patchDoc({
      ...doc,
      nodes: doc.nodes.filter((n) => n.id !== id),
      // 连着它的关联一起删，否则会留下悬空引用（校验会拦，但用户得自己找）
      relations: doc.relations.filter((r) => r.from.node !== id && r.to.node !== id),
    });
    if (selectedId === id) setSelectedId(null);
    // 高亮的那张被删了要跟着清，否则画布上留一个「有高亮却没有面板」的幽灵状态
    if (highlightId === id) setHighlightId(null);
    setAiHistory([]);
    setAiClarify(null);
    setAiClarifyRound(0);
  };

  /**
   * 让后端把所有「镜像父表主键」的关系同步到当前结构：父表加减主键 / 改类型，
   * 子表的外键副本与连线跟着变。规则只有后端一份（前端不复制）。
   *
   * 乐观更新：先按本地结果渲染，再拿后端 reconcile 回来的文档覆盖 —— 后端是权威，
   * 但一次 round-trip 不该让界面卡住。
   */
  const syncMirrors = async (next: DbDesignDocument) => {
    try {
      const synced = await invoke<DbDesignDocument>("dbd_sync_relations", { doc: next });
      patchDoc(synced);
    } catch (e) {
      console.error("同步镜像主键失败", e);
    }
  };

  // ── 字段 ──
  const updateField = (nodeId: string, index: number, patch: Partial<DbField>) => {
    if (!doc) return;
    const next = {
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        const fields = n.table.fields.map((f, i) => (i === index ? { ...f, ...patch } : f));
        return { ...n, table: { ...n.table, fields } };
      }),
    };
    patchDoc(next);
    // 改了主键 / 类型可能让别处的镜像外键过期（见 dbd_sync_relations）
    void syncMirrors(next);
  };

  const addField = (nodeId: string) => {
    if (!doc) return;
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        const f = emptyField("varchar");
        f.name = `col_${n.table.fields.length + 1}`;
        f.type = { base: "varchar", length: 255 };
        return { ...n, table: { ...n.table, fields: [...n.table.fields, f] } };
      }),
    });
  };

  const removeField = (nodeId: string, name: string) => {
    if (!doc) return;
    const next = {
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        return { ...n, table: { ...n.table, fields: n.table.fields.filter((f) => f.name !== name) } };
      }),
      relations: doc.relations.filter(
        (r) => !((r.from.node === nodeId && r.from.field === name) || (r.to.node === nodeId && r.to.field === name)),
      ),
    };
    patchDoc(next);
    // 删掉的可能正是父表主键 → 子表那份镜像副本也该消失，交给后端 reconcile
    void syncMirrors(next);
  };

  const updateIndex = (nodeId: string, index: number, patch: Partial<DbIndex>) => {
    if (!doc) return;
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        const indexes = (n.table.indexes ?? []).map((x, i) => (i === index ? { ...x, ...patch } : x));
        return { ...n, table: { ...n.table, indexes } };
      }),
    });
  };

  const addIndex = (nodeId: string) => {
    if (!doc) return;
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        // 新索引不预选列：索引是「哪些列的组合」由用户点选决定。
        // 名字先占位 idx_<表名>，之后每勾一列会自动跟着列名重算（见 toggleIndexField）。
        return {
          ...n,
          table: {
            ...n.table,
            indexes: [...(n.table.indexes ?? []), { name: `idx_${n.name}`, kind: "index", fields: [] }],
          },
        };
      }),
    });
  };

  /** 勾选 / 取消索引列。索引名若还是「按列自动生成的那个」，就跟着列一起重算，
   *  一旦用户自己改过名字就不再自动改（否则会把用户起的名字冲掉）。 */
  const toggleIndexField = (nodeId: string, index: number, field: string) => {
    if (!doc) return;
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        const indexes = n.table.indexes ?? [];
        const target = indexes[index];
        if (!target) return n;
        const has = target.fields.includes(field);
        const fields = has ? target.fields.filter((x) => x !== field) : [...target.fields, field];
        const autoName = (cols: string[]) => `idx_${n.name}${cols.length ? `_${cols.join("_")}` : ""}`;
        return {
          ...n,
          table: {
            ...n.table,
            indexes: indexes.map((x, i) =>
              i === index ? { ...x, fields, name: x.name === autoName(x.fields) ? autoName(fields) : x.name } : x,
            ),
          },
        };
      }),
    });
  };

  /** 删索引不动连线：连线描述的是外键关系，与索引是两回事（之前 removeField 才需要清连线） */
  const removeIndex = (nodeId: string, index: number) => {
    if (!doc) return;
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) =>
        n.id === nodeId && n.table
          ? { ...n, table: { ...n.table, indexes: (n.table.indexes ?? []).filter((_, i) => i !== index) } }
          : n,
      ),
    });
  };

  /** 改名走后端命令：它会级联更新关联与索引（前端不自己改，避免漏掉连线） */  const renameField = async (nodeId: string, oldName: string, newName: string) => {
    if (!doc || !newName.trim() || newName === oldName) return;
    try {
      const next = await invoke<DbDesignDocument>("dbd_rename_field", {
        doc,
        nodeId,
        old: oldName,
        new: newName.trim(),
      });
      patchDoc(next);
    } catch (e) {
      toast(String(e), "err");
    }
  };

  // ── 关联 ──
  /** 手动补录一条关联：以「当前选中的表」为外键侧（from）起步，
   *  另一端默认挑第一张**别的**表。原来是无视选中状态从 tables[0]/tables[1] 乱配。 */
  const addRelation = () => {
    if (!doc) return;
    const tables = doc.nodes.filter((n) => n.kind === "table");
    if (tables.length < 1 || !selected) return;
    const self = tables.find((n) => n.id === selected.id) ?? tables[0];
    const other = tables.find((n) => n.id !== self.id);
    if (!other) return;
    // 外键侧优先挑主键（语义上从表引用的就是主键），没有就第一列
    const selfField = (self.table?.fields.find((f) => f.pk) ?? self.table?.fields[0])?.name ?? "";
    const otherField = (other.table?.fields.find((f) => f.pk) ?? other.table?.fields[0])?.name ?? "";
    if (!selfField || !otherField) return;
    const rel = {
      id: newId("r"),
      name: "",
      from: { node: self.id, field: selfField },
      to: { node: other.id, field: otherField },
      kind: "1-n" as const,
      onDelete: "RESTRICT",
      onUpdate: "RESTRICT",
    };
    patchDoc({ ...doc, relations: [...doc.relations, rel] });
  };

  const updateRelation = (id: string, patch: Partial<DbDesignRelation>) => {
    if (!doc) return;
    patchDoc({ ...doc, relations: doc.relations.map((r) => (r.id === id ? { ...r, ...patch } : r)) });
  };

  /** 标签 / 配色：只改节点上的两个展示属性，不动结构 */
  const updateNodeMeta = (nodeId: string, patch: Partial<DbDesignNode>) => {
    if (!doc) return;
    patchDoc({ ...doc, nodes: doc.nodes.map((n) => (n.id === nodeId ? { ...n, ...patch } : n)) });
  };

  /** 最新文档的引用：给延迟提交用，避免闭包里那份过期（见 commitNodeColor） */
  const docRef = useRef(doc);
  docRef.current = doc;
  const colorTimer = useRef<number | null>(null);
  const pendingColor = useRef<string | null>(null);

  /**
   * 原生取色器提交颜色：合并 120ms 再落文档。
   *
   * 为什么必须合并：`input[type=color]` 在色盘上拖动时按**帧**触发 onChange，
   * 一次拖动能打出几十次。每次都 patchDoc 都会把文档标记为已修改、并触发一次
   * 后端校验（`useEffect([doc])`），手感会明显发涩。120ms 足够跟手。
   *
   * 读 `docRef` 而不是闭包里的 `doc`：定时器等待期间用户可能正在改表名，
   * 拿旧文档回写会把那次输入覆盖掉。
   */
  const commitNodeColor = (nodeId: string, hex: string) => {
    pendingColor.current = hex;
    if (colorTimer.current !== null) window.clearTimeout(colorTimer.current);
    colorTimer.current = window.setTimeout(() => {
      colorTimer.current = null;
      const next = pendingColor.current;
      pendingColor.current = null;
      const cur = docRef.current;
      if (!next || !cur) return;
      patchDoc({ ...cur, nodes: cur.nodes.map((n) => (n.id === nodeId ? { ...n, color: next } : n)) });
    }, 120);
  };

  const toggleNodeTag = (nodeId: string, tag: string) => {
    const node = doc?.nodes.find((n) => n.id === nodeId);
    if (!node) return;
    const tags = node.tags ?? [];
    updateNodeMeta(nodeId, { tags: tags.includes(tag) ? tags.filter((x) => x !== tag) : [...tags, tag] });
  };

  const removeRelation = (id: string) => {
    if (!doc) return;
    patchDoc({ ...doc, relations: doc.relations.filter((r) => r.id !== id) });
  };

  /** 开/关镜像后立刻同步一次：勾上就应马上看到子表补出来的外键列与连线 */
  const toggleRelationMirror = (id: string, mirror: boolean) => {
    const next = {
      ...doc!,
      relations: doc!.relations.map((r) => (r.id === id ? { ...r, mirror } : r)),
    };
    patchDoc(next);
    void syncMirrors(next);
  };

  // ── 画布 ──
  //
  // 拖拽方案**照抄项目里已经跑通的思维导图写法**（不要重新发明）：
  //   ① 位置只写本地 posOverrides，松手才落文档 —— 避免 onNodesChange → 重建文档 → 重建节点 的反馈循环；
  //   ② RF 实测的节点尺寸（dimensions）必须记录并挂回节点对象 —— 缺了它 RF 会在拖动中每帧重测，
  //      节点尺寸塌陷/恢复，人眼看到的就是「拖不跟手 / 闪烁」；
  //   ③ 节点对象按 id 缓存：只有位置变动的那个重建对象，其余复用旧 data 引用，memo 才能跳过 → 无闪烁。
  const [positions, setPositions] = useState<Record<string, { x: number; y: number }>>({});
  const [measuredMap, setMeasuredMap] = useState<Record<string, { width: number; height: number }>>({});
  const nodeObjCache = useRef(new Map<string, { obj: Node; px: number; py: number; doc: DbDesignDocument; node: DbDesignNode }>());

  /** 画布上出现过的所有标签（筛选条用） */
  const allTags = useMemo(() => {
    const set = new Set<string>();
    for (const n of doc?.nodes ?? []) for (const tag of n.tags ?? []) set.add(tag);
    return [...set].sort();
  }, [doc]);
  /** 标签筛选：命中的表才上画布（关系两端有一端被藏起来时，连线也一并消失） */
  const visibleNodes = useMemo(() => {
    if (!doc) return [];
    if (tagFilter.length === 0) return doc.nodes;
    return doc.nodes.filter((n) => (n.tags ?? []).some((tag) => tagFilter.includes(tag)));
  }, [doc, tagFilter]);

  /**
   * 与当前选中表直接相关的节点集合（选中项自己也算）。
   * 供节点卡片画高亮、供「只看关联」筛选用 —— 表一多时这是唯一能快速
   * 看清「这张表跟谁有关系」的办法。
   */
  const relatedIds = useMemo(
    () => (doc ? relatedTableIds(doc, highlightId, highlightDepth) : new Set<string>()),
    [doc, highlightId, highlightDepth],
  );

  /**
   * 每张表相差多少跳。连线用它分级亮度（越远越模糊）——
   * 只有 Set 的话无法区分「1 跳的线」与「3 跳的线」，会让设到 3 跳与 1 跳一样。
   */
  const relatedHops = useMemo(
    () => (doc ? relatedTableHops(doc, highlightId, highlightDepth) : new Map<string, number>()),
    [doc, highlightId, highlightDepth],
  );

  /**
   * 该亮的**连线**（与 relatedIds / relatedHops 同一次 BFS 的产物，见 series.highlightScope）。
   *
   * 判据必须是「这条线是从选中表走出去 d 步以内**走得到**的」，
   * 不能是「两端有一端在 d 跳集合内」——后者会把同一跳层内部的横向连线也点亮，
   * 产出「两端都只是远房亲戚、线却亮着」的连线，而它们的表是暗的：
   * 屏幕上就是**只亮线、不亮表**（用户实测，见 series.relatedEdgeIds 的注释）。
   */
  const relatedEdges = useMemo(
    () => (doc ? relatedEdgeIds(doc, highlightId, highlightDepth) : new Set<string>()),
    [doc, highlightId, highlightDepth],
  );

  /**
   * 表导航的行数据：按引用关系澄清父子并缩进，形成一棵树。
   * 一张被引用多少次就当作父节点；成环的表同样当根，不会死循环。
   */
  const tableTreeRows = useMemo(() => {
    // fields / indexes 一并带下来：悬浮预览要展示完整字段，而它们本来就在 doc.nodes 里，
    // 悬浮时再按 id 去 find 一遍纯属多余（列表就几十行，但没必要的二次查找就不做）。
    type Row = {
      id: string;
      name: string;
      color: string;
      depth: number;
      related: boolean;
      relCount: number;
      comment: string;
      fields: DbField[];
      indexCount: number;
    };
    if (!doc) return [] as Row[];
    // 每张表被多少张表引用（from = 引用方）
    const refCount = new Map<string, number>();
    for (const r of doc.relations) refCount.set(r.from.node, (refCount.get(r.from.node) ?? 0) + 1);
    // 选父：引用方最多的那个，这样父表的子节点才能靠近它
    const parentOf = new Map<string, string>();
    for (const r of doc.relations) {
      const cur = parentOf.get(r.from.node);
      if (cur === undefined || (refCount.get(r.from.node) ?? 0) > (refCount.get(cur) ?? 0)) {
        parentOf.set(r.from.node, r.to.node);
      }
    }
    const childIds = new Set(parentOf.keys());
    const relCountOf = (id: string) =>
      doc.relations.filter((r) => r.from.node === id || r.to.node === id).length;
    const rows: Row[] = [];
    const walk = (id: string, depth: number, seen: Set<string>) => {
      const n = doc.nodes.find((x) => x.id === id);
      if (!n || seen.has(id)) return;
      seen.add(id);
      rows.push({
        id: n.id,
        name: n.name,
        color: n.color ?? "",
        depth,
        related: highlightId !== null && relatedIds.has(n.id),
        relCount: relCountOf(n.id),
        comment: n.comment ?? "",
        fields: n.table?.fields ?? [],
        indexCount: (n.table?.indexes ?? []).length,
      });
      for (const r of doc.relations) {
        if (r.to.node !== id) continue;
        if (parentOf.get(r.from.node) !== id) continue; // 只展展正确的父子边
        walk(r.from.node, depth + 1, new Set(seen));
      }
    };
    for (const n of doc.nodes) {
      if (!childIds.has(n.id)) walk(n.id, 0, new Set());
    }
    // 环里的表可能没被任何根走到，补到底部保证每张表都能找到
    for (const n of doc.nodes) {
      if (rows.some((x) => x.id === n.id)) continue;
      rows.push({
        id: n.id, name: n.name, color: n.color ?? "", depth: 0,
        related: highlightId !== null && relatedIds.has(n.id),
        relCount: relCountOf(n.id),
        comment: n.comment ?? "",
        fields: n.table?.fields ?? [],
        indexCount: (n.table?.indexes ?? []).length,
      });
    }
    return rows;
  }, [doc, highlightId, relatedIds]);
  const rfNodes: Node[] = useMemo(() => {
    if (!doc) return [];
    const cache = nodeObjCache.current;
    const out: Node[] = [];
    /** 无关节点淡化到多少：inline opacity（Tailwind 动态类名拼不出来） */
    const dimOpacity = dimmedOpacity(lookPrefs.dim);
    for (const n of visibleNodes) {
      const p = positions[n.id] ?? { x: n.x ?? 0, y: n.y ?? 0 };
      const selected = highlightId === n.id;
      /** 有选中项时，只有它和它的关联表点亮，其余压暗 */
      const related = highlightId !== null && relatedIds.has(n.id);
      /** 压暗只在「确实选中了某张表」时才做 ——
       *  没选中是一种状态，不该被当成「这张表跟谁都没关系」。 */
      const dimmed = highlightId !== null && !selected && !related;
      // 展开态只由「节点里显示」这一档决定：选「全部字段」= 表显示全部字段、
      // 视图展开 SQL 全文。早先另有一个「常驻展开」开关与它重复（两者对表的结果
      // 完全一样，只对视图的 SQL 有差别），已合并进这个下拉。
      // 刻意不做「悬停/选中就展开」—— 移开鼠标又缩回去，读字段时来回挪很烦。
      const expanded = fieldRows === "all";
      const measured = measuredMap[n.id];
      const prev = cache.get(n.id);
      const prevObj = prev?.obj;
      const prevData = prevObj?.data as DesignNodeData | undefined;
      // 缓存键：文档引用 + 节点引用 + 坐标 + 选中/关联/展开态 + 显示模式 + 尺寸。
      // 纯拖动时这些都没变 → 复用旧对象，memo 直接跳过。
      // ⚠️ 选中/关联态必须进这个判断：选中变化时 doc 与坐标都没变，
      // 若不比较就会复用旧 data，卡片一直停在旧的高亮状态（表现为「选了表它还是灰的」）。
      if (
        prev &&
        prev.doc === doc &&
        prev.node === n &&
        prev.px === p.x &&
        prev.py === p.y &&
        prevData?.fieldRows === fieldRows &&
        prevData?.selected === selected &&
        prevData?.related === related &&
        prevData?.dimmed === dimmed &&
        prevData?.expanded === expanded &&
        prevData?.hlBoost === lookPrefs.highlight &&
        prevData?.dimOpacity === dimOpacity
      ) {
        out.push(prev.obj);
        continue;
      }
      const dataChanged =
        !prevData ||
        prev!.doc !== doc ||
        prev!.node !== n ||
        (prevData.selected !== selected) ||
        (prevData.related !== related) ||
        (prevData.dimmed !== dimmed) ||
        (prevData.expanded !== expanded) ||
        (prevData.fieldRows !== fieldRows) ||
        (prevData.hlBoost !== lookPrefs.highlight) ||
        (prevData.dimOpacity !== dimOpacity);
      const data = (dataChanged
        ? {
            node: n,
            relationFields: relationFieldsOf(doc, n.id),
            // 外键列 = 在关系里当「多」端（from）出现的字段
            fkFields: fkFieldsOf(doc, n.id),
            expanded,
            selected,
            related,
            dimmed,
            fieldRows,
            hlBoost: lookPrefs.highlight,
            dimOpacity,
          }
        : prevData) as unknown as Record<string, unknown>;
      const obj: Node = prevObj
        ? { ...prevObj, position: p, data, measured: measured ?? prevObj.measured }
        : { id: n.id, type: "designNode", position: p, data, measured };
      cache.set(n.id, { obj, px: p.x, py: p.y, doc, node: n });
      out.push(obj);
    }
    return out;
  // lookPrefs 在依赖里：滑块一动就得重算节点（否则卡片保持旧样式）
}, [doc, positions, highlightId, relatedIds, measuredMap, fieldRows, visibleNodes, lookPrefs]);

  const rfEdges: Edge[] = useMemo(() => {
    if (!doc) return [];
    /** 连线颜色 = 外键所在那张表（子表）的配色；没配色就回退模块强调色 */
    const colorOf = (id: string) => {
      const c = doc.nodes.find((n) => n.id === id)?.color ?? "";
      return /^#[0-9a-f]{6}$/i.test(c) ? c : "var(--module-accent)";
    };
    /** 关联连线的高亮规则与卡片一致：**优先用外键所在那张表自己的颜色**，
     *  没设色才回退青色。早先这里与卡片都硬用青色，于是红表被套一圈青霓虹 ——
     *  两个色相对着干。跟随表色后，节点环、表头、连线是同一个颜色。 */
    const HL_RELATED_FALLBACK = "var(--cyan, var(--vex-cyan, var(--vex-fallback-cyan)))";
    /** 表自己的颜色（用户选色/同系列配色）；未设色返回空串 */
    const ownColorOf = (id: string) => {
      const c = doc.nodes.find((n) => n.id === id)?.color ?? "";
      return /^#[0-9a-f]{6}$/i.test(c) ? c : "";
    };
    // 只画「两端都在画布上」的关系：被标签筛掉的表，它的连线也该消失
    const onCanvas = new Set(visibleNodes.map((n) => n.id));
    return doc.relations.filter((r) => onCanvas.has(r.from.node) && onCanvas.has(r.to.node)).map((r) => {
      // **判据必须用 relatedEdges（BFS 实际走过的边），不能写死「与选中表直连」**（那等于 1 跳），
      // 也不能退回「两端有一端在集合内」—— 那会点亮同一跳层内部的横向连线，
      // 造成「线亮着、两端表却暗着」（用户实测的「只亮线、不亮表」）。
      // 层级语义的完整定义见 series.highlightScope。
      //
      // 分级：跳数越远线越模糊 —— 否则设到 3 跳与 1 跳画出来一样。
      const related = relatedEdges.has(r.id);
      const hopA = highlightId === null ? undefined : relatedHops.get(r.from.node);
      const hopB = highlightId === null ? undefined : relatedHops.get(r.to.node);
      const nearHop = hopA === undefined ? hopB : hopB === undefined ? hopA : Math.min(hopA, hopB);
      const dimmed = highlightId !== null ? !related : false;
      const hopFade = related ? (nearHop ? Math.max(0.25, 1 - (nearHop - 1) * 0.25) : 1) : 1;
      const own = ownColorOf(r.from.node);
      // 弱化档（选中了别的表）：**要退后，但不能退到看不见**。
      // 早先这里是 rgba(148,163,184,0.18) + 1px，在深色画布上等于隐形 ——
      // 而相邻两张卡常常只隔几十像素（用户截图：product_category 与 product 之间
      // 一条线都看不见，误以为关联丢了）。现在默认 0.4 / 1.3px，由「淡化强度」滑块调节；
      // 拉到 0 则完全不弱化（用节点自己的颜色画，与未选中时同等待遇）。
      const dimOff = lookPrefs.dim <= 0.001;
      const stroke = related
        ? own || HL_RELATED_FALLBACK
        : dimmed && !dimOff
          ? dimmedEdgeStroke(lookPrefs.dim)
          : colorOf(r.from.node);
      const { sourceHandle, targetHandle } = edgeAnchors(doc, r);
      return {
        id: r.id,
        // 画成「父 → 子」：箭头指向多端（外键所在表），与 PowerDesigner 的观感一致
        source: r.to.node,
        target: r.from.node,
        // 落在两端**对应的字段**上（复合主键时 A 的 3 个主键各有各的线，各自指向 B 里
        // 自己那一列）；字段找不到时回退表级锚点，保证线还在（见 edgeAnchors 注释）
        sourceHandle,
        targetHandle,
        type: edgeStyle,
        // 多对多常驻流动 —— **仅在没有选中任何表时**。静态图里 m:n 确实只能靠流动认出来；
        // 但一旦选中了某张表，常驻流动就变成了「与高亮无关却最抢眼的线」：
        // 它既不在高亮范围里、又比被压暗的线活跃，看上去就像「线亮了表没亮」
        // （用户实测）。所以有选中项时 n-n 老老实实按高亮/压暗走。
        // 选中表的关联线仍然流动 —— 这正是要突出它的时候。
        animated: (highlightId === null && r.kind === "n-n") || related,
        // 流动节奏统一走 graph-edge-flow：内置 `.animated` 是 dasharray:5 / 0.5s，
        // 太快，看着像虚线在抖而不是「有东西在流动」
        className: EDGE_FLOW_CLASS,
        // 不画连线文字：线的两端就落在具体字段上，谁引用谁一眼看得出来；
        // 挂一串「A.b → C.d」反而盖住线，密集时糊成一片。
        style: {
          stroke,
          // 关联线更粗 + 泛一层同色辉光（辉光色 = 线色 = 该表自己的颜色），
          // 节点环、表头、连线是同一套语言。粗细/辉光随「高亮强度」缩放。
          strokeWidth: related
            ? relatedEdgeWidth(lookPrefs.highlight) * hopFade
            : dimmed && !dimOff
              ? dimmedEdgeWidth(lookPrefs.dim)
              : 1.5,
          ...(related ? { filter: edgeGlow(stroke, lookPrefs.highlight * hopFade) } : null),
        },
        zIndex: related ? 10 : 0,
        // 箭头**不受 strokeWidth 影响**：线宽归零到看不见了，箭头若照画，
        // 满屏仍是实心表色的三角 —— 那正是「只亮线、表不亮」的残留半边。
        // 所以箭头与线宽走同一个强度判据（见 graphHighlight.hasVisibleHighlight）。
        markerEnd: related && !hasVisibleHighlight(lookPrefs.highlight)
          ? undefined
          : { type: MarkerType.ArrowClosed, color: stroke },
      };
    });
  // lookPrefs 也要在依赖里：滑块一动，连线的粗细/颜色/辉光要立刻跟着变。
  // 依赖里必须含 relatedEdges 与 relatedHops：判据全靠它们，漏一个都会让
  // 拖层级滑块时表卡变了、连线核心数组一点不重算 —— 「表亮线不活」。
  }, [doc, edgeStyle, visibleNodes, highlightId, relatedEdges, relatedHops, lookPrefs]);
  const onNodesChange = useCallback((changes: NodeChange[]) => {
    // 位置：一次性写本地覆盖（思维导图同款，避免每帧重建文档）
    setPositions((prev) => {
      let next: Record<string, { x: number; y: number }> | null = null;
      for (const ch of changes) {
        if (ch.type === "position" && ch.position) {
          next = next ?? { ...prev };
          next[ch.id] = { x: ch.position.x, y: ch.position.y };
        }
      }
      return next ?? prev;
    });
    // 尺寸：必须记录（少了它 RF 每帧重测 → 拖动卡顿 / 闪烁）
    for (const ch of changes) {
      if (ch.type === "dimensions" && ch.id && ch.dimensions) {
        const d = ch.dimensions;
        setMeasuredMap((prev) =>
          prev[ch.id!]?.width === d.width && prev[ch.id!]?.height === d.height
            ? prev
            : { ...prev, [ch.id!]: { width: d.width, height: d.height } },
        );
      }
    }
  }, []);

  const onNodeDragStop = useCallback(
    (_e: unknown, node: Node) => {
      if (!doc) return;
      const x = Math.round(node.position.x);
      const y = Math.round(node.position.y);
      patchDoc({ ...doc, nodes: doc.nodes.map((n) => (n.id === node.id ? { ...n, x, y } : n)) });
    },
    [doc, patchDoc],
  );

  /**
   * 拖线建关联（PowerDesigner 式）：
   *   · 从「整表」锚点拖出 → 复制父表**全部主键**字段到子表，逐个建外键；
   *   · 从某个字段锚点拖出 → 用该字段建关联；源字段是主键且目标表没有同名字段时，自动复制过去。
   * 方向约定：`from` = 子表（外键侧），`to` = 父表（被引用侧）。
   */
  const onConnect = useCallback(
    (conn: Connection) => {
      if (!doc) return;
      /**
       * 方向归一：两侧锚点**都能拖、都能接**（用户要求「黄→蓝」和「蓝→黄」结果一样），
       * 所以不能再靠「source 一定是父表」推断 —— 改看拖线是从哪一侧开始的：
       *   从右侧（黄，`f:` 前缀）拖出 → 起点是**父表**（被引用的那端）
       *   从左侧（蓝，`b:` 前缀）拖出 → 起点是**子表**（外键所在的那端）
       * 两种写法最终落到同一组 (parent, child)。
       */
      const handle = conn.sourceHandle ?? "";
      const fromLeftSide = handle.startsWith("b:");
      const parentId = fromLeftSide ? conn.target : conn.source;
      const childId = fromLeftSide ? conn.source : conn.target;
      if (!parentId || !childId || parentId === childId) return;
      const parent = doc.nodes.find((n) => n.id === parentId);
      const child = doc.nodes.find((n) => n.id === childId);
      if (!parent?.table || !child?.table || parent.kind !== "table" || child.kind !== "table") {
        toast(t("dbd.connectNeedTables"), "err");
        return;
      }
      // 整表锚点 → 全部主键；字段锚点 → 该字段
      const fromTableAnchor = handle === handleAllPk || handle === HANDLE_TABLE_IN || handle === HANDLE_TABLE;
      const wanted: string[] = fromTableAnchor
        ? parent.table.fields.filter((f) => f.pk).map((f) => f.name)
        : handle.startsWith("f:")
          ? [handle.slice(2)]
          : handle.startsWith("b:")
            ? [handle.slice(2)]
            : [];
      // 是否算「整表引用」（镜像）：金色锚点明确是；字段锚点则看父表是不是只有这一个主键 ——
      // 单主键表拖它跟拖金色锚点是一回事，不该让用户去分辨两个几乎看不见的锚点。
      const parentPkCount = parent.table.fields.filter((f) => f.pk).length;
      const mirror = fromTableAnchor || (wanted.length === 1 && parentPkCount === 1);
      if (wanted.length === 0) {
        toast(t("dbd.connectNoPk"), "err");
        return;
      }

      let childTable: DbTableBody = child.table;
      let relations = doc.relations;
      const created: string[] = [];
      const skipped: string[] = [];
      for (const pf of wanted) {
        const src = parent.table.fields.find((f) => f.name === pf);
        if (!src) continue;
        let fieldName = src.name;
        let copied = false;
        if (!childTable.fields.some((f) => f.name === fieldName)) {
          // 目标表没有同名字段 → 按 PowerDesigner 的做法把主键复制过去当外键
          fieldName = src.name;
          copied = true;
          childTable = {
            ...childTable,
            fields: [
              ...childTable.fields,
              { ...src, pk: false, autoIncrement: false, nullable: false, comment: src.comment },
            ],
          };
        }
        const dup = relations.some(
          (r) => r.from.node === childId && r.from.field === fieldName && r.to.node === parentId && r.to.field === src.name,
        );
        if (dup) {
          skipped.push(fieldName);
          continue;
        }
        relations = [
          ...relations,
          {
            id: newId("r"),
            name: "",
            from: { node: childId, field: fieldName },
            to: { node: parentId, field: src.name },
            kind: "1-n",
            onDelete: "RESTRICT",
            onUpdate: "RESTRICT",
            // 镜像 = "B 跟着 A 的主键走"：以后 A 改主键，B 自动同步（后端 reconcile）。
            // 复合主键只拖了其中一列时不标记 —— 用户挑的那部分，不该被自动推广。
            mirror,
          },
        ];
        created.push(copied ? `${fieldName}（已复制）` : fieldName);
      }

      if (created.length === 0 && skipped.length === 0) return;
      patchDoc({
        ...doc,
        nodes: doc.nodes.map((n) => (n.id === childId ? { ...n, table: childTable } : n)),
        relations,
      });
      toast(
        t("dbd.connected", {
          count: created.length,
          table: child.name,
          skipped: skipped.length > 0 ? t("dbd.connectSkipped", { fields: skipped.join(", ") }) : "",
        }),
        "ok",
      );
    },
    [doc, patchDoc, t],
  );

  /**
   * AI 生成表结构：后端直接返回一份设计文档，**不落盘** —— 载入画布后由用户自己决定保存。
   * 生成结果没有坐标（AI 不给），所以载入后立刻自动布局 + 适应视野，
   * 否则 100 张表全叠在左上角。
   *
   * **边生成边落图**：后端流式回传部分文档（`dbd-ai-partial`），这里收到就上画布，
   * 不等整篇到齐。位置**只给没排过的新节点**（已排过的一律不动），
   * 否则每来一张表全体重排一次会满屏乱跳；最终那次仍走完整自动布局。
   */
  /**
   * `overrideText`：追问回答时直接发起下一轮。
   *
   * 不把它写进 aiText 再读：那样需要 setState 后等一个循环才能发，
   * 而且用户会看到输入框先清空再出字——两次变化。
   */
  const runAi = async (overrideText?: string) => {
    const ask = (overrideText ?? aiText).trim();
    // 「读盘中」也要挡：换文档 / 切会话的那一瞬，`sessionId` 与 `aiHistory`
    // 还是上一份的，此时**任何**入口发出去都是「新表 + 旧对话」。
    //
    // 闸门放在这里而不是逐个给按钮加 `disabled` —— 发起一轮的入口有三个
    // （发送按钮、追问卡的选项、追问卡的自由输入），UI 上漏一个就绕过了。
    // 只禁发送按钮是不够的：追问卡那会儿还亮着（旧会话的气泡还没被替换）。
    if (!doc || !ask || chatLoading) return;
    setAiBusy(true);
    setAiAborted(false);
    setAiErr("");
    // 开一轮：用户气泡 + 助手占位气泡（进度先去那里【）
    setChatMsgs((m) => startTurn(m, ask));
    // 追问不落图：流式期间后端会阻止推部分文档，但一旦模型先写了几张表才转问，
    // 画布上就会留下半套。快照用来把它撤回去。
    setDocSnapshot(doc);
    dbdAiProgressBuffer.clear();
    dbdAiPartialBuffer.clear();
    // 运行开始的基线快照：画布有表时 AI 基于它修改（后端提示词带它作基线）。
    // 1) 现有表坐标预填进 aiPlacedRef —— 流式落图时它们原地不动；
    // 2) 流式期间「AI 还没重新吐出的表」保留在画布（见 applyPartial 合并）；
    // 3) 收尾时现有表保持手动摆过的位置，只有新表走自动布局。
    const baseDoc = doc.nodes.length > 0 ? doc : null;
    const posSeed: Record<string, { x: number; y: number }> = {};
    for (const n of baseDoc?.nodes ?? []) posSeed[n.id] = { x: n.x ?? 0, y: n.y ?? 0 };
    aiPlacedRef.current = posSeed;
    const runId = crypto.randomUUID();
    aiRunIdRef.current = runId;

    const applyPartial = (partial: DbDesignDocument) => {
      const placed = aiPlacedRef.current;
      let slot = Object.keys(placed).length;
      const laid = partial.nodes.map((n) => {
        const known = placed[n.id];
        if (known) return { ...n, x: known.x, y: known.y };
        const p = {
          x: 40 + (slot % AI_STREAM_COLS) * 320,
          y: 40 + Math.floor(slot / AI_STREAM_COLS) * 300,
        };
        slot += 1;
        placed[n.id] = p;
        return { ...n, ...p };
      });
      if (!baseDoc) {
        // commonFields 要原样带回：partial 是 AI 吐的新文档（只管表结构），
        // 直接采纳会把用户配好的公共字段抹掉 —— 而这是新建场景（画布还没表）唯一的分支。
        patchDoc({ ...partial, commonFields: doc.commonFields, nodes: laid });
        setPositions({});
        return;
      }
      // 修改场景：AI 要重吐全部表，未吐到的先留在画布（按 id、再按名字匹配——
      // 提示词里带了原 id，正常会被原样抄回；漏抄也能靠名字接上），吐到了就顶替。
      const emittedId = new Set(partial.nodes.map((n) => n.id));
      const emittedName = new Set(partial.nodes.map((n) => n.name.trim().toLowerCase()));
      const keptNodes = baseDoc.nodes.filter(
        (n) => !emittedId.has(n.id) && !emittedName.has(n.name.trim().toLowerCase()),
      );
      const emittedRelId = new Set(partial.relations.map((r) => r.id));
      const keptRels = baseDoc.relations.filter((r) => !emittedRelId.has(r.id));
      patchDoc({
        ...baseDoc,
        name: partial.name || baseDoc.name,
        nodes: [...laid, ...keptNodes],
        relations: [...partial.relations, ...keptRels],
      });
      setPositions({});
    };

    // 订阅缓冲（模块级单例，面板切走再回来也不会漏）；退订放在 finally
    const unsubPartial = dbdAiPartialBuffer.subscribe(() => {
      const items = dbdAiPartialBuffer.snapshot();
      const last = items[items.length - 1];
      // runId 对不上 = 上一轮残留（用户连点两次生成），丢弃
      if (!last?.doc || last.runId !== runId) return;
      const firstBatch = Object.keys(aiPlacedRef.current).length === 0;
      applyPartial(last.doc);
      // 只在第一批落图时适应一次视野，之后让画面自己长，不追着镜头跑
      if (firstBatch) setFitSignal((n) => n + 1);
    });

    try {
      const outcome = await invoke<AiOutcome>("dbd_ai_generate", {
        input: {
          text: ask,
          // 会话历史（滚动窗口）与追问轮次：这两个让模型知道「刚问过什么」「你不能再问了」
          history: aiHistory,
          clarifyRound: aiClarifyRound,
          name: doc.name || undefined,
          dialect: doc.dialect,
          providerId: aiProviderId || null,
          modelId: aiModelId || null,
          runId,
          // 画布有表就把当前设计带上：AI 基于它修改（输出完整更新后的设计）；
          // 是「新建」还是「修改」由模型看需求判断（系统提示里写明了）。
          //
          // **无条件传整个 doc**（不再按「有没有表」筛）：公共字段配置是**文档级设置**，
          // 哪怕画布上一张表都没有，它也必须跟着走 —— 否则「刚配好公共字段就点 AI 生成」
          // 这条最自然的路径，提示词里恰好没有那句声明，模型于是自己建一遍 created_at，
          // 而导出时又因重名被跳过（表自己的赢），用户完全看不出问题出在哪。
          // 「提示词要不要展示当前设计」由后端按「有没有表」判断
          // （build_user_prompt 里过滤空节点文档），所以这里放开不会让空文档进提示词。
          currentDoc: doc,
        },
      });
      // 追问分支：文档一个字都不动。流式期间后端已阻止推部分文档，
      // 但为了不让「半套设计」留在画布上，这里把本轮开始前的文档写回去。
      if (outcome.kind === "clarify") {
        if (docSnapshot) patchDoc(docSnapshot);
        setAiClarify({ question: outcome.question, options: outcome.options });
        setChatMsgs((m) => patchLast(m, { status: "clarify", clarify: { question: outcome.question, options: outcome.options } }));
        setAiHistory((h) =>
          trimHistory([...h, { role: "assistant", text: `【追问】${outcome.question}` }]),
        );
        // 追问轮次 +1：后端据此在提示词里写明「你已经问过 N 轮，不要再问」。
        // 它守的是「一直追问下去」这个最坏情况 —— 用户不答就一直问。
        setAiClarifyRound((n) => n + 1);
        setAiText("");
        return;
      }
      const generated = outcome.document;
      // **决策记录的所有权在前端**：模型每轮都要回吐整份文档，
      // 很可能把你之前写的决策丢了或改写。所以保留原文档的这份，
      // 只把本轮新增的叠上去（同文不重复追加）。
      const mergedDecisions = mergeDecisions(doc.decisions, outcome.decisions);
      // 新表的位置跟随用户当前的布局偏好：否则界面选「核心左右」、AI 生成的新表却摆成放射，
      // 同一幅画里两种形态混着，比一开始就用放射还怪。
      const pos = computeLayout(generated, { fieldRows }, layoutMode);
      const placed: DbDesignDocument = {
        ...generated,
        // **公共字段配置必须从原文档带过来**：AI 返回的是一份全新文档（它只管表结构），
        // 直接采纳的话，用户配好的公共字段会被一次生成抹掉 —— 而这配置只存在于设计文件里，
        // 抹掉就等于白配（还得重新勾一遍）。
        commonFields: doc.commonFields,
        decisions: mergedDecisions,
        nodes: generated.nodes.map((n) => {
          // 现有表（按 id、再按名字匹配）保持手动摆过的位置，新表才用自动布局
          const old =
            baseDoc?.nodes.find((o) => o.id === n.id) ??
            baseDoc?.nodes.find((o) => o.name.trim().toLowerCase() === n.name.trim().toLowerCase());
          if (old) return { ...n, x: old.x ?? 0, y: old.y ?? 0 };
          const p = pos[n.id];
          return p ? { ...n, x: Math.round(p.x), y: Math.round(p.y) } : n;
        }),
      };
      patchDoc(placed);
      setPositions({});
      setSelectedId(null);
      setHighlightId(null);
      setFitSignal((n) => n + 1);
      // 成功之后才记历史。assistant 那一轮**只存决策摘要**，不存整份 JSON ——
      // 下一轮模型看的是更新后的文档，旧 JSON 里的表定义还是那些，纯烧 token。
      // 只留最近 3 轮（滚动窗口）。
      setAiNewDecisions(outcome.decisions);
      // 本轮收尾的三个真值**先算出来**，state 与落盘共用同一份。
      //
      // 不能写完 state 再从 `chatMsgsRef` / `aiHistory` / `aiClarifyRound` 里读：
      // React 自动批处理意味着那一刻还没 re-render，读到的全是**上一帧**——
      // 于是每轮成功生成后落盘的末条气泡都卡在 status:"writing"，
      // 重启看着像还在写，且决策摘要与本轮历史全丢。
      const donePatch = {
        status: "done" as const,
        summary: t("dbd.aiDone", { count: placed.nodes.length }),
        thinking: aiThinking?.text,
        chars: aiStream?.length,
        decisions: outcome.decisions,
        patch: outcome.kind === "patch" ? patchCounts(outcome.applied) : undefined,
        truncated: outcome.kind === "design" && outcome.truncated,
      };
      const roundSummary =
        outcome.decisions.length > 0
          ? `已按需求修改设计（${outcome.decisions.length} 项决策）`
          : "已按需求修改设计";
      const finalMsgs = patchLast(chatMsgsRef.current, donePatch);
      const finalHistory = trimHistory([
        ...aiHistory,
        { role: "user" as const, text: ask },
        { role: "assistant" as const, text: roundSummary },
      ]);
      setChatMsgs(finalMsgs);
      setAiHistory(finalHistory);
      setAiClarify(null);
      setAiClarifyRound(0);
      // 侧栏保持展开（你还要接着聊），但输入框清空：它是「你正在写什么」，
      // 留着上一句会让人以为还没发出去。历史里有上一轮，问「接着改」也够用了。
      // 自校验结果要说清楚：用户必须知道这份设计**现在能不能用**。
      // 这正是 Q-0410 那个死锁的教训 —— 「满屏报错且导不出来」而界面一声不吭。
      if (outcome.kind === "patch") {
        const a = outcome.applied;
        const parts: string[] = [];
        if (a.addedTables.length) parts.push(t("dbd.aiPatchAdded", { count: a.addedTables.length }));
        if (a.updatedTables.length) parts.push(t("dbd.aiPatchUpdated", { count: a.updatedTables.length }));
        if (a.removedTables.length) parts.push(t("dbd.aiPatchRemoved", { count: a.removedTables.length }));
        toast(parts.length ? parts.join("\uff0c") : t("dbd.aiPatchNoop"), "ok");
        if (a.droppedRelations.length) {
          toast(t("dbd.aiPatchDropped", { count: a.droppedRelations.length }), "err");
        }
      }
      // 产出被截断：这里必须强调“不完整”——用户看到了表，
      // 但不知道关联与决策记录被截尾了。不说他会以为完整。
      if (outcome.kind === "design" && outcome.truncated) {
        toast(t("dbd.aiTruncated", { count: placed.nodes.length }), "err");
      }
      if (outcome.repair.ok) {
        if (outcome.repair.rounds > 0) {
          toast(t("dbd.aiRepaired", { rounds: outcome.repair.rounds }), "ok");
        }
      } else {
        // 没修好也照样落图（否则这轮钱白花、你也看不到设计），但明确告知剩余问题
        toast(
          t("dbd.aiRepairFailed", { count: outcome.repair.remainingErrors.length }),
          "err",
        );
      }
      toast(t("dbd.aiDone", { count: placed.nodes.length }), "ok");
      // 会话落盘**只在这一处**，刻意不放在 finally：
      // 前序设计定了「生成失败 / 你点停止 → 没有历史，这一轮没成功就不该留痕」，
      // 放 finally 会把失败那轮也写进去。
      //
      // 写的是上面算出的 finalMsgs / finalHistory（与 state 同一份真值），
      // 不是 chatMsgsRef —— 批处理下 ref 还是上一帧。
      void commitChat({
        msgs: finalMsgs,
        history: finalHistory,
        clarifyRound: 0,
      });
    } catch (e) {
      const msg = String(e);
      // 主动停止不算失败：只闪一句提示，不要把「已取消」当错误堆在界面上
      if (msg.includes("已取消")) {
        setAiAborted(true);
        setChatMsgs((m) => patchLast(m, { status: "done", error: t("dbd.aiCancelled") }));
        toast(t("dbd.aiCancelled"), "ok");
      } else {
        setAiErr(msg);
        setChatMsgs((m) => patchLast(m, { status: "error", error: msg, thinking: aiThinking?.text }));
      }
    } finally {
      unsubPartial();
      aiRunIdRef.current = "";
      setAiBusy(false);
    }
  };

  /** 停止生成：后端靠 run_id 在全局集合里对上号（前端不等待结果，界面立刻恢复） */
  const cancelAi = async () => {
    const runId = aiRunIdRef.current;
    if (!runId) return;
    try {
      await invoke("dbd_ai_cancel", { runId });
    } catch (e) {
      console.error("停止 AI 失败", e);
    }
  };

;

  /** 重新布局：算完直接写回 node.x/y，并把拖动覆盖清掉（否则旧坐标会盖住新布局）。
   *  没有撤销栈，所以先问一句 —— 手工摆好的位置会被覆盖。 */
  const autoLayout = async () => {
    if (!doc) return;
    if (doc.nodes.length === 0) {
      toast(t("dbd.layoutEmpty"), "err");
      return;
    }
    const ok = await theamedConfirm(t("dbd.layoutConfirm"), {
      title: t("dbd.layoutTitle"),
      confirmText: t("dbd.layoutBtn"),
    });
    if (!ok) return;
    const pos = computeLayout(doc, { fieldRows }, layoutMode);
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        const p = pos[n.id];
        return p ? { ...n, x: Math.round(p.x), y: Math.round(p.y) } : n;
      }),
    });
    setPositions({});
    setFitSignal((n) => n + 1);
    toast(t("dbd.layoutDone", { count: Object.keys(pos).length }), "ok");
  };

  /**
   * 整理**高亮中的那几张表**（= 选中表 + 与它直接有外键关系的表），其余表完全不动。
   *
   * 这里曾经整理的是「选中表所在的整个外键连通分量」，而按钮文案写的是
   * 「只把高亮中的表…其余不动」。多数业务设计里所有表外键连通，
   * 于是「整理选中」实际等于「整理全部」—— 提示条如实报「已整理 22 张表」，
   * 而位置因为早已按全量排过而看不出变化，看着像按钮坏了。
   *
   * 现在整理集合 = 高亮集合 = `relatedTableIds` 的返回值，**同一份定义**。
   * 之前是三份各写各的（高亮一份、整理一份、标签一份），才会出现三者不一致。
   *
   * 落点是**子文档**：把这几张表与它们之间的关联抽成一份小设计，交给同一个
   * `computeLayout`（`x = PAD + 深度 * (卡宽 + 间距)`，天然父左子右）。
   * 复用它而不是另写一套，是为了两份布局口径永不漂移。父表不在集合内的节点
   * 会成为这个子布局的根 —— 这正是「忽略其它节点」的预期。
   *
   * **不弹确认**：它只改坐标，不改任何数据（字段、关联、索引一概不动），
   * 且撤销 / 再点一次「自动布局」就能回来。为一个可逆的重排拦一道确认，
   * 只会让「点一下看看排得怎么样」变成一件有心理负担的事。
   */
  const arrangeHighlighted = () => {
    if (!doc || !highlightId) return;
    const ids = relatedTableIds(doc, highlightId, highlightDepth);
    // 只有一张表时排了等于没排，直接说清楚而不是默默挪一下
    if (ids.size < 2) {
      toast(t("dbd.arrangeNeedMore"), "err");
      return;
    }
    const sub = buildArrangeSubDoc(doc, ids);
    const pos = computeLayout(sub, { fieldRows }, layoutMode);
    if (Object.keys(pos).length === 0) return;
    // 排完整体平移到原高亮区域的左上角：否则这批表会被搬到画布原点，
    // 视觉上像「图表跳了一下」，用户的空间记忆就断了
    const before = sub.nodes.map((n) => ({ x: n.x ?? 0, y: n.y ?? 0 }));
    const anchorX = Math.min(...before.map((p) => p.x));
    const anchorY = Math.min(...before.map((p) => p.y));
    const after = Object.values(pos);
    const offX = anchorX - Math.min(...after.map((p) => p.x));
    const offY = anchorY - Math.min(...after.map((p) => p.y));
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        const p = pos[n.id];
        return p ? { ...n, x: Math.round(p.x + offX), y: Math.round(p.y + offY) } : n;
      }),
    });
    setPositions({});
    // 整理后必须定位：新排布的边界比原来的大多（放射布局可能拉了上万像素），
    // 而用户的视野还停在原位置，看不到被整理的那几张。
    setArrangeFitIds(Object.keys(pos));
    setFitFocusSignal((n) => n + 1);
    toast(t("dbd.arrangeDone", { count: Object.keys(pos).length }), "ok");
  };

  const selected = doc?.nodes.find((n) => n.id === selectedId) ?? null;
  /** dock 只管「当前这张表」：字段、索引都是表级的，关联也必须一致 ——
   *  否则选中 orders 却要在一堆无关表里翻它那几条关系。 */
  const tableRelations = selected
    ? (doc?.relations ?? []).filter((r) => r.from.node === selected.id || r.to.node === selected.id)
    : [];
  /** 再按「我这一侧」筛：作为主表（别人引用我）/ 作为子表（我引用别人）。 */
  const shownRelations =
    relFilter === "all"
      ? tableRelations
      : relFilter === "parent"
        ? tableRelations.filter((r) => r.to.node === selected?.id)
        : tableRelations.filter((r) => r.from.node === selected?.id);
  const REL_FILTERS = [
    ["all", t("dbd.relFilterAll")],
    ["parent", t("dbd.relFilterParent")],
    ["child", t("dbd.relFilterChild")],
  ] as const;

  /**
   * 卡片点击分流（用户明确要求的两条）：
   *
   * - **点表头 → 只高亮**（连同它的一跳关联一起点亮、其余淡化），**不开底部面板**。
   * - **点字段区域 → 打开底部编辑面板**（显示这张表的全部字段页签），**不高亮**。
   *
   * 靠卡片上的 `data-dbd-role` 区分：`table-header` / `field-row` / `field-area`
   * （见 DesignNodeCard）。为什么不把回调塞进 node data：data 会进 `rfNodes` 的依赖数组，
   * 每次渲染新建函数就会让整张图重算，而这里只需要一个字符串判断。
   *
   * 点字段**不改高亮**：既不清掉已有的高亮，也不把被点的表点亮 —— 用户要的是
   * 「读字段时不惊动画布」。
   */
  const onCardClick = useCallback(
    (e: React.MouseEvent, n: { id: string }) => {
      const act = cardClickAction(clickRoleOf(e.target));
      if (act.kind === "openDock") {
        setSelectedId(n.id); // 面板目标；dock 由 selectedAlive 的既有 effect 展开
        setDockTab("fields"); // 从「索引/关联」页签点字段时切回字段，避免看不到点的是啥
        return;
      }
      setHighlightId(n.id);
      // 点表头高亮**别的**表时，把还停在上一张表上的面板收掉。
      //
      // 为什么必须收：面板标题写着 A 的名字，画布却高亮着 B —— 用户看到的是
      // 「我明明在点 B，为什么下面编辑的还能是 A」，改字段时最容易改错对象。
      // 点的是**同一张**表则保留面板（用户就是想在高亮它的同时继续编辑）。
      setSelectedId((cur) => (cur === n.id ? cur : null));
    },
    [],
  );

  return (
    <div
      ref={rootRef}
      // 拖分隔线时禁掉文本选中，否则拖过表格会顺带把字段名全刷成选中态
      style={dockDragging ? { userSelect: "none", cursor: "row-resize" } : undefined}
      className="relative flex h-full min-h-0 flex-col text-slate-200"
    >
      {/* 工具栏。relative：AI 面板是挂在工具栏上的浮层，
          少了它浮层会以整个模块为基准（top-full = 模块底部）→ 渲染在可视区外，看着像"没反应"。

          **为什么大半按钮收进了下拉**：平铺 14 个按钮会占满两行、把画布挤到只剩一条，
          而且每加一个动作就再挤一格。收成「同一条主线上的动作进一个菜单」后，
          顶栏固定 8 个入口（收藏模块早就用`shared/Menu` 解决了同一个问题）。
          文件类动作（打开/新建/保存/另存为）已全部收进「文件」菜单。*/}
      <div className="relative flex flex-wrap items-center gap-1.5 border-b border-white/5 bg-white/[0.02] px-3 py-2">
        {/* 文件类动作全部收进这一个菜单（打开 / 新建 / 保存 / 另存为）。
            早先「打开」「保存」是直连按钮、另两个在菜单里 —— 同一类动作分裂在两处，
            顶栏还多占两格；用户明确要求合并。 */}
        <Menu
          label={t("dbd.menuFile")}
          className="ui-btn px-2 py-1 text-caption"
          items={[
            // 生成中禁掉打开/新建：`runAi` 的流式落图与收尾 patchDoc 都基于**它闭包里
            // 那份 doc**，此时换文档会把新画布整体覆盖成旧文档的内容并置脏 ——
            // 后果比「对话串台」更重（设计文件被覆盖，不只是聊天记录）。
            { key: "open", label: t("dbd.open"), disabled: aiBusy, onSelect: () => void openFile() },
            { key: "sep1", type: "separator" as const },
            { key: "new", label: t("dbd.new"), disabled: aiBusy, onSelect: () => void newDoc() },
            { key: "save", label: t("dbd.save"), disabled: !doc, onSelect: () => void saveFile(false) },
            { key: "saveAs", label: t("dbd.saveAs"), disabled: !doc, onSelect: () => void saveFile(true) },
          ]}
        />
        <div className="mx-1 h-4 w-px bg-white/10" />
        {/* 导出：SQL（带选项弹框）/ 结构说明 */}
        <Menu
          label={t("dbd.menuExport")}
          className="ui-btn px-2 py-1 text-caption"
          disabled={!doc}
          items={[
            { key: "sql", label: <><FileDown className="h-3.5 w-3.5" />{t("dbd.exportSql")}</>, onSelect: () => void openExportDialog() },
            { key: "outline", label: <><Sparkles className="h-3.5 w-3.5" />{t("dbd.exportOutline")}</>, onSelect: () => void exportOutline() },
          ]}
        />
        <div className="mx-1 h-4 w-px bg-white/10" />
        {/* 添加：表 / 视图 */}
        <Menu
          label={t("dbd.menuAdd")}
          className="ui-btn px-2 py-1 text-caption"
          disabled={!doc}
          items={[
            { key: "table", label: <><Database className="h-3.5 w-3.5" />{t("dbd.addTable")}</>, onSelect: () => addNode("table") },
            { key: "view", label: <><Eye className="h-3.5 w-3.5" />{t("dbd.addView")}</>, onSelect: () => addNode("view") },
          ]}
        />
        {/* 公共字段：与 AI 助手并列的独立入口（配置类动作，不塞进「添加」菜单）。
            徽章直接显示已勾选几个 —— 那是「导出时会给每张表多出几列」的量化后果，
            配完不必打开弹框就能确认配置生效了。 */}
        <button
          onClick={() => setCommonFieldsOpen(true)}
          disabled={!doc}
          className="ui-btn relative px-2 py-1 text-caption disabled:opacity-40"
          title={
            doc?.commonFields?.length
              ? doc.commonFields.map((f) => f.name).join("、")
              : t("dbd.commonFieldsNone")
          }
        >
          <SlidersHorizontal className="h-3.5 w-3.5" /> {t("dbd.commonFieldsMenu")}
          {doc?.commonFields?.length ? (
            <span className="ml-1 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-[var(--module-accent)] px-1 text-micro font-semibold text-white">
              {doc.commonFields.length}
            </span>
          ) : null}
        </button>
        <div className="mx-1 h-4 w-px bg-white/10" />
        {/* 逆向：最少用的一组，正好收进菜单 */}
        <Menu
          label={t("dbd.menuReverse")}
          className="ui-btn px-2 py-1 text-caption"
          items={[
            { key: "sqlite", label: <><Database className="h-3.5 w-3.5" />{t("dbd.reverseSqlite")}</>, onSelect: () => void reverseSqlite() },
            { key: "ddl", label: <><FileCode className="h-3.5 w-3.5" />{t("dbd.reverseDdl")}</>, onSelect: () => void reverseDdl() },
          ]}
        />

        {doc ? (
          <div className="ml-auto flex items-center gap-2">
            <input
              value={doc.name}
              onChange={(e) => patchDoc({ ...doc, name: e.target.value })}
              className="w-40 rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-100 outline-none focus:border-[var(--module-accent-ring)]"
            />
            <select
              value={doc.dialect}
              onChange={(e) => patchDoc({ ...doc, dialect: e.target.value as Dialect })}
              className="rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none"
            >
              {DIALECTS.map((d) => (
                <option key={d} value={d}>{d}</option>
              ))}
            </select>
            {/* 连线样式：只影响观感，不进设计文件；拖线时的实时预览也跟着换 */}
            <EdgeStyleSelect
              value={edgeStyle}
              onChange={setEdgeStyle}
              className="rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none"
            />
            {/* 节点卡片里显示哪些字段行：主键+外键（精简）/ 全部字段（展开，视图同时展开 SQL） */}
            <select
              value={fieldRows}
              onChange={(e) => {
                const next = e.target.value as FieldRowMode;
                setFieldRows(next);
                localStorage.setItem(FIELD_ROWS_KEY, next);
              }}
              title={t("dbd.fieldRows")}
              className="rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none"
            >
              {FIELD_ROW_MODES.map((mode) => (
                <option key={mode} value={mode}>
                  {t(`dbd.fieldRows_${mode}`)}
                </option>
              ))}
            </select>
            <span className="text-micro text-slate-600">
              {dirty ? t("dbd.unsaved") : path ? t("dbd.savedShort") : ""}
            </span>
          </div>
        ) : null}

        {/* 标签筛选：只看带这些标签的表（多选；空 = 全部显示） */}
        {doc && allTags.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1 border-b border-white/5 px-3 py-1.5">
            <span className="text-micro text-slate-500">{t("dbd.tagFilter")}</span>
            {allTags.map((tag) => (
              <button
                key={tag}
                onClick={() => setTagFilter((prev) => (prev.includes(tag) ? prev.filter((x) => x !== tag) : [...prev, tag]))}
                className={`cursor-pointer rounded-full border px-1.5 py-0.5 text-micro transition ${
                  tagFilter.includes(tag)
                    ? "border-[var(--module-accent)] bg-[var(--module-accent)]/20 text-[var(--module-accent)]"
                    : "border-white/10 text-slate-500 hover:border-white/20 hover:text-slate-300"
                }`}
              >
                {tag}
              </button>
            ))}
            {tagFilter.length > 0 ? (
              <>
                <span className="text-micro text-slate-600">
                  {t("dbd.tagFilterShow", {
                    count: doc.nodes.filter((n) => (n.tags ?? []).some((tag) => tagFilter.includes(tag))).length,
                    total: doc.nodes.length,
                  })}
                </span>
                <button onClick={() => setTagFilter([])} className="ui-btn px-1.5 py-0.5 text-micro">
                  {t("dbd.tagFilterClear")}
                </button>
              </>
            ) : null}
          </div>
        ) : null}
      </div>

      {/* 校验结果条：每条都是长句，一换行就是两三行 —— 不设上限时会直接把画布顶没。
          所以折叠只是一行摘要（✕ 数 · ! 数），展开也限高滚动，画布始终拿得回来。 */}
      {report && (report.errors.length > 0 || report.warnings.length > 0) ? (
        <div className="border-b border-white/5 px-3 py-1 text-micro">
          <button
            onClick={() => setReportOpen((v) => !v)}
            className="flex w-full items-center gap-1.5 text-left text-slate-400 hover:text-slate-200 cursor-pointer"
            title={t("dbd.reportToggleHint")}
          >
            <ChevronDown className={`w-3 h-3 shrink-0 transition-transform ${reportOpen ? "" : "-rotate-90"}`} />
            <span className="shrink-0 font-semibold text-slate-300">{t("dbd.reportTitle")}</span>
            {report.errors.length > 0 ? (
              <span className="shrink-0 text-rose-400">✕ {report.errors.length}</span>
            ) : null}
            {report.warnings.length > 0 ? (
              <span className="shrink-0 text-amber-400/90">! {report.warnings.length}</span>
            ) : null}
          </button>
          {reportOpen ? (
            <div className="mt-1 max-h-20 overflow-y-auto pr-1">
              {report.errors.map((e) => (
                <div key={e} className="text-rose-400">✕ {e}</div>
              ))}
              {report.warnings.map((w) => (
                <div key={w} className="text-amber-400/90">! {w}</div>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}

      {/* 操作结果条：**必须限高滚动**。这里是顶部的常规流容器，不是浮层 ——
          一条不限高的长文案（后端把校验清单整段抛回来就是这种）会把下面的画布
          整个顶出视野，看起来像「界面被盖住」而且无处滚动。校验类的长清单另有去处：
          上面的可折叠校验条（见 ensureValid）。 */}
      {result ? (
        <div className="max-h-24 shrink-0 overflow-y-auto px-3 pt-2">
          <ResultNote ok={result.ok} message={result.msg} />
        </div>
      ) : null}

      {/* 左 = 画布（上）+ 对象属性 dock（下）；右 = AI 助手侧栏（思维导图同款右栏）。
          侧栏收起时画布+dock 拿回全宽，展开时左侧让位 —— 画布高度不变。 */}
      <div className="flex min-h-0 flex-1">
        {/* 表导航（左栏 · 常驻可折叠）：纯文字树，一行一个表，按引用关系缩进。
            放左栏而不是画布角落的小浮层 —— 浮层会被当成「装饰」而根本没人找。 */}
        {doc && doc.nodes.length > 0 ? (
          /* 宽度必须**显式给**：早先靠内容撑开，展开时列宽等于最长表名（`truncate`
             因此永远不生效，表名越长画布被挤得越窄）；收起后表名虽然卸载了，但标题行
             「表 + 表数 + 箭头」仍撑出约 85px 的空条，白占画布。现在展开固定 208px、
             收起只留 32px 一竖条（图标 + 表数 + 折叠箭头竖排）。 */
          <div
            className={`flex min-h-0 flex-col border-r border-white/5 bg-white/[0.015] ${
              tableNavOpen ? "w-52" : "w-8 shrink-0"
            }`}
          >
            <button
              onClick={() => setTableNavOpen((v) => !v)}
              className={`flex shrink-0 border-b border-white/5 text-tiny font-semibold text-slate-400 hover:text-slate-200 cursor-pointer ${
                tableNavOpen ? "items-center gap-1.5 px-2.5 py-2" : "flex-col items-center gap-0.5 px-0 py-1.5"
              }`}
              title={t("dbd.tableNavTitle")}
            >
              <ListTree className="w-3.5 h-3.5 shrink-0" />
              {tableNavOpen ? (
                <>
                  {t("dbd.tableNav")}
                  <span className="text-slate-600">{doc.nodes.length}</span>
                  <ChevronDown className="ml-auto w-3 h-3 transition-transform" />
                </>
              ) : (
                <>
                  <span className="text-micro leading-none text-slate-600">{doc.nodes.length}</span>
                  <ChevronDown className="w-3 h-3 -rotate-90 transition-transform" />
                </>
              )}
            </button>
            {tableNavOpen ? (
              // 列表滚动时立刻关掉悬浮预览：预览是 fixed 定位的，滚动列表它不会跟着走，
              // 会「留在原地」指向已经滚走的表。思维导图没这问题（它不滚列表）。
              <div className="min-h-0 flex-1 overflow-y-auto px-1 py-1" onScroll={() => hp.hideNow()}>
                {tableTreeRows.map((r) => (
                  <button
                    key={r.id}
                    onClick={() => {
                      // 导航点表名 = 「我要看这张表」：高亮 + 画布移过去 + 打开面板。
                      // 三个都做，因为它本来就是唯一带「定位」的入口（画布上找一张表全靠拖）。
                      setHighlightId(r.id);
                      setSelectedId(r.id);
                      setCenterTargetId(r.id);
                      setCenterSignal((s) => s + 1);
                    }}
                    className={`flex w-full items-center gap-1 rounded py-1 pr-1.5 text-left text-tiny cursor-pointer transition-colors ${
                      r.id === highlightId
                        ? "bg-[var(--module-accent-soft)] text-[var(--module-accent)]"
                        : r.related
                          ? "text-cyan-300 hover:bg-white/5"
                          : "text-slate-400 hover:bg-white/5 hover:text-slate-200"
                    }`}
                    style={{ paddingLeft: 6 + r.depth * 10 }}
                    // 悬浮展示完整字段（与思维导图的节点预览共用 shared/HoverPreview）。
                    //
                    // **锚点用行的右缘 + 顶边，不是鼠标坐标**：跟随鼠标时每一行弹出的位置
                    // 都不同，眼睛要重新找；按列对齐后所有预览的左边框落在同一条竖线上，
                    // 上下扫过列表时视线不用横向漂移（用户要求）。
                    // 原生 title 只能塞一行字，放不下「这张表有哪些字段」—— 而表列表
                    // 刻意做成一行一个（字段最多、名字最短），不给悬浮就只能放大画布去看，
                    // 那是另一件事。onMouseMove 只在换行时重建内容，避免逐帧 setState。
                    onMouseEnter={(e) => {
                      const rect = e.currentTarget.getBoundingClientRect();
                      hp.show({
                        title: r.name,
                        x: rect.right,
                        y: rect.top,
                        accent: /^#[0-9a-f]{6}$/i.test(r.color ?? "") ? r.color : undefined,
                        content: <TablePreviewBody row={r} />,
                      });
                    }}
                    onMouseMove={(e) => {
                      if (hp.preview?.title !== r.name) {
                        const rect = e.currentTarget.getBoundingClientRect();
                        hp.show({
                          title: r.name, x: rect.right, y: rect.top,
                          accent: /^#[0-9a-f]{6}$/i.test(r.color ?? "") ? r.color : undefined,
                          content: <TablePreviewBody row={r} />,
                        });
                      }
                    }}
                    onMouseLeave={() => hp.scheduleHide()}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-1">
                        {/* 与画布同源的配色，一眼分出系列 */}
                        <span
                          className="w-1.5 h-1.5 rounded-full flex-shrink-0"
                          style={{ background: /^#[0-9a-f]{6}$/i.test(r.color ?? "") ? r.color : "var(--module-accent)" }}
                        />
                        <span className="truncate">{r.name}</span>
                        {r.relCount > 0 ? <span className="ml-auto text-micro text-slate-600">{r.relCount}</span> : null}
                      </span>
                      {/* 表注释：AI 产出里最值钱的部分，光看表名猜不出它存什么。
                          窄侧栏里必须 truncate —— 否则一行注释就能把整栏撑宽 */}
                      {r.comment ? (
                        <span className="block truncate text-[9px] leading-tight text-slate-600" title={r.comment}>
                          {r.comment}
                        </span>
                      ) : null}
                    </span>
                  </button>
                ))}
              </div>
            ) : null}
          </div>
        ) : null}

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {/* React Flow 必须有确定高度的容器，否则画布高度为 0 */}
        <div className="relative min-h-0 min-w-0 flex-1">
          {/* 侧栏收起但生成还在跑：画布角落留一个胶囊（点它重新展开侧栏看进度）。
              侧栏关掉不影响任务 —— 流式订阅挂在生成流程里，不在侧栏 DOM 上。 */}
          {doc && aiBusy && !aiOpen ? (
            <button
              onClick={() => setAiOpen(true)}
              title={t("dbd.aiPillHint")}
              className="absolute right-2 top-2 z-10 flex max-w-[280px] items-center gap-1.5 rounded-full border border-white/10 bg-slate-900/90 px-3 py-1.5 text-micro text-slate-300 shadow-lg transition hover:text-white"
            >
              <Loader2 className="h-3 w-3 flex-shrink-0 animate-spin" />
              <span className="truncate">{aiStatusText}</span>
            </button>
          ) : null}
          {doc ? (
            <ReactFlowProvider>
              <ReactFlow
                nodes={rfNodes}
                edges={rfEdges}
                nodeTypes={nodeTypes}
                onNodesChange={onNodesChange}
                onNodeDragStop={onNodeDragStop}
                onConnect={onConnect}
                connectionLineType={CONNECTION_LINE_BY_STYLE[edgeStyle]}
                onNodeClick={onCardClick}
                onPaneClick={() => {
                  // 点空白：两个「选中」一起清掉。面板随之收起（现有规则：取消选中 = 收缩），
                  // 高亮与淡化也一并解除。
                  setSelectedId(null);
                  setHighlightId(null);
                }}
                connectionMode={ConnectionMode.Loose}
                fitView
                minZoom={0.2}
                proOptions={{ hideAttribution: true }}
              >
                {/* 缩放控件（Controls）与缩略图（MiniMap）已按用户要求移除：
                    画布本来就常驻自动布局 + 手动平移缩放，缩略图在几十张表时只是噪音。
                    快捷键（滚轮缩放 / 空格拖拽 / 双击适应）仍可用。 */}
                <Background gap={16} size={1} color="rgba(148,163,184,0.15)" />
                <FitOnSignal signal={fitSignal} />
                <FitOnSignal signal={fitFocusSignal} ids={arrangeFitIds} />
                <CenterOnSignal signal={centerSignal} nodeId={centerTargetId} />
              </ReactFlow>
              {/* 外观调面板（高亮 / 淡化强度）：浮在画布右下角，避开左侧表导航、
                  右下 AI 侧栏与工具栏。默认折叠成一个图标钮 —— 调它是低频操作，
                  但展开后要一直能看见效果，所以不能塞进菜单里「打开→拖→关→看」。 */}
              <div className="pointer-events-auto absolute bottom-3 right-3 z-20">
                <GraphLookPanel
                  highlight={lookPrefs.highlight}
                  dim={lookPrefs.dim}
                  depth={highlightDepth}
                  onDepth={setHighlightDepth}
                  maxHighlight={HL_BOOST_MAX}
                  maxDim={DIM_MAX}
                  onHighlight={setHighlightStrength}
                  onDim={setDimStrength}
                  onReset={resetLookPrefs}
                  layoutMode={layoutMode}
                  onLayoutMode={(m) => {
                    setLayoutMode(m);
                    localStorage.setItem(LAYOUT_MODE_KEY, m);
                  }}
                  onAutoLayout={() => void autoLayout()}
                  onArrange={arrangeHighlighted}
                  onAutoColor={autoColorAll}
                  arrangeDisabled={!highlightId || relatedIds.size < 2}
                />
              </div>
            </ReactFlowProvider>
          ) : (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-slate-500">
              <Database className="h-10 w-10 opacity-30" />
              <p className="text-body">{t("dbd.emptyTitle")}</p>
              <p className="max-w-sm text-center text-caption text-slate-600">{t("dbd.emptyHint")}</p>
              <button onClick={() => void newDoc()} className="ui-btn ui-btn-primary mt-1 px-3 py-1.5 text-caption">
                <FilePlus2 className="h-3.5 w-3.5" /> {t("dbd.new")}
              </button>
            </div>
          )}
        </div>

        {/* 拖拽分隔线：上下拖动调整 dock 高度（↑↓ 键也能微调），高度记到 localStorage。
            收起时整条隐藏 —— 只剩一条边没必要再能拖。 */}
        {doc && dockOpen ? (
          <div
            role="separator"
            aria-orientation="horizontal"
            aria-label={t("dbd.dockResize")}
            tabIndex={0}
            onPointerDown={onSplitterDown}
            onPointerMove={onSplitterMove}
            onPointerUp={onSplitterUp}
            onKeyDown={(e) => {
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setDockHeight((h) => clampDockHeight(h + 24));
              } else if (e.key === "ArrowDown") {
                e.preventDefault();
                setDockHeight((h) => clampDockHeight(h - 24));
              }
            }}
            className="group relative z-10 h-1.5 shrink-0 cursor-row-resize"
          >
            <div className="absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-white/10 transition-colors group-hover:bg-[var(--module-accent)]/60" />
          </div>
        ) : null}

        {/* 对象属性 dock：表名 / 注释 / 删除 + 页签（字段 · 索引 · 关联） */}
        {doc ? (
          <div
            style={dockOpen ? { height: dockHeight } : undefined}
            className="flex w-full shrink-0 flex-col overflow-hidden border-t border-white/10 bg-surface-panel/60"
          >
            <div className="flex shrink-0 items-center gap-1.5 border-b border-white/5 px-3 py-1.5">
              {selected ? (
                <>
                  <Database className="h-3.5 w-3.5 flex-shrink-0 text-[var(--module-accent)]" />
                  <input
                    value={selected.name}
                    onChange={(e) => updateNode(selected.id, { name: e.target.value })}
                    className="min-w-0 flex-1 rounded-ctl bg-transparent px-2 py-1 text-caption font-semibold text-slate-100 outline-none focus:bg-white/5"
                  />
                  <input
                    value={selected.comment ?? ""}
                    onChange={(e) => updateNode(selected.id, { comment: e.target.value })}
                    placeholder={t("dbd.comment")}
                    className="min-w-0 flex-1 rounded-ctl bg-transparent px-2 py-1 text-micro text-slate-400 outline-none focus:bg-white/5"
                  />
                  <button onClick={() => deleteNode(selected.id)} className="ui-btn p-1" title={t("common.delete")}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </>
              ) : (
                <span className="text-caption text-slate-500">{t("dbd.dockHint")}</span>
              )}
              <button
                onClick={() => {
                  const next = !dockOpen;
                  setDockOpen(next);
                  dockLockedRef.current = !next; // 收起=上锁，展开=解锁
                }}
                className="ui-btn shrink-0 px-1.5 py-0.5 text-micro"
                title={dockOpen ? t("dbd.dockCollapse") : t("dbd.dockExpand")}
              >
                {dockOpen ? <ChevronDown className="h-3 w-3" /> : <ChevronUp className="h-3 w-3" />}
                {dockOpen ? t("dbd.dockCollapse") : t("dbd.dockExpand")}
              </button>
            </div>

            {selected && selected.kind === "table" ? (
              /* 标签与配色：标签用于筛选，配色会带到卡片色条和它的连线上 */
              <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-white/5 px-3 py-1.5">
                <span className="text-micro text-slate-500">{t("dbd.tags")}</span>
                {allTags.map((tag) => (
                  <button
                    key={tag}
                    onClick={() => toggleNodeTag(selected.id, tag)}
                    className={`cursor-pointer rounded-full border px-1.5 py-0.5 text-micro transition ${
                      (selected.tags ?? []).includes(tag)
                        ? "border-[var(--module-accent)] bg-[var(--module-accent)]/20 text-[var(--module-accent)]"
                        : "border-white/10 text-slate-500 hover:border-white/20 hover:text-slate-300"
                    }`}
                  >
                    {tag}
                  </button>
                ))}
                <input
                  value={newTag}
                  onChange={(e) => setNewTag(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key !== "Enter") return;
                    const tag = newTag.trim();
                    if (!tag) return;
                    if (!(selected.tags ?? []).includes(tag)) toggleNodeTag(selected.id, tag);
                    setNewTag("");
                  }}
                  placeholder={t("dbd.tagAddPh")}
                  className="w-20 rounded bg-white/5 px-1.5 py-0.5 text-micro text-slate-200 outline-none placeholder:text-slate-600"
                />
                <span className="ml-1 text-micro text-slate-500">{t("dbd.color")}</span>
                {NODE_COLORS.map((c) => (
                  <button
                    key={c}
                    onClick={() => updateNodeMeta(selected.id, { color: selected.color === c ? "" : c })}
                    title={c}
                    className={`h-3.5 w-3.5 cursor-pointer rounded-full border-2 transition ${
                      (selected.color ?? "") === c ? "scale-110 border-white" : "border-transparent"
                    }`}
                    style={{ background: c }}
                  />
                ))}
                {/* 原生取色器：预设之外还要对齐某个特定色（品牌色 / 与别处一致）时用。
                    左边一道竖线是为了和上面 12 个预设点区分开 —— 它不是「又一个预设」。 */}
                <span className="mx-0.5 h-3.5 w-px flex-shrink-0 bg-white/15" />
                <input
                  type="color"
                  value={isPresetColor(selected.color) ? selected.color! : NODE_COLORS[0]}
                  onChange={(e) => commitNodeColor(selected.id, e.target.value)}
                  title={t("dbd.colorCustom")}
                  className="h-3.5 w-5 flex-shrink-0 cursor-pointer rounded border border-white/20 bg-transparent p-0"
                />
                {/* 预设色靠「再点一次同一个圆点」取消，自定义色没有这个等价操作，
                    所以给它补一个清除按钮（否则用户设了自定义色就回不去默认强调色） */}
                {selected.color && !isPresetColor(selected.color) && (
                  <button
                    onClick={() => updateNodeMeta(selected.id, { color: "" })}
                    title={t("dbd.colorClear")}
                    className="flex h-3.5 w-3.5 flex-shrink-0 cursor-pointer items-center justify-center rounded-full border border-white/15 text-micro leading-none text-slate-400 transition hover:border-red-400/40 hover:text-red-300"
                  >
                    ×
                  </button>
                )}
              </div>
            ) : null}
            {selected && selected.kind === "view" ? (
              <textarea
                value={selected.view?.sql ?? ""}
                onChange={(e) => updateNode(selected.id, { view: { sql: e.target.value } })}
                className="min-h-0 flex-1 resize-none rounded-none bg-black/30 px-3 py-1.5 font-mono text-micro text-slate-200 outline-none"
              />
            ) : null}

            {/* 页签：字段 / 索引 / 关联。视图没有字段与索引，只有上面那段 SQL。 */}
            {selected && selected.kind === "table" && dockOpen ? (
              <>
                <div className="flex shrink-0 items-center gap-0.5 border-b border-white/5 px-2">
                  {(
                    [
                      ["fields", t("dbd.fields"), selected.table?.fields.length ?? 0],
                      ["indexes", t("dbd.indexes"), selected.table?.indexes?.length ?? 0],
                      ["relations", t("dbd.relations"), tableRelations.length],
                    ] as const
                  ).map(([id, label, count]) => (
                    <button
                      key={id}
                      onClick={() => setDockTab(id)}
                      className={`-mb-px flex cursor-pointer items-center gap-1 border-b-2 px-2.5 py-1.5 text-caption transition ${
                        dockTab === id
                          ? "border-[var(--module-accent)] text-slate-100"
                          : "border-transparent text-slate-500 hover:text-slate-300"
                      }`}
                    >
                      {label}
                      <span className="text-micro text-slate-600">{count}</span>
                    </button>
                  ))}
                </div>

                <div className="flex min-h-0 flex-1 flex-col">
                  {dockTab === "fields" ? (
                    <FieldTable
                      table={selected.table ?? { fields: [] }}
                      onUpdateField={(index, patch) => updateField(selected.id, index, patch)}
                      onRenameField={(oldName, newName) => void renameField(selected.id, oldName, newName)}
                      onAddField={() => addField(selected.id)}
                      onRemoveField={(name) => removeField(selected.id, name)}
                    />
                  ) : null}

                  {/* 索引：模型和导出早就支持（UNIQUE / KEY / FULLTEXT KEY），只是界面没入口。
                      primary 由字段上的 P 列表达，这里不重复提供 primary 选项，避免导出成两行。
                      一条索引一个小卡片，网格排布（宽屏一行 4 个），不让它占满一整行。 */}
                  {dockTab === "indexes" ? (
                    <div className="flex min-h-0 flex-1 flex-col">
                      <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-3 py-1.5">
                        <p className="min-w-0 flex-1 truncate text-micro text-slate-600">{t("dbd.indexHint")}</p>
                        <button onClick={() => addIndex(selected.id)} className="ui-btn shrink-0 px-1.5 py-0.5 text-micro">
                          <Plus className="h-3 w-3" /> {t("dbd.addIndex")}
                        </button>
                      </div>
                      <div className="min-h-0 flex-1 overflow-auto p-2">
                        {(selected.table?.indexes ?? []).length === 0 ? (
                          <div className="text-micro text-slate-600">{t("dbd.noIndexes")}</div>
                        ) : null}
                        <div className="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
                          {(selected.table?.indexes ?? []).map((idx, i) => (
                            /* 一个索引 = 名称 + 类型 + 「哪些列的组合」。列用勾选而不是手打逗号：
                               既能多列组合，也不会打出不存在的字段名。 */
                            <div
                              key={`${idx.name}-${i}`}
                              className="flex flex-col gap-1.5 rounded-ctl border border-white/5 bg-black/20 p-1.5"
                            >
                              <div className="flex items-center gap-1">
                                <input
                                  value={idx.name}
                                  onChange={(e) => updateIndex(selected.id, i, { name: e.target.value })}
                                  placeholder={t("dbd.indexName")}
                                  className="min-w-0 flex-1 rounded bg-black/30 px-1.5 py-0.5 font-mono text-micro text-slate-100 outline-none placeholder:font-sans placeholder:text-slate-600"
                                />
                                <button
                                  onClick={() => removeIndex(selected.id, i)}
                                  className="ui-btn shrink-0 p-0.5"
                                  title={t("common.delete")}
                                >
                                  <Trash2 className="h-3 w-3" />
                                </button>
                              </div>
                              <select
                                value={idx.kind}
                                onChange={(e) => updateIndex(selected.id, i, { kind: e.target.value as DbIndex["kind"] })}
                                className="rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                              >
                                <option value="index">{t("dbd.indexKindIndex")}</option>
                                <option value="unique">{t("dbd.indexKindUnique")}</option>
                                <option value="fulltext">{t("dbd.indexKindFulltext")}</option>
                              </select>
                              <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
                                {(selected.table?.fields ?? []).map((f) => (
                                  /* 勾选框而不是「点一下就切换」的 chip：多选组合时勾选框的
                                     「当前勾了哪些」一目了然，不用点完再回头看配色。 */
                                  <label
                                    key={f.name}
                                    className="flex cursor-pointer items-center gap-1 text-micro text-slate-300"
                                    title={typeLabel(f.type)}
                                  >
                                    <input
                                      type="checkbox"
                                      className="h-3 w-3 accent-[var(--module-accent)]"
                                      checked={idx.fields.includes(f.name)}
                                      onChange={() => toggleIndexField(selected.id, i, f.name)}
                                    />
                                    <span className="font-mono">{f.name}</span>
                                    {f.pk ? <span className="text-amber-400">*</span> : null}
                                  </label>
                                ))}
                                {(selected.table?.fields ?? []).length === 0 ? (
                                  <span className="text-micro text-slate-600">{t("dbd.noFields")}</span>
                                ) : null}
                              </div>
                              {idx.fields.length > 0 ? (
                                <div className="font-mono text-micro text-slate-500" title={t("dbd.indexChosenHint")}>
                                  {idx.fields.join(" , ")}
                                </div>
                              ) : null}
                            </div>
                          ))}
                        </div>
                      </div>
                    </div>
                  ) : null}

                {/* 关联：建关联的主入口是「拖线」，这里是补录 / 改属性 */}
                {dockTab === "relations" ? (
                  <div className="flex min-h-0 flex-1 flex-col">
                    <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-3 py-1.5">
                      {/* 只看自己这一侧的关系：审一张表时另外那些是噪音 */}
                      <div className="flex shrink-0 items-center gap-0.5 rounded-ctl bg-white/5 p-0.5">
                        {REL_FILTERS.map(([id, label]) => (
                          <button
                            key={id}
                            onClick={() => setRelFilter(id)}
                            className={`cursor-pointer rounded px-1.5 py-0.5 text-micro transition ${
                              relFilter === id
                                ? "bg-[var(--module-accent)] text-white"
                                : "text-slate-400 hover:text-slate-200"
                            }`}
                          >
                            {label}
                          </button>
                        ))}
                      </div>
                      <p className="min-w-0 flex-1 truncate text-micro text-slate-600">{t("dbd.connectHint")}</p>
                      <button onClick={addRelation} className="ui-btn shrink-0 px-1.5 py-0.5 text-micro">
                        <Plus className="h-3 w-3" /> {t("dbd.addRelation")}
                      </button>
                    </div>
                    <div className="min-h-0 flex-1 overflow-auto p-2">
              {shownRelations.length === 0 ? (
                <div className="text-micro text-slate-600">{t("dbd.noRelations")}</div>
              ) : null}
              {/* 一条关联一个小卡片，网格排布（宽屏一行 4 个）。
                  方向用「主表在上 / 子表在下 + 箭头图标」表达，原来那个 ↓ 字符太轻，
                   分不清谁是主表；现在主表天蓝、子表琥珀，两端下拉也各自染色。 */}
              <div className="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
              {shownRelations.map((r) => {
                const parentName = doc.nodes.find((n) => n.id === r.to.node)?.name ?? "?";
                const childName = doc.nodes.find((n) => n.id === r.from.node)?.name ?? "?";
                return (
                <div
                  key={r.id}
                  className="flex flex-col gap-1 rounded-ctl border border-white/5 bg-black/20 p-1.5 text-micro"
                >
                  <div className="flex items-center gap-1">
                    <span className="min-w-0 flex-1 truncate text-tiny font-semibold text-sky-300" title={t("dbd.relParent")}>
                      {parentName}.{r.to.field}
                    </span>
                    <button
                      onClick={() => removeRelation(r.id)}
                      className="ui-btn shrink-0 p-0.5"
                      title={t("common.delete")}
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                  <div className="flex items-center gap-1 text-slate-600">
                    <span className="h-px flex-1 bg-white/10" />
                    <ArrowRight className="h-3 w-3 flex-shrink-0" />
                    <span className="h-px flex-1 bg-white/10" />
                  </div>
                  <div className="min-w-0 truncate text-tiny font-semibold text-amber-300" title={t("dbd.relChild")}>
                    {childName}.{r.from.field}
                  </div>

                  <div className="mt-0.5 flex items-center gap-1">
                    <select
                      value={r.to.node}
                      onChange={(e) => updateRelation(r.id, { to: { ...r.to, node: e.target.value } })}
                      title={t("dbd.relParent")}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-micro text-sky-300 outline-none"
                    >
                      {doc.nodes.filter((n) => n.kind === "table").map((n) => (
                        <option key={n.id} value={n.id}>{n.name}</option>
                      ))}
                    </select>
                    <select
                      value={r.to.field}
                      onChange={(e) => updateRelation(r.id, { to: { ...r.to, field: e.target.value } })}
                      className="min-w-0 w-[5.5rem] rounded bg-white/5 px-1 py-0.5 font-mono text-micro text-sky-300 outline-none"
                    >
                      {(doc.nodes.find((n) => n.id === r.to.node)?.table?.fields ?? []).map((f) => (
                        <option key={f.name} value={f.name}>{f.name}</option>
                      ))}
                    </select>
                  </div>
                  <div className="flex items-center gap-1">
                    <select
                      value={r.from.node}
                      onChange={(e) => updateRelation(r.id, { from: { ...r.from, node: e.target.value } })}
                      title={t("dbd.relChild")}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-micro text-amber-300 outline-none"
                    >
                      {doc.nodes.filter((n) => n.kind === "table").map((n) => (
                        <option key={n.id} value={n.id}>{n.name}</option>
                      ))}
                    </select>
                    <select
                      value={r.from.field}
                      onChange={(e) => updateRelation(r.id, { from: { ...r.from, field: e.target.value } })}
                      className="min-w-0 w-[5.5rem] rounded bg-white/5 px-1 py-0.5 font-mono text-micro text-amber-300 outline-none"
                    >
                      {(doc.nodes.find((n) => n.id === r.from.node)?.table?.fields ?? []).map((f) => (
                        <option key={f.name} value={f.name}>{f.name}</option>
                      ))}
                    </select>
                  </div>

                  <div className="mt-0.5 flex items-center gap-1">
                    <select
                      value={r.kind}
                      onChange={(e) => updateRelation(r.id, { kind: e.target.value as DbDesignRelation["kind"] })}
                      className="rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                    >
                      {REL_KINDS.map((k) => (
                        <option key={k} value={k}>{k}</option>
                      ))}
                    </select>
                    <select
                      value={r.onDelete ?? "RESTRICT"}
                      onChange={(e) => updateRelation(r.id, { onDelete: e.target.value })}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                      title={t("dbd.onDelete")}
                    >
                      {FK_ACTIONS.map((a) => (
                        <option key={a} value={a}>{a}</option>
                      ))}
                    </select>
                    {/* 跟随主键 = 子表的外键列跟着父表主键走。默认由「拖金色锚点」自动判定，
                        这里可以手动改 —— 已有设计忘了开也能就地补救。 */}
                    <label
                      className="flex shrink-0 cursor-pointer items-center gap-1 text-micro text-slate-400"
                      title={t("dbd.hintMirror")}
                    >
                      <input
                        type="checkbox"
                        className="h-3 w-3 accent-[var(--module-accent)]"
                        checked={!!r.mirror}
                        onChange={(e) => toggleRelationMirror(r.id, e.target.checked)}
                      />
                      {t("dbd.chipMirror")}
                    </label>
                  </div>
                </div>
                );
              })}
              </div>
                    </div>
                  </div>
                ) : null}
              </div>
            </>
          ) : null}
        </div>
      ) : null}
      </div>

      {/* AI 助手侧栏（思维导图同款右栏）：全高、左缘可拖宽、工具栏可收起。
          收起不中断生成 —— 流式订阅挂在生成流程里，画布角落的胶囊可恢复。 */}
      {/* AI 助手侧栏：窄条 / 展开两态 + 气泡式对话。抽成组件是因为这一块
          既有窄条壳、又有气泡列表、还有追问卡与决策区 —— 内联进来会让本文件
          再长几百行，且每一处 JSX 改动都要在三层嵌套里定位。 */}
      {doc ? (
        <AiSidebar
          open={aiOpen}
          busy={aiBusy}
          width={aiPanelW}
          messages={chatMsgs}
          decisions={doc.decisions ?? []}
          decisionsOpen={decisionsOpen}
          onToggleDecisions={() => setDecisionsOpen((v) => !v)}
          onRemoveDecision={(text) =>
            patchDoc({ ...doc, decisions: (doc.decisions ?? []).filter((x) => x !== text) })
          }
          onDecisionDraft={setDecisionDraft}
          decisionDraft={decisionDraft}
          onClose={() => setAiOpen(false)}
          onOpen={() => setAiOpen(true)}
          onResizeStart={(e) => {
            if (e.button !== 0) return;
            e.preventDefault();
            const startX = e.clientX;
            const startW = aiPanelW;
            const onMove = (ev: MouseEvent) => setAiPanelW(startW - (ev.clientX - startX));
            const onUp = () => {
              window.removeEventListener("mousemove", onMove);
              window.removeEventListener("mouseup", onUp);
            };
            window.addEventListener("mousemove", onMove);
            window.addEventListener("mouseup", onUp);
          }}
          onAnswer={(answer) => {
            setAiClarify(null);
            void runAi(answer);
          }}
          text={aiText}
          onTextChange={setAiText}
          onSubmit={() => {
            // runAi 读的是**本次渲染闭包里的** aiText，清空要到下一次渲染才生效，
            // 所以这里先清空不会把这一轮打成空串（反过来才要担心顺序）。
            setAiText("");
            void runAi();
          }}
          onCancel={() => void cancelAi()}
          noProvider={aiProviders.length === 0}
          providers={aiProviders as unknown as ProviderLike[]}
          providerId={aiProviderId}
          modelId={aiModelId}
          onModelSelect={onAiModelSelect}
          sessionId={sessionId}
          sessionPath={path}
          sessionDocName={doc?.name ?? ""}
          sessions={sessions}
          onSessionSelect={(id) => void switchSession(id)}
          onSessionCreate={() => void createSession()}
          onSessionDelete={(id) => void deleteSession(id)}
          chatLoading={chatLoading}
        />
      ) : null}

      {/* 表列表的悬浮字段预览（fixed 定位，随鼠标） */}
      {hp.preview && <HoverPreview preview={hp.preview} width={320} maxHeight={340} hoverGuard={hp.hoverGuard} />}
      </div>
      {/* 导出弹框：只在打开时挂载 —— 每次打开都用默认选项（不记忆上次选择） */}
      {commonFieldsOpen && doc ? (
        <CommonFieldsDialog
          open
          value={doc.commonFields}
          onClose={() => setCommonFieldsOpen(false)}
          onSave={(commonFields) => patchDoc({ ...doc, commonFields })}
        />
      ) : null}
      {exportOpen ? (
        <ExportDialog
          open
          designPath={path}
          busy={exportBusy}
          onClose={() => setExportOpen(false)}
          onExport={(choice) => void exportSql(choice)}
        />
      ) : null}
    </div>
  );
}
