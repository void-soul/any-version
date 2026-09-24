import { describe, expect, it } from "vitest";
import { partitionAgentOps, type AgentOp } from "../types";

function op(action: AgentOp["action"], id?: string): AgentOp {
  return { action, id };
}

describe("partitionAgentOps", () => {
  it("把删除/移动分进确认组，新增/编辑分进直接应用组", () => {
    const { auto, confirm } = partitionAgentOps([
      op("add", "a1"),
      op("delete", "d1"),
      op("update", "u1"),
      op("move", "m1"),
    ]);
    expect(auto.map((o) => o.action)).toEqual(["add", "update"]);
    expect(confirm.map((o) => o.action)).toEqual(["delete", "move"]);
  });

  it("保持各组内的原始顺序", () => {
    const { auto, confirm } = partitionAgentOps([
      op("delete", "d1"),
      op("delete", "d2"),
      op("move", "m1"),
    ]);
    expect(confirm.map((o) => o.id)).toEqual(["d1", "d2", "m1"]);
    expect(auto).toEqual([]);
  });

  it("空数组返回两个空组", () => {
    expect(partitionAgentOps([])).toEqual({ auto: [], confirm: [] });
  });

  it("不改动入参数组", () => {
    const ops = [op("add", "a1"), op("delete", "d1")];
    partitionAgentOps(ops);
    expect(ops).toHaveLength(2);
  });
});
