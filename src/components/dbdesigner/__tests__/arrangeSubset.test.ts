import { describe, it, expect } from "vitest";
import { groupByForeignKey, relatedTableIds, buildArrangeSubDoc } from "../series";
import type { DbDesignDocument, DbDesignNode, DbDesignRelation } from "../types";

function n(id: string): DbDesignNode {
  return {
    id,
    kind: "table",
    name: id,
    color: "",
    x: 0,
    y: 0,
    table: {
      fields: [{ name: "c", type: { base: "int" }, nullable: false, default: null, comment: "", pk: false, autoIncrement: false, unique: false }],
      indexes: [],
    },
  } as DbDesignNode;
}

/** from 指向 to：语义上是「子表 from → 父表 to」 */
function rel(from: string, to: string): DbDesignRelation {
  return {
    id: `${from}-${to}`,
    name: "",
    from: { node: from, field: "u" },
    to: { node: to, field: "c" },
    kind: "1-n",
    onDelete: "",
    onUpdate: "",
    mirror: false,
  } as DbDesignRelation;
}

/**
 * 一条「长链」：a → b → c → d → e，外加 f 挂在 b 上。
 * 外键连通分量是全部 6 张，而选中 a 时高亮只有 a / b / f 三张。
 */
function chainDoc(): DbDesignDocument {
  return {
    id: "d1",
    name: "链",
    dialect: "mysql",
    folderId: null,
    updatedAt: "",
    nodes: ["a", "b", "c", "d", "e", "f"].map(n),
    relations: [rel("b", "a"), rel("c", "b"), rel("d", "c"), rel("e", "d"), rel("f", "b")],
  } as DbDesignDocument;
}

describe("relatedTableIds（高亮集合）", () => {
  it("是选中表加一跳外键邻居，不含更远的表", () => {
    const got = relatedTableIds(chainDoc(), "a");
    // a 的邻居只有 b（b→a）；c 与 a 之间隔着 b，不该出现
    expect([...got].sort()).toEqual(["a", "b"]);
  });

  it("无向：父表被选中时子表同样高亮", () => {
    const got = relatedTableIds(chainDoc(), "b");
    expect([...got].sort()).toEqual(["a", "b", "c", "f"]);
  });

  it("未选中时为空集", () => {
    expect(relatedTableIds(chainDoc(), null).size).toBe(0);
  });
});

describe("buildArrangeSubDoc（整理集合）", () => {
  /**
   * 这条是本次修复的核心不变量。
   *
   * 曾经的实现整理的是 `groupByForeignKey` 的**整个连通分量**（本例 6 张全上），
   * 而按钮文案写的是「只把高亮中的表…其余不动」，提示条也报「已整理 6 张表」。
   * 用户的设计常常是一个业务整体（全部外键连通），于是「整理选中」实际等于
   * 「整理全部」，位置还因为早已按全量排过而看不出变化。
   */
  it("严格只含高亮集合里的表，连通分量里其余的表一律排除", () => {
    const doc = chainDoc();
    const ids = relatedTableIds(doc, "a");
    const sub = buildArrangeSubDoc(doc, ids);

    expect(sub.nodes.map((x) => x.id).sort()).toEqual(["a", "b"]);
    // 明确断言：连通分量里的其它表一个都不许混进来
    for (const absent of ["c", "d", "e", "f"]) {
      expect(sub.nodes.map((x) => x.id)).not.toContain(absent);
    }
  });

  it("只保留两端都在集合内的关联（指向外部的会让分层多出不存在的���端）", () => {
    const doc = chainDoc();
    // 高亮 b 时：b↔a、b↔c、b↔f 都在集合内…这里手工只给 a,b 来验证过滤
    const sub = buildArrangeSubDoc(doc, new Set(["a", "b"]));
    expect(sub.relations.map((r) => r.id).sort()).toEqual(["b-a"]);
  });

  it("n-n 关联不参与分层，从子文档里剔除", () => {
    const doc = chainDoc();
    const nn: DbDesignRelation = { ...rel("f", "a"), id: "nn", kind: "n-n" } as DbDesignRelation;
    const withNn = { ...doc, relations: [...doc.relations, nn] };
    const sub = buildArrangeSubDoc(withNn, new Set(["a", "b", "f"]));
    expect(sub.relations.map((r) => r.id)).not.toContain("nn");
  });

  it("空集合得到空 nodes（调用方据此放弃整理，不该崩）", () => {
    const sub = buildArrangeSubDoc(chainDoc(), new Set());
    expect(sub.nodes).toEqual([]);
    expect(sub.relations).toEqual([]);
  });

  /**
   * 照用户实际遇到的场景复现：22 张表**全部外键连通**（一个业务整体，
   * ai_task → order → order_escalate 那种），点「整理选中」却报「已整理 22 张表」。
   *
   * 断言写成 `not.toBe(22)` 之外还钉住「恰好等于高亮数」，这样将来若有人把
   * 整理集合改回连通分量，会**因为数字不等**而失败，而不是又一次悄悄退化。
   */
  it("22 张全连通的设计里，选中 1 张只整理高亮的那几张，绝不等于 22", () => {
    const ids = Array.from({ length: 22 }, (_, i) => `t${i}`);
    const doc = {
      id: "d1",
      name: "业务整体",
      dialect: "mysql",
      folderId: null,
      updatedAt: "",
      nodes: ids.map(n),
      // 串成一条长链：22 张表两两外键连通 → 连通分量就是全部 22 张
      relations: ids.slice(1).map((id, i) => rel(id, ids[i])),
    } as DbDesignDocument;

    // 先确认前提：这个设计里「选中表所在的连通分量」确实是 22 张
    const component = groupByForeignKey(doc).find((g) => g.some((x) => x.id === "t5"));
    expect(component).toHaveLength(22);

    const highlighted = relatedTableIds(doc, "t5");
    const sub = buildArrangeSubDoc(doc, highlighted);

    // 高亮 = t5 及其两个邻居 t4 / t6
    expect(highlighted.size).toBe(3);
    // 整理集合必须精确等于高亮集合
    expect(sub.nodes.map((x) => x.id).sort()).toEqual([...highlighted].sort());
    expect(sub.nodes).toHaveLength(3);
    expect(sub.nodes).not.toHaveLength(22);
  });
});
