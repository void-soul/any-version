import { beforeEach, describe, expect, it, vi } from "vitest";

const { listenMock } = vi.hoisted(() => ({ listenMock: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

import { listenSubscriptionUpdated, SUBSCRIPTION_UPDATED_EVENT } from "./subscriptionEvents";

describe("自动更新订阅事件", () => {
  beforeEach(() => { listenMock.mockReset(); });

  it("用后端 emit 的事件名注册监听，事件到达时透传回调", async () => {
    const unlisten = vi.fn();
    listenMock.mockResolvedValue(unlisten);
    const handler = vi.fn();

    const cleanup = listenSubscriptionUpdated(handler);

    // 必须与 lifecycle.rs `auto_update_profiles` 中 app.emit 的字符串一致。
    expect(SUBSCRIPTION_UPDATED_EVENT).toBe("mihomo://subscription-updated");
    expect(listenMock).toHaveBeenCalledWith(SUBSCRIPTION_UPDATED_EVENT, handler);
    const registered = listenMock.mock.calls[0][1] as (e: unknown) => void;
    registered({ payload: { id: "sub_1" } });
    expect(handler).toHaveBeenCalledTimes(1);

    await Promise.resolve();
    cleanup();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("卸载发生在 listen resolve 之前时也能就地注销", async () => {
    let resolve!: (fn: () => void) => void;
    listenMock.mockReturnValueOnce(new Promise((r) => { resolve = r; }));
    const cleanup = listenSubscriptionUpdated(vi.fn());

    const unlisten = vi.fn();
    cleanup();
    resolve(unlisten);
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
