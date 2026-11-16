import { describe, expect, it } from "vitest";
import { buildGroups, filterConnections, type GroupableConnection } from "../connectionGroups";

function conn(id: string, group: string, name = id): GroupableConnection {
  return { id, name, group };
}

describe("分组聚合", () => {
  it("空列表得到空分组", () => {
    expect(buildGroups([])).toEqual([]);
  });

  it("未分组统一收进空标签分组，并排在最后", () => {
    const groups = buildGroups([conn("a", ""), conn("b", "生产"), conn("c", "root")]);
    expect(groups.map((g) => g.label)).toEqual(["生产", ""]);
    expect(groups[1].items.map((i) => i.id)).toEqual(["a", "c"]);
  });

  it("分组按名称排序，组内按名称排序", () => {
    const groups = buildGroups([
      conn("z", "B组", "z"),
      conn("y", "A组", "y"),
      conn("x", "B组", "x"),
    ]);
    expect(groups.map((g) => g.label)).toEqual(["A组", "B组"]);
    expect(groups[1].items.map((i) => i.id)).toEqual(["x", "z"]);
  });

  it("分组带稳定 key，便于折叠状态持久化", () => {
    const groups = buildGroups([conn("a", "生产"), conn("b", "")]);
    expect(groups.map((g) => g.key)).toEqual(["生产", ""]);
    expect(groups[1].isUngrouped).toBe(true);
  });
});

describe("分组筛选", () => {
  const list = [conn("a", "生产", "web-1"), conn("b", "生产", "db-1"), conn("c", "", "local")];

  it("未选中分组时返回全部", () => {
    expect(filterConnections(list, null).map((c) => c.id)).toEqual(["a", "b", "c"]);
  });

  it("选中分组只返回该组", () => {
    expect(filterConnections(list, "生产").map((c) => c.id)).toEqual(["a", "b"]);
  });

  it("未分组用空字符串精确匹配，不会误伤有分组的连接", () => {
    expect(filterConnections(list, "").map((c) => c.id)).toEqual(["c"]);
  });
});
