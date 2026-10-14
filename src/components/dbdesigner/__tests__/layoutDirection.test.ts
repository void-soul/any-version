import { describe, expect, it } from "vitest";
import { computeLayout } from "../layout";
import { groupByForeignKey } from "../series";
import type { DbDesignDocument, DbDesignNode } from "../types";

function table(id: string, name: string, fields: string[]): DbDesignNode {
  return {
    id,
    name,
    kind: "table",
    comment: "",
    color: "",
    x: 0,
    y: 0,
    table: {
      fields: fields.map((f, i) => ({
        name: f,
        type: { base: "bigint" } as any,
        nullable: false,
        default: null,
        comment: "",
        pk: i === 0,
        autoIncrement: i === 0,
        unique: false,
      })),
      indexes: [],
    },
  } as DbDesignNode;
}

function rel(from: string, to: string, field: string) {
  return {
    id: `r-${from}-${to}`,
    name: "",
    from: { node: from, field },
    to: { node: to, field: `${to}_id` },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  };
}

/**
 * 父子方向：父必须在子**左边**（x 更小），即「父左子右」。
 *
 * 这是「整理选中系列」的语义底线 —— 用户是照着思维导图的习惯看 ER 图的
 * （根在左、子往右逐层展开），父在子右边会让人第一眼就读反引用方向。
 */
describe("layout direction", () => {
  it("places parents strictly to the left of their children", () => {
    const doc: DbDesignDocument = {
      id: "d",
      name: "d",
      dialect: "mysql",
      nodes: [
        table("t1", "user", ["user_id", "name"]),
        table("t2", "article", ["article_id", "user_id", "title"]),
        table("t3", "comment", ["comment_id", "article_id", "content"]),
      ],
      relations: [rel("t2", "t1", "user_id"), rel("t3", "t2", "article_id")],
    } as any;

    const pos = computeLayout(doc, { fieldRows: "all" });
    expect(pos["t1"].x).toBeLessThan(pos["t2"].x);
    expect(pos["t2"].x).toBeLessThan(pos["t3"].x);
  });

  it("still orders left-to-right when the selected node is a middle child", () => {
    // 选中 article 时的高亮子集：父 user + 自己 + 子 comment
    const doc: DbDesignDocument = {
      id: "d",
      name: "d",
      dialect: "mysql",
      nodes: [
        table("t1", "user", ["user_id", "name"]),
        table("t2", "article", ["article_id", "user_id", "title"]),
        table("t3", "comment", ["comment_id", "article_id", "content"]),
      ],
      relations: [rel("t2", "t1", "user_id"), rel("t3", "t2", "article_id")],
    } as any;

    const pos = computeLayout(doc, { fieldRows: "all" });
    // user(0) → article(1) → comment(2)
    expect(pos["t1"].x).toBeLessThan(pos["t2"].x);
    expect(pos["t2"].x).toBeLessThan(pos["t3"].x);
  });

  it("keeps a root with several children on one column to the right", () => {
    const doc: DbDesignDocument = {
      id: "d",
      name: "d",
      dialect: "mysql",
      nodes: [
        table("t1", "user", ["user_id", "name"]),
        table("t2", "article", ["article_id", "user_id"]),
        table("t3", "order", ["order_id", "user_id"]),
      ],
      relations: [rel("t2", "t1", "user_id"), rel("t3", "t1", "user_id")],
    } as any;

    const pos = computeLayout(doc, { fieldRows: "all" });
    expect(pos["t1"].x).toBeLessThan(pos["t2"].x);
    expect(pos["t1"].x).toBeLessThan(pos["t3"].x);
    // 两个孩子同列（都是 depth 1）
    expect(pos["t2"].x).toBe(pos["t3"].x);
    // 且不重叠
    expect(pos["t3"].y).toBeGreaterThan(pos["t2"].y);
  });

  /**
   * 「整理选中系列」的范围必须是**整个系列**，不是选中表的直接邻居。
   *
   * 直接邻居通常只有两三张：把它们排齐了，同系列的孙表、兄弟表仍然散在原地，
   * 整体依旧不整齐。系列（外键连通分量）才是那个有完整父子层次的整体。
   */
  it("arranges the whole series, not just the selected node's neighbours", () => {
    const doc: DbDesignDocument = {
      id: "d",
      name: "d",
      dialect: "mysql",
      nodes: [
        table("t1", "user", ["user_id", "name"]),
        table("t2", "article", ["article_id", "user_id"]),
        // t3 是孙表：直接邻居里没有它，但它和 t2 同系列
        table("t3", "comment", ["comment_id", "article_id"]),
        table("t4", "tag", ["tag_id", "article_id"]),
      ],
      relations: [
        rel("t2", "t1", "user_id"),
        rel("t3", "t2", "article_id"),
        rel("t4", "t2", "article_id"),
      ],
    } as any;

    // 选中 t2：直接邻居是 t1 / t3 / t4，但系列是全部四张
    const tables = doc.nodes.filter((n) => n.kind === "table");
    const series = groupByForeignKey(doc, tables).find((g) =>
      g.some((n) => n.id === "t2"),
    );
    expect(series?.length).toBe(4);

    const sub = { ...doc, nodes: series!, relations: doc.relations };
    const pos = computeLayout(sub as any, { fieldRows: "all" });

    // 四张全部有落点（一张都不能漏 —— 漏掉的会留在原地，破坏「整齐」）
    expect(Object.keys(pos).sort()).toEqual(["t1", "t2", "t3", "t4"]);
    // 逐层往右：user(0) → article(1) → comment/tag(2)
    expect(pos["t1"].x).toBeLessThan(pos["t2"].x);
    expect(pos["t2"].x).toBeLessThan(pos["t3"].x);
    expect(pos["t2"].x).toBeLessThan(pos["t4"].x);
    // 孙表同列且不重叠
    expect(pos["t3"].x).toBe(pos["t4"].x);
    expect(pos["t4"].y).toBeGreaterThan(pos["t3"].y);
  });

  /**
   * 只动一个系列：另一个系列的坐标必须原样保留。
   * 这正是它区别于整份「自动布局」的地方 —— 后者会把用户摆过的其它系列也重排。
   */
  it("leaves tables of other series untouched", () => {
    const doc: DbDesignDocument = {
      id: "d",
      name: "d",
      dialect: "mysql",
      nodes: [
        table("t1", "user", ["user_id", "name"]),
        table("t2", "article", ["article_id", "user_id"]),
        // 另一个系列（与 t1/t2 无关联）
        table("t9", "config", ["config_id", "value"]),
      ],
      relations: [rel("t2", "t1", "user_id")],
    } as any;

    const tables = doc.nodes.filter((n) => n.kind === "table");
    const series = groupByForeignKey(doc, tables).find((g) => g.some((n) => n.id === "t2"))!;
    expect(series.map((n) => n.id).sort()).toEqual(["t1", "t2"]);

    const pos = computeLayout({ ...doc, nodes: series, relations: doc.relations } as any, {
      fieldRows: "all",
    });
    // 只返回系列内的落点，别的系列根本没有 key —— 调用方据此跳过它们
    expect(Object.keys(pos).sort()).toEqual(["t1", "t2"]);
    expect(pos["t9"]).toBeUndefined();
  });
});
