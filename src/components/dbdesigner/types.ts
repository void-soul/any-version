// 与 Rust `commands/db_designer/models.rs` 一一对应的前端类型（后端 serde camelCase）。
// 新增/改名字段必须两边同步，否则前端读到 undefined（静默失效）。
export type Dialect = "mysql" | "postgres" | "sqlite";

export const DIALECTS: Dialect[] = ["mysql", "postgres", "sqlite"];

/** 逻辑类型白名单（与后端 BASE_TYPES 一致） */
export const BASE_TYPES = [
  "int", "bigint", "smallint", "tinyint", "decimal", "float", "double", "char", "varchar",
  "text", "date", "time", "datetime", "timestamp", "boolean", "json", "uuid", "blob", "enum",
];

export interface DbLogicalType {
  base: string;
  length?: number | null;
  precision?: number | null;
  scale?: number | null;
  unsigned?: boolean;
  values?: string[];
}

export interface DbField {
  name: string;
  type: DbLogicalType;
  nullable?: boolean;
  default?: string | null;
  comment?: string;
  pk?: boolean;
  autoIncrement?: boolean;
  unique?: boolean;
}

export interface DbIndex {
  name: string;
  kind: "primary" | "unique" | "index" | "fulltext";
  fields: string[];
}

export interface DbTableBody {
  fields: DbField[];
  indexes?: DbIndex[];
}

export interface DbViewBody {
  sql: string;
}

export interface DbDesignNode {
  id: string;
  kind: "table" | "view";
  name: string;
  comment?: string;
  /** 标签：自由文本，画布上按标签筛选 */
  tags?: string[];
  /** 主题色 `#rrggbb`：节点与它的连线都用这个颜色 */
  color?: string;
  x?: number;
  y?: number;
  table?: DbTableBody | null;
  view?: DbViewBody | null;
}

export interface DbRelationEnd {
  /** 节点 id */
  node: string;
  /** 字段名（不是字段 id） */
  field: string;
}

export interface DbDesignRelation {
  id: string;
  name?: string;
  from: DbRelationEnd;
  to: DbRelationEnd;
  kind: "1-1" | "1-n" | "n-n";
  onDelete?: string;
  onUpdate?: string;
  /**
   * true = 这条关系是「镜像父表主键」拖出来的，from 那一列是我们复制过去的副本。
   * 之后父表加减主键，子表跟着变（后端 dbd_sync_relations 负责同步）。
   * 字段级手动拖的关系为 false —— 父表后来加主键不该自动推给子表。
   */
  mirror?: boolean;
}

export interface DbDesignDocument {
  id: string;
  name: string;
  description?: string;
  dialect: Dialect;
  /**
   * 公共字段：导出时自动附加到每张表末尾；表里已有同名字段时以表自己的为准
   * （比对时忽略大小写与下划线 —— MySQL 列名不区分大小写，两个都写会建表失败）。
   *
   * 存的是**完整定义**（自包含：任何工具都能读懂这份设计往每张表加了什么）。
   * 缺省 / 空数组 = 不附加，旧文件与不配的设计完全不受影响。
   */
  commonFields?: DbField[];
  nodes: DbDesignNode[];
  relations: DbDesignRelation[];
  updatedAt?: string;
}

export interface ValidationReport {
  errors: string[];
  warnings: string[];
}

/** 逻辑类型的展示名（含参数） */
export function typeLabel(t: DbLogicalType): string {
  switch (t.base) {
    case "varchar":
    case "char":
      return `${t.base}(${t.length ?? 255})`;
    case "decimal":
      return `decimal(${t.precision ?? 10},${t.scale ?? 2})`;
    case "enum":
      return `enum(${t.values?.join("|") ?? ""})`;
    default:
      return t.base;
  }
}

/** 参与关联的字段名（折叠态只显示这些） */
export function relationFieldsOf(doc: DbDesignDocument, nodeId: string): string[] {
  const out: string[] = [];
  for (const r of doc.relations) {
    for (const end of [r.from, r.to]) {
      if (end.node === nodeId && !out.includes(end.field)) out.push(end.field);
    }
  }
  return out;
}

/** 节点卡片里显示哪些字段行：只主键+外键，还是全部字段。 */
export type FieldRowMode = "keys" | "all";
export const FIELD_ROW_MODES: FieldRowMode[] = ["keys", "all"];

/** 某张表里当外键用的字段（在关系里位于「多」端，即 from 那一侧）。 */
export function fkFieldsOf(doc: DbDesignDocument, nodeId: string): string[] {
  const out: string[] = [];
  for (const r of doc.relations) {
    if (r.from.node === nodeId && !out.includes(r.from.field)) out.push(r.from.field);
  }
  return out;
}

// ── 连线锚点（PowerDesigner 式）──
//
// 放在这里而不是组件文件里：锚点 id 的规则是**纯逻辑**（要能测），
// 而 DesignNodeCard 只是照着这些 id 渲染 <Handle>。

/** 右侧（引用别人）字段锚点 */
export const handleField = (field: string) => `f:${field}`;
/**
 * 左侧（被引用侧）字段锚点。
 *
 * 为什么要和右侧分开命名：为了让**同一个锚点既能接、也能拖**（用户要求左右双向等价），
 * React Flow 需要 `connectionMode="loose"`；而 loose 模式下锚点不再按声明类型区分，
 * id 必须左右各一个，否则同名字段两个锚点会被当成同一个。
 * 面板里靠这个前缀判断「拖线是从哪一侧开始的」，见 DbDesignerPanel.onConnect。
 */
export const handleFieldIn = (field: string) => `b:${field}`;
/** 表级锚点：连到「表」而不是具体某列 */
export const HANDLE_TABLE = "table";
export const HANDLE_TABLE_IN = "table-in";
/** 拖出它 = 复制本表全部主键并建关联（复合主键用这个） */
export const handleAllPk = "pk-all";

export interface EdgeAnchors {
  sourceHandle: string;
  targetHandle: string;
  /** 字段锚点不存在、已回退到表级锚点 —— 关联指向的字段在这张表里没有 */
  degraded: boolean;
}

/**
 * 一条关联在画布上两端的锚点。
 *
 * **为什么要回退**：React Flow 找不到 `sourceHandle` / `targetHandle` 对应的锚点时
 * 会**静默不画这条边**（只在 devtools 里 warn 一句）。用户看到的是「关联明明存在、
 * 表也明明高亮着，那条线就是不见了」，而且完全无法定位 —— 实际原因是关联指向的
 * 字段名在这张表里找不到（手改过字段、或手写的设计文件对不上）。
 *
 * 所以这里显式判定：字段在表里就用字段锚点（复合主键时各连各的列），
 * 不在就退回表级锚点 —— **宁可连到表头，也不要整条线凭空消失**。
 * 字段真的丢了由 `validate` 报错（那是 error 级，会阻断导出），画布这边只负责「看得见」。
 */
export function edgeAnchors(doc: DbDesignDocument, rel: DbDesignRelation): EdgeAnchors {
  const hasField = (nodeId: string, field: string): boolean =>
    (doc.nodes.find((n) => n.id === nodeId)?.table?.fields ?? []).some((f) => f.name === field);
  // 方向约定：source = 被引用端（父，to），target = 外键所在端（子，from）
  const parentOk = hasField(rel.to.node, rel.to.field);
  const childOk = hasField(rel.from.node, rel.from.field);
  return {
    sourceHandle: parentOk ? handleField(rel.to.field) : HANDLE_TABLE,
    targetHandle: childOk ? handleFieldIn(rel.from.field) : HANDLE_TABLE_IN,
    degraded: !parentOk || !childOk,
  };
}
