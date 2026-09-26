import { describe, expect, it } from "vitest";
import { partitionAgentOps, type AgentOp } from "../types";

function op(action: AgentOp["action"], id?: string): AgentOp {
  return { action, id };
}

describe("partitionAgentOps", () => {
  it("所有 op 都直接应用（不再有待确认清单）", () => {
    // 早先删除/移动要弹确认清单、后端阻塞等用户裁决；现在统一直接落图，
    // 回退交给界面上的 Ctrl+Z 撤销快照。
    const { auto, confirm } = partitionAgentOps([
      op("add", "a1"),
      op("delete", "d1"),
      op("update", "u1"),
      op("move", "m1"),
    ]);
    expect(auto.map((o) => o.action)).toEqual(["add", "delete", "update", "move"]);
    expect(confirm).toEqual([]);
  });

  it("保持原始顺序（ops 有先后依赖：父必须先于子落图）", () => {
    const ops = [op("delete", "d1"), op("delete", "d2"), op("move", "m1")];
    const { auto } = partitionAgentOps(ops);
    expect(auto.map((o) => o.id)).toEqual(["d1", "d2", "m1"]);
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
