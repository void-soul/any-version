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
  Controls,
  MarkerType,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  type Connection,
  type Edge,
  type Node,
  type NodeChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { CONNECTION_LINE_BY_STYLE, useEdgeStyle } from "../../utils/graphEdgeStyle";
import EdgeStyleSelect from "../shared/EdgeStyleSelect";
import {
  Database,
  Eye,
  FileCode,
  FileDown,
  FilePlus2,
  FolderOpen,
  Plus,
  Save,
  Sparkles,
  Trash2,
} from "lucide-react";

import DesignNodeCard, {
  HANDLE_TABLE,
  handleAllPk,
  handleField,
  type DesignNodeData,
} from "./DesignNodeCard";
import { ResultNote } from "../shared/Note";
import { theamedConfirm } from "../shared/ThemedAlert";
import { toast } from "../shared/Toast";
import {
  BASE_TYPES,
  DIALECTS,
  relationFieldsOf,
  typeLabel,
  type DbDesignDocument,
  type DbDesignNode,
  type DbDesignRelation,
  type DbField,
  type DbLogicalType,
  type DbTableBody,
  type Dialect,
  type ValidationReport,
} from "./types";

// nodeTypes 必须是组件外的常量：内联新建会让 React Flow 每次渲染都重挂载节点（官方文档警告的卡顿源）。
const nodeTypes = { designNode: DesignNodeCard };

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
  const [hoverId, setHoverId] = useState<string | null>(null);
  /** 连线样式（曲线 / 直角折线 / 直线）：与思维导图、JSON 图共用一份偏好 */
  const [edgeStyle, setEdgeStyle] = useEdgeStyle();
  const [report, setReport] = useState<ValidationReport | null>(null);
  const [result, setResult] = useState<{ ok: boolean; msg: string } | null>(null);

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

  // ── 字段 ──
  const updateField = (nodeId: string, index: number, patch: Partial<DbField>) => {
    if (!doc) return;
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        const fields = n.table.fields.map((f, i) => (i === index ? { ...f, ...patch } : f));
        return { ...n, table: { ...n.table, fields } };
      }),
    });
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
    patchDoc({
      ...doc,
      nodes: doc.nodes.map((n) => {
        if (n.id !== nodeId || !n.table) return n;
        return { ...n, table: { ...n.table, fields: n.table.fields.filter((f) => f.name !== name) } };
      }),
      relations: doc.relations.filter(
        (r) => !((r.from.node === nodeId && r.from.field === name) || (r.to.node === nodeId && r.to.field === name)),
      ),
    });
  };

  /** 改名走后端命令：它会级联更新关联与索引（前端不自己改，避免漏掉连线） */
  const renameField = async (nodeId: string, oldName: string, newName: string) => {
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
  const addRelation = () => {
    if (!doc) return;
    const tables = doc.nodes.filter((n) => n.kind === "table");
    if (tables.length < 1) return;
    const first = tables[0];
    const second = tables[1] ?? first;
    const rel = {
      id: newId("r"),
      name: "",
      from: { node: first.id, field: first.table?.fields[0]?.name ?? "" },
      to: { node: second.id, field: second.table?.fields[0]?.name ?? "" },
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

  const removeRelation = (id: string) => {
    if (!doc) return;
    patchDoc({ ...doc, relations: doc.relations.filter((r) => r.id !== id) });
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

  const rfNodes: Node[] = useMemo(() => {
    if (!doc) return [];
    const cache = nodeObjCache.current;
    const out: Node[] = [];
    for (const n of doc.nodes) {
      const p = positions[n.id] ?? { x: n.x ?? 0, y: n.y ?? 0 };
      const selected = selectedId === n.id;
      const expanded = hoverId === n.id || selected;
      const measured = measuredMap[n.id];
      const prev = cache.get(n.id);
      // 缓存键：文档引用 + 节点引用 + 坐标 + 选中/展开态 + 尺寸。
      // 纯拖动时 doc / node / selected / expanded 都没变 → 复用旧对象，memo 直接跳过。
      if (prev && prev.doc === doc && prev.node === n && prev.px === p.x && prev.py === p.y) {
        out.push(prev.obj);
        continue;
      }
      const prevObj = prev?.obj;
      const prevData = prevObj?.data as DesignNodeData | undefined;
      const dataChanged =
        !prevData ||
        prev!.doc !== doc ||
        prev!.node !== n ||
        (prevData.selected !== selected) ||
        (prevData.expanded !== expanded);
      const data = (dataChanged
        ? {
            node: n,
            relationFields: relationFieldsOf(doc, n.id),
            expanded,
            selected,
          }
        : prevData) as unknown as Record<string, unknown>;
      const obj: Node = prevObj
        ? { ...prevObj, position: p, data, measured: measured ?? prevObj.measured }
        : { id: n.id, type: "designNode", position: p, data, measured };
      cache.set(n.id, { obj, px: p.x, py: p.y, doc, node: n });
      out.push(obj);
    }
    return out;
  }, [doc, positions, hoverId, selectedId, measuredMap]);

  const rfEdges: Edge[] = useMemo(() => {
    if (!doc) return [];
    const nameOf = (id: string) => doc.nodes.find((n) => n.id === id)?.name ?? "?";
    return doc.relations.map((r) => ({
      id: r.id,
      // 画成「父 → 子」：箭头指向多端（外键所在表），与 PowerDesigner 的观感一致
      source: r.to.node,
      target: r.from.node,
      sourceHandle: handleField(r.to.field),
      targetHandle: HANDLE_TABLE,
      type: edgeStyle,
      animated: r.kind === "n-n",
      label: `${nameOf(r.from.node)}.${r.from.field} → ${nameOf(r.to.node)}.${r.to.field}`,
      labelStyle: { fontSize: 9 },
      style: { stroke: "var(--module-accent)", strokeWidth: 1.5 },
      markerEnd: { type: MarkerType.ArrowClosed, color: "var(--module-accent)" },
    }));
  }, [doc, edgeStyle]);

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
      const parentId = conn.source;
      const childId = conn.target;
      if (!parentId || !childId || parentId === childId) return;
      const parent = doc.nodes.find((n) => n.id === parentId);
      const child = doc.nodes.find((n) => n.id === childId);
      if (!parent?.table || !child?.table || parent.kind !== "table" || child.kind !== "table") {
        toast(t("dbd.connectNeedTables"), "err");
        return;
      }
      const handle = conn.sourceHandle ?? "";
      // 整表锚点 → 全部主键；字段锚点 → 该字段
      const wanted: string[] =
        handle === handleAllPk || handle === HANDLE_TABLE
          ? parent.table.fields.filter((f) => f.pk).map((f) => f.name)
          : handle.startsWith("f:")
            ? [handle.slice(2)]
            : [];
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

  const selected = doc?.nodes.find((n) => n.id === selectedId) ?? null;

  return (
    <div className="flex h-full min-h-0 flex-col text-slate-200">
      {/* 工具栏 */}
      <div className="flex flex-wrap items-center gap-1.5 border-b border-white/5 bg-white/[0.02] px-3 py-2">
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
        <button onClick={() => void reverseSqlite()} className="ui-btn px-2 py-1 text-caption" title={t("dbd.reverseSqlite")}>
          <Database className="h-3.5 w-3.5" /> {t("dbd.reverseSqlite")}
        </button>
        <button onClick={() => void reverseDdl()} className="ui-btn px-2 py-1 text-caption" title={t("dbd.reverseDdl")}>
          <FileCode className="h-3.5 w-3.5" /> {t("dbd.reverseDdl")}
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
            <span className="text-micro text-slate-600">
              {dirty ? t("dbd.unsaved") : path ? t("dbd.savedShort") : ""}
            </span>
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

      {/* 画布 + 检查器 */}
      <div className="flex min-h-0 flex-1">
        {/* React Flow 必须有确定高度的容器，否则画布高度为 0 */}
        <div className="h-full min-w-0 flex-1">
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
                onNodeMouseEnter={(_, n) => setHoverId(n.id)}
                onNodeMouseLeave={() => setHoverId(null)}
                onPaneClick={() => setSelectedId(null)}
                fitView
                minZoom={0.2}
                proOptions={{ hideAttribution: true }}
              >
                <Background gap={16} size={1} color="rgba(148,163,184,0.15)" />
                <Controls showInteractive={false} />
                <MiniMap pannable zoomable className="!bg-slate-900" />
              </ReactFlow>
            </ReactFlowProvider>
          ) : (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-slate-500">
              <Database className="h-10 w-10 opacity-30" />
              <p className="text-body">{t("dbd.emptyTitle")}</p>
              <p className="max-w-sm text-center text-caption text-slate-600">{t("dbd.emptyHint")}</p>
              <button onClick={() => void newDoc()} className="ui-btn-primary mt-1 px-3 py-1.5 text-caption">
                <FilePlus2 className="h-3.5 w-3.5" /> {t("dbd.new")}
              </button>
            </div>
          )}
        </div>

        {/* 检查器 */}
        {doc && selected ? (
          <div className="w-[320px] flex-shrink-0 overflow-y-auto border-l border-white/5 bg-white/[0.02] p-3">
            <div className="mb-2 flex items-center gap-1.5">
              <input
                value={selected.name}
                onChange={(e) => updateNode(selected.id, { name: e.target.value })}
                className="min-w-0 flex-1 rounded-ctl bg-white/5 px-2 py-1 text-caption font-semibold text-slate-100 outline-none"
              />
              <button onClick={() => deleteNode(selected.id)} className="ui-btn p-1" title={t("common.delete")}>
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
            <input
              value={selected.comment ?? ""}
              onChange={(e) => updateNode(selected.id, { comment: e.target.value })}
              placeholder={t("dbd.comment")}
              className="mb-2 w-full rounded-ctl bg-white/5 px-2 py-1 text-caption text-slate-200 outline-none"
            />

            {selected.kind === "view" ? (
              <textarea
                value={selected.view?.sql ?? ""}
                onChange={(e) => updateNode(selected.id, { view: { sql: e.target.value } })}
                rows={6}
                className="w-full rounded-ctl bg-black/30 px-2 py-1.5 font-mono text-micro text-slate-200 outline-none"
              />
            ) : (
              <div className="space-y-1.5">
                <div className="flex items-center justify-between">
                  <span className="text-micro font-semibold text-slate-400">{t("dbd.fields")}</span>
                  <button onClick={() => addField(selected.id)} className="ui-btn px-1.5 py-0.5 text-micro">
                    <Plus className="h-3 w-3" /> {t("dbd.addField")}
                  </button>
                </div>
                {(selected.table?.fields ?? []).map((f, i) => (
                  <div key={`${f.name}-${i}`} className="rounded-ctl border border-white/5 bg-black/20 p-1.5">
                    <div className="flex items-center gap-1">
                      <input
                        defaultValue={f.name}
                        onBlur={(e) => void renameField(selected.id, f.name, e.target.value)}
                        className="min-w-0 flex-1 bg-transparent text-caption text-slate-100 outline-none"
                      />
                      <select
                        value={f.type.base}
                        onChange={(e) => updateField(selected.id, i, { type: { ...f.type, base: e.target.value } })}
                        className="rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                      >
                        {BASE_TYPES.map((b) => (
                          <option key={b} value={b}>{b}</option>
                        ))}
                      </select>
                      <button onClick={() => removeField(selected.id, f.name)} className="ui-btn p-0.5" title={t("common.delete")}>
                        <Trash2 className="h-3 w-3" />
                      </button>
                    </div>
                    <div className="mt-1 flex flex-wrap items-center gap-1 text-micro text-slate-500">
                      {(f.type.base === "varchar" || f.type.base === "char") ? (
                        <input
                          type="number"
                          value={f.type.length ?? 255}
                          onChange={(e) => updateField(selected.id, i, { type: { ...f.type, length: Number(e.target.value) } })}
                          className="w-14 rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                          title={t("dbd.length")}
                        />
                      ) : null}
                      {f.type.base === "decimal" ? (
                        <>
                          <input
                            type="number"
                            value={f.type.precision ?? 10}
                            onChange={(e) => updateField(selected.id, i, { type: { ...f.type, precision: Number(e.target.value) } })}
                            className="w-12 rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                            title={t("dbd.precision")}
                          />
                          <input
                            type="number"
                            value={f.type.scale ?? 2}
                            onChange={(e) => updateField(selected.id, i, { type: { ...f.type, scale: Number(e.target.value) } })}
                            className="w-10 rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                            title={t("dbd.scale")}
                          />
                        </>
                      ) : null}
                      {f.type.base === "enum" ? (
                        <input
                          value={(f.type.values ?? []).join(",")}
                          onChange={(e) =>
                            updateField(selected.id, i, {
                              type: { ...f.type, values: e.target.value.split(",").map((s) => s.trim()).filter(Boolean) },
                            })
                          }
                          className="w-24 rounded bg-white/5 px-1 py-0.5 text-micro text-slate-200 outline-none"
                          placeholder="a,b,c"
                        />
                      ) : null}
                      <label className="cursor-pointer"><input type="checkbox" checked={!!f.pk} onChange={(e) => updateField(selected.id, i, { pk: e.target.checked })} /> PK</label>
                      <label className="cursor-pointer"><input type="checkbox" checked={!!f.autoIncrement} onChange={(e) => updateField(selected.id, i, { autoIncrement: e.target.checked })} /> AI</label>
                      <label className="cursor-pointer"><input type="checkbox" checked={!!f.unique} onChange={(e) => updateField(selected.id, i, { unique: e.target.checked })} /> UQ</label>
                      <label className="cursor-pointer"><input type="checkbox" checked={!!f.nullable} onChange={(e) => updateField(selected.id, i, { nullable: e.target.checked })} /> NULL</label>
                    </div>
                    <input
                      value={f.comment ?? ""}
                      onChange={(e) => updateField(selected.id, i, { comment: e.target.value })}
                      placeholder={t("dbd.comment")}
                      className="mt-1 w-full bg-transparent text-micro text-slate-400 outline-none"
                    />
                    <div className="mt-0.5 text-micro text-slate-700">{typeLabel(f.type as DbLogicalType)}</div>
                  </div>
                ))}
              </div>
            )}

            {/* 关联 */}
            <div className="mt-3 space-y-1.5">
              <div className="flex items-center justify-between">
                <span className="text-micro font-semibold text-slate-400">{t("dbd.relations")}</span>
                <button onClick={addRelation} className="ui-btn px-1.5 py-0.5 text-micro">
                  <Plus className="h-3 w-3" /> {t("dbd.addRelation")}
                </button>
              </div>
              {/* 建关联的主入口是「拖线」，这里是补录 / 改属性；提示要说清楚 */}
              <p className="text-micro text-slate-600">{t("dbd.connectHint")}</p>
              {doc.relations.length === 0 ? (
                <div className="text-micro text-slate-600">{t("dbd.noRelations")}</div>
              ) : null}
              {doc.relations.map((r) => (
                <div key={r.id} className="rounded-ctl border border-white/5 bg-black/20 p-1.5 text-micro">
                  <div className="flex items-center gap-1">
                    <select
                      value={r.from.node}
                      onChange={(e) => updateRelation(r.id, { from: { ...r.from, node: e.target.value } })}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-slate-200 outline-none"
                    >
                      {doc.nodes.filter((n) => n.kind === "table").map((n) => (
                        <option key={n.id} value={n.id}>{n.name}</option>
                      ))}
                    </select>
                    <select
                      value={r.from.field}
                      onChange={(e) => updateRelation(r.id, { from: { ...r.from, field: e.target.value } })}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-slate-200 outline-none"
                    >
                      {(doc.nodes.find((n) => n.id === r.from.node)?.table?.fields ?? []).map((f) => (
                        <option key={f.name} value={f.name}>{f.name}</option>
                      ))}
                    </select>
                  </div>
                  <div className="my-0.5 text-center text-slate-600">↓</div>
                  <div className="flex items-center gap-1">
                    <select
                      value={r.to.node}
                      onChange={(e) => updateRelation(r.id, { to: { ...r.to, node: e.target.value } })}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-slate-200 outline-none"
                    >
                      {doc.nodes.filter((n) => n.kind === "table").map((n) => (
                        <option key={n.id} value={n.id}>{n.name}</option>
                      ))}
                    </select>
                    <select
                      value={r.to.field}
                      onChange={(e) => updateRelation(r.id, { to: { ...r.to, field: e.target.value } })}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-slate-200 outline-none"
                    >
                      {(doc.nodes.find((n) => n.id === r.to.node)?.table?.fields ?? []).map((f) => (
                        <option key={f.name} value={f.name}>{f.name}</option>
                      ))}
                    </select>
                  </div>
                  <div className="mt-1 flex items-center gap-1">
                    <select
                      value={r.kind}
                      onChange={(e) => updateRelation(r.id, { kind: e.target.value as DbDesignRelation["kind"] })}
                      className="rounded bg-white/5 px-1 py-0.5 text-slate-200 outline-none"
                    >
                      {REL_KINDS.map((k) => (
                        <option key={k} value={k}>{k}</option>
                      ))}
                    </select>
                    <select
                      value={r.onDelete ?? "RESTRICT"}
                      onChange={(e) => updateRelation(r.id, { onDelete: e.target.value })}
                      className="min-w-0 flex-1 rounded bg-white/5 px-1 py-0.5 text-slate-200 outline-none"
                      title={t("dbd.onDelete")}
                    >
                      {FK_ACTIONS.map((a) => (
                        <option key={a} value={a}>{a}</option>
                      ))}
                    </select>
                    <button onClick={() => removeRelation(r.id)} className="ui-btn p-0.5" title={t("common.delete")}>
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
