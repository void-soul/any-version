import { describe, it, expect } from "vitest";
import { isGroupType } from "../ctrl";

describe("isGroupType (GROUP_TYPES 大小写修复回归测试)", () => {
  it("匹配 mihomo 内核返回的代理组 type（首字母大写驼峰）", () => {
    for (const t of ["Selector", "URLTest", "Fallback", "LoadBalance", "Relay"]) {
      expect(isGroupType(t)).toBe(true);
    }
  });

  it("对大小写/连字符不敏感", () => {
    expect(isGroupType("url-test")).toBe(true);
    expect(isGroupType("load_balance")).toBe(true);
    expect(isGroupType("SELECTOR")).toBe(true);
  });

  it("拒绝非组类型（普通节点类型 / 空值）", () => {
    for (const t of ["Direct", "Reject", "Shadowsocks", "Trojan", "Vmess", "", undefined as any]) {
      expect(isGroupType(t)).toBe(false);
    }
  });
});
