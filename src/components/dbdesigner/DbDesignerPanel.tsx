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
import { computeLayout } from "./layout";
import EdgeStyleSelect from "../shared/EdgeStyleSelect";
import {
  ArrowRight,
  Ban,
  Brain,
  Check,
  ChevronDown,
  Loader2,
  ChevronUp,
  Database,
  Eye,
  FileCode,
  FileDown,
  FilePlus2,
  FolderOpen,
  LayoutGrid,
  Plus,
  Rows3,
  Save,
  Sparkles,
  Terminal,
  Trash2,
} from "lucide-react";

import DesignNodeCard, {
  HANDLE_TABLE,
  HANDLE_TABLE_IN,
  handleAllPk,
  handleField,
  handleFieldIn,
  type DesignNodeData,
} from "./DesignNodeCard";
import FieldTable from "./FieldTable";
import { ModelSelector } from "../ai/ModelSelector";
import type { ProviderLike } from "../ai/ModelSelector";
import { useAiPanelWidth } from "../ai/paneWidth";
import { createEventBuffer, useEventBufferSnapshot } from "../../utils/eventBuffer";
import { ResultNote } from "../shared/Note";
import { theamedConfirm } from "../shared/ThemedAlert";
import { toast } from "../shared/Toast";
import {
  DIALECTS,
  FIELD_ROW_MODES,
  fkFieldsOf,
  relationFieldsOf,
  typeLabel,
  type FieldRowMode,
  type DbDesignDocument,
  type DbDesignNode,
  type DbDesignRelation,
  type DbField,
  type DbIndex,
  type DbTableBody,
  type Dialect,
  type ValidationReport,
} from "./types";

// nodeTypes 必须是组件外的常量：内联新建会让 React Flow 每次渲染都重挂载节点（官方文档警告的卡顿源）。
const nodeTypes = { designNode: DesignNodeCard };

/** 画布内的「适应视野」触发器：signal 每 +1 就重新 fitView 一次。
 *  必须待在 ReactFlowProvider 里面（useReactFlow 依赖它），所以单独拆个小组件。 */
function FitOnSignal({ signal }: { signal: number }) {
  const { fitView } = useReactFlow();
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false; // 首挂载交给 <ReactFlow fitView>，别动初始视野
      return;
    }
    if (signal > 0) {
      void fitView({ padding: 0.15, duration: 260 });
    }
  }, [signal, fitView]);
  return null;
}

/** dock 高度（px）：拖分隔线调整并记住。上下限 = 至少能看到表头+页签+一行字段，
 *  且必须给画布留 MIN_CANVAS_HEIGHT，否则窗口小的时候画布会被挤没。 */
const DOCK_HEIGHT_KEY = "kira.dbd.dockHeight";
const FIELD_ROWS_KEY = "kira.dbd.fieldRows";
const EXPAND_KEY = "kira.dbd.alwaysExpand";
/** 模块级进度缓冲：与思维导图同款（keep-alive 切换页面也不会丢事件）。
 *  data：stream/reasoning 帧携带 length（累计字符）+ text（末尾 200 字预览）；
 *  reasoning 收尾帧带 done=true。 */
const dbdAiProgressBuffer = createEventBuffer<{
  step?: string;
  data?: { length?: number; text?: string; done?: boolean };
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

/** 表配色预设（点一下取消；不选 = 用模块强调色） */
const NODE_COLORS = ["#0ea5e9", "#22c55e", "#f59e0b", "#ef4444", "#a855f7", "#14b8a6"];
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
  return { name: "id", type: { base }, nullable: false, pk: false, autoIncrement: false, unique: false, comment: "" };
}

export default function DbDesignerPanel() {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<DbDesignDocument | null>(null);
  const [path, setPath] = useState<string>("");
  const [dirty, setDirty] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  /** 常驻展开：打开后所有节点一直显示全部字段。
   *  刻意**不做**「悬停/选中就展开」—— 那个靠鼠标悬停才能看到全貌，
   *  一旦移开又缩回去，读字段时来回移动很烦（用户明确要求去掉）。 */
  const [alwaysExpand, setAlwaysExpand] = useState<boolean>(() => localStorage.getItem(EXPAND_KEY) === "1");
  /** 布局变化计数：+1 就让画布重新适应视野（自动布局后必须做，否则停在角落） */
  const [fitSignal, setFitSignal] = useState(0);
  /** 连线样式（曲线 / 直角折线 / 直线）：与思维导图、JSON 图共用一份偏好 */
  const [edgeStyle, setEdgeStyle] = useEdgeStyle();
  const [report, setReport] = useState<ValidationReport | null>(null);
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null);
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
  const [aiAborted, setAiAborted] = useState(false);
  const [aiErr, setAiErr] = useState("");
  const [aiProviders, setAiProviders] = useState<AiProviderLike[]>([]);
  const [aiProviderId, setAiProviderId] = useState("");
  const [aiModelId, setAiModelId] = useState("");
  /** 本次运行的 id：发给后端，用于「停止」 */
  const aiRunIdRef = useRef("");
  /** 流式落图时已经排好位的节点（id → 坐标）：同一个节点不会被后来的批次挪动 */
  const aiPlacedRef = useRef<Record<string, { x: number; y: number }>>({});
  const aiProgress = useEventBufferSnapshot(dbdAiProgressBuffer);
  /** 思考过程（DeepSeek-R1 / Qwen 思考模式等推理模型）：最新一帧思考。
   *  思考块收尾（done）或正文已经开始输出（后面出现 stream 帧）后为 null。 */
  const aiReasoning = useMemo(() => {
    for (let i = aiProgress.length - 1; i >= 0; i--) {
      const e = aiProgress[i];
      if (e.step === "reasoning") return e.data?.done ? null : { length: e.data?.length ?? 0, text: e.data?.text ?? "" };
      if (e.step === "stream") return null;
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
  /** 状态条 / 胶囊文案：思考中 → 写设计中 → 已完成 */
  const aiStatusText = aiBusy
    ? aiStream
      ? t("dbd.aiWritingChars", { count: aiStream.length })
      : aiThinkFinal
        ? t("dbd.aiThinkingChars", { count: aiThinkFinal.length })
        : t("dbd.aiThinking")
    : t("dbd.aiDoneShort");

  // 供应商 / 模型列表来自 AI 模块的现有配置（与思维导图、API 模块同一份）
  useEffect(() => {
    invoke<any>("get_ai_config")
      .then((cfg) => {
        const list: AiProviderLike[] = (cfg?.providers ?? []).filter(
          (p: AiProviderLike) => p.api_key && p.openai_url,
        );
        setAiProviders(list);
        if (list.length > 0) {
          setAiProviderId(list[0].id);
          setAiModelId(list[0].active_model_id ?? list[0].models?.[0]?.id ?? "");
        }
      })
      .catch(() => {});
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

  // ── 文件 ──
  const newDoc = async () => {
    const d = await invoke<DbDesignDocument>("dbd_new_document", { name: t("dbd.untitled"), dialect: "mysql" });
    setDoc(d);
    setPath("");
    setDirty(true);
    setSelectedId(null);
    setPositions({});
  };

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
      setDoc(d);
      setPath(picked);
      setDirty(false);
      setSelectedId(null);
      setPositions({});
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  const saveFile = async (as: boolean) => {
    if (!doc) return;
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
      setResult({ ok: true, msg: t("dbd.saved", { path: target }) });
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
  };

  const exportSql = async () => {
    if (!doc) return;
    if (!path) {
      // 没落盘过：直接导出文本，让用户自己另存
      try {
        const sql = await invoke<string>("dbd_export_sql", { doc });
        await navigator.clipboard.writeText(sql);
        toast(t("dbd.sqlCopied"), "ok");
      } catch (e) {
        setResult({ ok: false, msg: String(e) });
      }
      return;
    }
    try {
      const out = await invoke<string>("dbd_export_sql_file", { path });
      setResult({ ok: true, msg: t("dbd.sqlExported", { path: out }) });
    } catch (e) {
      setResult({ ok: false, msg: String(e) });
    }
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

  const rfNodes: Node[] = useMemo(() => {
    if (!doc) return [];
    const cache = nodeObjCache.current;
    const out: Node[] = [];
    for (const n of visibleNodes) {
      const p = positions[n.id] ?? { x: n.x ?? 0, y: n.y ?? 0 };
      const selected = selectedId === n.id;
      const expanded = alwaysExpand;
      const measured = measuredMap[n.id];
      const prev = cache.get(n.id);
      const prevObj = prev?.obj;
      const prevData = prevObj?.data as DesignNodeData | undefined;
      // 缓存键：文档引用 + 节点引用 + 坐标 + 选中/展开态 + 显示模式 + 尺寸。
      // 纯拖动时这些都没变 → 复用旧对象，memo 直接跳过。
      if (
        prev &&
        prev.doc === doc &&
        prev.node === n &&
        prev.px === p.x &&
        prev.py === p.y &&
        prevData?.fieldRows === fieldRows
      ) {
        out.push(prev.obj);
        continue;
      }
      const dataChanged =
        !prevData ||
        prev!.doc !== doc ||
        prev!.node !== n ||
        (prevData.selected !== selected) ||
        (prevData.expanded !== expanded) ||
        (prevData.fieldRows !== fieldRows);
      const data = (dataChanged
        ? {
            node: n,
            relationFields: relationFieldsOf(doc, n.id),
            // 外键列 = 在关系里当「多」端（from）出现的字段
            fkFields: fkFieldsOf(doc, n.id),
            expanded,
            selected,
            fieldRows,
          }
        : prevData) as unknown as Record<string, unknown>;
      const obj: Node = prevObj
        ? { ...prevObj, position: p, data, measured: measured ?? prevObj.measured }
        : { id: n.id, type: "designNode", position: p, data, measured };
      cache.set(n.id, { obj, px: p.x, py: p.y, doc, node: n });
      out.push(obj);
    }
    return out;
  }, [doc, positions, selectedId, measuredMap, fieldRows, alwaysExpand, visibleNodes]);

  const rfEdges: Edge[] = useMemo(() => {
    if (!doc) return [];
    /** 连线颜色 = 外键所在那张表（子表）的配色；没配色就回退模块强调色 */
    const colorOf = (id: string) => {
      const c = doc.nodes.find((n) => n.id === id)?.color ?? "";
      return /^#[0-9a-f]{6}$/i.test(c) ? c : "var(--module-accent)";
    };
    // 只画「两端都在画布上」的关系：被标签筛掉的表，它的连线也该消失
    const onCanvas = new Set(visibleNodes.map((n) => n.id));
    return doc.relations.filter((r) => onCanvas.has(r.from.node) && onCanvas.has(r.to.node)).map((r) => {
      const stroke = colorOf(r.from.node);
      return {
        id: r.id,
        // 画成「父 → 子」：箭头指向多端（外键所在表），与 PowerDesigner 的观感一致
        source: r.to.node,
        target: r.from.node,
        sourceHandle: handleField(r.to.field),
        // 落在子表**对应的外键字段**上（左侧「被引用」锚点），不是整张表的表头入口 ——
        // 复合主键时 A 的 3 个主键各有各的线，各自指向 B 里自己那一列。
        targetHandle: handleFieldIn(r.from.field),
        type: edgeStyle,
        animated: r.kind === "n-n",
        // 不画连线文字：线的两端就落在具体字段上，谁引用谁一眼看得出来；
        // 挂一串「A.b → C.d」反而盖住线，密集时糊成一片。
        style: { stroke, strokeWidth: 1.5 },
        markerEnd: { type: MarkerType.ArrowClosed, color: stroke },
      };
    });
  }, [doc, edgeStyle, visibleNodes]);

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
  const runAi = async () => {
    if (!doc || !aiText.trim()) return;
    setAiBusy(true);
    setAiAborted(false);
    setAiErr("");
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
        patchDoc({ ...partial, nodes: laid });
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
      const generated = await invoke<DbDesignDocument>("dbd_ai_generate", {
        input: {
          text: aiText,
          name: doc.name || undefined,
          dialect: doc.dialect,
          providerId: aiProviderId || null,
          modelId: aiModelId || null,
          runId,
          // 画布有表就把当前设计带上：AI 基于它修改（输出完整更新后的设计）；
          // 是「新建」还是「修改」由模型看需求判断（系统提示里写明了）
          currentDoc: baseDoc ?? undefined,
        },
      });
      const pos = computeLayout(generated, { alwaysExpand, fieldRows });
      const placed: DbDesignDocument = {
        ...generated,
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
      setFitSignal((n) => n + 1);
      // 侧栏保持展开：需求文本还在，用户改一句话就能再生成一轮
      toast(t("dbd.aiDone", { count: placed.nodes.length }), "ok");
    } catch (e) {
      const msg = String(e);
      // 主动停止不算失败：只闪一句提示，不要把「已取消」当错误堆在界面上
      if (msg.includes("已取消")) {
        setAiAborted(true);
        toast(t("dbd.aiCancelled"), "ok");
      } else {
        setAiErr(msg);
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
    const pos = computeLayout(doc, { alwaysExpand, fieldRows });
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

  return (
    <div
      ref={rootRef}
      // 拖分隔线时禁掉文本选中，否则拖过表格会顺带把字段名全刷成选中态
      style={dockDragging ? { userSelect: "none", cursor: "row-resize" } : undefined}
      className="relative flex h-full min-h-0 flex-col text-slate-200"
    >
      {/* 工具栏。relative：AI 面板是挂在工具栏上的浮层，
          少了它浮层会以整个模块为基准（top-full = 模块底部）→ 渲染在可视区外，看着像"没反应"。 */}
      <div className="relative flex flex-wrap items-center gap-1.5 border-b border-white/5 bg-white/[0.02] px-3 py-2">
        <button onClick={() => void newDoc()} className="ui-btn px-2 py-1 text-caption" title={t("dbd.new")}>
          <FilePlus2 className="h-3.5 w-3.5" /> {t("dbd.new")}
        </button>
        <button onClick={() => void openFile()} className="ui-btn px-2 py-1 text-caption" title={t("dbd.open")}>
          <FolderOpen className="h-3.5 w-3.5" /> {t("dbd.open")}
        </button>
        <button onClick={() => void saveFile(false)} disabled={!doc} className="ui-btn px-2 py-1 text-caption disabled:opacity-40" title={t("dbd.save")}>
          <Save className="h-3.5 w-3.5" /> {t("dbd.save")}
        </button>
        <button onClick={() => void saveFile(true)} disabled={!doc} className="ui-btn px-2 py-1 text-caption disabled:opacity-40">
          {t("dbd.saveAs")}
        </button>
        <div className="mx-1 h-4 w-px bg-white/10" />
        <button onClick={() => void exportSql()} disabled={!doc} className="ui-btn px-2 py-1 text-caption disabled:opacity-40" title={t("dbd.exportSql")}>
          <FileDown className="h-3.5 w-3.5" /> {t("dbd.exportSql")}
        </button>
        <button onClick={() => void exportOutline()} disabled={!doc} className="ui-btn px-2 py-1 text-caption disabled:opacity-40">
          <Sparkles className="h-3.5 w-3.5" /> {t("dbd.exportOutline")}
        </button>
        <div className="mx-1 h-4 w-px bg-white/10" />
        <button onClick={() => addNode("table")} disabled={!doc} className="ui-btn px-2 py-1 text-caption disabled:opacity-40">
          <Database className="h-3.5 w-3.5" /> {t("dbd.addTable")}
        </button>
        <button onClick={() => addNode("view")} disabled={!doc} className="ui-btn px-2 py-1 text-caption disabled:opacity-40">
          <Eye className="h-3.5 w-3.5" /> {t("dbd.addView")}
        </button>
        <div className="mx-1 h-4 w-px bg-white/10" />
        <button onClick={() => void autoLayout()} disabled={!doc} className="ui-btn px-2 py-1 text-caption disabled:opacity-40" title={t("dbd.autoLayout")}>
          <LayoutGrid className="h-3.5 w-3.5" /> {t("dbd.autoLayout")}
        </button>
        <button onClick={() => void reverseSqlite()} className="ui-btn px-2 py-1 text-caption" title={t("dbd.reverseSqlite")}>
          <Database className="h-3.5 w-3.5" /> {t("dbd.reverseSqlite")}
        </button>
        <button onClick={() => void reverseDdl()} className="ui-btn px-2 py-1 text-caption" title={t("dbd.reverseDdl")}>
          <FileCode className="h-3.5 w-3.5" /> {t("dbd.reverseDdl")}
        </button>

        {/* AI 助手：给一段需求 → 生成一份设计文档。**只载入画布，不落盘**（用户再自己保存），
            避免 AI 直接覆盖正在画的设计。生成结果没有坐标，载入后立刻跑一次自动布局。
            入口在右侧栏（与思维导图同构），这里只是展开/收起开关；生成中按钮转圈提示。 */}
        <button
          onClick={() => setAiOpen((v) => !v)}
          disabled={!doc}
          className={`ui-btn px-2 py-1 text-caption disabled:opacity-40 ${aiOpen ? "ui-btn-primary" : ""}`}
          title={t("dbd.aiTitle")}
        >
          {aiBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Sparkles className="h-3.5 w-3.5" />}
          {aiBusy ? t("dbd.aiBusy") : t("dbd.aiGenerate")}
        </button>

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
            {/* 节点卡片里显示哪些字段行（常驻展开打开时这个选择就无意义了，置灰） */}
            <select
              value={fieldRows}
              disabled={alwaysExpand}
              onChange={(e) => {
                const next = e.target.value as FieldRowMode;
                setFieldRows(next);
                localStorage.setItem(FIELD_ROWS_KEY, next);
              }}
              title={t("dbd.fieldRows")}
              className="rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none disabled:opacity-40"
            >
              {FIELD_ROW_MODES.map((mode) => (
                <option key={mode} value={mode}>
                  {t(`dbd.fieldRows_${mode}`)}
                </option>
              ))}
            </select>
            <button
              onClick={() => {
                const next = !alwaysExpand;
                setAlwaysExpand(next);
                localStorage.setItem(EXPAND_KEY, next ? "1" : "0");
              }}
              className={`ui-btn px-2 py-1 text-caption ${alwaysExpand ? "ui-btn-primary" : ""}`}
              title={t("dbd.alwaysExpandHint")}
            >
              <Rows3 className="h-3.5 w-3.5" /> {t("dbd.alwaysExpand")}
            </button>
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

      {report && (report.errors.length > 0 || report.warnings.length > 0) ? (
        <div className="border-b border-white/5 px-3 py-1.5 text-micro">
          {report.errors.slice(0, 3).map((e) => (
            <div key={e} className="text-rose-400">✕ {e}</div>
          ))}
          {report.warnings.slice(0, 3).map((w) => (
            <div key={w} className="text-amber-400/90">! {w}</div>
          ))}
        </div>
      ) : null}

      {result ? (
        <div className="px-3 pt-2">
          <ResultNote ok={result.ok} message={result.msg} />
        </div>
      ) : null}

      {/* 左 = 画布（上）+ 对象属性 dock（下）；右 = AI 助手侧栏（思维导图同款右栏）。
          侧栏收起时画布+dock 拿回全宽，展开时左侧让位 —— 画布高度不变。 */}
      <div className="flex min-h-0 flex-1">
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
                onNodeClick={(_, n) => setSelectedId(n.id)}
                onPaneClick={() => setSelectedId(null)}
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
              </ReactFlow>
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
      {aiOpen && doc ? (
        <aside
          className="relative flex-shrink-0 border-l border-white/10 bg-surface-panel/60"
          style={{ width: aiPanelW }}
        >
          <div className="flex h-full min-h-0 flex-col">
            <div className="flex flex-shrink-0 items-center gap-2 border-b border-white/5 px-3 py-2">
              <Sparkles className="h-3.5 w-3.5 flex-shrink-0 text-[var(--module-accent)]" />
              <span className="min-w-0 flex-1 truncate text-caption font-semibold text-slate-200">
                {t("dbd.aiTitle")}
              </span>
              <button
                onClick={() => setAiOpen(false)}
                className="ui-btn shrink-0 p-1"
                title={t("common.dialogClose")}
              >
                ✕
              </button>
            </div>

            <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-y-auto p-3">
              <p className="text-micro leading-relaxed text-slate-500">{t("dbd.aiHint")}</p>
              {/* 画布已有设计时提示：这不只是生成器，还能直接改当前设计 */}
              {doc && doc.nodes.length > 0 ? (
                <p className="text-micro leading-relaxed text-slate-500">
                  {t("dbd.aiModifyHint", { count: doc.nodes.length })}
                </p>
              ) : null}
              <textarea
                value={aiText}
                onChange={(e) => setAiText(e.target.value)}
                rows={6}
                placeholder={t("dbd.aiPh")}
                className="w-full resize-none rounded-ctl bg-black/30 px-2 py-1.5 text-caption text-slate-200 outline-none placeholder:text-slate-600"
              />
              {/* 模型选择器用 AI 模块的共享组件（与思维导图 Agent、API 智能导入同一套 UI） */}
              <div className="flex items-center gap-2">
                <span className="shrink-0 text-micro text-slate-500">{t("dbd.aiModel")}</span>
                <ModelSelector
                  providers={aiProviders as unknown as ProviderLike[]}
                  providerId={aiProviderId}
                  modelId={aiModelId}
                  onSelect={(pid, mid) => {
                    setAiProviderId(pid);
                    setAiModelId(mid);
                  }}
                  disabled={aiBusy}
                  compact
                  emptyText={t("dbd.aiNoProvider")}
                />
              </div>

              {/* AI 设计控制台（与思维导图智能体控制台同款）：思考过程 / 输出进度 / 实时预览，
                  长生成时一眼看清「它现在在干什么」；跑完后保留摘要可回看。
                  画布上同时能看到表一张张长出来（流式落图）。 */}
              {aiBusy || aiProgress.length > 0 ? (
                <div className="overflow-hidden rounded-ctl border border-white/10 bg-slate-950/50">
                  <div className="flex items-center gap-1.5 border-b border-white/5 px-2 py-1.5">
                    <Brain className={`h-3 w-3 ${aiBusy ? "animate-pulse text-cyan-300" : "text-slate-500"}`} />
                    <span className="text-micro font-semibold uppercase tracking-wide text-slate-400">{t("dbd.aiConsole")}</span>
                    <span className={`ml-auto text-micro ${aiBusy ? "text-cyan-300" : aiAborted ? "text-amber-300" : "text-emerald-300"}`}>
                      {aiBusy ? t("dbd.aiWorking") : aiAborted ? t("dbd.aiCancelled") : t("dbd.aiDoneShort")}
                    </span>
                  </div>
                  <div className="space-y-1 p-2">
                    <div className="flex items-center gap-1.5 text-micro text-slate-300">
                      <Sparkles className="h-3 w-3 shrink-0 text-cyan-300" />
                      {t("dbd.aiStart")}
                    </div>
                    {/* 思考过程：推理模型先想再写，思考块单独一行（结束时定格总字数） */}
                    {aiThinkFinal ? (
                      <div className="flex items-center gap-1.5 text-micro text-fuchsia-200/90">
                        <Brain className={`h-3 w-3 shrink-0 text-fuchsia-300 ${aiBusy && !aiThinkFinal.done ? "animate-pulse" : ""}`} />
                        {aiThinkFinal.done
                          ? t("dbd.aiThought", { count: aiThinkFinal.length })
                          : t("dbd.aiThinkingChars", { count: aiThinkFinal.length })}
                      </div>
                    ) : null}
                    {/* 正文输出：累计字数（运行中实时跳，结束后定格） */}
                    {aiStream ? (
                      <div className="flex items-center gap-1.5 text-micro text-emerald-200/90">
                        <Terminal className="h-3 w-3 shrink-0 text-emerald-300" />
                        {t("dbd.aiWritingChars", { count: aiStream.length })}
                      </div>
                    ) : null}
                    {/* 实时预览：正文优先（emerald），否则思考尾部（fuchsia）—— 光标闪烁表示还在动 */}
                    {aiBusy && (aiStream || aiReasoning) ? (
                      <div
                        className={`max-h-20 overflow-y-auto whitespace-pre-wrap rounded border px-1.5 py-1 font-mono text-[8px] leading-3.5 ${
                          aiStream
                            ? "border-emerald-400/15 bg-emerald-400/[0.03] text-emerald-100/80"
                            : "border-fuchsia-400/15 bg-fuchsia-400/[0.03] text-fuchsia-100/80"
                        }`}
                      >
                        {(aiStream ?? aiReasoning)!.text || "…"}
                        <span className={`ml-0.5 inline-block h-2 w-1 animate-pulse align-middle ${aiStream ? "bg-emerald-300" : "bg-fuchsia-300"}`} />
                      </div>
                    ) : null}
                    {/* 刚启动还没收到任何帧：转圈占位，别让用户以为卡死 */}
                    {aiBusy && !aiReasoning && !aiStream ? (
                      <div className="flex items-center gap-1.5 pl-1">
                        <Loader2 className="h-3 w-3 animate-spin text-slate-400" />
                        <span className="text-micro text-slate-500">{t("dbd.aiThinking")}</span>
                      </div>
                    ) : null}
                    {!aiBusy && aiStream && !aiAborted ? (
                      <div className="flex items-center gap-1.5 text-micro text-emerald-300">
                        <Check className="h-3 w-3 shrink-0" />
                        {t("dbd.aiFinished")}
                      </div>
                    ) : null}
                  </div>
                </div>
              ) : null}
              {aiErr ? <p className="break-words text-micro text-rose-400">{aiErr}</p> : null}
            </div>

            <div className="flex flex-shrink-0 items-center gap-2 border-t border-white/5 px-3 py-2">
              <button
                onClick={() => void runAi()}
                disabled={aiBusy || !aiText.trim() || aiProviders.length === 0}
                className="ui-btn ui-btn-primary px-3 py-1 text-caption disabled:opacity-40"
              >
                {aiBusy ? t("dbd.aiBusy") : t("dbd.aiGenerate")}
              </button>
              {aiBusy ? (
                <button onClick={() => void cancelAi()} className="ui-btn px-2 py-1 text-caption">
                  <Ban className="h-3 w-3" /> {t("dbd.aiCancel")}
                </button>
              ) : null}
              {!aiBusy && aiProviders.length === 0 ? (
                <span className="min-w-0 truncate text-micro text-amber-400">{t("dbd.aiNoProvider")}</span>
              ) : null}
            </div>
          </div>
          {/* 左缘拖宽把手：与思维导图右栏同一套逻辑（拖动方向相反） */}
          <div
            className="absolute -left-1 top-0 z-10 flex h-full w-2.5 cursor-col-resize items-center justify-center hover:bg-white/[0.06]"
            onMouseDown={(e) => {
              if (e.button !== 0) return;
              e.preventDefault();
              const startX = e.clientX;
              const startW = aiPanelW;
              const onMove = (ev: MouseEvent) => {
                setAiPanelW(startW - (ev.clientX - startX));
              };
              const onUp = () => {
                window.removeEventListener("mousemove", onMove);
                window.removeEventListener("mouseup", onUp);
              };
              window.addEventListener("mousemove", onMove);
              window.addEventListener("mouseup", onUp);
            }}
          />
        </aside>
      ) : null}
      </div>
    </div>
  );
}
