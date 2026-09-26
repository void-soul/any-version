import { describe, expect, it } from "vitest";
import { filterSyncDetails, SYNC_STATUS_FILTERS } from "../sessionSync";

describe("filterSyncDetails", () => {
  const details = [
    { id: "a", status: "copied" },
    { id: "b", status: "conflict" },
    { id: "c", status: "skipped" },
    { id: "d", status: "conflict" },
  ];

  it("all 原样返回入参", () => {
    expect(filterSyncDetails(details, "all")).toBe(details);
  });

  it("按状态筛出全部冲突项", () => {
    expect(filterSyncDetails(details, "conflict").map((d) => d.id)).toEqual(["b", "d"]);
  });

  it("没有该状态时返回空数组", () => {
    expect(filterSyncDetails(details, "failed")).toEqual([]);
  });

  it("未知状态返回空数组而不是报错", () => {
    expect(filterSyncDetails(details, "brand-new-status")).toEqual([]);
  });

  it("不改动入参", () => {
    filterSyncDetails(details, "conflict");
    expect(details.map((d) => d.id)).toEqual(["a", "b", "c", "d"]);
  });

  // 「已处理」不是后端状态，而是「有冲突处理结果」的前端视图：只有传入 resolvedIds 才筛得出东西
  it("resolved 按处理结果过滤", () => {
    const resolved = new Set(["b", "c"]);
    expect(filterSyncDetails(details, "resolved", resolved).map((d) => d.id)).toEqual(["b", "c"]);
  });

  it("resolved 未传处理结果集合时为空", () => {
    expect(filterSyncDetails(details, "resolved")).toEqual([]);
  });

  it("筛选项覆盖后端全部状态并追加已处理", () => {
    expect(SYNC_STATUS_FILTERS).toEqual([
      "all",
      "copied",
      "skipped",
      "partial",
      "conflict",
      "failed",
      "resolved",
    ]);
  });
});
