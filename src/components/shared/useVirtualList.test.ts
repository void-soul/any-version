import { describe, expect, it } from "vitest";

import { buildOffsets, computeRange, findIndexAtOffset } from "./useVirtualList";

const ids = () => (i: number) => `k${i}`;

describe("buildOffsets", () => {
  it("未测量的行用估计高度，测量过的用真实高度", () => {
    const heights = new Map([["k1", 100]]);
    const offsets = buildOffsets(3, 50, heights, ids());
    expect(offsets).toEqual([0, 50, 150, 200]);
    expect(offsets[offsets.length - 1]).toBe(200);
  });

  it("空列表总高度为 0", () => {
    expect(buildOffsets(0, 72, new Map(), ids())).toEqual([0]);
  });
});

describe("findIndexAtOffset", () => {
  it("落在行内返回该行，边界归上行/下行正确", () => {
    const offsets = [0, 50, 150, 200];
    expect(findIndexAtOffset(offsets, 0)).toBe(0);
    expect(findIndexAtOffset(offsets, 49)).toBe(0);
    expect(findIndexAtOffset(offsets, 50)).toBe(1);
    expect(findIndexAtOffset(offsets, 149)).toBe(1);
    expect(findIndexAtOffset(offsets, 150)).toBe(2);
    // 超出总高度也要夹到最后一行，不能越界
    expect(findIndexAtOffset(offsets, 10_000)).toBe(2);
    expect(findIndexAtOffset([0], 100)).toBe(0);
  });
});

describe("computeRange", () => {
  it("渲染区间含 overscan，占位高度之和等于总高度", () => {
    const heights = new Map<string, number>();
    const offsets = buildOffsets(1000, 50, heights, ids());
    const range = computeRange(offsets, 1000, 5000, 500, 4);
    // scrollTop=5000 落在第 100 行（每行 50），前 4 行缓冲 → start=96；
    // 视口底边 5500 落在第 110 行，含该行 + 后 4 行缓冲 → end=115
    expect(range.start).toBe(96);
    expect(range.end).toBe(115);
    expect(range.topPad).toBe(4800);
    expect(range.bottomPad).toBe(offsets[1000] - offsets[115]);
    // 关键不变量：上占位 + 渲染区高度 + 下占位 = 总高度
    expect(range.topPad + (offsets[range.end] - offsets[range.start]) + range.bottomPad).toBe(
      offsets[1000],
    );
  });

  it("滚动到顶部/底部不会越界，end 至少比 start 大 1", () => {
    const offsets = buildOffsets(10, 40, new Map(), ids());
    const top = computeRange(offsets, 10, 0, 200, 4);
    expect(top.start).toBe(0);
    expect(top.topPad).toBe(0);
    const bottom = computeRange(offsets, 10, 10_000, 200, 4);
    expect(bottom.end).toBe(10);
    expect(bottom.bottomPad).toBe(0);
    // 视口为 0（首帧尚未测量）时也要给出至少一行
    const zero = computeRange(offsets, 10, 0, 0, 4);
    expect(zero.end).toBeGreaterThan(zero.start);
  });

  it("空列表返回全零区间，不会让调用方渲染出占位", () => {
    expect(computeRange([0], 0, 0, 500)).toEqual({ start: 0, end: 0, topPad: 0, bottomPad: 0 });
  });

  it("大列表只渲染视口量级的行数（这正是性能收益所在）", () => {
    const offsets = buildOffsets(20_000, 60, new Map(), ids());
    const range = computeRange(offsets, 20_000, 600_000, 800, 4);
    expect(range.end - range.start).toBeLessThan(40);
  });
});
