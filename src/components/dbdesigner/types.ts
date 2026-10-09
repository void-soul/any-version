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
