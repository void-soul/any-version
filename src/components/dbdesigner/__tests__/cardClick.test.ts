import { describe, it, expect } from "vitest";

import {
  CARD_ROLE_ATTR,
  ROLE_FIELD_AREA,
  ROLE_FIELD_ROW,
  ROLE_HEADER,
  cardClickAction,
  clickRoleOf,
} from "../cardClick";

/** 造一个「closest 能返回带某角色的元素」的目标，用来验证属性读取路径 */
function targetWithRole(role: string | null): EventTarget {
  const node = role
    ? { getAttribute: (k: string) => (k === CARD_ROLE_ATTR ? role : null) }
    : null;
  return {
    closest: () => node,
  } as unknown as EventTarget;
}

describe("卡片点击分流", () => {
  it("点表头 → 只高亮，不开面板", () => {
    expect(cardClickAction(ROLE_HEADER)).toEqual({ kind: "highlight" });
  });

  it("点字段行 / 字段区 → 开面板，不高亮", () => {
    expect(cardClickAction(ROLE_FIELD_ROW)).toEqual({ kind: "openDock" });
    expect(cardClickAction(ROLE_FIELD_AREA)).toEqual({ kind: "openDock" });
  });

  it("未知角色与缺失角色都按「高亮」处理（fail-safe 到旧行为）", () => {
    // 新增标记忘了加规则时，用户至少看到「点了有反应」，而不是完全没反应
    expect(cardClickAction("some-future-role")).toEqual({ kind: "highlight" });
    expect(cardClickAction(null)).toEqual({ kind: "highlight" });
    expect(cardClickAction(undefined)).toEqual({ kind: "highlight" });
  });

  it("读角色用 closest：点在字段行内部的子元素上也算点字段", () => {
    // 字段行里还有 span / 连线锚点，点它们不能落到「表头」分支上
    expect(clickRoleOf(targetWithRole(ROLE_FIELD_ROW))).toBe(ROLE_FIELD_ROW);
    expect(clickRoleOf(targetWithRole(ROLE_HEADER))).toBe(ROLE_HEADER);
    // closest 没命中（点在卡片外的空白）→ null
    expect(clickRoleOf(targetWithRole(null))).toBeNull();
  });

  it("目标不是元素时不抛错（防御：target 可能为空或非 Element）", () => {
    expect(clickRoleOf(null)).toBeNull();
    expect(() => clickRoleOf({} as EventTarget)).not.toThrow();
    expect(clickRoleOf({} as EventTarget)).toBeNull();
  });

  it("两处行为互斥：不会既高亮又开面板", () => {
    for (const role of [ROLE_HEADER, ROLE_FIELD_ROW, ROLE_FIELD_AREA, null, "weird"]) {
      const a = cardClickAction(role);
      // kind 是联合类型里的一个，天然互斥；这里再钉一次取值集合防止以后加出第四种
      expect(["openDock", "highlight"]).toContain(a.kind);
    }
  });
});
