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
