import { describe, expect, it } from "vitest";
import { getProxyInfo } from "../ToolLauncher";

const TOOL = {
  id: "chatgptdesktop",
  installed: true,
  supports_model: true,
  supports_anthropic: false,
  supports_google: false,
} as any;

const PROVIDER = { id: "p1", openai_url: "https://x/v1" } as any;

describe("getProxyInfo 的模型 / 伪装信息", () => {
  it("即使没有伪装，也返回真实模型（底部要常驻显示「用什么模型」）", () => {
    // 这里的 masquerade 参数是**后端 resolve_claimed_model 解析后的生效声明名**。
    // ChatGPT Desktop 现在会由后端自动补一个官方名（gpt-5.1-codex），所以真实运行中
    // 它不会是空 —— 本例保留「原样返回」的退化路径（后端取不到时前端退回手填值）。
    const info = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "space-bunny", "", "");
    expect(info).not.toBeNull();
    expect(info!.model).toBe("space-bunny");
    expect(info!.alias).toBeNull();
  });

  it("后端自动补的官方别名会显示成伪装名（ChatGPT Desktop + space-bunny）", () => {
    // 截图里的场景：真实模型 space-bunny 不是官方 OpenAI 名，后端补 gpt-5.1-codex
    const info = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "gpt-5.1-codex", "", "");
    expect(info!.model).toBe("space-bunny");
    expect(info!.alias).toBe("gpt-5.1-codex");
  });

  it("配了伪装时返回生效别名", () => {
    const info = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "claude-sonnet-4-6", "", "");
    expect(info!.model).toBe("space-bunny");
    expect(info!.alias).toBe("claude-sonnet-4-6");
  });

  it("fallback 配了伪装才出现在 fallbackAliases 里", () => {
    const withAlias = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "", "glm-5.3", "claude-haiku-4-5");
    expect(withAlias!.fallbackAliases).toEqual([["claude-haiku-4-5", "glm-5.3"]]);
    const noAlias = getProxyInfo(TOOL, PROVIDER, false, "space-bunny", "", "glm-5.3", "");
    expect(noAlias!.fallbackAliases).toEqual([]);
  });

  it("真实名与伪装名相同时 alias 为 null（不显示无意义的 x → x）", () => {
    const info = getProxyInfo(TOOL, PROVIDER, false, "glm-5.3", "glm-5.3", "", "");
    expect(info!.alias).toBeNull();
  });

  it("官方模式 / 无供应商时仍返回 null（不启动代理就不显示）", () => {
    expect(getProxyInfo(TOOL, PROVIDER, true, "m", "", "", "")).toBeNull();
    expect(getProxyInfo(TOOL, null, false, "m", "", "", "")).toBeNull();
  });
});