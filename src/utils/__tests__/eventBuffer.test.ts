/**
 * eventBuffer 单元测试：模块级 Tauri 事件缓冲存储。
 * 通过 mock @tauri-apps/api/event 捕获后端订阅的 handler，手动注入事件载荷，
 * 验证缓冲、环形上限、微任务合并通知、clear 语义与版本号/快照引用稳定性。
 */
import { describe, it, expect, vi, beforeEach } from "vitest";

// mock 掉 Tauri 事件 API（node 测试环境无 window/__TAURI_INTERNALS__）：
// listen(event, handler) 记录 handler，返回退订函数。
const listenMock = vi.fn(
  async (_event: string, _handler: (e: { payload: unknown }) => void) => () => {}
);
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: Parameters<typeof listenMock>) => listenMock(...args),
}));

import { createEventBuffer } from "../eventBuffer";

// 取出某事件名注册的 handler（后端 → 前端推送入口）
function handlerOf(event: string): (e: { payload: unknown }) => void {
  const call = listenMock.mock.calls.find(([name]) => name === event);
  if (!call) throw new Error(`事件 ${event} 未注册监听`);
  return call[1];
}

const flushMicrotasks = () => new Promise<void>((r) => setTimeout(r, 0));

describe("eventBuffer", () => {
  beforeEach(() => {
    listenMock.mockClear();
  });

  it("同一事件单例：重复创建返回同一实例，后端订阅只挂一次", async () => {
    const bufA = createEventBuffer<{ v: number }>("test-event-a");
    const bufB = createEventBuffer<{ v: number }>("test-event-a", { limit: 1 }); // options 不覆盖已有实例

    handlerOf("test-event-a")({ payload: { v: 1 } });
    handlerOf("test-event-a")({ payload: { v: 2 } });
    await flushMicrotasks();

    // 同一事件名只挂一次后端订阅，且两个引用指向同一缓冲
    const calls = listenMock.mock.calls.filter(([name]) => name === "test-event-a");
    expect(calls).toHaveLength(1);
    expect(bufB).toBe(bufA);

    expect(bufA.size).toBe(2);
    expect(bufA.snapshot().map((x) => x.v)).toEqual([1, 2]);
  });

  it("环形缓冲：超过 limit 丢弃最旧条目", async () => {
    const buf = createEventBuffer<number>("test-event-b", { limit: 3 });
    const h = handlerOf("test-event-b");
    for (let i = 1; i <= 5; i++) h({ payload: i });
    await flushMicrotasks();

    expect(buf.size).toBe(3);
    expect(buf.snapshot()).toEqual([3, 4, 5]);
  });

  it("微任务合并通知：一轮事件风暴只触发一次订阅回调", async () => {
    const buf = createEventBuffer<number>("test-event-c");
    const h = handlerOf("test-event-c");
    const spy = vi.fn();
    buf.subscribe(spy);

    h({ payload: 1 });
    h({ payload: 2 });
    h({ payload: 3 });
    expect(spy).not.toHaveBeenCalled(); // 同步阶段不通知

    await flushMicrotasks();
    expect(spy).toHaveBeenCalledTimes(1);
    expect(buf.snapshot()).toEqual([1, 2, 3]);
  });

  it("clear 清空缓冲并通知订阅者；空缓冲重复 clear 不通知", async () => {
    const buf = createEventBuffer<number>("test-event-d");
    const h = handlerOf("test-event-d");
    const spy = vi.fn();
    buf.subscribe(spy);

    h({ payload: 1 });
    await flushMicrotasks();
    expect(buf.size).toBe(1);

    buf.clear();
    await flushMicrotasks();
    expect(buf.size).toBe(0);
    expect(buf.snapshot()).toEqual([]);
    expect(spy).toHaveBeenCalledTimes(2); // push 一次 + clear 一次

    spy.mockClear();
    buf.clear(); // 空缓冲再 clear：无变化不通知
    await flushMicrotasks();
    expect(spy).not.toHaveBeenCalled();
  });

  it("版本号随内容变化自增，快照引用在无变化时保持稳定", async () => {
    const buf = createEventBuffer<number>("test-event-e");
    const h = handlerOf("test-event-e");

    const s0 = buf.snapshot();
    expect(buf.version()).toBe(0);

    h({ payload: 1 });
    await flushMicrotasks();
    const v1 = buf.version();
    const s1 = buf.snapshot();
    expect(v1).toBeGreaterThan(0);
    expect(s1).not.toBe(s0); // 内容变了：新数组

    await flushMicrotasks();
    expect(buf.snapshot()).toBe(s1); // 内容没变：同一引用（useSyncExternalStore 依赖）
  });

  it("transform 转换载荷，异常时丢弃该事件", async () => {
    const buf = createEventBuffer<{ at: number }>("test-event-f", {
      transform: (p) => {
        const v = p as { bad?: boolean; n: number };
        if (v.bad) throw new Error("bad payload");
        return { at: v.n };
      },
    });
    const h = handlerOf("test-event-f");
    h({ payload: { n: 7 } });
    h({ payload: { bad: true } });
    await flushMicrotasks();

    expect(buf.size).toBe(1);
    expect(buf.snapshot()[0].at).toBe(7);
  });
});
