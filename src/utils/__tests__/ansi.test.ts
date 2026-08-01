import { describe, it, expect } from "vitest";
import { stripAnsi } from "../ansi";

describe("stripAnsi", () => {
  it("移除颜色转义序列", () => {
    expect(stripAnsi("\x1b[31mred\x1b[0m")).toBe("red");
  });

  it("移除组合转义码", () => {
    expect(stripAnsi("\x1b[1;32mok\x1b[0m")).toBe("ok");
  });

  it("移除控制字符", () => {
    expect(stripAnsi("a\x07b\x1bc")).toBe("abc");
  });

  it("保留纯文本", () => {
    expect(stripAnsi("hello world")).toBe("hello world");
  });
});
