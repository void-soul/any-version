import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "../shared/Toast";
import { type DbDesignDocument, type DbDesignNode, type DbDesignRelation, type DbField, type DbIndex } from "./types";
import { emptyField, newId } from "./DbDesignerPanel.model";
import type { Dispatch, SetStateAction } from "react";

/** Owns the DbDocumentEditing feature, independently of page rendering. */
export function useDbDocumentEditing({ doc, patchDoc, setSelectedId, setHighlightId, selectedId, highlightId, resetAiConversation, selected }: {
  doc: DbDesignDocument | null;
  patchDoc: (next: DbDesignDocument) => void;
  setSelectedId: Dispatch<SetStateAction<string | null>>;
  setHighlightId: Dispatch<SetStateAction<string | null>>;
  selectedId: string | null;
  highlightId: string | null;
  resetAiConversation: () => void;
  selected: DbDesignNode | null;
}) {
  const { t } = useTranslation();

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
    resetAiConversation();
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
  return {
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
  };
}
