import { describe, it, expect } from "vitest";
import { pinyinInitials, matchPinyin } from "../pinyin";

describe("pinyinInitials", () => {
  it("中文取拼音首字母", () => {
    expect(pinyinInitials("微信")).toBe("WX");
    expect(pinyinInitials("谷歌浏览器")).toBe("GGLLQ");
  });

  it("英文取单词首字母", () => {
    expect(pinyinInitials("Visual Studio Code")).toBe("VSC");
  });

  it("单英文单词取首字母", () => {
    expect(pinyinInitials("VSCode")).toBe("V");
  });

  it("混合名称：英文取单词首字母 + 中文取拼音首字母", () => {
    expect(pinyinInitials("WPS 表格")).toBe("WBG");
  });
});

describe("matchPinyin", () => {
  it("支持中文拼音首字母匹配", () => {
    expect(matchPinyin("微信", "wx")).toBe(true);
    expect(matchPinyin("微信", "WX")).toBe(true);
  });

  it("支持中文全拼匹配", () => {
    expect(matchPinyin("微信", "weixin")).toBe(true);
  });

  it("支持子串匹配", () => {
    expect(matchPinyin("Visual Studio Code", "visual")).toBe(true);
    expect(matchPinyin("微信", "微")).toBe(true);
  });

  it("支持英文单词首字母匹配", () => {
    expect(matchPinyin("Visual Studio Code", "vsc")).toBe(true);
  });

  it("不匹配时返回 false", () => {
    expect(matchPinyin("微信", "qq")).toBe(false);
    expect(matchPinyin("Visual Studio Code", "xyz")).toBe(false);
  });
});
