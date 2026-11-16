import { describe, expect, it } from "vitest";
import { pickXImportChannel } from "../xImportChannel";

describe("X 导入通道选择", () => {
  it("未配置任何凭据时要求先配置", () => {
    expect(pickXImportChannel({ cookieReady: false, officialReady: false })).toBe("none");
  });

  it("只配了 Cookie 时走免费通道", () => {
    expect(pickXImportChannel({ cookieReady: true, officialReady: false })).toBe("cookie");
  });

  it("只配了官方授权时走官方通道", () => {
    expect(pickXImportChannel({ cookieReady: false, officialReady: true })).toBe("official");
  });

  /// 官方 API 按量计费，且只能取最近 800 条；Cookie 通道免费且能翻全量。
  /// 两者都可用时必须默认免费，否则用户会在不知情的情况下被计费。
  it("两者都可用时优先免费通道", () => {
    expect(pickXImportChannel({ cookieReady: true, officialReady: true })).toBe("cookie");
  });
});
