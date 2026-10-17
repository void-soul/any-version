import { describe, it, expect } from "vitest";

import { resolveModelChoice, modelChoiceSettings } from "../modelChoice";

/**
 * 「选模型 → 记住 → 下次沿用」这条链的契约。
 *
 * 它曾经被静默丢弃过一次：Q-0421 把设计器 AI 面板重写成「窄条 + 气泡」时，
 * 输入框旁的模型选择器连同它的 onSelect 回调一起没了 —— 而那个回调是
 * **唯一**往 `dbd_save_settings` 写 lastProviderId/lastModelId 的地方。
 * 于是同时坏了两件事：用户再也选不了模型，且**上次选的也不再被记住**
 * （每次进模块都回退到第一个供应商的 active model）。
 *
 * 纯逻辑测试拖不动组件（带 i18n / Tauri / React Flow 依赖），所以这条链
 * 必须落成可单测的纯函数 —— Q-0424 补输入框时就是这么漏的：tsc 不报错，
 * 纯逻辑测试也测不到「界面上有没有那个入口」。
 */

/** 只取纯函数真正需要的字段，避免把整个 AiProviderLike 拖进测试 */
function prov(id: string, models: string[], active?: string) {
  return {
    id,
    active_model_id: active ?? null,
    models: models.map((m) => ({ id: m, name: m })),
  };
}

describe("resolveModelChoice（沿用上次选的模型，带失效回落）", () => {
  it("记住的供应商与模型都还在：原样沿用", () => {
    const got = resolveModelChoice({
      providers: [prov("p1", ["m1", "m2"], "m2"), prov("p2", ["x"])],
      remembered: { lastProviderId: "p2", lastModelId: "x" },
    });
    expect(got).toEqual({ providerId: "p2", modelId: "x", usedRemembered: true });
  });

  it("记住的供应商已被删除：回落到第一个供应商的 active model", () => {
    const got = resolveModelChoice({
      providers: [prov("p1", ["m1", "m2"], "m2")],
      remembered: { lastProviderId: "gone", lastModelId: "x" },
    });
    expect(got).toEqual({ providerId: "p1", modelId: "m2", usedRemembered: false });
  });

  it("供应商还在、但记住的模型已被删除：回落到该供应商的 active model", () => {
    // 直接套用一个不存在的模型 id 会得到空 id，后端只能回退默认，
    // 用户却以为自己还选着它 —— 这是这条回落存在的唯一理由。
    const got = resolveModelChoice({
      providers: [prov("p1", ["m1", "m2"], "m2")],
      remembered: { lastProviderId: "p1", lastModelId: "deleted_model" },
    });
    expect(got).toEqual({ providerId: "p1", modelId: "m2", usedRemembered: false });
  });

  it("active model 也为空：再回落到 models[0]", () => {
    const got = resolveModelChoice({
      providers: [prov("p1", ["m1", "m2"])],
      remembered: { lastProviderId: "p1", lastModelId: "deleted_model" },
    });
    expect(got).toEqual({ providerId: "p1", modelId: "m1", usedRemembered: false });
  });

  it("没有记住的选择：第一个供应商的 active model", () => {
    expect(resolveModelChoice({ providers: [prov("p1", ["m1"], "m1")] })).toEqual({
      providerId: "p1",
      modelId: "m1",
      usedRemembered: false,
    });
  });

  it("一个可用供应商都没有：null（调用方据此禁用提交）", () => {
    expect(resolveModelChoice({ providers: [] })).toBeNull();
    expect(resolveModelChoice({ providers: [], remembered: { lastProviderId: "p", lastModelId: "m" } })).toBeNull();
  });

  it("供应商连 models 都是空的：仍然给出 providerId，modelId 留空", () => {
    // 不能返回 null —— 供应商是配置好的，只是没列模型；
    // 返回 null 会让界面误判成「没配供应商」而禁用提交。
    const got = resolveModelChoice({ providers: [{ id: "p1", models: [] }] });
    expect(got).toEqual({ providerId: "p1", modelId: "", usedRemembered: false });
  });

  it("往返幂等：选定的结果存回去，下次解析必须得到同一个模型", () => {
    // 这条钉住「选择 → 保存 → 下次沿用」闭环。只要 resolveModelChoice 与
    // modelChoiceSettings 对不齐，用户就会遇到「刚选完的模型下次又变回去」。
    const providers = [prov("p1", ["m1", "m2"], "m2"), prov("p2", ["x", "y"], "y")];
    for (const p of providers) {
      for (const m of p.models) {
        const settings = modelChoiceSettings({ providerId: p.id, modelId: m.id });
        const again = resolveModelChoice({ providers, remembered: settings });
        expect(again).toEqual({ providerId: p.id, modelId: m.id, usedRemembered: true });
      }
    }
  });
});

describe("modelChoiceSettings（切换后要写回 dbd_save_settings 的载荷）", () => {
  it("载荷就是 providerId/modelId 两个字段（字段名不能改，后端按名读）", () => {
    expect(modelChoiceSettings({ providerId: "p2", modelId: "y" })).toEqual({
      lastProviderId: "p2",
      lastModelId: "y",
    });
  });

  it("空 modelId 也照写：宁可存空值，也不要留下上一次的旧 id", () => {
    // 反面后果：切到一个没有模型的供应商却没写回，下次进模块
    // 「沿用上次的选择」就会拿回上一个供应商的模型 id，与实际不符。
    expect(modelChoiceSettings({ providerId: "p1", modelId: "" })).toEqual({
      lastProviderId: "p1",
      lastModelId: "",
    });
  });
});