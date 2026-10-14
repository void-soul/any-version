import { describe, expect, it } from "vitest";

import {
  HANDLE_TABLE,
  HANDLE_TABLE_IN,
  edgeAnchors,
  handleField,
  handleFieldIn,
  type DbDesignDocument,
  type DbDesignNode,
  type DbDesignRelation,
} from "../types";

function node(id: string, name: string, fields: string[]): DbDesignNode {
  return {
    id,
    kind: "table",
    name,
    comment: "",
    tags: [],
    color: "",
    x: 0,
    y: 0,
    table: {
      fields: fields.map((f) => ({
        name: f,
        type: { base: "bigint", length: null, precision: null, scale: null, unsigned: false, values: [] },
        nullable: false,
        default: null,
        comment: "",
        pk: false,
        autoIncrement: false,
        unique: false,
      })),
      indexes: [],
    },
    view: null,
  } as unknown as DbDesignNode;
}

function rel(fromNode: string, fromField: string, toNode: string, toField: string): DbDesignRelation {
  return {
    id: "r1",
    name: "",
    from: { node: fromNode, field: fromField },
    to: { node: toNode, field: toField },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  } as unknown as DbDesignRelation;
}

function doc(nodes: DbDesignNode[], relations: DbDesignRelation[]): DbDesignDocument {
  return {
    id: "d",
    name: "d",
    description: "",
    dialect: "mysql",
    nodes,
    relations,
  } as unknown as DbDesignDocument;
}

describe("edgeAnchors", () => {
  it("字段都在时用字段锚点（复合主键才能各连各的列）", () => {
    const d = doc(
      [node("t1", "child", ["id", "parent_id"]), node("t2", "parent", ["id", "code"])],
      [rel("t1", "parent_id", "t2", "id")],
    );
    const a = edgeAnchors(d, d.relations[0]);
    expect(a.sourceHandle).toBe(handleField("id"));
    expect(a.targetHandle).toBe(handleFieldIn("parent_id"));
    expect(a.degraded).toBe(false);
  });

  /**
   * 回归：字段锚点不存在时 React Flow 会**静默不画这条边**，用户只看到
   * 「关联存在、表也高亮，那条线就是不见了」。所以必须回退到表级锚点。
   */
  it("字段缺失时回退表级锚点，宁可连到表头也不整条线消失", () => {
    const d = doc(
      [node("t1", "child", ["id"]), node("t2", "parent", ["id"])],
      [rel("t1", "ghost_fk", "t2", "id")],
    );
    const a = edgeAnchors(d, d.relations[0]);
    expect(a.targetHandle).toBe(HANDLE_TABLE_IN);
    expect(a.sourceHandle).toBe(handleField("id"));
    expect(a.degraded).toBe(true);
  });

  it("父端字段缺失也回退（两端都缺时两端都是表级锚点）", () => {
    const d = doc(
      [node("t1", "child", ["id"]), node("t2", "parent", ["id"])],
      [rel("t1", "nope", "t2", "also_nope")],
    );
    const a = edgeAnchors(d, d.relations[0]);
    expect(a.sourceHandle).toBe(HANDLE_TABLE);
    expect(a.targetHandle).toBe(HANDLE_TABLE_IN);
    expect(a.degraded).toBe(true);
  });

  it("节点本身不存在也不能抛（画布上少一张表时连线仍要能画）", () => {
    const d = doc([node("t1", "child", ["id"])], [rel("t1", "id", "t_missing", "id")]);
    const a = edgeAnchors(d, d.relations[0]);
    expect(a.sourceHandle).toBe(HANDLE_TABLE);
    expect(a.targetHandle).toBe(handleFieldIn("id"));
  });

  /**
   * 真实数据回归：滑雪助手抖音小程序里的 `product` → `product_category` 那条关联
   * （r16，两端字段都真实存在）。用户报「这张表明明和 product 有关系，线却不见了」——
   * 排查后确认线是画了的，是弱化档透明度 0.18 + 1px 在深色底上等于隐形。
   * 这里钉住「字段都在 → 必须用字段锚点」，配合样式层的透明度调整共同防回归。
   */
  it("父子两表字段都在时用字段锚点（真实数据：product → product_category）", () => {
    const d = doc(
      [
        node("t_product", "product", ["product_id", "category_id", "product_name"]),
        node("t_category", "product_category", ["product_category_id", "category_name"]),
      ],
      [rel("t_product", "category_id", "t_category", "product_category_id")],
    );
    const a = edgeAnchors(d, d.relations[0]);
    expect(a.degraded).toBe(false);
    expect(a.sourceHandle).toBe(handleField("product_category_id"));
    expect(a.targetHandle).toBe(handleFieldIn("category_id"));
  });
});