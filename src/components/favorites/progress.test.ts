import { describe, expect, it } from "vitest";

import { progressKeyOf, progressRowsOf } from "./progress";

describe("progressKeyOf", () => {
  it("优先用 task", () => {
    expect(progressKeyOf({ task: "zhihu", source: "zhihu", stage: "import" })).toBe("zhihu");
  });

  // 回归：知乎导入的进度事件漏填了 task，key 退化成 "import"，
  // 与在跑的任务名对不上 → 整条进度被静默丢弃，界面看起来没有进度条。
  it("漏填 task 时回退到 source，仍能对上任务名", () => {
    expect(progressKeyOf({ source: "zhihu", stage: "import" })).toBe("zhihu");
    const rows = progressRowsOf(
      { zhihu: { source: "zhihu", stage: "import" } },
      ["zhihu"],
    );
    expect(rows.map(([k]) => k)).toEqual(["zhihu"]);
  });

  it("task / source 都没有时退化到 stage，不会返回空串之外的怪值", () => {
    expect(progressKeyOf({ stage: "classify" })).toBe("classify");
    expect(progressKeyOf({})).toBe("");
  });
});

describe("progressRowsOf", () => {
  it("只保留正在跑的任务，任务结束后飘来的事件不会留在界面上", () => {
    const map = {
      github: { task: "github" },
      zhihu: { task: "zhihu" },
      classify: { task: "classify" },
    };
    expect(progressRowsOf(map, ["github", "classify"]).map(([k]) => k)).toEqual([
      "github",
      "classify",
    ]);
    expect(progressRowsOf(map, [])).toEqual([]);
  });
});
