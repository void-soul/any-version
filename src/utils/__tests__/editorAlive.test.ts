import { describe, expect, it } from "vitest";
import { isEditorAlive } from "../editorAlive";

/**
 * 活性判定是防 `InstantiationService has been disposed` 的第二道防线，
 * 所以它的每种「不活性」都必须被正确识别 —— 漏一种就等于没防住。
 */
describe("isEditorAlive", () => {
  it("returns false for null/undefined refs", () => {
    expect(isEditorAlive(null)).toBe(false);
    expect(isEditorAlive(undefined)).toBe(false);
  });

  it("returns true for a live editor", () => {
    const editor = {
      getModel: () => ({ isDisposed: () => false }),
    };
    expect(isEditorAlive(editor)).toBe(true);
  });

  it("returns false once the model is disposed", () => {
    // 这就是踩坑现场：编辑器已卸载、被 dispose，但 ref 仍指向它
    const editor = {
      getModel: () => ({ isDisposed: () => true }),
    };
    expect(isEditorAlive(editor)).toBe(false);
  });

  it("returns false when the editor has no model", () => {
    // 还没加载完文件的编辑器：model 为 null
    const editor = { getModel: () => null };
    expect(isEditorAlive(editor)).toBe(false);
  });

  it("returns false when the object is not an editor at all", () => {
    // 传错了对象（比如把 state 存进了 ref）—— 宁可跳过也不要抛内部错误
    expect(isEditorAlive({})).toBe(false);
    expect(isEditorAlive({ getModel: "not a function" })).toBe(false);
    expect(isEditorAlive("editor")).toBe(false);
  });

  it("tolerates models without isDisposed (older shapes)", () => {
    const editor = { getModel: () => ({}) };
    expect(isEditorAlive(editor)).toBe(true);
  });
});
