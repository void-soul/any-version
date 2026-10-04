import { describe, expect, it } from "vitest";
import { getProxyInfo } from "../ToolLauncher";

const TOOL = {
  id: "claudedesktop",
  installed: true,
  supports_model: true,
  supports_anthropic: true,
  supports_google: false,
} as any;

const PROVIDER = { id: "p1", anthropic_url: "https://x/anthropic" } as any;

describe("getProxyInfo 的伪装映射条目", () => {
  it("用外部传入的生效别名构造条目（Claude Desktop 留空时也能显示）", () => {
    const info = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "claude-sonnet-4-6", "", "");
    expect(info).not.toBeNull();
    expect(info!.aliasEntries).toEqual([["claude-sonnet-4-6", "space-bunny"]]);
  });

  it("真实名与伪装名相同时不产生条目（避免 glm-5.3 → glm-5.3 这种无意义映射）", () => {
    const info = getProxyInfo(TOOL, PROVIDER, false, "glm-5.3", "glm-5.3", "", "");
    expect(info!.aliasEntries).toEqual([]);
  });

  it("别名不同才显示（手填伪装名的既有行为不变）", () => {
    const info = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "claude-opus-4", "", "");
    expect(info!.aliasEntries).toEqual([["claude-opus-4", "space-bunny"]]);
  });

  it("fallback 小模型的条目仍然正确，且与主模型条目并存", () => {
    const info = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "claude-sonnet-4-6", "glm-5.3", "claude-haiku-4-5");
    expect(info!.aliasEntries).toEqual([["claude-sonnet-4-6", "space-bunny"], ["claude-haiku-4-5", "glm-5.3"]]);
  });

  it("fallback 未填伪装名时也不产生条目（与主模型同规则）", () => {
    // 用户只填了 fallback 模型、没填伪装名 → 没发生伪装，不该显示 `glm-5.3 → glm-5.3`
    const info = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "", "glm-5.3", "");
    expect(info!.aliasEntries).toEqual([]);
  });

  it("fallback 填了伪装名时照常显示", () => {
    const info = getProxyInfo(TOOL, PROVIDER, false, "", "", "glm-5.3", "claude-haiku-4-5");
    expect(info!.aliasEntries).toEqual([["claude-haiku-4-5", "glm-5.3"]]);
  });
});