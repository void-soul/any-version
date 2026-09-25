import { describe, expect, it } from "vitest";
import { parseAiResultLine } from "../aiResult";

describe("parseAiResultLine", () => {
  it("把清单项拆成标题 / 链接 / 说明", () => {
    // 这是 agent 输出的主力形态：`- [标题](链接) —— 一句话理由`，
    // 链接必须能单独取出来（前端要靠它 openUrl）
    const got = parseAiResultLine("- [pdfplumber](https://github.com/jsvine/pdfplumber) —— 表格提取很强");
    expect(got).toEqual({
      kind: "item",
      title: "pdfplumber",
      url: "https://github.com/jsvine/pdfplumber",
      note: "表格提取很强",
    });
  });

  it("说明为空时不留下空的破折号", () => {
    const got = parseAiResultLine("- [x](https://a.b)");
    expect(got.kind).toBe("item");
    if (got.kind === "item") {
      expect(got.note).toBe("");
    }
  });

  it("识别二级与三级标题", () => {
    expect(parseAiResultLine("## 汇总")).toEqual({ kind: "heading", level: 2, text: "汇总" });
    expect(parseAiResultLine("### PDF 解析")).toEqual({ kind: "heading", level: 3, text: "PDF 解析" });
  });

  it("认不出的行当普通文本，绝不丢内容", () => {
    expect(parseAiResultLine("共找到 3 条相关收藏")).toEqual({
      kind: "text",
      text: "共找到 3 条相关收藏",
    });
    // 没链接的清单项也要显示出来（模型偶尔会漏写链接）
    expect(parseAiResultLine("- 纯文本条目")).toEqual({ kind: "bullet", text: "纯文本条目" });
    expect(parseAiResultLine("   ")).toEqual({ kind: "blank" });
  });
});
