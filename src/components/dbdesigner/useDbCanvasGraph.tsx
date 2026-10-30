import type { DbTableNavigationRow } from "./canvasTypes";
import { DEFAULT_LAYOUT_MODE, computeLayout, type LayoutMode } from "./layout";
import { theamedConfirm } from "../shared/ThemedAlert";
import { buildArrangeSubDoc } from "./series";
import { LAYOUT_MODE_KEY } from "./DbDesignerPanel.model";
import type { GraphHighlightPrefs } from "../../utils/graphHighlight";
import { useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { MarkerType, type Connection, type Edge, type Node, type NodeChange } from "@xyflow/react";
import { relatedEdgeIds, relatedTableHops, relatedTableIds } from "./series";
import { type DesignNodeData } from "./DesignNodeCard";
import { EDGE_FLOW_CLASS, edgeGlow } from "../../utils/graphHighlight";
import { dimmedEdgeStroke, dimmedEdgeWidth, dimmedOpacity, hasVisibleHighlight, relatedEdgeWidth } from "../../utils/graphHighlight";
import { toast } from "../shared/Toast";
import { HANDLE_TABLE, HANDLE_TABLE_IN, edgeAnchors, fkFieldsOf, handleAllPk, relationFieldsOf, type FieldRowMode, type DbDesignDocument, type DbDesignNode, type DbTableBody } from "./types";
import { newId } from "./DbDesignerPanel.model";

/** Owns the DbCanvasGraph feature, independently of page rendering. */
export function useDbCanvasGraph({ doc, tagFilter, highlightId, highlightDepth, lookPrefs, fieldRows, edgeStyle, patchDoc }: {
  doc: DbDesignDocument | null;
  tagFilter: string[];
  highlightId: string | null;
  highlightDepth: 1 | 2 | 3 | 4 | 5;
  lookPrefs: GraphHighlightPrefs;
  fieldRows: FieldRowMode;
  edgeStyle: "bezier" | "smoothstep" | "straight";
  patchDoc: (next: DbDesignDocument) => void;
}) {
  const { t } = useTranslation();

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

    if (!doc) return [] as DbTableNavigationRow[];
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
    const rows: DbTableNavigationRow[] = [];
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

;

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
  return {
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
  };
}
