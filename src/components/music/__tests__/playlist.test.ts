import { describe, expect, it } from "vitest";

import { createCursor, nextTrack, prevTrack, shuffleIndices } from "../playlist";

/** 固定序列的伪随机源（用于稳定断言洗牌结果） */
function seededRandom(seed: number) {
  let value = seed;
  return () => {
    value = (value * 1103515245 + 12345) % 2147483648;
    return value / 2147483648;
  };
}

describe("playlist 推进逻辑", () => {
  it("顺序模式循环推进", () => {
    const cursor = createCursor();
    expect(nextTrack({ current: 0, total: 3, mode: "sequence", cursor }).index).toBe(1);
    expect(nextTrack({ current: 2, total: 3, mode: "sequence", cursor }).index).toBe(0);
    // 还没有当前曲目 → 从第一首开始
    expect(nextTrack({ current: null, total: 3, mode: "sequence", cursor }).index).toBe(0);
  });

  it("单曲模式始终返回当前曲", () => {
    const cursor = createCursor();
    expect(nextTrack({ current: 2, total: 5, mode: "single", cursor }).index).toBe(2);
    expect(prevTrack({ current: 2, total: 5, mode: "single", cursor }).index).toBe(2);
  });

  it("列表为空时返回 null", () => {
    const cursor = createCursor();
    expect(nextTrack({ current: null, total: 0, mode: "shuffle", cursor }).index).toBeNull();
    expect(prevTrack({ current: null, total: 0, mode: "sequence", cursor }).index).toBeNull();
  });

  it("随机模式一个洗牌袋内不重复，且覆盖全部曲目", () => {
    const total = 6;
    let cursor = createCursor();
    const random = seededRandom(42);
    const played: number[] = [];
    let current: number | null = null;
    for (let i = 0; i < total; i += 1) {
      const result = nextTrack({ current, total, mode: "shuffle", cursor, random });
      current = result.index;
      cursor = result.cursor;
      played.push(current!);
    }
    expect(new Set(played).size).toBe(total);
  });

  it("随机模式重新洗牌后不会紧接着重复当前曲", () => {
    const total = 4;
    const random = seededRandom(7);
    let cursor = createCursor();
    // 先放掉一整袋
    let current: number | null = null;
    for (let i = 0; i < total; i += 1) {
      const result = nextTrack({ current, total, mode: "shuffle", cursor, random });
      current = result.index;
      cursor = result.cursor;
    }
    // 袋已空：再取一首应触发重洗，且不等于 current
    const result = nextTrack({ current, total, mode: "shuffle", cursor, random });
    expect(result.index).not.toBe(current);
    expect(result.cursor.bag).toHaveLength(total - 1);
  });

  it("洗牌是完整的排列", () => {
    const indices = shuffleIndices(10, seededRandom(123));
    expect([...indices].sort((a, b) => a - b)).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
  });

  it("洗牌可避免把当前曲放在首位", () => {
    // 用极端随机源（始终取 0）让洗牌结果确定，再验证排除逻辑
    const indices = shuffleIndices(3, () => 0, 0);
    expect(indices[0]).not.toBe(0);
  });

  it("上一首按历史回退（顺序与随机通用）", () => {
    const random = seededRandom(99);
    let cursor = createCursor();
    const first = nextTrack({ current: null, total: 5, mode: "shuffle", cursor, random });
    cursor = first.cursor;
    const second = nextTrack({ current: first.index, total: 5, mode: "shuffle", cursor, random });
    cursor = second.cursor;
    // 回退应回到第一首，且历史被弹出
    const back = prevTrack({ current: second.index, total: 5, mode: "shuffle", cursor });
    expect(back.index).toBe(first.index);
    expect(back.cursor.history).toHaveLength(0);
  });

  it("没有历史时顺序模式退一格、随机模式回到首曲", () => {
    const cursor = createCursor();
    expect(prevTrack({ current: 2, total: 5, mode: "sequence", cursor }).index).toBe(1);
    expect(prevTrack({ current: 2, total: 5, mode: "shuffle", cursor }).index).toBe(0);
  });
});
