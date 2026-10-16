import { describe, it, expect } from "vitest";

import {
  MAX_CHAT_MSGS,
  nextMsgId,
  patchCounts,
  patchLast,
  startTurn,
  type ChatMsg,
} from "../chatState";
import type { PatchApplied } from "../types";

function doneMsgs(): ChatMsg[] {
  return [
    { id: "m1", role: "user", text: "第一句" },
    { id: "m2", role: "assistant", status: "done", summary: "已生成 4 个表" },
  ];
}

describe("startTurn（一轮 = 用户气泡 + 助手占位）", () => {
  it("一次产出两条，助手侧带 thinking 状态", () => {
    const got = startTurn([], "设计订单系统");
    expect(got).toHaveLength(2);
    expect(got[0]).toMatchObject({ role: "user", text: "设计订单系统" });
    expect(got[1]).toMatchObject({ role: "assistant", status: "thinking" });
  });

  it("继续接在已有对话后面（不是替换）", () => {
    const got = startTurn(doneMsgs(), "再加一个字段");
    expect(got).toHaveLength(4);
    expect(got[2].text).toBe("再加一个字段");
  });

  it("id 不重复（React key 撞车会让气泡错位）", () => {
    // startTurn 返回的是累积后的列表，b 已包含 a —— 不能再拼一遍
    const a = startTurn([], "x");
    const b = startTurn(a, "y");
    const ids = b.map((m) => m.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("超过上限时丢最旧的（否则面板里要滚很久才看到开头）", () => {
    let msgs: ChatMsg[] = [];
    for (let i = 0; i < MAX_CHAT_MSGS; i++) msgs = startTurn(msgs, `第${i}句`);
    expect(msgs.length).toBeLessThanOrEqual(MAX_CHAT_MSGS);
    // 最新一轮必须还在
    expect(msgs.some((m) => m.text === `第${MAX_CHAT_MSGS - 1}句`)).toBe(true);
  });
});

describe("patchLast（进度只能改最后一轮）", () => {
  it("改最后一轮助手气泡，而不是往前那些轮", () => {
    // 两轮对话：做过进度的应是第一轮助手气泡，而不是最后一个气泡
    const two = [...doneMsgs(), ...startTurn(doneMsgs(), "再加一个字段")];
    const got = patchLast(two, { status: "done", summary: "改过了" });
    expect(got[got.length - 1].summary).toBe("改过了");
    // 第一轮的结果不能被后来的进度覆盖
    expect(got[1].summary).toBe("已生成 4 个表");
  });

  it("忽略用户气泡，落到助手那一条", () => {
    const withUserLast: ChatMsg[] = [
      { id: "m1", role: "assistant", status: "thinking" },
      { id: "m2", role: "user", text: "追问" },
    ];
    const got = patchLast(withUserLast, { status: "done" });
    expect(got[0].status).toBe("done");
    expect(got[1].status).toBeUndefined();
  });

  it("没有助手气泡时原样返回（不崩、不凭空造一条）", () => {
    const onlyUser: ChatMsg[] = [{ id: "m1", role: "user", text: "hi" }];
    expect(patchLast(onlyUser, { status: "done" })).toEqual(onlyUser);
  });

  it("空对话原样返回", () => {
    expect(patchLast([], { status: "done" })).toEqual([]);
  });

  it("不改传入的数组", () => {
    const before = doneMsgs();
    patchLast(before, { status: "error" });
    expect(before[1].status).toBe("done");
  });
});

describe("patchCounts", () => {
  it("只取张数，不必把整份 PatchApplied 塞进气泡", () => {
    const p: PatchApplied = {
      addedTables: ["a", "b"],
      updatedTables: ["c"],
      removedTables: [],
      addedRelations: 1,
      removedRelations: 0,
      droppedRelations: [],
    };
    expect(patchCounts(p)).toEqual({ added: 2, updated: 1, removed: 0 });
  });
});

describe("nextMsgId", () => {
  it("单调递增", () => {
    const a = nextMsgId();
    const b = nextMsgId();
    expect(Number(b.slice(1))).toBeGreaterThan(Number(a.slice(1)));
  });
});
