import { DbDocumentToolbar } from "./DbDocumentToolbar";
import { DbRelationsEditor } from "./DbRelationsEditor";
import { DbIndexesEditor } from "./DbIndexesEditor";
import { DbNodeAppearanceEditor } from "./DbNodeAppearanceEditor";
import { DbTableNavigationList } from "./DbTableNavigationList";
import { useDbEditorDock } from "./useDbEditorDock";
import { useDbDocumentFiles } from "./useDbDocumentFiles";
import { useDbDesignExport } from "./useDbDesignExport";
import { useDbCanvasGraph } from "./useDbCanvasGraph";
import { useDbChatSessions } from "./useDbChatSessions";
import { useDbAiRunner } from "./useDbAiRunner";
import { useDbDocumentEditing } from "./useDbDocumentEditing";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Background, ConnectionMode, ReactFlow, ReactFlowProvider } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { CONNECTION_LINE_BY_STYLE, useEdgeStyle } from "../../utils/graphEdgeStyle";
import type { ProviderLike } from "../shared/ai/ModelSelector";
import { cardClickAction, clickRoleOf } from "./cardClick";
import { clampHighlightDepth, type HighlightDepth } from "./series";
import { Menu } from "../shared/Menu";
import { ChevronDown, Loader2, ChevronUp, Database, Eye, FileCode, FileDown, FilePlus2, ListTree, Sparkles, SlidersHorizontal, Trash2 } from "lucide-react";
import FieldTable from "./FieldTable";
import GraphLookPanel from "./GraphLookPanel";
import AiSidebar from "./AiSidebar";
import CommonFieldsDialog from "./CommonFieldsDialog";
import ExportDialog from "./ExportDialog";
import { DIM_MAX, HL_BOOST_MAX, useGraphHighlightPrefs } from "../../utils/graphHighlight";
import { useAiPanelWidth } from "../shared/ai/paneWidth";
import { ResultNote } from "../shared/Note";
import { HoverPreview, useHoverPreview } from "../shared/HoverPreview";
import { type DbDesignDocument } from "./types";
import { HIGHLIGHT_DEPTH_KEY, LAYOUT_MODE_KEY, assignSeriesColors, nodeTypes } from "./DbDesignerPanel.model";
import { FitOnSignal } from "./DbDesignerPanel.FitOnSignal";
import { CenterOnSignal } from "./DbDesignerPanel.CenterOnSignal";

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

  /** 整理选中后要定位到那几张表：下标这张。与 fitSignal 同次写入，
   *  避免两个状态分开更新导致视野与坐标不匹配。 */

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
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const {
    report,
    reportOpen,
    setReportOpen,
    exportOpen,
    setExportOpen,
    exportBusy,
    ensureValid,
    exportSql,
    openExportDialog,
    exportOutline,
  } = useDbDesignExport({
    doc,
    path,
    setResult,
  });
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
  const [commonFieldsOpen, setCommonFieldsOpen] = useState(false);
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

  const {
    sessionId,
    sessions,
    chatLoading,
    touchSession,
    commitChat,
    saveChatNow,
    loadChatForDoc,
    switchSession,
    createSession,
    deleteSession,
  } = useDbChatSessions({
    doc, path,
    readChatSnapshot: () => ({ msgs: chatMsgsRef.current, history: aiHistory, clarifyRound: aiClarifyRound }),
    applyChatSnapshot: chat => { setChatMsgs(chat.msgs); setAiHistory(chat.history); setAiClarifyRound(chat.clarifyRound); },
    resetChatEphemeral: () => { setAiClarify(null); setAiNewDecisions([]); setDocSnapshot(null); },
    assignDocumentIdentity: (id, source) => {
      if (source) setDoc(prev => prev && prev.id === source.id ? { ...prev, id } : prev);
      else setDoc(doc ? { ...doc, id } : doc);
      setDirty(true);
    },
  });

  // 供应商 / 模型列表来自 AI 模块的现有配置（与思维导图、API 模块同一份）

  const {
    dockOpen,
    setDockOpen,
    dockLockedRef,
    dockHeight,
    setDockHeight,
    dockDragging,
    fieldRows,
    setFieldRows,
    rootRef,
    clampDockHeight,
    onSplitterDown,
    onSplitterMove,
    onSplitterUp,
  } = useDbEditorDock({
    selectedId,
    doc,
  });

  // 文档一变就问后端要校验结果（后端是唯一真源，前端不复制校验规则）

  const patchDoc = useCallback((next: DbDesignDocument) => {
    setDoc(next);
    setDirty(true);
  }, []);

  const {
    fitSignal,
    setFitSignal,
    fitFocusSignal,
    arrangeFitIds,
    layoutMode,
    setLayoutMode,
    autoLayout,
    arrangeHighlighted,
    setPositions,
    allTags,
    relatedIds,
    tableTreeRows,
    rfNodes,
    rfEdges,
    onNodesChange,
    onNodeDragStop,
    onConnect,
  } = useDbCanvasGraph({
    doc,
    tagFilter,
    highlightId,
    highlightDepth,
    lookPrefs,
    fieldRows,
    edgeStyle,
    patchDoc,
  });

  const {
    aiText,
    setAiText,
    aiBusy,
    aiHistory,
    setAiHistory,
    chatMsgs,
    setChatMsgs,
    setAiClarify,
    aiClarifyRound,
    setAiClarifyRound,
    setAiNewDecisions,
    setDocSnapshot,
    decisionsOpen,
    setDecisionsOpen,
    decisionDraft,
    setDecisionDraft,
    aiProviders,
    aiProviderId,
    aiModelId,
    chatMsgsRef,
    aiStatusText,
    onAiModelSelect,
    runAi,
    cancelAi,
  } = useDbAiRunner({
    doc,
    chatLoading,
    applyGeneratedDoc: (next, { resetPositions = true } = {}) => {
      patchDoc(next);
      if (resetPositions) setPositions({});
    },
    fitCanvas: (clearSelection = false) => {
      if (clearSelection) { setSelectedId(null); setHighlightId(null); }
      setFitSignal(n => n + 1);
    },
    layoutMode,
    fieldRows,
    commitChat: snapshot => commitChat(snapshot),
  });

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

  const {
    newDoc,
    openFile,
    saveFile,
    reverseSqlite,
    reverseDdl,
  } = useDbDocumentFiles({
    doc, path, ensureValid, saveChatNow, loadChatForDoc, setResult,
    onAdoptDocument: (next, options) => {
      setDoc(next); setPath(options.path); setDirty(options.dirty);
      setSelectedId(null); setHighlightId(null); setPositions({});
      if (options.resetDraft) setAiText("");
      if (options.resetMemory) {
        setAiHistory([]); setAiClarify(null); setAiClarifyRound(0);
        setAiNewDecisions([]); setDocSnapshot(null);
      }
    },
    onSavedDocument: target => { setPath(target); setDirty(false); void touchSession(sessionId, target); },
  });
  // ── 节点 ──
  const {
    addNode,
    updateNode,
    deleteNode,
    updateField,
    addField,
    removeField,
    updateIndex,
    addIndex,
    toggleIndexField,
    removeIndex,
    renameField,
    addRelation,
    updateRelation,
    updateNodeMeta,
    commitNodeColor,
    toggleNodeTag,
    removeRelation,
    toggleRelationMirror,
  } = useDbDocumentEditing({
    doc,
    patchDoc,
    setSelectedId,
    setHighlightId,
    selectedId,
    highlightId,
    resetAiConversation: () => { setAiHistory([]); setAiClarify(null); setAiClarifyRound(0); },
    selected: doc?.nodes.find(node => node.id === selectedId) ?? null,
  });

  // ── 画布 ──
  //
  // 拖拽方案**照抄项目里已经跑通的思维导图写法**（不要重新发明）：
  //   ① 位置只写本地 posOverrides，松手才落文档 —— 避免 onNodesChange → 重建文档 → 重建节点 的反馈循环；
  //   ② RF 实测的节点尺寸（dimensions）必须记录并挂回节点对象 —— 缺了它 RF 会在拖动中每帧重测，
  //      节点尺寸塌陷/恢复，人眼看到的就是「拖不跟手 / 闪烁」；
  //   ③ 节点对象按 id 缓存：只有位置变动的那个重建对象，其余复用旧 data 引用，memo 才能跳过 → 无闪烁。

  /** 重新布局：算完直接写回 node.x/y，并把拖动覆盖清掉（否则旧坐标会盖住新布局）。
   *  没有撤销栈，所以先问一句 —— 手工摆好的位置会被覆盖。 */

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
          <DbDocumentToolbar
            doc={doc}
            patchDoc={patchDoc}
            edgeStyle={edgeStyle}
            setEdgeStyle={setEdgeStyle}
            fieldRows={fieldRows}
            setFieldRows={setFieldRows}
            dirty={dirty}
            path={path}
          />
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
              <DbTableNavigationList
                hp={hp}
                tableTreeRows={tableTreeRows}
                setHighlightId={setHighlightId}
                setSelectedId={setSelectedId}
                setCenterTargetId={setCenterTargetId}
                setCenterSignal={setCenterSignal}
                highlightId={highlightId}
              />
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
              <DbNodeAppearanceEditor
                allTags={allTags}
                toggleNodeTag={toggleNodeTag}
                selected={selected}
                newTag={newTag}
                setNewTag={setNewTag}
                updateNodeMeta={updateNodeMeta}
                commitNodeColor={commitNodeColor}
              />
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
                    <DbIndexesEditor
                      addIndex={addIndex}
                      selected={selected}
                      updateIndex={updateIndex}
                      removeIndex={removeIndex}
                      toggleIndexField={toggleIndexField}
                    />
                  ) : null}

                {/* 关联：建关联的主入口是「拖线」，这里是补录 / 改属性 */}
                {dockTab === "relations" ? (
                  <DbRelationsEditor
                    REL_FILTERS={REL_FILTERS}
                    setRelFilter={setRelFilter}
                    relFilter={relFilter}
                    addRelation={addRelation}
                    shownRelations={shownRelations}
                    doc={doc}
                    removeRelation={removeRelation}
                    updateRelation={updateRelation}
                    toggleRelationMirror={toggleRelationMirror}
                  />
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
