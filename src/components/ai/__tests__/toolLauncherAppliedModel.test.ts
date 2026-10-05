import { describe, expect, it } from "vitest";
import { resolveAppliedModelRef } from "../ToolLauncher";

// 复刻真实供应商列表形状：OpenCode 真有一个叫 gpt-6-astra 的模型，
// 而 WorkBuddy2API 的真实模型叫 space-bunny —— 伪装名正好撞上前者。
const PROVIDERS = [
  { id: "workbuddy2api", models: [{ id: "space-bunny" }, { id: "hy3" }] },
  { id: "custom_opencode", models: [{ id: "gpt-6-astra" }, { id: "gpt-5.1-codex" }] },
] as any;

describe("resolveAppliedModelRef", () => {
  it("hasLastSelection=true: 伪装名不得夺走选择权", () => {
    // config.toml 里写的是伪装名 gpt-6-astra；拿它反查会命中 OpenCode 的同名模型，
    // 把「WorkBuddy2API / space-bunny」整个顶掉（Q-0323 引入的回归）。
    expect(resolveAppliedModelRef(PROVIDERS, "gpt-6-astra", true)).toBeNull();
    expect(resolveAppliedModelRef(PROVIDERS, "space-bunny", true)).toBeNull();
  });

  it("hasLastSelection=false: 仍要回填（用户在别处改过工具配置的兜底路径不能丢）", () => {
    expect(resolveAppliedModelRef(PROVIDERS, "space-bunny", false)).toEqual({
      providerId: "workbuddy2api",
      modelId: "space-bunny",
    });
    expect(resolveAppliedModelRef(PROVIDERS, "gpt-6-astra", false)).toEqual({
      providerId: "custom_opencode",
      modelId: "gpt-6-astra",
    });
  });

  it("空值 / 空白 / 带 provider 前缀", () => {
    expect(resolveAppliedModelRef(PROVIDERS, null, false)).toBeNull();
    expect(resolveAppliedModelRef(PROVIDERS, "  ", false)).toBeNull();
    expect(resolveAppliedModelRef(PROVIDERS, "anyversion/space-bunny", false)).toEqual({
      providerId: "workbuddy2api",
      modelId: "space-bunny",
    });
  });
});