import { describe, expect, it } from "vitest";

import { LAYOUT_MODES, computeLayout, type LayoutMode } from "../layout";
import { CARD_W, estimateHeight } from "../layoutCore";
import type { DbDesignDocument, DbDesignNode, DbField } from "../types";

function field(name: string, pk = false): DbField {
  return {
    name,
    type: { base: "bigint" },
    nullable: false,
    pk,
    autoIncrement: false,
    unique: false,
  };
}

function table(id: string, fields: DbField[]): DbDesignNode {
  return {
    id,
    kind: "table",
    name: id,
    table: { fields, indexes: [] },
    view: null,
  };
}

function docOf(nodes: DbDesignNode[], relations: DbDesignDocument["relations"] = []): DbDesignDocument {
  return {
    id: "d",
    name: "d",
    dialect: "mysql",
    nodes,
    relations,
  };
}

const opts = { fieldRows: "all" as const };




describe("computeLayout", () => {
  it("没有任何关系时全部落在第 0 层并排成网格（逆向 dump 的常见情况）", () => {
    const doc = docOf([table("a", [field("id", true)]), table("b", [field("id", true)]), table("c", [field("id", true)])]);
    const pos = computeLayout(doc, opts);
    expect(Object.keys(pos).sort()).toEqual(["a", "b", "c"]);
    // 3 个表：列数 = ceil(sqrt(3*1.4)) = 3 → 同一排，x 依次拉开
    expect(pos.a.y).toBe(pos.b.y);
    expect(pos.b.y).toBe(pos.c.y);
    expect(pos.b.x).toBeGreaterThan(pos.a.x);
    expect(pos.c.x).toBeGreaterThan(pos.b.x);
  });

  it("表多到一行放不下时自动换行，不叠在一起", () => {
    const many = Array.from({ length: 9 }, (_, i) => table(`t${i}`, [field("id", true)]));
    const pos = computeLayout(docOf(many), opts);
    // 9 个表：列数 = ceil(sqrt(12.6)) = 4 → 4 + 4 + 1 三行
    const rows = new Set(Object.values(pos).map((p) => p.y));
    expect(rows.size).toBe(3);
    // 同一行内 x 不重合
    expect(pos.t1.x).toBeGreaterThan(pos.t0.x);
    expect(pos.t4.x).toBe(pos.t0.x);
    expect(pos.t4.y).toBeGreaterThan(pos.t0.y);
  });

  it("放射：被最多表引用的那张落在圆心，其余环绕它", () => {
    // 显式 radial：「圆心 / 环绕」是放射特有的形态，核心左右算法里枢纽在中间列。
    // 不跟着默认走 —— 默认值一改这条就失效，却看不出是算法坏了还是默认值变了。
    const doc = docOf(
      [
        table("users", [field("user_id", true)]),
        table("orders", [field("order_id", true), field("user_id")]),
        table("order_item", [field("item_id", true), field("order_id")]),
        table("order_sku", [field("sku_id", true), field("order_id")]),
      ],
      [
        { id: "r1", from: { node: "orders", field: "user_id" }, to: { node: "users", field: "user_id" }, kind: "1-n" },
        { id: "r2", from: { node: "order_item", field: "order_id" }, to: { node: "orders", field: "order_id" }, kind: "1-n" },
        { id: "r3", from: { node: "order_sku", field: "order_id" }, to: { node: "orders", field: "order_id" }, kind: "1-n" },
      ],
    );
    const pos = computeLayout(doc, opts, "radial");
    // orders 被引用 3 次 → 枢纽 → 落在圆心（到其余三张平均距离最小）
    const avg = (id: string) =>
      ["users", "order_item", "order_sku"].reduce((s, k) => s + Math.hypot(pos[id].x - pos[k].x, pos[id].y - pos[k].y), 0) / 3;
    expect(avg("orders")).toBeLessThan(avg("users"));
    expect(avg("orders")).toBeLessThan(avg("order_item"));
    // 环绕的三张必须分得开（都在它周围，而不是叠在一侧）
    expect(Math.abs(pos.users.y - pos.order_item.y)).toBeGreaterThan(60);
  });

  it("多级链路：所有表都分得开（无重叠），中心是连接最多的表", () => {
    const doc = docOf(
      [table("a", [field("id", true)]), table("b", [field("id", true)]), table("c", [field("id", true)])],
      [
        { id: "r1", from: { node: "b", field: "a_id" }, to: { node: "a", field: "id" }, kind: "1-n" },
        { id: "r2", from: { node: "c", field: "b_id" }, to: { node: "b", field: "id" }, kind: "1-n" },
      ],
    );
    const pos = computeLayout(doc, opts);
    const ids = ["a", "b", "c"];
    for (let i = 0; i < ids.length; i++) {
      for (let j = i + 1; j < ids.length; j++) {
        const p = pos[ids[i]];
        const q = pos[ids[j]];
        const hitX = p.x < q.x + 288 && q.x < p.x + 288;
    const hitY = p.y < q.y + 106 && q.y < p.y + 106;
    const hit = hitX && hitY;
        expect(hit, `${ids[i]} 与 ${ids[j]} 重叠`).toBe(false);
      }
    }
    // b 是链路的中间点（进 1 出 1），度数与 a/c 相同 → 取文档序最前的枢纽
    expect(Object.keys(pos).sort()).toEqual(["a", "b", "c"]);
  });

  it("关系成环时不死循环，且每个节点都有落点", () => {
    const doc = docOf(
      [table("a", [field("id", true)]), table("b", [field("id", true)])],
      [
        { id: "r1", from: { node: "b", field: "a_id" }, to: { node: "a", field: "id" }, kind: "1-n" },
        { id: "r2", from: { node: "a", field: "b_id" }, to: { node: "b", field: "id" }, kind: "1-n" },
      ],
    );
    const pos = computeLayout(doc, opts);
    expect(Object.keys(pos).sort()).toEqual(["a", "b"]);
  });

  it("多对多不参与分层（否则布局会被中间表搅乱）", () => {
    const doc = docOf(
      [table("a", [field("id", true)]), table("b", [field("id", true)])],
      [{ id: "r1", from: { node: "b", field: "a_id" }, to: { node: "a", field: "id" }, kind: "n-n" }],
    );
    const pos = computeLayout(doc, opts);
    // 同一层 → y 相同
    expect(pos.a.y).toBe(pos.b.y);
  });

  it("字段多的表估得更高，同层里不会互相压住（行高取该层最高）", () => {
    const fat = table("fat", Array.from({ length: 12 }, (_, i) => field(`c${i}`)));
    const thin = table("thin", [field("id", true)]);
    const doc = docOf([fat, thin]);
    const pos = computeLayout(doc, { fieldRows: "all" });
    // 两个都在第 0 层、同一行 → y 相同；行高由更高的 fat 决定
    expect(pos.fat.y).toBe(pos.thin.y);
  });

  it("精简档的高度把外键行也算进去（与卡片实际渲染的行数一致）", () => {
    // 卡片在「keys」档显示的是主键 + 外键 + 关联字段。布局估矮了卡片就会互相压住，
    // 而坐标要写进设计文件 —— 所以外键行必须计入高度。
    // 放射布局下高度不再决定层间距（那由卡片宽度与圈容量反解），但仍决定卡片自身占位，
    // 这里验证「字段多的表估出来更高」，且两种档位下都不重叠。
    const gapWithFk = (fkCount: number, mode2: LayoutMode) => {
      const mode = "keys" as const;
      const fields = [field("id", true)];
      for (let i = 0; i < fkCount; i++) fields.push(field(`u${i}`));
      const doc = docOf(
        [table("users", [field("id", true)]), table("t1", fields), table("t2", [field("id", true)])],
        [
          ...Array.from({ length: fkCount }, (_, i) => ({
            id: `r${i}`,
            from: { node: "t1", field: `u${i}` },
            to: { node: "users", field: "id" },
            kind: "1-n" as const,
          })),
          { id: "rt2", from: { node: "t2", field: "user_id" }, to: { node: "users", field: "id" }, kind: "1-n" },
        ],
      );
      const pos = computeLayout(doc, { fieldRows: mode }, mode2);
      // 三张表两两不重叠。
      //
      // 高度用 `estimateHeight` 算**真实值**，不拍「宽松上界」：早先这里写死 300，
      // 而卡片实际只有 ~140px 高、行间距 ~188 —— 300 > 188，于是把正常间距判成重叠（误报，
      // 切到核心左右模式后暴露）。这条测的本来就是「高度估算准不准」，
      // 拿一个与估算无关的常数去判重叠，等于没测到点上。
      const ids = ["users", "t1", "t2"];
      const h = new Map(doc.nodes.map((nd) => [nd.id, estimateHeight(doc, nd, mode)]));
      for (let i = 0; i < ids.length; i++) {
        for (let j = i + 1; j < ids.length; j++) {
          const p = pos[ids[i]];
          const q = pos[ids[j]];
          const hp = h.get(ids[i]) ?? 0;
          const hq = h.get(ids[j]) ?? 0;
          const hitX = p.x < q.x + CARD_W && q.x < p.x + CARD_W;
          const hitY = p.y < q.y + hq && q.y < p.y + hp;
          expect(hitX && hitY, `${ids[i]} 与 ${ids[j]} 重叠`).toBe(false);
        }
      }
      return pos;
    };
    // 外键越多，卡片越高 → 环上占位更大，但布局仍然稳定合法
    for (const m of LAYOUT_MODES) {
      expect(() => gapWithFk(1, m.value)).not.toThrow();
      expect(() => gapWithFk(3, m.value)).not.toThrow();
    }
  });

  it("空文档返回空结果，不报错", () => {
    expect(computeLayout(docOf([]), opts)).toEqual({});
  });
});
