import { describe, expect, it } from "vitest";
import { filterProviders } from "../providerSearch";
import type { AiProvider } from "../types";

function provider(partial: Partial<AiProvider> & { id: string; name: string }): AiProvider {
  return {
    category: "provider",
    api_key: "",
    website: "",
    openai_url: "",
    anthropic_url: "",
    google_url: "",
    models: [],
    active_model_id: null,
    custom_headers: [],
    ...partial,
  };
}

const providers = [
  provider({ id: "anthropic", name: "Claude 官方", website: "https://anthropic.com" }),
  provider({
    id: "zhipu",
    name: "智谱 GLM",
    openai_url: "https://open.bigmodel.cn/api/coding/paas/v4",
  }),
  provider({ id: "local-relay", name: "本地聚合", category: "local" }),
];

describe("filterProviders", () => {
  it("关键词为空时原样返回入参", () => {
    expect(filterProviders(providers, "")).toBe(providers);
  });

  it("关键词只有空白时也原样返回", () => {
    expect(filterProviders(providers, "   ")).toBe(providers);
  });

  it("按显示名匹配且大小写不敏感", () => {
    expect(filterProviders(providers, "claude").map((p) => p.id)).toEqual(["anthropic"]);
  });

  it("按 id 匹配", () => {
    expect(filterProviders(providers, "zhipu").map((p) => p.id)).toEqual(["zhipu"]);
  });

  it("按官网与协议端点匹配", () => {
    expect(filterProviders(providers, "anthropic.com").map((p) => p.id)).toEqual(["anthropic"]);
    expect(filterProviders(providers, "bigmodel").map((p) => p.id)).toEqual(["zhipu"]);
  });

  it("支持部分匹配并保留原始顺序", () => {
    expect(filterProviders(providers, "o").map((p) => p.id)).toEqual([
      "anthropic",
      "zhipu",
      "local-relay",
    ]);
  });

  it("无匹配时返回空数组", () => {
    expect(filterProviders(providers, "不存在的供应商")).toEqual([]);
  });

  it("不改动入参", () => {
    filterProviders(providers, "zhipu");
    expect(providers).toHaveLength(3);
  });
});
