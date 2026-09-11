import { describe, expect, it } from "vitest";
import { sortLauncherItemsByUsage } from "../usageStats";
import type { Item } from "../types";

function item(id: number, name: string, openNumber: number): Item {
  return {
    id,
    classificationId: 1,
    name,
    itemType: 0,
    data: { openNumber },
    shortcutKey: null,
    globalShortcutKey: false,
    order: id,
  };
}

describe("sortLauncherItemsByUsage", () => {
  it("按启动次数降序排列，并用名称稳定打破平局", () => {
    const result = sortLauncherItemsByUsage([
      item(1, "Beta", 2),
      item(2, "Alpha", 5),
      item(3, "Gamma", 2),
      item(4, "Never", 0),
    ]);

    expect(result.map((entry) => entry.name)).toEqual(["Alpha", "Beta", "Gamma"]);
  });

  it("不修改原始数组，并过滤从未启动的项目", () => {
    const source = [item(1, "Used", 1), item(2, "Never", 0)];
    const result = sortLauncherItemsByUsage(source);

    expect(result).not.toBe(source);
    expect(source.map((entry) => entry.name)).toEqual(["Used", "Never"]);
    expect(result).toHaveLength(1);
  });
});
