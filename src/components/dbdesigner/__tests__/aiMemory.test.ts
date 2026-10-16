import { describe, it, expect } from "vitest";

import { HISTORY_TURNS, mergeDecisions, trimHistory } from "../aiMemory";
import type { ChatTurn } from "../types";

describe("mergeDecisions（决策记录的所有权在前端）", () => {
  it("保留文档里已有的，本轮新增叠在后面", () => {
    const out = mergeDecisions(["status 含已关闭：财务要留关单记录"], ["article 增加草稿态"]);
    expect(out).toEqual(["status 含已关闭：财务要留关单记录", "article 增加草稿态"]);
  });

  it("模型这轮把旧决策原样吐回来时不会重复累加", () => {
    // 模型很可能下一轮把上一轮的决策再写一遍；不去重的话记录会越来越长而内容没变
    const once = mergeDecisions(undefined, ["A：枚举比 varchar 好"]);
    const twice = mergeDecisions(once, ["A：枚举比 varchar 好"]);
    expect(twice).toHaveLength(1);
  });

  it("去重忽略空白与大小写，但首尾空白会被清理", () => {
    const out = mergeDecisions(["Status  用枚举"], ["  status 用枚举  "]);
    expect(out).toEqual(["Status  用枚举"]);
  });

  it("空白项不算一条决策", () => {
    expect(mergeDecisions(undefined, ["", "   ", "\n"])).toEqual([]);
    expect(mergeDecisions(["A"], ["", "B"])).toEqual(["A", "B"]);
  });

  it("文档里还没有决策时（undefined）也能接上", () => {
    expect(mergeDecisions(undefined, ["新决策"])).toEqual(["新决策"]);
  });

  it("不改动传入的数组（别在别人的引用上追加）", () => {
    const existing = ["A"];
    mergeDecisions(existing, ["B"]);
    expect(existing).toEqual(["A"]);
  });
});

describe("trimHistory（会话历史滚动窗口）", () => {
  const turns = (n: number): ChatTurn[] =>
    Array.from({ length: n }, (_, i) => ({
      role: i % 2 === 0 ? ("user" as const) : ("assistant" as const),
      text: "t" + i,
    }));

  it("默认只留最近 3 轮（6 条）", () => {
    expect(HISTORY_TURNS).toBe(3);
    // 5 条还不到 6 条窗口，原样返回（只有超过窗口才截）
    expect(trimHistory(turns(5))).toHaveLength(5);
    expect(trimHistory(turns(9))).toHaveLength(6);
    expect(trimHistory(turns(6))).toHaveLength(6);
  });

  it("不足窗口时原样返回", () => {
    expect(trimHistory(turns(4))).toHaveLength(4);
    expect(trimHistory([])).toHaveLength(0);
  });

  it("**按轮次截断而不是按条数**：不能把某轮的问答劈成两半", () => {
    const out = trimHistory(turns(8));
    expect(out[0].role).toBe("user");
    expect(out[1].role).toBe("assistant");
  });

  it("助手那轮存的是决策摘要而不是整份 JSON（控制 token）", () => {
    const out = trimHistory([
      { role: "user", text: "加个 remark 字段" },
      { role: "assistant", text: "已按需求修改设计（1 项决策）" },
    ]);
    expect(out[1].text).not.toContain("nodes");
  });
});
